#!/usr/bin/env bash
# Starts (idempotently) the local pgvector dev/test database container.
set -euo pipefail

NAME=dochestra-pg-dev

state="$(docker inspect -f '{{.State.Status}}' "$NAME" 2>/dev/null || true)"
case "$state" in
  running) ;;
  "")
    docker run -d --name "$NAME" \
      -e POSTGRES_USER=dochestra -e POSTGRES_PASSWORD=dochestra -e POSTGRES_DB=dochestra \
      -p 127.0.0.1:55432:5432 pgvector/pgvector:pg16 >/dev/null
    ;;
  *) docker start "$NAME" >/dev/null ;;
esac

for _ in $(seq 1 60); do
  if docker exec "$NAME" pg_isready -U dochestra >/dev/null 2>&1; then
    echo "$NAME ready (postgres://dochestra:dochestra@127.0.0.1:55432/dochestra)"
    exit 0
  fi
  sleep 1
done
echo "$NAME did not become ready within 60s" >&2
exit 1
