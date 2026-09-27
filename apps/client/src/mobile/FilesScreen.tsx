import { useCallback, useEffect, useRef, useState } from 'react'
import { VList, type VListHandle } from 'virtua'
import { Loader2, MoreVertical, Star } from 'lucide-react'
import type { AppModel } from '@/App'
import { iconFor, type Entry } from '@/components/FileList'
import { isKind } from '@/lib/useLibrary'
import { thumbUrl } from '@/lib/thumbs'
import { cn, formatBytes } from '@/lib/utils'
import { android } from '@/lib/android'
import { useScrollMemory } from '@/lib/useScrollMemory'

const ROW = 64

/**
 * A folder on the drive, as a phone shows one.
 *
 * Tap to open; long-press to start choosing several, then tap to add or take
 * away; the ⋮ on each row is the same menu a right-click gives on the
 * desktop. Photos and videos show a picture of themselves.
 */
export function FilesScreen({
  model,
  entries,
  onActions,
  selecting,
  emptyLabel,
  showPath,
  scrollKey,
}: {
  model: AppModel
  entries: Entry[]
  /** The menu for one entry. */
  onActions: (entry: Entry) => void
  selecting: boolean
  emptyLabel: string
  /** Under each name, the folder it is in — for Recent and Starred. */
  showPath?: boolean
  /** What is on show, so the list keeps its place per folder; see `useScrollMemory`. */
  scrollKey: string
}): React.JSX.Element {
  const { selected, setSelected, openEntry, mediaBase, cutPaths, stars } = model

  const toggle = useCallback(
    (entry: Entry) => {
      setSelected((prev) => {
        const next = new Set(prev)
        if (next.has(entry.id)) next.delete(entry.id)
        else next.add(entry.id)
        return next
      })
    },
    [setSelected],
  )

  const onTap = useCallback(
    (entry: Entry) => {
      if (selecting) {
        toggle(entry)
        return
      }
      // A file the app cannot show opens its menu, rather than starting a
      // download nobody asked for with one tap.
      const opens =
        entry.kind === 'dir' ||
        isKind(entry.name, 'photos') ||
        isKind(entry.name, 'videos') ||
        isKind(entry.name, 'music')
      if (opens) openEntry(entry)
      else onActions(entry)
    },
    [selecting, toggle, openEntry, onActions],
  )

  const onLong = useCallback(
    (entry: Entry) => {
      void android.haptic('long')
      toggle(entry)
    },
    [toggle],
  )

  const scroll = useScrollMemory(scrollKey)
  const pull = usePullToRefresh(model.vault.refresh, scroll.handle)

  if (entries.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center px-10 text-center">
        {model.vault.loading ? (
          <Loader2 size={20} className="animate-spin text-textFaint" />
        ) : (
          <span className="text-[14px] text-textFaint">{emptyLabel}</span>
        )}
      </div>
    )
  }

  return (
    <div className="relative h-full" {...pull.handlers}>
      <PullIndicator distance={pull.distance} refreshing={pull.refreshing} />
      <VList
        ref={scroll.ref}
        onScroll={scroll.onScroll}
        style={{ height: '100%', paddingBottom: 96 }}
        count={entries.length}
        itemSize={ROW}
        overscan={8}
      >
        {(index) => {
          const entry = entries[index]!
          return (
            <Row
              key={entry.id}
              entry={entry}
              base={mediaBase}
              selected={selected.has(entry.id)}
              selecting={selecting}
              cut={cutPaths?.has(entry.id) ?? false}
              starred={stars.isStarred(entry.id)}
              showPath={showPath}
              onTap={onTap}
              onLong={onLong}
              onMenu={onActions}
            />
          )
        }}
      </VList>
    </div>
  )
}

function Row({
  entry,
  base,
  selected,
  selecting,
  cut,
  starred,
  showPath,
  onTap,
  onLong,
  onMenu,
}: {
  entry: Entry
  base: string
  selected: boolean
  selecting: boolean
  cut: boolean
  starred: boolean
  showPath?: boolean
  onTap: (entry: Entry) => void
  onLong: (entry: Entry) => void
  onMenu: (entry: Entry) => void
}): React.JSX.Element {
  const press = useLongPress(
    () => onLong(entry),
    () => onTap(entry),
  )
  const Icon = iconFor(entry)
  const pictured =
    entry.kind === 'file' && (isKind(entry.name, 'photos') || isKind(entry.name, 'videos'))
  const folder = showPath ? entry.id.split('/').slice(0, -1).join('/') || 'Top of the drive' : ''

  return (
    <div
      {...press}
      role="row"
      aria-selected={selected}
      style={{ height: ROW }}
      className={cn(
        'flex select-none items-center gap-3.5 px-4 transition-colors duration-100',
        selected ? 'bg-white/[0.08]' : 'active:bg-white/[0.04]',
        cut && 'opacity-45',
      )}
    >
      <div className="relative flex h-11 w-11 shrink-0 items-center justify-center overflow-hidden rounded-[10px] bg-white/[0.05]">
        {pictured && base ? (
          <Thumbnail src={thumbUrl(base, entry.id, Math.floor(entry.modified / 1000))} Icon={Icon} />
        ) : (
          <Icon size={21} className={entry.kind === 'dir' ? 'text-basaltDeep' : 'text-textDim'} />
        )}
        {selecting && (
          <span
            className={cn(
              'absolute inset-0 flex items-center justify-center transition-colors',
              selected ? 'bg-basalt/85' : 'bg-black/35',
            )}
          >
            <span
              className={cn(
                'h-5 w-5 rounded-full border-2',
                selected ? 'border-ink bg-ink' : 'border-white/80',
              )}
            >
              {selected && (
                <svg viewBox="0 0 20 20" className="h-full w-full text-basalt">
                  <path d="M5 10.5l3.2 3.2L15 7" fill="none" stroke="currentColor" strokeWidth="2.2" />
                </svg>
              )}
            </span>
          </span>
        )}
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[15px] text-text">{entry.name}</span>
          {starred && <Star size={12} className="shrink-0 fill-basalt text-basalt" />}
        </div>
        <div className="mt-0.5 truncate font-mono text-[11px] text-textFaint">
          {showPath
            ? folder
            : [entry.kind === 'dir' ? 'Folder' : formatBytes(entry.size), shortDate(entry.modified)]
                .filter(Boolean)
                .join(' · ')}
        </div>
      </div>

      {!selecting && (
        <button
          aria-label={`More for ${entry.name}`}
          onClick={(e) => {
            e.stopPropagation()
            onMenu(entry)
          }}
          onPointerDown={(e) => e.stopPropagation()}
          className="-mr-2 flex h-11 w-11 items-center justify-center rounded-full text-textFaint active:bg-white/[0.08]"
        >
          <MoreVertical size={19} />
        </button>
      )}
    </div>
  )
}

function Thumbnail({
  src,
  Icon,
}: {
  src: string
  Icon: React.ComponentType<{ size?: number; className?: string }>
}): React.JSX.Element {
  const [state, setState] = useState<'loading' | 'ok' | 'none'>('loading')
  return (
    <>
      {state !== 'ok' && <Icon size={20} className="text-textDim" />}
      {state !== 'none' && (
        <img
          src={src}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={() => setState('ok')}
          onError={() => setState('none')}
          className={cn(
            'absolute inset-0 h-full w-full object-cover transition-opacity duration-200',
            state === 'ok' ? 'opacity-100' : 'opacity-0',
          )}
        />
      )}
    </>
  )
}

function shortDate(ms: number): string {
  if (!ms) return ''
  const date = new Date(ms)
  const now = new Date()
  const sameYear = date.getFullYear() === now.getFullYear()
  return date.toLocaleDateString(undefined, {
    day: 'numeric',
    month: 'short',
    ...(sameYear ? {} : { year: 'numeric' }),
  })
}

/**
 * A press that is either a tap or, held, a long press — never both, and
 * neither if the finger moved to scroll.
 */
export function useLongPress(
  onLong: () => void,
  onTap: () => void,
): {
  onPointerDown: (e: React.PointerEvent) => void
  onPointerUp: () => void
  onPointerMove: (e: React.PointerEvent) => void
  onPointerCancel: () => void
  onContextMenu: (e: React.MouseEvent) => void
} {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const start = useRef<{ x: number; y: number } | null>(null)
  const fired = useRef(false)

  const clear = (): void => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
  }

  return {
    onPointerDown: (e) => {
      if (e.button !== 0) return
      fired.current = false
      start.current = { x: e.clientX, y: e.clientY }
      clear()
      timer.current = setTimeout(() => {
        fired.current = true
        timer.current = null
        onLong()
      }, 450)
    },
    onPointerMove: (e) => {
      const s = start.current
      if (s && Math.hypot(e.clientX - s.x, e.clientY - s.y) > 10) {
        clear()
        start.current = null
      }
    },
    onPointerUp: () => {
      const wasTap = timer.current !== null && !fired.current && start.current !== null
      clear()
      start.current = null
      if (wasTap) onTap()
    },
    onPointerCancel: () => {
      clear()
      start.current = null
    },
    // A long press also fires the browser's context menu; this is ours.
    onContextMenu: (e) => e.preventDefault(),
  }
}

/** Pull down at the top of a list to refresh it. */
function usePullToRefresh(
  refresh: () => void,
  listRef: React.RefObject<VListHandle | null>,
): {
  handlers: {
    onTouchStart: (e: React.TouchEvent) => void
    onTouchMove: (e: React.TouchEvent) => void
    onTouchEnd: () => void
  }
  distance: number
  refreshing: boolean
} {
  const from = useRef<number | null>(null)
  const [distance, setDistance] = useState(0)
  const [refreshing, setRefreshing] = useState(false)

  useEffect(() => {
    if (!refreshing) return undefined
    const timer = setTimeout(() => setRefreshing(false), 900)
    return () => clearTimeout(timer)
  }, [refreshing])

  return {
    distance,
    refreshing,
    handlers: {
      onTouchStart: (e) => {
        const atTop = (listRef.current?.scrollOffset ?? 0) <= 0
        from.current = atTop ? e.touches[0]!.clientY : null
      },
      onTouchMove: (e) => {
        if (from.current === null) return
        const pulled = e.touches[0]!.clientY - from.current
        setDistance(pulled > 0 ? Math.min(110, pulled * 0.5) : 0)
      },
      onTouchEnd: () => {
        if (distance > 64) {
          setRefreshing(true)
          void android.haptic('confirm')
          refresh()
        }
        from.current = null
        setDistance(0)
      },
    },
  }
}

function PullIndicator({
  distance,
  refreshing,
}: {
  distance: number
  refreshing: boolean
}): React.JSX.Element | null {
  if (distance <= 0 && !refreshing) return null
  const shown = refreshing ? 48 : distance
  return (
    <div
      className="pointer-events-none absolute inset-x-0 top-0 z-10 flex justify-center"
      style={{ transform: `translateY(${shown - 36}px)`, opacity: refreshing ? 1 : Math.min(1, distance / 64) }}
    >
      <span className="flex h-9 w-9 items-center justify-center rounded-full border border-white/10 bg-[#1b1b1e] shadow-lift">
        <Loader2
          size={17}
          className={cn('text-text', refreshing && 'animate-spin')}
          style={refreshing ? undefined : { transform: `rotate(${distance * 4}deg)` }}
        />
      </span>
    </div>
  )
}
