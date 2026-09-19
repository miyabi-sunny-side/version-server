#!/usr/bin/env bash
set -euo pipefail

# Polling constructs the HTTPS client even when its first request fails.
# A closed local port keeps this startup check independent of GitHub.
container=$(docker run -d -p 127.0.0.1::3000 \
  -e WATCH_REPOS=example/repo -e GITHUB_API_URL=http://127.0.0.1:9 \
  "${1:?usage: smoke-container.sh IMAGE}")
trap 'docker logs "$container" >&2 2>/dev/null || true; docker rm -f "$container" >/dev/null 2>&1 || true' EXIT
port=$(docker port "$container" 3000/tcp)
for _ in {1..40}; do
  if response=$(curl --fail --silent "http://$port/healthz") && [[ "$response" == ok ]]; then
    echo 'Container starts with polling enabled'
    exit 0
  fi
  if [[ $(docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null) != true ]]; then
    break
  fi
  sleep 0.25
done
exit 1
