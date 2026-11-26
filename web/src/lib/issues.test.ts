import { describe, expect, it } from 'vitest'
import type { ValidationIssue } from '@/bindings/ValidationIssue'
import { formatIssue, issueNodeIds } from './issues.ts'

const label = (id: string) => `노드(${id})`
const PDF = { scalar: 'PDF' as const, list: false }
const PDFS = { scalar: 'PDF' as const, list: true }

const ALL: ValidationIssue[] = [
  { code: 'DUPLICATE_ID', id: 'x' },
  { code: 'UNKNOWN_NODE', edgeId: 'e1', nodeId: 'n9' },
  { code: 'UNKNOWN_SOCKET', edgeId: 'e1', nodeId: 'a', socketId: 'zz' },
  { code: 'TYPE_MISMATCH', edgeId: 'e2', expected: PDF, actual: PDFS },
  { code: 'MULTIPLE_INCOMING', nodeId: 'b', socketId: 'md' },
  { code: 'CYCLE_DETECTED', nodeIds: ['a', 'b', 'c'] },
  { code: 'NO_OUTPUT_NODE' },
  { code: 'UNCONNECTED_INPUT', nodeId: 'c', socketId: 'pdf' },
  { code: 'INVALID_PARAMS', nodeId: 'd', message: 'index 는 0 이상의 정수여야 합니다' },
]

describe('formatIssue', () => {
  it('describes every issue code in one line', () => {
    expect(ALL.map((i) => formatIssue(i, label))).toEqual([
      'ID 중복: x',
      '엣지 e1 가 존재하지 않는 노드 n9 를 참조합니다',
      "노드(a): 소켓 'zz' 을(를) 찾을 수 없습니다 (엣지 e1)",
      '엣지 e2 타입 불일치 (PDF ← LIST[PDF])',
      "노드(b): 입력 'md' 에 엣지가 여러 개 연결되었습니다",
      '사이클: 노드(a) → 노드(b) → 노드(c)',
      '출력 노드가 없습니다 — 인스펙터에서 출력 노드를 지정하세요',
      "노드(c): 입력 'pdf' 이(가) 연결되지 않았습니다",
      '노드(d): 파라미터 오류 — index 는 0 이상의 정수여야 합니다',
    ])
    for (const line of ALL.map((i) => formatIssue(i, label))) expect(line).not.toContain('\n')
  })
})

describe('issueNodeIds', () => {
  it('collects nodeId and nodeIds, ignoring issues without nodes', () => {
    expect([...issueNodeIds(ALL)].sort()).toEqual(['a', 'b', 'c', 'd', 'n9'])
  })

  it('returns an empty set for no issues', () => {
    expect(issueNodeIds([]).size).toBe(0)
  })
})
