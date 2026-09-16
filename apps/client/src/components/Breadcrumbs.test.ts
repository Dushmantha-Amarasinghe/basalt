import { describe, expect, it } from 'vitest'
import { visibleSegments } from './Breadcrumbs'

describe('visibleSegments', () => {
  it('shows every segment while the trail is short', () => {
    expect(visibleSegments(1)).toEqual([0])
    expect(visibleSegments(2)).toEqual([0, 1])
    expect(visibleSegments(3)).toEqual([0, 1, 2])
    expect(visibleSegments(4)).toEqual([0, 1, 2, 3])
  })

  it('collapses the middle once the trail gets long', () => {
    expect(visibleSegments(5)).toEqual([0, null, 3, 4])
    expect(visibleSegments(12)).toEqual([0, null, 10, 11])
  })

  // The point of collapsing: width stops growing with depth, so the toolbar
  // cannot be pushed apart by a deep path.
  it('never shows more than four positions however deep the path goes', () => {
    for (const depth of [5, 8, 20, 100]) {
      expect(visibleSegments(depth), `depth ${depth}`).toHaveLength(4)
    }
  })

  it('always keeps the root and the current folder', () => {
    for (const depth of [1, 4, 5, 30]) {
      const shown = visibleSegments(depth)
      expect(shown[0], `depth ${depth}`).toBe(0)
      expect(shown[shown.length - 1], `depth ${depth}`).toBe(depth - 1)
    }
  })

  it('leaves no gap in the indices it does show', () => {
    const shown = visibleSegments(9).filter((i): i is number => i !== null)
    expect(shown).toEqual([...shown].sort((a, b) => a - b))
    expect(new Set(shown).size).toBe(shown.length)
  })
})
