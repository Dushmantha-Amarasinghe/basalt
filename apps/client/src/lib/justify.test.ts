import { describe, expect, it } from 'vitest'
import { clampAspect, justify } from './justify'

describe('justify', () => {
  it('fills every full row to the width exactly', () => {
    const aspects = [4 / 3, 3 / 4, 16 / 9, 1, 4 / 3, 3 / 2, 2 / 3, 4 / 3, 1, 16 / 9]
    const width = 900
    const gap = 4
    const rows = justify(aspects, width, 180, gap)

    for (const row of rows.slice(0, -1)) {
      const used =
        aspects
          .slice(row.start, row.start + row.count)
          .reduce((sum, a) => sum + clampAspect(a) * row.height, 0) +
        gap * (row.count - 1)
      expect(used).toBeCloseTo(width, 6)
    }
  })

  it('keeps every full row near the height asked for', () => {
    const mixed = Array.from({ length: 200 }, (_, i) => [4 / 3, 3 / 4, 16 / 9, 1, 2 / 3][i % 5]!)
    const rows = justify(mixed, 1180, 200, 4)
    for (const row of rows.slice(0, -1)) {
      expect(row.height).toBeGreaterThan(200 * 0.7)
      expect(row.height).toBeLessThan(200 * 1.4)
    }
  })

  it('places every photo once, in order', () => {
    const aspects = Array.from({ length: 57 }, (_, i) => (i % 3 === 0 ? 0.75 : 1.5))
    const rows = justify(aspects, 1200, 190, 6)
    let next = 0
    for (const row of rows) {
      expect(row.start).toBe(next)
      expect(row.count).toBeGreaterThan(0)
      next += row.count
    }
    expect(next).toBe(aspects.length)
  })

  it('leaves the last row at the target instead of stretching it', () => {
    const rows = justify([1.5, 1.5, 1.5, 1.5, 1.5], 1000, 200, 4)
    const last = rows[rows.length - 1]!
    expect(last.height).toBe(200)
    expect(last.count).toBe(2)
  })

  it('keeps a panorama and a tall screenshot from wrecking a row', () => {
    expect(clampAspect(12)).toBe(3)
    expect(clampAspect(0.1)).toBe(0.5)
    // Unknown shapes are laid out as an ordinary photo.
    expect(clampAspect(Number.NaN)).toBeCloseTo(4 / 3)
    expect(clampAspect(0)).toBeCloseTo(4 / 3)
  })

  it('lays out nothing into no space', () => {
    expect(justify([1, 1], 0, 200, 4)).toEqual([])
    expect(justify([], 800, 200, 4)).toEqual([])
  })
})
