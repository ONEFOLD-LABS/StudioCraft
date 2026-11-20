#!/usr/bin/env bash
# Install StudioCraft's desktop entry and icons for the current user, so the launcher, task bar
# and (on Wayland) the window show the app icon. Useful for the AppImage and for `cargo run`
# builds; distribution packages install these files system-wide.
#
#   packaging/linux/install-user.sh [PATH-TO-BINARY-OR-APPIMAGE]   install (default: studiocraft on PATH)
#   packaging/linux/install-user.sh --uninstall                    remove again
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP_ID=io.github.onefold_labs.studiocraft
# App id until 0.7.0; its desktop entry is removed so the launcher shows StudioCraft once.
OLD_APP_ID=io.github.xxcrusaderxx.studiocraft
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
ICONS="$DATA/icons/hicolor"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA/applications" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
  command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
  rm -f "$DATA/applications/$APP_ID.desktop" "$DATA/applications/$OLD_APP_ID.desktop"
  find "$ICONS" -path '*/apps/studiocraft.*' -delete 2>/dev/null || true
  refresh
  echo "removed"
  exit 0
fi

EXEC="${1:-studiocraft}"
if [[ "$EXEC" == */* ]]; then EXEC="$(cd "$(dirname "$EXEC")" && pwd)/$(basename "$EXEC")"; fi

mkdir -p "$DATA/applications"
rm -f "$DATA/applications/$OLD_APP_ID.desktop"
# Exec may contain spaces: quote it as the desktop entry spec requires.
sed "s|^Exec=studiocraft |Exec=\"$EXEC\" |" "$HERE/$APP_ID.desktop" >"$DATA/applications/$APP_ID.desktop"
for dir in "$ROOT"/assets/app-icon/hicolor/*/apps; do
  size="$(basename "$(dirname "$dir")")"
  mkdir -p "$ICONS/$size/apps"
  cp "$dir"/studiocraft.* "$ICONS/$size/apps/"
done
refresh
echo "installed $DATA/applications/$APP_ID.desktop (Exec: $EXEC)"
