#!/usr/bin/env bash
# Upload a built zip to Nexus Mods via the v3 API (single-part, files <100 MiB).
#
# Flow (https://api-docs.nexusmods.com): create upload session -> PUT the bytes to
# the presigned URL -> finalise -> poll until `available` -> create a new mod-file
# VERSION (of an existing file) or a brand-NEW mod file.
#
# Publishing is outward-facing, so this DRY-RUNS by default: it validates inputs
# and prints the plan without touching Nexus. Add --publish to actually upload.
#
# Auth: personal API key from https://www.nexusmods.com/settings/api-keys, via
# --key or the NEXUS_API_KEY env var. Needs curl + jq.
#
# Usage:
#   # new VERSION of an existing mod file (the common case — e.g. the editor):
#   nexus-upload.sh --file dist/aml-save-editor-windows-x86_64.zip \
#     --file-id <FILE_ID> --version 0.1.9 --name "Aincrad Save Editor 0.1.9" --publish
#
#   # brand-NEW file on a mod page (first upload for a new mod):
#   nexus-upload.sh --file mods.zip --mod-id <GAME_SCOPED_MOD_ID> \
#     --version 0.1.0 --name "Echoes of Aincrad mods" --publish
#
# Find FILE_ID: on the mod's Files tab, click a file's "Manual download" — the URL
# ends with `file_id=<N>`. Find the game-scoped MOD_ID via GET /v3/mods or the URL.
set -euo pipefail

API="https://api.nexusmods.com/v3"
file="" file_id="" mod_id="" version="" name="" desc="" category="main"
key="${NEXUS_API_KEY:-}" publish=0

die() { echo "ERROR: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
  case "$1" in
    --file) file="$2"; shift 2;;
    --file-id) file_id="$2"; shift 2;;
    --mod-id) mod_id="$2"; shift 2;;
    --version) version="$2"; shift 2;;
    --name) name="$2"; shift 2;;
    --desc) desc="$2"; shift 2;;
    --category) category="$2"; shift 2;;
    --key) key="$2"; shift 2;;
    --publish) publish=1; shift;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0;;
    *) die "unknown arg: $1";;
  esac
done

[ -n "$file" ] && [ -f "$file" ] || die "--file <zip> required (and must exist)"
[ -n "$version" ] || die "--version required"
[ -n "$name" ] || name="$(basename "$file")"
[ -n "$file_id" ] || [ -n "$mod_id" ] || die "one of --file-id (new version) or --mod-id (new file) required"
[ -z "$file_id" ] || [ -z "$mod_id" ] || die "--file-id and --mod-id are mutually exclusive"
size="$(stat -c%s "$file")"
[ "$size" -le $((100*1024*1024)) ] || die "file is >100 MiB; needs the multipart flow (not implemented here)"
[[ "$version" =~ ^[a-zA-Z0-9.-]+$ ]] || die "version must match ^[a-zA-Z0-9.-]+$"
[[ "$name" =~ ^[a-zA-Z0-9\ _\'\(\).-]+$ ]] || die "name has characters Nexus rejects (allowed: a-zA-Z0-9 _'().-)"

mode=$([ -n "$file_id" ] && echo "new version of file $file_id" || echo "new file on mod $mod_id")
echo "Plan: upload $(basename "$file") ($size bytes) as $mode"
echo "  version=$version  name=\"$name\"  category=$category"
if [ "$publish" -ne 1 ]; then
  echo "DRY RUN — add --publish to actually upload. (No Nexus request was made.)"
  exit 0
fi
# Key resolution: --key > $NEXUS_API_KEY > the key aml stored (aml nxm set-key).
if [ -z "$key" ]; then
  for cfg in "${XDG_CONFIG_HOME:-$HOME/.config}/aml/config.json" "$(dirname "$0")/../aml-data/config.json"; do
    if [ -f "$cfg" ]; then
      key="$(jq -r '.nexus_api_key // empty' "$cfg" 2>/dev/null || true)"
      [ -n "$key" ] && { echo "(using the Nexus key stored by aml: $cfg)"; break; }
    fi
  done
fi
[ -n "$key" ] || die "no key: pass --key, set NEXUS_API_KEY, or run 'aml nxm set-key <k>'"

api() { # api METHOD PATH [json-body]
  local m="$1" p="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -fsS -X "$m" "$API$p" -H "apikey: $key" -H 'Content-Type: application/json' -d "$body"
  else
    curl -fsS -X "$m" "$API$p" -H "apikey: $key"
  fi
}

echo "1/5 create upload session…"
up="$(api POST /uploads "$(jq -n --argjson s "$size" --arg f "$(basename "$file")" '{size_bytes:$s, filename:$f}')")"
uid="$(echo "$up" | jq -r '.data.id')"
purl="$(echo "$up" | jq -r '.data.presigned_url')"
[ -n "$uid" ] && [ "$uid" != null ] || die "no upload id in response: $up"

echo "2/5 PUT file to presigned URL…"
curl -fsS -X PUT "$purl" --data-binary @"$file" -H 'Content-Type: application/octet-stream' >/dev/null

echo "3/5 finalise…"
api POST "/uploads/$uid/finalise" >/dev/null

echo "4/5 poll until available…"
for _ in $(seq 1 60); do
  st="$(api GET "/uploads/$uid" | jq -r '.data.state')"
  echo "   state=$st"
  [ "$st" = available ] && break
  sleep 3
done
[ "${st:-}" = available ] || die "upload never became available (last state: ${st:-none})"

echo "5/5 create mod $([ -n "$file_id" ] && echo "file version" || echo "file")…"
if [ -n "$file_id" ]; then
  body="$(jq -n --arg u "$uid" --arg n "$name" --arg v "$version" --arg c "$category" --arg d "$desc" \
    '{upload_id:$u, name:$n, version:$v, file_category:$c} + (if $d=="" then {} else {description:$d} end)')"
  out="$(api POST "/mod-files/$file_id/versions" "$body")"
else
  body="$(jq -n --arg u "$uid" --arg m "$mod_id" --arg n "$name" --arg v "$version" --arg c "$category" --arg d "$desc" \
    '{upload_id:$u, mod_id:$m, name:$n, version:$v, file_category:$c} + (if $d=="" then {} else {description:$d} end)')"
  out="$(api POST /mod-files "$body")"
fi
echo "$out" | jq .
echo "Done — uploaded to Nexus."
