#!/usr/bin/env python3
"""Checks an analyze (image -> Markdown) result against ground truth (stdlib only).

    analyze_check.py <analysis.md> --case chart|table|text|photo|small --facts vision_facts.json

Prints {"ok": bool, "checks": [{"name", "ok", "detail"}], "score": float}.
score = share of ground-truth values and table rows that are present (the value read-out accuracy).
"""

import argparse
import json
import re
import sys


def norm(t: str) -> str:
    return re.sub(r"[\s,]", "", t).lower()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("analysis")
    ap.add_argument("--case", required=True)
    ap.add_argument("--facts", required=True)
    a = ap.parse_args()
    out = open(a.analysis, encoding="utf-8").read()
    truth = json.load(open(a.facts, encoding="utf-8"))[a.case]
    flat = norm(out)
    checks: list[dict] = []

    def add(name: str, ok: bool, detail: str) -> None:
        checks.append({"name": name, "ok": ok, "detail": detail})

    hits = [v[0] for v in truth["values"] if any(norm(alt) in flat for alt in v)]
    miss = [v[0] for v in truth["values"] if v[0] not in hits]
    row_ok = 0
    for row in truth.get("rows", []):
        if any(all(norm(c) in norm(line) for c in row) for line in out.splitlines()):
            row_ok += 1
    total = len(truth["values"]) + len(truth.get("rows", []))
    score = (len(hits) + row_ok) / total if total else 1.0
    detail = f"값 {len(hits)}/{len(truth['values'])}"
    if truth.get("rows"):
        detail += f", 행(같은 줄에 항목·수치) {row_ok}/{len(truth['rows'])}"
    add("정답 값 일치율", score >= 0.9, f"{detail} → {score:.0%}" + (f", 누락: {miss}" if miss else ""))

    # numbers the image does not contain (hallucinated values); axis ticks etc. are allowed
    if truth.get("allowed_numbers") is not None and truth["allowed_numbers"]:
        allowed = {n.replace(",", "") for n in truth["allowed_numbers"]}
        # numbers the model derived itself (differences, sums) are marked '(계산값)': not read-outs
        stated = "\n".join(l for l in out.splitlines() if "계산" not in l)
        found = {n.replace(",", "") for n in re.findall(r"\d[\d,]*(?:\.\d+)?", stated)}
        extra = sorted(n for n in found if n not in allowed and len(n) > 1)
        add("이미지에 없는 수치 없음", not extra, "없음" if not extra else f"이미지에 없는 수치: {extra[:10]}")

    forbid = truth.get("forbid", [])
    if forbid:
        bad = [w for w in forbid for l in out.splitlines() if w in l and not re.search(r"없|않|아니|보이지", l)]
        add("환각(없는 대상 서술) 없음", not bad, "없음" if not bad else f"의심 단어: {sorted(set(bad))}")

    first = next((l for l in out.splitlines() if l.strip()), "")
    heads = [l.strip() for l in out.splitlines() if l.startswith("#")]
    add("구조: '## 요약' 으로 시작", first.strip().startswith("## 요약"), f"첫 줄: {first[:40]!r}")
    add("제목 목록(참고)", True, " | ".join(heads)[:160] or "(제목 없음)")

    hangul = len(re.findall(r"[가-힣]", out))
    latin = len(re.findall(r"[A-Za-z]", out))
    add("한국어 출력", hangul / max(1, hangul + latin) >= 0.25, f"한글 비율 {hangul / max(1, hangul + latin):.0%}")
    add("길이 ≤ 4000자", len(out) <= 4000, f"{len(out)}자")
    unreadable = len(re.findall(r"판독 불가|불확실|읽을 수 없|확인 불가", out))
    add("불확실 표기(참고)", True, f"{unreadable}회")
    noise = re.match(r"^\s*(다음은|아래는|물론|sure|here)", out, re.IGNORECASE)
    add("서문/군더더기 없음", not noise, "없음" if not noise else f"의심: {out[:50]!r}")

    print(json.dumps({"ok": all(c["ok"] for c in checks), "score": round(score, 3), "checks": checks}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
