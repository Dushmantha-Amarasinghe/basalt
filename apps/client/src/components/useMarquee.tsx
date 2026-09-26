import { useCallback, useEffect, useRef, useState } from 'react'

/**
 * Selecting by dragging a box, as in Windows Explorer.
 *
 * Press on empty space and drag: everything the box touches is selected, and
 * the list scrolls when the pointer nears its top or bottom edge. Ctrl adds
 * to what was already selected. A click on empty space without a drag clears
 * the selection, which is the other half of how Explorer behaves and was
 * missing entirely — the only way to deselect was Escape.
 *
 * **What the box covers is worked out, not measured.** Every view here is a
 * virtual list of fixed-size rows, so an item's position is arithmetic on its
 * index. Asking the page where each item is would mean it had to exist, and
 * in a virtual list most of them do not — nor should they, in a folder of a
 * hundred thousand files.
 */

/** Where items sit inside the scrolling element, in pixels. */
export interface MarqueeGrid {
  /** From one row's top to the next. */
  rowStride: number
  /** How tall an item is within its row. */
  itemHeight: number
  columns: number
  /** From one column's left to the next. */
  colStride: number
  itemWidth: number
  /** Where the first column starts. */
  left: number
}

/** Indices of the items a box touches. Pure; exported for tests. */
export function itemsInBox(
  box: { x0: number; y0: number; x1: number; y1: number },
  grid: MarqueeGrid,
  count: number,
): number[] {
  const top = Math.min(box.y0, box.y1)
  const bottom = Math.max(box.y0, box.y1)
  const leftEdge = Math.min(box.x0, box.x1)
  const rightEdge = Math.max(box.x0, box.x1)

  const firstRow = Math.max(0, Math.floor(top / grid.rowStride))
  const lastRow = Math.floor(bottom / grid.rowStride)
  const hits: number[] = []
  for (let row = firstRow; row <= lastRow; row++) {
    const rowTop = row * grid.rowStride
    // Inside a row's stride but below its item: the gap between rows.
    if (bottom < rowTop || top > rowTop + grid.itemHeight) continue
    for (let col = 0; col < grid.columns; col++) {
      const itemLeft = grid.left + col * grid.colStride
      if (rightEdge < itemLeft || leftEdge > itemLeft + grid.itemWidth) continue
      const index = row * grid.columns + col
      if (index < count) hits.push(index)
    }
  }
  return hits
}

const EDGE = 36
const SCROLL_SPEED = 14
/** Moves smaller than this are a click, not a drag. */
const SLOP = 4

export function useMarquee({
  grid,
  count,
  idAt,
  selected,
  onChange,
}: {
  grid: MarqueeGrid
  count: number
  idAt: (index: number) => string | undefined
  selected: Set<string>
  onChange: (ids: Set<string>) => void
}): {
  onPointerDown: (e: React.PointerEvent<HTMLElement>) => void
  box: React.CSSProperties | null
} {
  const [box, setBox] = useState<React.CSSProperties | null>(null)
  const state = useRef<{
    scroller: HTMLElement
    container: HTMLElement
    /** The press, in the scroller's content coordinates. */
    start: { x: number; y: number }
    pointer: { x: number; y: number }
    base: Set<string>
    moved: boolean
    frame: number
  } | null>(null)

  const latest = useRef({ grid, count, idAt, onChange })
  latest.current = { grid, count, idAt, onChange }

  const update = useCallback(() => {
    const s = state.current
    if (!s) return
    const { grid: g, count: n, idAt: id, onChange: change } = latest.current
    const scrollerBox = s.scroller.getBoundingClientRect()
    const containerBox = s.container.getBoundingClientRect()
    const here = {
      x: s.pointer.x - scrollerBox.left + s.scroller.scrollLeft,
      y: s.pointer.y - scrollerBox.top + s.scroller.scrollTop,
    }
    if (!s.moved && Math.hypot(here.x - s.start.x, here.y - s.start.y) < SLOP) return
    s.moved = true

    const hits = itemsInBox({ x0: s.start.x, y0: s.start.y, x1: here.x, y1: here.y }, g, n)
    const next = new Set(s.base)
    for (const index of hits) {
      const key = id(index)
      if (key) next.add(key)
    }
    change(next)

    // Drawn in the container's coordinates, clipped to the visible list.
    const toContainerY = (y: number): number =>
      Math.min(
        scrollerBox.bottom - containerBox.top,
        Math.max(scrollerBox.top - containerBox.top, y - s.scroller.scrollTop + scrollerBox.top - containerBox.top),
      )
    const x0 = s.start.x - s.scroller.scrollLeft + scrollerBox.left - containerBox.left
    const x1 = here.x - s.scroller.scrollLeft + scrollerBox.left - containerBox.left
    const y0 = toContainerY(s.start.y)
    const y1 = toContainerY(here.y)
    setBox({
      left: Math.min(x0, x1),
      top: Math.min(y0, y1),
      width: Math.abs(x1 - x0),
      height: Math.abs(y1 - y0),
    })
  }, [])

  // Scrolls while the pointer is held near an edge, and keeps the box and
  // the selection following the content as it moves.
  const tick = useCallback(() => {
    const s = state.current
    if (!s) return
    const rect = s.scroller.getBoundingClientRect()
    if (s.pointer.y < rect.top + EDGE) s.scroller.scrollTop -= SCROLL_SPEED
    else if (s.pointer.y > rect.bottom - EDGE) s.scroller.scrollTop += SCROLL_SPEED
    update()
    s.frame = requestAnimationFrame(tick)
  }, [update])

  const end = useCallback(
    (cancelled: boolean) => {
      const s = state.current
      if (!s) return
      cancelAnimationFrame(s.frame)
      state.current = null
      setBox(null)
      if (cancelled) latest.current.onChange(s.base)
      // A click on empty space, not a drag: that clears the selection —
      // unless Ctrl was held, which means "keep what I have".
      else if (!s.moved && s.base.size === 0) latest.current.onChange(new Set())
    },
    [],
  )

  useEffect(() => {
    const move = (e: PointerEvent): void => {
      if (!state.current) return
      state.current.pointer = { x: e.clientX, y: e.clientY }
    }
    const up = (): void => end(false)
    const key = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && state.current) {
        e.stopPropagation()
        end(true)
      }
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('keydown', key, true)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('keydown', key, true)
    }
  }, [end])

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLElement>) => {
      if (e.button !== 0) return
      const target = e.target as HTMLElement
      // On an item, the item handles it: selecting, opening, dragging it.
      if (target.closest('[data-entry], button, input, a')) return
      const container = e.currentTarget
      const scroller = container.querySelector<HTMLElement>('.marquee-scroll')
      if (!scroller) return
      const rect = scroller.getBoundingClientRect()
      // Only inside the list itself; a column header is not empty space.
      if (e.clientY < rect.top || e.clientY > rect.bottom) return
      // Nor the scrollbar.
      if (e.clientX > rect.left + scroller.clientWidth) return

      e.preventDefault()
      const additive = e.ctrlKey || e.metaKey
      state.current = {
        scroller,
        container,
        start: {
          x: e.clientX - rect.left + scroller.scrollLeft,
          y: e.clientY - rect.top + scroller.scrollTop,
        },
        pointer: { x: e.clientX, y: e.clientY },
        base: additive ? new Set(selected) : new Set(),
        moved: false,
        frame: requestAnimationFrame(tick),
      }
    },
    [selected, tick],
  )

  return { onPointerDown, box }
}

/** The box itself: a thin outline over a faint fill, like Explorer's. */
export function MarqueeBox({ style }: { style: React.CSSProperties | null }): React.JSX.Element | null {
  if (!style) return null
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute z-20 rounded-[3px] border border-white/45 bg-white/[0.07]"
      style={style}
    />
  )
}
