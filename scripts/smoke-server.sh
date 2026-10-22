#!/usr/bin/env bash
set -euo pipefail

# Starts a built server against a throwaway DB and checks health, the SPA and 404s.
# Build client/dist and the binary first (default target/debug/version-server).
binary=${1:-target/debug/version-server}
smoke_dir=$(mktemp -d)
export PORT=${PORT:-43127}
export VERSION_SERVER_DB="$smoke_dir/version-server.db" WATCH_REPOS='' GITHUB_WEBHOOK_SECRET=''
"$binary" &
server_pid=$!
trap 'kill -TERM "$server_pid" 2>/dev/null || true; rm -rf "$smoke_dir"' EXIT

for attempt in {1..20}; do
  if curl --fail --silent --show-error http://127.0.0.1:$PORT/healthz >/dev/null; then
    break
  fi
  if [[ "$attempt" == 20 ]]; then
    exit 1
  fi
  sleep 0.25
done

test "$(curl --fail --silent http://127.0.0.1:$PORT/healthz)" = "ok"
test "$(curl --fail --silent http://127.0.0.1:$PORT/api/health)" = '{"status":"ok"}'
curl --fail --silent http://127.0.0.1:$PORT/ | grep --quiet --ignore-case '<!doctype html'
test "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:$PORT/api/missing)" = "404"
test "$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:$PORT/projects/example)" = "200"
echo 'Server serves health, the SPA and 404s'
