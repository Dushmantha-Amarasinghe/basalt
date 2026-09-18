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
moved and how fast it is going right now. Four settings live at the bottom:

- **Ask for a PIN when pairing** — on by default. A new device appears on this
  screen with a six-digit number to type on that device. With it off, anything
  on your network that finds this machine can read the drive without being let
  in, and the app says so.
- **Start when Windows starts** — comes up in the notification area at login,
  so the drive is there before you go looking for it.
- **Recognise films and series** — off by default. When on, the host reads
  through the drive and works out which files are films and which are
  episodes, so your devices get **Movies** and **TV Series** sections instead
  of only folders. It rescans by itself whenever the drive changes, so
  anything you add, rename or delete turns up without being asked.

  Underneath it, **Download posters** takes a free
  [TMDb](https://www.themoviedb.org/settings/api) API key. Leave it empty and
  the app draws its own covers instead — which is the default, because looking
  titles up means sending every one of them to a third party, and a list of
  titles is a list of what you watch.
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

Run the installer:

```
apps\client\src-tauri\target\release\bundle\nsis\Basalt_0.1.0_x64-setup.exe
```

It opens on the pairing screen.

1. It lists the Basalt hosts answering on this network, by name. Click yours.
2. If the host is asking for a PIN, the six digits appear **on the host's
   screen**, next to the name of the device asking. Type them in.

That is the last time you do any of this. From then on the app reconnects on
its own whenever the host is up — including after the router gives the host a
different address, which it finds again by itself.

### What you see once you are in

**Everything is live.** Delete a file on the host in Explorer and it vanishes
here; rename one and the name changes. It works the other way too, and between
two devices at once, because the host reports what the drive actually did
rather than what it was asked to do.

**Movies and TV Series** appear in the sidebar when the host has the library
switched on. Series open into seasons and episodes. Posters are drawn from the
title rather than downloaded — nothing is sent to anyone about what is on your
drive. A film the app is unsure about is labelled *a guess* rather than filed
silently under the wrong name, and everything stays in **Files** regardless,
recognised or not.

A host that has not been given a drive yet is still listed, greyed out and
labelled, rather than left out with no explanation.

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

**Posters are missing for some titles**
Only what TMDb recognises gets one; everything else keeps the cover the app
draws. A title it got wrong is usually a parsing problem rather than a TMDb
one — check the name against the shapes below.

**A film is missing from Movies, or filed under the wrong name**
The index reads the path, not the file. `Arrival (2016).mkv` and
`Show/Season 01/S01E01.mkv` are the shapes it knows; something named
`video1.mkv` has nothing to go on and stays in Files. Anything it was unsure
about is marked *a guess*. Extras, trailers, samples and anything under 50 MB
are left out on purpose.

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
target\release\basalt.exe status
target\release\basalt.exe ls
target\release\basalt.exe get films/holiday.mp4 C:\Users\you\Downloads\holiday.mp4
target\release\basalt.exe put C:\Users\you\clip.mp4 films/clip.mp4
```

It drives exactly the same code the app does — `find` is literally the call the
pairing screen makes — so anything that works here works there, and every
failure is printed in full rather than turned into a banner.

`probe` and `pair` still take an address, unlike the app. That is deliberate:
when discovery is the thing that is broken, a tool that depends on discovery
cannot tell you why.

---

## Where things are kept

| | |
|---|---|
| Host identity, drive and paired devices | `%APPDATA%\Basalt\host.json` on the host |
| The media index | `%APPDATA%\Basalt\library-*.json` on the host, one per drive |
| Downloaded posters | `%APPDATA%\Basaltrt\` on the host |
| Paired hosts and their tokens | `%APPDATA%\Basalt\client.json` on the client |
| The startup entry | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Basalt Host` |

Deleting the host's file changes its identity, and every device has to pair
again. Both files are worth the same care as a password manager's.

---

## The old benchmark harness

`basalt-bench.exe` measured whether any of this was worth building. It is not
needed to run Basalt; the numbers it produced are written up in
[`docs/measured-facts.md`](docs/measured-facts.md).
