#!/usr/bin/env bash
# Authenticate with Space-Track and fetch one small GP record without violating
# the provider's once-per-hour GP retrieval guidance.

set -euo pipefail

readonly LOGIN_URL="https://www.space-track.org/ajaxauth/login"
readonly GP_URL="https://www.space-track.org/basicspacedata/query/class/gp/NORAD_CAT_ID/25544/format/json"
readonly MIN_GP_INTERVAL_SECONDS="${SPACE_TRACK_MIN_GP_INTERVAL_SECONDS:-3600}"
readonly REQUEST_DELAY_SECONDS="${SPACE_TRACK_REQUEST_DELAY_SECONDS:-5}"
readonly STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/world-at-war"
readonly LAST_GP_QUERY_FILE="$STATE_DIR/space-track-test.last-gp-query"

read -r -p "Space-Track username: " ST_USER
read -r -s -p "Space-Track password: " ST_PASS
printf '\n'

cookie_jar=$(mktemp)
result=$(mktemp)
trap 'rm -f "$cookie_jar" "$result"' EXIT

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

if ! grep -q '^[[:space:]]*\[' "$result"; then
  printf 'GP request returned HTTP %s but not a JSON catalog. The first 500 bytes follow:\n' "$gp_status" >&2
  head -c 500 "$result" >&2
  printf '\n' >&2
  exit 1
fi

printf 'GP request succeeded with HTTP %s. First 500 bytes:\n' "$gp_status"
head -c 500 "$result"
printf '\n'
