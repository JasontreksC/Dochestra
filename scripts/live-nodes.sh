#!/usr/bin/env bash
# Live check of the OpenRouter-backed nodes: builds a real workflow per node through the engine API,
# runs it, and collects artifacts, timings and errors into a report a human can review.
#
#   BASE_URL=<url> LIVE_CONFIRM=1 ./scripts/live-nodes.sh [--only translate,summary,...] [--out live-results] [--doc en|ko]
#   LIVE_CONFIRM=1 ./scripts/live-nodes.sh --with-verify-stack [...]   # isolated stack dochestra-verify:18080
#
# Without LIVE_CONFIRM=1 it only prints the plan and the rough number of paid external calls.
# BASE_URL has no default on purpose (the user's own stack must not be hit by accident).
# With --with-verify-stack the script starts/stops the isolated stack itself (same project, port
# and lock as scripts/verify-stack.sh) and uses it as BASE_URL.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
FIXTURES="$ROOT/fixtures/live"

ALL_NODES=(translate summary analyze slide rag voice fill)
ONLY=""
OUT_BASE="live-results"
DOC=en
WITH_STACK=false

usage() {
  echo "usage: BASE_URL=<url> LIVE_CONFIRM=1 $0 [--only a,b] [--out DIR] [--doc en|ko] [--with-verify-stack]" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only) ONLY="${2:-}"; shift 2 ;;
    --out) OUT_BASE="${2:-}"; shift 2 ;;
    --doc) DOC="${2:-}"; shift 2 ;;
    --with-verify-stack) WITH_STACK=true; shift ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

case "$DOC" in
  en) DOC_KEY=report_en; TARGET_LANG=ko ;;
  ko) DOC_KEY=report_ko; TARGET_LANG=en ;;
  *) echo "live: --doc must be en or ko" >&2; exit 2 ;;
esac
DOC_PDF="$FIXTURES/$DOC_KEY.pdf"
FORM_PDF="$ROOT/engine/tests/fixtures/form.pdf"
[[ -f "$DOC_PDF" ]] || { echo "live: missing $DOC_PDF (run: uv run --project extractor python fixtures/live/make_fixtures.py)" >&2; exit 2; }

NODES=("${ALL_NODES[@]}")
if [[ -n "$ONLY" ]]; then
  IFS=',' read -r -a NODES <<<"$ONLY"
  for n in "${NODES[@]}"; do
    [[ " ${ALL_NODES[*]} " == *" $n "* ]] || { echo "live: unknown node '$n' (known: ${ALL_NODES[*]})" >&2; exit 2; }
  done
fi

if [[ "$WITH_STACK" != true && -z "${BASE_URL:-}" ]]; then
  echo "live: BASE_URL is required (or use --with-verify-stack for an isolated stack)" >&2
  exit 2
fi

# shellcheck disable=SC1091
. "$ROOT/scripts/live/lib.sh"
# shellcheck disable=SC1091
. "$ROOT/scripts/live/nodes.sh"

result_files_of() { # result.json files for one node name (also its -<case> dirs)
  local f
  for f in "$OUT_DIR/$1/result.json" "$OUT_DIR/$1"-*/result.json; do [[ -f "$f" ]] && echo "$f"; done
  return 0
}

# ---- gate: plan only unless LIVE_CONFIRM=1 ---------------------------------------------------
if [[ "${LIVE_CONFIRM:-}" != 1 ]]; then
  echo "live-nodes: PLAN ONLY (no workflows are created, no paid calls are made)"
  echo "  target : ${BASE_URL:-<isolated verify stack http://localhost:18080>}"
  echo "  document: fixtures/live/$DOC_KEY.pdf (translate -> $TARGET_LANG)"
  echo "  nodes and approximate paid OpenRouter calls:"
  for n in "${NODES[@]}"; do printf '    - %-10s %s\n' "$n" "$(live_estimate "$n")"; done
  echo "  Set LIVE_CONFIRM=1 to run for real."
  exit 0
fi

# ---- optional isolated stack -----------------------------------------------------------------
if [[ "$WITH_STACK" == true ]]; then
  avail_gb=$(df --output=avail -BG / | tail -n1 | tr -dc '0-9')
  if ((avail_gb < 10)); then
    echo "live: only ${avail_gb}G free on / (need >= 10G) - aborting before the image build" >&2
    exit 3
  fi
  exec 9>/tmp/dochestra-verify.lock
  flock -w 1800 9
  export COMPOSE_PROJECT_NAME=dochestra-verify
  export WEB_PORT=18080
  export BASE_URL=http://localhost:18080
  if [[ -f .env ]]; then export ENV_FILE=.env; else export ENV_FILE=.env.example; fi
  export COMPOSE_ENV_FILES="$ENV_FILE"
  echo "[live] isolated stack: project=$COMPOSE_PROJECT_NAME port=$WEB_PORT env=$ENV_FILE"
  stack_down() { docker compose down -v --remove-orphans >/dev/null 2>&1 || true; }
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  if ! docker compose up -d --build --wait; then
    echo "live: the isolated stack failed to start (is engine/ mid-change? it must build from the working tree)" >&2
    docker compose logs --no-color --tail 40 engine >&2 || true
    stack_down
    exit 3
  fi
fi
API="$BASE_URL/api"

TS=$(date -u +%Y%m%dT%H%M%SZ)
OUT_DIR="$ROOT/$OUT_BASE/$TS"
mkdir -p "$OUT_DIR"

finish() {
  local rc=$?
  live_cleanup_resources
  if [[ "$WITH_STACK" == true ]]; then stack_down; fi
  exit "$rc"
}
trap finish EXIT

echo "[live] BASE_URL=$BASE_URL doc=$DOC_KEY out=$OUT_DIR"
curl -fsS "$API/health" >/dev/null || { echo "live: $API/health is not reachable" >&2; exit 3; }

# ---- run every selected node (a failure does not stop the others) ----------------------------
failed=0
for n in "${NODES[@]}"; do
  echo "[live] === $n ($(live_estimate "$n"))"
  mkdir -p "$OUT_DIR/$n"
  if "live_node_$n"; then
    echo "[live] $n: done"
  else
    failed=$((failed + 1))
    if [[ -z "$(result_files_of "$n")" ]]; then
      mkdir -p "$OUT_DIR/$n/artifacts"
      live_write_result "$OUT_DIR/$n" "$n" failed 0 "시나리오 준비 실패(번들·워크플로 생성)" ""
    fi
    echo "[live] $n: FAILED - $(for f in $(result_files_of "$n"); do jq -r 'select(.status != "succeeded") | "\(.node): \(.error)"' "$f"; done | head -3 | tr '\n' ' ')"
  fi
done

# ---- SUMMARY.md ------------------------------------------------------------------------------
# result.json of every selected node, including multi-case nodes such as translate-ko/-long/-settings
result_files() {
  local n f
  for n in "${NODES[@]}"; do
    for f in "$OUT_DIR/$n/result.json" "$OUT_DIR/$n"-*/result.json; do
      [[ -f "$f" ]] && echo "$f"
    done
  done
}
cell() { tr '\r\n|' '  /' | tr -s ' '; }
{
  echo "# 라이브 노드 검증 결과"
  echo
  echo "- 실행 시각(UTC): $TS"
  echo "- 대상: \`$BASE_URL\`"
  echo "- 문서: \`fixtures/live/$DOC_KEY.pdf\` (translate → $TARGET_LANG)"
  echo
  echo "| 노드 | 상태 | 소요 시간 | 노드 상태 | 산출물 | 오류 | 미리보기 |"
  echo "| :--- | :--- | ---: | :--- | :--- | :--- | :--- |"
  for f in $(result_files); do
    n=$(jq -r .node "$f")
    st=$(jq -r .status "$f")
    secs=$(jq -r '.seconds | tostring + "s"' "$f")
    pipe=$(jq -r .pipeline "$f" | cell)
    arts=$(jq -r '.artifacts | map("\(.name) (\(.bytes) B)") | join(", ")' "$f" | cell)
    err=$(jq -r .error "$f" | cell)
    prev=$(jq -r .preview "$f" | cell)
    printf '| %s | %s | %s | %s | %s | %s | %s |\n' "$n" "$st" "$secs" "$pipe" "$arts" "$err" "$prev"
  done
  # translate checklist (translate_check.py) for every case that has one
  for f in $(result_files); do
    jq -e '.checks' "$f" >/dev/null 2>&1 || continue
    echo
    echo "### $(jq -r .node "$f") 점검표 (원문 $(jq -r .sourceChars "$f")자 → 번역 $(jq -r .translatedChars "$f")자)"
    echo
    echo "| 항목 | 결과 | 상세 |"
    echo "| :--- | :--- | :--- |"
    jq -r '.checks[] | "| \(.name) | \(if .ok then "✓" else "✗" end) | \(.detail | gsub("\\|"; "/")) |"' "$f"
  done
  echo
  echo "결과 파일: \`<node>/run.json\`(실행 상세), \`<node>/artifacts/\`(출력 노드 산출물), \`<node>/events.jsonl\`(SSE)."
  echo "rag 는 \`rag-query-N.json\`(질의 응답), 미리보기의 [기대 키워드 ✓/✗] 는 정답 키워드 포함 여부입니다."
} >"$OUT_DIR/SUMMARY.md"

echo
echo "[live] summary: $OUT_DIR/SUMMARY.md"
cat "$OUT_DIR/SUMMARY.md"

# skipped nodes are not failures; anything else that is not succeeded is
bad=$(for f in $(result_files); do jq -r 'select(.status != "succeeded" and .status != "skipped") | .node' "$f" 2>/dev/null; done | wc -l)
if ((bad > 0)); then
  echo "[live] $bad node(s) did not succeed" >&2
  exit 1
fi
echo "[live] all nodes succeeded"
