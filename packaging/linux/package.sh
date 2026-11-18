#!/usr/bin/env bash
# StudioCraft für Linux bauen und verpacken (<arch>: x86_64 oder aarch64):
#
#   $DIST/studiocraft-<version>-linux-<arch>.tar.gz    FHS-Baum (bin/, share/)
#   $DIST/studiocraft-<version>-linux-<arch>.AppImage  läuft auf jeder Distribution mit passender glibc
#   $DIST/studiocraft-<version>-linux-<arch>.deb       Debian, Ubuntu, Mint …   (braucht nfpm)
#   $DIST/studiocraft-<version>-linux-<arch>.rpm       Fedora, openSUSE …       (braucht nfpm)
#
# Aufruf: packaging/linux/package.sh [--skip-build] [--formats "tar appimage deb rpm"]
#
# appimagetool und die AppImage-Runtime werden bei Bedarf in fester Version geladen und per
# SHA-256 geprüft (gleiche Stände wie SoundCraft). Optional: desktop-file-validate, appstreamcli.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

SKIP_BUILD=0
FORMATS="tar appimage deb rpm"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unbekanntes Argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "Architektur $ARCH wird nicht unterstützt" >&2; exit 2 ;;
esac
BASENAME="studiocraft-$VERSION-linux-$ARCH"
has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

echo "==> StudioCraft $VERSION für Linux $ARCH ($FORMATS)"
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p studiocraft -p studiocraft-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- FHS-Baum (Grundlage aller Formate) ----
install -Dm755 "$BIN/studiocraft" "$STAGE/usr/bin/studiocraft"
install -Dm755 "$BIN/studiocraft-cli" "$STAGE/usr/bin/studiocraft-cli"
strip "$STAGE/usr/bin/studiocraft" "$STAGE/usr/bin/studiocraft-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$STUDIOCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/studiocraft"
copy_docs "$STAGE/usr/share/doc/studiocraft"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --pedantic "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml" || warn "AppStream-Prüfung mit Hinweisen"
fi

# ---- tar.gz ----
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "geschrieben: $DIST/$BASENAME.tar.gz"
fi

# ---- deb / rpm ----
if has deb || has rpm; then
  if command -v nfpm >/dev/null; then
    export NFPM_ARCH="$DEB_ARCH"
    sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
    for fmt in deb rpm; do
      if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
    done
  else
    warn "nfpm fehlt (https://nfpm.goreleaser.com/install/): deb und rpm übersprungen"
  fi
fi

# ---- AppImage ----
if has appimage; then
  APPIMAGETOOL_VERSION=1.9.1
  APPIMAGE_RUNTIME_VERSION=20251108
  case "$ARCH" in
    x86_64)
      APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
      APPIMAGE_RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    aarch64)
      APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
      APPIMAGE_RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
  esac
  verify() { printf '%s  %s\n' "$1" "$2" | sha256sum -c - >/dev/null; }
  fetch() { # url sha ziel
    if [[ ! -f "$3" ]] || ! verify "$2" "$3"; then
      curl --proto '=https' -fsSL -o "$3.download" "$1"
      verify "$2" "$3.download" || { echo "Prüfsumme falsch: $1" >&2; exit 1; }
      mv "$3.download" "$3"
    fi
  }
  TOOL="${APPIMAGETOOL:-$CARGO_TARGET_DIR/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage}"
  if [[ -z "${APPIMAGETOOL:-}" ]]; then
    fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage" \
      "$APPIMAGETOOL_SHA256" "$TOOL"
    chmod 755 "$TOOL"
  fi
  RUNTIME="$CARGO_TARGET_DIR/appimage-runtime-$APPIMAGE_RUNTIME_VERSION-$ARCH"
  fetch "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$ARCH" \
    "$APPIMAGE_RUNTIME_SHA256" "$RUNTIME"

  APPDIR="$WORK/StudioCraft.AppDir"
  cp -R "$STAGE" "$APPDIR"
  ln -s usr/bin/studiocraft "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/studiocraft.png" "$APPDIR/studiocraft.png"
  ln -s studiocraft.png "$APPDIR/.DirIcon"
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  REPO="${GITHUB_REPOSITORY:-xXCrusaderXx/StudioCraft}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|studiocraft-*-linux-$ARCH.AppImage.zsync"
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream \
    --runtime-file "$RUNTIME" -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "geschrieben: $OUT"
fi

"$STAGE/usr/bin/studiocraft-cli" --version
echo "==> fertig"
ls -lh "$DIST"
