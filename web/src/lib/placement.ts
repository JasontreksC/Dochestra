/** Pure helpers for placing new nodes on the canvas without overlapping existing ones. */

export interface XY {
  x: number
  y: number
}

export interface Size {
  width: number
  height: number
}

export interface Box extends XY, Size {}

/** Fallback node size (flow units) when a node has not been measured yet. */
export const DEFAULT_NODE_SIZE: Size = { width: 260, height: 120 }
/** Diagonal shift applied until a candidate position is free. */
export const PLACEMENT_STEP = 40
const MAX_TRIES = 500

interface NodeLike {
  position: XY
  measured?: { width?: number; height?: number }
  width?: number
  height?: number
}

/** Bounding box of a node in flow coordinates, using measured size when available. */
export function nodeBox(n: NodeLike): Box {
  return {
    x: n.position.x,
    y: n.position.y,
    width: n.measured?.width ?? n.width ?? DEFAULT_NODE_SIZE.width,
    height: n.measured?.height ?? n.height ?? DEFAULT_NODE_SIZE.height,
  }
}

/** Strict overlap: boxes that only touch at an edge do not overlap. */
export function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/** True when `box` lies entirely inside `area`. */
export function contains(area: Box, box: Box): boolean {
  return (
    box.x >= area.x &&
    box.y >= area.y &&
    box.x + box.width <= area.x + area.width &&
    box.y + box.height <= area.y + area.height
  )
}

/**
 * Returns the first position, starting at `start` and shifting by `step` on both axes, where a
 * node of `size` does not overlap any of `existing`. Deterministic for the same inputs.
 */
export function findFreePosition(
  start: XY,
  existing: readonly Box[],
  size: Size = DEFAULT_NODE_SIZE,
  step: number = PLACEMENT_STEP,
): XY {
  const pos = { x: start.x, y: start.y }
  for (let i = 0; i < MAX_TRIES; i++) {
    const candidate = { ...pos, ...size }
    if (!existing.some((b) => overlaps(candidate, b))) return pos
    pos.x += step
    pos.y += step
  }
  return pos
}

/** `DataTransfer` type carrying a node kind dragged from the palette onto the canvas. */
export const NODE_DRAG_MIME = 'application/dochestra-node'
