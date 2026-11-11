#!/usr/bin/env bash
# Builds IPTV-<version>-x86_64.AppImage: the IPTV binary plus mpv and every
# library they need, in one file that runs on most Linux distributions.
#
# Run on Ubuntu 22.04 (the oldest system it should run on: glibc 2.35), as
# the release workflow does:
#   sudo apt-get install -y mpv file desktop-file-utils
#   packaging/linux/build-appimage.sh
# Output: target/appimage/IPTV-<version>-x86_64.AppImage
set -euo pipefail
cd "$(dirname "$0")/../.."

# Pinned packaging tools (verified by SHA-256).
LINUXDEPLOY_URL=https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage
LINUXDEPLOY_SHA256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0

version=$(grep -m1 -E '^\s*version\s*=' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
out=target/appimage
tools=$out/tools
appdir=$out/AppDir
rm -rf "$appdir"
mkdir -p "$tools" "$appdir/usr/bin"

fetch() { # url sha256 dest
    if [ ! -f "$3" ] || ! echo "$2  $3" | sha256sum -c --status; then
        curl -fsSL "$1" -o "$3"
        echo "$2  $3" | sha256sum -c
    fi
    chmod +x "$3"
}
fetch "$LINUXDEPLOY_URL" "$LINUXDEPLOY_SHA256" "$tools/linuxdeploy"
fetch "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "$tools/appimagetool"
# Docker / CI machines often lack FUSE: run the tools unpacked.
export APPIMAGE_EXTRACT_AND_RUN=1

echo "Building IPTV $version..."
cargo build --release -p iptv-desktop
bin="${CARGO_TARGET_DIR:-target}/release/iptv"
mpv_bin=$(command -v mpv)
"$mpv_bin" --version | head -1

# IPTV and mpv, with the libraries they link to (linuxdeploy skips the core
# system ones every distribution has: glibc, OpenGL drivers, X11/Wayland...).
"$tools/linuxdeploy" --appdir "$appdir" \
    --executable "$bin" \
    --executable "$mpv_bin" \
    --desktop-file packaging/linux/iptv.desktop \
    --icon-file packaging/linux/iptv.png

# Libraries linuxdeploy leaves to the system but that aren't on every
# desktop (mpv won't start without them): bundle these too.
for lib in libjack.so.0 libusb-1.0.so.0; do
    path=$(ldconfig -p | awk -v l="$lib" '$1 == l && /x86-64/ { p = $NF } END { print p }')
    [ -n "$path" ] || { echo "missing $lib" >&2; exit 1; }
    cp -L "$path" "$appdir/usr/lib/"
    # ...and what they need beyond the basics (e.g. Berkeley DB for JACK).
    ldd "$path" | awk '/=> \// { print $3 }' | grep -E 'libdb-|libopus|libsamplerate' | xargs -r -I{} cp -Ln {} "$appdir/usr/lib/" || true
done

# Licenses: ours, mpv's, and a note on what is bundled.
docs=$appdir/usr/share/doc/iptv
mkdir -p "$docs"
cp LICENSE "$docs/LICENSE.txt"
mpv_version=$("$mpv_bin" --version | head -1 | cut -d" " -f1-2)
sed "s|@MPV_VERSION@|$mpv_version|" packaging/linux/THIRD-PARTY-NOTICES.txt > "$docs/THIRD-PARTY-NOTICES.txt"

file="$out/IPTV-$version-x86_64.AppImage"
rm -f "$file"
ARCH=x86_64 "$tools/appimagetool" --no-appstream "$appdir" "$file"
ls -la "$file"
