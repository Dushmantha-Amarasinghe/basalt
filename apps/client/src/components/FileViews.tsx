import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { VList } from 'virtua'
import { Folder } from 'lucide-react'
import type { Entry } from './FileList'
import { iconFor } from './FileList'
import { cn, formatBytes } from '@/lib/utils'

/**
 * Tiles and List, the two Explorer-style alternatives to Details.
 *
 * Both stay virtualised, which is the whole trick: a grid is chunked into rows
 * of N items and the *rows* are virtualised, so 100,000 files cost the same as
 * 100 regardless of view. The column count is derived from the measured
 * container width rather than a media query, so it reflows correctly inside a
 * resizable window.
 */

const TILE_WIDTH = 132
const TILE_HEIGHT = 108
const LIST_COLUMN_WIDTH = 232
const LIST_ROW_HEIGHT = 28

/** Measures a container, so column counts follow the real width. */
function useContainerWidth(): [React.RefObject<HTMLDivElement | null>, number] {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)

  useEffect(() => {
    const el = ref.current
    if (!el) return undefined
    const observer = new ResizeObserver((entries) => {
      const next = entries[0]?.contentRect.width ?? 0
      // Only commit whole pixels; sub-pixel resize noise would re-render the
      // whole grid for no visible change.
      setWidth((prev) => (Math.abs(prev - next) > 1 ? next : prev))
    })
    observer.observe(el)
    setWidth(el.clientWidth)
    return () => observer.disconnect()
  }, [])

  return [ref, width]
}

/** Chunks a flat list into fixed-size rows. */
function rowCountFor(total: number, perRow: number): number {
  return perRow > 0 ? Math.ceil(total / perRow) : 0
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

const Tile = memo(function Tile({
  entry,
  selected,
  onSelect,
  onOpen,
}: {
  entry: Entry
  selected: boolean
  onSelect: (id: string, additive: boolean) => void
  onOpen: (entry: Entry) => void
}): React.JSX.Element {
  const Icon = iconFor(entry)
  return (
    <button
      onClick={(e) => onSelect(entry.id, e.ctrlKey || e.metaKey)}
      onDoubleClick={() => onOpen(entry)}
      style={{ width: TILE_WIDTH, height: TILE_HEIGHT }}
      className={cn(
        'row-contain flex flex-col items-center justify-center gap-2 rounded-md px-2 text-center transition-colors',
        selected
          ? 'bg-basalt/[0.09] ring-1 ring-inset ring-basalt/20'
          : 'hover:bg-white/[0.035]',
      )}
    >
      <Icon
        size={30}
        strokeWidth={1.3}
        className={cn(
          entry.kind === 'dir' ? 'text-basaltDeep' : 'text-textFaint',
          selected && 'text-basalt',
        )}
      />
      <span
        className={cn(
          'line-clamp-2 w-full break-all text-[11px] leading-tight',
          selected ? 'text-text' : 'text-textDim',
        )}
      >
        {entry.name}
      </span>
      <span className="tnum font-mono text-[9px] text-textFaint">
        {entry.kind === 'dir' ? '—' : formatBytes(entry.size)}
      </span>
    </button>
  )
})

export function TileView({
  entries,
  selected,
  onSelect,
  onOpen,
}: {
  entries: Entry[]
  selected: Set<string>
  onSelect: (id: string, additive: boolean) => void
  onOpen: (entry: Entry) => void
}): React.JSX.Element {
  const [ref, width] = useContainerWidth()
  const perRow = Math.max(1, Math.floor((width - 16) / TILE_WIDTH))
  const rows = rowCountFor(entries.length, perRow)

  const renderRow = useCallback(
    (rowIndex: number) => {
      const start = rowIndex * perRow
      const slice = entries.slice(start, start + perRow)
      return (
        <div className="flex gap-1 px-2" style={{ height: TILE_HEIGHT + 8 }}>
          {slice.map((entry) => (
            <Tile
              key={entry.id}
              entry={entry}
              selected={selected.has(entry.id)}
              onSelect={onSelect}
              onOpen={onOpen}
            />
          ))}
        </div>
      )
    },
    [entries, perRow, selected, onSelect, onOpen],
  )

  return (
    <div ref={ref} className="h-full pb-2 pt-2">
      {width > 0 && (
        <VList style={{ height: '100%' }} count={rows} itemSize={TILE_HEIGHT + 8} overscan={3}>
          {renderRow}
        </VList>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// List — compact, wrapping into columns the way Explorer's List view does
// ---------------------------------------------------------------------------

const ListCell = memo(function ListCell({
  entry,
  selected,
  onSelect,
  onOpen,
}: {
  entry: Entry
  selected: boolean
  onSelect: (id: string, additive: boolean) => void
  onOpen: (entry: Entry) => void
}): React.JSX.Element {
  const Icon = iconFor(entry)
  return (
    <button
      onClick={(e) => onSelect(entry.id, e.ctrlKey || e.metaKey)}
      onDoubleClick={() => onOpen(entry)}
      style={{ width: LIST_COLUMN_WIDTH, height: LIST_ROW_HEIGHT }}
      className={cn(
        'row-contain flex items-center gap-2 rounded px-2 text-left text-[12px] transition-colors',
        selected
          ? 'bg-basalt/[0.09] text-text ring-1 ring-inset ring-basalt/20'
          : 'text-textDim hover:bg-white/[0.035] hover:text-text',
      )}
    >
      <Icon
        size={14}
        className={cn(
          'shrink-0',
          entry.kind === 'dir' ? 'text-basaltDeep' : 'text-textFaint',
          selected && 'text-basalt',
        )}
      />
      <span className="truncate">{entry.name}</span>
    </button>
  )
})

export function ListView({
  entries,
  selected,
  onSelect,
  onOpen,
}: {
  entries: Entry[]
  selected: Set<string>
  onSelect: (id: string, additive: boolean) => void
  onOpen: (entry: Entry) => void
}): React.JSX.Element {
  const [ref, width] = useContainerWidth()
  const perRow = Math.max(1, Math.floor((width - 16) / LIST_COLUMN_WIDTH))
  const rows = rowCountFor(entries.length, perRow)

  const renderRow = useCallback(
    (rowIndex: number) => {
      const start = rowIndex * perRow
      const slice = entries.slice(start, start + perRow)
      return (
        <div className="flex gap-1 px-2" style={{ height: LIST_ROW_HEIGHT + 2 }}>
          {slice.map((entry) => (
            <ListCell
              key={entry.id}
              entry={entry}
              selected={selected.has(entry.id)}
              onSelect={onSelect}
              onOpen={onOpen}
            />
          ))}
        </div>
      )
    },
    [entries, perRow, selected, onSelect, onOpen],
  )

  return (
    <div ref={ref} className="h-full pb-2 pt-2">
      {width > 0 && (
        <VList
          style={{ height: '100%' }}
          count={rows}
          itemSize={LIST_ROW_HEIGHT + 2}
          overscan={6}
        >
          {renderRow}
        </VList>
      )}
    </div>
  )
}

/** Shared empty state, so all three views agree on what "nothing here" looks like. */
export function EmptyState({ label = 'Nothing here' }: { label?: string }): React.JSX.Element {
  return (
    <div className="flex h-full items-center justify-center">
      <div className="text-center">
        <Folder size={32} className="mx-auto text-textFaint/40" />
        <p className="mt-3 text-sm text-textFaint">{label}</p>
      </div>
    </div>
  )
}
