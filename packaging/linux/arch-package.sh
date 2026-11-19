#!/usr/bin/env bash
# Repackage the StudioCraft Linux tarball as an Arch Linux package:
#
#   $DIST/studiocraft-<pkgver>-1-<arch>.pkg.tar.zst
#
# Usage: packaging/linux/arch-package.sh [TARBALL]
#
# TARBALL defaults to $DIST/studiocraft-<version>-linux-<arch>.tar.gz from package.sh. Needs
# makepkg (Arch Linux or a container with base-devel); it must not run as root.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64 | aarch64) ;;
  arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
TARBALL="${1:-$DIST/studiocraft-$VERSION-linux-$ARCH.tar.gz}"
[[ -f "$TARBALL" ]] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
command -v makepkg >/dev/null || { echo "error: makepkg not found" >&2; exit 1; }
# pkgver may not contain "-": 0.5.0-rc.1 becomes 0.5.0rc.1, which sorts before 0.5.0.
PKGVER="${VERSION//-/}"

WORK="$CARGO_TARGET_DIR/arch-package"
rm -rf "$WORK"
mkdir -p "$WORK"
cp "$TARBALL" "$WORK/"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@PKGVER@/$PKGVER/g" -e "s/@ARCH@/$ARCH/g" \
  -e "s/@TARBALL@/$(basename "$TARBALL")/g" -e "s/@SHA256@/$(sha256 "$TARBALL")/g" \
  "$HERE/arch/PKGBUILD.in" >"$WORK/PKGBUILD"
# --nodeps: build without installing dependencies (the package only copies files).
(cd "$WORK" && PKGDEST="$DIST" PKGEXT=.pkg.tar.zst makepkg --nodeps --noconfirm --force)
PKG="$DIST/studiocraft-$PKGVER-1-$ARCH.pkg.tar.zst"
[[ -f "$PKG" ]] || { echo "error: $PKG was not written" >&2; exit 1; }
echo "==> wrote $PKG"
tar -tf "$PKG" | grep -E '^usr/(bin|share/(applications|mime/packages|licenses))/' 
