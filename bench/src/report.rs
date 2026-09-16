//! Report output.
//!
//! Phase 0 has a gate attached to it: match or beat SMB, or revisit the whole
//! approach. That means results have to be durable and comparable across runs,
//! not scrollback that disappears. Every run writes both a machine-readable
//! JSON file and a human-readable Markdown summary, each stamped with the
//! machine it ran on — numbers from a different CPU or a different radio are
//! not comparable and the report should make that obvious.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::stats::{Suite, fmt_duration_ms};

#[derive(Debug, Clone, Serialize)]
pub struct Environment {
    pub hostname: String,
    pub os: String,
    pub cpu: String,
    pub cores: usize,
    pub ram_gb: f64,
    pub rustc: String,
    /// Wi-Fi band/rate/signal when the machine is on wireless. `None` on wired
    /// or when the query fails.
    pub wifi: Option<WifiLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WifiLink {
    pub ssid: String,
    pub radio_type: String,
    pub band: String,
    pub receive_mbps: u32,
    pub transmit_mbps: u32,
    pub signal_percent: u32,
    pub channel: String,
}

impl WifiLink {
    /// Rough one-way ceiling implied by the link rate.
    ///
    /// Wi-Fi is half duplex, so real one-way throughput lands near 50–60% of
    /// the negotiated rate before any other client competes for airtime.
    pub fn estimated_ceiling_mbs(&self) -> f64 {
        (self.transmit_mbps.min(self.receive_mbps) as f64 * 0.55) / 8.0
    }

    /// Advice worth acting on, or `None` if the link is healthy or unknown.
    ///
    /// Every check here has to distinguish *bad* from *unknown*. Without
    /// Location permission the signal strength is not merely low, it is
    /// unavailable, and it arrives as 0. An earlier version read that 0 as a
    /// terrible signal and told the user to move closer to the router — advice
    /// derived entirely from missing data.
    pub fn advice(&self) -> Option<String> {
        if self.band.contains("2.4") {
            return Some(format!(
                "On 2.4 GHz at {} Mbps. Moving to 5 GHz typically gives 3-5x \
                 the throughput and is the single biggest change available.",
                self.transmit_mbps
            ));
        }
        // 0 means "not measured", not "no signal". Only advise when there is a
        // real reading.
        if self.signal_percent > 0 && self.signal_percent < 60 {
            return Some(format!(
                "Signal is {}%. Moving the machine closer to the router, or \
                 away from obstructions, will raise the link rate.",
                self.signal_percent
            ));
        }
        if self.transmit_mbps > 0 && self.transmit_mbps < 400 {
            return Some(format!(
                "Link rate is only {} Mbps, well below what this adapter can do. \
                 Moving closer to the router, or onto a wider channel (80 or 160 \
                 MHz), would raise it.",
                self.transmit_mbps
            ));
        }
        None
    }
}

impl Environment {
    pub fn detect() -> Self {
        Self {
            hostname: env_var("COMPUTERNAME").unwrap_or_else(|| "unknown".into()),
            os: powershell("(Get-CimInstance Win32_OperatingSystem).Caption")
                .unwrap_or_else(|| std::env::consts::OS.into()),
            cpu: powershell("(Get-CimInstance Win32_Processor).Name")
                .unwrap_or_else(|| "unknown".into()),
            cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0),
            ram_gb: powershell(
                "[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1)",
            )
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
            rustc: option_env!("CARGO_PKG_RUST_VERSION")
                .unwrap_or("unknown")
                .into(),
            wifi: detect_wifi(),
        }
    }
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn powershell(script: &str) -> Option<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Describes the wireless link without requiring any permission.
///
/// `netsh wlan show interfaces` is the obvious source, but on Windows 11 it is
/// gated behind **Location services** — SSID and BSSID can locate a machine, so
/// reading them needs the same consent as GPS. Without it the command fails
/// with `WlanQueryInterface returns error 5: Access is denied`, which is what
/// laptop B did.
///
/// Asking someone to enable Location just to read a link speed is exactly the
/// settings-fiddling this project ruled out after the SMB experience. So the
/// primary source is `Get-NetAdapter`, which needs no permission and still
/// yields the two things that matter: the negotiated rate and the adapter
/// model. The richer netsh fields (SSID, band, channel, signal) are layered on
/// top only when they happen to be available.
fn detect_wifi() -> Option<WifiLink> {
    let adapter = detect_adapter();

    let netsh = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_netsh_interfaces(&String::from_utf8_lossy(&o.stdout)));

    match (adapter, netsh) {
        // Both available: netsh carries the richer detail, but the adapter's
        // negotiated rate is the more reliable of the two figures.
        (Some(a), Some(mut n)) => {
            if a.transmit_mbps > 0 {
                n.transmit_mbps = a.transmit_mbps;
                n.receive_mbps = a.transmit_mbps;
            }
            Some(n)
        }
        (Some(a), None) => Some(a),
        (None, Some(n)) => Some(n),
        (None, None) => None,
    }
}

/// Reads the wireless adapter via `Get-NetAdapter`, which needs no permission.
///
/// Gives the negotiated link rate and the adapter description. The description
/// usually names the Wi-Fi generation ("Wi-Fi 6 AX201", "Wireless-AC 9560"),
/// which is enough to tell whether the radio is the weak link.
fn detect_adapter() -> Option<WifiLink> {
    let script = "Get-NetAdapter -Physical -ErrorAction SilentlyContinue | \
         Where-Object { $_.Status -eq 'Up' -and \
         ($_.InterfaceDescription -match 'Wi-?Fi|Wireless|802\\.11|WLAN') } | \
         Select-Object -First 1 | \
         ForEach-Object { \"$($_.InterfaceDescription)|$($_.LinkSpeed)\" }";

    let mut link = parse_adapter_line(&powershell(script)?)?;

    // `Get-NetAdapter` reports the adapter's *maximum* PHY rate, not the rate
    // currently negotiated. On laptop B it claimed 866 Mbps while Windows
    // Settings showed the real link running at 585/325 Mbps — a 2.7x
    // overstatement, and the sort of error that makes a throughput estimate
    // look broken when it is the input that is wrong.
    //
    // MSNdis_LinkSpeed carries the live rate and, unlike `netsh wlan`, needs no
    // Location permission.
    if let Some(live) = live_link_speed_mbps() {
        link.receive_mbps = live;
        link.transmit_mbps = live;
    }
    Some(link)
}

/// Current negotiated link rate in Mbps, via WMI. No permission required.
fn live_link_speed_mbps() -> Option<u32> {
    // Wi-Fi Direct virtual adapters share the name and report a fixed 54 Mbps,
    // which would otherwise be picked up instead of the real radio.
    let script = "Get-CimInstance -Namespace root/wmi -ClassName MSNdis_LinkSpeed \
         -ErrorAction SilentlyContinue | \
         Where-Object { $_.InstanceName -match 'Wi-?Fi|Wireless|802\\.11' -and \
         $_.InstanceName -notmatch 'Virtual|Direct' } | \
         Select-Object -First 1 -ExpandProperty NdisLinkSpeed";

    let raw = powershell(script)?;
    // NdisLinkSpeed is in units of 100 bits per second.
    let hundred_bps: u64 = raw.trim().parse().ok()?;
    let mbps = (hundred_bps * 100) / 1_000_000;
    if mbps == 0 { None } else { Some(mbps as u32) }
}

/// Parses `"Intel(R) Wi-Fi 6 AX201 160MHz|866.7 Mbps"`.
fn parse_adapter_line(raw: &str) -> Option<WifiLink> {
    let (description, speed) = raw.trim().split_once('|')?;
    if description.trim().is_empty() {
        return None;
    }
    let mbps = parse_link_speed(speed);
    Some(WifiLink {
        ssid: "unknown (needs Location permission)".into(),
        radio_type: infer_radio_type(description),
        band: "unknown".into(),
        receive_mbps: mbps,
        transmit_mbps: mbps,
        signal_percent: 0,
        channel: String::new(),
    })
}

/// `Get-NetAdapter` reports link speed as "866.7 Mbps" or "1.2 Gbps".
fn parse_link_speed(raw: &str) -> u32 {
    let Some((number, unit)) = raw.trim().split_once(' ') else {
        return 0;
    };
    let Ok(value) = number.trim().parse::<f64>() else {
        return 0;
    };
    let unit = unit.trim().to_ascii_lowercase();
    let mbps = if unit.starts_with("gbps") {
        value * 1000.0
    } else if unit.starts_with("mbps") {
        value
    } else if unit.starts_with("kbps") {
        value / 1000.0
    } else {
        return 0;
    };
    mbps as u32
}

/// Guesses the Wi-Fi generation from the adapter's model string.
fn infer_radio_type(description: &str) -> String {
    let d = description.to_ascii_lowercase();
    // Newest first: "Wi-Fi 6E AX211" contains both "ax2" and "6e", and the
    // more specific answer is the useful one.
    if d.contains("be200") || d.contains("wi-fi 7") {
        "802.11be (Wi-Fi 7)".into()
    } else if d.contains("6e") {
        "802.11ax (Wi-Fi 6E)".into()
    } else if d.contains("wi-fi 6") || d.contains("ax2") || d.contains("ax1") {
        "802.11ax (Wi-Fi 6)".into()
    } else if d.contains("wireless-ac") || d.contains("802.11ac") || d.contains("ac 9") {
        "802.11ac (Wi-Fi 5)".into()
    } else if d.contains("802.11n") || d.contains("wireless-n") {
        "802.11n (Wi-Fi 4)".into()
    } else {
        description.trim().to_string()
    }
}

fn parse_netsh_interfaces(text: &str) -> Option<WifiLink> {
    let field = |name: &str| -> Option<String> {
        text.lines()
            .find(|l| {
                let trimmed = l.trim_start();
                trimmed
                    .to_ascii_lowercase()
                    .starts_with(&name.to_ascii_lowercase())
                    && trimmed.contains(':')
            })
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };

    // Rates arrive as "866.7" and signal as "92%", so parse as f64 after
    // stripping the suffix rather than going straight to u32.
    let num = |name: &str| -> u32 {
        field(name)
            .and_then(|v| {
                v.split_whitespace()
                    .next()
                    .and_then(|n| n.trim_end_matches('%').parse::<f64>().ok())
            })
            .map(|f| f as u32)
            .unwrap_or(0)
    };

    // "State : connected" must be present, otherwise there is no live link.
    let state = field("State")?;
    if !state.eq_ignore_ascii_case("connected") {
        return None;
    }

    let channel = field("Channel").unwrap_or_default();
    let band = field("Band").unwrap_or_else(|| {
        // Older Windows builds omit "Band"; infer it from the channel number.
        match channel.parse::<u32>() {
            Ok(c) if c <= 14 => "2.4 GHz".into(),
            Ok(c) if c >= 36 => "5 GHz".into(),
            _ => "unknown".into(),
        }
    });

    Some(WifiLink {
        ssid: field("SSID").unwrap_or_else(|| "unknown".into()),
        radio_type: field("Radio type").unwrap_or_else(|| "unknown".into()),
        band,
        receive_mbps: num("Receive rate"),
        transmit_mbps: num("Transmit rate"),
        signal_percent: num("Signal"),
        channel,
    })
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub generated_at: String,
    pub environment: Environment,
    pub suites: Vec<Suite>,
}

impl Report {
    pub fn new(suites: Vec<Suite>) -> Self {
        Self {
            generated_at: timestamp(),
            environment: Environment::detect(),
            suites,
        }
    }

    pub fn write(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;

        let json_path = dir.join("benchmarks.json");
        fs::write(&json_path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", json_path.display()))?;

        let md_path = dir.join("benchmarks.md");
        fs::write(&md_path, self.to_markdown())
            .with_context(|| format!("writing {}", md_path.display()))?;

        println!("\nwrote {}", md_path.display());
        println!("wrote {}", json_path.display());
        Ok(())
    }

    fn to_markdown(&self) -> String {
        let env = &self.environment;
        let mut s = String::new();

        s.push_str("# Basalt — Phase 0 benchmarks\n\n");
        s.push_str(&format!("_Generated {}_\n\n", self.generated_at));

        s.push_str("## Machine\n\n");
        s.push_str("| | |\n|---|---|\n");
        s.push_str(&format!("| Host | {} |\n", env.hostname));
        s.push_str(&format!("| OS | {} |\n", env.os));
        s.push_str(&format!("| CPU | {} ({} threads) |\n", env.cpu, env.cores));
        s.push_str(&format!("| RAM | {:.1} GB |\n", env.ram_gb));

        match &env.wifi {
            Some(w) => {
                s.push_str(&format!(
                    "| Wi-Fi | {} · {} · {} · tx {} Mbps / rx {} Mbps · signal {}% |\n",
                    w.ssid, w.radio_type, w.band, w.transmit_mbps, w.receive_mbps, w.signal_percent
                ));
                s.push_str(&format!(
                    "| Implied one-way ceiling | ~{:.0} MB/s |\n",
                    w.estimated_ceiling_mbs()
                ));
            }
            None => s.push_str("| Wi-Fi | not connected (wired, or query failed) |\n"),
        }
        s.push('\n');

        if let Some(advice) = env.wifi.as_ref().and_then(|w| w.advice()) {
            s.push_str(&format!("> **Link advice:** {advice}\n\n"));
        }

        for suite in &self.suites {
            s.push_str(&format!("## {}\n\n{}\n\n", suite.name, suite.description));

            let has_throughput = suite.measurements.iter().any(|m| m.bytes > 0);
            let has_items = suite.measurements.iter().any(|m| m.items > 0);

            s.push_str("| Measurement | Median |");
            if has_throughput {
                s.push_str(" Throughput | Ratio |");
            }
            if has_items {
                s.push_str(" Items/s |");
            }
            s.push_str(" p95 | Runs |\n|---|---|");
            if has_throughput {
                s.push_str("---|---|");
            }
            if has_items {
                s.push_str("---|");
            }
            s.push_str("---|---|\n");

            for m in &suite.measurements {
                s.push_str(&format!(
                    "| {} | {} |",
                    m.label,
                    fmt_duration_ms(m.median_ms())
                ));
                if has_throughput {
                    if m.bytes > 0 {
                        s.push_str(&format!(" {:.1} MB/s |", m.throughput_mbs()));
                        let r = m.compression_ratio();
                        if r > 1.001 {
                            s.push_str(&format!(" {r:.2}x |"));
                        } else {
                            s.push_str(" — |");
                        }
                    } else {
                        s.push_str(" — | — |");
                    }
                }
                if has_items {
                    if m.items > 0 {
                        s.push_str(&format!(" {:.0} |", m.items_per_sec()));
                    } else {
                        s.push_str(" — |");
                    }
                }
                s.push_str(&format!(
                    " {} | {} |\n",
                    fmt_duration_ms(m.p95_ms()),
                    m.runs()
                ));
            }
            s.push('\n');

            let notes: Vec<&String> = suite
                .measurements
                .iter()
                .flat_map(|m| m.notes.iter())
                .collect();
            if !notes.is_empty() {
                s.push_str("<details><summary>Notes</summary>\n\n");
                for (m, note) in suite
                    .measurements
                    .iter()
                    .flat_map(|m| m.notes.iter().map(move |n| (m, n)))
                {
                    s.push_str(&format!("- **{}** — {note}\n", m.label));
                }
                s.push_str("\n</details>\n\n");
            }
        }

        s.push_str("---\n\n");
        s.push_str(
            "Numbers are medians over repeated runs. Compare only against runs \
             on the same machines and the same radio conditions.\n",
        );
        s
    }
}

fn timestamp() -> String {
    powershell("Get-Date -Format 'yyyy-MM-dd HH:mm:ss'").unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
There is 1 interface on the system:

    Name                   : Wi-Fi
    Description            : Intel(R) Wi-Fi 6 AX201 160MHz
    GUID                   : abc
    Physical address       : 00:11:22:33:44:55
    Interface type         : Primary
    State                  : connected
    SSID                   : HomeNetwork
    BSSID                  : aa:bb:cc:dd:ee:ff
    Network type           : Infrastructure
    Radio type             : 802.11ax
    Authentication         : WPA2-Personal
    Cipher                 : CCMP
    Connection mode        : Profile
    Band                   : 5 GHz
    Channel                : 44
    Receive rate (Mbps)    : 866.7
    Transmit rate (Mbps)   : 866.7
    Signal                 : 92%
    Profile                : HomeNetwork
"#;

    #[test]
    fn unknown_signal_produces_no_advice() {
        // Without Location permission the signal arrives as 0 because it was
        // never measured. An earlier version read that as "no signal" and told
        // the user to move closer to the router - advice invented entirely
        // from missing data.
        let w = WifiLink {
            ssid: "unknown".into(),
            radio_type: "802.11ac (Wi-Fi 5)".into(),
            band: "unknown".into(),
            receive_mbps: 866,
            transmit_mbps: 866,
            signal_percent: 0,
            channel: String::new(),
        };
        assert_eq!(
            w.advice(),
            None,
            "a missing signal reading must not be reported as a weak signal"
        );
    }

    #[test]
    fn a_genuinely_weak_signal_still_produces_advice() {
        let w = WifiLink {
            ssid: "x".into(),
            radio_type: "802.11ac".into(),
            band: "5 GHz".into(),
            receive_mbps: 866,
            transmit_mbps: 866,
            signal_percent: 35,
            channel: "36".into(),
        };
        assert!(w.advice().unwrap().contains("35%"));
    }

    #[test]
    fn unknown_link_rate_produces_no_advice() {
        let w = WifiLink {
            ssid: "x".into(),
            radio_type: "x".into(),
            band: "unknown".into(),
            receive_mbps: 0,
            transmit_mbps: 0,
            signal_percent: 0,
            channel: String::new(),
        };
        assert_eq!(w.advice(), None, "0 Mbps means unmeasured, not broken");
    }

    #[test]
    fn a_low_link_rate_produces_advice() {
        // Laptop B's real transmit rate, from Windows Settings.
        let w = WifiLink {
            ssid: "x".into(),
            radio_type: "802.11ac (Wi-Fi 5)".into(),
            band: "5 GHz".into(),
            receive_mbps: 585,
            transmit_mbps: 325,
            signal_percent: 0,
            channel: "104".into(),
        };
        assert!(w.advice().unwrap().contains("325"));
    }

    #[test]
    fn adapter_line_parses_without_any_permission() {
        // The path that matters on Windows 11: netsh needs Location consent,
        // Get-NetAdapter does not. Laptop B returned "Access is denied" from
        // netsh, so this is the only source that works there.
        let w =
            parse_adapter_line("Intel(R) Wi-Fi 6 AX201 160MHz|866.7 Mbps").expect("should parse");
        assert_eq!(w.transmit_mbps, 866);
        assert_eq!(w.receive_mbps, 866);
        assert_eq!(w.radio_type, "802.11ax (Wi-Fi 6)");
    }

    #[test]
    fn link_speed_handles_every_unit_windows_uses() {
        assert_eq!(parse_link_speed("866.7 Mbps"), 866);
        assert_eq!(parse_link_speed("1.2 Gbps"), 1200);
        assert_eq!(parse_link_speed("2.4 Gbps"), 2400);
        assert_eq!(parse_link_speed("100 Mbps"), 100);
        assert_eq!(parse_link_speed("540 Kbps"), 0);
        // Garbage must be 0, never a panic or a wild number.
        assert_eq!(parse_link_speed(""), 0);
        assert_eq!(parse_link_speed("fast"), 0);
        assert_eq!(parse_link_speed("866.7"), 0);
        assert_eq!(parse_link_speed("abc Mbps"), 0);
    }

    #[test]
    fn radio_generation_is_inferred_from_the_adapter_name() {
        let cases = [
            ("Intel(R) Wi-Fi 6 AX201 160MHz", "802.11ax (Wi-Fi 6)"),
            ("Intel(R) Wi-Fi 6E AX211 160MHz", "802.11ax (Wi-Fi 6E)"),
            ("Intel(R) Wireless-AC 9560", "802.11ac (Wi-Fi 5)"),
            ("Realtek 802.11n Wireless LAN", "802.11n (Wi-Fi 4)"),
            ("Intel(R) BE200 320MHz", "802.11be (Wi-Fi 7)"),
        ];
        for (description, expected) in cases {
            assert_eq!(infer_radio_type(description), expected, "for {description}");
        }
    }

    #[test]
    fn an_unrecognised_adapter_keeps_its_own_name() {
        // Better to show the raw model than to guess wrong about it.
        assert_eq!(
            infer_radio_type("Acme SuperWave 9000"),
            "Acme SuperWave 9000"
        );
    }

    #[test]
    fn a_malformed_adapter_line_is_rejected() {
        assert!(parse_adapter_line("").is_none());
        assert!(parse_adapter_line("no pipe here").is_none());
        assert!(parse_adapter_line("|866.7 Mbps").is_none());
    }

    #[test]
    fn an_adapter_with_an_unreadable_speed_still_reports_its_model() {
        // Losing the rate should not lose the adapter name too — the name
        // alone often answers "is this radio the weak link".
        let w = parse_adapter_line("Intel(R) Wireless-AC 9560|Unknown").expect("should parse");
        assert_eq!(w.transmit_mbps, 0);
        assert_eq!(w.radio_type, "802.11ac (Wi-Fi 5)");
    }

    #[test]
    fn parses_a_connected_wifi_interface() {
        let w = parse_netsh_interfaces(SAMPLE).expect("should parse");
        assert_eq!(w.ssid, "HomeNetwork");
        assert_eq!(w.band, "5 GHz");
        assert_eq!(w.channel, "44");
        assert_eq!(w.transmit_mbps, 866);
        assert_eq!(w.receive_mbps, 866);
        assert_eq!(w.signal_percent, 92);
        assert_eq!(w.radio_type, "802.11ax");
    }

    #[test]
    fn disconnected_interface_yields_no_link() {
        let text = SAMPLE.replace(
            "State                  : connected",
            "State                  : disconnected",
        );
        assert!(parse_netsh_interfaces(&text).is_none());
    }

    #[test]
    fn missing_state_yields_no_link() {
        assert!(parse_netsh_interfaces("garbage output").is_none());
    }

    #[test]
    fn band_is_inferred_from_channel_when_absent() {
        let text = SAMPLE.replace("    Band                   : 5 GHz\n", "");
        let w = parse_netsh_interfaces(&text).expect("should still parse");
        assert_eq!(w.band, "5 GHz", "channel 44 implies the 5 GHz band");

        let text24 = text.replace("Channel                : 44", "Channel                : 6");
        let w24 = parse_netsh_interfaces(&text24).expect("should still parse");
        assert_eq!(w24.band, "2.4 GHz", "channel 6 implies the 2.4 GHz band");
    }

    #[test]
    fn ceiling_estimate_halves_the_link_rate_and_converts_to_bytes() {
        let w = parse_netsh_interfaces(SAMPLE).unwrap();
        let ceiling = w.estimated_ceiling_mbs();
        // 866 Mbps * 0.55 / 8 ≈ 59.5 MB/s
        assert!(
            (55.0..65.0).contains(&ceiling),
            "expected ~60 MB/s, got {ceiling:.1}"
        );
    }

    #[test]
    fn advice_flags_the_24ghz_band_first() {
        let text = SAMPLE.replace(
            "Band                   : 5 GHz",
            "Band                   : 2.4 GHz",
        );
        let w = parse_netsh_interfaces(&text).unwrap();
        let advice = w.advice().expect("2.4 GHz should always produce advice");
        assert!(
            advice.contains("5 GHz"),
            "advice should point at 5 GHz: {advice}"
        );
    }

    #[test]
    fn advice_flags_a_weak_signal() {
        let text = SAMPLE.replace(
            "Signal                 : 92%",
            "Signal                 : 40%",
        );
        let w = parse_netsh_interfaces(&text).unwrap();
        assert!(w.advice().unwrap().contains("40%"));
    }

    #[test]
    fn a_healthy_link_gets_no_advice() {
        let w = parse_netsh_interfaces(SAMPLE).unwrap();
        assert!(w.advice().is_none(), "a strong 5 GHz link needs no advice");
    }

    #[test]
    fn markdown_renders_without_panicking_on_empty_suites() {
        let report = Report {
            generated_at: "now".into(),
            environment: Environment {
                hostname: "h".into(),
                os: "o".into(),
                cpu: "c".into(),
                cores: 8,
                ram_gb: 16.0,
                rustc: "1.98".into(),
                wifi: None,
            },
            suites: vec![Suite::new("empty", "nothing here")],
        };
        let md = report.to_markdown();
        assert!(md.contains("# Basalt"));
        assert!(md.contains("not connected"));
    }
}
