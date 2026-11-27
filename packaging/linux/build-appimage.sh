#!/usr/bin/env bash
# Packs a release build of Camallo as an AppImage: camallo-<version>-linux-x86_64.AppImage.
#
#   packaging/linux/build-appimage.sh <version> [release folder] [output folder]
#
# The release folder is target/release, where `cargo build --release` leaves camallo and camallo-browser, and the
# output folder is dist/ unless said otherwise. Nothing is bundled but the two programs. camallo needs no library
# of its own. camallo-browser is the window of the built-in browser and needs WebKitGTK (libwebkit2gtk-4.1) from the
# system, which is why it is a program of its own: without WebKitGTK, Camallo still starts and says what to install.
# yt-dlp and ffmpeg are not shipped either, Camallo downloads them on the first run.
#
# appimagetool is taken from $APPIMAGETOOL when that is set, and downloaded otherwise.
set -euo pipefail

version=${1:?usage: build-appimage.sh <version> [release folder] [output folder]}
release=${2:-target/release}
out=${3:-dist}

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
release=$(cd "$release" && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
appdir=$work/Camallo.AppDir

# The folders as a Linux desktop expects them, and the same files at the top for the AppImage itself.
install -Dm755 "$release/camallo" "$appdir/usr/bin/camallo"
install -Dm755 "$release/camallo-browser" "$appdir/usr/bin/camallo-browser"
install -Dm644 "$root/assets/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/camallo.png"
sed "s/^Type=Application$/Type=Application\nX-AppImage-Version=$version/" "$here/camallo.desktop" >"$work/camallo.desktop"
install -Dm644 "$work/camallo.desktop" "$appdir/usr/share/applications/camallo.desktop"
install -m644 "$work/camallo.desktop" "$appdir/camallo.desktop"
install -m644 "$root/assets/icon-256.png" "$appdir/camallo.png"
ln -s camallo.png "$appdir/.DirIcon"
ln -s usr/bin/camallo "$appdir/AppRun"

tool=${APPIMAGETOOL:-}
if [ -z "$tool" ]; then
  tool=$work/appimagetool
  curl -fsSL --retry 3 -o "$tool" \
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$tool"
fi

# Runs appimagetool without FUSE, which a build machine or a container may not have.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream "$appdir" "$out/camallo-$version-linux-x86_64.AppImage"
echo "$out/camallo-$version-linux-x86_64.AppImage"
