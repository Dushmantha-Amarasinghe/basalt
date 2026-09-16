//! A local HTTP server that makes a file on the host look like a local URL.
//!
//! This is how video and music play. A `<video>` element cannot speak the
//! Basalt protocol, but it is very good at HTTP range requests — which is
//! exactly the shape of [`Session::read_range`](crate::Session::read_range). So
//! the proxy listens on loopback, translates each range request into a ranged
//! read, and hands the player back a perfectly ordinary `206 Partial Content`.
//!
//! Seeking then works for free: dragging the scrubber makes the player ask for
//! a different range, which becomes a read at a different offset, which the
//! host answers by seeking the file. Nothing is downloaded ahead of the
//! playhead and nothing is cached on disk.
//!
//! The same URL is what an mpv sidecar would use later for the formats WebView2
//! cannot decode, so none of this is throwaway.
//!
//! Two deliberate restrictions, because this opens a door onto the whole vault:
//!
//! - It binds to `127.0.0.1` only, never `0.0.0.0`. Nothing off this machine
//!   can reach it.
//! - Every URL carries a random token generated at startup. Without it any
//!   other program on this computer — including a web page in a browser —
//!   could read the drive through it.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::client::Basalt;
use crate::{ClientError, Result};

/// Bytes served per read when the player does not ask for a bounded range.
///
/// A player that opens with `Range: bytes=0-` wants the whole file, but
/// answering literally would mean buffering it. Serving a window at a time
/// keeps memory flat and the player simply asks again.
const WINDOW_BYTES: u64 = 4 * 1024 * 1024;

pub struct MediaProxy {
    addr: SocketAddr,
    token: String,
}

impl MediaProxy {
    /// Starts the proxy on a port the OS chooses.
    pub async fn start(client: Arc<Basalt>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let token = basalt_net::pairing::random_token()
            .map_err(|e| ClientError::Protocol(format!("no randomness available: {e}")))?;

        let accept_token = token.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let client = Arc::clone(&client);
                let token = accept_token.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, client, token).await {
                        tracing::debug!("media proxy: {e}");
                    }
                });
            }
        });

        Ok(Self { addr, token })
    }

    /// The URL a player should open for a vault path.
    pub fn url_for(&self, path: &str) -> String {
        format!(
            "http://{}/{}/{}",
            self.addr,
            self.token,
            percent_encode(path)
        )
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn handle(stream: TcpStream, client: Arc<Basalt>, token: String) -> Result<()> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut range_header = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range_header = Some(value.trim().to_string());
        }
    }

    let mut stream = reader.into_inner();

    if method != "GET" && method != "HEAD" {
        return respond_status(&mut stream, 405, "Method Not Allowed").await;
    }

    let Some(path) = authorise(&target, &token) else {
        // Deliberately indistinguishable from a missing file: a wrong token
        // should not confirm that the right one would have worked.
        return respond_status(&mut stream, 404, "Not Found").await;
    };

    let entry = match client.stat(&path).await {
        Ok(entry) => entry,
        Err(e) => {
            tracing::debug!("media proxy could not stat {path}: {e}");
            return respond_status(&mut stream, 404, "Not Found").await;
        }
    };
    let total = entry.size;

    let (start, end) = match &range_header {
        Some(header) => match parse_range(header, total) {
            Some(range) => range,
            // A range that cannot be satisfied has its own status, and players
            // rely on it to discover a file's length.
            None => {
                let head = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\n\
                     Content-Range: bytes */{total}\r\n\
                     Content-Length: 0\r\n\
                     Connection: close\r\n\r\n"
                );
                stream.write_all(head.as_bytes()).await?;
                return Ok(());
            }
        },
        None => (0, total.saturating_sub(1)),
    };

    let length = if total == 0 { 0 } else { end - start + 1 };
    let content_type = content_type_for(&path);

    let head = if range_header.is_some() {
        format!(
            "HTTP/1.1 206 Partial Content\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {length}\r\n\
             Content-Range: bytes {start}-{end}/{total}\r\n\
             Accept-Ranges: bytes\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {length}\r\n\
             Accept-Ranges: bytes\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\r\n"
        )
    };
    stream.write_all(head.as_bytes()).await?;

    if method == "HEAD" || length == 0 {
        return Ok(());
    }

    let mut sent = 0u64;
    while sent < length {
        let want = WINDOW_BYTES.min(length - sent);
        let data = match client.read_range(&path, start + sent, want).await {
            Ok(data) => data,
            Err(e) => {
                // The header is already on the wire, so there is no way to turn
                // this into a status. Closing the connection is what tells the
                // player something went wrong.
                tracing::debug!("media proxy read failed at {}: {e}", start + sent);
                return Ok(());
            }
        };
        if data.is_empty() {
            break;
        }
        // A player that seeks away closes the connection mid-write. That is
        // normal behaviour, not an error worth reporting.
        if stream.write_all(&data).await.is_err() {
            return Ok(());
        }
        sent += data.len() as u64;
    }
    stream.flush().await.ok();
    Ok(())
}

async fn respond_status(stream: &mut TcpStream, code: u16, reason: &str) -> Result<()> {
    let head =
        format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    Ok(())
}

/// Checks the token and returns the vault path it guards.
fn authorise(target: &str, token: &str) -> Option<String> {
    let trimmed = target.strip_prefix('/')?;
    // Query strings are not part of the path; players append them freely.
    let trimmed = trimmed.split('?').next().unwrap_or(trimmed);
    let (presented, rest) = trimmed.split_once('/')?;
    if !basalt_proto::hex::constant_time_eq(presented.as_bytes(), token.as_bytes()) {
        return None;
    }
    let path = percent_decode(rest);
    if path.is_empty() { None } else { Some(path) }
}

/// Parses a `Range: bytes=…` header into an inclusive range.
///
/// Returns `None` when the range cannot be satisfied, which is a different
/// answer from a malformed one; both get 416, which is what players expect.
fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    // Multiple ranges are legal HTTP and no media player sends them. Taking the
    // first is better than failing, and better than pretending to support
    // multipart responses.
    let spec = spec.split(',').next()?.trim();
    let (from, to) = spec.split_once('-')?;

    if from.is_empty() {
        // A suffix range: the last N bytes. Players use this to read the
        // trailing index of an MP4 that was not written for streaming.
        let n: u64 = to.parse().ok()?;
        if n == 0 || total == 0 {
            return None;
        }
        let start = total.saturating_sub(n);
        return Some((start, total - 1));
    }

    let start: u64 = from.parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if to.is_empty() {
        // An open range. Answering with a window rather than the rest of the
        // file keeps memory flat; the player asks again for the next piece.
        (start + WINDOW_BYTES - 1).min(total - 1)
    } else {
        to.parse::<u64>().ok()?.min(total - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

/// Content type from the extension.
///
/// Enough to let WebView2 pick a decoder. The formats it cannot play are not
/// helped by a more accurate guess — they need mpv, not a better header.
fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            match u8::from_str_radix(hex, 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                Err(_) => {
                    // A stray `%` is left alone rather than dropped, so a
                    // filename containing one still resolves.
                    out.push(bytes[i]);
                    i += 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0011223344556677";

    #[test]
    fn a_bounded_range_parses() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-999", 1000), Some((500, 999)));
        assert_eq!(parse_range("  bytes=0-0  ", 1000), Some((0, 0)));
    }

    #[test]
    fn a_range_past_the_end_is_clamped_not_refused() {
        assert_eq!(parse_range("bytes=900-9999", 1000), Some((900, 999)));
    }

    #[test]
    fn an_open_range_is_answered_with_a_window() {
        // What a player sends when it opens a file: "give me everything".
        // Answering literally would mean buffering the whole film.
        let (start, end) = parse_range("bytes=0-", 10 * 1024 * 1024).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, WINDOW_BYTES - 1);
    }

    #[test]
    fn an_open_range_on_a_small_file_stops_at_the_end() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
    }

    #[test]
    fn a_suffix_range_reads_from_the_end() {
        // How a player finds the index of an MP4 that was not written for
        // streaming.
        assert_eq!(parse_range("bytes=-500", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn unsatisfiable_and_malformed_ranges_are_refused() {
        assert_eq!(
            parse_range("bytes=1000-1500", 1000),
            None,
            "starts past the end"
        );
        assert_eq!(parse_range("bytes=500-100", 1000), None, "backwards");
        assert_eq!(parse_range("items=0-10", 1000), None, "wrong unit");
        assert_eq!(parse_range("bytes=abc-def", 1000), None);
        assert_eq!(parse_range("bytes=", 1000), None);
        assert_eq!(parse_range("", 1000), None);
        assert_eq!(
            parse_range("bytes=0-0", 0),
            None,
            "an empty file has no ranges"
        );
    }

    #[test]
    fn only_the_first_of_several_ranges_is_used() {
        assert_eq!(parse_range("bytes=0-99,200-299", 1000), Some((0, 99)));
    }

    #[test]
    fn the_right_token_unlocks_a_path() {
        assert_eq!(
            authorise(&format!("/{TOKEN}/films/a.mkv"), TOKEN),
            Some("films/a.mkv".into())
        );
    }

    #[test]
    fn a_wrong_or_missing_token_unlocks_nothing() {
        assert_eq!(authorise("/wrong/films/a.mkv", TOKEN), None);
        assert_eq!(authorise("/films/a.mkv", TOKEN), None);
        assert_eq!(authorise("/", TOKEN), None);
        assert_eq!(authorise("", TOKEN), None);
        assert_eq!(authorise(&format!("/{TOKEN}/"), TOKEN), None);
        assert_eq!(
            authorise(&format!("/{TOKEN}extra/a.mkv"), TOKEN),
            None,
            "a token prefix must not be enough"
        );
    }

    #[test]
    fn a_query_string_is_not_part_of_the_path() {
        assert_eq!(
            authorise(&format!("/{TOKEN}/a.mkv?t=42"), TOKEN),
            Some("a.mkv".into())
        );
    }

    #[test]
    fn encoding_survives_a_round_trip() {
        for path in [
            "films/a.mkv",
            "My Films/Holiday 2024.mp4",
            "music/Café del Mar.flac",
            "odd/name#with?chars.mp3",
            "percent%20literal.txt",
        ] {
            assert_eq!(percent_decode(&percent_encode(path)), path, "{path}");
        }
    }

    #[test]
    fn separators_stay_readable_in_a_url() {
        assert_eq!(percent_encode("films/a.mkv"), "films/a.mkv");
        assert_eq!(percent_encode("a b"), "a%20b");
    }

    #[test]
    fn a_truncated_escape_does_not_eat_the_rest_of_the_path() {
        assert_eq!(percent_decode("a%"), "a%");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
    }

    // The proxy hands paths to the vault, which rejects traversal itself — but
    // the encoding must not be a way to smuggle something past it either.
    #[test]
    fn encoded_traversal_still_arrives_as_traversal_for_the_vault_to_refuse() {
        assert_eq!(
            authorise(&format!("/{TOKEN}/%2e%2e/secret"), TOKEN),
            Some("../secret".into()),
            "decoding must be honest; the vault is what refuses it"
        );
    }

    #[test]
    fn content_types_cover_what_the_player_can_open() {
        assert_eq!(content_type_for("a.mp4"), "video/mp4");
        assert_eq!(content_type_for("a.MKV"), "video/x-matroska");
        assert_eq!(content_type_for("a.mp3"), "audio/mpeg");
        assert_eq!(content_type_for("a.jpeg"), "image/jpeg");
        assert_eq!(
            content_type_for("films/no-extension"),
            "application/octet-stream"
        );
        assert_eq!(content_type_for(""), "application/octet-stream");
    }
}
