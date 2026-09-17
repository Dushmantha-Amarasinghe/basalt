# Phase 2 — the host app, and no more typing addresses

Two things, and the second is the one that changes how the whole product feels.

---

## The problem with what exists

Pairing today asks for an IP address. That is wrong for a product where both
ends are ours:

- Nobody knows their laptop's address without going and looking.
- Routers hand out different ones. The app caches the last known address, so it
  usually reconnects — but the day the lease changes, the user is back to
  hunting through `ipconfig` on another machine.
- It is the one piece of the setup that cannot be made to work by clicking.

The PIN is different. A PIN is a *deliberate* piece of friction that buys
something real: it stops anyone on the Wi-Fi from reading the drive. It should
stay, but as a switch.

## What replaces it

**The client finds hosts itself, and identifies them by public key rather than
by address.**

A host announces itself on the local network. The client asks who is there,
gets back a list of names, and shows them. Picking one connects.

The important part is what happens afterwards: what gets remembered is the
host's **public key**, not its address. Reconnecting means asking the network
again and connecting to whichever machine presents the key that was pinned. The
address is re-learned every time, so it can change as often as the router likes
and nothing breaks. There is no configuration to get stale.

This also makes discovery safe to be casual about. Anything on the network can
claim to be a Basalt host in a reply — and it will fail the TLS pin check a
moment later. Discovery is a **hint about where to look**, never a statement of
who to trust.

### The beacon

A small protocol of its own, on UDP port 7743.

```text
query   [magic "BSLTd"][version u8][nonce u64]
reply   [magic "BSLTd"][version u8][nonce u64][JSON]
```

The JSON carries only what a person needs in order to choose: host name, vault
name, port, whether a PIN is required, and the host id. Nothing secret — the
host id is a public key hash and is published deliberately.

Hosts also announce unprompted every few seconds, because a query that has to
cross a virtual adapter does not always arrive. Belt and braces: the client
sends a query *and* listens for announcements.

Built rather than using mDNS. mDNS means a dependency, a second responder
fighting Windows' own, and a protocol far larger than "who is out there". This
is about two hundred lines that we control completely, which is the same
reasoning that produced the file protocol.

## Pairing, reshaped

Today: open a pairing window on the host, read a PIN, type it into the client.

Tomorrow, with the PIN switched **off**: pick the drive in the client, confirm,
done.

With the PIN switched **on**: pick the drive in the client, and it asks for a
PIN. At the same moment the host shows *that request* — the device's name, and
the PIN it expects. Read it across, type it in, and the host shows the device as
connected.

The difference is that the host no longer has to be prepared in advance. A
request arrives, and the host displays what to do about it.

**The PIN defaults to on.** A drive with everything on it, readable by anyone
who joins the Wi-Fi, is not a default to choose on someone's behalf — but it is
one click to turn off, and the host says plainly what that means.

## The host app

A Tauri app sharing the client's design system: the same palette, the same
title bar, the same motion.

- **First run** — pick a drive, name it. That is the setup.
- **Dashboard** — the drive and its space; every paired device with its live
  transfer rate; pairing requests as they arrive.
- **Devices** — rename, make read-only, remove. Removing means that device has
  to pair again.
- **Settings** — require a PIN, start with Windows, port, the firewall rule.

Live per-device rates mean the host has to account for bytes per connection,
which it does not do today. That is a counter on the connection, attributed to
the device that authenticated it.

### Start with Windows

A value under `HKCU\…\CurrentVersion\Run`. Per-user, no administrator, and
removable from the same switch that set it.

### Installing

`tauri build` produces an NSIS installer. The host becomes a normal Windows
program: install, launch, pick a drive.

---

## Order of work

1. The beacon — protocol, host responder, client scanner, tests.
2. Per-device byte accounting on the host.
3. Pairing requests: PIN optional, request visible on the host.
4. The client connects by discovery rather than by address.
5. The host app.
6. Start with Windows, and the installer.

## What is deliberately not in this

Remote access, any kind of relay, and discovery across subnets. The beacon is
link-local by design; a host on another network is a VPN's problem, not this
protocol's.
