import { LayoutGrid, LayoutList, Rows3 } from 'lucide-react'
import { Menu } from './ui/Menu'
import { cn } from '@/lib/utils'

export type ViewMode = 'details' | 'tiles' | 'list'

export const VIEW_OPTIONS: {
  mode: ViewMode
  label: string
  hint: string
  icon: typeof Rows3
}[] = [
  { mode: 'details', label: 'Details', hint: 'Name, size and date', icon: Rows3 },
  { mode: 'tiles', label: 'Tiles', hint: 'Large icons in a grid', icon: LayoutGrid },
  { mode: 'list', label: 'List', hint: 'Compact, wraps into columns', icon: LayoutList },
]

/**
 * View switcher, modelled on Windows Explorer's.
 *
 * Three modes rather than Explorer's eight: Details for working, Tiles for
 * media, List for scanning a lot of names at once. The rest of Explorer's
 * options are variations on icon size and earn their keep only in a file
 * manager that has to be everything to everyone.
 *
 * Portalled to `<body>` for the same reason the settings menus are — the
 * toolbar sits inside a clipping container.
 */
export function ViewMenu({
  mode,
  onChange,
}: {
  mode: ViewMode
  onChange: (mode: ViewMode) => void
}): React.JSX.Element {
  const current = VIEW_OPTIONS.find((o) => o.mode === mode) ?? VIEW_OPTIONS[0]!
  const CurrentIcon = current.icon

  return (
    <Menu<ViewMode>
      items={VIEW_OPTIONS.map((o) => ({
        value: o.mode,
        label: o.label,
        hint: o.hint,
        icon: o.icon,
      }))}
      value={mode}
      onSelect={onChange}
      trigger={({ open, onClick, ref }) => (
        <button
          ref={ref}
          onClick={onClick}
          aria-label="Change view"
          title="Change view"
          className={cn(
            'no-drag flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-[12px] transition-colors',
            open
              ? 'border-white/20 bg-white/[0.04] text-text'
              : 'border-transparent text-textDim hover:bg-white/[0.05] hover:text-text',
          )}
        >
          <CurrentIcon size={14} />
          <span>{current.label}</span>
        </button>
      )}
    />
  )
}
