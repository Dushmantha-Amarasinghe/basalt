import { memo, useCallback } from 'react'
import { VList } from 'virtua'
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
  Star,
  Video,
} from 'lucide-react'
import { cn, formatBytes, formatDate } from '@/lib/utils'

export interface Entry {
  id: string
  name: string
  kind: 'dir' | 'file'
  size: number
  modified: number
}

const ROW_HEIGHT = 34

/** Maps an extension to an icon. Cheap lookup, called once per visible row. */
function iconFor(entry: Entry): typeof FileIcon {
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
    <div
      role="row"
      aria-selected={selected}
      onClick={(e) => onSelect(entry.id, e.ctrlKey || e.metaKey)}
      onDoubleClick={() => onOpen(entry)}
      style={{ height: ROW_HEIGHT }}
      className={cn(
        'row-contain group flex cursor-default items-center gap-3 rounded-md px-3 text-sm',
        // Colour only. Animating anything here would cost frames during scroll.
        selected
          ? 'bg-basalt/[0.09] text-text ring-1 ring-inset ring-basalt/20'
          : 'text-textDim hover:bg-white/[0.035] hover:text-text',
      )}
    >
      <Icon
        size={16}
        className={cn(
          'shrink-0',
          entry.kind === 'dir' ? 'text-basaltDeep' : 'text-textFaint',
          selected && 'text-basalt',
        )}
      />

      <span className="min-w-0 flex-1 truncate">{entry.name}</span>

      {/*
        Quick actions appear on hover, in the space the metadata occupies. CSS
        only, no React state and no Framer Motion: this has to be free during a
        fast scroll, and a hover handler that sets state would re-render rows
        under the cursor at whatever rate the mouse moves.
      */}
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
        <QuickAction icon={Download} label="Download" />
        <QuickAction icon={Star} label="Star" />
        <QuickAction icon={MoreHorizontal} label="More" />
      </span>

      <span className="tnum w-20 shrink-0 text-right font-mono text-[11px] text-textFaint group-hover:opacity-0">
        {entry.kind === 'dir' ? '—' : formatBytes(entry.size)}
      </span>

      <span className="tnum w-28 shrink-0 text-right font-mono text-[11px] text-textFaint">
        {formatDate(entry.modified)}
      </span>
    </div>
  )
})

function QuickAction({
  icon: Icon,
  label,
}: {
  icon: typeof FileIcon
  label: string
}): React.JSX.Element {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={(e) => e.stopPropagation()}
      className="flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.07] hover:text-text"
    >
      <Icon size={13} />
    </button>
  )
}

export function FileList({
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
  // Index-based, so Virtua can create elements lazily. Passing
  // `entries.map(...)` built 100,000 React elements on **every** render even
  // though only ~37 were ever mounted — the single largest cost in the view.
  const renderRow = useCallback(
    (index: number) => {
      const entry = entries[index]
      if (!entry) return <div style={{ height: ROW_HEIGHT }} />
      return (
        <Row
          entry={entry}
          selected={selected.has(entry.id)}
          onSelect={onSelect}
          onOpen={onOpen}
        />
      )
    },
    [entries, selected, onSelect, onOpen],
  )

  if (entries.length === 0) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center">
          <Folder size={32} className="mx-auto text-textFaint/40" />
          <p className="mt-3 text-sm text-textFaint">Nothing here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full px-2 pb-2" role="grid">
      <ColumnHeader />
      {/*
        Virtua renders only the visible window. `count` plus a render function
        means elements are built lazily, and `itemSize` tells it the rows are a
        fixed height so it never has to measure them — both matter far more at
        100,000 rows than the virtualisation itself.
      */}
      <VList
        style={{ height: 'calc(100% - 28px)' }}
        count={entries.length}
        itemSize={ROW_HEIGHT}
        overscan={6}
      >
        {renderRow}
      </VList>
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
