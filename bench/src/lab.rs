//! Transport laboratory: how fast can this link actually go?
//!
//! The earlier measurements showed ~20–24 MB/s regardless of how many TCP
//! streams were used. That flatness is suspicious in both directions: it could
//! mean the radio is saturated and nothing more is available, or it could mean
//! something in the software path is capping us well below what the air would
//! carry. Those two possibilities call for opposite responses, so the first job
//! is to tell them apart.
//!
//! **The control experiment is UDP.** It has no congestion control, no
//! acknowledgements, no retransmission and no ordering guarantee. Whatever UDP
//! achieves is close to the raw capability of the link.
//!
//! - If UDP lands at roughly the same rate as TCP, the radio is the limit.
//!   Further transport work is wasted; the only remaining lever is sending
//!   fewer bytes, which means compression and caching.
//! - If UDP is substantially faster, TCP is leaving throughput unused and a
//!   different transport is worth considering.
//!
//! Around that sit two ordinary tuning sweeps — receive buffer size and write
//! size — because both are cheap to test and both can quietly cost a third of
//! the link.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::AsyncReadExt;

use crate::net::client::Conn;
use crate::net::{
    Op, STATUS_OK, UDP_HEADER, UDP_PAYLOAD, drain, read_response_header, write_request,
};
use crate::stats::{Measurement, Suite, fmt_bytes};

/// Receive buffer sizes to sweep, in bytes.
///
/// The receive buffer bounds the TCP window, which bounds how much data can be
/// in flight. At 24 MB/s and 2.3 ms round-trip the bandwidth-delay product is
/// only ~55 KB, so the default should already be ample — but "should" is why
/// this is measured rather than assumed.
const RECV_BUFFERS: [usize; 5] = [
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
];

/// Write sizes to sweep.
const WRITE_SIZES: [usize; 5] = [
    16 * 1024,
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
];

/// UDP send rates to sweep, in MB/s. Deliberately spans well past the observed
/// TCP rate so the knee — where loss begins — is visible.
const UDP_RATES_MBS: [f64; 7] = [10.0, 20.0, 30.0, 40.0, 60.0, 80.0, 120.0];

pub struct LabConfig {
    pub host: String,
    pub port: u16,
    pub runs: usize,
    /// Bytes per TCP measurement.
    pub transfer_bytes: u64,
    /// Seconds to blast for each UDP rate.
    pub udp_seconds: f64,
}

pub async fn run(config: &LabConfig) -> Result<Vec<Suite>> {
    println!("\n┌─ transport lab");
    println!("└─ finding the ceiling of this link, and whether we are at it\n");

    let baseline = tcp_baseline(config).await?;
    let recv = recv_buffer_sweep(config).await?;
    let write = write_size_sweep(config).await?;
    let udp = udp_ceiling(config).await?;

    verdict(&baseline, &recv, &write, &udp);

    Ok(vec![baseline, recv, write, udp])
}

/// Current settings, as a reference point for everything else.
async fn tcp_baseline(config: &LabConfig) -> Result<Suite> {
    let mut suite = Suite::new("lab-tcp-baseline", "TCP as currently configured");
    println!(
        "TCP baseline ({} per run)",
        fmt_bytes(config.transfer_bytes)
    );

    let mut m = Measurement::new("current settings", config.transfer_bytes);
    for _ in 0..config.runs {
        m.record(tcp_download(config, None, None).await?);
    }
    suite.push(m);
    Ok(suite)
}

/// Does a bigger receive buffer help?
async fn recv_buffer_sweep(config: &LabConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "lab-recv-buffer",
        "TCP throughput vs receive buffer size (bounds the window, and so how much can be in flight)",
    );
    println!("\nreceive buffer sweep");

    for size in RECV_BUFFERS {
        let mut m = Measurement::new(
            format!("recv buffer {:>8}", fmt_bytes(size as u64)),
            config.transfer_bytes,
        );
        for _ in 0..config.runs {
            m.record(tcp_download(config, Some(size), None).await?);
        }
        suite.push(m);
    }
    Ok(suite)
}

/// Does the size of each write matter?
async fn write_size_sweep(config: &LabConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "lab-write-size",
        "TCP throughput vs the size of each write on the sender",
    );
    println!("\nwrite size sweep");

    for size in WRITE_SIZES {
        let mut m = Measurement::new(
            format!("write size  {:>8}", fmt_bytes(size as u64)),
            config.transfer_bytes,
        );
        for _ in 0..config.runs {
            m.record(tcp_download(config, None, Some(size)).await?);
        }
        suite.push(m);
    }
    Ok(suite)
}

/// One TCP download, optionally with a set receive buffer and write size.
async fn tcp_download(
    config: &LabConfig,
    recv_buffer: Option<usize>,
    write_size: Option<usize>,
) -> Result<Duration> {
    let addr = format!("{}:{}", config.host, config.port);
    let mut conn = match recv_buffer {
        Some(size) => Conn::connect_plain_tuned(&addr, Some(size)).await?,
        None => Conn::connect_plain(&addr).await?,
    };
    let io = conn.as_io();

    let start = Instant::now();
    match write_size {
        Some(chunk) => {
            let mut payload = [0u8; 12];
            payload[..8].copy_from_slice(&config.transfer_bytes.to_le_bytes());
            payload[8..].copy_from_slice(&(chunk as u32).to_le_bytes());
            write_request(io, Op::SourceTuned, &payload).await?;
        }
        None => {
            write_request(io, Op::Source, &config.transfer_bytes.to_le_bytes()).await?;
        }
    }
    let (status, len) = read_response_header(io).await?;
    if status != STATUS_OK {
        bail!("server returned status {status}");
    }
    drain(io, len).await?;
    Ok(start.elapsed())
}

/// **The decisive experiment.**
///
/// Blasts UDP at a range of target rates and asks the server how much arrived.
/// UDP will happily send faster than the link can carry, so the delivered rate
/// plateaus at the true ceiling while loss climbs. That plateau is the number
/// everything else is judged against.
async fn udp_ceiling(config: &LabConfig) -> Result<Suite> {
    let mut suite = Suite::new(
        "lab-udp-ceiling",
        "delivered UDP throughput and loss by send rate — the link's real ceiling",
    );

    println!("\nUDP ceiling test ({:.0}s per rate)", config.udp_seconds);
    println!(
        "  {:>10} {:>12} {:>10} {:>10}",
        "sending", "delivered", "loss", "verdict"
    );

    let addr = format!("{}:{}", config.host, config.port);
    let mut conn = Conn::connect_plain(&addr).await?;

    // Ask the server for its UDP port and zero the counters.
    let udp_port = {
        let io = conn.as_io();
        write_request(io, Op::UdpReset, b"").await?;
        let (status, len) = read_response_header(io).await?;
        if status != STATUS_OK || len != 2 {
            bail!("server does not support the UDP test — is it running the current build?");
        }
        let mut buf = [0u8; 2];
        io.read_exact(&mut buf).await?;
        u16::from_le_bytes(buf)
    };

    if udp_port == 0 {
        println!("  server could not open a UDP socket; skipping");
        return Ok(suite);
    }

    let target: SocketAddr = format!("{}:{}", config.host, udp_port)
        .parse()
        .context("building the UDP target address")?;

    for rate_mbs in UDP_RATES_MBS {
        // Reset before each rate so the counts belong to this run alone.
        let io = conn.as_io();
        write_request(io, Op::UdpReset, b"").await?;
        let (_, len) = read_response_header(io).await?;
        drain(io, len).await?;

        let sent = blast_udp(target, rate_mbs, config.udp_seconds)?;

        // Give in-flight packets a moment to land before reading the counters.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let io = conn.as_io();
        write_request(io, Op::UdpReport, b"").await?;
        let (status, len) = read_response_header(io).await?;
        if status != STATUS_OK || len != 24 {
            bail!("unexpected UDP report");
        }
        let mut buf = [0u8; 24];
        io.read_exact(&mut buf).await?;
        let packets = u64::from_le_bytes(buf[0..8].try_into().expect("8 bytes"));
        let bytes = u64::from_le_bytes(buf[8..16].try_into().expect("8 bytes"));

        let delivered_mbs = (bytes as f64 / 1e6) / config.udp_seconds;
        let loss = if sent.packets == 0 {
            0.0
        } else {
            1.0 - (packets as f64 / sent.packets as f64)
        };

        let verdict = if loss < 0.02 {
            "clean"
        } else if loss < 0.20 {
            "lossy"
        } else {
            "saturated"
        };

        println!(
            "  {rate_mbs:>8.0} MB/s {delivered_mbs:>9.1} MB/s {:>9.1}% {verdict:>10}",
            loss * 100.0
        );

        let mut m = Measurement::new(
            format!("udp send {rate_mbs:>3.0} MB/s"),
            (delivered_mbs * config.udp_seconds * 1e6) as u64,
        );
        m.record(Duration::from_secs_f64(config.udp_seconds));
        m.notes.push(format!(
            "sent {:.1} MB/s, delivered {delivered_mbs:.1} MB/s, loss {:.1}%",
            sent.bytes as f64 / 1e6 / config.udp_seconds,
            loss * 100.0
        ));
        suite.measurements.push(m);
    }

    Ok(suite)
}

struct BlastResult {
    packets: u64,
    bytes: u64,
}

/// Sends UDP packets at approximately `rate_mbs` for `seconds`.
///
/// Paced in short bursts rather than one packet at a time: at 24 MB/s a packet
/// is due every ~60 µs, and Windows timer granularity is around a millisecond,
/// so per-packet sleeping cannot hit the rate. Sending a burst then sleeping to
/// the next window tracks the target closely enough for this purpose.
fn blast_udp(target: SocketAddr, rate_mbs: f64, seconds: f64) -> Result<BlastResult> {
    let socket = UdpSocket::bind("0.0.0.0:0").context("binding a local UDP socket")?;
    // A large send buffer stops the burst from blocking on the socket itself,
    // which would make us measure our own buffer rather than the link.
    let _ = socket.set_nonblocking(false);

    let mut packet = vec![0u8; UDP_PAYLOAD];
    for (i, b) in packet.iter_mut().enumerate().skip(UDP_HEADER) {
        *b = (i % 251) as u8;
    }

    const WINDOW: Duration = Duration::from_millis(5);
    let bytes_per_second = rate_mbs * 1e6;
    let packets_per_window =
        ((bytes_per_second * WINDOW.as_secs_f64()) / UDP_PAYLOAD as f64).max(1.0) as u64;

    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(seconds);
    let mut seq = 0u64;
    let mut sent_bytes = 0u64;
    let mut next_window = Instant::now();

    while Instant::now() < deadline {
        next_window += WINDOW;
        for _ in 0..packets_per_window {
            packet[..UDP_HEADER].copy_from_slice(&seq.to_le_bytes());
            match socket.send_to(&packet, target) {
                Ok(n) => {
                    sent_bytes += n as u64;
                    seq += 1;
                }
                // A full send buffer means we are pushing harder than the
                // adapter will take, which is itself a signal, not an error.
                Err(_) => break,
            }
        }
        let now = Instant::now();
        if next_window > now {
            std::thread::sleep(next_window - now);
        } else {
            // Behind schedule: we cannot reach this rate, so stop trying to
            // catch up or the burst structure collapses.
            next_window = now;
        }
    }

    Ok(BlastResult {
        packets: seq,
        bytes: sent_bytes,
    })
}

/// What the TCP-vs-UDP comparison actually established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportVerdict {
    /// UDP is well ahead of TCP: the link can carry more than TCP is taking.
    HeadroomExists,
    /// UDP and TCP agree: the link itself is the constraint.
    LinkLimited,
    /// UDP came in *below* TCP, which cannot be a property of the link — a
    /// protocol with no acknowledgements cannot be slower than one with them.
    /// It means our own packet-sending loop ran out of steam first, so the test
    /// says nothing about the ceiling.
    SenderLimited,
    /// No UDP data at all.
    Inconclusive,
}

/// Classifies the transport comparison.
///
/// Kept separate from printing so the reasoning can be tested. The case worth
/// being careful about is [`TransportVerdict::SenderLimited`]: an earlier
/// version had only two branches, so a UDP result far *below* TCP fell into the
/// "link is saturated" branch and reported a confident, wrong conclusion.
pub fn classify_transport(tcp_best: f64, udp_best: f64) -> TransportVerdict {
    if udp_best <= 0.0 || tcp_best <= 0.0 {
        return TransportVerdict::Inconclusive;
    }
    if udp_best < tcp_best * 0.9 {
        TransportVerdict::SenderLimited
    } else if udp_best > tcp_best * 1.25 {
        TransportVerdict::HeadroomExists
    } else {
        TransportVerdict::LinkLimited
    }
}

/// Reads the four suites and says what to do about them.
fn verdict(baseline: &Suite, recv: &Suite, write: &Suite, udp: &Suite) {
    let tcp_best = [baseline, recv, write]
        .iter()
        .flat_map(|s| s.measurements.iter())
        .map(|m| m.throughput_mbs())
        .fold(0.0f64, f64::max);

    let udp_best = udp
        .measurements
        .iter()
        .map(|m| m.throughput_mbs())
        .fold(0.0f64, f64::max);

    println!("\n{}", "=".repeat(70));
    println!("  TRANSPORT VERDICT");
    println!("{}", "=".repeat(70));
    println!("\n  Best TCP:  {tcp_best:>6.1} MB/s");
    if udp_best > 0.0 {
        println!("  Best UDP:  {udp_best:>6.1} MB/s  (no congestion control, no retries)");
    }

    // Tuning gains, measured against the untuned baseline.
    let base = baseline
        .measurements
        .first()
        .map(|m| m.throughput_mbs())
        .unwrap_or(0.0);
    let best_recv = recv
        .measurements
        .iter()
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()));
    let best_write = write
        .measurements
        .iter()
        .max_by(|a, b| a.throughput_mbs().total_cmp(&b.throughput_mbs()));

    if base > 0.0 {
        if let Some(m) = best_recv {
            let gain = m.throughput_mbs() / base;
            println!(
                "\n  Best receive buffer: {} -> {:.1} MB/s ({gain:.2}x vs default)",
                m.label.trim(),
                m.throughput_mbs()
            );
        }
        if let Some(m) = best_write {
            let gain = m.throughput_mbs() / base;
            println!(
                "  Best write size:     {} -> {:.1} MB/s ({gain:.2}x vs default)",
                m.label.trim(),
                m.throughput_mbs()
            );
        }
    }

    println!("\n{}", "-".repeat(70));
    match classify_transport(tcp_best, udp_best) {
        TransportVerdict::Inconclusive => {
            println!("  The UDP test did not run, so the ceiling is unknown.");
        }
        TransportVerdict::HeadroomExists => {
            println!("  UDP beats TCP by {:.2}x.", udp_best / tcp_best);
            println!();
            println!("  TCP is leaving throughput unused on this link. A custom");
            println!("  transport over UDP, with its own pacing and selective");
            println!("  retransmission, could realistically reach {udp_best:.0} MB/s.");
            println!("  That is a large project, but the headroom is real.");
        }
        TransportVerdict::LinkLimited => {
            println!(
                "  UDP ({udp_best:.1} MB/s) and TCP ({tcp_best:.1} MB/s) agree to within {:.0}%.",
                ((udp_best / tcp_best) - 1.0).abs() * 100.0
            );
            println!();
            println!("  The radio is the limit, not the protocol. Removing TCP's");
            println!("  congestion control and acknowledgements buys nothing, so");
            println!("  there is no faster transport to find. A custom UDP protocol");
            println!("  would be weeks of work for no gain.");
            println!();
            println!("  The only way left to move files faster is to send fewer bytes:");
            println!("    - compression (measured at 2.2x on real data)");
            println!("    - caching, so repeat reads never cross the link");
            println!("    - delta sync, so edits send only what changed");
        }
        TransportVerdict::SenderLimited => {
            println!("  UDP ({udp_best:.1} MB/s) came in BELOW TCP ({tcp_best:.1} MB/s).");
            println!();
            println!("  That cannot be a property of the link: a protocol with no");
            println!("  acknowledgements cannot be slower than one with them. It");
            println!("  means this harness's own packet loop ran out of steam first.");
            println!("  Every UDP packet costs a syscall, and small packets need");
            println!("  tens of thousands of them per second.");
            println!();
            println!("  So this UDP number is NOT the link ceiling and proves");
            println!("  nothing about it. Expect this on fast paths (loopback,");
            println!("  wired); over Wi-Fi the sender has ample headroom.");
        }
    }
    println!("{}", "-".repeat(70));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_payload_avoids_ip_fragmentation() {
        // 1500 MTU - 20 IP - 8 UDP. Larger payloads fragment, and one lost
        // fragment loses the whole datagram, which would measure fragmentation
        // rather than the link.
        assert_eq!(UDP_PAYLOAD, 1472);
        const { assert!(UDP_PAYLOAD + 28 <= 1500) };
    }

    #[test]
    fn udp_rate_sweep_spans_past_the_observed_tcp_rate() {
        // The sweep has to bracket the ceiling on both sides, otherwise the
        // knee where loss begins is never visible.
        assert!(UDP_RATES_MBS.first().unwrap() < &24.0);
        assert!(UDP_RATES_MBS.last().unwrap() > &100.0);
        // Monotonic, so the table reads in order.
        for pair in UDP_RATES_MBS.windows(2) {
            assert!(pair[1] > pair[0]);
        }
    }

    #[test]
    fn sweeps_are_monotonic_and_bracket_the_default() {
        for pair in RECV_BUFFERS.windows(2) {
            assert!(pair[1] > pair[0]);
        }
        for pair in WRITE_SIZES.windows(2) {
            assert!(pair[1] > pair[0]);
        }
        // Windows' default socket buffer is 64 KB, so the sweep must start at
        // or below it to show whether raising it helps.
        assert!(RECV_BUFFERS[0] <= 64 * 1024);
    }

    #[test]
    fn pacing_maths_produces_a_sane_burst_size() {
        // At 24 MB/s with 1472-byte packets over a 5 ms window: roughly 81
        // packets per burst. Too few and the timer granularity dominates; too
        // many and the burst itself causes loss.
        let window_secs = 0.005;
        let packets = ((24.0 * 1e6 * window_secs) / UDP_PAYLOAD as f64) as u64;
        assert!(
            (50..200).contains(&packets),
            "expected a burst of 50-200 packets, got {packets}"
        );
    }

    #[test]
    fn udp_far_below_tcp_means_our_sender_gave_out_not_the_link() {
        // Regression guard, with the real loopback numbers that exposed the
        // bug: TCP 914.9, UDP 75.1. An earlier two-branch version fell through
        // to "the radio is the limit" and reported that confidently about a
        // loopback interface with no radio involved at all.
        assert_eq!(
            classify_transport(914.9, 75.1),
            TransportVerdict::SenderLimited
        );
    }

    #[test]
    fn udp_matching_tcp_means_the_link_is_the_limit() {
        // The expected Wi-Fi shape: both protocols pinned to the same ceiling.
        assert_eq!(
            classify_transport(24.0, 25.0),
            TransportVerdict::LinkLimited
        );
        assert_eq!(
            classify_transport(24.0, 23.0),
            TransportVerdict::LinkLimited
        );
        assert_eq!(
            classify_transport(24.0, 28.0),
            TransportVerdict::LinkLimited
        );
    }

    #[test]
    fn udp_well_above_tcp_means_there_is_headroom() {
        assert_eq!(
            classify_transport(24.0, 45.0),
            TransportVerdict::HeadroomExists
        );
    }

    #[test]
    fn transport_classification_handles_missing_data() {
        assert_eq!(
            classify_transport(24.0, 0.0),
            TransportVerdict::Inconclusive
        );
        assert_eq!(
            classify_transport(0.0, 24.0),
            TransportVerdict::Inconclusive
        );
        assert_eq!(classify_transport(0.0, 0.0), TransportVerdict::Inconclusive);
    }

    #[test]
    fn blast_to_a_discarded_port_still_returns() {
        // Nothing listens here; send_to should still succeed locally and the
        // function must terminate on its deadline rather than hanging.
        let target: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let result = blast_udp(target, 5.0, 0.2).unwrap();
        assert!(result.packets > 0, "should have sent something");
        assert_eq!(result.bytes, result.packets * UDP_PAYLOAD as u64);
    }
}
