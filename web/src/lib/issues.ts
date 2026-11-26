import type { ValidationIssue } from '@/bindings/ValidationIssue'
import { formatDataType } from '@/lib/socketColors.ts'

/** One-line Korean description of a validation issue (CONTEXT §4.1.2). */
export function formatIssue(issue: ValidationIssue, nodeLabel: (id: string) => string): string {
  switch (issue.code) {
    case 'DUPLICATE_ID':
      return `ID 중복: ${issue.id}`
    case 'UNKNOWN_NODE':
      return `엣지 ${issue.edgeId} 가 존재하지 않는 노드 ${issue.nodeId} 를 참조합니다`
    case 'UNKNOWN_SOCKET':
      return `${nodeLabel(issue.nodeId)}: 소켓 '${issue.socketId}' 을(를) 찾을 수 없습니다 (엣지 ${issue.edgeId})`
    case 'TYPE_MISMATCH':
      return `엣지 ${issue.edgeId} 타입 불일치 (${formatDataType(issue.expected)} ← ${formatDataType(issue.actual)})`
    case 'MULTIPLE_INCOMING':
      return `${nodeLabel(issue.nodeId)}: 입력 '${issue.socketId}' 에 엣지가 여러 개 연결되었습니다`
    case 'CYCLE_DETECTED':
      return `사이클: ${issue.nodeIds.map(nodeLabel).join(' → ')}`
    case 'NO_OUTPUT_NODE':
      return '출력 노드가 없습니다 — 인스펙터에서 출력 노드를 지정하세요'
    case 'UNCONNECTED_INPUT':
      return `${nodeLabel(issue.nodeId)}: 입력 '${issue.socketId}' 이(가) 연결되지 않았습니다`
    case 'INVALID_PARAMS':
      return `${nodeLabel(issue.nodeId)}: 파라미터 오류 — ${issue.message}`
  }
}

/** Ids of the nodes an issue list points at. */
export function issueNodeIds(issues: ValidationIssue[]): Set<string> {
  const ids = new Set<string>()
  for (const issue of issues) {
    if ('nodeId' in issue) ids.add(issue.nodeId)
    if ('nodeIds' in issue) for (const id of issue.nodeIds) ids.add(id)
  }
  return ids
}
