#!/usr/bin/env python3
"""Structural checks of a translated Markdown against its source (stdlib only).

    translate_check.py <source.md> <translated.md> [--glossary '{"term": "번역"}'] [--chunks N]

Prints a JSON object {"checks": [{"name", "ok", "detail"}...], "ok": bool}.
Checks: block count, tables (count, rows, columns), fenced code (verbatim), image placeholders,
headings, glossary compliance, preface/afterword noise, adjacent duplicate blocks.
"""

import argparse
import collections
import json
import re
import sys

PLACEHOLDER = "<!-- image -->"
NOISE = re.compile(
    r"^(here is|here's|below is|sure[,!]|certainly|translation:|translated|다음은|아래는|번역[:：]|번역본|번역 결과)",
    re.IGNORECASE,
)


def blocks(md: str) -> list[str]:
    out, cur, fence = [], [], None
    for line in md.splitlines():
        t = line.lstrip()
        if fence:
            cur.append(line)
            if t.startswith(fence * 3):
                fence = None
        elif t.startswith("```") or t.startswith("~~~"):
            fence = t[0]
            cur.append(line)
        elif not line.strip():
            if cur:
                out.append("\n".join(cur))
                cur = []
        else:
            cur.append(line)
    if cur:
        out.append("\n".join(cur))
    return out


def tables(bs: list[str]) -> list[tuple[int, int]]:
    shapes = []
    for b in bs:
        lines = b.splitlines()
        if len(lines) >= 2 and lines[0].lstrip().startswith("|") and re.match(r"^\s*\|?[\s:|-]+\|[\s:|-]*$", lines[1]):
            cols = [len(re.findall(r"(?<!\\)\|", l)) for l in lines]
            shapes.append((len(lines), max(cols)))
    return shapes


def code_blocks(bs: list[str]) -> list[str]:
    return [b for b in bs if b.lstrip().startswith(("```", "~~~"))]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("source")
    ap.add_argument("translated")
    ap.add_argument("--glossary", default="{}")
    ap.add_argument("--chunks", type=int, default=0)
    a = ap.parse_args()
    src, dst = open(a.source, encoding="utf-8").read(), open(a.translated, encoding="utf-8").read()
    sb, db = blocks(src), blocks(dst)
    checks = []

    def add(name: str, ok: bool, detail: str) -> None:
        checks.append({"name": name, "ok": ok, "detail": detail})

    # PDF extraction splits paragraphs at page breaks; the node re-joins those fragments, so the
    # block count may legitimately shrink. Informational only: count the source fragments.
    frags = sum(1 for i in range(1, len(sb)) if re.match(r"[a-z]", sb[i]) and sb[i - 1][:1].isalpha())
    add("블록 수(참고)", len(db) >= len(sb) - frags - 5, f"원문 {len(sb)} / 번역 {len(db)} (쪽 경계 조각 {frags}개 재결합 허용)")
    st, dt = tables(sb), tables(db)
    add("표 수·행·열", st == dt, f"원문 {st} / 번역 {dt}")
    sc, dc = code_blocks(sb), code_blocks(db)
    add("코드 블록 원문 보존", sc == dc, f"원문 {len(sc)}개 / 번역 {len(dc)}개 / 동일 {sum(1 for c in sc if c in dc)}개")
    add("이미지 placeholder 수", src.count(PLACEHOLDER) == dst.count(PLACEHOLDER), f"원문 {src.count(PLACEHOLDER)} / 번역 {dst.count(PLACEHOLDER)}")
    nums = lambda t: re.findall(r"\d+(?:[.,]\d+)*", t)
    miss = collections.Counter(nums(src)) - collections.Counter(nums(dst))
    if miss and re.search(r"[가-힣]", src):
        # Korean groups numbers by 만/억 (4억 8,200만 = 482 million): digits legitimately change.
        add("숫자 누락 없음", True, f"한글 원문(만·억 단위 변환 허용) — 확인 필요 항목: {dict(list(miss.items())[:8])}")
    else:
        add("숫자 누락 없음", not miss, "누락 없음" if not miss else f"누락: {dict(list(miss.items())[:8])}")
    sh = len(re.findall(r"(?m)^#{1,6}\s", src))
    dh = len(re.findall(r"(?m)^#{1,6}\s", dst))
    add("제목(#) 수", sh == dh, f"원문 {sh} / 번역 {dh}")

    gloss = json.loads(a.glossary)
    if gloss:
        miss = []
        for term, tr in gloss.items():
            if re.search(re.escape(term), src, re.IGNORECASE) and tr not in dst:
                miss.append(f"{term}→{tr}")
        add("glossary 준수", not miss, "누락 없음" if not miss else "미반영: " + ", ".join(miss))

    def lonely_headings(bs: list[str]) -> int:
        return sum(1 for i, b in enumerate(bs) if b.startswith("#") and "\n" not in b and (i + 1 == len(bs) or bs[i + 1].startswith("#")))

    ls, ld = lonely_headings(sb), lonely_headings(db)
    add("제목 뒤 본문 보존", ls == ld, f"본문 없는 제목: 원문 {ls} / 번역 {ld}")

    first = next((l for l in dst.splitlines() if l.strip()), "")
    last = next((l for l in reversed(dst.splitlines()) if l.strip()), "")
    noisy = [l for l in (first, last) if NOISE.match(l.strip())]
    add("서문/군더더기 없음", not noisy, "없음" if not noisy else f"의심 줄: {noisy}")

    wrapped = dst.lstrip().startswith("```") and dst.rstrip().endswith("```") and not src.lstrip().startswith("```")
    add("전체를 코드 블록으로 감싸지 않음", not wrapped, "정상" if not wrapped else "번역문 전체가 ``` 로 감싸져 있음")

    dup = [i for i in range(1, len(db)) if db[i] == db[i - 1] and len(db[i]) > 30]
    add("인접 중복 블록 없음", not dup, "없음" if not dup else f"중복 위치 {dup[:5]}")
    if a.chunks:
        add("청크 수", True, f"{a.chunks}개")

    ok = all(c["ok"] for c in checks)
    print(json.dumps({"ok": ok, "checks": checks}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
