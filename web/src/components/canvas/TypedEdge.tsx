import { BaseEdge, getBezierPath, type EdgeProps } from '@xyflow/react'
import { edgeStrokeWidth, socketColor } from '@/lib/socketColors.ts'
import type { TypedFlowEdge } from '@/lib/graphMapping.ts'

export default function TypedEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
}: EdgeProps<TypedFlowEdge>) {
  const [path] = getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition })
  const dt = data?.dataType
  const width = dt ? edgeStrokeWidth(dt) : 2
  return (
    <BaseEdge
      id={id}
      path={path}
      style={{
        stroke: dt ? socketColor(dt) : '#94a3b8',
        strokeWidth: selected ? width + 1.5 : width,
        opacity: selected ? 1 : 0.85,
      }}
    />
  )
}
