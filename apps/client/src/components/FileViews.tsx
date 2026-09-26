import { memo, useCallback, useEffect, useRef, useState } from 'react'
import { VList } from 'virtua'
import { Folder } from 'lucide-react'
import type { Entry, RowHandlers } from './FileList'
import { DRAG_MIME, draggedPaths, iconFor } from './FileList'
import { cn, formatBytes } from '@/lib/utils'
import { MarqueeBox, useMarquee } from './useMarquee'

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
  cut,
  dropHighlight,
  handlers,
}: {
  entry: Entry
  selected: boolean
  cut: boolean
  /** True while files dragged in from outside are hovering this folder. */
  dropHighlight: boolean
  handlers: RowHandlers
}): React.JSX.Element {
  const Icon = iconFor(entry)
  const isDir = entry.kind === 'dir'
  return (
    <button
      draggable
      // Advertises this row as a drop destination for files dragged in
      // from outside. The position of an external drag arrives as a bare
      // coordinate, so the hit test reads it back out of the document.
      data-drop-dir={isDir ? entry.id : undefined}
      onClick={(e) =>
        handlers.onSelect(entry.id, {
          additive: e.ctrlKey || e.metaKey,
          range: e.shiftKey,
        })
      }
      onDoubleClick={() => handlers.onOpen(entry)}
      onContextMenu={(e) => {
        e.preventDefault()
        handlers.onContextMenu(entry, e)
      }}
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_MIME, JSON.stringify(handlers.onDragStart(entry)))
        e.dataTransfer.effectAllowed = 'move'
      }}
      onDragOver={
        isDir
          ? (e) => {
              if (!e.dataTransfer.types.includes(DRAG_MIME)) return
              e.preventDefault()
              e.dataTransfer.dropEffect = 'move'
            }
          : undefined
      }
      onDrop={
        isDir
          ? (e) => {
              const paths = draggedPaths(e.dataTransfer)
              if (paths.length === 0) return
              e.preventDefault()
              e.stopPropagation()
              handlers.onDropInto(entry, paths)
            }
          : undefined
      }
      data-entry=""
      style={{ width: TILE_WIDTH, height: TILE_HEIGHT }}
      className={cn(
        'row-contain flex flex-col items-center justify-center gap-2 rounded-md px-2 text-center transition-colors',
        selected
          ? 'bg-basalt/[0.09] ring-1 ring-inset ring-basalt/20'
          : 'hover:bg-white/[0.035]',
        cut && 'opacity-45',
        dropHighlight && 'bg-basalt/[0.14] ring-1 ring-inset ring-basalt/45',
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
  cutPaths,
  dropHighlight,
  handlers,
  onBackgroundContextMenu,
}: {
  entries: Entry[]
  selected: Set<string>
  cutPaths?: Set<string>
  /** Vault path of the folder an external drag is hovering, if any. */
  dropHighlight?: string | null
  handlers: RowHandlers
  onBackgroundContextMenu?: (event: { clientX: number; clientY: number }) => void
}): React.JSX.Element {
  const [ref, width] = useContainerWidth()
  const perRow = Math.max(1, Math.floor((width - 16) / TILE_WIDTH))
  const rows = rowCountFor(entries.length, perRow)
  const marquee = useMarquee({
    grid: {
      rowStride: TILE_HEIGHT + 8,
      itemHeight: TILE_HEIGHT,
      columns: perRow,
      colStride: TILE_WIDTH + 4,
      itemWidth: TILE_WIDTH,
      left: 8,
    },
    count: entries.length,
    idAt: (index) => entries[index]?.id,
    selected,
    onChange: handlers.onSelectSet,
  })

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
              cut={cutPaths?.has(entry.id) ?? false}
              dropHighlight={dropHighlight === entry.id}
              handlers={handlers}
            />
          ))}
        </div>
      )
    },
    [entries, perRow, selected, cutPaths, handlers],
  )

  return (
    <div
      ref={ref}
      className="relative h-full pb-2 pt-2"
      onPointerDown={marquee.onPointerDown}
      onContextMenu={(e) => {
        if (e.defaultPrevented) return
        e.preventDefault()
        onBackgroundContextMenu?.(e)
      }}
    >
      {width > 0 && (
        <VList
          className="marquee-scroll"
          style={{ height: '100%' }}
          count={rows}
          itemSize={TILE_HEIGHT + 8}
          overscan={3}
        >
          {renderRow}
        </VList>
      )}
      <MarqueeBox style={marquee.box} />
    </div>
  )
}

// ---------------------------------------------------------------------------
// List — compact, wrapping into columns the way Explorer's List view does
// ---------------------------------------------------------------------------

const ListCell = memo(function ListCell({
  entry,
  selected,
  cut,
  dropHighlight,
  handlers,
}: {
  entry: Entry
  selected: boolean
  cut: boolean
  /** True while files dragged in from outside are hovering this folder. */
  dropHighlight: boolean
  handlers: RowHandlers
}): React.JSX.Element {
  const Icon = iconFor(entry)
  const isDir = entry.kind === 'dir'
  return (
    <button
      draggable
      // Advertises this row as a drop destination for files dragged in
      // from outside. The position of an external drag arrives as a bare
      // coordinate, so the hit test reads it back out of the document.
      data-drop-dir={isDir ? entry.id : undefined}
      onClick={(e) =>
        handlers.onSelect(entry.id, {
          additive: e.ctrlKey || e.metaKey,
          range: e.shiftKey,
        })
      }
      onDoubleClick={() => handlers.onOpen(entry)}
      onContextMenu={(e) => {
        e.preventDefault()
        handlers.onContextMenu(entry, e)
      }}
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_MIME, JSON.stringify(handlers.onDragStart(entry)))
        e.dataTransfer.effectAllowed = 'move'
      }}
      onDragOver={
        isDir
          ? (e) => {
              if (!e.dataTransfer.types.includes(DRAG_MIME)) return
              e.preventDefault()
              e.dataTransfer.dropEffect = 'move'
            }
          : undefined
      }
      onDrop={
        isDir
          ? (e) => {
              const paths = draggedPaths(e.dataTransfer)
              if (paths.length === 0) return
              e.preventDefault()
              e.stopPropagation()
              handlers.onDropInto(entry, paths)
            }
          : undefined
      }
      data-entry=""
      style={{ width: LIST_COLUMN_WIDTH, height: LIST_ROW_HEIGHT }}
      className={cn(
        'row-contain flex items-center gap-2 rounded px-2 text-left text-[12px] transition-colors',
        selected
          ? 'bg-basalt/[0.09] text-text ring-1 ring-inset ring-basalt/20'
          : 'text-textDim hover:bg-white/[0.035] hover:text-text',
        cut && 'opacity-45',
        dropHighlight && 'bg-basalt/[0.14] ring-1 ring-inset ring-basalt/45',
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
  cutPaths,
  dropHighlight,
  handlers,
  onBackgroundContextMenu,
}: {
  entries: Entry[]
  selected: Set<string>
  cutPaths?: Set<string>
  /** Vault path of the folder an external drag is hovering, if any. */
  dropHighlight?: string | null
  handlers: RowHandlers
  onBackgroundContextMenu?: (event: { clientX: number; clientY: number }) => void
}): React.JSX.Element {
  const [ref, width] = useContainerWidth()
  const perRow = Math.max(1, Math.floor((width - 16) / LIST_COLUMN_WIDTH))
  const rows = rowCountFor(entries.length, perRow)
  const marquee = useMarquee({
    grid: {
      rowStride: LIST_ROW_HEIGHT + 2,
      itemHeight: LIST_ROW_HEIGHT,
      columns: perRow,
      colStride: LIST_COLUMN_WIDTH + 4,
      itemWidth: LIST_COLUMN_WIDTH,
      left: 8,
    },
    count: entries.length,
    idAt: (index) => entries[index]?.id,
    selected,
    onChange: handlers.onSelectSet,
  })

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
              cut={cutPaths?.has(entry.id) ?? false}
              dropHighlight={dropHighlight === entry.id}
              handlers={handlers}
            />
          ))}
        </div>
      )
    },
    [entries, perRow, selected, cutPaths, handlers],
  )

  return (
    <div
      ref={ref}
      className="relative h-full pb-2 pt-2"
      onPointerDown={marquee.onPointerDown}
      onContextMenu={(e) => {
        if (e.defaultPrevented) return
        e.preventDefault()
        onBackgroundContextMenu?.(e)
      }}
    >
      {width > 0 && (
        <VList
          className="marquee-scroll"
          style={{ height: '100%' }}
          count={rows}
          itemSize={LIST_ROW_HEIGHT + 2}
          overscan={6}
        >
          {renderRow}
        </VList>
      )}
      <MarqueeBox style={marquee.box} />
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
