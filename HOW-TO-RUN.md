# How to run Basalt

Two programs. One on the laptop with the drive, one on the machine you browse
from. Nothing to install on either.

---

## On the laptop with the drive — Basalt Host

Copy one file across:

```
target\release\basalt-host.exe
```

Plug in the drive, note the letter Windows gives it (open **This PC** and look),
then run:

```powershell
.\basalt-host.exe serve --path E:\ --name "My Drive"
```

Replace `E:\` with your drive letter. It prints something like:

```
  Basalt Host
  sharing   E:\
  as        My Drive
  identity  7e508aaa
  port      7742
  reachable at  192.168.1.11

  No devices paired yet.
  Pairing PIN:  653 443
```

**Leave the window open** — it has to keep running to serve the drive.

Two things to write down: the **address** (`192.168.1.11`) and the **PIN**.

The PIN lasts three minutes. Press **Enter** in that window any time for a new
one. Type `d` and Enter to see which devices are paired.

### The one thing Windows will ask

The first time it runs, Windows shows a **Windows Security Alert** asking
whether to allow it through the firewall. Tick both **Private** and **Public**,
then **Allow access**. That needs an administrator click, once, on this machine
only.

That is the entire setup. No network profile changes, no credentials, no
sharing settings, nothing on any other machine.

## On the machine you browse from — Basalt

Run:

```
apps\client\src-tauri\target\release\basalt-client-shell.exe
```

It opens on the pairing screen.

1. Type the **address** from the host window. Press Enter.
2. It shows what it found — the host's name, the drive, and its identity.
   Check the identity matches the one on the host window before continuing:
   after this it is trusted permanently and never asked about again.
3. Type the **PIN**.

That is the last time you do any of this. From then on the app reconnects on
its own whenever the host is up.

---

## If something goes wrong

**Nothing answers at that address**
Check the host window is still open, and that you used the address it printed.
If the laptop has several adapters it prints more than one — the right one
usually starts `192.168.`. If it still fails, the firewall prompt was probably
dismissed; see below.

**It worked yesterday and not today**
The router most likely gave the host a different address. Open the host window,
read the new one, and enter it in the app.

**"This host is not accepting new devices"**
The three-minute pairing window has closed. Press Enter in the host window for
a new PIN.

**"This is not the host this device paired with"**
The app is refusing a machine that is not the one you paired with. Either
something else is now at that address, or the host's config file was deleted
and it generated a new identity. If you know why, pair again from Settings.

**The firewall prompt never appeared, or was dismissed**
Run this once on the host machine, in PowerShell **as administrator**:

```powershell
New-NetFirewallRule -DisplayName "Basalt Host" -Direction Inbound -Protocol TCP -LocalPort 7742 -Action Allow
```

**A video plays but there is no sound**
Almost always an MKV. The window is Chromium, and Chromium only reads Matroska
in order to support WebM — so it accepts WebM's codecs and silently drops
everything else. An MKV with an ordinary AAC track therefore shows a picture
and no audio, even though the very same AAC inside an MP4 plays perfectly.

The app detects this and says so, and offers **Open in your player** — it
fetches the file and hands it to whatever you normally watch films with.

To stream it instead of downloading, get a URL and paste it into VLC or mpv:

```bash
basalt.exe url films/holiday.mkv
```

That serves the file over a local address with full seeking, and a real player
gets every track including the audio. Verified on a 2.2 GB 4K HEVC file:
seeking ten minutes in took about two seconds.

**A video will not open at all**
AVI, MOV and WMV are containers the window cannot read. Same answer: **Open in
your player**, or use `basalt url`.

---

## Without a window

There is a command-line client too, which is the quickest way to tell whether a
problem is the network or the app:

```
target\release\basalt.exe probe 192.168.1.11
target\release\basalt.exe pair 192.168.1.11 653443
target\release\basalt.exe ls
target\release\basalt.exe get films/holiday.mp4 C:\Users\you\Downloads\holiday.mp4
target\release\basalt.exe put C:\Users\you\clip.mp4 films/clip.mp4
```

It drives exactly the same code the app does, so anything that works here works
there — and every failure is printed in full rather than turned into a banner.

---

## Where things are kept

| | |
|---|---|
| Host identity and paired devices | `%APPDATA%\Basalt\host.json` on the host |
| Paired hosts and their tokens | `%APPDATA%\Basalt\client.json` on the client |

Deleting the host's file changes its identity, and every device has to pair
again. Both files are worth the same care as a password manager's.

---

## The old benchmark harness

`basalt-bench.exe` measured whether any of this was worth building. It is not
needed to run Basalt; the numbers it produced are written up in
[`docs/measured-facts.md`](docs/measured-facts.md).
