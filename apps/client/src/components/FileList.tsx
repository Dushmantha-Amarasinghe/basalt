import { memo, useCallback, useState } from 'react'
import { VList } from 'virtua'
import { MarqueeBox, useMarquee, type MarqueeGrid } from './useMarquee'
import {
  Download,
  File as FileIcon,
  FileArchive,
  FileCode,
  FileText,
  Folder,
  Image as ImageIcon,
  MoreHorizontal,
  Music,
  Video,
} from 'lucide-react'
import { cn, formatBytes, formatDate } from '@/lib/utils'
import { EmptyState } from './FileViews'

export interface Entry {
  id: string
  name: string
  kind: 'dir' | 'file'
  size: number
  modified: number
}

/**
 * Everything a row can do, as one object.
 *
 * Passed down whole rather than as separate props so the memoised rows keep
 * comparing equal: one stable object beats six callbacks that each have to be
 * individually memoised at the call site, and forgetting one of those would
 * silently re-render every visible row on every keystroke.
 */
export interface RowHandlers {
  onSelect: (id: string, modifiers: { additive: boolean; range: boolean }) => void
  onOpen: (entry: Entry) => void
  onContextMenu: (entry: Entry, event: { clientX: number; clientY: number }) => void
  onDownload: (entry: Entry) => void
  /** Drag started on a row. Returns the paths being dragged. */
  onDragStart: (entry: Entry) => string[]
  /** Something was dropped onto a folder row. */
  onDropInto: (entry: Entry, paths: string[]) => void
  /** The selection, replaced whole: by a drag box, or cleared by a click on
   *  empty space. */
  onSelectSet: (ids: Set<string>) => void
}

/** The drag payload type. Internal, so Explorer drops are not confused for it. */
export const DRAG_MIME = 'application/x-basalt-paths'

const ROW_HEIGHT = 34

/** Where rows sit, for the drag box: one full-width column. */
const DETAILS_GRID: MarqueeGrid = {
  rowStride: ROW_HEIGHT,
  itemHeight: ROW_HEIGHT,
  columns: 1,
  colStride: 0,
  itemWidth: 1_000_000,
  left: 0,
}

/** Maps an extension to an icon. Cheap lookup, called once per visible row. */
export function iconFor(entry: Entry): typeof FileIcon {
  if (entry.kind === 'dir') return Folder
  const ext = entry.name.split('.').pop()?.toLowerCase() ?? ''
  if (['mp4', 'mkv', 'avi', 'mov', 'webm'].includes(ext)) return Video
  if (['mp3', 'flac', 'wav', 'm4a', 'opus'].includes(ext)) return Music
  if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic'].includes(ext)) return ImageIcon
  if (['zip', 'rar', '7z', 'tar', 'gz'].includes(ext)) return FileArchive
  if (['rs', 'ts', 'tsx', 'js', 'py', 'go', 'json', 'toml'].includes(ext)) return FileCode
  if (['txt', 'md', 'pdf', 'doc', 'docx'].includes(ext)) return FileText
  return FileIcon
}

/** Reads dragged vault paths out of a drop event, if they are ours. */
export function draggedPaths(transfer: DataTransfer | null): string[] {
  if (!transfer) return []
  try {
    const raw = transfer.getData(DRAG_MIME)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter((p) => typeof p === 'string') : []
  } catch {
    return []
  }
}

/**
 * One row.
 *
 * Memoised, and deliberately free of layout-triggering work. The acceptance
 * criterion for this list is 60 fps while scrolling 100,000 rows, and the way
 * that budget gets spent is a row component that re-renders or measures itself
 * on every frame.
 */
const Row = memo(function Row({
  entry,
  selected,
  cut,
  dropTarget,
  handlers,
}: {
  entry: Entry
  selected: boolean
  /** Dimmed because it is on the clipboard waiting to be moved. */
  cut: boolean
  dropTarget: boolean
  handlers: RowHandlers
}): React.JSX.Element {
  const Icon = iconFor(entry)
  const isDir = entry.kind === 'dir'

  return (
    <div
      role="row"
      aria-selected={selected}
      data-entry=""
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
        const paths = handlers.onDragStart(entry)
        e.dataTransfer.setData(DRAG_MIME, JSON.stringify(paths))
        e.dataTransfer.effectAllowed = 'move'
      }}
      // Only folders accept a drop, and only from inside the app.
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
      style={{ height: ROW_HEIGHT }}
      className={cn(
        // The transparent border above and below, with the fill and outline
        // kept inside it, is a 2px gap between rows — selected neighbours
        // used to draw their outlines on top of each other. The row keeps its
        // full height, so there is no dead strip between rows to click into.
        'row-contain group flex cursor-default items-center gap-3 rounded-lg border-y border-transparent bg-clip-padding px-3 text-sm',
        // Colour only, and briefly. Animating anything else here would cost
        // frames during a scroll.
        'transition-colors duration-100',
        selected
          ? 'bg-white/[0.075] text-text ring-1 ring-inset ring-white/[0.12]'
          : 'text-textDim hover:bg-white/[0.035] hover:text-text',
        dropTarget && 'bg-basalt/[0.14] ring-1 ring-inset ring-basalt/45',
        cut && 'opacity-45',
      )}
    >
      <Icon
        size={16}
        className={cn(
          'pointer-events-none shrink-0',
          isDir ? 'text-basaltDeep' : 'text-textFaint',
          selected && 'text-basalt',
        )}
      />

      <span className="pointer-events-none min-w-0 flex-1 truncate">{entry.name}</span>

      {/*
        Quick actions appear on hover, in the space the metadata occupies. CSS
        only, no React state and no Framer Motion: this has to be free during a
        fast scroll, and a hover handler that sets state would re-render rows
        under the cursor at whatever rate the mouse moves.
      */}
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
        {!isDir && (
          <QuickAction
            icon={Download}
            label="Download"
            onClick={() => handlers.onDownload(entry)}
          />
        )}
        <QuickAction
          icon={MoreHorizontal}
          label="More"
          onClick={(e) => handlers.onContextMenu(entry, e)}
        />
      </span>

      <span className="tnum pointer-events-none w-20 shrink-0 text-right font-mono text-[11px] text-textFaint group-hover:opacity-0">
        {isDir ? '—' : formatBytes(entry.size)}
      </span>

      <span className="tnum pointer-events-none w-28 shrink-0 text-right font-mono text-[11px] text-textFaint">
        {formatDate(entry.modified)}
      </span>
    </div>
  )
})

function QuickAction({
  icon: Icon,
  label,
  onClick,
}: {
  icon: typeof FileIcon
  label: string
  onClick: (event: { clientX: number; clientY: number }) => void
}): React.JSX.Element {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation()
        onClick(e)
      }}
      className="flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.07] hover:text-text"
    >
      <Icon size={13} />
    </button>
  )
}

export function FileList({
  entries,
  selected,
  cutPaths,
  dropHighlight,
  handlers,
  onBackgroundContextMenu,
}: {
  entries: Entry[]
  selected: Set<string>
  /** Paths on the clipboard awaiting a move, drawn dimmed. */
  cutPaths?: Set<string>
  /** Vault path of the folder an external drag is hovering, if any. */
  dropHighlight?: string | null
  handlers: RowHandlers
  /** Right-click on empty space, for New folder / Paste. */
  onBackgroundContextMenu?: (event: { clientX: number; clientY: number }) => void
}): React.JSX.Element {
  const [dropTarget, setDropTarget] = useState<string | null>(null)

  // Index-based, so Virtua can create elements lazily. Passing
  // `entries.map(...)` built 100,000 React elements on **every** render even
  // though only ~37 were ever mounted — the single largest cost in the view.
  const renderRow = useCallback(
    (index: number) => {
      const entry = entries[index]
      if (!entry) return <div style={{ height: ROW_HEIGHT }} />
      return (
        <div
          onDragEnter={
            entry.kind === 'dir' ? () => setDropTarget(entry.id) : undefined
          }
          onDragLeave={
            entry.kind === 'dir'
              ? () => setDropTarget((id) => (id === entry.id ? null : id))
              : undefined
          }
          onDrop={() => setDropTarget(null)}
        >
          <Row
            entry={entry}
            selected={selected.has(entry.id)}
            cut={cutPaths?.has(entry.id) ?? false}
            dropTarget={dropTarget === entry.id || dropHighlight === entry.id}
            handlers={handlers}
          />
        </div>
      )
    },
    [entries, selected, cutPaths, dropTarget, dropHighlight, handlers],
  )

  const marquee = useMarquee({
    grid: DETAILS_GRID,
    count: entries.length,
    idAt: (index) => entries[index]?.id,
    selected,
    onChange: handlers.onSelectSet,
  })

  if (entries.length === 0) return <EmptyState />

  return (
    <div
      className="relative h-full px-2 pb-2"
      role="grid"
      onPointerDown={marquee.onPointerDown}
      onContextMenu={(e) => {
        // Only when the click missed every row; a row handles its own and
        // stops this from firing by preventing the default first.
        if (e.defaultPrevented) return
        e.preventDefault()
        onBackgroundContextMenu?.(e)
      }}
    >
      <ColumnHeader />
      {/*
        Virtua renders only the visible window. `count` plus a render function
        means elements are built lazily, and `itemSize` tells it the rows are a
        fixed height so it never has to measure them — both matter far more at
        100,000 rows than the virtualisation itself.
      */}
      <VList
        className="marquee-scroll"
        style={{ height: 'calc(100% - 28px)' }}
        count={entries.length}
        itemSize={ROW_HEIGHT}
        overscan={6}
      >
        {renderRow}
      </VList>
      <MarqueeBox style={marquee.box} />
    </div>
  )
}

function ColumnHeader(): React.JSX.Element {
  return (
    <div className="flex h-7 items-center gap-3 border-b border-line px-3 font-mono text-[10px] uppercase tracking-[0.18em] text-textFaint">
      <span className="w-4 shrink-0" />
      <span className="min-w-0 flex-1">Name</span>
      <span className="w-20 shrink-0 text-right">Size</span>
      <span className="w-28 shrink-0 text-right">Modified</span>
    </div>
  )
}
