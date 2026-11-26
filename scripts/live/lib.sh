#!/usr/bin/env bash
# Helpers for scripts/live-nodes.sh (sourced; not executable on its own).
#
# Every node check creates its own bundle + workflow, runs it through the public engine API,
# waits for run_finished over SSE, then stores run.json, the output node's artifacts and a
# result.json under "$OUT_DIR/<node>/". Nothing here prints API keys (it never sees them).

LIVE_BUNDLES=()
LIVE_WORKFLOWS=()

live_log() { echo "[live] $*" >&2; }

live_cleanup_resources() {
  local id
  for id in "${LIVE_WORKFLOWS[@]:-}"; do
    [[ -n "$id" ]] && curl -s -o /dev/null -X DELETE "$API/workflows/$id" || true
  done
  for id in "${LIVE_BUNDLES[@]:-}"; do
    [[ -n "$id" ]] && curl -s -o /dev/null -X DELETE "$API/bundles/$id" || true
  done
}

# live_curl <outfile> <expected-status> <curl args...>; returns 1 (with a message) on mismatch.
live_curl() {
  local out="$1" want="$2"
  shift 2
  local got
  got=$(curl -sS -o "$out" -w '%{http_code}' "$@") || { live_log "curl failed: $*"; return 1; }
  [[ "$got" == "$want" ]] || { live_log "expected HTTP $want, got $got: $* ($(head -c 300 "$out" | tr '\n' ' '))"; return 1; }
}

live_json_post() { live_curl "$1" "$2" -X POST -H 'Content-Type: application/json' -d "$4" "$3"; }

# live_make_bundle <name> <pdf> -> sets LIVE_ID (not echoed: the id must be registered for
# cleanup in this shell, not in a command-substitution subshell)
live_make_bundle() {
  local tmp id
  tmp=$(mktemp)
  live_json_post "$tmp" 201 "$API/bundles" "$(jq -nc --arg n "$1" '{name: $n}')" || { rm -f "$tmp"; return 1; }
  id=$(jq -er .id "$tmp")
  LIVE_BUNDLES+=("$id")
  live_curl "$tmp" 201 -X POST -F "files=@$2;type=application/pdf" "$API/bundles/$id/files" || { rm -f "$tmp"; return 1; }
  rm -f "$tmp"
  LIVE_ID="$id"
}

# live_make_workflow <name> <graph-json> -> sets LIVE_ID
live_make_workflow() {
  local tmp id
  tmp=$(mktemp)
  live_json_post "$tmp" 201 "$API/workflows" "$(jq -nc --arg n "$1" '{name: $n}')" || { rm -f "$tmp"; return 1; }
  id=$(jq -er .id "$tmp")
  LIVE_WORKFLOWS+=("$id")
  live_curl "$tmp" 200 -X PUT -H 'Content-Type: application/json' \
    -d "$(jq -nc --argjson g "$2" '{graph: $g}')" "$API/workflows/$id" || { rm -f "$tmp"; return 1; }
  rm -f "$tmp"
  LIVE_ID="$id"
}

# ---- graph building (jq) -------------------------------------------------------------------

# live_base_graph <bundle-id>: bundle -> pick_pdf -> extract (the common front end)
live_base_graph() {
  jq -nc --arg b "$1" '{
    nodes: [
      {id: "n_b", kind: "bundle",   position: {x: 0,   y: 0}, params: {bundleId: $b}, isOutput: false},
      {id: "n_p", kind: "pick_pdf", position: {x: 250, y: 0}, params: {index: 0},      isOutput: false},
      {id: "n_x", kind: "extract",  position: {x: 500, y: 0}, params: {},              isOutput: false}
    ],
    edges: [
      {id: "e_b_p", source: "n_b", sourceHandle: "files", target: "n_p", targetHandle: "files"},
      {id: "e_p_x", source: "n_p", sourceHandle: "pdf",   target: "n_x", targetHandle: "pdf"}
    ]}'
}

# live_add_node <id> <kind> <params-json> <isOutput true|false>   (graph on stdin)
live_add_node() {
  jq -c --arg id "$1" --arg kind "$2" --argjson params "$3" --argjson out "$4" \
    '.nodes += [{id: $id, kind: $kind, position: {x: (250 * (.nodes | length)), y: 200}, params: $params, isOutput: $out}]'
}

# live_add_edge <source> <sourceHandle> <target> <targetHandle>   (graph on stdin)
live_add_edge() {
  jq -c --arg s "$1" --arg sh "$2" --arg t "$3" --arg th "$4" \
    '.edges += [{id: ("e_" + $s + "_" + $sh + "_" + $t + "_" + $th), source: $s, sourceHandle: $sh, target: $t, targetHandle: $th}]'
}

# ---- running -------------------------------------------------------------------------------

live_now() { date +%s.%N; }

# live_preview <file> -> first 300 chars of a text artifact on one line, or a note for binaries
live_preview() {
  local f="$1"
  case "$f" in
    *.md | *.txt | *.json | *.csv) head -c 600 "$f" | tr '\r\n\t|' '    ' | tr -s ' ' | cut -c1-300 ;;
    *.pdf) echo "(PDF)" ;;
    *.mp3) echo "(MP3)" ;;
    *.jpg) echo "(JPG)" ;;
    *) echo "" ;;
  esac
}

# live_write_result <node-dir> <node> <status> <seconds> <error> <preview> [extra-json]
live_write_result() {
  local dir="$1" extra="${7:-}"
  [[ -n "$extra" ]] || extra='{}'
  local files='[]' f
  while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    files=$(jq -c --arg n "$(basename "$f")" --argjson s "$(stat -c %s "$f")" '. + [{name: $n, bytes: $s}]' <<<"$files")
  done < <(find "$dir/artifacts" -type f 2>/dev/null | sort)
  jq -n --arg node "$2" --arg status "$3" --arg secs "$4" --arg err "$5" --arg prev "$6" \
    --argjson files "$files" --argjson extra "$extra" --arg nodes "$(cat "$dir/node-status.txt" 2>/dev/null || true)" \
    '{node: $node, status: $status, seconds: ($secs | tonumber), error: $err, preview: $prev, artifacts: $files, pipeline: $nodes} + $extra' \
    >"$dir/result.json"
}

# live_run_workflow <node> <workflow-id> <output-node-ids...>
# Runs the workflow, waits for run_finished, downloads the output nodes' artifacts and writes
# result.json. Returns 0 when the run succeeded, 1 otherwise (result.json is written either way).
live_run_workflow() {
  local node="$1" wf="$2"
  shift 2
  local dir="$OUT_DIR/$node"
  mkdir -p "$dir/artifacts"
  local t0 t1 secs status="failed" error="" preview="" tmp
  tmp=$(mktemp)
  t0=$(live_now)

  if ! curl -sS -o "$tmp" -w '%{http_code}' -X POST "$API/workflows/$wf/runs" >"$dir/start.code" 2>/dev/null; then
    error="실행 요청 실패(네트워크)"
  elif [[ "$(cat "$dir/start.code")" == 202 ]]; then
    local run_id
    run_id=$(jq -er .runId "$tmp")
    # SSE until the server closes the stream after run_finished (or the timeout hits).
    curl -sN --max-time "${NODE_TIMEOUT:-600}" "$API/runs/$run_id/events" 2>/dev/null | sed -n 's/^data: *//p' >"$dir/events.jsonl" || true
    curl -sS -o "$dir/run.json" "$API/runs/$run_id" || true
    if [[ "$(tail -n1 "$dir/events.jsonl" 2>/dev/null | jq -r .type 2>/dev/null)" != "run_finished" ]]; then
      error="타임아웃(${NODE_TIMEOUT:-600}s) 안에 run_finished 를 받지 못했습니다"
    fi
  else
    # 4xx/5xx: keep the engine's error body (validation issues etc.)
    cp "$tmp" "$dir/run.json"
    error="실행 거부 HTTP $(cat "$dir/start.code"): $(jq -r '[.error.message] + [(.error.issues // [])[] | .code] | join(" ")' "$tmp" 2>/dev/null | cut -c1-300)"
  fi
  t1=$(live_now)
  secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.1f", b - a }')

  if [[ -s "$dir/run.json" ]] && jq -e '.nodes' "$dir/run.json" >/dev/null 2>&1; then
    status=$(jq -r .status "$dir/run.json")
    jq -r '[.nodes[] | "\(.kind):\(if .status == "succeeded" then "✓" elif .status == "failed" then "✗" elif .status == "skipped" then "–" else .status end)"] | join(" ")' \
      "$dir/run.json" >"$dir/node-status.txt"
    local failed
    failed=$(jq -r '[.nodes[] | select(.status == "failed") | "\(.kind): \(.error // "(오류 문구 없음)")"] | join(" / ")' "$dir/run.json")
    [[ -n "$failed" ]] && error="$failed"
    if [[ "$status" == succeeded ]]; then
      local id f
      for id in "$@"; do
        while IFS=$'\t' read -r sock idx fname url; do
          [[ -z "$url" ]] && continue
          f="$dir/artifacts/${id}-${sock}-${idx}-${fname}"
          curl -sS -o "$f" "$BASE_URL$url" || error="${error:+$error; }산출물 다운로드 실패: $fname"
        done < <(jq -r --arg id "$id" '.nodes[] | select(.nodeId == $id) | .outputs[] | .socketId as $s | .files[] | [$s, (.index | tostring), .fileName, .url] | @tsv' "$dir/run.json")
      done
      local first
      first=$(find "$dir/artifacts" -type f 2>/dev/null | sort | head -n1)
      [[ -n "$first" ]] && preview=$(live_preview "$first")
    fi
  else
    [[ -z "$error" ]] && error="실행 결과(run.json)를 가져오지 못했습니다"
  fi
  rm -f "$tmp"
  live_write_result "$dir" "$node" "$status" "$secs" "$error" "$preview"
  [[ "$status" == succeeded ]]
}

# live_patch_result <node-dir> <jq-filter> [jq args...]   rewrites result.json in place
live_patch_result() {
  local dir="$1" filter="$2"
  shift 2
  jq "$@" "$filter" "$dir/result.json" >"$dir/result.json.tmp" && mv "$dir/result.json.tmp" "$dir/result.json"
}
