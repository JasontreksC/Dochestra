#!/usr/bin/env bash
# Per-node scenarios for scripts/live-nodes.sh (sourced after lib.sh).
# Each live_node_<name> creates its own bundle + workflow, runs it, and returns 0 on success.
# Globals expected: DOC_PDF, DOC_KEY (report_en|report_ko), TARGET_LANG, FORM_PDF, FIXTURES.

# Approximate paid external calls per node, used by the dry-run plan (LLM / embedding / TTS).
live_estimate() {
  case "$1" in
    translate) echo "LLM 1~4 (문서 길이에 따라 분할)" ;;
    summary) echo "LLM 1 (이미지 포함)" ;;
    analyze) echo "LLM 1 (이미지 1장)" ;;
    slide) echo "LLM 1~2" ;;
    rag) echo "임베딩 1~2 + 질의 2회(임베딩 2 + LLM 2)" ;;
    voice) echo "LLM 1 + TTS 1~N (대본 길이에 따라 청크)" ;;
    fill) echo "LLM 1" ;;
  esac
}

# live_front_end <node> -> sets BUNDLE and WF_BASE graph in $GRAPH (common bundle -> pick_pdf -> extract)
live_front_end() {
  live_make_bundle "live-$1" "$DOC_PDF" || return 1
  BUNDLE="$LIVE_ID"
  GRAPH=$(live_base_graph "$BUNDLE")
}

live_finish() { # live_finish <node> <graph> <output-node-ids...>
  local node="$1" graph="$2"
  shift 2
  local wf
  if live_make_workflow "live-$node" "$graph"; then
    wf="$LIVE_ID"
  else
    mkdir -p "$OUT_DIR/$node/artifacts"
    live_write_result "$OUT_DIR/$node" "$node" failed 0 "워크플로 생성/저장 실패" ""
    return 1
  fi
  live_run_workflow "$node" "$wf" "$@"
}

live_node_translate() {
  live_front_end translate || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_j json_input "$(jq -nc --arg l "$TARGET_LANG" '{value: {targetLanguage: $l}}')" false |
    live_add_node n_t translate '{}' true |
    live_add_edge n_x md n_t md | live_add_edge n_j json n_t settings)
  live_finish translate "$GRAPH" n_t
}

live_node_summary() {
  live_front_end summary || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_s summary '{}' true |
    live_add_edge n_x md n_s md | live_add_edge n_x images n_s images)
  live_finish summary "$GRAPH" n_s
}

live_node_analyze() {
  live_front_end analyze || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_i pick_jpg '{"index":0}' false |
    live_add_node n_a analyze '{}' true |
    live_add_edge n_x images n_i images | live_add_edge n_i image n_a image)
  live_finish analyze "$GRAPH" n_a
}

live_node_slide() {
  live_front_end slide || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_i pick_jpg '{"index":0}' false |
    live_add_node n_s slide '{}' true |
    live_add_edge n_x images n_i images | live_add_edge n_x md n_s md | live_add_edge n_i image n_s image)
  live_finish slide "$GRAPH" n_s
}

live_node_voice() {
  live_front_end voice || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_v voice '{}' true |
    live_add_edge n_x md n_v md | live_add_edge n_x images n_v images)
  live_finish voice "$GRAPH" n_v
}

live_node_rag() {
  live_front_end rag || return 1
  GRAPH=$(echo "$GRAPH" |
    live_add_node n_r rag '{}' true |
    live_add_edge n_x md n_r md)
  local dir="$OUT_DIR/rag"
  live_finish rag "$GRAPH" n_r || return 1

  # The rag node outputs a JSON describing the query API; ask it questions about the document.
  local api_file index_id
  api_file=$(find "$dir/artifacts" -type f -name 'n_r-api-*' | sort | head -n1)
  index_id=$(jq -er .indexId "$api_file" 2>/dev/null) || {
    live_patch_result "$dir" '.status = "failed" | .error = "api 출력에서 indexId 를 찾지 못했습니다"'
    return 1
  }
  local n=0 ok=true q expect answer hit checks='[]' tmp
  tmp=$(mktemp)
  while IFS= read -r item; do
    n=$((n + 1))
    q=$(jq -r .question <<<"$item")
    expect=$(jq -c .expect <<<"$item")
    if live_json_post "$tmp" 200 "$API/rag/$index_id/query" "$(jq -nc --arg q "$q" '{question: $q, topK: 5}')"; then
      cp "$tmp" "$dir/rag-query-$n.json"
      answer=$(jq -r .answer "$tmp")
      hit=$(jq -nr --arg a "$answer" --argjson e "$expect" '[$e[] | select(. as $k | $a | ascii_downcase | contains($k | ascii_downcase))] | length > 0')
      checks=$(jq -c --arg q "$q" --arg a "${answer:0:200}" --argjson h "$hit" '. + [{question: $q, answer: $a, expectedKeywordFound: $h}]' <<<"$checks")
    else
      ok=false
      checks=$(jq -c --arg q "$q" '. + [{question: $q, answer: "(질의 실패)", expectedKeywordFound: false}]' <<<"$checks")
    fi
  done < <(jq -c --arg k "$DOC_KEY" '.[$k][]' "$FIXTURES/questions.json")
  rm -f "$tmp"
  live_patch_result "$dir" '.ragQueries = $c | .preview = ($c | map("Q: " + .question + " → " + .answer + (if .expectedKeywordFound then " [기대 키워드 ✓]" else " [기대 키워드 ✗]" end)) | join(" ‖ ") | .[0:300])' --argjson c "$checks"
  if [[ "$ok" != true ]]; then
    live_patch_result "$dir" '.status = "failed" | .error = "RAG 질의 실패(rag-query-*.json 참고)"'
    return 1
  fi
}

live_node_fill() {
  if [[ ! -f "$FORM_PDF" ]]; then
    mkdir -p "$OUT_DIR/fill/artifacts"
    live_log "fill 건너뜀: 폼 PDF 없음 ($FORM_PDF)"
    live_write_result "$OUT_DIR/fill" fill skipped 0 "폼 PDF($FORM_PDF)가 없어 건너뜀" ""
    return 0
  fi
  live_make_bundle "live-fill" "$FORM_PDF" || return 1
  BUNDLE="$LIVE_ID"
  # Keys deliberately differ from the AcroForm field names (full_name, Email Address, comments,
  # agree_terms, plan, country) so the engine has to match them.
  local values
  values='{"Full Name":"Hong Gildong","E-mail":"hong.gildong@example.com","Additional comments":"Please contact me in the afternoon.","I agree to the terms":true,"Selected plan":"pro","Country":"Korea"}'
  GRAPH=$(jq -nc --arg b "$BUNDLE" '{
      nodes: [
        {id: "n_b", kind: "bundle",   position: {x: 0,   y: 0}, params: {bundleId: $b}, isOutput: false},
        {id: "n_p", kind: "pick_pdf", position: {x: 250, y: 0}, params: {index: 0},      isOutput: false}
      ],
      edges: [{id: "e_b_p", source: "n_b", sourceHandle: "files", target: "n_p", targetHandle: "files"}]}' |
    live_add_node n_j json_input "$(jq -nc --argjson v "$values" '{value: $v}')" false |
    live_add_node n_f fill '{}' true |
    live_add_edge n_p pdf n_f pdf | live_add_edge n_j json n_f values)
  live_finish fill "$GRAPH" n_f
}
