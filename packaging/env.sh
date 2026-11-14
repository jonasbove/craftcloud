# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# Adapted from craft-launcher's packaging/env.sh (MIT OR Apache-2.0; see NOTICE).
#
# Exports:
#   ROOT                     workspace root
#   VERSION                  [workspace.package] version from Cargo.toml (override: CRAFTCLOUD_VERSION)
#   DIST                     output directory for release files (default: $ROOT/dist/release)
#   CRAFTCLOUD_BUILD_SHA     git commit of the build
#   CRAFTCLOUD_BUILD_DATE    UTC build date, YYYY-MM-DD (the AppStream release date)
#   CRAFTCLOUD_MAINTAINER    the deb/rpm Maintainer field (override for your own builds)
#   CARGO_TARGET_DIR         cargo's target dir (default: $ROOT/target)
#
# And the helpers warn, copy_docs and sha256.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
  return
}

VERSION="${CRAFTCLOUD_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [[ -z "${CRAFTCLOUD_BUILD_SHA:-}" ]]; then
  CRAFTCLOUD_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export CRAFTCLOUD_BUILD_SHA
export CRAFTCLOUD_BUILD_DATE="${CRAFTCLOUD_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CRAFTCLOUD_MAINTAINER="${CRAFTCLOUD_MAINTAINER:-jonasbove <31095576+jonasbove@users.noreply.github.com>}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
  return
}

# Copy the licence, notice and readme files into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  return
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  local file="$1"
  if command -v sha256sum >/dev/null; then sha256sum "$file" | cut -d' ' -f1; else shasum -a 256 "$file" | cut -d' ' -f1; fi
  return
}
