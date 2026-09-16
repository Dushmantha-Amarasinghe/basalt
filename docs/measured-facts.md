# Measured facts

Everything here came off the real hardware: **laptop B** (old, low-end, hard
drive attached, the NAS) serving **laptop A** (the client), both on 5 GHz Wi-Fi.
These numbers replace the estimates the original plan was built on. Where a
measurement contradicted an assumption, the assumption lost.

Re-measure with `basalt-bench lab` and `basalt-bench measure` after any change
to the network — none of this transfers to a different setup.

---

## The link

| | |
|---|---|
| Usable throughput | **22.7 MB/s** |
| Round-trip time | **2.3–2.9 ms** |
| Upload vs download | symmetric (21.6 up / 22.4 down) |

Both machines are wireless, so **every byte crosses the air twice**: B to the
router, then the router to A. That halves the radio's capacity before anything
else. A single Ethernet cable to B would remove one of those hops and is worth
roughly 2.5–3x — more than anything achievable in software. Not available right
now; revisit when B can sit near the router.

## Transport: finished, no headroom left

The UDP control experiment settles it. UDP has no congestion control, no
acknowledgements and no retransmission, so it should beat TCP if TCP were the
constraint. It did not:

| Offered | Delivered | Loss |
|---|---|---|
| 10 MB/s | 8.7 MB/s | 8.7% |
| 20 MB/s | 12.4 MB/s | 31.4% |
| 30 MB/s | 14.9 MB/s | 44.5% |
| 40 MB/s | 17.4 MB/s | 42.5% |
| 80 MB/s | 17.7 MB/s | 38.9% |
| 120 MB/s | 16.2 MB/s | 42.7% |

Delivery plateaus near 17.5 MB/s while loss climbs past 40%, and pushing harder
makes it *worse* — 120 MB/s offered delivered less than 80 MB/s offered.

**TCP (22.7 MB/s) beat raw UDP (17.7 MB/s).** The link refuses traffic above its
rate, and TCP's congestion control paces to what the radio will actually accept.
A custom UDP protocol would have to rebuild congestion control from scratch
merely to draw level with what TCP already provides.

**Conclusion: there is no faster transport to find. Transport work is closed.**

## Socket tuning: one real win

Receive buffer size, measured end to end:

| Buffer | Throughput |
|---|---|
| 64 KiB | 7.4 MB/s |
| 256 KiB | 17.4 MB/s |
| 1 MiB | 22.1 MB/s |
| **4 MiB** | **22.7 MB/s** |
| 8 MiB | 20.1 MB/s (bufferbloat) |

A **3x spread from one socket option**. The OS default landed at 20.4 MB/s.
`DEFAULT_RECV_BUFFER` is set to 2 MiB — inside the flat optimum, clear of the
8 MiB regression.

`SO_RCVBUF` must be set *before* connecting: it determines the window scale
negotiated in the handshake. Setting it afterwards resizes the buffer but not
the scale, silently capping data in flight.

Write size barely matters: 64 KiB marginally best, ~12% across the whole range.

## Parallel streams: no benefit

| Streams | Throughput |
|---|---|
| 1 | 20.8 MB/s |
| 2 | 22.4 |
| 4 | 22.6 |
| 8 | 23.4 |
| 16 | 24.1 |

Essentially flat. A single connection already saturates the radio.

**The adaptive multi-stream transfer engine is cut from the plan.** It was
budgeted at 1.3–2x; it measures 1.0x.

## Encryption: free

Plain 20.2–24.1 MB/s, TLS 20.6–22.9 MB/s — within noise, occasionally faster.
At this link speed AES-GCM costs nothing.

**TLS is always on. No insecure mode, no toggle, no argument.**

## Batching: the biggest win in the system

2,000 small files:

| Approach | Time | Speed |
|---|---|---|
| One request per file | 19.15 s | 2.7 MB/s |
| One batched request, raw | 3.54 s | 14.6 MB/s |
| **One batched request, zstd** | **2.54 s** | **20.4 MB/s** |

**7.6x faster than per-file requests.** At 2.3 ms round-trip, 2,000 sequential
requests spend ~6 seconds doing nothing but waiting.

Worth noting: on loopback this same test showed only 1.3x, because the
round-trip was 77 µs. Loopback could not have answered this question — only the
real radio could.

## Compression: works, and pays

- Wire ratio on the real corpus: **2.23x**
- zstd-1 on laptop A: 318–554 MB/s, roughly 20x faster than the link
- Entropy heuristic: **32/32 correct**, clean separation (compressible ≤ 4.73
  bits/byte, incompressible ≥ 7.997)
- Threshold kept at 7.5, not the sweep's suggested 6.0 — the two error
  directions have asymmetric cost, and wasting CPU is far cheaper than wasting
  link.

Still outstanding: the same measurement on **laptop B**, which is the machine
that actually does the compressing and has the weaker CPU. Run
`basalt-bench env` and `basalt-bench compress` there.

## Windows file sharing: disqualified on setup, not speed

Never successfully measured for throughput. That turned out not to matter,
because getting SMB working at all required:

- administrator rights to create the share
- changing the network profile from Public to Private
- a firewall rule
- password-protected sharing disabled
- **insecure guest logons enabled on the client** — a real security downgrade

Over an hour, and it still did not work. Our own protocol needed one executable
copied across and one command, and worked first time.

**Zero configuration is now a hard product requirement.** An app that asks people
to disable Windows security settings before it works is a bad app, and that cost
would recur on every device ever added.

## Design targets

- Link: **22.7 MB/s**, RTT **2.3 ms**
- Transport: **TCP, single stream, 2 MiB receive buffer, 64 KiB writes, TLS on**
- Every remaining speed gain must come from **sending fewer bytes**:
  - compression — built, 2.23x
  - batching — built, 7.6x
  - caching — not built; a local hit is ~50x faster than the radio
  - delta sync — not built; edits send only changed blocks

## Still unmeasured

- Laptop B's Wi-Fi radio type and link rate (B may be the weaker end)
- Laptop B's compression speed
- Disk behaviour of the actual USB hard drive (`basalt-bench disk` on B) —
  the concurrency curve there decides whether the host needs an I/O scheduler
