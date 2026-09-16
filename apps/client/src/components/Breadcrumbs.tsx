import { ChevronRight } from 'lucide-react'
import { Menu } from './ui/Menu'
import { cn } from '@/lib/utils'

/**
 * How many segments stay visible before the middle collapses.
 *
 * Root plus the last two is what Explorer settles on, and it is the smallest
 * set that still answers both questions a path has to answer: where am I, and
 * what is one level up.
 */
const VISIBLE_TAIL = 2

/**
 * The path trail, which collapses instead of shrinking.
 *
 * Letting flexbox squeeze every segment equally looks fine at three levels and
 * unreadable at ten — a row of two-character stubs, none of which can be
 * identified. Collapsing the middle into a menu keeps each remaining segment at
 * full width no matter how deep the path goes, and the hidden levels are still
 * one click away.
 */
export function Breadcrumbs({
  path,
  onNavigateTo,
}: {
  path: string[]
  onNavigateTo: (index: number) => void
}): React.JSX.Element {
  const collapsed = path.length > VISIBLE_TAIL + 2
  const hidden = collapsed ? path.slice(1, path.length - VISIBLE_TAIL) : []
  const tail = collapsed ? path.slice(path.length - VISIBLE_TAIL) : path.slice(1)
  const tailOffset = path.length - tail.length

  return (
    <nav className="flex min-w-0 items-center gap-0.5 text-sm">
      {/*
        The root holds its width. It is short, it is the one label that is
        always the same, and watching "Vault" collapse to "Va…" while there is
        room to spare just looks broken.
      */}
      <Crumb
        label={path[0] ?? ''}
        current={path.length === 1}
        fixed
        onClick={() => onNavigateTo(0)}
      />

      {collapsed && (
        <>
          <Separator />
          <Menu<string>
            align="left"
            width={200}
            value=""
            items={hidden.map((segment, index) => ({
              // Index is part of the value because the same folder name can
              // legitimately appear twice in one path.
              value: `${index + 1}:${segment}`,
              label: segment,
            }))}
            onSelect={(value) => onNavigateTo(Number(value.split(':')[0]))}
            trigger={({ open, onClick, ref }) => (
              <button
                ref={ref}
                onClick={onClick}
                aria-label="Show hidden path segments"
                title={hidden.join(' / ')}
                className={cn(
                  'no-drag shrink-0 rounded px-1.5 py-0.5 leading-none transition-colors',
                  open ? 'bg-white/[0.06] text-text' : 'text-textFaint hover:bg-white/[0.04] hover:text-text',
                )}
              >
                …
              </button>
            )}
          />
        </>
      )}

      {tail.map((segment, index) => (
        <div key={`${segment}-${tailOffset + index}`} className="flex min-w-0 items-center">
          <Separator />
          <Crumb
            label={segment}
            current={index === tail.length - 1}
            onClick={() => onNavigateTo(tailOffset + index)}
          />
        </div>
      ))}
    </nav>
  )
}

function Separator(): React.JSX.Element {
  return <ChevronRight size={14} className="mx-0.5 shrink-0 text-textFaint" />
}

function Crumb({
  label,
  current,
  fixed,
  onClick,
}: {
  label: string
  current: boolean
  fixed?: boolean
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      title={label}
      className={cn(
        'no-drag truncate rounded px-1.5 py-0.5 transition-colors',
        fixed ? 'shrink-0' : 'min-w-[4ch]',
        current
          ? 'font-semibold text-text'
          : 'text-textDim hover:bg-white/[0.04] hover:text-text',
      )}
    >
      {label}
    </button>
  )
}

/**
 * The segments a trail of this length actually shows, exported for tests.
 * Returns path indices, in display order, with `null` for the collapsed gap.
 */
export function visibleSegments(length: number): (number | null)[] {
  if (length <= VISIBLE_TAIL + 2) {
    return Array.from({ length }, (_, i) => i)
  }
  return [
    0,
    null,
    ...Array.from({ length: VISIBLE_TAIL }, (_, i) => length - VISIBLE_TAIL + i),
  ]
}
