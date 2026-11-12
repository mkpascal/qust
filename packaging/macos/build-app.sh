#!/bin/bash
# Builds Qust's macOS bundle, disk image and update archive from built binaries, without Tauri:
#
#   packaging/macos/build-app.sh --version 1.2.0 \
#     --binary target/universal-apple-darwin/release/qust          (a universal binary)
#   or --arm64 target/aarch64-apple-darwin/release/qust --x86_64 target/x86_64-apple-darwin/release/qust
#     [--icon src-tauri/icons/icon.icns] [--out build/bin] [--minimum-system-version 10.15]
#
# writes into <out>:
#   Qust.app                               the bundle (Contents/{Info.plist,MacOS/qust,Resources/icon.icns})
#   Qust_<version>_universal.dmg           for people to download: the app and an Applications link
#   Qust_<version>_universal.app.tar.gz    for the updaters: Qust.app/… at the top, nothing else
# Sign the .tar.gz for the updaters afterwards (qust-sign). The bundle is signed ad hoc (or with
# $APPLE_SIGNING_IDENTITY when set), not notarized: the first launch of a downloaded copy still
# needs right-click > Open (System Settings > Privacy & Security on macOS 15). Updates are
# downloaded by the app itself, so they carry no quarantine flag and open without asking.
#
# Needs macOS (lipo, codesign, hdiutil, plutil, bsdtar). Written for CI; not run on Windows.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

version="" binary="" arm64="" x86_64="" icon="$root/src-tauri/icons/icon.icns" out="$root/build/bin"
minimum="10.15"
while [ $# -gt 0 ]; do
  case "$1" in
    --version) version="${2#v}"; shift 2 ;;
    --binary) binary="$2"; shift 2 ;;
    --arm64) arm64="$2"; shift 2 ;;
    --x86_64) x86_64="$2"; shift 2 ;;
    --icon) icon="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --minimum-system-version) minimum="$2"; shift 2 ;;
    *) echo "build-app: unknown argument $1" >&2; exit 1 ;;
  esac
done

fail() { echo "build-app: $*" >&2; exit 1; }

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || fail "--version x.y.z is required"
[ -f "$icon" ] || fail "$icon not found"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# One universal executable: Intel and Apple silicon Macs update from the same archive.
if [ -n "$binary" ]; then
  [ -f "$binary" ] || fail "$binary not found"
  cp "$binary" "$work/qust"
else
  [ -f "$arm64" ] && [ -f "$x86_64" ] || fail "give --binary, or both --arm64 and --x86_64"
  lipo -create -output "$work/qust" "$arm64" "$x86_64"
fi
archs="$(lipo -archs "$work/qust")"
[[ "$archs" == *arm64* && "$archs" == *x86_64* ]] || fail "the executable is not universal ($archs)"

app="$out/Qust.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$work/qust" "$app/Contents/MacOS/qust"
cp "$icon" "$app/Contents/Resources/icon.icns"
sed -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_SYSTEM_VERSION@/$minimum/g" "$here/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" > /dev/null
printf 'APPL????' > "$app/Contents/PkgInfo"

# A bundle must be signed to run on Apple silicon; ad hoc ("-") unless an identity is given.
codesign --force --deep --timestamp=none --sign "${APPLE_SIGNING_IDENTITY:--}" "$app"
codesign --verify --deep --strict "$app"

# The update archive: the updaters drop the first path component, so it must be Qust.app/ and
# only that. COPYFILE_DISABLE and --no-mac-metadata keep bsdtar from adding ._ AppleDouble files.
archive="$out/Qust_${version}_universal.app.tar.gz"
rm -f "$archive"
COPYFILE_DISABLE=1 tar --no-mac-metadata -czf "$archive" -C "$out" Qust.app
if tar -tzf "$archive" | grep -Eqv '^Qust\.app(/|$)'; then
  fail "the archive holds something besides Qust.app"
fi

# The disk image: the app and a link to /Applications to drag it to.
dmg="$out/Qust_${version}_universal.dmg"
rm -f "$dmg"
mkdir "$work/dmg"
cp -R "$app" "$work/dmg/Qust.app"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -volname Qust -srcfolder "$work/dmg" -fs HFS+ -format UDZO -ov "$dmg" > /dev/null

echo "build-app: wrote $app"
echo "build-app: wrote $archive"
echo "build-app: wrote $dmg"
