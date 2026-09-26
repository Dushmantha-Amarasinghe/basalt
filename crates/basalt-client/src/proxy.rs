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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::client::Basalt;
use crate::{ClientError, Result};

/// Bytes read from the host per iteration while streaming a response.
///
/// A window on the *reads*, not on the answer. The whole requested range is
/// served on one connection; this only bounds how much is held in memory at
/// once, so a four-gigabyte film costs four megabytes of buffer.
const WINDOW_BYTES: u64 = 4 * 1024 * 1024;

pub struct MediaProxy {
    addr: SocketAddr,
    token: String,
    /// How far through each file a player has read.
    ///
    /// The only signal an external player gives away. It is not a playback
    /// position — a player reads ahead of what it is showing, so this runs
    /// ahead by however much it has buffered — but "roughly where they got to"
    /// is all Continue watching needs, and it is the difference between having
    /// a resume point for PotPlayer and having none.
    reach: Arc<Mutex<HashMap<String, Reach>>>,
}

/// The furthest a player has read into one file, and how big the file is.
#[derive(Debug, Clone, Copy)]
pub struct Reach {
    pub offset: u64,
    pub total: u64,
}

impl Reach {
    /// How far through, 0..1.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.offset as f64 / self.total as f64).clamp(0.0, 1.0)
    }
}

impl MediaProxy {
    /// Starts the proxy on a port the OS chooses.
    pub async fn start(client: Arc<Basalt>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let token = basalt_net::pairing::random_token()
            .map_err(|e| ClientError::Protocol(format!("no randomness available: {e}")))?;

        let accept_token = token.clone();
        let reach: Arc<Mutex<HashMap<String, Reach>>> = Arc::new(Mutex::new(HashMap::new()));
        let accept_reach = Arc::clone(&reach);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let client = Arc::clone(&client);
                let token = accept_token.clone();
                let reach = Arc::clone(&accept_reach);
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, client, token, reach).await {
                        tracing::debug!("media proxy: {e}");
                    }
                });
            }
        });

        Ok(Self { addr, token, reach })
    }

    /// How far a player has read into each file it has been handed.
    ///
    /// Cleared as it is read: this exists to be turned into a resume point once
    /// per poll, and keeping it would mean re-reporting a stale position for a
    /// film nobody is watching any more.
    pub fn take_reach(&self) -> HashMap<String, Reach> {
        std::mem::take(&mut *self.reach.lock().expect("reach lock"))
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

    /// The start every URL shares: a vault path, percent-encoded, goes on the
    /// end. Handed to the interface once so a grid of thumbnails needs no
    /// round trip per tile.
    pub fn base_url(&self) -> String {
        format!("http://{}/{}/", self.addr, self.token)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn handle(
    stream: TcpStream,
    client: Arc<Basalt>,
    token: String,
    reach: Arc<Mutex<HashMap<String, Reach>>>,
) -> Result<()> {
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

    // A thumbnail rather than the file: the same URL with `?thumb=320`, so a
    // grid of tiles is a grid of plain image URLs the page loads lazily and
    // caches, instead of a round trip through the app for every one.
    if let Some(size) = thumb_size(&target) {
        return match client.thumbnail(&path, size).await {
            Ok(bytes) => respond_image(&mut stream, &bytes, method == "HEAD").await,
            Err(e) => {
                tracing::debug!("no thumbnail for {path}: {e}");
                respond_status(&mut stream, 404, "Not Found").await
            }
        };
    }

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
        // Recorded as it goes rather than at the end: a player that is closed
        // mid-film never reaches the end of the loop, and that is precisely the
        // case a resume point exists for.
        {
            let position = start + sent + data.len() as u64;
            let mut reach = reach.lock().expect("reach lock");
            let entry = reach
                .entry(path.clone())
                .or_insert(Reach { offset: 0, total });
            entry.total = total;
            // Furthest, not latest: a player seeking backwards to re-watch a
            // scene has not un-watched what came before it.
            entry.offset = entry.offset.max(position);
        }

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

/// A finished JPEG, cached by the page.
///
/// For a week: the URL carries the file's modification time, so a changed
/// file is a different URL and never meets its old picture.
async fn respond_image(stream: &mut TcpStream, bytes: &[u8], head_only: bool) -> Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: image/jpeg\r\n\
         Content-Length: {}\r\n\
         Cache-Control: private, max-age=604800\r\n\
         Connection: close\r\n\r\n",
        bytes.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(bytes).await?;
    }
    Ok(())
}

/// The size in `?thumb=N`, when the request is for a thumbnail.
fn thumb_size(target: &str) -> Option<u32> {
    let (_, query) = target.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("thumb="))
        .and_then(|n| n.parse().ok())
        .filter(|&n| n > 0)
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
        // An open range means "from here to the end", and that is what it
        // gets. Memory stays flat regardless, because the body is streamed
        // `WINDOW_BYTES` at a time — the cap that used to be here bought
        // nothing and broke playback outright.
        //
        // It answered `bytes=0-` with four megabytes and closed. A browser
        // and VLC both ask again for the next piece, so this looked fine for
        // a year. FFmpeg — and therefore mpv — reads a short body as the end
        // of the stream, stops dead, and will not resume, because as far as
        // it is concerned the file is over. Every film played for the same
        // handful of seconds and froze.
        total - 1
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
    #[test]
    fn a_thumbnail_is_asked_for_in_the_query() {
        assert_eq!(
            super::thumb_size("/tok/Photos%2Fa.jpg?thumb=320&v=17"),
            Some(320)
        );
        assert_eq!(
            super::thumb_size("/tok/Photos%2Fa.jpg?v=17&thumb=1600"),
            Some(1600)
        );
        assert_eq!(super::thumb_size("/tok/film.mkv"), None);
        assert_eq!(super::thumb_size("/tok/film.mkv?thumb=0"), None);
        assert_eq!(super::thumb_size("/tok/film.mkv?thumb=big"), None);
    }

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

    /// An open range means the rest of the file, and must be answered as one.
    ///
    /// This test used to assert the opposite — that `bytes=0-` was answered
    /// with a four-megabyte window — and the app shipped that way because a
    /// browser and VLC both quietly ask again for the next piece. FFmpeg does
    /// not: a short body is the end of the stream, so every film played for
    /// the same few seconds and froze with no way to resume. Memory is bounded
    /// by the read loop, not by lying about the length.
    #[test]
    fn an_open_range_runs_to_the_end_of_the_file() {
        let big = 4_000_000_000u64;
        assert_eq!(parse_range("bytes=0-", big), Some((0, big - 1)));
        // And from a seek, not only from the start.
        assert_eq!(parse_range("bytes=1500-", big), Some((1500, big - 1)));
    }

    /// The window is a read size now, and has to stay well under the file it
    /// is reading, or the loop would be pointless.
    #[test]
    fn the_read_window_bounds_memory_not_the_answer() {
        let (start, end) = parse_range("bytes=0-", 10 * WINDOW_BYTES).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 10 * WINDOW_BYTES - 1, "the answer is the whole file");
        assert!(WINDOW_BYTES < end, "but it is read in pieces");
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
    fn how_far_through_a_file_a_player_read() {
        assert_eq!(
            Reach {
                offset: 50,
                total: 100
            }
            .fraction(),
            0.5
        );
        assert_eq!(
            Reach {
                offset: 100,
                total: 100
            }
            .fraction(),
            1.0
        );
    }

    /// A file whose size the host would not report must not produce a
    /// division by zero, and must not claim any progress either.
    #[test]
    fn a_file_of_unknown_size_reports_no_progress() {
        assert_eq!(
            Reach {
                offset: 9,
                total: 0
            }
            .fraction(),
            0.0
        );
    }

    /// A player that read past the end — because the file shrank under it —
    /// has still only watched the whole thing once.
    #[test]
    fn reading_past_the_end_is_still_the_whole_file() {
        assert_eq!(
            Reach {
                offset: 500,
                total: 100
            }
            .fraction(),
            1.0
        );
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
