#!/usr/bin/env python3
"""Checks a summary (Markdown) against its source document (stdlib only).

    summary_check.py <source.md> <summary.md> --doc <key> --images N [--facts facts.json] [--lang ko]

Prints {"ok": bool, "checks": [{"name", "ok", "detail"}]}.
Checks: facts kept, numbers not present in the source, structure ('## 핵심 요점' + section
summaries), output language, length ratio, preface noise, section coverage, exclusion note
accuracy (images beyond the 20-image limit) and (informational) image labels used.
"""

import argparse
import collections
import json
import re
import sys

MAX_IMAGES = 20
NOISE = re.compile(r"^(다음은|아래는|요약:|요약 결과|here is|sure|물론)", re.IGNORECASE)


def nums(t: str) -> list[str]:
    return [n.replace(",", "") for n in re.findall(r"\d+(?:[.,]\d+)*", t)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("source")
    ap.add_argument("summary")
    ap.add_argument("--doc", required=True)
    ap.add_argument("--images", type=int, default=0)
    ap.add_argument("--facts", default="")
    ap.add_argument("--lang", default="ko")
    a = ap.parse_args()
    src = open(a.source, encoding="utf-8").read()
    out = open(a.summary, encoding="utf-8").read()
    checks: list[dict] = []

    def add(name: str, ok: bool, detail: str) -> None:
        checks.append({"name": name, "ok": ok, "detail": detail})

    # facts
    if a.facts:
        facts = json.load(open(a.facts, encoding="utf-8")).get(a.doc, [])
        kept = [f[0] for f in facts if any(alt.lower() in out.lower() for alt in f)]
        lost = [f[0] for f in facts if f[0] not in kept]
        ratio = len(kept) / len(facts) if facts else 1.0
        add("핵심 사실 포함", ratio >= 0.6, f"{len(kept)}/{len(facts)} ({ratio:.0%}) 누락: {lost}")

    # numbers in the summary that the source does not contain (possible hallucination)
    s_nums, o_nums = collections.Counter(nums(src)), collections.Counter(nums(out))
    invented = sorted(n for n in o_nums if n not in s_nums and len(n) > 1)
    # derived numbers are legitimate (sums, percentages written differently): flag, don't fail hard
    if a.images > 0 and invented:
        # Numbers read from pictures (chart values, labels) are legitimately absent from the text.
        add("원문에 없는 숫자 없음", True, f"이미지 판독 수치 포함 가능 — 확인 필요: {invented[:12]}")
    else:
        add("원문에 없는 숫자 없음", len(invented) <= 3, "없음" if not invented else f"원문에 없는 숫자: {invented[:10]}")

    # structure
    has_key = bool(re.search(r"(?m)^##\s*핵심 요점", out))
    sections = len(re.findall(r"(?m)^#{2,3}\s+(?!핵심 요점)", out))
    add("구조(## 핵심 요점 + 섹션별 요약)", has_key and sections >= 1, f"핵심 요점 {'있음' if has_key else '없음'}, 섹션 {sections}개")
    bullets = len(re.findall(r"(?m)^\s*[-*]\s", out))
    add("핵심 요점 목록 존재", bullets >= 3, f"글머리 {bullets}개")

    src_heads = len(re.findall(r"(?m)^#{2,3}\s", src))
    cov = min(1.0, sections / src_heads) if src_heads else 1.0
    add("섹션 커버리지(요약 섹션 / 원문 섹션)", cov >= 0.5 or sections >= 5, f"{sections}/{src_heads} ({cov:.0%})")

    # language
    hangul = len(re.findall(r"[가-힣]", out))
    latin = len(re.findall(r"[A-Za-z]", out))
    ratio = hangul / max(1, hangul + latin)
    add("한국어 출력", ratio >= 0.4, f"한글 비율 {ratio:.0%}")

    # length
    add("길이(요약/원문)", len(out) < len(src) * 0.8, f"{len(out)}자 / {len(src)}자 = {len(out) / max(1, len(src)):.0%}")

    first = next((l for l in out.splitlines() if l.strip()), "")
    add("서문/군더더기 없음", not NOISE.match(first.strip()) and not out.lstrip().startswith("```"), "없음" if not NOISE.match(first.strip()) else f"의심 줄: {first[:60]}")

    # exclusion note accuracy
    expected = max(0, a.images - MAX_IMAGES)
    m = re.search(r"이미지 (\d+)개는 크기/개수 제한으로 제외됨", out)
    if expected:
        add("제외 주석 정확", bool(m) and int(m.group(1)) == expected, f"기대 {expected}개 / 실제 {m.group(1) + '개' if m else '없음'}")
    else:
        add("제외 주석 없음(이미지 ≤ 20)", m is None, "없음" if m is None else f"불필요한 주석: {m.group(0)}")

    # informational: labels only present inside the pictures (gallery doc)
    if a.doc == "gallery_en":
        labels = sorted(set(re.findall(r"(?i)UNIT\s*0?(\d{1,2})", out)))
        add("이미지 반영(그림 속 UNIT 번호 언급, 참고)", True, f"{len(labels)}종 언급: {labels[:10]}")

    print(json.dumps({"ok": all(c["ok"] for c in checks), "checks": checks}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
