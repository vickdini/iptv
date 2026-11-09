# IPTV

Watch free, publicly available IPTV channels like a TV: zap with Up/Down, type a
channel number, jump back to the previous channel, or pick from a scrollable
channel list. Channels that don't work are found automatically and hidden.

By default the app uses the public [iptv-org](https://github.com/iptv-org/iptv)
playlist, `https://iptv-org.github.io/iptv/index.m3u` (about 11,500 channels, of
which roughly three quarters work at any given time).

Runs on Windows today; Linux and macOS work from source; Android is planned.

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

Download and run `IPTV-Setup-<version>.exe`. It's a single installer with
everything included (the app and its video engine). Nothing else needs to be
installed. It:

* installs for the current user only, without asking for administrator rights
* adds a Start-menu shortcut and, optionally, a desktop icon
* can be removed from *Settings → Apps* like any other program

The app opens maximized and starts playing straight away.

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

## Architecture

```
crates/
  iptv-core/       platform-independent logic, no I/O (reused by every front-end)
    m3u.rs         extended-M3U parser (tvg-*, group-title, user-agent/referrer)
    tuner.rs       TV state machine: Action (remote button) -> Vec<Effect>;
                   channel numbers, zapping, groups, favorites, dead channels
    browser.rs     scroll/selection model for the channel list
  iptv-desktop/    Windows / Linux / macOS application `iptv`
    main.rs        startup and event loop: mpv events -> Tuner -> mpv commands
    mpv.rs         runs mpv, JSON IPC (named pipe / Unix socket), key bindings
    panel.rs       channel list drawn as an mpv ASS overlay
    playlist.rs    playlist download with conditional requests + local copy
    health.rs      background channel checks + saved results
    storage.rs     config / cache locations, saved state
    gui.rs         Windows GUI integration (console attach, error dialog)
packaging/windows/ installer script (Inno Setup), icon, third-party licenses
```

mpv plays the video and draws the on-screen text. Keys pressed in its window
are bound to `script-message iptv <action>`, delivered to the app over IPC,
turned into `Action`s, and the `Tuner`'s `Effect`s (`Tune`, `Osd`,
`StartDigitTimer`, `Save`) are carried out by sending commands back to mpv.

### Android (planned)

`iptv-core` depends only on `serde`, so it compiles for Android as is. An
Android front-end only has to map D-pad / remote key events to `Action`s and
carry out `Effect`s with a player (libmpv, or ExoPlayer/Media3 through a thin
Kotlin UI calling `iptv-core` via `uniffi` or `jni`).

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
