#!/usr/bin/env bash
# Prüfsummen und Signaturen für alle Release-Dateien in $DIST (Selbst-Update, docs/selbst-update.md):
#
#   $DIST/SHA256SUMS.txt     eine Zeile je Datei
#   $DIST/<datei>.minisig    minisign-Signatur (Ed25519); der signierte Kommentar nennt
#                            „studiocraft <version> <datei>“, das Selbst-Update prüft ihn
#
# Aufruf: packaging/sign-release.sh
#
# Signiert wird nur mit dem geheimen Schlüssel in MINISIGN_SECRET_KEY (Inhalt der Schlüsseldatei)
# und seinem Passwort in MINISIGN_PASSWORD. Ohne Schlüssel entsteht nur SHA256SUMS.txt mit einer
# Warnung: StudioCraft meldet ein solches Release dann nur und installiert es nicht selbst.
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"

shopt -s nullglob
FILES=()
for f in "$DIST"/*; do
  case "$f" in
    *.minisig | */SHA256SUMS.txt) ;;
    *) [[ -f "$f" ]] && FILES+=("$f") ;;
  esac
done
[[ ${#FILES[@]} -gt 0 ]] || { echo "Fehler: nichts zu signieren in $DIST" >&2; exit 1; }

(cd "$DIST" && for f in "${FILES[@]}"; do
  printf '%s  %s\n' "$(sha256 "$f")" "$(basename "$f")"
done) >"$DIST/SHA256SUMS.txt"
echo "geschrieben: $DIST/SHA256SUMS.txt (${#FILES[@]} Dateien)"

if [[ -z "${MINISIGN_SECRET_KEY:-}" ]]; then
  warn "MINISIGN_SECRET_KEY fehlt: nicht signiert; StudioCraft aktualisiert sich nicht selbst auf dieses Release"
  exit 0
fi
command -v minisign >/dev/null || { echo "Fehler: minisign nicht gefunden" >&2; exit 1; }
KEY="$(mktemp)"
trap 'rm -f "$KEY"' EXIT
printf '%s\n' "$MINISIGN_SECRET_KEY" >"$KEY"
for f in "${FILES[@]}" "$DIST/SHA256SUMS.txt"; do
  name="$(basename "$f")"
  # Der Kommentar wird mitsigniert: das Selbst-Update prüft, dass er diese Datei und Version nennt.
  printf '%s\n' "${MINISIGN_PASSWORD:-}" | minisign -S -s "$KEY" -m "$f" \
    -t "studiocraft $VERSION $name" -x "$f.minisig"
done
echo "signiert: ${#FILES[@]} Dateien und SHA256SUMS.txt"
