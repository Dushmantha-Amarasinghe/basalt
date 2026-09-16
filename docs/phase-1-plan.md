# Phase 1 — the real thing

Everything below is decided by the Phase 0 measurements in
[`measured-facts.md`](measured-facts.md). Where the original plan and the
numbers disagree, the numbers win, and the difference is recorded here rather
than quietly applied.

---

## What is being built

**Basalt Host** — runs on laptop B, locks in the USB drive, serves it.
**Basalt** — runs on laptop A, pairs once with a PIN, then it is just there.

Working means: browse the real drive, open folders, download, upload, rename,
move, delete, and play video and music straight off it — with no Windows
sharing, no credentials, no network profile changes, and nothing to configure
on a second client beyond typing the PIN once.

---

## Decisions, and what overturned them

| Original plan | Now | Why |
|---|---|---|
| HTTP/2 over TLS | **Our own binary protocol over TLS 1.3** | We already have tested framing. HTTP/2's multiplexing measured 1.0x here, so it buys nothing and costs a dependency tree. |
| Adaptive multi-stream transfers | **Cut** | 1 stream 20.8 MB/s, 16 streams 24.1. Budgeted 1.3–2x; measures 1.0x. |
| Multiplex one connection | **A small connection pool** | Simpler than multiplexing and removes head-of-line blocking outright: a 2 GB download must not freeze browsing. Extra connections measured free. |
| mTLS with issued device certificates | **Pinned server SPKI + a device token** | With the server's public key pinned there is nothing a client certificate adds on a LAN, and it is a great deal less machinery to get wrong. |
| SQLite index mirrored to the client | **List directories live** | 20,000 entries list in 11 ms on B. An index would add a watcher, a schema, a sync protocol and a class of staleness bugs to solve a problem we cannot measure. Revisit when a real folder is actually slow. |
| libmpv sidecar | **Ranged reads + a local HTTP proxy, `<video>` for now** | The proxy is the plumbing mpv needs anyway. This gets seeking and playback working end to end immediately; mpv slots in behind the same URL later for the formats WebView2 cannot decode. |
| Disk concurrency capped at 2–4 | **Queue depth 12** | The drive scaled monotonically to 16 threads, 2.45x faster than serial. The textbook rule is wrong for a USB bridge. |

Carried forward unchanged, because measurement confirmed them: TLS always on
(free at this speed), zstd-1 (level 9 is slower than the link), batch streams
for small files (7.6x), sorted manifests (2.71x), 2 MiB `SO_RCVBUF` set before
connect (3x spread).

---

## Shape

```
crates/
  basalt-proto    wire format: framing, codec, ops, request/response bodies
  basalt-net      TLS + SPKI pinning, pairing handshake, framed connection
  basalt-host     filesystem layer, server, device registry, config
  basalt-client   session, connection pool, high-level API, media proxy
apps/
  client          Tauri shell + React UI (exists; gets wired to basalt-client)
  host            Phase 2 — for now the host ships as a console app
```

`basalt-proto` stays dependency-light: both ends and the benchmark harness
already depend on it, and the pre-commit hook runs its tests on every commit.

---

## The protocol

Request and response framing is the shape Phase 0 already proved:

```text
request   [op u8][len u32 LE][payload len bytes]
response  [status u8][len u64 LE][payload len bytes]
```

Bodies are JSON for control operations and raw bytes for data, because control
messages are tiny and infrequent while data is the only thing that has to be
fast.

| Op | Request | Response |
|---|---|---|
| `Hello` | client version, device name | server version, vault name, whether paired |
| `PairBegin` | client nonce | server nonce |
| `PairFinish` | HMAC proof, device name | device token |
| `Auth` | device token | ok / denied |
| `List` | directory path | JSON entries (name, kind, size, mtime) |
| `Stat` | path | JSON entry |
| `Read` | path, offset, length | raw bytes — this is what makes seeking work |
| `ReadBatch` | manifest | one `basalt-proto` batch stream |
| `WriteBegin` | path, total size | upload id |
| `WriteChunk` | upload id, offset, bytes | ok |
| `WriteCommit` | upload id, BLAKE3 | ok |
| `Mkdir` / `Rename` / `Remove` | paths | ok |
| `Ping` | — | — |

Uploads are three ops rather than one so they can resume: a dropped Wi-Fi
connection mid-way through a 2 GB file must not mean starting again.

## Pairing

1. The host generates a self-signed certificate once and keeps it. Its SPKI
   SHA-256 is the host's permanent identity.
2. The user puts the host in pairing mode, which shows a 6-digit PIN and lasts
   three minutes.
3. The client connects, and *before trusting anything*, reads the certificate
   the server actually presented and computes its SPKI hash.
4. Both sides exchange nonces and compute
   `HMAC-SHA256(PIN, spki_hash ‖ client_nonce ‖ server_nonce)`.
   The SPKI hash is in the proof deliberately: it is what stops a machine in the
   middle from getting its own key pinned during first contact, which is the one
   moment this design is vulnerable.
5. On a correct proof the host issues a 32-byte device token and records the
   device. The client stores the token and the pinned SPKI hash.
6. Every connection after that verifies the pin first, then presents the token.
   No prompts, ever.

Five wrong PINs and the host leaves pairing mode and generates a new one.

## Zero configuration — what is actually required

The honest list for the host machine, and nothing else anywhere:

- One inbound firewall rule for the Basalt port. Windows asks once, on first
  run, with a UAC prompt. The host detects when it has been refused and offers
  to add the rule itself rather than leaving the user to guess why nothing
  connects.
- Nothing on the client. No credentials, no profile change, no guest logons, no
  administrator rights.

The rule is scoped to Private *and* Public profiles on purpose. SMB could not
safely do that, which is why it needed the profile changed; we can, because
every connection is TLS with a pinned key and an authenticated token.

## Performance, applied

- One control connection plus up to three data connections, pooled and kept
  alive; TLS session resumption makes a new one cheap.
- `SO_RCVBUF` 2 MiB set before connect, `TCP_NODELAY` on.
- Directory fetches use the batch stream and are compressed; listings are highly
  compressible JSON.
- Small-file transfers go through `ReadBatch` with the manifest sorted, read at
  queue depth 12.
- Large files use `Read` with 4 MiB ranges on one connection.
- Compression is decided per payload by the existing entropy policy: a film is
  sent raw, a source tree is sent compressed.

## Testing

Everything below runs in the pre-commit hook, alongside the existing suite.

- Path resolution against traversal, UNC, drive letters, symlinks and junctions
  that escape the root — the host's most security-critical function.
- Pairing: correct PIN succeeds, wrong PIN fails, a proof computed against a
  *different* SPKI fails (the MITM case), replay of a captured proof fails,
  expiry works, lockout after five attempts works.
- Token auth: a valid token connects, a revoked one does not.
- Full round trip in-process — host and client in one test, over a real socket
  on port 0: pair, list, upload, download, verify BLAKE3, rename, delete.
- Resumable upload: interrupt mid-file, reconnect, finish, verify the hash.
- Range reads: every offset and length combination lands on the right bytes,
  including reads past the end.

## Out of scope for this phase, deliberately

The host dashboard as a Tauri app, the client-side content cache, delta sync,
thumbnails generated on the host, the drive-letter mount, and search across the
whole drive (which is the one feature that genuinely needs the index we just
cut). Each is a phase of its own and none of them block the thing this phase is
for: the connection actually working.
