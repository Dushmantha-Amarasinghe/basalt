//! Network measurement protocol.
//!
//! A deliberately thin request/response protocol over TCP, optionally wrapped
//! in TLS 1.3. It is not the shipping protocol — Phase 1 uses HTTP/2 so we get
//! range requests and multiplexing — but it isolates the questions Phase 0
//! actually needs answered:
//!
//! - What is the raw ceiling of this link, with the disk entirely out of the
//!   picture? ([`Op::Source`])
//! - How many parallel streams does it take to reach that ceiling? A single
//!   TCP flow frequently cannot saturate Wi-Fi.
//! - What does TLS cost on the *laptop's* CPU, not on a desktop?
//! - How much does batching beat per-file requests for many small files?
//!   ([`Op::GetBatch`] vs [`Op::GetFile`])
//!
//! Wire format, both directions:
//!
//! ```text
//! request   [op: u8][len: u32 LE][payload: len bytes]
//! response  [status: u8][len: u64 LE][payload: len bytes]
//! ```

pub mod client;
pub mod server;

use anyhow::{Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Default port. Chosen to sit clear of anything common.
pub const DEFAULT_PORT: u16 = 7742;

/// Payload the server streams for [`Op::Source`], regenerated per chunk to keep
/// the server from becoming the bottleneck.
pub const CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    /// Empty request, empty response. Measures round-trip latency.
    Ping = 1,
    /// Payload is a u64: send me this many synthetic bytes. No disk involved,
    /// so this measures the link and the crypto, nothing else.
    Source = 2,
    /// Payload is a u64 followed by that many bytes. Measures upload.
    Sink = 3,
    /// Payload is a UTF-8 relative path. Responds with the file's bytes.
    GetFile = 4,
    /// Payload is a JSON `BatchRequest`. Responds with a basalt-proto batch
    /// stream covering every requested path.
    GetBatch = 5,
}

impl Op {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Op::Ping,
            2 => Op::Source,
            3 => Op::Sink,
            4 => Op::GetFile,
            5 => Op::GetBatch,
            other => bail!("unknown opcode {other}"),
        })
    }
}

pub const STATUS_OK: u8 = 0;
pub const STATUS_ERR: u8 = 1;

pub async fn write_request<W: AsyncWrite + Unpin + ?Sized>(
    w: &mut W,
    op: Op,
    payload: &[u8],
) -> Result<()> {
    let mut head = [0u8; 5];
    head[0] = op as u8;
    head[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    w.write_all(&head).await?;
    if !payload.is_empty() {
        w.write_all(payload).await?;
    }
    w.flush().await?;
    Ok(())
}

pub async fn read_request<R: AsyncRead + Unpin + ?Sized>(r: &mut R) -> Result<(Op, Vec<u8>)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head).await?;
    let op = Op::from_u8(head[0])?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    // Requests are manifests and paths, never bulk data. Cap generously but
    // finitely so a bad length cannot drive an unbounded allocation.
    if len > 64 * 1024 * 1024 {
        bail!("request payload of {len} bytes exceeds the limit");
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok((op, payload))
}

pub async fn write_response_header<W: AsyncWrite + Unpin + ?Sized>(
    w: &mut W,
    status: u8,
    len: u64,
) -> Result<()> {
    let mut head = [0u8; 9];
    head[0] = status;
    head[1..9].copy_from_slice(&len.to_le_bytes());
    w.write_all(&head).await?;
    Ok(())
}

pub async fn read_response_header<R: AsyncRead + Unpin + ?Sized>(r: &mut R) -> Result<(u8, u64)> {
    let mut head = [0u8; 9];
    r.read_exact(&mut head).await?;
    let len = u64::from_le_bytes(head[1..9].try_into().expect("slice is 8 bytes"));
    Ok((head[0], len))
}

/// Reads and discards exactly `len` bytes, returning how long that took.
///
/// Discarding rather than collecting is deliberate: for a multi-gigabyte
/// throughput test we want to measure the network, not the allocator, and not
/// run the machine out of memory.
pub async fn drain<R: AsyncRead + Unpin + ?Sized>(r: &mut R, len: u64) -> Result<u64> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut remaining = len;
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = r.read(&mut buf[..want]).await?;
        if n == 0 {
            bail!("peer closed with {remaining} bytes still outstanding");
        }
        remaining -= n as u64;
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcodes_round_trip() {
        for op in [Op::Ping, Op::Source, Op::Sink, Op::GetFile, Op::GetBatch] {
            assert_eq!(Op::from_u8(op as u8).unwrap(), op);
        }
    }

    #[test]
    fn unknown_opcode_is_rejected() {
        assert!(Op::from_u8(200).is_err());
        assert!(Op::from_u8(0).is_err());
    }

    #[tokio::test]
    async fn request_round_trips_over_a_pipe() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_request(&mut a, Op::GetFile, b"large/prose.txt")
            .await
            .unwrap();
        let (op, payload) = read_request(&mut b).await.unwrap();
        assert_eq!(op, Op::GetFile);
        assert_eq!(payload, b"large/prose.txt");
    }

    #[tokio::test]
    async fn empty_request_round_trips() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_request(&mut a, Op::Ping, b"").await.unwrap();
        let (op, payload) = read_request(&mut b).await.unwrap();
        assert_eq!(op, Op::Ping);
        assert!(payload.is_empty());
    }

    #[tokio::test]
    async fn response_header_round_trips() {
        let (mut a, mut b) = tokio::io::duplex(64);
        write_response_header(&mut a, STATUS_OK, 1 << 40)
            .await
            .unwrap();
        let (status, len) = read_response_header(&mut b).await.unwrap();
        assert_eq!(status, STATUS_OK);
        assert_eq!(len, 1 << 40);
    }

    #[tokio::test]
    async fn drain_consumes_exactly_the_declared_length() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let writer = tokio::spawn(async move {
            a.write_all(&vec![7u8; 100_000]).await.unwrap();
            // Trailing byte must survive: drain must not over-read.
            a.write_all(&[0xFF]).await.unwrap();
            a
        });
        let got = drain(&mut b, 100_000).await.unwrap();
        assert_eq!(got, 100_000);
        let mut tail = [0u8; 1];
        b.read_exact(&mut tail).await.unwrap();
        assert_eq!(tail[0], 0xFF);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn drain_errors_when_the_peer_closes_early() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            a.write_all(&[0u8; 100]).await.unwrap();
            drop(a);
        });
        assert!(drain(&mut b, 100_000).await.is_err());
    }
}
