//! One authenticated connection to a host.
//!
//! A session is a TLS stream that has already proved the host is the pinned one
//! and has already presented this device's token. Everything above it can
//! assume both.

use std::net::SocketAddr;

use basalt_net::framing::{
    call_json, call_unit, read_response, read_response_header, write_request,
};
use basalt_net::socket;
use basalt_net::tls::{Trust, client_config, sni_name};
use basalt_proto::msg::*;
use basalt_proto::ops::Op;
use basalt_proto::{ErrorCode, PROTOCOL_VERSION};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::{ClientError, Result};

/// What the host said about itself when this session opened.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub host_id: String,
    pub host_name: String,
    pub vault: String,
    pub writable: bool,
    pub address: SocketAddr,
}

pub struct Session {
    stream: TlsStream<TcpStream>,
    info: SessionInfo,
}

/// Opens a TLS connection and reads the host's greeting.
///
/// Returns the stream and the greeting separately because pairing and
/// reconnecting need the same first half and diverge afterwards.
async fn open(
    addr: SocketAddr,
    trust: Trust,
    device_name: &str,
) -> Result<(TlsStream<TcpStream>, HelloResponse, String)> {
    let tcp = socket::connect(addr).await?;
    let (connector, verifier) = client_config(trust);
    let mut stream = connector.connect(sni_name(), tcp).await.map_err(|e| {
        // The pin failing is the one connection error worth explaining
        // precisely, because it means something is wrong rather than merely
        // unavailable.
        ClientError::Net(basalt_net::NetError::Io(e))
    })?;

    let hello: HelloResponse = call_json(
        &mut stream,
        Op::Hello,
        &HelloRequest {
            protocol: PROTOCOL_VERSION,
            device_name: device_name.to_string(),
        },
    )
    .await?;

    if hello.protocol != PROTOCOL_VERSION {
        return Err(ClientError::Incompatible {
            ours: PROTOCOL_VERSION,
            theirs: hello.protocol,
        });
    }

    // The id the certificate actually carried, not the one in the greeting. A
    // host that lies in `host_id` cannot lie about the key it signed with.
    let presented = verifier
        .seen_host_id()
        .ok_or_else(|| ClientError::Protocol("the handshake produced no host identity".into()))?;

    Ok((stream, hello, presented))
}

impl Session {
    /// Reconnects to a host already paired with.
    pub async fn connect(
        addr: SocketAddr,
        host_id: &str,
        token: &str,
        device_name: &str,
    ) -> Result<Self> {
        let (mut stream, hello, presented) =
            open(addr, Trust::Pinned(host_id.to_string()), device_name).await?;

        // Belt and braces: the TLS verifier has already refused anything else,
        // so this can only fire if that check were ever weakened.
        if presented != host_id {
            return Err(ClientError::WrongHost {
                expected: host_id.to_string(),
                got: presented,
            });
        }

        let auth: AuthResponse = call_json(
            &mut stream,
            Op::Auth,
            &AuthRequest {
                token: token.to_string(),
            },
        )
        .await?;

        Ok(Self {
            stream,
            info: SessionInfo {
                host_id: presented,
                host_name: hello.host_name,
                vault: auth.vault,
                writable: auth.writable,
                address: addr,
            },
        })
    }

    /// Looks at a host without pairing, so the client can show what it found.
    pub async fn probe(addr: SocketAddr, device_name: &str) -> Result<HelloResponse> {
        let (_stream, hello, presented) = open(addr, Trust::FirstContact, device_name).await?;
        // Report the key that was actually presented rather than the claim in
        // the body, so the id shown to the user is the one that will be pinned.
        Ok(HelloResponse {
            host_id: presented,
            ..hello
        })
    }

    /// Pairs with a host using the PIN shown on its screen.
    ///
    /// Trust is [`Trust::FirstContact`] here because there is nothing to
    /// compare against yet — which is exactly why the PIN proof is bound to the
    /// key that turned up. See [`basalt_net::pairing`].
    pub async fn pair(addr: SocketAddr, pin: &str, device_name: &str) -> Result<(Self, String)> {
        let (mut stream, hello, presented) = open(addr, Trust::FirstContact, device_name).await?;

        if !hello.pairing_open {
            return Err(ClientError::PairingClosed);
        }

        let client_nonce = basalt_net::pairing::random_nonce()
            .map_err(|e| ClientError::Protocol(format!("no randomness available: {e}")))?;

        let begin: PairBeginResponse = call_json(
            &mut stream,
            Op::PairBegin,
            &PairBeginRequest {
                client_nonce: client_nonce.clone(),
            },
        )
        .await?;

        let proof =
            basalt_net::pairing::compute_proof(pin, &presented, &client_nonce, &begin.server_nonce)
                .map_err(|e| ClientError::Protocol(format!("could not build the proof: {e}")))?;

        let finish: PairFinishResponse = call_json(
            &mut stream,
            Op::PairFinish,
            &PairFinishRequest {
                proof,
                device_name: device_name.to_string(),
            },
        )
        .await
        .map_err(|e| match e.code() {
            // The host refusing the proof almost always means the PIN was
            // mistyped, and saying so is more useful than the wire message.
            Some(ErrorCode::PairingRefused) => ClientError::BadPin(e.to_string()),
            _ => ClientError::Net(e),
        })?;

        let token = finish.token.clone();
        Ok((
            Self {
                stream,
                info: SessionInfo {
                    host_id: presented,
                    host_name: hello.host_name,
                    vault: finish.vault,
                    // Pairing always grants write access; the host can demote a
                    // device afterwards.
                    writable: true,
                    address: addr,
                },
            },
            token,
        ))
    }

    pub fn info(&self) -> &SessionInfo {
        &self.info
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    pub async fn ping(&mut self) -> Result<()> {
        write_request(&mut self.stream, Op::Ping, &[]).await?;
        read_response(&mut self.stream).await?;
        Ok(())
    }

    pub async fn list(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let response: ListResponse = call_json(
            &mut self.stream,
            Op::List,
            &ListRequest {
                path: path.to_string(),
            },
        )
        .await?;
        Ok(response.entries)
    }

    pub async fn stat(&mut self, path: &str) -> Result<DirEntry> {
        let response: StatResponse = call_json(
            &mut self.stream,
            Op::Stat,
            &StatRequest {
                path: path.to_string(),
            },
        )
        .await?;
        Ok(response.entry)
    }

    pub async fn space(&mut self) -> Result<(u64, u64)> {
        let response: SpaceResponse =
            call_json(&mut self.stream, Op::Space, &serde_json::json!({})).await?;
        Ok((response.free, response.total))
    }

    /// Reads a byte range. Shorter than requested at the end of the file.
    pub async fn read_range(&mut self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(&ReadRequest {
            path: path.to_string(),
            offset,
            length,
        })
        .map_err(|e| ClientError::Protocol(format!("could not encode the read: {e}")))?;
        write_request(&mut self.stream, Op::Read, &body).await?;
        Ok(read_response(&mut self.stream).await?)
    }

    /// Streams a byte range into a writer without buffering it whole.
    ///
    /// This is what a download uses: a 2 GB file must never exist in memory,
    /// on a host with 5.9 GB of RAM or on a client either.
    pub async fn read_range_into<W>(
        &mut self,
        path: &str,
        offset: u64,
        length: u64,
        out: &mut W,
    ) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        let body = serde_json::to_vec(&ReadRequest {
            path: path.to_string(),
            offset,
            length,
        })
        .map_err(|e| ClientError::Protocol(format!("could not encode the read: {e}")))?;
        write_request(&mut self.stream, Op::Read, &body).await?;

        let (status, len) = read_response_header(&mut self.stream).await?;
        if status != basalt_proto::STATUS_OK {
            // Drain the error body so the connection stays usable, then report
            // it the same way a buffered read would.
            let mut body = vec![0u8; len as usize];
            self.stream.read_exact(&mut body).await?;
            return Err(ClientError::Net(basalt_net::NetError::Remote(
                serde_json::from_slice(&body).unwrap_or_else(|_| {
                    basalt_proto::WireError::new(
                        ErrorCode::Io,
                        String::from_utf8_lossy(&body).into_owned(),
                    )
                }),
            )));
        }

        let mut remaining = len;
        let mut buf = vec![0u8; 256 * 1024];
        while remaining > 0 {
            let want = buf.len().min(remaining as usize);
            let n = self.stream.read(&mut buf[..want]).await?;
            if n == 0 {
                return Err(ClientError::Protocol(format!(
                    "the host stopped with {remaining} bytes still owed"
                )));
            }
            out.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        Ok(len)
    }

    /// Fetches many files as one batch stream, already decompressed.
    pub async fn read_batch(&mut self, paths: Vec<String>) -> Result<Vec<basalt_proto::Entry>> {
        let body = serde_json::to_vec(&basalt_proto::BatchRequest::new(paths))
            .map_err(|e| ClientError::Protocol(format!("could not encode the manifest: {e}")))?;
        write_request(&mut self.stream, Op::ReadBatch, &body).await?;
        let stream = read_response(&mut self.stream).await?;

        // Decoding is CPU work on a potentially large buffer, so it runs off
        // the runtime rather than stalling every other connection.
        tokio::task::spawn_blocking(move || {
            basalt_proto::frame::BatchReader::new(stream.as_slice())?
                .collect::<basalt_proto::Result<Vec<_>>>()
        })
        .await
        .map_err(|e| ClientError::Protocol(format!("decoding failed: {e}")))?
        .map_err(Into::into)
    }

    // --- writing -----------------------------------------------------------

    pub async fn write_begin(
        &mut self,
        path: &str,
        size: u64,
        overwrite: bool,
        resume: Option<&str>,
    ) -> Result<WriteBeginResponse> {
        call_json(
            &mut self.stream,
            Op::WriteBegin,
            &WriteBeginRequest {
                path: path.to_string(),
                size,
                overwrite,
                resume: resume.map(str::to_string),
            },
        )
        .await
        .map_err(Into::into)
    }

    pub async fn write_chunk(&mut self, upload: &str, offset: u64, data: &[u8]) -> Result<()> {
        let id = parse_upload_id(upload)?;
        let payload = encode_chunk(&id, offset, data);
        write_request(&mut self.stream, Op::WriteChunk, &payload).await?;
        read_response(&mut self.stream).await?;
        Ok(())
    }

    pub async fn write_commit(
        &mut self,
        upload: &str,
        blake3: &str,
        mtime: Option<i64>,
    ) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::WriteCommit,
            &WriteCommitRequest {
                upload: upload.to_string(),
                blake3: blake3.to_string(),
                mtime,
            },
        )
        .await
        .map_err(Into::into)
    }

    pub async fn write_abort(&mut self, upload: &str) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::WriteAbort,
            &WriteAbortRequest {
                upload: upload.to_string(),
            },
        )
        .await
        .map_err(Into::into)
    }

    // --- mutations ---------------------------------------------------------

    pub async fn mkdir(&mut self, path: &str) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::Mkdir,
            &MkdirRequest {
                path: path.to_string(),
            },
        )
        .await
        .map_err(Into::into)
    }

    pub async fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::Rename,
            &RenameRequest {
                from: from.to_string(),
                to: to.to_string(),
            },
        )
        .await
        .map_err(Into::into)
    }

    pub async fn copy(&mut self, from: &str, to: &str) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::Copy,
            &CopyRequest {
                from: from.to_string(),
                to: to.to_string(),
            },
        )
        .await
        .map_err(Into::into)
    }

    pub async fn remove(&mut self, path: &str, recursive: bool) -> Result<()> {
        call_unit(
            &mut self.stream,
            Op::Remove,
            &RemoveRequest {
                path: path.to_string(),
                recursive,
            },
        )
        .await
        .map_err(Into::into)
    }
}
