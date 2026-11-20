#!/usr/bin/env bash
# Build and package StudioCraft for FreeBSD:
#
#   $DIST/studiocraft-<version>-freebsd-<arch>.tar.gz   a /usr/local-style tree
#       studiocraft-<version>-freebsd-<arch>/{bin, share/applications, share/mime, share/metainfo,
#       share/icons, share/doc/studiocraft}
#   $DIST/studiocraft-<version>-freebsd-<arch>.pkg      the same files as a native pkg(8) package
#                                                       (only when built on FreeBSD)
#
# Install the tarball by copying its contents into /usr/local, e.g.
#   tar -xzf studiocraft-*-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
# or the package with `pkg add studiocraft-*.pkg`.
#
# Usage: packaging/freebsd/package.sh [--skip-build]
#
# Needs bash, cargo and alsa-lib (cpal's audio I/O; at run time alsa-plugins-oss routes ALSA to
# FreeBSD's OSS sound(4)). FreeBSD's install(1) has no -D, so directories are created first.
# <arch> is x86_64, or aarch64 when CARGO_BUILD_TARGET=aarch64-unknown-freebsd.
# Adapted from SoundCraft (storytold/soundcraft, Apache-2.0), see NOTICE.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD calls x86_64 "amd64"; release names use the Rust/Linux spelling.
release_arch() {
  case "$1" in
    amd64 | x86_64) echo x86_64 ;;
    arm64 | aarch64) echo aarch64 ;;
    *) echo "unsupported architecture $1" >&2; return 2 ;;
  esac
}
case "${CARGO_BUILD_TARGET:-}" in
  "") ARCH="$(release_arch "$(uname -m)")" ;;
  aarch64-unknown-freebsd | x86_64-unknown-freebsd) ARCH="$(release_arch "${CARGO_BUILD_TARGET%%-*}")" ;;
  *) echo "CARGO_BUILD_TARGET=$CARGO_BUILD_TARGET is not a supported FreeBSD target" >&2; exit 2 ;;
esac
HOST_ARCH="$(release_arch "$(uname -m)" 2>/dev/null)" || HOST_ARCH="$(uname -m)"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]] && [[ "$(uname -s)" != FreeBSD ]]; then
  HOST_ARCH="$(uname -s) $HOST_ARCH"
fi
BASENAME="studiocraft-$VERSION-freebsd-$ARCH"

echo "==> StudioCraft $VERSION for FreeBSD $ARCH"
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p studiocraft -p studiocraft-cli)
fi
BIN="$CARGO_TARGET_DIR/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"

# ---- stage a /usr/local-style tree -------------------------------------------------------------
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/mime/packages" "$STAGE/share/metainfo" \
  "$STAGE/share/icons" "$STAGE/share/doc/studiocraft"
install -m 755 "$BIN/studiocraft" "$BIN/studiocraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/studiocraft" "$STAGE/bin/studiocraft-cli" 2>/dev/null || true
# Desktop entry, MIME type and metainfo are the freedesktop files of the Linux packages.
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$STUDIOCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/studiocraft"

# ---- .tar.gz -----------------------------------------------------------------------------------
tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"

# ---- .pkg (native package; needs pkg(8), so only on FreeBSD) -------------------------------------
if [[ "$(uname -s)" = FreeBSD ]] && command -v pkg >/dev/null; then
  ROOTDIR="$WORK/pkg-root"
  mkdir -p "$ROOTDIR/usr/local"
  cp -R "$STAGE/." "$ROOTDIR/usr/local/"
  ABI_ARCH="$ARCH"; [[ "$ABI_ARCH" != x86_64 ]] || ABI_ARCH=amd64
  ABI="FreeBSD:$(uname -r | cut -d. -f1):$ABI_ARCH"
  {
    echo "name: studiocraft"
    echo "version: \"${VERSION//-/.}\""
    echo "origin: audio/studiocraft"
    echo "comment: \"Digital audio workstation: record, arrange and mix music\""
    echo "desc: \"StudioCraft is a cross-platform digital audio workstation written in Rust.\""
    echo "maintainer: \"StudioCraft contributors\""
    echo "www: https://github.com/ONEFOLD-LABS/StudioCraft"
    echo "prefix: /usr/local"
    echo "abi: \"$ABI\""
    echo "licenselogic: dual"
    echo "licenses: [MIT, APACHE20]"
    echo "deps: {"
    echo "  alsa-lib: {origin: audio/alsa-lib, version: \"0\"},"
    echo "  alsa-plugins: {origin: audio/alsa-plugins, version: \"0\"}"
    echo "}"
  } >"$WORK/+MANIFEST"
  (cd "$ROOTDIR" && find . -type f | sed 's|^\./|/|') >"$WORK/plist"
  pkg create -M "$WORK/+MANIFEST" -p "$WORK/plist" -r "$ROOTDIR" -o "$WORK/out"
  mv "$WORK"/out/studiocraft-*.pkg "$DIST/$BASENAME.pkg"
  echo "wrote $DIST/$BASENAME.pkg"
  pkg info -F "$DIST/$BASENAME.pkg" | head -20
fi

if [[ "$ARCH" = "$HOST_ARCH" ]]; then
  "$STAGE/bin/studiocraft-cli" --version
  "$STAGE/bin/studiocraft" --version
else
  # Cross-built binaries cannot run here; check that they are FreeBSD executables instead.
  want="$ARCH"; [[ "$ARCH" != x86_64 ]] || want=x86-64
  for b in "$STAGE"/bin/*; do
    case "$(file -b "$b")" in
      ELF*" executable, "*"$want"*FreeBSD* | ELF*"$want"*FreeBSD*interpreter*) ;;
      *) echo "error: $b is not a FreeBSD $ARCH executable: $(file -b "$b")" >&2; exit 1 ;;
    esac
  done
  echo "skipping --version: an $ARCH binary does not run on this $HOST_ARCH machine"
fi
echo "==> done"
ls -lh "$DIST"
