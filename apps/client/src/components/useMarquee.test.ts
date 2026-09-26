import { describe, expect, it } from 'vitest'
import { itemsInBox, type MarqueeGrid } from './useMarquee'

/** Details view: one full-width column of 34-pixel rows. */
const details: MarqueeGrid = {
  rowStride: 34,
  itemHeight: 34,
  columns: 1,
  colStride: 0,
  itemWidth: 10_000,
  left: 0,
}

/** Tiles: 132-pixel tiles 4 apart, rows of 108 with 8 between them. */
const tiles: MarqueeGrid = {
  rowStride: 116,
  itemHeight: 108,
  columns: 5,
  colStride: 136,
  itemWidth: 132,
  left: 8,
}

describe('itemsInBox', () => {
  it('takes every row the box crosses, whichever way it was dragged', () => {
    expect(itemsInBox({ x0: 10, y0: 40, x1: 200, y1: 150 }, details, 100)).toEqual([1, 2, 3, 4])
    expect(itemsInBox({ x0: 200, y0: 150, x1: 10, y1: 40 }, details, 100)).toEqual([1, 2, 3, 4])
  })

  it('never names an item past the end', () => {
    expect(itemsInBox({ x0: 0, y0: 0, x1: 50, y1: 10_000 }, details, 3)).toEqual([0, 1, 2])
  })

  it('takes only the tiles the box touches', () => {
    // Across the second and third tiles of the first two rows.
    const hits = itemsInBox({ x0: 150, y0: 20, x1: 300, y1: 130 }, tiles, 50)
    expect(hits).toEqual([1, 2, 6, 7])
  })

  it('touches nothing in the gaps', () => {
    // Between the first and second tile, inside a row.
    expect(itemsInBox({ x0: 141, y0: 20, x1: 143, y1: 30 }, tiles, 50)).toEqual([])
    // Between two rows.
    expect(itemsInBox({ x0: 20, y0: 110, x1: 60, y1: 114 }, tiles, 50)).toEqual([])
  })
})
