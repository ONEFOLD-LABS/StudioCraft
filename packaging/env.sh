# shellcheck shell=bash
# Gemeinsame Einstellungen der Packaging-Skripte. Einbinden: `. "$(dirname "$0")/../env.sh"`.
#
# Exportiert:
#   ROOT                      Wurzel des Workspace
#   APP_ID                    Reverse-DNS-Kennung (Desktop-Datei, AppStream)
#   VERSION                   [workspace.package] version aus Cargo.toml (überschreibbar: STUDIOCRAFT_VERSION)
#   DIST                      Ausgabeordner der Pakete (Standard: $ROOT/dist/release)
#   STUDIOCRAFT_BUILD_SHA     Git-Commit
#   STUDIOCRAFT_BUILD_DATE    Build-Datum (UTC, JJJJ-MM-TT)
#   CARGO_TARGET_DIR          Zielordner von cargo (Standard: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT
export APP_ID="${APP_ID:-io.github.xxcrusaderxx.studiocraft}"

# Die Version steht an genau einer Stelle: `[workspace.package] version` in Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${STUDIOCRAFT_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "Fehler: [workspace.package] version in $ROOT/Cargo.toml nicht lesbar" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

STUDIOCRAFT_BUILD_SHA="${STUDIOCRAFT_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)}"
export STUDIOCRAFT_BUILD_SHA
export STUDIOCRAFT_BUILD_DATE="${STUDIOCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Warnung (in GitHub Actions als Annotation).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "Warnung: $*" >&2; fi
}

# Lizenz- und Hinweisdateien in ein Paketverzeichnis kopieren.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE THIRD-PARTY-LICENSES.txt; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
}

sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file" | cut -d' ' -f1; else shasum -a 256 "$file" | cut -d' ' -f1; fi
}
