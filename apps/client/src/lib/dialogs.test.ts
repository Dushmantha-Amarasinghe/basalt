import { afterEach, describe, expect, it } from 'vitest'
import { folderUnder } from './dialogs'

/**
 * A stand-in for the bits of the DOM `folderUnder` actually touches.
 *
 * These tests run without a DOM on purpose — everything else under test in
 * this app is pure logic — and this function is small enough that faking
 * `elementFromPoint` covers it honestly. What is being checked is the lookup
 * rule, not that browsers implement `closest`.
 */
function stubDocument(at: Record<string, string | null>): void {
  const element = (dir: string | null): unknown => ({
    closest: (selector: string) =>
      selector === '[data-drop-dir]' && dir !== null
        ? { getAttribute: () => dir }
        : null,
  })
  ;(globalThis as { document?: unknown }).document = {
    elementFromPoint: (x: number, y: number) => {
      const found = at[`${x},${y}`]
      return found === undefined ? null : element(found)
    },
  }
}

afterEach(() => {
  delete (globalThis as { document?: unknown }).document
})

describe('folderUnder', () => {
  it('finds the folder a drag is hovering', () => {
    stubDocument({ '100,200': 'Films/Season 1' })
    expect(folderUnder(100, 200)).toBe('Films/Season 1')
  })

  it('reads the folder off an ancestor row, not just the exact element', () => {
    // The pointer lands on the icon or the label inside the row, never on the
    // row itself, so anything that only checked the topmost element would
    // never find a target at all.
    stubDocument({ '10,20': 'Movies' })
    expect(folderUnder(10, 20)).toBe('Movies')
  })

  it('is empty over a file, or over nothing', () => {
    stubDocument({ '5,5': null })
    expect(folderUnder(5, 5)).toBeNull()
    // Nowhere near anything.
    expect(folderUnder(999, 999)).toBeNull()
  })

  it('treats a missing position as no target rather than as the origin', () => {
    // Tauri omits the position on some events. Letting -1 through would hit
    // whatever happens to sit in the top-left corner of the window, which is
    // an upload into a folder nobody pointed at.
    stubDocument({ '-1,-1': 'Wrong' })
    expect(folderUnder(-1, -1)).toBeNull()
  })
})
