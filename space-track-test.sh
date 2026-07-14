#!/usr/bin/env bash
# Authenticate with Space-Track and cache the current GP catalog for local app
# runs without violating the provider's once-per-hour GP retrieval guidance.

set -euo pipefail

readonly LOGIN_URL="https://www.space-track.org/ajaxauth/login"
readonly GP_URL="https://www.space-track.org/basicspacedata/query/class/gp/decay_date/null-val/epoch/%3Enow-10/orderby/norad_cat_id/format/json"
readonly MIN_GP_INTERVAL_SECONDS="${SPACE_TRACK_MIN_GP_INTERVAL_SECONDS:-3600}"
readonly REQUEST_DELAY_SECONDS="${SPACE_TRACK_REQUEST_DELAY_SECONDS:-5}"
readonly STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/world-at-war"
readonly LAST_GP_QUERY_FILE="$STATE_DIR/space-track-test.last-gp-query"
readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly CACHE_DIR="$SCRIPT_DIR/data/cache/space-track"
readonly CACHE_FILE="$CACHE_DIR/latest.json"

read -r -p "Space-Track username: " ST_USER
read -r -s -p "Space-Track password: " ST_PASS
printf '\n'

cookie_jar=$(mktemp)
result=$(mktemp)
mkdir -p "$CACHE_DIR"
cache_tmp=$(mktemp "$CACHE_DIR/latest.json.XXXXXX")
trap 'rm -f "$cookie_jar" "$result" "$cache_tmp"' EXIT

if ! login_status=$(curl \
  --location \
  --silent \
  --show-error \
  --fail-with-body \
  --connect-timeout 20 \
  --max-time 60 \
  --cookie-jar "$cookie_jar" \
  --data-urlencode "identity=$ST_USER" \
  --data-urlencode "password=$ST_PASS" \
  --output /dev/null \
  --write-out "%{http_code}" \
  "$LOGIN_URL"); then
  printf 'Login request failed. Check the credentials and Space-Track account status before trying again.\n' >&2
  exit 1
fi

printf 'Login request completed with HTTP %s.\n' "$login_status"

now=$(date +%s)
if [[ -f "$LAST_GP_QUERY_FILE" ]]; then
  last_query=$(<"$LAST_GP_QUERY_FILE")
  if [[ "$last_query" =~ ^[0-9]+$ ]]; then
    elapsed=$((now - last_query))
    if ((elapsed < MIN_GP_INTERVAL_SECONDS)); then
      remaining=$((MIN_GP_INTERVAL_SECONDS - elapsed))
      printf 'Skipping GP request: this script last made one %ss ago. Wait %ss before testing again.\n' \
        "$elapsed" "$remaining"
      exit 0
    fi
  fi
fi

printf 'Waiting %ss before the single GP request...\n' "$REQUEST_DELAY_SECONDS"
sleep "$REQUEST_DELAY_SECONDS"

if ! gp_status=$(curl \
  --location \
  --silent \
  --show-error \
  --fail-with-body \
  --connect-timeout 20 \
  --max-time 60 \
  --cookie "$cookie_jar" \
  --output "$result" \
  --write-out "%{http_code}" \
  "$GP_URL"); then
  printf 'GP request failed. Do not retry the GP query immediately; Space-Track limits GP retrievals to once per hour.\n' >&2
  exit 1
fi

mkdir -p "$STATE_DIR"
printf '%s\n' "$now" > "$LAST_GP_QUERY_FILE"

synced_unix=$(date +%s)
if ! object_count=$(python3 - "$result" "$cache_tmp" "$synced_unix" "$GP_URL" <<'PY'
import hashlib
import json
import sys

input_path, output_path, synced_unix, source = sys.argv[1:]

with open(input_path, encoding="utf-8") as input_file:
    objects = json.load(input_file)

if not isinstance(objects, list) or not objects:
    raise ValueError("expected a non-empty JSON catalog array")

sidcs = {
    "PAYLOAD": "100305000011010000000000000000",
    "ROCKET BODY": "100305000011020000000000000000",
    "DEBRIS": "100305000011030000000000000000",
}
for object_record in objects:
    if isinstance(object_record, dict):
        object_record["SIDC"] = sidcs.get(
            object_record.get("OBJECT_TYPE"), "100305000000000000000000000000"
        )

objects_bytes = json.dumps(objects, separators=(",", ":"), ensure_ascii=False).encode()
snapshot = {
    "synced_unix": int(synced_unix),
    "source": source,
    "checksum": hashlib.sha256(objects_bytes).hexdigest(),
    "objects": objects,
}
with open(output_path, "w", encoding="utf-8") as output_file:
    json.dump(snapshot, output_file, separators=(",", ":"), ensure_ascii=False)
    output_file.write("\n")

print(len(objects))
PY
); then
  printf 'GP request returned HTTP %s but could not be saved as an application catalog. The first 500 bytes follow:\n' "$gp_status" >&2
  head -c 500 "$result" >&2
  printf '\n' >&2
  exit 1
fi

mv "$cache_tmp" "$CACHE_FILE"
printf 'GP request succeeded with HTTP %s. Saved %s objects to %s.\n' \
  "$gp_status" "$object_count" "$CACHE_FILE"
