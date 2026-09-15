# Basalt

A personal NAS built from a spare laptop and a desktop HDD.

- **Basalt Host** — runs on the laptop, locks in a drive, serves it over the LAN.
- **Basalt** — runs on your PC. Pair once; after that it is just there.

Currently at **Phase 0**: measuring before building. See
[the plan](../../../Users/dsbam/.claude/plans/i-want-to-make-quirky-rainbow.md)
for the full design.

## Why Phase 0 exists

Building a NAS client is months of work. Windows already ships SMB, and on a
LAN a tuned SMB3 share is genuinely fast. Discovering in month six that file
sharing was already quicker would be an expensive way to learn it.

So before any UI is written, `basalt-bench` measures the real stack — the same
framing and compression code the shipped apps will use — against an SMB
baseline, on the actual laptop, the actual drive, and the actual radio.

**The gate:** match or beat SMB on both a single large file and ten thousand
small ones. If we cannot, the approach gets revisited.

## Layout

```
crates/basalt-proto/   wire framing + compression policy (ships in Phase 1)
bench/                 Phase 0 measurement harness
docs/                  generated benchmark reports
```

`basalt-proto` is deliberately not benchmark-only code. The batch stream format
and the entropy-based compression policy are what the host and client will
actually speak, so Phase 0 measures the real thing rather than a stand-in.

## Setup

Requires Rust (stable) and the MSVC build tools.

```bash
cargo build --release
cargo test
```

Enable the pre-commit hook once per clone. It runs `cargo fmt --check`,
`cargo clippy -D warnings`, and the full test suite before every commit, so a
regression cannot land quietly:

```bash
git config core.hooksPath .githooks
```

The whole suite runs in a few seconds — a slow gate is one people start
skipping. Use `git commit --no-verify` to bypass it deliberately.

## Tests

| Suite | What it guards |
|---|---|
| `basalt-proto` unit | framing, codec policy, path rules |
| `frame_stress` | randomised round trips, truncation at every byte offset, bit-flip corruption, hostile length fields |
| `path_safety` | ~50 adversarial paths — traversal, UNC, drive letters, NUL, Windows device names, trailing-dot tricks |
| `net_integration` | the real server and client over real TCP and TLS: batching, inline errors, concurrency, malformed input |
| `compression_policy` | the Phase 0 conclusion itself — that the corpus still models reality and compression still beats the link |
| `disk` / `winio` | unbuffered reads return correct bytes, the concurrency classifier reads curves the right way round |

Two tests measure real throughput and so are time-sensitive. They take the
**best of five runs** rather than one, because `cargo test` runs them in
parallel with everything else and any single run can be descheduled
mid-measurement. Thresholds are set loose (2x the link speed against ~14x
measured headroom) so they only fire on a genuinely bad change, such as raising
the default zstd level.

`net_integration` and `frame_stress` deliberately attack the code rather than
demonstrate it. Two real bugs came out of writing them: a decoder that accepted
a stream truncated after its terminator, and a server that put an unsanitised
path into an error entry, which made the *decoder* reject the whole batch —
exactly the failure inline errors exist to prevent.

## Running the benchmarks

### 1. Compression — run this first, no second machine needed

The highest-value measurement. On a ~30 MB/s Wi-Fi link, zstd compresses
roughly 20x faster than the radio can transmit, so compressible data should
move several times faster than raw.

```bash
cargo run --release -p basalt-bench -- compress
```

### 2. Generate the corpus on the laptop

Point `--root` at the drive under test. Use `--quick` (~1 GB) to smoke-test the
harness, or omit it for the full ~8 GB corpus.

```bash
cargo run --release -p basalt-bench -- gen-corpus --root D:\bench-corpus
```

The corpus is deterministic: the same `--seed` produces byte-identical files, so
runs are comparable across machines and across days.

### 3. Serve from the laptop

```bash
cargo run --release -p basalt-bench -- serve --root D:\bench-corpus
```

Prints the addresses it is reachable on. Allow it through Windows Firewall when
prompted — ports 7742 (plaintext) and 7743 (TLS).

### 4. Disk benchmarks, on the laptop

Run this against the drive under test. Every read bypasses the Windows file
cache, so the numbers are the drive rather than RAM.

```bash
cargo run --release -p basalt-bench -- disk --root D:\bench-corpus
```

The output that matters is the **concurrency curve**. If throughput falls as
threads are added, the host needs an I/O scheduler that caps disk parallelism.
If it keeps climbing, that whole component can be dropped.

### 5. Measure from the PC

```bash
cargo run --release -p basalt-bench -- net --host 192.168.1.42
```

### 6. The SMB baseline — the actual gate

Share the corpus folder on the laptop, then from the PC:

```bash
cargo run --release -p basalt-bench -- smb --share \\LAPTOP\bench-corpus
```

Use the same `--small-files` count as the `net` run, or the comparison is
meaningless. Compare the batched request against SMB's **best parallel**
result, not its sequential one — beating a strawman proves nothing.

### Check the environment any time

```bash
cargo run --release -p basalt-bench -- env
```

Reports CPU, RAM, and the Wi-Fi link — band, channel, rate, signal — plus
advice when the link is the thing holding transfers back.

## Output

Every run writes `docs/benchmarks.md` (readable) and `docs/benchmarks.json`
(diffable), both stamped with the machine that produced them. Numbers from
different CPUs or different radio conditions are not comparable, and the report
makes that explicit.

## Reading the results

- **Medians, not means.** One antivirus scan turns a mean into fiction.
- **`⚠ unstable`** means p95 ran more than 25% over the median. Close background
  apps and re-run; do not record unstable numbers.
- **Loopback proves the harness works, not the design.** With a 77 µs RTT and a
  2900 MB/s "link", batching and compression both look pointless — correctly so.
  Only a run across the real radio answers the real question.
