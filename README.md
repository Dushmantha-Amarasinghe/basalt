<div align="center">
  <h1>Basalt</h1>
  <p><b>One drive, on every device in the house. No addresses, no accounts, no setup.</b></p>
  <p>
    <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-blue?style=flat-square" alt="Windows 10/11" />
    <img src="https://img.shields.io/badge/Android-8.0%2B-3ddc84?style=flat-square" alt="Android 8.0+" />
    <img src="https://img.shields.io/github/license/Dushmantha-Amarasinghe/basalt?style=flat-square" alt="License" />
    <img src="https://img.shields.io/github/v/release/Dushmantha-Amarasinghe/basalt?style=flat-square" alt="Release" />
    <img src="https://img.shields.io/github/downloads/Dushmantha-Amarasinghe/basalt/total?style=flat-square" alt="Downloads" />
  </p>
  <p>
    <a href="https://github.com/Dushmantha-Amarasinghe/basalt/releases/latest">Download</a> &nbsp;·&nbsp;
    <a href="https://reforatech.com">Refora Technologies</a>
  </p>
</div>

<br/>

## Overview

**Basalt** by Refora Technologies turns one spare machine into a drive your
other devices can use. It comes in two halves: **Basalt Host** runs on the
machine with the drive, and **Basalt** runs everywhere else — on Windows PCs,
and on Android phones and tablets.

The point of it is that there is nothing to configure. You pick a drive on the
host; on another device you pick the host from a list and read a PIN across.
That is the whole setup. No IP address is ever typed, no account is made, and
no Windows sharing settings are touched — your devices find the host by
themselves and keep finding it when the router hands it a different address,
because what they trust is its pinned identity rather than where it happens to
be today.

## Screenshots

<div align="center">
  <img src="docs/screenshots/host.png" alt="Basalt Host — sharing a drive" width="420" />
  <img src="docs/screenshots/files.png" alt="Browsing the drive" width="420" />
</div>
<div align="center">
  <img src="docs/screenshots/series.png" alt="The TV Series library" width="420" />
  <img src="docs/screenshots/about.png" alt="Settings" width="420" />
</div>

## Key Features

- **Finds itself.** The host announces itself on the local network and the
  client lists what it finds. Pairing is a PIN read from one screen to the
  other, once. Addresses change freely afterwards and nothing breaks.
- **Pinned, encrypted, private.** Every connection is TLS 1.3, and the client
  pins the host's public key on first pairing — an imposter on the same network
  is refused rather than trusted. Nothing leaves your network, and there is no
  cloud account anywhere in the design.
- **Browse the whole drive.** Files, folders, search, copy, move, rename,
  delete, and select several by dragging a box around them. Upload files or
  whole folders by dropping them onto whichever folder you want. Transfers are
  compressed where that helps, batched for small files, and verified with
  BLAKE3 end to end.
- **Photos, music and videos, sorted.** The host sorts every photo, song and
  video on the drive, however deep, into their own sections. Photos are laid
  out in rows by month at their own shapes, with a viewer that zooms; videos
  show a picture from inside them; music is a track list by artist and album.
  Thumbnails are made once, on the host, and shared by every device.
- **Films and series, recognised.** Turn it on and the host reads the drive and
  files what it finds under Movies and TV Series, with seasons and episodes in
  order. Every film is checked against a bundled catalogue of released titles,
  so screen recordings and home videos stay out of Movies — offline, with
  nothing sent anywhere. Posters are optional and need no API key.
- **A real player.** Built on **mpv**, so it plays what a browser cannot —
  HEVC, E-AC3, DTS, MKV, and the rest — without the host transcoding anything.
  Click to pause, arrow keys to seek and change volume, `,` and `.` to step one
  frame at a time, and `C` for subtitles. Subtitles are named by language,
  remembered on or off from one video to the next, found beside the video on
  the drive or dropped onto it, and nudged into sync when they drift.
- **Profiles, if you want them.** Pick a profile after connecting and its
  watch history and stars follow you to every device in the house: start
  something on the laptop, finish it on the television. A profile is a name, a
  colour and a PIN, stored hashed on the host, and "keep me signed in" means
  typing it once per device. Or skip it and carry on as the device, with a
  history of its own, as before.
- **Carries on where you left off.** Resume points live on the host, and
  episodes play on to the next one by themselves.
- **Live, both ways.** The host watches the drive itself, so a file added,
  renamed or deleted — by Basalt, by Explorer, or by anything else — reaches
  every connected device at once, and a new film or episode is filed under
  Movies or TV Series the moment it lands.
- **A drive that comes and goes.** Unplug the host's drive and it says so, on
  the host and on every device; plug it back in and it is shared again, with
  nothing to redo.
- **On your phone and tablet too.** The Android app is the same app, laid out
  for touch: tabs along the bottom on a phone and a rail down the side on a
  tablet, a tap to open, a long press to choose several. Upload from the
  phone's own picker or share straight into Basalt from any other app;
  downloads land in the phone's Download folder. Films play in the app on the
  same mpv as the desktop — full screen, turned to suit the picture, with a
  double tap either side to skip — and photos pinch, swipe and zoom. A
  transfer or a song carries on with the screen off.
- **Several devices at once.** There is no device limit and no connection
  limit; the host serves bytes and nothing more, so more viewers cost it
  almost nothing.

## Technical Stack

| Part | Built with |
|---|---|
| Host and client cores | Rust 2024, Tokio |
| Transport | TCP, TLS 1.3 (rustls + ring), SPKI pinning, custom binary protocol |
| Discovery | UDP beacon on the local network |
| Desktop shells | Tauri v2, React 19, TypeScript, Tailwind, Framer Motion |
| Android app | Tauri v2 mobile, the same React interface, a Kotlin plugin |
| Playback | libmpv, on Windows and Android |
| Integrity | BLAKE3 per transfer, SHA-256 on updates |

## Installation

Download the latest installers from the
[releases page](https://github.com/Dushmantha-Amarasinghe/basalt/releases/latest):

- **[`Basalt-Host-Setup.exe`](https://github.com/Dushmantha-Amarasinghe/basalt/releases/latest/download/Basalt-Host-Setup.exe)** — on the machine with the drive.
- **[`Basalt-Client-Setup.exe`](https://github.com/Dushmantha-Amarasinghe/basalt/releases/latest/download/Basalt-Client-Setup.exe)** — on every Windows PC that should reach it.
- **[`Basalt-Android.apk`](https://github.com/Dushmantha-Amarasinghe/basalt/releases/latest/download/Basalt-Android.apk)** — on Android phones and tablets (Android 8.0
  or later, 64-bit).

Both Windows installers install per-user and need no administrator. Each
release also publishes a `.sha256` beside each file if you want to check what
you downloaded.

The Android app is not on the Play Store. Open the `.apk` on the phone and
allow your browser or file manager to install it when Android asks; that
permission is only for installing, and can be switched off again afterwards.

Then: open Basalt Host, pick a drive, and open Basalt on another device. It
will list the host; select it and type the PIN the host shows.

All three apps check for updates on their own and will tell you what is in the
new version before you install it. On Android the update is downloaded,
checked against its published checksum, and handed to Android's own installer;
the first time, Android asks you to allow Basalt to install it.

## Configuration & Usage

Everything Basalt keeps lives in `%APPDATA%\Basalt\`:

| File | What it is |
|---|---|
| `host.json` | The host's identity, its settings and its paired devices |
| `client.json` | The vault this device is paired with |
| `host.log` | The host's log, replaced at each start |
| `library-*.json` | The media index, rebuilt by a scan |
| `progress-*.json` | Where each file was watched to |
| `art/` | Downloaded posters |

Deleting `host.json` regenerates the host's identity, which un-pairs every
device. The rest can be deleted freely.

On Android the app keeps what it needs in its own private storage, which is
left out of phone backups, so its pairing never travels to another device.
Clearing the app's storage, or **Forget this drive** under **More**, means
pairing again.

**Optional, and off by default:** recognising films and series reads the whole
drive, and downloading posters sends each recognised title to a lookup service.
Neither happens until you turn it on.

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
libmpv-android build of it — and links a number of open-source libraries. See `THIRD-PARTY-NOTICES.txt` for full attribution.

---

<div align="center">
  <p>Crafted by <b>Refora Technologies</b></p>
  <p><a href="https://reforatech.com">reforatech.com</a></p>
</div>
