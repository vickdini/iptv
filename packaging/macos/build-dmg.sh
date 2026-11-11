#!/usr/bin/env bash
# Builds IPTV-<version>-macOS-<arch>.dmg: IPTV.app with the official mpv
# build inside, ready to drag into Applications.
#
# Usage (on a Mac with Xcode command-line tools and Rust):
#   packaging/macos/build-dmg.sh arm64  mpv-v0.42.0-macos-15-arm.zip
#   packaging/macos/build-dmg.sh x86_64 mpv-v0.42.0-macos-15-intel.zip
# The mpv zips come from https://github.com/mpv-player/mpv/releases.
# Output: target/macos/IPTV-<version>-macOS-<arch>.dmg
#
# The app is signed "ad hoc" (no Apple Developer ID), so the first time
# macOS asks the user to confirm opening it (see the README).
set -euo pipefail
cd "$(dirname "$0")/../.."

arch=$1
mpv_zip=$2
case "$arch" in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *) echo "arch must be arm64 or x86_64" >&2; exit 1 ;;
esac
version=$(grep -m1 -E '^\s*version\s*=' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
out=target/macos/$arch
app=$out/IPTV.app
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

echo "Building IPTV $version for $target..."
rustup target add "$target" >/dev/null
cargo build --release -p iptv-desktop --target "$target"
cp "target/$target/release/iptv" "$app/Contents/MacOS/iptv"

# mpv: the zip holds mpv.tar.gz, which holds mpv.app (libraries included).
mkdir -p "$out/mpv"
unzip -q "$mpv_zip" -d "$out/mpv"
tar -xzf "$out/mpv/mpv.tar.gz" -C "$out/mpv"
mpv_app=$app/Contents/Resources/mpv.app
ditto "$out/mpv/mpv.app" "$mpv_app"
mpv_version="mpv $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$mpv_app/Contents/Info.plist")"
echo "Bundling $mpv_version"

# Icon, from the same 256 px artwork as the other platforms.
iconset=$out/iptv.iconset
mkdir -p "$iconset"
for s in 16 32 128 256; do
    sips -z $s $s packaging/linux/iptv.png --out "$iconset/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) packaging/linux/iptv.png --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/iptv.icns"

# IPTV.app only starts mpv, which shows the video window, so IPTV itself
# stays out of the Dock (LSUIElement)...
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>iptv</string>
    <key>CFBundleIdentifier</key><string>io.github.vickdini.iptv</string>
    <key>CFBundleName</key><string>IPTV</string>
    <key>CFBundleDisplayName</key><string>IPTV</string>
    <key>CFBundleIconFile</key><string>iptv</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.entertainment</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>GPL-3.0-or-later</string>
</dict>
</plist>
EOF

# ...and the player shows up there as "IPTV" with IPTV's icon instead.
plist=$mpv_app/Contents/Info.plist
pb() { /usr/libexec/PlistBuddy -c "$1" "$plist"; }
pb "Set :CFBundleName IPTV"
pb "Delete :CFBundleDisplayName" 2>/dev/null || true
pb "Add :CFBundleDisplayName string IPTV"
pb "Set :CFBundleIdentifier io.github.vickdini.iptv.player"
pb "Delete :CFBundleIconName" 2>/dev/null || true # use the .icns, not mpv's asset catalog
cp "$app/Contents/Resources/iptv.icns" "$mpv_app/Contents/Resources/icon.icns"

# Licenses.
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
cp packaging/windows/licenses/mpv-LICENSE.GPL.txt "$app/Contents/Resources/mpv-LICENSE.GPL.txt"
sed "s|@MPV_VERSION@|$mpv_version|" packaging/macos/THIRD-PARTY-NOTICES.txt > "$app/Contents/Resources/THIRD-PARTY-NOTICES.txt"

# Ad-hoc signature (required to run on Apple silicon; we changed mpv.app).
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
echo "Minimum macOS version:"
otool -l "$mpv_app/Contents/MacOS/mpv" | grep -A3 LC_BUILD_VERSION | grep minos || true
otool -l "$app/Contents/MacOS/iptv" | grep -A3 LC_BUILD_VERSION | grep minos || true

# Disk image: the app and a shortcut to Applications to drag it onto.
stage=$out/dmg
mkdir -p "$stage"
ditto "$app" "$stage/IPTV.app"
ln -s /Applications "$stage/Applications"
dmg=target/macos/IPTV-$version-macOS-$arch.dmg
rm -f "$dmg"
hdiutil create -volname "IPTV" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
ls -la "$dmg"
