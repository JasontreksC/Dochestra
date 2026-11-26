#!/usr/bin/env bash
# Isolated end-to-end verification: brings up a private compose project (dochestra-verify) on its
# own port and volumes, runs scripts/smoke.sh against it, then removes everything it created.
# Never touches the default `dochestra` stack (port 80), which belongs to the user.
set -euo pipefail

cd "$(dirname "$0")/.."

RUN_E2E=false
RUN_UI=false
for arg in "$@"; do
  case "$arg" in
    --e2e) RUN_E2E=true ;;
    --ui) RUN_UI=true ;;
    *) echo "usage: $0 [--e2e] [--ui]" >&2; exit 2 ;;
  esac
done

# Serialize concurrent verification runs (workers / orchestrator share one small host).
exec 9>/tmp/dochestra-verify.lock
flock -w 1800 9

export COMPOSE_PROJECT_NAME=dochestra-verify
export WEB_PORT=18080
export BASE_URL=http://localhost:18080

# Use the local .env when present, otherwise the template; never create or modify .env.
if [[ -f .env ]]; then
  export ENV_FILE=.env
else
  export ENV_FILE=.env.example
fi
export COMPOSE_ENV_FILES="$ENV_FILE"

echo "[verify-stack] project=$COMPOSE_PROJECT_NAME port=$WEB_PORT env=$ENV_FILE"

LOG_ROOT="${VERIFY_LOG_DIR:-/tmp/dochestra-verify-logs}"
KEEP_LOG_DIRS=10

# On failure, keep container state and logs before the stack is torn down (diagnostics only).
save_diagnostics() {
  local dir svc
  dir="$LOG_ROOT/$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$dir" || return 0
  docker compose ps -a >"$dir/ps.txt" 2>&1 || true
  for svc in $(docker compose config --services 2>/dev/null); do
    docker compose logs --no-color --tail 200 "$svc" >"$dir/$svc.log" 2>&1 || true
  done
  for svc in engine extractor; do
    local cid
    cid=$(docker compose ps -a -q "$svc" 2>/dev/null | head -n1)
    [[ -n "$cid" ]] && docker inspect --format '{{json .State}}' "$cid" >"$dir/$svc.state.json" 2>&1 || true
  done
  echo "[verify-stack] failure diagnostics saved to $dir"
  # Keep only the newest $KEEP_LOG_DIRS diagnostic directories.
  local old
  while IFS= read -r old; do
    [[ -n "$old" ]] && rm -r -- "$LOG_ROOT/$old"
  done < <(ls -1 "$LOG_ROOT" 2>/dev/null | sort -r | tail -n +"$((KEEP_LOG_DIRS + 1))")
}

cleanup() {
  local rc=$?
  if [[ "$rc" -ne 0 ]]; then
    save_diagnostics || true
  fi
  echo "[verify-stack] cleaning up project $COMPOSE_PROJECT_NAME"
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT

# Start from a clean slate in case an earlier run was killed.
docker compose down -v --remove-orphans >/dev/null 2>&1 || true

docker compose up -d --build --wait
./scripts/smoke.sh
# Test-only hook: simulate a failure right after smoke to exercise the diagnostics path.
if [[ "${VERIFY_FAIL_INJECT:-}" == 1 ]]; then
  echo "[verify-stack] VERIFY_FAIL_INJECT=1: failing on purpose" >&2
  exit 1
fi
if [[ "$RUN_E2E" == true ]]; then
  ./scripts/e2e.sh
fi
if [[ "$RUN_UI" == true ]]; then
  export PATH="$HOME/.local/bin:$PATH" # corepack-managed pnpm shim (CONTEXT §6.2)
  (cd web && E2E_BASE_URL="$BASE_URL" pnpm e2e)
fi
