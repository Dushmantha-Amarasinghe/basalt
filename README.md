<div align="center">
  <img src="apps/client/src-tauri/icons/128x128@2x.png" alt="Basalt" width="88" />
  <h1>Basalt</h1>
  <p><b>Your home drive, on every screen in the house.<br/>No cloud, no subscription, no account.</b></p>
  <p>
    <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-blue?style=flat-square" alt="Windows 10/11" />
    <img src="https://img.shields.io/badge/Android-8.0%2B-3ddc84?style=flat-square" alt="Android 8.0+" />
    <img src="https://img.shields.io/github/license/refora-technologies/basalt?style=flat-square" alt="License" />
    <img src="https://img.shields.io/github/v/release/refora-technologies/basalt?style=flat-square" alt="Release" />
    <img src="https://img.shields.io/github/downloads/refora-technologies/basalt/total?style=flat-square" alt="Downloads" />
  </p>
  <p>
    <a href="https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Host-Setup.exe"><b>Host for Windows</b></a> &nbsp;·&nbsp;
    <a href="https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Client-Setup.exe"><b>Basalt for Windows</b></a> &nbsp;·&nbsp;
    <a href="https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Android.apk"><b>Basalt for Android</b></a>
  </p>
  <p>
    <a href="https://basalt.reforatech.com">basalt.reforatech.com</a> &nbsp;·&nbsp;
    <a href="https://github.com/refora-technologies/basalt/releases/latest">Release notes</a> &nbsp;·&nbsp;
    <a href="https://reforatech.com">Refora Technologies</a>
  </p>
</div>

<br/>

<p align="center">
  <img src="docs/screenshots/library.jpg" alt="Basalt for Windows: the Movies library, with Continue watching" width="900" />
</p>

## Overview

**Basalt** by Refora Technologies turns one Windows PC into a private drive
for the whole household. **Basalt Host** runs on the machine with the drive;
**Basalt** runs everywhere else, on Windows PCs and on Android phones and
tablets. Every device browses the same files, watches the same library, and
picks up where the last one left off.

There is nothing to configure. You pick a drive on the host; on another device
you pick the host from a list and type the PIN it shows. That is the whole
setup. No IP address is ever typed, no account is made, and no Windows sharing
settings are touched. Devices find the host by themselves and keep finding it
when the router hands it a different address, because what they trust is its
pinned identity rather than where it happens to be today.

Nothing leaves your home. There is no Basalt server and no cloud in the
design: the host serves your files to your devices over your own network,
encrypted, and that is all it does.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/player.jpg" alt="The player, with subtitles" /></td>
    <td width="50%"><img src="docs/screenshots/photos.jpg" alt="Photos, in rows by month" /></td>
  </tr>
  <tr>
    <td align="center"><sub>The player: streamed from the drive, every format, every subtitle track</sub></td>
    <td align="center"><sub>Photos from every folder, by month, at their own shape</sub></td>
  </tr>
</table>

<p align="center">
  <img src="docs/screenshots/phone.jpg" alt="Basalt for Android: files, the library, a series and photos" width="900" />
  <br/>
  <sub>Basalt for Android: the same drive, library and profiles, laid out for touch</sub>
</p>

<p align="center">
  <img src="docs/screenshots/host.jpg" alt="Basalt Host: the shared drive, a device asking to join, and who is connected" width="620" />
  <br/>
  <sub>Basalt Host: the shared drive, a new device asking to join, and everyone connected</sub>
</p>

## Key Features

### Set up once, then forget it

- **Finds itself.** The host announces itself on the local network and every
  device lists what it finds. If a network blocks that, a device can be given
  the host's address instead.
- **Nobody joins without your say-so.** A new device shows up on the host with
  a PIN to type on that device, once. After that it simply connects, and the
  host can make any device read-only or let it go.
- **A drive that comes and goes.** Unplug the host's drive and every device
  says so; plug it back in and it is shared again, with nothing to redo.
- **Keeps itself up to date.** Every app checks for new versions, shows what
  changed, and installs only after checking the download against its published
  checksum.

### Private by design

- **Encrypted and pinned.** Every connection is TLS 1.3, and each device pins
  the host's key when it pairs, so a different machine at the same address is
  refused rather than trusted.
- **Local only.** No account, no cloud, no telemetry. Files travel between your
  own devices and nowhere else. The only requests that leave your network are
  the update check, to GitHub, and poster lookups if you turn them on.
- **Careful with what it keeps.** Profile PINs are stored only as slow, salted
  hashes, and repeated wrong guesses lock a profile for longer each time.
  Sign-ins kept on a Windows PC are encrypted for that Windows account, and on
  Android they stay in the app's private storage, out of phone backups.

### The whole drive, from anywhere in the house

- **Browse everything.** Files, folders, search, copy, move, rename, delete,
  and select several by dragging a box around them.
- **Upload the easy way.** Drop files or whole folders onto the folder you
  want. On a phone, upload from the gallery, any file or a folder, or share to
  Basalt from any other app.
- **Fast and verified.** Transfers are compressed where that helps, batched
  for small files, and checked with BLAKE3 end to end.
- **Live.** A file added, renamed or deleted on the host, by Basalt or anything
  else, reaches every connected device at once.

### Films and series, filed for you

- **Recognised automatically.** Turn it on and the host files the films and
  series on the drive under Movies and TV Series, with seasons and episodes in
  order, and new ones the moment they are copied in. Every film is checked
  against a bundled catalogue of released titles, offline, so screen recordings
  and home videos stay out of Movies.
- **Posters and picture quality.** Optional posters need no API key. Films,
  series and episodes are tagged HD, Full HD, 2K or 4K from the video itself,
  not from its file name.
- **Continue watching.** Resume points live on the host, so a film started on
  the laptop finishes on the phone, and episodes play on to the next one.
- **Photos, music and home videos.** The host finds every photo, song and video
  on the drive, however deep, and sorts each into its own section. Photos are
  laid out by month at their own shape with a viewer that zooms; music is a
  track list by artist and album that plays on; videos show a picture from
  inside them. Thumbnails are made once, on the host, and shared by every
  device.
- **Choose what devices show.** The host decides which sections appear on
  your devices. Hiding one only tidies the sidebar; every file stays in Files.

### A real player

- **Plays what a browser cannot.** Built on **mpv**, on Windows and Android
  alike, so HEVC, HDR, E-AC3, DTS, MKV and the rest play straight from the
  drive, with nothing downloaded first and nothing transcoded on the host.
- **Subtitles done properly.** Tracks named by language, remembered on or off
  from one video to the next, found beside the video on the drive or dropped
  onto it, and nudged into sync when they drift.
- **Made for the keyboard.** Space to pause, arrow keys to seek and change
  volume, `C` for subtitles, Shift+P and Shift+N for the previous and next
  episode, and `,` and `.` to step one frame at a time. Pausing shows what is
  playing and what comes next.

### Profiles for the household

- **Everyone gets their own.** A profile is a name, a colour and a PIN. Its
  Continue watching and starred files follow it to every device, and "keep me
  signed in" means typing the PIN once per device.
- **Or skip it.** Carry on as the device, with a history of its own.
- **Managed from the host.** See who is signed in where, reset a forgotten PIN,
  or remove a profile along with its history.

### On your phone and tablet

- **The same app, laid out for touch.** Tabs along the bottom on a phone and a
  rail down the side on a tablet; tap to open, long-press to choose several.
- **Films in your hand.** Full screen, turned to suit the picture, with a double
  tap either side to skip. Or hand the stream to another player such as VLC.
- **Photos by touch.** Pinch to zoom, swipe to the next, swipe down to put it
  away.
- **Carries on with the screen off.** A transfer or a song keeps going while
  the phone is locked, and downloads land in the phone's Download folder.
- **Stays on the Wi-Fi.** On a Wi-Fi network without internet access, where a
  phone would normally switch to mobile data, Basalt keeps its connection to
  the host.

### Several devices at once

There is no device limit and no connection limit. The host serves bytes and
nothing more, so more viewers cost it almost nothing.

## Technical Stack

| Part | Built with |
|---|---|
| Host and client cores | Rust 2024, Tokio |
| Transport | TCP, TLS 1.3 (rustls + ring), SPKI pinning, custom binary protocol |
| Discovery | UDP beacon on the local network |
| Desktop apps | Tauri v2, React 19, TypeScript, Tailwind, Framer Motion |
| Android app | Tauri v2 mobile, the same React interface, a Kotlin plugin |
| Playback and thumbnails | libmpv, on Windows and Android |
| Film recognition | A bundled catalogue of film and series titles from Wikidata |
| Integrity | BLAKE3 per transfer, SHA-256 on updates |

## Installation

Download the latest version:

| Download | Install on |
|---|---|
| **[`Basalt-Host-Setup.exe`](https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Host-Setup.exe)** | The Windows PC with the drive |
| **[`Basalt-Client-Setup.exe`](https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Client-Setup.exe)** | Every Windows PC that should reach it |
| **[`Basalt-Android.apk`](https://github.com/refora-technologies/basalt/releases/latest/download/Basalt-Android.apk)** | Android phones and tablets (Android 8.0 or later, 64-bit) |

These links always give the newest release. Each file has a `.sha256` beside
it on the [releases page](https://github.com/refora-technologies/basalt/releases/latest)
if you want to check what you downloaded.

Both Windows installers install per user and need no administrator rights.
They update an existing installation in place and keep its pairing and
settings.

The Android app is not on the Play Store. Open the `.apk` on the phone and
allow your browser or file manager to install it when Android asks; that
permission is only for installing, and can be switched off again afterwards.

**Getting started:**

1. Install Basalt Host on the PC with the drive, open it, and choose the drive
   to share.
2. Open Basalt on another device. It lists the host; select it.
3. Type the PIN the host shows. Done.

All three apps check for updates on their own and show what is in the new
version before you install it. On Android the update is downloaded, checked
against its published checksum, and handed to Android's own installer; the
first time, Android asks you to allow Basalt to install it.

## Configuration & Usage

Everything Basalt keeps on Windows lives in `%APPDATA%\Basalt\`:

| File | What it is |
|---|---|
| `host.json` | The host's identity, settings, paired devices and profiles |
| `client.json` | The drive this device is paired with |
| `host.log` | The host's log, replaced at each start |
| `library-*.json`, `collections-*.json` | The media index, rebuilt by a scan |
| `watched-*.json`, `stars-*.json` | Watch history and starred files, per device and per profile |
| `resolutions-*.json` | Each video's measured picture size |
| `thumbs/`, `art/` | Thumbnails, and downloaded posters |

Deleting `host.json` regenerates the host's identity, which un-pairs every
device and removes its profiles. The rest can be deleted freely and is
rebuilt as needed, apart from watch history and stars.

On Android the app keeps what it needs in its own private storage, which is
left out of phone backups, so its pairing never travels to another device.
Clearing the app's storage, or **Forget this drive** under **More**, means
pairing again.

**Optional, and off by default:** recognising films and series reads the whole
drive, and downloading posters sends each recognised title to a lookup service.
Neither happens until you turn it on in Basalt Host.

## Building from source

```
rustup toolchain install stable          # Rust 1.98+ MSVC
cd apps/client/src-tauri && pwsh -File fetch-libmpv.ps1
cd apps/host/src-tauri   && pwsh -File fetch-libmpv.ps1
cd apps/client && npm install && npx tauri build
cd apps/host   && npm install && npx tauri build
```

`fetch-libmpv.ps1` downloads libmpv and a small wrapper into `src-tauri/lib/`
and verifies the wrapper's checksum. They are not in the repository because
`libmpv-2.dll` is 96 MB. The host uses the same libmpv to make thumbnails of
videos; its script copies the client's rather than downloading it again.

The host recognises films against a catalogue of every film and series title
on Wikidata, bundled so that it works offline and sends nothing anywhere. It is
committed at `crates/basalt-catalog/data/catalog.bin` (about 3 MB) and rebuilt
for each release with:

```
cargo run -p catalog-build --release
```

`cargo test --all` runs the Rust suite; `npm test` in either app runs its own.

### The Android app

It needs the Android SDK with NDK 28, JDK 17, and Rust's Android targets:

```
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
set NDK_HOME=%LOCALAPPDATA%\Android\Sdk\ndk\28.2.13676358
set JAVA_HOME=C:\Program Files\Java\jdk-17
cd apps/client && npx tauri android build --apk --target aarch64
```

The phone's player is mpv too, from the `dev.jdtech.mpv:libmpv` package, which
Gradle fetches by itself; the desktop's Windows libraries are left out.

Release builds are signed with a key kept outside the repository. Gradle reads
its location and passwords from
`%USERPROFILE%\.basalt\android-signing\keystore.properties`, or from the file
`BASALT_SIGNING` names:

```
storeFile=D:/path/to/basalt-release.keystore
storePassword=...
keyAlias=basalt
keyPassword=...
```

Without it the build is left unsigned. Keep the key safe: Android installs an
update only over an app signed with the same key.

`npx tauri android build --debug --apk --target x86_64` builds for the
emulator. After regenerating icons with `npx tauri icon`, run
`python tools/make-icons.py` again so the Android project has its own.

## License

This project is licensed under the **GNU General Public License v3.0 (GPLv3)**.

You are free to use, modify and distribute this software, provided any
derivative works are also open-source under the identical terms. See the
`LICENSE` file for the complete terms.

Basalt bundles libmpv (LGPL-2.1-or-later) — and, in the Android app, the
libmpv-android build of it — and links a number of open-source libraries. See
`THIRD-PARTY-NOTICES.txt` for full attribution.

---

<div align="center">
  <p>Crafted by <b>Refora Technologies</b></p>
  <p><a href="https://basalt.reforatech.com">basalt.reforatech.com</a> &nbsp;·&nbsp; <a href="https://reforatech.com">reforatech.com</a></p>
</div>
