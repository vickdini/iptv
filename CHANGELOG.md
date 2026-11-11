# Changelog

What's new in each version of IPTV. Downloads for every version are on the
[releases page](https://github.com/vickdini/iptv/releases).

When releasing, add a section for the new version at the top. The release
workflow uses that section as the release notes.

## [Unreleased]

### Added
- **Move favorites between devices** with a small file,
  `IPTV favorites.json`. Importing adds to the favorites already there.
  - Phone remote: *Save to this phone* and *Add from a file*.
  - Computers: **E** to export and **I** to import, in the Downloads folder.
  - Android: long-press ★ in the channel list.
- **Channel logos** in the channel list, the channel banner and the phone
  remote, on every platform.
- **Recently watched:** the last 20 channels you watched, most recent first.
  - Computers: **g**.
  - TV remotes: Right in the channel list (right after *All channels*).
  - Android and the phone remote: the 🕘 button.

## [0.10.0] - 2026-11-11

### Added
- **Two channels at once on phones and tablets:** hold the device upright
  and the screen splits in two, one channel above the other. Tap a half to
  hear it, swipe a half to change its channel.
- **Picture-in-picture:**
  - Android: leave the app while watching, and the channel keeps playing in
    a small window over other apps, with channel buttons.
  - Computers: **p** shows a second channel, small and silent, in the
    bottom-right corner over the big one. **[** / **]** change its channel,
    and **x** (or a click on it) swaps the two.

### Changed
- **Computers:** **p** no longer pauses; use **Space**.

## [0.9.0] - 2026-11-11

### Added
- **Phone remote:** control IPTV from your phone's browser, on Android or
  iPhone, with nothing to install. You can browse and search every channel
  and tap one to watch it, and the remote also has favorites, channel up /
  down, previous channel, volume, mute, audio language and subtitles.
  - Computers: press **r** to show a QR code.
  - Android TV, Google TV and Fire TV: the phone remote button in the
    channel list.
  - The remote works on your home Wi-Fi only, and phones need the PIN shown
    on the TV.
- **Quick start guide:** a printable PDF explaining how to install and use
  IPTV on every platform (linked from the README).

## [0.8.0] - 2026-11-11

### Added
- **Linux download:** `IPTV-<version>-x86_64.AppImage`, a single file with
  everything included (the app, the mpv video engine and its libraries). It
  runs on Linux from 2022 onward (Ubuntu 22.04, Debian 12, Fedora 36, Mint 21
  and newer).
- **Mac downloads:** disk images for Apple silicon (`-macOS-arm64.dmg`) and
  Intel Macs (`-macOS-x86_64.dmg`), with mpv included. IPTV shows in the Dock
  with its own name and icon.
- **Automatic updates on Linux and Macs**, as on Windows: the AppImage
  replaces itself, and the Mac app replaces IPTV.app, then restarts.
- On Linux and Macs, errors appear in a dialog when IPTV wasn't started from
  a terminal.

### Fixed
- **Windows, Linux and Mac:** each audio or subtitle language is listed once,
  even when a stream repeats it for every quality level.

## [0.7.0] - 2026-11-10

### Added
- **Audio language and subtitles** on channels that broadcast several, for
  example an original version next to a dubbed one, audio description, or
  subtitles. When you tune to such a channel, a short note says what it
  offers.
  - Windows: **a** steps through the audio languages, **t** through the
    subtitles and then off.
  - Android: a subtitles button in the channel list header, and the
    remote's Audio / CC keys.
  - Samsung TV: **Yellow** opens an Audio / Subtitles menu.
  - Your choice is remembered by language and used on every channel that
    offers it.
- **"Clear all" in the countries & languages picker** shows every channel
  again in one step.

### Changed
- **Samsung TV:** **Yellow** outside the channel list now opens the audio &
  subtitles menu. It still shows the help on channels without a choice.

## [0.6.0] - 2026-11-10

### Added
- **Filter by country and language:** show only channels from the countries
  you choose, or in the languages you choose. The filter applies to zapping,
  the channel list, groups, search and favorites, and is remembered.
  - Windows: **c**.
  - Android: the 🌐 button.
  - Samsung TV: **Yellow** in the channel list.

## [0.5.1] - 2026-11-09

### Added
- **Exit button** in the Android app's channel list on phones and tablets,
  where leaving a full-screen app isn't obvious.

## [0.5.0] - 2026-11-09

### Added
- **Search channels by name** on every platform. The search ignores accents
  and capital letters.
  - Windows: **/** or **Ctrl+F**.
  - Android: the search box in the channel list.
  - Samsung TV: **Blue**, or Up from the first channel in the list.

## [0.4.1] - 2026-11-09

### Fixed
- **Samsung TV updates:** they are now published reliably.
- **TV install script:** `install-on-tv.ps1` works on Windows PowerShell 5.1
  without the `-AppDir` option.

## [0.4.0] - 2026-11-09

### Added
- **Automatic updates:** every app looks for a new release each time it
  starts, and installs it by itself after checking its SHA-256.
  - Windows: a silent install, then IPTV restarts.
  - Android: the app installs the new APK itself.
  - Samsung TV: new app code downloads from GitHub Pages.

## [0.3.0] - 2026-11-09

### Added
- **Samsung smart TV app** (2019 models and newer), installed through
  Developer Mode. The installer can be downloaded from GitHub, and it also
  works from a USB stick.
- **TV boxes:** Android TV, Google TV and Fire TV support, with remote
  control keys.
- **Chromecast:** casting from the Android app.

## [0.2.0] - 2026-11-09

### Added
- **Android app** for phones and tablets: swipe to change channel, tap for
  the channel list. It is published as a signed APK with each release.

## [0.1.0] - 2026-11-09

First release, for Windows.

### Added
- **Zapping like a TV** through the public iptv-org playlist:
  - channel up / down
  - channel numbers
  - the previous channel
  - groups (News, Movies, Sports…)
  - favorites
- **Channel list:** a scrollable panel, used with the mouse wheel, clicks or
  the keyboard.
- **Always the latest playlist:** it is checked at every launch and
  downloaded only when it has changed.
- **Channels that don't work are hidden** automatically, after a background
  check and when playback fails.
- **One Windows installer** with everything included. It installs per user,
  without administrator rights, and IPTV opens maximized.
- **GPL-3.0-or-later license**, plus a disclaimer that the app only loads a
  publicly available playlist.

[0.10.0]: https://github.com/vickdini/iptv/releases/tag/v0.10.0
[0.9.0]: https://github.com/vickdini/iptv/releases/tag/v0.9.0
[0.8.0]: https://github.com/vickdini/iptv/releases/tag/v0.8.0
[0.7.0]: https://github.com/vickdini/iptv/releases/tag/v0.7.0
[0.6.0]: https://github.com/vickdini/iptv/releases/tag/v0.6.0
[0.5.1]: https://github.com/vickdini/iptv/releases/tag/v0.5.1
[0.5.0]: https://github.com/vickdini/iptv/releases/tag/v0.5.0
[0.4.1]: https://github.com/vickdini/iptv/releases/tag/v0.4.1
[0.4.0]: https://github.com/vickdini/iptv/releases/tag/v0.4.0
[0.3.0]: https://github.com/vickdini/iptv/releases/tag/v0.3.0
[0.2.0]: https://github.com/vickdini/iptv/releases/tag/v0.2.0
[0.1.0]: https://github.com/vickdini/iptv/releases/tag/v0.1.0
