#!/usr/bin/env bash
# Live quality check of the voice node (narration script + TTS -> MP3) against the real OpenRouter models.
#
#   BASE_URL=<url> LIVE_CONFIRM=1 scripts/live/voice_check.sh [--out live-results] [--label NAME] [--docs en,ko,handbook]
#   LIVE_CONFIRM=1 scripts/live/voice_check.sh --with-verify-stack [...]   # isolated stack dochestra-verify:18080
#
# For every document it runs bundle -> pick_pdf -> extract -> voice through the engine API, downloads the
# MP3 and (with --with-verify-stack) reads the narration script the node saved at
# /data/runs/<run>/n_v/_work/script.txt out of the isolated stack's engine container. Then it checks:
#   script : length (target 1500-2500 chars), language (Korean), leftover markdown / bullets / emoji / URLs,
#            key facts of the source document, mention of the extracted images, longest sentence
#   TTS    : number of synthesis chunks (the script is split at 4000 chars) and where the cuts fall
#   audio  : decoded duration, sample rate, channels, bitrate, size, speaking rate (script chars per second),
#            leading / trailing / internal silence (20 ms RMS frames), abrupt ending, clipping, and the
#            silences around the estimated chunk boundaries
#   run    : total seconds
# The MP3 is decoded with a one-off `uv run --with miniaudio --with numpy` environment (nothing is
# installed on the host).
#
# Results: <out>/<UTC timestamp>/voice-check/{report.md,results.json,<doc>/...}. API keys are never read or
# printed (the engine inside the stack holds them). Without LIVE_CONFIRM=1 only the plan is printed.
# BASE_URL has no default on purpose (the user's own stack must not be hit by accident).
# Exit status: 0 when every selected document passes, 1 otherwise, 3 for infrastructure problems.
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
  [[ -n "${DOC_FILE[$d]:-}" ]] || { echo "voice_check: unknown doc '$d' (known: en, ko, handbook)" >&2; exit 2; }
  if [[ -f "$FIXTURES/${DOC_FILE[$d]}" ]]; then SELECTED+=("$d"); else echo "voice_check: skipping $d (missing fixtures/live/${DOC_FILE[$d]})" >&2; fi
done
((${#SELECTED[@]} > 0)) || { echo "voice_check: no documents available" >&2; exit 2; }

if [[ "$WITH_STACK" != true && -z "${BASE_URL:-}" ]]; then
  echo "voice_check: BASE_URL is required (or use --with-verify-stack for an isolated stack)" >&2
  exit 2
fi

# ---- gate: plan only unless LIVE_CONFIRM=1 ---------------------------------------------------
if [[ "${LIVE_CONFIRM:-}" != 1 ]]; then
  echo "voice_check: PLAN ONLY (no workflows are created, no paid calls are made)"
  echo "  target   : ${BASE_URL:-<isolated verify stack http://localhost:18080>}"
  echo "  documents: ${SELECTED[*]}"
  echo "  per document: 1 extraction run + 1 LLM call (narration script) + 1 TTS call per 4000 characters of script"
  echo "  Set LIVE_CONFIRM=1 to run for real."
  exit 0
fi

command -v uv >/dev/null || { echo "voice_check: uv is required for the audio analysis (one-off environment)" >&2; exit 3; }
command -v jq >/dev/null || { echo "voice_check: jq is required" >&2; exit 3; }

# ---- optional isolated stack (same project, port and lock as scripts/verify-stack.sh) -----------
if [[ "$WITH_STACK" == true ]]; then
  avail_gb=$(df --output=avail -BG / | tail -n1 | tr -dc '0-9')
  if ((avail_gb < 10)); then
    echo "voice_check: only ${avail_gb}G free on / (need >= 10G) - aborting before the image build" >&2
    exit 3
  fi
  exec 9>/tmp/dochestra-verify.lock
  flock -w 1800 9
  export COMPOSE_PROJECT_NAME=dochestra-verify
  export WEB_PORT=18080
  export BASE_URL=http://localhost:18080
  if [[ -f .env ]]; then export ENV_FILE=.env; else export ENV_FILE=.env.example; fi
  export COMPOSE_ENV_FILES="$ENV_FILE"
  echo "[voice] isolated stack: project=$COMPOSE_PROJECT_NAME port=$WEB_PORT env=$ENV_FILE"
  stack_down() { docker compose down -v --remove-orphans >/dev/null 2>&1 || true; }
  docker compose down -v --remove-orphans >/dev/null 2>&1 || true
  if ! docker compose up -d --build --wait; then
    echo "voice_check: the isolated stack failed to start" >&2
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
VOICE_DIR="$OUT_DIR/voice-check"
mkdir -p "$VOICE_DIR/inputs"

finish() {
  local rc=$?
  live_cleanup_resources
  if [[ "$WITH_STACK" == true ]]; then stack_down; fi
  exit "$rc"
}
trap finish EXIT

echo "[voice] BASE_URL=$BASE_URL label=$LABEL out=$VOICE_DIR"
curl -fsS "$API/health" >/dev/null || { echo "voice_check: $API/health is not reachable" >&2; exit 3; }

# ---- run bundle -> pick_pdf -> extract -> voice for every document ----------------------------
for d in "${SELECTED[@]}"; do
  src="$FIXTURES/${DOC_FILE[$d]}"
  cp "$src" "$VOICE_DIR/inputs/${DOC_FILE[$d]}" # snapshot: other agents may regenerate the fixtures
  doc_dir="$VOICE_DIR/$d"
  mkdir -p "$doc_dir"
  echo "[voice] === $d: narrating ${DOC_FILE[$d]}"

  if ! live_make_bundle "voice-check-$d" "$VOICE_DIR/inputs/${DOC_FILE[$d]}"; then continue; fi
  bundle_id="$LIVE_ID"
  graph=$(live_base_graph "$bundle_id" | live_add_node n_v voice '{}' true |
    live_add_edge n_x md n_v md | live_add_edge n_x images n_v images)
  if ! live_make_workflow "voice-check-$d" "$graph"; then continue; fi
  wf="$LIVE_ID"

  OUT_DIR="$VOICE_DIR" NODE_TIMEOUT=900 live_run_workflow "$d" "$wf" n_v n_x || true

  # The narration script lives only inside the engine's data volume (isolated project only).
  if [[ "$WITH_STACK" == true && -s "$doc_dir/run.json" ]]; then
    run_id=$(jq -r '.id // empty' "$doc_dir/run.json" 2>/dev/null || true)
    if [[ -n "$run_id" ]]; then
      docker compose exec -T engine cat "/data/runs/$run_id/n_v/_work/script.txt" >"$doc_dir/script.txt" 2>"$doc_dir/script.err" ||
        echo "[voice] $d: could not read script.txt (see $doc_dir/script.err)" >&2
    fi
  fi
done

# ---- analysis (python3 + a one-off uv environment for MP3 decoding) -----------------------------
cat >"$VOICE_DIR/analyze.py" <<'PYEOF'
import json, math, re, sys, unicodedata
from pathlib import Path

import miniaudio
import numpy as np

out_dir, label = Path(sys.argv[1]), sys.argv[2]
selected = sys.argv[3].split(",")

TTS_CHUNK = 4000
SCRIPT_MIN, SCRIPT_MAX = 1500, 2500
FRAME_S = 0.02
SILENT_RMS = 0.01  # about -40 dBFS

# Key facts of each source document (alternatives per fact).
DOCS = {
    "en": dict(file="report_en.pdf", facts=[["482", "4억8,200만", "4억8200만", "4억8천2백만", "4억8천200만"], ["Min-jun Park", "박민준", "민준"], ["31"], ["Asia-Pacific", "아시아"], ["Busan", "부산"], ["2012"], ["540", "5억4천만", "5억4,000만"], ["Taiwan", "대만"], ["Gimhae", "김해"]]),
    "ko": dict(file="report_ko.pdf", facts=[["482", "4억8,200만", "4억8200만", "4억8천2백만", "4억8천200만"], ["박민준"], ["31"], ["아시아"], ["부산"], ["2012"], ["대만"], ["김해"], ["62"]]),
    "handbook": dict(file="handbook_en.pdf", facts=[["Aurora Robotics", "오로라 로보틱스"], ["0.3", "영 점 삼", "0.3미터"], ["30분", "30 minutes"], ["20%", "20 percent", "20퍼센트", "20 퍼센트"], ["24시간", "24 hours"]]),
}

MARKDOWN = [
    ("heading", re.compile(r"^\s{0,3}#{1,6}\s", re.M)),
    ("bold/code", re.compile(r"\*\*|__|`|~~")),
    ("bullet", re.compile(r"^\s*[-*•·▪●○■□▶►]\s", re.M)),
    ("numbered list", re.compile(r"^\s*\d{1,2}[.)]\s", re.M)),
    ("table", re.compile(r"\|.*\|")),
    ("link", re.compile(r"\[[^\]]+\]\([^)]+\)|https?://")),
    ("rule", re.compile(r"^\s*(-{3,}|\*{3,}|={3,})\s*$", re.M)),
]
IMAGE_WORDS = re.compile(r"그림|이미지|사진|차트|그래프|도표|도식|삽화|figure", re.I)


def is_emoji(ch):
    cp = ord(ch)
    return cp >= 0x1F000 or 0x2600 <= cp <= 0x27BF or cp in (0x2B50, 0x2B55, 0x2705, 0x274C, 0xFE0F)


def hangul_ratio(text):
    letters = [c for c in text if c.isalpha()]
    return sum("가" <= c <= "힣" for c in letters) / len(letters) if letters else 0.0


def sentences(text):
    out, cur = [], ""
    chars = list(text)
    for i, c in enumerate(chars):
        cur += c
        nxt = chars[i + 1] if i + 1 < len(chars) else None
        if c in "\n。！？" or (c in ".!?" and (nxt is None or nxt.isspace())):
            out.append(cur)
            cur = ""
    if cur:
        out.append(cur)
    return out


def tts_chunks(text, limit=TTS_CHUNK):
    """Mirror of engine/src/clients/tts.rs split_text (sentence-packed chunks)."""
    chunks, cur = [], ""
    for s in sentences(text):
        if len(s) > limit:
            if cur:
                chunks.append(cur)
                cur = ""
            pieces = [s[i : i + limit] for i in range(0, len(s), limit)]
            cur = pieces.pop()
            chunks.extend(pieces)
        elif len(cur) + len(s) > limit:
            chunks.append(cur)
            cur = s
        else:
            cur += s
    if cur:
        chunks.append(cur)
    return [c.strip() for c in chunks if c.strip()]


def silence_runs(rms):
    """[(start_frame, length_frames)] of runs below the silence threshold."""
    runs, start = [], None
    for i, silent in enumerate(rms < SILENT_RMS):
        if silent and start is None:
            start = i
        elif not silent and start is not None:
            runs.append((start, i - start))
            start = None
    if start is not None:
        runs.append((start, len(rms) - start))
    return runs


def analyze_audio(path, script_len, chunk_offsets):
    info = miniaudio.mp3_get_file_info(str(path))
    dec = miniaudio.mp3_read_file_f32(str(path))
    x = np.asarray(dec.samples, dtype=np.float32)
    if dec.nchannels > 1:
        x = x.reshape(-1, dec.nchannels).mean(axis=1)
    rate = dec.sample_rate
    dur = len(x) / rate
    size = path.stat().st_size
    frame = int(rate * FRAME_S)
    n = len(x) // frame
    rms = np.sqrt((x[: n * frame].reshape(n, frame) ** 2).mean(axis=1)) if n else np.zeros(1)
    runs = silence_runs(rms)
    lead = runs[0][1] * FRAME_S if runs and runs[0][0] == 0 else 0.0
    trail = runs[-1][1] * FRAME_S if runs and runs[-1][0] + runs[-1][1] == len(rms) else 0.0
    internal = [(s * FRAME_S, ln * FRAME_S) for s, ln in runs if s > 0 and s + ln < len(rms)]
    longest = max((ln for _, ln in internal), default=0.0)
    tail_rms = float(np.sqrt((x[-int(rate * 0.1) :] ** 2).mean())) if len(x) else 0.0
    res = dict(
        sampleRate=rate, channels=dec.nchannels, seconds=round(dur, 1), bytes=size,
        kbps=round(size * 8 / dur / 1000, 1) if dur else 0,
        lead=round(lead, 2), trail=round(trail, 2), longestSilence=round(longest, 2),
        silences08=sum(1 for _, ln in internal if ln >= 0.8), silences15=sum(1 for _, ln in internal if ln >= 1.5),
        endsAbruptly=bool(tail_rms > 0.05), peak=round(float(np.abs(x).max()), 3) if len(x) else 0,
        clipped=round(float((np.abs(x) > 0.99).mean()) * 100, 4) if len(x) else 0,
        rmsDb=round(20 * math.log10(float(np.sqrt((x**2).mean())) + 1e-9), 1) if len(x) else -99,
        charsPerSecond=round(script_len / dur, 2) if dur else 0,
        mp3Frames=info.num_frames,
    )
    # Silences around the estimated chunk boundaries (position estimated from the character offset).
    bounds = []
    for off in chunk_offsets:
        t = dur * off / max(script_len, 1)
        window = max(8.0, dur * 0.03)
        near = [ln for s, ln in internal if abs(s + ln / 2 - t) <= window]
        bounds.append(dict(atSeconds=round(t, 1), window=round(window, 1), longestSilenceNear=round(max(near, default=0.0), 2)))
    res["chunkBoundaries"] = bounds
    # Per-chunk loudness (by estimated position) to catch level jumps between chunks.
    if chunk_offsets:
        edges = [0.0] + [dur * o / max(script_len, 1) for o in chunk_offsets] + [dur]
        levels = []
        for a, b in zip(edges, edges[1:]):
            seg = x[int(a * rate) : int(b * rate)]
            speech = seg[np.abs(seg) > SILENT_RMS]
            levels.append(round(20 * math.log10(float(np.sqrt((speech**2).mean())) + 1e-9), 1) if len(speech) else -99)
        res["chunkLevelsDb"] = levels
    return res


def analyze_script(text, doc):
    norm = unicodedata.normalize("NFKC", text)
    sents = sentences(text)
    lens = sorted(len(s.strip()) for s in sents)
    md = [name for name, rx in MARKDOWN if rx.search(text)]
    emoji = sorted({c for c in text if is_emoji(c)})
    facts = DOCS[doc]["facts"]
    squeezed = re.sub(r"\s", "", norm).lower()  # "4억 8천2백만" == "4억8천2백만"
    hits = [any(re.sub(r"\s", "", alt).lower() in squeezed for alt in alts) for alts in facts]
    chunks = tts_chunks(text)
    offsets, acc = [], 0
    for c in chunks[:-1]:
        acc = text.find(c, acc) + len(c)
        offsets.append(acc)
    return dict(
        chars=len(text), nonSpaceChars=len(re.sub(r"\s", "", text)), lines=text.count("\n") + 1,
        lengthOk=SCRIPT_MIN <= len(text) <= SCRIPT_MAX,
        hangulRatio=round(hangul_ratio(text), 2), koreanOk=hangul_ratio(text) >= 0.6,
        markdownResidue=md, emoji=emoji, symbolsOk=not md and not emoji,
        parentheses=len(re.findall(r"[(（][^)）]*[)）]", text)),
        latinWords=len(re.findall(r"[A-Za-z]{2,}", text)),
        digitTokens=len(re.findall(r"\d[\d,.]*", text)),
        longestSentence=lens[-1] if lens else 0, sentencesOver100=sum(1 for n in lens if n > 100),
        factsTotal=len(facts), factsHit=sum(hits), factsMissed=[alts[0] for alts, h in zip(facts, hits) if not h],
        imageMentions=len(IMAGE_WORDS.findall(text)),
        ttsChunks=len(chunks), chunkSizes=[len(c) for c in chunks], chunkOffsets=offsets,
        chunkEndsAtSentence=[bool(re.search(r"[.!?。！？]$", c)) for c in chunks],
    )


results, all_ok = [], True
for d in selected:
    doc_dir = out_dir / d
    r = dict(doc=d, file=DOCS[d]["file"])
    rr = json.loads((doc_dir / "result.json").read_text()) if (doc_dir / "result.json").exists() else {}
    r.update(status=rr.get("status", "no result"), seconds=rr.get("seconds"), error=rr.get("error", ""), pipeline=rr.get("pipeline", ""))
    mp3s = sorted((doc_dir / "artifacts").glob("n_v-mp3-*"))
    images = len(list((doc_dir / "artifacts").glob("n_x-images-*")))
    r["extractedImages"] = images
    script_path = doc_dir / "script.txt"
    script = script_path.read_text() if script_path.exists() and script_path.stat().st_size else None
    r["script"] = analyze_script(script, d) if script is not None else None
    r["audio"] = None
    if mp3s:
        try:
            r["audio"] = analyze_audio(mp3s[0], len(script) if script else 0, r["script"]["chunkOffsets"] if r["script"] else [])
        except Exception as exc:  # decoding problems are findings, not crashes
            r["audioError"] = f"{type(exc).__name__}: {exc}"

    s, a = r["script"], r["audio"]
    checks = {
        "실행 성공": r["status"] == "succeeded",
        "대본 확보": s is not None,
        "MP3 디코딩": a is not None,
    }
    if s:
        checks.update({
            f"대본 길이 {SCRIPT_MIN}~{SCRIPT_MAX}자": s["lengthOk"],
            "한국어 구어체(한글 비율≥0.6)": s["koreanOk"],
            "마크다운·이모지 없음": s["symbolsOk"],
            "핵심 사실 ≥80%": s["factsHit"] >= math.ceil(0.8 * s["factsTotal"]),
            "청크가 문장 끝에서 끊김": all(s["chunkEndsAtSentence"]),
            "영문 상호·이름의 한글 표기(영문 단어 ≤2)": s["latinWords"] <= 2,
        })
        if images:
            checks["이미지 언급"] = s["imageMentions"] > 0
    if a:
        checks.update({
            "샘플레이트 24kHz": a["sampleRate"] == 24000,
            "재생 길이 ∝ 대본(3~12자/초)": 3.0 <= a["charsPerSecond"] <= 12.0,
            "내부 무음 ≤2.5초": a["longestSilence"] <= 2.5,
            "끝이 잘리지 않음": not a["endsAbruptly"],
            "클리핑 <0.1%": a["clipped"] < 0.1,
        })
    r["checks"] = checks
    r["pass"] = all(checks.values())
    all_ok &= r["pass"]
    results.append(r)

(out_dir / "results.json").write_text(json.dumps(dict(label=label, accepted=all_ok, docs=results), ensure_ascii=False, indent=1))

L = [f"# voice 품질 점검 — {label}", "", f"- 판정: **{'통과' if all_ok else '실패'}**", ""]
L += ["## 요약", "", "| 문서 | 실행 | 소요 | 대본 글자 | 재생(초) | 글자/초 | 청크 | 사실 | 최장 무음 | 판정 |", "| :-- | :-: | --: | --: | --: | --: | --: | :-: | --: | :-: |"]
for r in results:
    s, a = r["script"], r["audio"]
    L.append("| {} | {} | {}s | {} | {} | {} | {} | {} | {} | {} |".format(
        r["doc"], r["status"], r["seconds"], s["chars"] if s else "-", a["seconds"] if a else "-",
        a["charsPerSecond"] if a else "-", s["ttsChunks"] if s else "-",
        f"{s['factsHit']}/{s['factsTotal']}" if s else "-", f"{a['longestSilence']}s" if a else "-", "✓" if r["pass"] else "✗"))
L += ["", "## 문서별 점검", ""]
for r in results:
    L += [f"### {r['doc']} ({r['file']})", ""]
    if r["error"]:
        L += [f"- 오류: {r['error']}"]
    L += [f"- 파이프라인: {r['pipeline']} · 추출 이미지 {r['extractedImages']}개"]
    for name, ok in r["checks"].items():
        L.append(f"- {'✓' if ok else '✗'} {name}")
    s, a = r["script"], r["audio"]
    if s:
        L += [f"- 대본: {s['chars']}자(공백 제외 {s['nonSpaceChars']}), 줄 {s['lines']}, 한글 비율 {s['hangulRatio']}, 괄호 {s['parentheses']}개, 영문 단어 {s['latinWords']}개, 숫자 {s['digitTokens']}개, 최장 문장 {s['longestSentence']}자(100자 초과 {s['sentencesOver100']}개), 이미지 언급 {s['imageMentions']}회",
              f"- 잔존 기호: 마크다운 {s['markdownResidue'] or '없음'}, 이모지 {s['emoji'] or '없음'} · 누락 사실: {s['factsMissed'] or '없음'}",
              f"- TTS 청크 {s['ttsChunks']}개 {s['chunkSizes']} (문장 끝에서 끊김: {s['chunkEndsAtSentence']})"]
    if a:
        L += [f"- MP3: {a['sampleRate']}Hz {a['channels']}ch {a['kbps']}kbps {a['bytes']}B, 재생 {a['seconds']}s, 음량 {a['rmsDb']}dBFS(피크 {a['peak']}, 클리핑 {a['clipped']}%)",
              f"- 무음: 앞 {a['lead']}s, 뒤 {a['trail']}s, 내부 최장 {a['longestSilence']}s (0.8s↑ {a['silences08']}회, 1.5s↑ {a['silences15']}회), 끝 잘림 {a['endsAbruptly']}"]
        for b in a["chunkBoundaries"]:
            L.append(f"- 청크 경계 추정 {b['atSeconds']}s(±{b['window']}s): 근처 최장 무음 {b['longestSilenceNear']}s")
        if a.get("chunkLevelsDb"):
            L.append(f"- 청크별 음량(dBFS): {a['chunkLevelsDb']}")
    if r.get("audioError"):
        L.append(f"- MP3 분석 오류: {r['audioError']}")
    L.append("")
L.append("대본 원문은 `<doc>/script.txt`, MP3 는 `<doc>/artifacts/`, 전체 수치는 `results.json`.")
(out_dir / "report.md").write_text("\n".join(L) + "\n")
print("\n".join(L))
sys.exit(0 if all_ok else 1)
PYEOF

rc=0
uv run --no-project --isolated --with miniaudio --with numpy python -I "$VOICE_DIR/analyze.py" "$VOICE_DIR" "$LABEL" "$(IFS=,; echo "${SELECTED[*]}")" || rc=$?
echo "[voice] report: $VOICE_DIR/report.md (exit $rc)"
exit "$rc"
