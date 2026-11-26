#!/usr/bin/env bash
# Per-node scenarios for scripts/live-nodes.sh (sourced after lib.sh).
# Each live_node_<name> creates its own bundle + workflow, runs it, and returns 0 on success.
# Globals expected: DOC_PDF, DOC_KEY (report_en|report_ko), TARGET_LANG, FIXTURES.

# Approximate paid external calls per node, used by the dry-run plan (LLM / embedding / TTS).
live_estimate() {
  case "$1" in
    translate) echo "LLM 1~4 (문서 길이에 따라 분할)" ;;
    summary) echo "LLM 1 (이미지 포함)" ;;
    analyze) echo "LLM 1 (이미지 1장)" ;;
    slide) echo "LLM 1~2" ;;
    rag) echo "임베딩 1~2 + 질의 2회(임베딩 2 + LLM 2)" ;;
    voice) echo "LLM 1 + TTS 1~N (대본 길이에 따라 청크)" ;;
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

# live_translate_case <case> <pdf> <settings-json> [glossary-json]
# One translate scenario on its own document; downloads the source (extract md) and the
# translation, then runs scripts/live/translate_check.py and stores the checklist in result.json.
live_translate_case() {
  local case="$1" pdf="$2" settings="$3" glossary="${4:-}"
  local node="translate-$case" dir="$OUT_DIR/translate-$case"
  mkdir -p "$dir"
  live_make_bundle "live-$node" "$pdf" || return 1
  local graph
  graph=$(live_base_graph "$LIVE_ID" |
    live_add_node n_j json_input "$(jq -nc --argjson v "$settings" '{value: $v}')" false |
    live_add_node n_t translate '{}' true |
    live_add_edge n_x md n_t md | live_add_edge n_j json n_t settings)
  # n_x md is downloaded too (it is the source the translation is checked against).
  live_finish "$node" "$graph" n_t n_x || return 1
  local src="$dir/artifacts/n_x-md-0-md.md" dst="$dir/artifacts/n_t-md-0-md.md"
  if [[ -f "$src" && -f "$dst" ]]; then
    python3 "$ROOT/scripts/live/translate_check.py" "$src" "$dst" --glossary "${glossary:-{\}}" >"$dir/checks.json" || true
    live_patch_result "$dir" '.checks = $c[0].checks | .checksOk = $c[0].ok | .sourceChars = $sc | .translatedChars = $tc' \
      --slurpfile c "$dir/checks.json" --argjson sc "$(wc -m <"$src")" --argjson tc "$(wc -m <"$dst")" || true
  fi
}

live_node_translate() {
  # (1) Korean -> English, (2) 20-page English handbook -> Korean, (3) glossary + tone + instructions.
  # Settings carry glossary/tone/instructions in (3) only. Cases are independent: one failing does not stop the rest.
  local rc=0
  live_translate_case ko "$FIXTURES/report_ko.pdf" '{"targetLanguage":"en"}' || rc=1
  live_translate_case long "$FIXTURES/handbook_en.pdf" '{"targetLanguage":"ko"}' || rc=1
  local g='{"Aurora Robotics":"Aurora Robotics","operating margin":"영업이익률","fleet":"플릿"}'
  live_translate_case settings "$FIXTURES/report_en.pdf" "$(jq -nc --argjson g "$g" \
    '{targetLanguage:"ko", glossary:$g, tone:"격식 있는 보고서체(합니다체)", instructions:"숫자와 단위는 원문 그대로 유지하고 고유명사는 번역하지 않는다."}')" "$g" || rc=1
  return "$rc"
}

# live_summary_case <case> <doc-key> <pdf> <with-images true|false>
live_summary_case() {
  local case="$1" key="$2" pdf="$3" with_images="$4"
  local node="summary-$case" dir="$OUT_DIR/summary-$case"
  mkdir -p "$dir"
  live_make_bundle "live-$node" "$pdf" || return 1
  local graph
  graph=$(live_base_graph "$LIVE_ID" | live_add_node n_s summary '{}' true | live_add_edge n_x md n_s md)
  [[ "$with_images" == true ]] && graph=$(echo "$graph" | live_add_edge n_x images n_s images)
  live_finish "$node" "$graph" n_s n_x || return 1
  local src="$dir/artifacts/n_x-md-0-md.md" dst="$dir/artifacts/n_s-md-0-md.md"
  if [[ -f "$src" && -f "$dst" ]]; then
    local n_images=0
    [[ "$with_images" == true ]] && n_images=$(jq '[.nodes[] | select(.nodeId == "n_x") | .outputs[] | select(.socketId == "images") | .files | length] | add // 0' "$dir/run.json")
    python3 "$ROOT/scripts/live/summary_check.py" "$src" "$dst" --doc "$key" --images "$n_images" \
      --facts "$FIXTURES/facts.json" >"$dir/checks.json" || true
    live_patch_result "$dir" '.checks = $c[0].checks | .checksOk = $c[0].ok | .sourceChars = $sc | .translatedChars = $tc | .images = $ni' \
      --slurpfile c "$dir/checks.json" --argjson sc "$(wc -m <"$src")" --argjson tc "$(wc -m <"$dst")" --argjson ni "$n_images" || true
  fi
}

# live_case_enabled <case>: true when LIVE_CASES is unset or lists the case (comma separated)
live_case_enabled() { [[ -z "${LIVE_CASES:-}" || ",$LIVE_CASES," == *",$1,"* ]]; }

live_node_summary() {
  # (1) 20-page handbook with figures, (2) Korean report, (3) markdown only (images are optional, D-4),
  # (4) 28 figures: only 20 go to the model and the summary ends with the exclusion note.
  local rc=0
  live_case_enabled long && { live_summary_case long handbook_en "$FIXTURES/handbook_en.pdf" true || rc=1; }
  live_case_enabled ko && { live_summary_case ko report_ko "$FIXTURES/report_ko.pdf" true || rc=1; }
  live_case_enabled noimg && { live_summary_case noimg report_en "$FIXTURES/report_en.pdf" false || rc=1; }
  live_case_enabled many && { live_summary_case many gallery_en "$FIXTURES/gallery_en.pdf" true || rc=1; }
  return "$rc"
}

# live_analyze_case <case> <pdf> [expect-failure-pattern]
# One image per PDF: bundle -> pick_pdf -> extract -> pick_jpg(0) -> analyze. The image the model
# saw (n_i) and its analysis (n_a) are downloaded; analyze_check.py compares it with vision_facts.json.
live_analyze_case() {
  local case="$1" pdf="$2" expect_fail="${3:-}"
  local node="analyze-$case" dir="$OUT_DIR/analyze-$case"
  mkdir -p "$dir"
  live_make_bundle "live-$node" "$pdf" || return 1
  local graph
  graph=$(live_base_graph "$LIVE_ID" |
    live_add_node n_i pick_jpg '{"index":0}' false |
    live_add_node n_a analyze '{}' true |
    live_add_edge n_x images n_i images | live_add_edge n_i image n_a image)
  if ! live_finish "$node" "$graph" n_a n_i; then
    if [[ -n "$expect_fail" ]]; then
      local err
      err=$(jq -r .error "$dir/result.json")
      if [[ "$err" == *"$expect_fail"* ]]; then
        live_patch_result "$dir" '.status = "expected-failure"'
        return 0
      fi
      # failed somewhere else (e.g. the extractor): not the path under test
      live_patch_result "$dir" '.status = "skipped" | .error = ("대상 경로에 도달하지 못함: " + .error)'
      return 0
    fi
    return 1
  fi
  local img dst
  img=$(find "$dir/artifacts" -name 'n_i-image-*' | head -n1)
  dst="$dir/artifacts/n_a-md-0-md.md"
  if [[ -f "$dst" ]]; then
    python3 "$ROOT/scripts/live/analyze_check.py" "$dst" --case "$case" --facts "$FIXTURES/vision_facts.json" >"$dir/checks.json" || true
    live_patch_result "$dir" '.checks = $c[0].checks | .checksOk = $c[0].ok | .score = $c[0].score | .sourceChars = 0 | .translatedChars = $tc | .imageBytes = $ib' \
      --slurpfile c "$dir/checks.json" --argjson tc "$(wc -m <"$dst")" --argjson ib "$([[ -n "$img" ]] && stat -c %s "$img" || echo 0)" || true
  fi
}

live_node_analyze() {
  # (1) chart (2) table image (3) Korean text poster (4) photo-like scene (5) tiny image (<200px)
  # (6) the >5 MiB picture error path cannot be produced through the extractor (Docling rejects
  #     decoded pages above 20 MiB and its q90 JPEG tops out near 5 MB), so it is covered by the mock test
  #     `analyze_sends_the_image_and_enforces_the_size_limit` only.
  local rc=0
  live_case_enabled chart && { live_analyze_case chart "$FIXTURES/vision_chart.pdf" || rc=1; }
  live_case_enabled table && { live_analyze_case table "$FIXTURES/vision_table.pdf" || rc=1; }
  live_case_enabled text && { live_analyze_case text "$FIXTURES/vision_text.pdf" || rc=1; }
  live_case_enabled photo && { live_analyze_case photo "$FIXTURES/vision_photo.pdf" || rc=1; }
  live_case_enabled small && { live_analyze_case small "$FIXTURES/vision_small.pdf" || rc=1; }
  return "$rc"
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
