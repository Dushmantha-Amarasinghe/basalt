# Basalt — Phase 0 benchmarks

_Generated 2026-09-15 17:09:08_

## Machine

| | |
|---|---|
| Host | FST |
| OS | Microsoft Windows 11 Pro |
| CPU | 13th Gen Intel(R) Core(TM) i5-13420H (12 threads) |
| RAM | 15.7 GB |
| Wi-Fi | FST-5G · 802.11ac · 5 GHz · tx 866 Mbps / rx 866 Mbps · signal 100% |
| Implied one-way ceiling | ~60 MB/s |

## disk-sequential

unbuffered sequential read throughput by block size

| Measurement | Median | Throughput | Ratio | p95 | Runs |
|---|---|---|---|---|---|
| unbuffered, 64.0 KiB blocks | 166.3 ms | 1614.2 MB/s | — | 190.3 ms | 3 |
| unbuffered, 256 KiB blocks | 191.7 ms | 1400.4 MB/s | — | 203.3 ms | 3 |
| unbuffered, 1.00 MiB blocks | 100.3 ms | 2675.3 MB/s | — | 101.6 ms | 3 |
| unbuffered, 4.00 MiB blocks | 76.0 ms | 3532.7 MB/s | — | 76.6 ms | 3 |
| unbuffered, 8.00 MiB blocks | 69.9 ms | 3840.8 MB/s | — | 71.8 ms | 3 |
| buffered (page cache, for contrast) | 66.2 ms | 4055.8 MB/s | — | 115.1 ms | 3 |

<details><summary>Notes</summary>

- **buffered (page cache, for contrast)** — served largely from RAM; shown only to make the cache effect visible. Do not quote this as a disk speed.

</details>

## disk-concurrency

small-file read throughput vs thread count — decides the I/O scheduler's queue depth

| Measurement | Median | Throughput | Ratio | Items/s | p95 | Runs |
|---|---|---|---|---|---|---|
|  1 thread  | 556.6 ms | 92.8 MB/s | — | 3593 | 619.0 ms | 3 |
|  2 threads | 287.3 ms | 179.9 MB/s | — | 6962 | 287.7 ms | 3 |
|  4 threads | 175.6 ms | 294.2 MB/s | — | 11388 | 177.0 ms | 3 |
|  8 threads | 122.3 ms | 422.6 MB/s | — | 16358 | 134.3 ms | 3 |
| 16 threads | 110.4 ms | 468.0 MB/s | — | 18115 | 113.9 ms | 3 |

## disk-access-order

reading a file set in on-disk order vs shuffled — justifies sorting the batch manifest

| Measurement | Median | Throughput | Ratio | Items/s | p95 | Runs |
|---|---|---|---|---|---|---|
| sorted (on-disk order) | 248.7 ms | 207.7 MB/s | — | 8041 | 252.9 ms | 3 |
| shuffled (random) | 259.2 ms | 199.3 MB/s | — | 7715 | 273.5 ms | 3 |

## disk-listing

enumerating a large directory — the cost the metadata index removes

| Measurement | Median | Items/s | p95 | Runs |
|---|---|---|---|---|
| names only | 2.6 ms | 3844232 | 2.7 ms | 3 |
| names + metadata | 2.6 ms | 3861451 | 2.8 ms | 3 |

<details><summary>Notes</summary>

- **names + metadata** — this is the per-visit cost a file browser pays without an index; the mirrored SQLite index reduces it to a local query

</details>

---

Numbers are medians over repeated runs. Compare only against runs on the same machines and the same radio conditions.
