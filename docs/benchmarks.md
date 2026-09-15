# Basalt — Phase 0 benchmarks

_Generated 2026-09-15 11:52:22_

## Machine

| | |
|---|---|
| Host | FST |
| OS | Microsoft Windows 11 Pro |
| CPU | 13th Gen Intel(R) Core(TM) i5-13420H (12 threads) |
| RAM | 15.7 GB |
| Wi-Fi | FST-5G · 802.11ac · 5 GHz · tx 866 Mbps / rx 866 Mbps · signal 100% |
| Implied one-way ceiling | ~60 MB/s |

## latency

round-trip time — the per-request floor that batching exists to avoid

| Measurement | Median | Items/s | p95 | Runs |
|---|---|---|---|---|
| ping round trip | 77 µs | 12953 | 148 µs | 200 |

<details><summary>Notes</summary>

- **ping round trip** — 500 sequential requests would cost 0.0s in latency alone

</details>

## raw-throughput

link ceiling with no disk involved, by stream count and encryption

| Measurement | Median | Throughput | Ratio | p95 | Runs |
|---|---|---|---|---|---|
| download  1 stream  plain | 53.1 ms | 1264.9 MB/s | — | 67.1 ms | 3 |
| download  2 streams plain | 52.4 ms | 1281.4 MB/s | — | 95.0 ms | 3 |
| download  4 streams plain | 32.4 ms | 2069.2 MB/s | — | 100.4 ms | 3 |
| download  8 streams plain | 40.0 ms | 1677.2 MB/s | — | 54.9 ms | 3 |
| download 16 streams plain | 23.1 ms | 2901.6 MB/s | — | 31.0 ms | 3 |
| download  1 stream  tls | 108.2 ms | 620.1 MB/s | — | 208.5 ms | 3 |
| download  2 streams tls | 47.5 ms | 1411.7 MB/s | — | 52.7 ms | 3 |
| download  4 streams tls | 52.8 ms | 1271.4 MB/s | — | 66.7 ms | 3 |
| download  8 streams tls | 26.1 ms | 2573.1 MB/s | — | 33.6 ms | 3 |
| download 16 streams tls | 24.4 ms | 2746.6 MB/s | — | 27.6 ms | 3 |
| upload  1 stream  plain | 43.3 ms | 1549.1 MB/s | — | 56.3 ms | 3 |

## small-files

per-file requests vs one batched request, for many small files

| Measurement | Median | Throughput | Ratio | Items/s | p95 | Runs |
|---|---|---|---|---|---|---|
| per-file requests, sequential | 172.0 ms | 74.2 MB/s | — | 2907 | 186.2 ms | 3 |
| batched request, raw  | 83.1 ms | 153.5 MB/s | — | 6017 | 96.9 ms | 3 |
| batched request, zstd | 137.0 ms | 93.1 MB/s | 2.24x | 3649 | 140.6 ms | 3 |

---

Numbers are medians over repeated runs. Compare only against runs on the same machines and the same radio conditions.
