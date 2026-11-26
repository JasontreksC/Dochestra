#!/usr/bin/env bash
# API-level end-to-end scenario against a running stack:
#   bundle upload -> workflow (bundle -> pick_pdf -> extract, + json_input) -> validate -> run
#   -> SSE until run_finished -> run detail -> artifact downloads.
#
# BASE_URL is required on purpose (no default) so the user's default stack is never hit by accident.
# Normally run through ./scripts/verify-stack.sh --e2e, which provides an isolated stack.
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -z "${BASE_URL:-}" ]]; then
  echo "e2e: BASE_URL is required (use ./scripts/verify-stack.sh --e2e for an isolated stack)" >&2
  exit 2
fi
API="$BASE_URL/api"
SAMPLE=extractor/tests/fixtures/sample.pdf
[[ -f "$SAMPLE" ]] || { echo "e2e: missing $SAMPLE" >&2; exit 2; }

TMP=$(mktemp -d)
BUNDLE_ID=""
WF_ID=""
WF_NEG_ID=""

cleanup() {
  local rc=$?
  for id in "$WF_ID" "$WF_NEG_ID"; do
    [[ -n "$id" ]] && curl -s -o /dev/null -X DELETE "$API/workflows/$id" || true
  done
  [[ -n "$BUNDLE_ID" ]] && curl -s -o /dev/null -X DELETE "$API/bundles/$BUNDLE_ID" || true
  rm -f "$TMP"/* || true
  rmdir "$TMP" || true
  exit "$rc"
}
trap cleanup EXIT

pass() { echo "PASS  $1"; }
fail() { # fail <step> [file with response body]
  echo "FAIL  $1" >&2
  if [[ -n "${2:-}" && -f "$2" ]]; then
    echo "----- response -----" >&2
    head -c 4000 "$2" >&2
    echo >&2
    echo "--------------------" >&2
  fi
  exit 1
}

# http <outfile> <expected-status> <curl args...>
http() {
  local out="$1" want="$2"
  shift 2
  local got
  got=$(curl -sS -o "$out" -w '%{http_code}' "$@") || fail "curl failed: $*" "$out"
  [[ "$got" == "$want" ]] || fail "expected HTTP $want, got $got: $*" "$out"
}

json_post() { # json_post <outfile> <status> <url> <json>
  http "$1" "$2" -X POST -H 'Content-Type: application/json' -d "$4" "$3"
}

# ---- 1. bundle + upload ---------------------------------------------------------------
json_post "$TMP/bundle.json" 201 "$API/bundles" '{"name":"e2e"}'
BUNDLE_ID=$(jq -er .id "$TMP/bundle.json")
http "$TMP/upload.json" 201 -X POST -F "files=@$SAMPLE;type=application/pdf" "$API/bundles/$BUNDLE_ID/files"
jq -e 'length == 1' "$TMP/upload.json" >/dev/null || fail "1 upload returned unexpected body" "$TMP/upload.json"
pass "1 bundle created and sample.pdf uploaded ($BUNDLE_ID)"

# ---- 2. workflow ----------------------------------------------------------------------
json_post "$TMP/wf.json" 201 "$API/workflows" '{"name":"e2e"}'
WF_ID=$(jq -er .id "$TMP/wf.json")

GRAPH=$(jq -n --arg bundle "$BUNDLE_ID" '{
  nodes: [
    {id: "n_b", kind: "bundle",     position: {x: 0,   y: 0},   params: {bundleId: $bundle},                 isOutput: false},
    {id: "n_p", kind: "pick_pdf",   position: {x: 250, y: 0},   params: {index: 0},                          isOutput: false},
    {id: "n_x", kind: "extract",    position: {x: 500, y: 0},   params: {},                                  isOutput: true},
    {id: "n_j", kind: "json_input", position: {x: 0,   y: 200}, params: {value: {targetLanguage: "ko"}},      isOutput: true}
  ],
  edges: [
    {id: "e1", source: "n_b", sourceHandle: "files", target: "n_p", targetHandle: "files"},
    {id: "e2", source: "n_p", sourceHandle: "pdf",   target: "n_x", targetHandle: "pdf"}
  ]
}')
http "$TMP/put.json" 200 -X PUT -H 'Content-Type: application/json' -d "{\"graph\": $GRAPH}" "$API/workflows/$WF_ID"
pass "2 workflow graph saved ($WF_ID)"

# ---- 3. validate ----------------------------------------------------------------------
json_post "$TMP/validate.json" 200 "$API/workflows/$WF_ID/validate" ''
jq -e '.ok == true' "$TMP/validate.json" >/dev/null || fail "3 validate not ok" "$TMP/validate.json"
pass "3 validate ok"

# ---- 4. negative: no output node -> 422 NO_OUTPUT_NODE --------------------------------
json_post "$TMP/wfneg.json" 201 "$API/workflows" '{"name":"e2e-neg"}'
WF_NEG_ID=$(jq -er .id "$TMP/wfneg.json")
NEG_GRAPH=$(echo "$GRAPH" | jq '.nodes |= map(.isOutput = false)')
http "$TMP/negput.json" 200 -X PUT -H 'Content-Type: application/json' -d "{\"graph\": $NEG_GRAPH}" "$API/workflows/$WF_NEG_ID"
json_post "$TMP/negrun.json" 422 "$API/workflows/$WF_NEG_ID/runs" ''
jq -e '.error.issues | map(.code) | index("NO_OUTPUT_NODE") != null' "$TMP/negrun.json" >/dev/null \
  || fail "4 422 body lacks NO_OUTPUT_NODE" "$TMP/negrun.json"
pass "4 run without output node rejected (422 NO_OUTPUT_NODE)"

# ---- 5. start run ---------------------------------------------------------------------
json_post "$TMP/run.json" 202 "$API/workflows/$WF_ID/runs" ''
RUN_ID=$(jq -er .runId "$TMP/run.json")
pass "5 run started ($RUN_ID)"

# ---- 6. SSE ---------------------------------------------------------------------------
sse_rc=0
curl -sN --max-time 600 "$API/runs/$RUN_ID/events" | sed -n 's/^data: *//p' >"$TMP/events.jsonl" || sse_rc=$?
[[ -s "$TMP/events.jsonl" ]] || fail "6 no SSE events received (curl rc=$sse_rc)"
jq -e . "$TMP/events.jsonl" >/dev/null || fail "6 SSE data lines are not all valid JSON" "$TMP/events.jsonl"
[[ "$(head -n1 "$TMP/events.jsonl" | jq -r .type)" == "snapshot" ]] || fail "6 first SSE event is not snapshot" "$TMP/events.jsonl"
[[ "$(tail -n1 "$TMP/events.jsonl" | jq -r .type)" == "run_finished" ]] \
  || fail "6 stream ended without run_finished (curl rc=$sse_rc)" "$TMP/events.jsonl"

# Per node: status sequence from the snapshot plus node_status events, consecutive repeats
# collapsed (an event may repeat what the snapshot already showed). Printed below.
STATUS_REPORT=$(jq -s -r '
  (.[0].run.nodes | map({key: .nodeId, value: [.status]}) | from_entries) as $init
  | reduce (.[] | select(.type == "node_status")) as $e ($init; .[$e.nodeId] += [$e.status])
  | to_entries[]
  | .key + " " + (.value | reduce .[] as $s ([]; if .[-1] == $s then . else . + [$s] end) | join(","))' "$TMP/events.jsonl")
echo "$STATUS_REPORT" | sed 's/^/        node /'
while read -r node seq; do
  [[ -z "$node" ]] && continue
  last=${seq##*,}
  case "$last" in
    succeeded | failed | skipped) ;;
    *) fail "6 node $node never reached a final status (sequence: $seq)" ;;
  esac
  # Once a node reached a final status nothing else may follow, and 'running' must precede it.
  case ",$seq," in
    *,succeeded,*,* | *,failed,*,* | *,skipped,*,*) fail "6 node $node reported a status after its final one ($seq)" ;;
  esac
done <<<"$STATUS_REPORT"
pass "6 SSE: snapshot first, run_finished last, every node reached a final status in order"

# ---- 7. run detail --------------------------------------------------------------------
http "$TMP/detail.json" 200 "$API/runs/$RUN_ID"
if ! jq -e '.status == "succeeded" and (.nodes | length == 4) and all(.nodes[]; .status == "succeeded")' \
  "$TMP/detail.json" >/dev/null; then
  echo "run status: $(jq -r .status "$TMP/detail.json")" >&2
  jq -r '.nodes[] | "  \(.nodeId) (\(.kind)): \(.status)\(if .error then " — error: " + .error else "" end)"' \
    "$TMP/detail.json" >&2
  fail "7 run did not succeed with 4 succeeded nodes" "$TMP/detail.json"
fi
pass "7 run succeeded, 4 nodes succeeded"

# ---- 8. artifacts ---------------------------------------------------------------------
artifact() { # artifact <node> <socket> <outfile> [index] -> prints content-type
  local url="$API/runs/$RUN_ID/artifacts/$1/$2?index=${4:-0}"
  curl -sS -D "$TMP/headers.txt" -o "$3" -w '%{http_code}' "$url" >"$TMP/code.txt" || fail "8 download failed: $url"
  [[ "$(cat "$TMP/code.txt")" == 200 ]] || fail "8 $1/$2 returned HTTP $(cat "$TMP/code.txt")" "$3"
  grep -i '^content-type:' "$TMP/headers.txt" | tr -d '\r' | sed 's/^[^:]*: *//' | tr '[:upper:]' '[:lower:]'
}

ct=$(artifact n_x md "$TMP/md.md")
[[ "$ct" == text/markdown* ]] || fail "8 extract md Content-Type is '$ct'"
grep -q 'Annual Sales Report' "$TMP/md.md" || fail "8 extract md lacks 'Annual Sales Report'" "$TMP/md.md"
pass "8a extract md ($ct) contains 'Annual Sales Report'"

ct=$(artifact n_x json "$TMP/extract.json")
[[ "$ct" == application/json* ]] || fail "8 extract json Content-Type is '$ct'"
jq -e . "$TMP/extract.json" >/dev/null || fail "8 extract json is not parseable"
pass "8b extract json parses ($ct)"

ct=$(artifact n_x tables "$TMP/table.csv" 0)
[[ "$ct" == text/csv* ]] || fail "8 extract tables[0] Content-Type is '$ct'"
[[ -s "$TMP/table.csv" ]] || fail "8 extract tables[0] is empty"
pass "8c extract tables[0] non-empty ($ct)"

ct=$(artifact n_x images "$TMP/image.jpg" 0)
[[ "$ct" == image/jpeg* ]] || fail "8 extract images[0] Content-Type is '$ct'"
[[ -s "$TMP/image.jpg" ]] || fail "8 extract images[0] is empty"
pass "8d extract images[0] non-empty ($ct)"

artifact n_p pdf "$TMP/picked.pdf" >/dev/null
cmp -s "$TMP/picked.pdf" "$SAMPLE" || fail "8 pick_pdf output differs from sample.pdf"
pass "8e pick_pdf output identical to sample.pdf"

artifact n_j json "$TMP/input.json" >/dev/null
[[ "$(jq -S -c . "$TMP/input.json")" == '{"targetLanguage":"ko"}' ]] \
  || fail "8 json_input output differs from {\"targetLanguage\":\"ko\"}" "$TMP/input.json"
pass "8f json_input output equals {\"targetLanguage\":\"ko\"}"

# ---- 9. history -----------------------------------------------------------------------
http "$TMP/history.json" 200 "$API/workflows/$WF_ID/runs"
[[ "$(jq -r '.[0].id' "$TMP/history.json")" == "$RUN_ID" ]] || fail "9 latest run in history is not $RUN_ID" "$TMP/history.json"
pass "9 run history lists this run first"

echo
echo "e2e: all steps passed"
