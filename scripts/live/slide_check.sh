#!/usr/bin/env bash
# Live quality check of the slide node (LLM deck outline -> rendered 16:9 PDF) against the real OpenRouter models.
#
#   BASE_URL=<url> LIVE_CONFIRM=1 scripts/live/slide_check.sh [--out live-results] [--label NAME] [--docs en,ko,handbook,tiny] [--tiny-index N]
#   LIVE_CONFIRM=1 scripts/live/slide_check.sh --with-verify-stack [...]   # isolated stack dochestra-verify:18080
#
# Scenarios (every one runs bundle -> pick_pdf -> extract -> pick_jpg -> slide through the engine API):
#   en       report_en.pdf   short English report, representative image = extracted image 0
#   ko       report_ko.pdf   Korean report -> the deck must be Korean
#   handbook handbook_en.pdf 20 pages of near-duplicate chapters: deck size / balance
#   tiny     gallery_en.pdf  a degenerate representative image (default extracted image 3: 144 x 26 px, ratio 5.5;
#                            image 10 is an 867 x 23 strip) - the cover / closing-slide layout must keep the ratio
# With --with-verify-stack the deck outline the node saved (/data/runs/<run>/n_s/_work/slides.json) is read out of the
# isolated stack's engine container, and the engine log is searched for outline retries ("retrying once").
#
# Checks: outline parsed (and retries), page count (1 + N + 1 with an image), page numbers, language of the deck,
# text outside the page margins or in the footer zone, truncation ("…"), text that fell to the minimum font size,
# characters the font could not draw (compared with the outline), image aspect ratio / bounds / overlap with the
# cover text / upscaling factor, key facts, numbers that are not in the source, PDF size. Pages are rendered to PNG
# (pypdfium2 in a one-off `uv run --with` environment, nothing is installed on the host) next to report.md.
#
# Results: <out>/<UTC timestamp>/slide-check/{report.md,results.json,<doc>/{png,sheet.png,slides.json,...}}. API keys are
# never read or printed (the engine inside the stack holds them). Without LIVE_CONFIRM=1 only the plan is printed.
# BASE_URL has no default on purpose (the user's own stack must not be hit by accident).
# Exit status: 0 when every selected scenario passes, 1 otherwise, 3 for infrastructure problems.
set -euo pipefail

cd "$(dirname "$0")/../.."
ROOT=$(pwd)
FIXTURES="$ROOT/fixtures/live"

OUT_BASE="live-results"
LABEL="run"
DOCS="en,ko,handbook,tiny"
TINY_INDEX=3
WITH_STACK=false

usage() {
  echo "usage: BASE_URL=<url> LIVE_CONFIRM=1 $0 [--out DIR] [--label NAME] [--docs en,ko,handbook,tiny] [--tiny-index N] [--with-verify-stack]" >&2
  exit 2
}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_BASE="${2:-}"; shift 2 ;;
    --label) LABEL="${2:-}"; shift 2 ;;
    --docs) DOCS="${2:-}"; shift 2 ;;
    --tiny-index) TINY_INDEX="${2:-}"; shift 2 ;;
    --with-verify-stack) WITH_STACK=true; shift ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done
[[ "$TINY_INDEX" =~ ^[0-9]+$ ]] || { echo "slide_check: --tiny-index must be a number" >&2; exit 2; }

declare -A DOC_FILE=([en]=report_en.pdf [ko]=report_ko.pdf [handbook]=handbook_en.pdf [tiny]=gallery_en.pdf)
declare -A DOC_IMAGE=([en]=0 [ko]=0 [handbook]=0 [tiny]="$TINY_INDEX")
SELECTED=()
IFS=',' read -r -a requested <<<"$DOCS"
for d in "${requested[@]}"; do
  [[ -n "${DOC_FILE[$d]:-}" ]] || { echo "slide_check: unknown scenario '$d' (known: en, ko, handbook, tiny)" >&2; exit 2; }
  if [[ -f "$FIXTURES/${DOC_FILE[$d]}" ]]; then SELECTED+=("$d"); else echo "slide_check: skipping $d (missing fixtures/live/${DOC_FILE[$d]})" >&2; fi
done
((${#SELECTED[@]} > 0)) || { echo "slide_check: no scenarios available" >&2; exit 2; }

if [[ "$WITH_STACK" != true && -z "${BASE_URL:-}" ]]; then
  echo "slide_check: BASE_URL is required (or use --with-verify-stack for an isolated stack)" >&2
  exit 2
fi

# ---- gate: plan only unless LIVE_CONFIRM=1 ---------------------------------------------------
if [[ "${LIVE_CONFIRM:-}" != 1 ]]; then
  echo "slide_check: PLAN ONLY (no workflows are created, no paid calls are made)"
  echo "  target   : ${BASE_URL:-<isolated verify stack http://localhost:18080>}"
  echo "  scenarios: ${SELECTED[*]}"
  echo "  per scenario: 1 extraction run + 1 LLM call for the outline (a second one when the first answer cannot be parsed)"
  echo "  Set LIVE_CONFIRM=1 to run for real."
  exit 0
fi

command -v uv >/dev/null || { echo "slide_check: uv is required for the PDF analysis (one-off environment)" >&2; exit 3; }
command -v jq >/dev/null || { echo "slide_check: jq is required" >&2; exit 3; }

# ---- optional isolated stack (same project, port and lock as scripts/verify-stack.sh) -----------
if [[ "$WITH_STACK" == true ]]; then
  avail_gb=$(df --output=avail -BG / | tail -n1 | tr -dc '0-9')
  if ((avail_gb < 10)); then
    echo "slide_check: only ${avail_gb}G free on / (need >= 10G) - aborting before the image build" >&2
    exit 3
  fi
  exec 9>/tmp/dochestra-verify.lock
  flock -w 1800 9
  export COMPOSE_PROJECT_NAME=dochestra-verify
  export WEB_PORT=18080
  export BASE_URL=http://localhost:18080
  if [[ -f .env ]]; then export ENV_FILE=.env; else export ENV_FILE=.env.example; fi
  export COMPOSE_ENV_FILES="$ENV_FILE"
  echo "[slide] isolated stack: project=$COMPOSE_PROJECT_NAME port=$WEB_PORT env=$ENV_FILE"
  stack_down() { docker compose down -v --remove-orphans >/dev/null 2>&1 || true; }
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  if ! docker compose up -d --build --wait; then
    echo "slide_check: the isolated stack failed to start" >&2
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
SLIDE_DIR="$OUT_DIR/slide-check"
mkdir -p "$SLIDE_DIR/inputs"

finish() {
  local rc=$?
  live_cleanup_resources
  if [[ "$WITH_STACK" == true ]]; then stack_down; fi
  exit "$rc"
}
trap finish EXIT

echo "[slide] BASE_URL=$BASE_URL label=$LABEL out=$SLIDE_DIR"
curl -fsS "$API/health" >/dev/null || { echo "slide_check: $API/health is not reachable" >&2; exit 3; }

retry_lines() { docker compose logs --no-color engine 2>/dev/null | grep -c "slide outline rejected" || true; }

# ---- run bundle -> pick_pdf -> extract -> pick_jpg -> slide for every scenario ----------------
for d in "${SELECTED[@]}"; do
  src="$FIXTURES/${DOC_FILE[$d]}"
  cp "$src" "$SLIDE_DIR/inputs/${DOC_FILE[$d]}" # snapshot: other agents may regenerate the fixtures
  doc_dir="$SLIDE_DIR/$d"
  mkdir -p "$doc_dir"
  image_index="${DOC_IMAGE[$d]}"
  echo "[slide] === $d: deck for ${DOC_FILE[$d]} (representative image: extracted image $image_index)"
  echo "$image_index" >"$doc_dir/image-index.txt"

  if ! live_make_bundle "slide-check-$d" "$SLIDE_DIR/inputs/${DOC_FILE[$d]}"; then continue; fi
  bundle_id="$LIVE_ID"
  graph=$(live_base_graph "$bundle_id" |
    live_add_node n_i pick_jpg "{\"index\":$image_index}" false |
    live_add_node n_s slide '{}' true |
    live_add_edge n_x md n_s md | live_add_edge n_x images n_i images | live_add_edge n_i image n_s image)
  if ! live_make_workflow "slide-check-$d" "$graph"; then continue; fi
  wf="$LIVE_ID"

  retries_before=0
  [[ "$WITH_STACK" == true ]] && retries_before=$(retry_lines)
  OUT_DIR="$SLIDE_DIR" NODE_TIMEOUT=900 live_run_workflow "$d" "$wf" n_s n_x || true

  if [[ "$WITH_STACK" == true ]]; then
    echo $(($(retry_lines) - retries_before)) >"$doc_dir/outline-retries.txt"
    # The outline the node saved lives only inside the engine's data volume (isolated project only).
    run_id=$(jq -r '.id // empty' "$doc_dir/run.json" 2>/dev/null || true)
    if [[ -n "$run_id" ]]; then
      docker compose exec -T engine cat "/data/runs/$run_id/n_s/_work/slides.json" >"$doc_dir/slides.json" 2>"$doc_dir/slides.err" ||
        echo "[slide] $d: could not read slides.json (see $doc_dir/slides.err)" >&2
    fi
  fi
done

# ---- analysis (python3 + a one-off uv environment for PDF rendering) ----------------------------
cat >"$SLIDE_DIR/analyze.py" <<'PYEOF'
import json, math, re, sys, unicodedata
from collections import Counter
from pathlib import Path

import pypdfium2 as pdfium
import pypdfium2.raw as raw
from PIL import Image

out_dir, label = Path(sys.argv[1]), sys.argv[2]
selected = sys.argv[3].split(",")

PAGE_W, PAGE_H, MARGIN, FOOTER_TOP = 720.0, 405.0, 48.0, 40.0
MIN_BODY_PT = 12.0  # the renderer's smallest bullet size
SLACK = 1.5

FACTS = {
    "en": [["482"], ["Min-jun Park"], ["31"], ["Asia-Pacific"], ["Busan"], ["2012"], ["540"], ["Taiwan"], ["Gimhae"]],
    "ko": [["482", "4억 8,200만", "4억 8200만"], ["박민준"], ["31"], ["아시아"], ["부산"], ["2012"], ["대만"], ["김해"], ["62"]],
    "handbook": [["Aurora Robotics"], ["0.3"], ["30 minutes", "30분"], ["20%", "20 percent", "20퍼센트"], ["24 hours", "24시간"]],
    "tiny": [["Aurora Robotics"], ["28"], ["Gimhae", "김해"]],
}
DOC_LANG = {"en": "en", "ko": "ko", "handbook": "en", "tiny": "en"}


def hangul_ratio(text):
    letters = [c for c in text if c.isalpha()]
    return sum("가" <= c <= "힣" for c in letters) / len(letters) if letters else 0.0


def squeeze(text):
    return re.sub(r"\s", "", unicodedata.normalize("NFKC", text)).lower()


def numbers(text):
    return {m.replace(",", "") for m in re.findall(r"\d[\d,]*(?:\.\d+)?", unicodedata.normalize("NFKC", text))}


def deck_strings(deck):
    out = [deck.get("title", ""), deck.get("subtitle") or ""]
    for s in deck.get("slides", []):
        out.append(s.get("title", ""))
        out.extend(s.get("bullets", []))
    return [s for s in out if s]


def page_lines(page):
    """[(text, size, (l, b, r, t))] for every text object of the page."""
    tp = page.get_textpage()
    lines = []
    for obj in page.get_objects(filter=[raw.FPDF_PAGEOBJ_TEXT]):
        obj.textpage = tp
        lines.append((obj.extract().strip(), round(obj.get_font_size(), 1), obj.get_bounds()))
    return [x for x in lines if x[0]]


def analyze(d):
    doc_dir = out_dir / d
    r = dict(doc=d)
    rr = json.loads((doc_dir / "result.json").read_text()) if (doc_dir / "result.json").exists() else {}
    r.update(status=rr.get("status", "no result"), seconds=rr.get("seconds"), error=rr.get("error", ""), pipeline=rr.get("pipeline", ""))
    pdfs = sorted((doc_dir / "artifacts").glob("n_s-pdf-*"))
    mds = sorted((doc_dir / "artifacts").glob("n_x-md-*"))
    idx = int((doc_dir / "image-index.txt").read_text().strip() or 0) if (doc_dir / "image-index.txt").exists() else 0
    imgs = sorted((doc_dir / "artifacts").glob(f"n_x-images-{idx}-*"))
    r["imageIndex"] = idx
    retries_file = doc_dir / "outline-retries.txt"
    r["outlineRetries"] = int(retries_file.read_text().strip()) if retries_file.exists() else None
    deck = json.loads((doc_dir / "slides.json").read_text()) if (doc_dir / "slides.json").exists() and (doc_dir / "slides.json").stat().st_size else None
    r["deck"] = None
    r["pdf"] = None
    checks = {"실행 성공": r["status"] == "succeeded", "구성(slides.json) 확보": deck is not None, "PDF 산출": bool(pdfs)}

    source = mds[0].read_text(errors="replace") if mds else ""
    image_px = None
    if imgs:
        with Image.open(imgs[0]) as im:
            image_px = im.size
        r["image"] = dict(file=imgs[0].name, width=image_px[0], height=image_px[1], ratio=round(image_px[0] / image_px[1], 2))

    if deck:
        texts = deck_strings(deck)
        joined = "\n".join(texts)
        slides = deck.get("slides", [])
        per_slide_ratio = [round(hangul_ratio(s.get("title", "") + " ".join(s.get("bullets", []))), 2) for s in slides]
        want_ko = DOC_LANG[d] == "ko"
        lang_ok = all((x >= 0.4) if want_ko else (x <= 0.15) for x in per_slide_ratio) and ((hangul_ratio(deck.get("title", "")) >= 0.4) == want_ko or not deck.get("title"))
        facts = FACTS[d]
        sq = squeeze(joined)
        hits = [any(squeeze(a) in sq for a in alts) for alts in facts]
        src_nums = numbers(source)
        unverified = sorted(n for n in numbers(joined) if n not in src_nums and len(n.replace(".", "")) >= 2)
        bullets = [len(s.get("bullets", [])) for s in slides]
        longest = max((len(t) for t in texts), default=0)
        r["deck"] = dict(
            title=deck.get("title"), subtitle=deck.get("subtitle"), slides=len(slides),
            bulletsMin=min(bullets, default=0), bulletsMax=max(bullets, default=0), bulletsTotal=sum(bullets),
            longestText=longest, textsOver100=sum(1 for t in texts if len(t) > 100),
            hangulRatioPerSlide=per_slide_ratio, languageOk=lang_ok,
            factsTotal=len(facts), factsHit=sum(hits), factsMissed=[a[0] for a, h in zip(facts, hits) if not h],
            numbersNotInSource=unverified,
        )
        checks["언어가 원문을 따름"] = lang_ok
        checks["핵심 사실 ≥80%"] = sum(hits) >= math.ceil(0.8 * len(facts))

    if pdfs:
        pdf = pdfium.PdfDocument(str(pdfs[0]))
        png_dir = doc_dir / "png"
        png_dir.mkdir(exist_ok=True)
        pages, overflow, footer_zone, ellipsis, min_font, sizes_seen, numbering = [], [], [], [], [], Counter(), []
        all_text = []
        cover_text_boxes = []
        for i in range(len(pdf)):
            page = pdf[i]
            page.render(scale=1.4).to_pil().convert("RGB").save(png_dir / f"page-{i + 1:02d}.png")
            lines = page_lines(page)
            footer = [x for x in lines if re.fullmatch(r"\d+ / \d+", x[0])]
            body = [x for x in lines if x not in footer]
            numbering.append(footer[0][0] if footer else None)
            for text, size, (l, b, rt, t) in body:
                sizes_seen[size] += 1
                all_text.append(text)
                if rt > PAGE_W - MARGIN + SLACK or l < MARGIN - SLACK or t > PAGE_H or b < 0:
                    overflow.append((i + 1, text[:40]))
                elif b < FOOTER_TOP and len(pdf) > 0:
                    footer_zone.append((i + 1, text[:40]))
                if text.endswith("…") or text.endswith("..."):
                    ellipsis.append((i + 1, text[:40]))
                if size <= MIN_BODY_PT and i not in (0,):
                    min_font.append((i + 1, text[:30]))
            page_imgs = [(o.get_bounds(), o.get_px_size()) for o in page.get_objects(filter=[raw.FPDF_PAGEOBJ_IMAGE])]
            pages.append(dict(page=i + 1, lines=len(body), images=page_imgs, bodyLines=[(t, s, b) for t, s, b in body]))
        # contact sheet
        thumbs = [Image.open(png_dir / f"page-{i + 1:02d}.png") for i in range(len(pdf))]
        cols = 4
        tw, th = 360, int(360 * PAGE_H / PAGE_W)
        rows = math.ceil(len(thumbs) / cols)
        sheet = Image.new("RGB", (cols * (tw + 6) + 6, rows * (th + 6) + 6), (200, 200, 205))
        for k, im in enumerate(thumbs):
            sheet.paste(im.resize((tw, th)), (6 + (k % cols) * (tw + 6), 6 + (k // cols) * (th + 6)))
        sheet.save(doc_dir / "sheet.png")

        n_pages = len(pdf)
        expected_images = 1 if image_px else 0
        expected = 1 + (len(deck["slides"]) if deck else 0) + expected_images
        image_boxes = [(p["page"], box, px) for p in pages for box, px in p["images"]]
        image_info = []
        aspect_ok, in_bounds, no_overlap = True, True, True
        for pno, (l, b, rt, t), (pw, ph) in image_boxes:
            w, h = rt - l, t - b
            ratio_err = abs((w / h) / (pw / ph) - 1) if h > 0 else 1
            upscale = w / pw
            image_info.append(dict(page=pno, bbox=[round(v, 1) for v in (l, b, rt, t)], widthPt=round(w, 1), heightPt=round(h, 1), sourcePx=[pw, ph], ratioErrorPct=round(ratio_err * 100, 2), upscale=round(upscale, 2)))
            aspect_ok &= ratio_err < 0.01
            in_bounds &= l >= -0.5 and b >= -0.5 and rt <= PAGE_W + 0.5 and t <= PAGE_H + 0.5
            if pno == 1:  # cover: the title / subtitle must not run under the picture
                for text, size, (tl, tb, tr, tt) in pages[0]["bodyLines"]:
                    if tl < rt and tr > l and tb < t and tt > b:
                        no_overlap = False
        # characters of the outline that the PDF does not contain (font gaps) - on pages without truncation
        pdf_chars = Counter(c for c in "".join(all_text) if not c.isspace())
        deck_chars = Counter(c for c in "".join(deck_strings(deck)) if not c.isspace()) if deck else Counter()
        missing = deck_chars - pdf_chars
        truncated = bool(ellipsis)
        missing_chars = sorted(missing) if not truncated else []
        replacement = [c for c in pdf_chars if c in "�□▯"]
        r["pdf"] = dict(
            bytes=pdfs[0].stat().st_size, pages=n_pages, expectedPages=expected, pageSize=[round(v, 1) for v in pdf[0].get_size()],
            pageNumbers=numbering, overflowLines=overflow, footerZoneLines=footer_zone, ellipsisLines=ellipsis,
            minFontLines=min_font, fontSizes={str(k): v for k, v in sorted(sizes_seen.items())},
            images=image_info, missingChars=missing_chars, replacementChars=replacement,
        )
        checks.update({
            f"페이지 수 = 1+N{'+1' if image_px else ''}": n_pages == expected,
            "페이지 번호 'n / N'": numbering == [f"{k} / {n_pages}" for k in range(1, n_pages + 1)],
            "텍스트가 여백 밖으로 나가지 않음": not overflow and not footer_zone,
            "잘림(…) 없음": not ellipsis,
            "최소 글자 크기(12pt)로 줄어든 줄 없음": not min_font,
            "글리프 누락·깨짐 없음": not missing_chars and not replacement,
            "이미지 비율 유지(±1%)": aspect_ok,
            "이미지가 페이지 안": in_bounds,
            "표지 글자와 이미지가 겹치지 않음": no_overlap,
        })
        if image_px:
            checks["이미지 2회 배치(표지·마지막)"] = len(image_boxes) == 2
            checks["이미지 확대 ≤2배"] = all(i["upscale"] <= 2.01 for i in image_info)
    r["checks"] = checks
    r["pass"] = all(checks.values())
    return r


results = [analyze(d) for d in selected]
all_ok = all(r["pass"] for r in results)
(out_dir / "results.json").write_text(json.dumps(dict(label=label, accepted=all_ok, docs=results), ensure_ascii=False, indent=1))

L = [f"# slide 품질 점검 — {label}", "", f"- 판정: **{'통과' if all_ok else '실패'}**", ""]
L += ["## 요약", "", "| 시나리오 | 실행 | 소요 | 재시도 | 슬라이드(내용) | 페이지 | PDF 크기 | 언어 | 사실 | 판정 |", "| :-- | :-: | --: | --: | --: | --: | --: | :-: | :-: | :-: |"]
for r in results:
    dk, pf = r["deck"], r["pdf"]
    L.append("| {} | {} | {}s | {} | {} | {} | {} | {} | {} | {} |".format(
        r["doc"], r["status"], r["seconds"], r["outlineRetries"] if r["outlineRetries"] is not None else "-",
        dk["slides"] if dk else "-", pf["pages"] if pf else "-", f"{pf['bytes'] / 1e6:.2f}MB" if pf else "-",
        "✓" if dk and dk["languageOk"] else "✗", f"{dk['factsHit']}/{dk['factsTotal']}" if dk else "-", "✓" if r["pass"] else "✗"))
L += ["", "## 시나리오별 점검", ""]
for r in results:
    L += [f"### {r['doc']}", ""]
    if r["error"]:
        L.append(f"- 오류: {r['error']}")
    L.append(f"- 파이프라인: {r['pipeline']}")
    for name, ok in r["checks"].items():
        L.append(f"- {'✓' if ok else '✗'} {name}")
    dk, pf = r["deck"], r["pdf"]
    if r.get("image"):
        L.append(f"- 대표 이미지(추출 {r['imageIndex']}번): {r['image']['width']}×{r['image']['height']}px (가로/세로 {r['image']['ratio']})")
    if dk:
        L += [f"- 구성: 제목 '{dk['title']}' / 부제 '{dk['subtitle']}', 내용 슬라이드 {dk['slides']}개, 불릿 {dk['bulletsMin']}~{dk['bulletsMax']}개(합 {dk['bulletsTotal']}), 최장 문장 {dk['longestText']}자(100자 초과 {dk['textsOver100']}개), 슬라이드별 한글 비율 {dk['hangulRatioPerSlide']}",
              f"- 누락 사실: {dk['factsMissed'] or '없음'} · 원문에 없는 숫자(검토 필요): {dk['numbersNotInSource'] or '없음'}"]
    if pf:
        L += [f"- PDF: {pf['bytes']}B, {pf['pages']}쪽, 쪽 크기 {pf['pageSize']}, 본문 글자 크기 분포(pt→줄 수) {pf['fontSizes']}",
              f"- 여백 밖 {pf['overflowLines'] or '없음'}, 푸터 영역 침범 {pf['footerZoneLines'] or '없음'}, 잘림 {pf['ellipsisLines'] or '없음'}, 최소 크기 줄 {pf['minFontLines'] or '없음'}, 누락 글리프 {pf['missingChars'] or '없음'}, 대체 문자 {pf['replacementChars'] or '없음'}"]
        for im in pf["images"]:
            L.append(f"- 이미지(p{im['page']}): bbox {im['bbox']} ({im['widthPt']}×{im['heightPt']}pt) ← {im['sourcePx'][0]}×{im['sourcePx'][1]}px, 비율 오차 {im['ratioErrorPct']}%, 확대 {im['upscale']}배")
    L.append(f"- 렌더: `{r['doc']}/png/page-NN.png`, 한눈에 `{r['doc']}/sheet.png`")
    L.append("")
L.append("구성 JSON 은 `<doc>/slides.json`, PDF 는 `<doc>/artifacts/`, 전체 수치는 `results.json`. 시각 검토(레이아웃·가독성)는 PNG 를 직접 열어 기록한다.")
(out_dir / "report.md").write_text("\n".join(L) + "\n")
print("\n".join(L))
sys.exit(0 if all_ok else 1)
PYEOF

rc=0
uv run --no-project --isolated --with pypdfium2 --with pillow --with numpy python -I "$SLIDE_DIR/analyze.py" "$SLIDE_DIR" "$LABEL" "$(IFS=,; echo "${SELECTED[*]}")" || rc=$?
echo "[slide] report: $SLIDE_DIR/report.md (exit $rc)"
exit "$rc"
