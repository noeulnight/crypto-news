#!/usr/bin/env bash
set -euo pipefail

image=${1:?image required}
container="crypto-news-smoke-${RANDOM}"
volume="${container}-data"
cleanup() {
  docker logs "$container" || true
  docker rm --force "$container" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --detach --name "$container" \
  --publish 127.0.0.1::8787 --volume "$volume:/app/data" "$image"
port=$(docker port "$container" 8787/tcp | head -1 | cut -d: -f2)
base="http://127.0.0.1:$port"
curl --fail --silent --show-error --retry 30 --retry-all-errors --retry-delay 1 \
  "$base/api/status" >/dev/null
curl --fail --silent --show-error "$base/" | grep -q '<!doctype html>'
curl --fail --silent --show-error "$base/news" | grep -q '<!doctype html>'
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/api/overview?asset=invalid")" = 400
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "$base/api/unknown")" = 404
test "$(docker exec "$container" id -u)" = 10001
docker exec "$container" test -s /app/data/crypto-news.db
# A marker confirms the same writable volume survives a container restart.
docker exec "$container" touch /app/data/.smoke
docker restart "$container" >/dev/null
port=$(docker port "$container" 8787/tcp | head -1 | cut -d: -f2)
base="http://127.0.0.1:$port"
curl --fail --silent --show-error --retry 30 --retry-all-errors --retry-delay 1 \
  "$base/api/status" >/dev/null
docker exec "$container" test -f /app/data/.smoke
