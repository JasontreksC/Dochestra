#!/usr/bin/env bash
# Smoke test for a running Dochestra compose stack. Run from the repository root:
#   docker compose up -d --build --wait && ./scripts/smoke.sh
set -euo pipefail

cd "$(dirname "$0")/.."

# Every `docker compose` call below follows the caller's COMPOSE_PROJECT_NAME / COMPOSE_ENV_FILES;
# nothing is hard-coded so the script can target an isolated stack (see verify-stack.sh).
ENV_FILE="${ENV_FILE:-.env}"
if [[ -f "$ENV_FILE" ]]; then
  # Values already set by the caller (e.g. WEB_PORT, BASE_URL) win over the env file.
  keep=()
  for v in WEB_PORT BASE_URL POSTGRES_USER POSTGRES_DB; do
    if [[ -n "${!v+x}" ]]; then keep+=("$v=${!v}"); fi
  done
  set -a
  # shellcheck disable=SC1090
  . "./$ENV_FILE"
  set +a
  for kv in "${keep[@]}"; do export "$kv"; done
fi
BASE_URL="${BASE_URL:-http://localhost:${WEB_PORT:-80}}"
: "${POSTGRES_USER:=dochestra}"
: "${POSTGRES_DB:=dochestra}"

failures=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1${2:+ -- $2}"; failures=$((failures + 1)); }

check() { # check <name> <command...>: PASS when the command exits 0
  local name="$1"
  shift
  local out
  if out=$("$@" 2>&1); then pass "$name"; else fail "$name" "$(echo "$out" | tail -n 3 | tr '\n' ' ')"; fi
}

# Remove the scratch directory this script creates inside the engine container's /data volume.
SMOKE_DIR=/data/smoke
cleanup() {
  docker compose exec -T engine rm -rf "$SMOKE_DIR" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# (1) engine health through the web proxy
check "GET /api/health == {\"status\":\"ok\"}" \
  bash -c "curl -fsS '$BASE_URL/api/health' | jq -e '.status == \"ok\"'"

# (2) SPA index
check "GET / -> 200 with <div id=\"root\">" \
  bash -c "curl -fsS '$BASE_URL/' | grep -q '<div id=\"root\">'"

# (3) SPA fallback for client-side routes
check "GET /editor/smoke -> 200 (SPA fallback)" \
  bash -c "test \"\$(curl -s -o /dev/null -w '%{http_code}' '$BASE_URL/editor/smoke')\" = 200"

# (4) node catalog
check "GET /api/nodes -> 11 nodes, first kind == bundle" \
  bash -c "curl -fsS '$BASE_URL/api/nodes' | jq -e 'length == 11 and .[0].kind == \"bundle\"'"

# (5) pgvector extension installed by the engine migration
check "db has the vector extension" \
  bash -c "docker compose exec -T db psql -U '$POSTGRES_USER' -d '$POSTGRES_DB' -tAc \"SELECT extname FROM pg_extension WHERE extname='vector'\" | grep -qx vector"

# (6) shared volume round trip: engine writes PDF -> extractor converts -> engine reads output
cross_volume() {
  docker compose exec -T engine sh -c "mkdir -p $SMOKE_DIR && cat > $SMOKE_DIR/sample.pdf" \
    < extractor/tests/fixtures/sample.pdf
  local resp
  resp=$(docker compose exec -T engine curl -fsS -X POST http://extractor:8000/extract \
    -H 'Content-Type: application/json' \
    -d "{\"pdf_path\":\"$SMOKE_DIR/sample.pdf\",\"output_dir\":\"$SMOKE_DIR/out\"}")
  echo "$resp" | jq -e '(.tables | length >= 1) and (.images | length >= 1)' >/dev/null
  docker compose exec -T engine test -s "$SMOKE_DIR/out/md.md"
}
check "engine -> extractor /extract via shared volume (tables>=1, images>=1, md readable)" cross_volume

echo
if ((failures > 0)); then
  echo "smoke: $failures check(s) FAILED"
  exit 1
fi
echo "smoke: all checks passed"
