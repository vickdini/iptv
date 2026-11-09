# IPTV

Watch free, publicly available IPTV channels like a TV: zap with Up/Down, type a
channel number, jump back to the previous channel, or pick from a scrollable
channel list. Channels that don't work are found automatically and hidden.

By default the app uses the public [iptv-org](https://github.com/iptv-org/iptv)
playlist, `https://iptv-org.github.io/iptv/index.m3u` (about 11,500 channels, of
which roughly three quarters work at any given time).

Runs on Windows and Android (phones, tablets and Android TV); Linux and macOS
work from source.

> [!IMPORTANT]
> **Disclaimer.** This project does not host, provide, sell or modify any
> streams or video content. By default it only loads the publicly available
> M3U playlist published by the iptv-org project, which links to channels
> that their broadcasters make freely available on the internet. The playlist
> is maintained by a third party; we do not control its contents.
>
> This project does not promote, encourage or support piracy or any other
> illegal activity. Do not use it to access content you are not legally
> allowed to watch in your country. If you load a different playlist (the
> `--playlist` option), you alone are responsible for making sure you have
> the right to watch what it contains.
>
> The software is provided "as is", without warranty of any kind (see sections
> 15 and 16 of the [license](LICENSE)). The authors and contributors are not
> responsible for how anyone uses it, or for any damage or legal consequences
> arising from its use.

## Install (Windows)

**[Download the latest installer](https://github.com/vickdini/iptv/releases/latest)**
(`IPTV-Setup-<version>.exe` under *Assets*) and run it. It's a single
installer with everything included (the app and its video engine). Nothing
else needs to be installed. It:

* installs for the current user only, without asking for administrator rights
* adds a Start-menu shortcut and, optionally, a desktop icon
* can be removed from *Settings → Apps* like any other program

The app opens maximized and starts playing straight away.

The installer isn't code-signed yet, so Windows SmartScreen may say *"Windows
protected your PC"*. Click **More info → Run anyway**. Every installer is
built from this repository's source by GitHub Actions (see the
[workflow](.github/workflows/release.yml) and its run logs). To verify your
download, compare its SHA-256 with `SHA256SUMS.txt` on the release page:

```powershell
Get-FileHash IPTV-Setup-*.exe -Algorithm SHA256
```

## Android

The Android app (phones, tablets and Android TV, Android 7.0 or later) has the
same features: the latest playlist on every launch, dead channels hidden
automatically, channel numbers, groups and favorites. It plays in landscape,
full screen.

| Phone / tablet | Android TV remote | Action |
| --- | --- | --- |
| swipe up / down | Up / Down, Channel +/- | next / previous channel |
| tap | OK | open / close the channel list |
| double-tap | Backspace, Last channel | previous channel |
| long-press | Info | channel info |
| | Left / Right | previous / next group |
| | 0–9 | go to channel number (OK to confirm) |
| | Bookmark | toggle favorite |

In the channel list: tap (or OK) watches a channel, long-press toggles a
favorite, and the ◀ ▶ ★ buttons switch group / favorites only.

**[Download the latest APK](https://github.com/vickdini/iptv/releases/latest)**
(`IPTV-<version>.apk` under *Assets*) on the phone or TV and open it. Android
asks once to allow installing apps from your browser or file manager; allow
it, then tap **Install**. The app isn't on Google Play, so Play Protect may
show a warning for an unknown developer; choose **Install anyway**. Updates
are installed the same way, over the previous version.

The APK is built and signed by GitHub Actions from this repository's source.
Its signing certificate (`CN=IPTV, O=vickdini`) has the SHA-256 fingerprint
`4A:40:E4:CA:75:E2:C2:5E:0A:BC:77:4B:EE:33:02:34:A3:87:1D:7D:CB:8C:FE:A7:66:A5:7A:C6:DD:34:14:61`.

## Using it

### Remote control (in the player window)

| Key | Action |
| --- | --- |
| Up / Down, PgUp / PgDn, mouse wheel | channel up / down |
| 0–9, then wait 2 s or press Enter | go to channel number |
| Backspace | previous channel |
| Left / Right, m | volume, mute |
| l / Tab / right-click | open / close the channel list |
| g / G | next / previous group (Movies, News, Sports…) |
| s / S | toggle favorite / zap only through favorites |
| i | channel info |
| f, double-click | fullscreen (Esc leaves it) |
| h / ? / F1 | help |
| q | quit |

### Channel list

`l` opens a scrollable list of every working channel in the current view (all
channels, or the selected group / favorites) on the left of the window. The
video keeps playing beside it.

| Input | Action |
| --- | --- |
| mouse wheel | scroll (Shift+wheel: a page) |
| hover / Up / Down / PgUp / PgDn / Home / End | select |
| click / Enter | watch the selected channel (the list stays open) |
| click on the scrollbar | jump to that part of the list |
| g / G, S | switch group / favorites only (the list follows) |
| s | toggle favorite on the selected channel |
| 0–9 | go to channel number |
| Esc / l / click on the video | close |

The channel being watched is marked ▶; favorites are marked ★.

### Channel numbers

Channel numbers are positions in the playlist. Hidden (non-working) channels
leave gaps instead of renumbering the others, so a channel's number doesn't
depend on which other channels happen to be online. Numbers can shift when the
upstream playlist changes. The last channel and favorites are remembered by
stream address, so they survive such changes.

## How it works

### Always the latest playlist

On every launch the app asks the playlist server whether the playlist has
changed since the copy it saved last time (an HTTP conditional request using
`ETag` / `Last-Modified`).

* **Unchanged:** the server sends no data (≈60 ms) and the saved copy is used.
* **Changed:** the new version (≈2.6 MB) is downloaded and saved.
* **Offline:** the saved copy is used.

A new copy replaces the old one only once it has downloaded completely, so an
interrupted download never leaves a broken playlist behind.

### Hiding channels that don't work

Many public streams are offline, part-time or geo-blocked. Instead of showing
an error, the app hides them:

* **Background check.** On launch every channel is probed with a quick HTTP
  request, using the channel's own user-agent / referrer. HLS playlists must be
  valid and list segments; for a master playlist the first quality level must
  be reachable too. 24 probes run in parallel, starting with the channels
  nearest to the one you're watching. You can watch while it runs; the
  channel list shows `checking channels NN%` and shrinks as dead channels are
  found. A first full check takes about 7–8 minutes.
* **Remembered.** Results are kept for 6 hours, so later launches hide dead
  channels immediately and only re-check old results. Channels that come back
  online reappear.
* **Failures during playback.** If a channel still fails when tuned, it is
  hidden and the app moves on to the next working channel in the direction you
  were zapping, without a message.
* **When in doubt, shown.** Channels the check can't judge (non-HTTP
  addresses such as `rtmp://`, or HTTPS certificate problems the player
  tolerates) stay visible.

The check doesn't play video, so a stream that passes can still fail to play;
in that case it is skipped as described above.

### Where data is stored

| What | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Last channel, favorites | `%APPDATA%\iptv` | `~/.config/iptv` | `~/Library/Application Support/iptv` |
| Playlist copy, check results | `%LOCALAPPDATA%\iptv` | `~/.cache/iptv` | `~/Library/Caches/iptv` |

Uninstalling removes the playlist copy and check results; favorites are kept,
so a reinstall remembers them.

## Building from source

Requirements:

* Rust (stable)
* [mpv](https://mpv.io), the video engine. The installer bundles it; when
  running from source, install it yourself:
  * Windows: `winget install shinchiro.mpv`
  * macOS: `brew install mpv`
  * Linux: `sudo apt install mpv` / `dnf install mpv` / `pacman -S mpv`

mpv is found automatically: next to the `iptv` executable (also in an `mpv`
subfolder, as the installer lays it out), on `PATH`, or in common install
folders. Override with `--mpv <path>` or the `IPTV_MPV` environment variable.

```sh
cargo run --release                       # resume the last channel
cargo run --release -- --channel 2409     # start on channel 2409
cargo run --release -- --group News       # zap only through News
cargo run --release -- --list cnn         # print matching channels and exit
cargo run --release -- --list-groups      # print groups and exit
cargo run --release -- -p my.m3u          # another playlist (file or URL)
cargo test
```

Other options: `--fullscreen`, `--no-maximize` (normal-sized window),
`--volume <0-100>`, `--no-check` (don't check or hide channels). Set
`IPTV_DEBUG=1` to log key presses and skipped channels.

Release builds on Windows are GUI applications: no console window opens, and
errors are shown in a message box. Run from a terminal, they still print
output there.

### Windows installer

Requires [Inno Setup](https://jrsoftware.org/isinfo.php)
(`winget install JRSoftware.InnoSetup`) and an mpv build to bundle (by
default the one installed by `winget install shinchiro.mpv`).

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
# -> target\installer\IPTV-Setup-<version>.exe
```

The bundled mpv runs with `--config=no`, so a user's own mpv settings never
affect the app. `packaging/windows/make-icon.py` regenerates the icon.

### Publishing a release

Releases are built by GitHub Actions ([release.yml](.github/workflows/release.yml)),
not on a developer's PC:

1. Bump `version` in the root `Cargo.toml` and commit. It is the version of
   both apps (the Android `versionCode` is derived from it).
2. Tag and push: `git tag v0.2.0 && git push origin master v0.2.0`.
3. The workflow runs the tests, builds the Windows installer (with a pinned
   mpv build checked against its SHA-256) and the signed Android APK, and
   publishes a GitHub Release with `IPTV-Setup-0.2.0.exe`, `IPTV-0.2.0.apk`
   and `SHA256SUMS.txt`. It refuses to run if the tag doesn't match
   `Cargo.toml`, and refuses to release an unsigned APK.

*Actions → Release → Run workflow* builds both without publishing them
(download them from the run's artifacts). The bundled mpv version is pinned
by `MPV_URL` / `MPV_SHA256` in the workflow.

**Android signing (one-time setup).** Android only installs an update if it is
signed with the same key as the installed app, so the key must never change
or be lost. It lives outside the repository; GitHub Actions gets it from three
repository secrets (*Settings → Secrets and variables → Actions*):

| Secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | the keystore file (`.jks`), base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | its password (also used for the key) |
| `ANDROID_KEY_ALIAS` | the key alias |

To sign a release build locally, set `ANDROID_KEYSTORE_FILE` (path to the
`.jks`), `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS`, then run
`./gradlew assembleRelease`.

### Building the Android app

Requirements: the Android SDK with NDK 27 (e.g. via Android Studio), JDK 17
or later, and the Rust Android targets plus `cargo-ndk`:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
cd android
./gradlew assembleDebug        # -> app/build/outputs/apk/debug/app-debug.apk (debug-signed)
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Gradle builds the Rust core for arm64, armv7 and x86_64 (emulators)
automatically before the app. `android/make-icons.py` regenerates the
launcher icons and the Android TV banner.

## Architecture

```
crates/
  iptv-core/       platform-independent logic, no I/O (reused by every front-end)
    m3u.rs         extended-M3U parser (tvg-*, group-title, user-agent/referrer)
    tuner.rs       TV state machine: Action (remote button) -> Vec<Effect>;
                   channel numbers, zapping, groups, favorites, dead channels
    browser.rs     scroll/selection model for the channel list
  iptv-net/        networking shared by desktop and Android
    playlist.rs    playlist download with conditional requests + local copy
    health.rs      background channel checks + saved results
  iptv-desktop/    Windows / Linux / macOS application `iptv`
    main.rs        startup and event loop: mpv events -> Tuner -> mpv commands
    mpv.rs         runs mpv, JSON IPC (named pipe / Unix socket), key bindings
    panel.rs       channel list drawn as an mpv ASS overlay
    storage.rs     config / cache locations, saved state
    gui.rs         Windows GUI integration (console attach, error dialog)
  iptv-android/    JNI bridge: the Rust core as an Android native library
android/           Android app (Kotlin, Media3/ExoPlayer), Gradle project
packaging/windows/ installer script (Inno Setup), icon, third-party licenses
.github/workflows/ release.yml: builds and publishes the installer on tag push
```

mpv plays the video and draws the on-screen text. Keys pressed in its window
are bound to `script-message iptv <action>`, delivered to the app over IPC,
turned into `Action`s, and the `Tuner`'s `Effect`s (`Tune`, `Osd`,
`StartDigitTimer`, `Save`) are carried out by sending commands back to mpv.

### Android

The Android app reuses the same Rust code. `crates/iptv-android` is compiled
into a native library (`libiptv_android.so`) holding the tuner, playlist and
channel checks; the Kotlin app in `android/` only plays streams with
Media3/ExoPlayer and turns touches and remote keys into actions. They talk
through a small JNI interface (`Core.kt`): actions go in, JSON instructions
(`tune`, `osd`, `digitTimer`) come out. Channel lists cross as plain integer
arrays, so refreshing a list of ~11,000 channels stays cheap.

## License

Copyright (C) 2026 the IPTV contributors.

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. See [LICENSE](LICENSE) for the full text.

This program is distributed in the hope that it will be useful, but WITHOUT
ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE.

The Windows installer bundles [mpv](https://mpv.io) (GPL-2.0-or-later), run as
a separate program; its license text and source links are included in the
installation folder (`licenses\`, `THIRD-PARTY-NOTICES.txt`).

The iptv-org playlist and the streams it links to belong to their respective
owners and are not covered by this license.
