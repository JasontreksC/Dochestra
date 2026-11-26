export type AnswerPart = { kind: 'text'; text: string } | { kind: 'cite'; n: number }

// `[1]`, `[2, 3]`, `[1,2]` — numbers are 1-based positions in the sources list
// (engine/src/api/rag.rs numbers context passages `[i + 1]` in sources order).
const CITE_RE = /\[(\d{1,3}(?:\s*[,，]\s*\d{1,3})*)\]/g

/** Splits an answer into text and citation parts; numbers outside 1..sourceCount stay text. */
export function splitCitations(answer: string, sourceCount: number): AnswerPart[] {
  const parts: AnswerPart[] = []
  let last = 0
  const pushText = (text: string) => {
    if (!text) return
    const prev = parts[parts.length - 1]
    if (prev?.kind === 'text') prev.text += text
    else parts.push({ kind: 'text', text })
  }
  for (const m of answer.matchAll(CITE_RE)) {
    const nums = (m[1] ?? '').split(/[,，]/).map((s) => Number(s.trim()))
    const start = m.index
    pushText(answer.slice(last, start))
    if (nums.every((n) => n >= 1 && n <= sourceCount)) {
      for (const n of nums) parts.push({ kind: 'cite', n })
    } else {
      pushText(m[0])
    }
    last = start + m[0].length
  }
  pushText(answer.slice(last))
  return parts
}
