# How to run Basalt

Two programs. One on the laptop with the drive, one on the machine you browse
from.

**There is no address to type.** The client finds the host on the network by
itself, and keeps finding it after the router hands out a different address.

---

## On the laptop with the drive — Basalt Host

Run the installer:

```
apps\host\src-tauri\target\release\bundle\nsis\Basalt Host_0.1.0_x64-setup.exe
```

It installs for your account only, so Windows does not ask for an
administrator.

Plug in the drive and open **Basalt Host**. It lists the drives on the machine
with their labels and how much room is left. Click the one you want, give it a
name your other devices will see, and press **Share**.

That is the setup.

The window then shows the drive, the devices paired with it, what each has
moved and how fast it is going right now. Three settings live at the bottom:

- **Ask for a PIN when pairing** — on by default. A new device appears on this
  screen with a six-digit number to type on that device. With it off, anything
  on your network that finds this machine can read the drive without being let
  in, and the app says so.
- **Start when Windows starts** — comes up in the notification area at login,
  so the drive is there before you go looking for it.
- **This machine's name** — what your devices see in their list.

**Closing the window keeps the drive shared.** It goes to the notification
area; click the icon to bring it back, or right-click it for **Quit and stop
sharing**.

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

1. It lists the Basalt hosts answering on this network, by name. Click yours.
2. If the host is asking for a PIN, the six digits appear **on the host's
   screen**, next to the name of the device asking. Type them in.

That is the last time you do any of this. From then on the app reconnects on
its own whenever the host is up — including after the router gives the host a
different address, which it finds again by itself.

> The client's own pairing screen is still being rewritten around this. Until
> that lands, `basalt find` from the command line lists what is out there and
> `basalt pair <address>` joins it.

---

## If something goes wrong

**The client lists no hosts at all**
Check Basalt Host is still running on the other machine — look in the
notification area, not just the taskbar. Both machines have to be on the same
network, and some routers have a "client isolation" or "AP isolation" setting
that stops them talking to each other at all. If the host is running and the
list is still empty, the firewall prompt was probably dismissed; see below.

**It worked yesterday and not today**
This is the case the whole design is built around, and it should just work: the
client finds the host again wherever it has moved to. If it does not, the host
is not running.

**"That pairing request has expired"**
Requests last three minutes. Ask again from the client and a fresh number
appears on the host.

**The host says it is not sharing**
Another copy is probably already running — check the notification area before
starting a second one. The window says so at the top when that is what
happened.

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

The app detects this and says so, and offers **Play in your player**. That
**streams** — the player is handed a local URL and seeks through it with range
requests, so a 3 GB episode starts at once and nothing is written to your disk.

It looks for VLC, mpv, MPC-HC and PotPlayer, wherever they are installed. To
see which one it found:

```bash
basalt.exe player
```

You can also do it from the command line, or get a URL to paste in yourself:

```bash
basalt.exe play "Season 1/Episode 1.mkv"
basalt.exe url "Season 1/Episode 1.mkv"
```

**A video will not open at all**
AVI, MOV and WMV are containers the window cannot read. Same answer: **Play in
your player**.

**"No player found"**
Install VLC or mpv. Until then `basalt url` prints an address you can paste
into anything.

---

## Without a window

There is a command-line client too, which is the quickest way to tell whether a
problem is the network or the app:

```
target\release\basalt.exe find
target\release\basalt.exe probe 192.168.1.11
target\release\basalt.exe pair 192.168.1.11
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
| Host identity, drive and paired devices | `%APPDATA%\Basalt\host.json` on the host |
| Paired hosts and their tokens | `%APPDATA%\Basalt\client.json` on the client |
| The startup entry | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Basalt Host` |

Deleting the host's file changes its identity, and every device has to pair
again. Both files are worth the same care as a password manager's.

---

## The old benchmark harness

`basalt-bench.exe` measured whether any of this was worth building. It is not
needed to run Basalt; the numbers it produced are written up in
[`docs/measured-facts.md`](docs/measured-facts.md).
