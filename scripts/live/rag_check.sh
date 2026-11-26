#!/usr/bin/env bash
# Live quality check of the rag node and the RAG query API against the real OpenRouter models.
#
#   BASE_URL=<url> LIVE_CONFIRM=1 scripts/live/rag_check.sh [--out live-results] [--label NAME] [--docs en,ko,handbook]
#   LIVE_CONFIRM=1 scripts/live/rag_check.sh --with-verify-stack [...]   # isolated stack dochestra-verify:18080
#
# For every document it runs bundle -> pick_pdf -> extract -> rag through the engine API, then asks a
# fixed question set (facts, cross-language, questions the document cannot answer, topK 1/5/20) and
# scores: expected keywords, [n] citations vs sources, "I don't know" handling, answer language,
# latency. With --with-verify-stack it also dumps the stored chunks from the stack's database to
# judge the chunking (size, split tables/code blocks, Korean text).
#
# Results: <out>/<UTC timestamp>/rag-check/{report.md,results.json,<doc>/...}. API keys are never
# read or printed (the engine inside the stack holds them). Without LIVE_CONFIRM=1 only the plan is
# printed. BASE_URL has no default on purpose (the user's own stack must not be hit by accident).
# Exit status: 0 when the acceptance thresholds are met (facts >= 90 %, unknowns 2/2, citations
# consistent), 1 otherwise, 3 for infrastructure problems.
set -euo pipefail

cd "$(dirname "$0")/../.."
ROOT=$(pwd)
FIXTURES="$ROOT/fixtures/live"

OUT_BASE="live-results"
LABEL="run"
DOCS="en,ko,handbook"
WITH_STACK=false

usage() {
  echo "usage: BASE_URL=<url> LIVE_CONFIRM=1 $0 [--out DIR] [--label NAME] [--docs en,ko,handbook] [--with-verify-stack]" >&2
  exit 2
}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_BASE="${2:-}"; shift 2 ;;
    --label) LABEL="${2:-}"; shift 2 ;;
    --docs) DOCS="${2:-}"; shift 2 ;;
    --with-verify-stack) WITH_STACK=true; shift ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

declare -A DOC_FILE=([en]=report_en.pdf [ko]=report_ko.pdf [handbook]=handbook_en.pdf)
SELECTED=()
IFS=',' read -r -a requested <<<"$DOCS"
for d in "${requested[@]}"; do
  [[ -n "${DOC_FILE[$d]:-}" ]] || { echo "rag_check: unknown doc '$d' (known: en, ko, handbook)" >&2; exit 2; }
  if [[ -f "$FIXTURES/${DOC_FILE[$d]}" ]]; then SELECTED+=("$d"); else echo "rag_check: skipping $d (missing fixtures/live/${DOC_FILE[$d]})" >&2; fi
done
((${#SELECTED[@]} > 0)) || { echo "rag_check: no documents available" >&2; exit 2; }

if [[ "$WITH_STACK" != true && -z "${BASE_URL:-}" ]]; then
  echo "rag_check: BASE_URL is required (or use --with-verify-stack for an isolated stack)" >&2
  exit 2
fi

# ---- gate: plan only unless LIVE_CONFIRM=1 ---------------------------------------------------
if [[ "${LIVE_CONFIRM:-}" != 1 ]]; then
  echo "rag_check: PLAN ONLY (no workflows are created, no paid calls are made)"
  echo "  target   : ${BASE_URL:-<isolated verify stack http://localhost:18080>}"
  echo "  documents: ${SELECTED[*]}"
  echo "  per document: 1 extraction run (1 embedding batch) + its questions (each = 1 embedding + 1 LLM call)"
  echo "  question set: 18 queries in total across the three documents (about 3 + 18 + 18 paid calls)"
  echo "  Set LIVE_CONFIRM=1 to run for real."
  exit 0
fi

# ---- optional isolated stack (same project, port and lock as scripts/verify-stack.sh) -----------
if [[ "$WITH_STACK" == true ]]; then
  avail_gb=$(df --output=avail -BG / | tail -n1 | tr -dc '0-9')
  if ((avail_gb < 10)); then
    echo "rag_check: only ${avail_gb}G free on / (need >= 10G) - aborting before the image build" >&2
    exit 3
  fi
  exec 9>/tmp/dochestra-verify.lock
  flock -w 1800 9
  export COMPOSE_PROJECT_NAME=dochestra-verify
  export WEB_PORT=18080
  export BASE_URL=http://localhost:18080
  if [[ -f .env ]]; then export ENV_FILE=.env; else export ENV_FILE=.env.example; fi
  export COMPOSE_ENV_FILES="$ENV_FILE"
  echo "[rag] isolated stack: project=$COMPOSE_PROJECT_NAME port=$WEB_PORT env=$ENV_FILE"
  stack_down() { docker compose down -v --remove-orphans >/dev/null 2>&1 || true; }
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  if ! docker compose up -d --build --wait; then
    echo "rag_check: the isolated stack failed to start" >&2
    docker compose logs --no-color --tail 40 engine >&2 || true
    stack_down
    exit 3
  fi
fi
API="$BASE_URL/api"

# shellcheck disable=SC1091
. "$ROOT/scripts/live/lib.sh"

TS=$(date -u +%Y%m%dT%H%M%SZ)
OUT_DIR="$ROOT/$OUT_BASE/$TS"
RAG_DIR="$OUT_DIR/rag-check"
mkdir -p "$RAG_DIR/inputs"

finish() {
  local rc=$?
  live_cleanup_resources
  if [[ "$WITH_STACK" == true ]]; then stack_down; fi
  exit "$rc"
}
trap finish EXIT

echo "[rag] BASE_URL=$BASE_URL label=$LABEL out=$RAG_DIR"
curl -fsS "$API/health" >/dev/null || { echo "rag_check: $API/health is not reachable" >&2; exit 3; }

# ---- run bundle -> pick_pdf -> extract -> rag for every document ------------------------------
for d in "${SELECTED[@]}"; do
  src="$FIXTURES/${DOC_FILE[$d]}"
  cp "$src" "$RAG_DIR/inputs/${DOC_FILE[$d]}" # snapshot: other agents may regenerate the fixtures
  doc_dir="$RAG_DIR/$d"
  mkdir -p "$doc_dir"
  echo "[rag] === $d: indexing ${DOC_FILE[$d]}"

  if ! live_make_bundle "rag-check-$d" "$RAG_DIR/inputs/${DOC_FILE[$d]}"; then echo '{"error":"bundle upload failed"}' >"$doc_dir/index.json"; continue; fi
  bundle_id="$LIVE_ID"
  graph=$(live_base_graph "$bundle_id" | live_add_node n_r rag '{}' true | live_add_edge n_x md n_r md)
  if ! live_make_workflow "rag-check-$d" "$graph"; then echo '{"error":"workflow creation failed"}' >"$doc_dir/index.json"; continue; fi
  wf="$LIVE_ID"

  OUT_DIR="$RAG_DIR" NODE_TIMEOUT=900 live_run_workflow "$d" "$wf" n_r n_x || true
  api_json=$(find "$doc_dir/artifacts" -name 'n_r-api-*' -type f 2>/dev/null | head -n1)
  if [[ -z "$api_json" ]] || ! jq -e .indexId "$api_json" >/dev/null 2>&1; then
    jq -n --arg e "$(jq -r '.error // "rag run failed"' "$doc_dir/result.json" 2>/dev/null)" '{error: $e}' >"$doc_dir/index.json"
    echo "[rag] $d: indexing failed: $(jq -r .error "$doc_dir/index.json")" >&2
    continue
  fi
  index_id=$(jq -er .indexId "$api_json")
  jq -c --slurpfile r "$doc_dir/result.json" '. + {runSeconds: $r[0].seconds}' "$api_json" >"$doc_dir/index.json"

  # Stored chunks straight from the stack's database (only possible with --with-verify-stack).
  if [[ "$WITH_STACK" == true ]]; then
    printf "SELECT COALESCE(json_agg(json_build_object('index', chunk_index, 'chars', char_length(content), 'content', content) ORDER BY chunk_index), '[]'::json) FROM rag_chunks WHERE index_id = '%s';\n" "$index_id" |
      docker compose exec -T db sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -At' >"$doc_dir/chunks.json" 2>"$doc_dir/chunks.err" ||
      echo "[rag] $d: could not dump chunks (see $doc_dir/chunks.err)" >&2
  fi
done

# ---- question set + scoring (python3 standard library only) -----------------------------------
cat >"$RAG_DIR/evaluate.py" <<'PYEOF'
import json, re, sys, time, unicodedata, urllib.request, urllib.error
from pathlib import Path

base_url, out_dir, label = sys.argv[1], Path(sys.argv[2]), sys.argv[3]
api = base_url.rstrip("/") + "/api"

# kind: fact | cross (question language differs from the document) | unknown | boundary
# expect_all: every keyword must appear; expect_any: at least one must appear
Q = [
    # ---- English report, English questions
    dict(id="en-revenue", doc="en", kind="fact", lang="en", q="What was Aurora Robotics' total revenue in 2025?", expect_all=["482"]),
    dict(id="en-people", doc="en", kind="fact", lang="en", q="Who is the Chief Executive Officer, and in which city was the company founded?", expect_all=["Min-jun Park", "Busan"]),
    dict(id="en-compare", doc="en", kind="fact", lang="en", q="How many more Atlas picking robots than Nova conveyor shuttles were shipped in 2025?", expect_any=["1,250", "1250"], support=["4,200", "2,950"]),
    dict(id="en-decline", doc="en", kind="fact", lang="en", q="Which region's revenue declined in 2025, and by how much?", expect_all=["Middle East", "10"]),
    # ---- English report, Korean questions (cross-language retrieval)
    dict(id="x-growth", doc="en", kind="cross", lang="ko", q="2025년에 아시아 태평양 지역의 매출 성장률은 몇 퍼센트였나요?", expect_all=["31"]),
    dict(id="x-guidance", doc="en", kind="cross", lang="ko", q="2026년 매출 가이던스는 얼마인가요?", expect_all=["540", "560"]),
    # ---- Korean report, Korean questions
    dict(id="ko-revenue", doc="ko", kind="fact", lang="ko", q="2025년 총매출은 얼마였나요?", expect_any=["4억 8,200만", "4억8,200만", "482"]),
    dict(id="ko-supplier", doc="ko", kind="fact", lang="ko", q="모터 컨트롤러는 어느 나라의 공급사에서 얼마나 조달하나요?", expect_all=["대만", "40"]),
    dict(id="ko-green", doc="ko", kind="fact", lang="ko", q="김해 공장의 재생에너지 비율은 얼마이고 2028년 목표는 무엇인가요?", expect_all=["62", "100"]),
    # ---- questions the documents cannot answer
    dict(id="unk-customer", doc="en", kind="unknown", lang="en", q="What is the name of Aurora Robotics' largest customer?"),
    dict(id="unk-cfo", doc="ko", kind="unknown", lang="ko", q="이 회사의 최고재무책임자(CFO)는 누구인가요?"),
    # ---- handbook: many near-duplicate paragraphs, a CLI table and a JSON block
    dict(id="hb-response", doc="handbook", kind="fact", lang="en", q="What first response time does Aurora Robotics guarantee for remote support?", expect_all=["30 minutes"]),
    dict(id="hb-zone", doc="handbook", kind="fact", lang="en", q="According to the fleet status output for zone B, how many robots are idle?", expect_all=["11"]),
    dict(id="hb-config", doc="handbook", kind="fact", lang="en", q="What charge_threshold is configured for zone B?", expect_all=["0.2"]),
    # ---- topK boundaries on one factual question
    dict(id="k1", doc="en", kind="boundary", lang="en", q="What was Aurora Robotics' total revenue in 2025?", expect_all=["482"], topK=1),
    dict(id="k5", doc="en", kind="boundary", lang="en", q="What was Aurora Robotics' total revenue in 2025?", expect_all=["482"], topK=5),
    dict(id="k20", doc="en", kind="boundary", lang="en", q="What was Aurora Robotics' total revenue in 2025?", expect_all=["482"], topK=20),
    dict(id="hb-k20", doc="handbook", kind="boundary", lang="en", q="What first response time does Aurora Robotics guarantee for remote support?", expect_all=["30 minutes"], topK=20),
]

REFUSAL = re.compile(
    r"(does not (contain|mention|provide|include|specify|state)|do not (contain|mention|provide|include)|doesn't (contain|mention|provide)|"
    r"no (relevant )?(information|mention|details|data)|not (mentioned|specified|stated|provided|available|included|listed|found)|"
    r"(cannot|can't|couldn't|could not|unable to) (find|determine|identify|locate|answer)|isn't (mentioned|specified|stated)|"
    r"없|않습니다|찾을 수|알 수|언급되|확인할 수|제공되지|포함되어 있지)", re.I)


def norm(text):
    return re.sub(r"\s+", " ", unicodedata.normalize("NFKC", text)).lower()


def hangul_ratio(text):
    letters = [c for c in text if c.isalpha()]
    return sum("가" <= c <= "힣" for c in letters) / len(letters) if letters else 0.0


def post(path, body):
    req = urllib.request.Request(api + path, data=json.dumps(body).encode(), headers={"content-type": "application/json"})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=300) as r:
            return r.status, json.load(r), time.time() - t0
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.load(e), time.time() - t0
        except Exception:
            return e.code, {}, time.time() - t0


docs = {}
for d in ("en", "ko", "handbook"):
    p = out_dir / d / "index.json"
    if p.exists():
        docs[d] = json.loads(p.read_text())

results = []
for spec in Q:
    info = docs.get(spec["doc"])
    row = dict(id=spec["id"], doc=spec["doc"], kind=spec["kind"], question=spec["q"], topK=spec.get("topK", 5))
    if not info or "indexId" not in info:
        row.update(status="skipped", pass_=False, note="document was not indexed")
        results.append(row)
        continue
    status, body, secs = post(f"/rag/{info['indexId']}/query", {"question": spec["q"], "topK": row["topK"]})
    row.update(status=status, seconds=round(secs, 1))
    if status != 200:
        row.update(pass_=False, note=f"HTTP {status}: {body.get('error', {}).get('message', '')}"[:200], answer="")
        results.append(row)
        continue
    answer, sources = body["answer"], body["sources"]
    cites = [int(n) for n in re.findall(r"\[(\d+)\]", answer)]
    expect_all, expect_any = spec.get("expect_all", []), spec.get("expect_any", [])
    keyword_ok = all(norm(k) in norm(answer) for k in expect_all) and (not expect_any or any(norm(k) in norm(answer) for k in expect_any))
    wanted = spec.get("support") or (expect_all + expect_any)
    cited_text = " ".join(norm(sources[n - 1]["content"]) for n in cites if 1 <= n <= len(sources))
    row.update(
        answer=answer,
        sourcesCount=len(sources),
        scores=[round(s["score"], 3) for s in sources],
        chunkIndexes=[s["chunkIndex"] for s in sources],
        cites=sorted(set(cites)),
        hasCitation=bool(cites),
        citationsValid=all(1 <= n <= len(sources) for n in cites),
        citationSupports=(any(norm(k) in cited_text for k in wanted) if wanted and cites else None),
        keywordOk=keyword_ok if spec["kind"] != "unknown" else None,
        refusal=bool(REFUSAL.search(answer)) if spec["kind"] == "unknown" else None,
        languageOk=(hangul_ratio(answer) >= 0.2) if spec["lang"] == "ko" else (hangul_ratio(answer) <= 0.05),
        sourcesOrdered=all(sources[i]["score"] >= sources[i + 1]["score"] for i in range(len(sources) - 1)),
        topKOk=len(sources) == min(row["topK"], info.get("chunkCount", 10**9)),
        sourceHasKeyword=(any(norm(k) in " ".join(norm(s["content"]) for s in sources) for k in wanted) if wanted else None),
    )
    if spec["kind"] == "unknown":
        row["pass_"] = row["refusal"] and row["languageOk"]
    else:
        row["pass_"] = keyword_ok and row["languageOk"]
    results.append(row)

# ---- chunk statistics (only when the stack's database could be read)
chunk_stats = {}
for d, info in docs.items():
    p = out_dir / d / "chunks.json"
    if not p.exists() or p.stat().st_size == 0:
        continue
    try:
        chunks = json.loads(p.read_text().strip() or "[]")
    except json.JSONDecodeError:
        continue
    sizes = [c["chars"] for c in chunks]
    texts = [c["content"] for c in chunks]
    plain = [t.replace("\\", "") for t in texts]  # markdown escapes `_` as `\_`
    stats = dict(count=len(chunks), avgChars=round(sum(sizes) / len(sizes)) if sizes else 0, minChars=min(sizes, default=0), maxChars=max(sizes, default=0), over1500=sum(s > 1500 for s in sizes))
    # chunks cut exactly at the size limit (a sign of a hard cut) and small fragments
    stats["cutAtLimit"] = sum(s == 1500 for s in sizes)
    stats["tinyUnder200"] = sum(s < 200 for s in sizes)
    # a table or code block that was cut: header/row pairs must stay together
    pairs = [("ZONE ROBOTS IDLE CHARGING", "B 42 11 5"), ('"zone": "B"', '"charge_threshold"'), ("Asia-Pacific 152", "Middle East 30"), ("아시아 태평양 152", "중동 30")]
    stats["splitBlocks"] = [f"{a} | {b}" for a, b in pairs if any(a in t for t in plain) and not any(a in t and b in t for t in plain)]
    # fragments that begin in the middle of a sentence (lowercase start; headings and table rows do not count)
    stats["startsMidSentence"] = sum(bool(re.match(r"[a-z]", t.lstrip())) for t in texts)
    # chunks that end with a heading line (a heading stranded away from its text)
    stats["endsWithHeading"] = sum(bool(re.search(r"(^|\n)#{1,6} [^\n]+\s*$", t)) for t in texts)
    stats["withHeadingContext"] = sum(t.lstrip().startswith("#") for t in texts)
    stats["korean"] = round(sum(hangul_ratio(t) for t in texts) / len(texts), 2) if texts else 0
    chunk_stats[d] = stats

facts = [r for r in results if r["kind"] in ("fact", "cross") and r["status"] != "skipped"]
unknowns = [r for r in results if r["kind"] == "unknown" and r["status"] != "skipped"]
boundary = [r for r in results if r["kind"] == "boundary" and r["status"] != "skipped"]
answered = [r for r in results if r.get("status") == 200]
summary = dict(
    label=label,
    factAccuracy=round(sum(r["pass_"] for r in facts) / len(facts), 3) if facts else None,
    factsPassed=sum(r["pass_"] for r in facts), factsTotal=len(facts),
    unknownPassed=sum(r["pass_"] for r in unknowns), unknownTotal=len(unknowns),
    boundaryPassed=sum(r["pass_"] for r in boundary), boundaryTotal=len(boundary),
    citationsConsistent=all(r["citationsValid"] for r in answered) and all(r["citationSupports"] is not False for r in answered),
    citationRate=round(sum(r["hasCitation"] for r in answered if r["kind"] != "unknown") / max(1, len([r for r in answered if r["kind"] != "unknown"])), 2),
    sourcesOrdered=all(r["sourcesOrdered"] for r in answered), topKOk=all(r["topKOk"] for r in answered),
    avgSeconds=round(sum(r["seconds"] for r in answered) / len(answered), 1) if answered else None,
    skipped=sum(r["status"] == "skipped" for r in results),
    chunkStats=chunk_stats,
    indexes={d: {k: v for k, v in i.items() if k in ("chunkCount", "runSeconds", "error")} for d, i in docs.items()},
)
accept = bool(facts) and summary["factAccuracy"] >= 0.9 and summary["unknownPassed"] == summary["unknownTotal"] and summary["citationsConsistent"] and summary["skipped"] == 0
summary["accepted"] = accept
(out_dir / "results.json").write_text(json.dumps(dict(summary=summary, results=results), ensure_ascii=False, indent=2))

def cell(t, n=110):
    return re.sub(r"\s+", " ", str(t)).replace("|", "/")[:n]

lines = [f"# RAG 실사용 점검 ({label})", "", f"- 사실·교차언어 질문: **{summary['factsPassed']}/{summary['factsTotal']}** ({(summary['factAccuracy'] or 0) * 100:.0f}%)",
         f"- 문서 밖 질문 '모름' 처리: **{summary['unknownPassed']}/{summary['unknownTotal']}**", f"- topK 경계 질문: {summary['boundaryPassed']}/{summary['boundaryTotal']}",
         f"- 근거 [n] 일관성: {'✓' if summary['citationsConsistent'] else '✗'} (사실 질문 중 인용 포함 {summary['citationRate'] * 100:.0f}%), 점수 내림차순 {'✓' if summary['sourcesOrdered'] else '✗'}, topK 개수 {'✓' if summary['topKOk'] else '✗'}",
         f"- 평균 질의 지연: {summary['avgSeconds']}s", f"- 판정: **{'통과' if accept else '미달'}**", "", "## 색인", "", "| 문서 | 청크 수 | 평균/최소/최대 글자 | 1500자 초과 | 한계에서 잘림 | 200자 미만 | 문장 중간 시작 | 제목 고아 | 제목 맥락 포함 | 잘린 표·코드 | 오류 |", "| :-- | --: | :-- | --: | --: | --: | --: | --: | --: | :-- | :-- |"]
for d, info in docs.items():
    st = chunk_stats.get(d, {})
    lines.append(f"| {d} | {info.get('chunkCount', '-')} | {st.get('avgChars', '-')}/{st.get('minChars', '-')}/{st.get('maxChars', '-')} | {st.get('over1500', '-')} | {st.get('cutAtLimit', '-')} | {st.get('tinyUnder200', '-')} | {st.get('startsMidSentence', '-')} | {st.get('endsWithHeading', '-')} | {st.get('withHeadingContext', '-')} | {cell(', '.join(st.get('splitBlocks', [])) or ('없음' if st else '(DB 미조회)'), 80)} | {info.get('error', '')} |")
lines += ["", "## 질문별 결과", "", "| id | 종류 | topK | 판정 | 키워드 | 인용 | 언어 | 근거 점수 | 지연 | 답변 |", "| :-- | :-- | --: | :-: | :-: | :-- | :-: | :-- | --: | :-- |"]
mark = lambda v: "-" if v is None else ("✓" if v else "✗")
for r in results:
    if r["status"] != 200:
        lines.append(f"| {r['id']} | {r['kind']} | {r['topK']} | ✗ | | | | | | {cell(r.get('note', r['status']))} |")
        continue
    kw = mark(r["keywordOk"]) if r["kind"] != "unknown" else f"모름 {mark(r['refusal'])}"
    cites = ("없음" if not r["cites"] else ",".join(map(str, r["cites"]))) + ("" if r["citationsValid"] else " (범위 밖!)") + ("" if r["citationSupports"] is not False else " (근거 불일치)")
    lines.append(f"| {r['id']} | {r['kind']} | {r['topK']} | {mark(r['pass_'])} | {kw} | {cites} | {mark(r['languageOk'])} | {','.join(map(str, r['scores'][:5]))} | {r['seconds']}s | {cell(r['answer'])} |")
lines += ["", "질문·전체 답변·근거 원문은 `results.json`, 저장된 청크는 `<doc>/chunks.json`(스택 모드)."]
(out_dir / "report.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
sys.exit(0 if accept else 1)
PYEOF

set +e
python3 "$RAG_DIR/evaluate.py" "$BASE_URL" "$RAG_DIR" "$LABEL"
rc=$?
set -e
echo "[rag] report: $RAG_DIR/report.md (exit $rc)"
exit "$rc"
