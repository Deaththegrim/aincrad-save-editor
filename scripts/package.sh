#!/usr/bin/env bash
# Package the Aincrad Save Editor into portable Linux + Windows zips.
#
# The thumbnail/font payload (aml-data/) is generated from the game and is NOT
# version-controlled, so this reuses the payload from the existing dist zip and
# swaps in a freshly-built binary. Run from anywhere; writes to dist/.
#
# It VERIFIES each binary's embedded version string matches Cargo.toml — manual
# packaging has shipped a mislabeled binary before (fix present, version stale).
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
dist="$here/dist"
ver="$(grep -m1 '^version' "$here/Cargo.toml" | cut -d'"' -f2)"
echo "Packaging Aincrad Save Editor $ver"

package() { # $1=os  $2=exe-name-in-pack  $3=built-binary-path
  local os="$1" exe="$2" bin="$3"
  local name="aml-save-editor-${os}-x86_64"
  local zip="$dist/${name}.zip"
  [ -f "$bin" ] || { echo "ERROR: missing built binary: $bin" >&2; exit 1; }
  [ -f "$zip" ] || { echo "ERROR: need existing $zip for the aml-data payload" >&2; exit 1; }
  # Verify the binary carries the current version (guards against a stale build).
  # Use grep -c (consumes all input) not grep -q, so `strings` never takes SIGPIPE
  # under `set -o pipefail` — that would false-positive as a stale build.
  local hits; hits="$(strings "$bin" | grep -c "Aincrad Save Editor $ver" || true)"
  if [ "$hits" -eq 0 ]; then
    echo "ERROR: $bin does not contain version string $ver (stale build?)" >&2; exit 1
  fi
  local stage; stage="$(mktemp -d)"
  ( cd "$stage" && unzip -q "$zip" )              # reuse thumbnails/font/etc.
  # Drop any previously-shipped binary names so a rename can't leave two exes
  # in the zip (the payload is reused across releases).
  rm -f "$stage/$name/aml-save-editor" "$stage/$name/aml-save-editor.exe" \
        "$stage/$name/Aincrad-Save-Editor" "$stage/$name/Aincrad-Save-Editor.exe"
  cp "$bin" "$stage/$name/$exe"                   # swap in the fresh binary
  # Regenerate the in-zip README from the tracked template (the reused payload
  # once carried a 0.1.8-era README for six releases straight).
  sed "s/__VER__/$ver/" "$here/scripts/zip-readme.txt" > "$stage/$name/README.txt"
  [ -f "$here/LICENSE" ] && cp "$here/LICENSE" "$stage/$name/LICENSE.txt"
  rm -f "$zip"
  ( cd "$stage" && zip -qr "$zip" "$name" )
  rm -rf "$stage"
  echo "  -> $zip"
}

echo "Building Linux (native)…"
cargo build --release -p aml-save-editor
package linux   Aincrad-Save-Editor     "$here/target/release/aml-save-editor"

echo "Building Windows (MSVC via cargo-xwin)…"
cargo xwin build --release -p aml-save-editor --target x86_64-pc-windows-msvc
package windows Aincrad-Save-Editor.exe "$here/target/x86_64-pc-windows-msvc/release/aml-save-editor.exe"

echo "Done. dist:"; ls -la "$dist"/*.zip
