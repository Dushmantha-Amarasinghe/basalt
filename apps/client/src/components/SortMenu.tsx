import {
  ArrowDownAZ,
  ArrowUpDown,
  CalendarClock,
  FileType2,
  HardDrive,
  SortAsc,
  SortDesc,
} from 'lucide-react'
import type { Entry } from './FileList'
import { Menu } from './ui/Menu'
import { cn } from '@/lib/utils'

export type SortField = 'name' | 'size' | 'modified' | 'type'
export type SortDirection = 'asc' | 'desc'

/** Everything the menu can pick, fields and directions in one list. */
type SortChoice = SortField | SortDirection

const ITEMS = [
  { value: 'name' as const, label: 'Name', icon: ArrowDownAZ },
  { value: 'modified' as const, label: 'Date modified', icon: CalendarClock },
  { value: 'type' as const, label: 'Type', icon: FileType2 },
  { value: 'size' as const, label: 'Size', icon: HardDrive },
  { value: 'asc' as const, label: 'Ascending', icon: SortAsc },
  { value: 'desc' as const, label: 'Descending', icon: SortDesc },
]

const FIELD_LABELS: Record<SortField, string> = {
  name: 'Name',
  size: 'Size',
  modified: 'Date',
  type: 'Type',
}

export function SortMenu({
  field,
  direction,
  onFieldChange,
  onDirectionChange,
}: {
  field: SortField
  direction: SortDirection
  onFieldChange: (field: SortField) => void
  onDirectionChange: (direction: SortDirection) => void
}): React.JSX.Element {
  return (
    <Menu<SortChoice>
      items={ITEMS}
      value={[field, direction]}
      // A separator between the fields and the directions, matching the shape
      // of Explorer's own sort menu.
      groups={[4]}
      onSelect={(choice) => {
        if (choice === 'asc' || choice === 'desc') onDirectionChange(choice)
        else onFieldChange(choice)
      }}
      trigger={({ open, onClick, ref }) => (
        <button
          ref={ref}
          onClick={onClick}
          aria-label="Sort"
          title="Sort"
          className={cn(
            'no-drag flex h-8 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-[12px] transition-colors',
            open
              ? 'border-white/20 bg-white/[0.04] text-text'
              : 'border-transparent text-textDim hover:bg-white/[0.05] hover:text-text',
          )}
        >
          <ArrowUpDown size={14} />
          <span>{FIELD_LABELS[field]}</span>
        </button>
      )}
    />
  )
}

/**
 * Sorts entries, with folders pinned above files.
 *
 * Folders first is the behaviour every file manager has, and breaking it makes
 * a directory feel shuffled even when the sort is correct. `localeCompare` with
 * `numeric` means `file2` lands before `file10` rather than after it.
 */
export function sortEntries(
  entries: Entry[],
  field: SortField,
  direction: SortDirection,
): Entry[] {
  const sign = direction === 'asc' ? 1 : -1

  const extension = (name: string): string => {
    const dot = name.lastIndexOf('.')
    return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
  }

  const byName = (a: Entry, b: Entry): number =>
    a.name.localeCompare(b.name, undefined, { numeric: true })

  // Copy first: the caller's array is memoised upstream and must not be
  // mutated, or a re-sort would silently corrupt the source list.
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === 'dir' ? -1 : 1

    // Every field falls back to the name, so ties have one defined order.
    // Without it, sorting by size leaves folders — which are all size 0 —
    // in whatever order the listing arrived in, and the folder block appears
    // to shuffle itself whenever you change the sort.
    let result: number
    switch (field) {
      case 'size':
        result = a.size - b.size || byName(a, b)
        break
      case 'modified':
        result = a.modified - b.modified || byName(a, b)
        break
      case 'type':
        result = extension(a.name).localeCompare(extension(b.name)) || byName(a, b)
        break
      default:
        result = byName(a, b)
    }
    return result * sign
  })
}
