#!/usr/bin/env bash
# Repackage the StudioCraft Linux tarball as a single-file Flatpak bundle:
#
#   $DIST/studiocraft-<version>-linux-<arch>.flatpak
#
# Usage: packaging/linux/flatpak-bundle.sh [--no-test] [TARBALL]
#
# TARBALL defaults to $DIST/studiocraft-<version>-linux-<arch>.tar.gz from package.sh; the host
# architecture (x86_64 or aarch64) must match it. Needs flatpak and flatpak-builder; the
# freedesktop runtime and SDK from the manifest are installed per user from Flathub. Unless
# --no-test, the bundle is installed per user and `studiocraft-cli --version` runs inside the
# sandbox as a smoke test. Adapted from SoundCraft (storytold/soundcraft, Apache-2.0), see NOTICE.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
FLATHUB=https://dl.flathub.org/repo/flathub.flatpakrepo

TEST=1
TARBALL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-test) TEST=0; shift ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) TARBALL="$1"; shift ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
BASENAME="studiocraft-$VERSION-linux-$ARCH"
TARBALL="${TARBALL:-$DIST/$BASENAME.tar.gz}"
[[ -f "$TARBALL" ]] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
for tool in flatpak flatpak-builder; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found" >&2; exit 1; }
done

echo "==> StudioCraft $VERSION Flatpak bundle for $ARCH from $(basename "$TARBALL")"
WORK="$CARGO_TARGET_DIR/flatpak-bundle"
rm -rf "$WORK"
mkdir -p "$WORK/stage"
tar -xzf "$TARBALL" -C "$WORK/stage" --strip-components=1
[[ -x "$WORK/stage/bin/studiocraft" ]] || { echo "error: $TARBALL has no bin/studiocraft" >&2; exit 1; }
cp "$HERE/flatpak/$APP_ID.bundle.yml" "$WORK/$APP_ID.yml"

flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
# --disable-rofiles-fuse: no FUSE in containers and CI runners.
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --default-branch=stable --repo="$WORK/repo" --state-dir="$WORK/state" "$WORK/build" "$WORK/$APP_ID.yml"

OUT="$DIST/$BASENAME.flatpak"
# --runtime-repo lets `flatpak install` fetch the runtime from Flathub when it is missing.
flatpak build-bundle --runtime-repo="$FLATHUB" "$WORK/repo" "$OUT" "$APP_ID" stable
echo "wrote $OUT"

if [[ "$TEST" = 1 ]]; then
  flatpak install --user -y --noninteractive --reinstall "$OUT"
  flatpak info --user "$APP_ID"
  flatpak run --command=studiocraft-cli "$APP_ID" --version
  # The app itself, as the plugin scan starts it: same binary, stdout through a pipe.
  out="$(flatpak run --command=studiocraft "$APP_ID" --version)"
  [[ "$out" == "studiocraft "* ]] || { echo "error: studiocraft --version in the sandbox printed '$out'" >&2; exit 1; }
  echo "ok studiocraft in the sandbox: $out"
fi
echo "==> done"
ls -lh "$OUT"
