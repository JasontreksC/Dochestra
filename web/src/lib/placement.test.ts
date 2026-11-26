import { describe, expect, it } from 'vitest'
import {
  DEFAULT_NODE_SIZE,
  PLACEMENT_STEP,
  contains,
  findFreePosition,
  nodeBox,
  overlaps,
  type Box,
} from './placement.ts'

const box = (x: number, y: number, width = 260, height = 120): Box => ({ x, y, width, height })

describe('placement', () => {
  it('nodeBox prefers measured size, then width/height, then the default', () => {
    expect(nodeBox({ position: { x: 1, y: 2 }, measured: { width: 300, height: 200 } })).toEqual(box(1, 2, 300, 200))
    expect(nodeBox({ position: { x: 0, y: 0 }, width: 50, height: 60 })).toEqual(box(0, 0, 50, 60))
    expect(nodeBox({ position: { x: 0, y: 0 } })).toEqual(box(0, 0, DEFAULT_NODE_SIZE.width, DEFAULT_NODE_SIZE.height))
  })

  it('overlaps is strict (touching edges do not overlap)', () => {
    expect(overlaps(box(0, 0), box(100, 50))).toBe(true)
    expect(overlaps(box(0, 0), box(260, 0))).toBe(false)
    expect(overlaps(box(0, 0), box(0, 120))).toBe(false)
  })

  it('contains checks full containment', () => {
    expect(contains(box(0, 0, 1000, 1000), box(10, 10))).toBe(true)
    expect(contains(box(0, 0, 1000, 1000), box(900, 10))).toBe(false)
  })

  it('returns the start position when it is free', () => {
    expect(findFreePosition({ x: 5, y: 5 }, [box(1000, 1000)])).toEqual({ x: 5, y: 5 })
  })

  it('shifts diagonally until free and is deterministic', () => {
    const existing = [box(0, 0)]
    const a = findFreePosition({ x: 0, y: 0 }, existing)
    const b = findFreePosition({ x: 0, y: 0 }, existing)
    expect(a).toEqual(b)
    expect(a.x % PLACEMENT_STEP).toBe(0)
    expect(overlaps({ ...a, ...DEFAULT_NODE_SIZE }, existing[0]!)).toBe(false)
  })

  it('placing many nodes in a row never produces overlaps', () => {
    const placed: Box[] = []
    for (let i = 0; i < 10; i++) {
      const p = findFreePosition({ x: 100, y: 100 }, placed)
      const b = { ...p, ...DEFAULT_NODE_SIZE }
      for (const other of placed) expect(overlaps(b, other)).toBe(false)
      placed.push(b)
    }
    expect(new Set(placed.map((p) => `${p.x},${p.y}`)).size).toBe(10)
  })
})
