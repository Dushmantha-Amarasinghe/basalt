import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { ChevronLeft, ChevronRight, Download, Info, Minus, Plus, X } from 'lucide-react'
import type { MediaFile } from '@/lib/api'
import { folderOf, formatOf, stemOf } from '@/lib/mediaInfo'
import { GRID_THUMB, VIEW_THUMB, fileUrl, isDisplayable, thumbUrl } from '@/lib/thumbs'
import { cn, formatBytes } from '@/lib/utils'
import { isMobileShell } from '@/lib/platform'

/** Fingers rather than a pointer; see `Stage` for the gestures. */
const TOUCH = isMobileShell()

/**
 * The photo viewer: one photo, as large as the window allows, on black.
 *
 * It opened with a coloured placeholder in a framed box, three buttons that
 * did nothing, and a filmstrip of coloured squares. Now the photo fills the
 * space it has, and every control does what it says:
 *
 * - The grid's thumbnail is shown at once, and the full photo replaces it the
 *   moment it arrives — so moving through a folder is never a blank screen.
 * - A photo the window cannot show itself (HEIC from a phone, TIFF) is shown
 *   as a large JPEG the host makes, rather than not at all.
 * - The wheel zooms around the pointer, a double-click zooms in and out, and
 *   a zoomed photo is dragged to look around it.
 * - On a phone: pinch to zoom, swipe to the next photo, swipe down to put it
 *   away, tap to hide everything but the photo, double tap to zoom.
 */
export function ImageViewer({
  photos,
  index,
  base,
  onIndexChange,
  onClose,
  onDownload,
}: {
  photos: MediaFile[]
  index: number | null
  /** The start of every media URL; see `useMediaBase`. */
  base: string
  onIndexChange: (index: number) => void
  onClose: () => void
  onDownload: (path: string) => void
}): React.JSX.Element {
  const photo = index !== null ? photos[index] : undefined
  const [info, setInfo] = useState(false)
  /** The bars over the photo; a tap puts them away on a phone. */
  const [chrome, setChrome] = useState(true)
  const narrow = TOUCH && typeof window !== 'undefined' && window.innerWidth < 600

  const go = useCallback(
    (step: number) => {
      if (index === null) return
      const next = index + step
      if (next >= 0 && next < photos.length) onIndexChange(next)
    },
    [index, photos.length, onIndexChange],
  )

  // The two either side, fetched ahead, so the next photo is already there.
  useEffect(() => {
    if (index === null) return
    for (const near of [photos[index + 1], photos[index - 1]]) {
      if (near) new Image().src = fullSource(base, near)
    }
  }, [index, photos, base])

  return (
    <AnimatePresence>
      {photo && index !== null && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-40 flex flex-col bg-[#050506]"
        >
          <div
            className={cn(
              'flex shrink-0 items-center gap-3 px-4',
              TOUCH
                ? 'absolute inset-x-0 top-0 z-20 bg-gradient-to-b from-black/80 to-transparent pb-6 transition-opacity duration-200'
                : 'drag h-12 border-b border-white/[0.05]',
              TOUCH && !chrome && 'pointer-events-none opacity-0',
            )}
            style={TOUCH ? { paddingTop: 'calc(var(--inset-top, 0px) + 6px)' } : undefined}
          >
            {TOUCH && <ToolButton icon={ChevronLeft} label="Back" onClick={onClose} />}
            <div className="min-w-0">
              <div className="truncate text-[13px] font-medium text-text">{stemOf(photo.path)}</div>
              {narrow && (
                <div className="tnum truncate font-mono text-[10.5px] text-textFaint">
                  {index + 1} / {photos.length} · {formatBytes(photo.size)}
                </div>
              )}
            </div>
            <span className={cn('shrink-0 font-mono text-[10.5px] text-textFaint', narrow && 'hidden')}>
              {formatOf(photo.path)} · {formatBytes(photo.size)}
            </span>
            <div className="flex-1" />
            <span className={cn('tnum shrink-0 font-mono text-[11px] text-textFaint', narrow && 'hidden')}>
              {index + 1} / {photos.length}
            </span>
            <div className="no-drag flex items-center gap-0.5">
              <ToolButton
                icon={Info}
                label="Details (i)"
                active={info}
                onClick={() => setInfo((v) => !v)}
              />
              <ToolButton
                icon={Download}
                label="Download"
                onClick={() => onDownload(photo.path)}
              />
              {!TOUCH && <ToolButton icon={X} label="Close (Esc)" onClick={onClose} />}
            </div>
          </div>

          <div className="relative flex min-h-0 flex-1">
            <Stage
              key={photo.path}
              photo={photo}
              base={base}
              onNext={() => go(1)}
              onPrevious={() => go(-1)}
              onClose={onClose}
              onToggleInfo={() => setInfo((v) => !v)}
              onTap={() => setChrome((v) => !v)}
            />

            {!TOUCH && index > 0 && <NavArrow side="left" onClick={() => go(-1)} />}
            {!TOUCH && index < photos.length - 1 && (
              <NavArrow side="right" onClick={() => go(1)} />
            )}

            <AnimatePresence initial={false}>
              {info && <Details photo={photo} />}
            </AnimatePresence>
          </div>

          {/* A phone's screen is the photo's; a tablet has room for the strip. */}
          {!narrow && (
            <div
              className={cn(
                TOUCH && 'absolute inset-x-0 bottom-0 z-20 bg-black/70 transition-opacity duration-200',
                TOUCH && !chrome && 'pointer-events-none opacity-0',
              )}
              style={TOUCH ? { paddingBottom: 'var(--inset-bottom, 0px)' } : undefined}
            >
              <Filmstrip photos={photos} index={index} base={base} onPick={onIndexChange} />
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  )
}

/** The photo itself, with zoom and pan. Remounted per photo, so each one
 *  starts fitted to the window. */
function Stage({
  photo,
  base,
  onNext,
  onPrevious,
  onClose,
  onToggleInfo,
  onTap,
}: {
  photo: MediaFile
  base: string
  onNext: () => void
  onPrevious: () => void
  onClose: () => void
  onToggleInfo: () => void
  /** A single tap on a touch screen. */
  onTap: () => void
}): React.JSX.Element {
  const [full, setFull] = useState(false)
  const [view, setView] = useState({ scale: 1, x: 0, y: 0 })
  const area = useRef<HTMLDivElement | null>(null)
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null)

  /** Zooms to `scale`, keeping the point under (cx, cy) where it is. */
  const zoomAt = useCallback((scale: number, cx: number, cy: number) => {
    setView((v) => {
      const next = Math.min(8, Math.max(1, scale))
      if (next === 1) return { scale: 1, x: 0, y: 0 }
      const rect = area.current?.getBoundingClientRect()
      if (!rect) return { ...v, scale: next }
      const px = cx - rect.left - rect.width / 2
      const py = cy - rect.top - rect.height / 2
      const k = next / v.scale
      return { scale: next, x: px - (px - v.x) * k, y: py - (py - v.y) * k }
    })
  }, [])

  const zoomBy = useCallback(
    (factor: number) => {
      const rect = area.current?.getBoundingClientRect()
      if (!rect) return
      zoomAt(view.scale * factor, rect.left + rect.width / 2, rect.top + rect.height / 2)
    },
    [zoomAt, view.scale],
  )

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      switch (e.key) {
        case 'Escape':
          onClose()
          break
        case 'ArrowRight':
          onNext()
          break
        case 'ArrowLeft':
          onPrevious()
          break
        case 'i':
          onToggleInfo()
          break
        case '+':
        case '=':
          zoomBy(1.5)
          break
        case '-':
          zoomBy(1 / 1.5)
          break
        case '0':
          setView({ scale: 1, x: 0, y: 0 })
          break
        default:
          return
      }
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, onNext, onPrevious, onToggleInfo, zoomBy])

  const zoomed = view.scale > 1
  const touch = useTouchGestures({ view, setView, area, zoomAt, onNext, onPrevious, onClose, onTap })
  const moving = drag.current !== null || touch.active

  return (
    <div
      ref={area}
      className={cn(
        'relative flex min-w-0 flex-1 items-center justify-center overflow-hidden',
        TOUCH ? 'touch-none' : 'p-6',
        !TOUCH && (zoomed ? 'cursor-grab active:cursor-grabbing' : 'cursor-zoom-in'),
      )}
      onWheel={(e) => zoomAt(view.scale * (e.deltaY < 0 ? 1.18 : 1 / 1.18), e.clientX, e.clientY)}
      onDoubleClick={
        TOUCH
          ? undefined
          : (e) => (zoomed ? setView({ scale: 1, x: 0, y: 0 }) : zoomAt(2.5, e.clientX, e.clientY))
      }
      {...(TOUCH
        ? touch.handlers
        : {
            onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
              if (!zoomed) return
              drag.current = { x: e.clientX, y: e.clientY, ox: view.x, oy: view.y }
              e.currentTarget.setPointerCapture(e.pointerId)
            },
            onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => {
              const d = drag.current
              if (!d) return
              setView((v) => ({ ...v, x: d.ox + e.clientX - d.x, y: d.oy + e.clientY - d.y }))
            },
            onPointerUp: () => {
              drag.current = null
            },
          })}
    >
      <div
        className="relative flex h-full w-full items-center justify-center"
        style={{
          transform: `translate(${view.x + touch.swipe.dx}px, ${view.y + touch.swipe.dy}px) scale(${view.scale})`,
          opacity: touch.swipe.dy > 0 ? Math.max(0.35, 1 - touch.swipe.dy / 500) : 1,
          transition: moving ? 'none' : 'transform 160ms cubic-bezier(0.22, 1, 0.36, 1)',
        }}
      >
        {/* The grid's picture first, instantly, then the photo over it. */}
        <img
          src={thumbUrl(base, photo.path, photo.mtime, GRID_THUMB)}
          alt=""
          aria-hidden="true"
          draggable={false}
          className={cn(
            'absolute max-h-full max-w-full object-contain transition-opacity duration-300',
            full ? 'opacity-0' : 'opacity-100 blur-[2px]',
          )}
          style={fitted(photo)}
        />
        <img
          src={fullSource(base, photo)}
          alt={stemOf(photo.path)}
          draggable={false}
          onLoad={() => setFull(true)}
          className={cn(
            'relative max-h-full max-w-full select-none object-contain transition-opacity duration-300',
            full ? 'opacity-100' : 'opacity-0',
          )}
          style={fitted(photo)}
        />
      </div>

      {zoomed && !TOUCH && (
        <div className="no-drag absolute bottom-4 left-1/2 flex -translate-x-1/2 items-center gap-1 rounded-full bg-black/70 px-1.5 py-1 backdrop-blur">
          <ToolButton icon={Minus} label="Zoom out (-)" onClick={() => zoomBy(1 / 1.5)} small />
          <span className="tnum w-11 text-center font-mono text-[10.5px] text-textDim">
            {Math.round(view.scale * 100)}%
          </span>
          <ToolButton icon={Plus} label="Zoom in (+)" onClick={() => zoomBy(1.5)} small />
        </div>
      )}
    </div>
  )
}

type View = { scale: number; x: number; y: number }

/**
 * A photo under fingers.
 *
 * Two fingers pinch, zooming around the point between them and following it
 * as it moves. One finger drags a zoomed photo about; on a photo at its
 * normal size it swipes sideways to the next or previous photo, or down to
 * put the viewer away, and the photo follows the finger until it is let go.
 * A tap hides or shows the bars; a double tap zooms in on that spot, or back
 * out.
 */
function useTouchGestures({
  view,
  setView,
  area,
  zoomAt,
  onNext,
  onPrevious,
  onClose,
  onTap,
}: {
  view: View
  setView: React.Dispatch<React.SetStateAction<View>>
  area: React.RefObject<HTMLDivElement | null>
  zoomAt: (scale: number, cx: number, cy: number) => void
  onNext: () => void
  onPrevious: () => void
  onClose: () => void
  onTap: () => void
}): {
  handlers: {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerCancel: (e: React.PointerEvent<HTMLDivElement>) => void
  }
  swipe: { dx: number; dy: number }
  active: boolean
} {
  const fingers = useRef(new Map<number, { x: number; y: number }>())
  const gesture = useRef<
    | { kind: 'pinch'; dist: number; mid: { x: number; y: number }; start: View }
    | { kind: 'pan'; x: number; y: number; start: View }
    | { kind: 'swipe'; x: number; y: number; axis: 'x' | 'y' | null }
    | null
  >(null)
  const tap = useRef<{ x: number; y: number; at: number } | null>(null)
  const lastTap = useRef(0)
  const tapTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [swipe, setSwipe] = useState({ dx: 0, dy: 0 })
  const [active, setActive] = useState(false)
  const latest = useRef(view)
  latest.current = view

  useEffect(
    () => () => {
      if (tapTimer.current) clearTimeout(tapTimer.current)
    },
    [],
  )

  const two = (): { dist: number; mid: { x: number; y: number } } => {
    const [a, b] = [...fingers.current.values()]
    return {
      dist: Math.hypot(a!.x - b!.x, a!.y - b!.y) || 1,
      mid: { x: (a!.x + b!.x) / 2, y: (a!.y + b!.y) / 2 },
    }
  }

  const begin = (): void => {
    const v = latest.current
    if (fingers.current.size >= 2) {
      gesture.current = { kind: 'pinch', ...two(), start: v }
      tap.current = null
      setSwipe({ dx: 0, dy: 0 })
      return
    }
    const [f] = [...fingers.current.values()]
    if (!f) {
      gesture.current = null
      return
    }
    gesture.current =
      v.scale > 1
        ? { kind: 'pan', x: f.x, y: f.y, start: v }
        : { kind: 'swipe', x: f.x, y: f.y, axis: null }
  }

  return {
    swipe,
    active,
    handlers: {
      onPointerDown: (e) => {
        e.currentTarget.setPointerCapture(e.pointerId)
        fingers.current.set(e.pointerId, { x: e.clientX, y: e.clientY })
        if (fingers.current.size === 1) tap.current = { x: e.clientX, y: e.clientY, at: performance.now() }
        setActive(true)
        begin()
      },
      onPointerMove: (e) => {
        if (!fingers.current.has(e.pointerId)) return
        fingers.current.set(e.pointerId, { x: e.clientX, y: e.clientY })
        const t = tap.current
        if (t && Math.hypot(e.clientX - t.x, e.clientY - t.y) > 10) tap.current = null

        const g = gesture.current
        if (!g) return
        if (g.kind === 'pinch' && fingers.current.size >= 2) {
          const { dist, mid } = two()
          const rect = area.current?.getBoundingClientRect()
          if (!rect) return
          const scale = Math.min(8, Math.max(1, (g.start.scale * dist) / g.dist))
          const k = scale / g.start.scale
          const px = g.mid.x - rect.left - rect.width / 2
          const py = g.mid.y - rect.top - rect.height / 2
          setView({
            scale,
            x: px - (px - g.start.x) * k + (mid.x - g.mid.x),
            y: py - (py - g.start.y) * k + (mid.y - g.mid.y),
          })
        } else if (g.kind === 'pan') {
          setView({ ...g.start, x: g.start.x + e.clientX - g.x, y: g.start.y + e.clientY - g.y })
        } else if (g.kind === 'swipe') {
          const dx = e.clientX - g.x
          const dy = e.clientY - g.y
          if (!g.axis && Math.hypot(dx, dy) > 10) g.axis = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y'
          if (g.axis === 'x') setSwipe({ dx, dy: 0 })
          else if (g.axis === 'y') setSwipe({ dx: 0, dy: Math.max(0, dy) })
        }
      },
      onPointerUp: (e) => {
        const g = gesture.current
        fingers.current.delete(e.pointerId)

        if (g?.kind === 'swipe') {
          if (g.axis === 'x' && Math.abs(swipe.dx) > 70) {
            if (swipe.dx < 0) onNext()
            else onPrevious()
          } else if (g.axis === 'y' && swipe.dy > 110) {
            onClose()
          }
          setSwipe({ dx: 0, dy: 0 })
        }
        if (g?.kind === 'pinch' && latest.current.scale < 1.05) setView({ scale: 1, x: 0, y: 0 })

        const t = tap.current
        tap.current = null
        if (fingers.current.size === 0 && t && performance.now() - t.at < 300) {
          const now = performance.now()
          if (now - lastTap.current < 300) {
            lastTap.current = 0
            if (tapTimer.current) clearTimeout(tapTimer.current)
            tapTimer.current = null
            if (latest.current.scale > 1) setView({ scale: 1, x: 0, y: 0 })
            else zoomAt(2.5, t.x, t.y)
          } else {
            lastTap.current = now
            tapTimer.current = setTimeout(() => {
              tapTimer.current = null
              onTap()
            }, 300)
          }
        }

        if (fingers.current.size === 0) {
          gesture.current = null
          setActive(false)
        } else {
          begin()
        }
      },
      onPointerCancel: (e) => {
        fingers.current.delete(e.pointerId)
        tap.current = null
        setSwipe({ dx: 0, dy: 0 })
        if (fingers.current.size === 0) {
          gesture.current = null
          setActive(false)
        } else {
          begin()
        }
      },
    },
  }
}

/** The photo itself, or the host's large JPEG of it when the window cannot
 *  show the format. */
function fullSource(base: string, photo: MediaFile): string {
  return isDisplayable(photo.path)
    ? fileUrl(base, photo.path)
    : thumbUrl(base, photo.path, photo.mtime, VIEW_THUMB)
}

/** Sized by its shape before it loads, so the placeholder and the photo sit
 *  exactly on top of each other. */
function fitted(photo: MediaFile): React.CSSProperties {
  return photo.width && photo.height ? { aspectRatio: `${photo.width} / ${photo.height}` } : {}
}

function Details({ photo }: { photo: MediaFile }): React.JSX.Element {
  const rows: Array<[string, string]> = [
    ['Name', photo.path.split('/').pop() ?? photo.path],
    ['Folder', folderOf(photo.path) || 'Top of the drive'],
    ['Size', formatBytes(photo.size)],
    ['Dimensions', photo.width && photo.height ? `${photo.width} × ${photo.height}` : '—'],
    ['Format', formatOf(photo.path)],
    [
      'Modified',
      new Date(photo.mtime * 1000).toLocaleString(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }),
    ],
  ]
  return (
    <motion.aside
      initial={{ opacity: 0, x: 16 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 16 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className={cn(
        'w-[272px] shrink-0 overflow-y-auto border-l border-white/[0.06] bg-[#0b0b0d] px-5 py-5',
        TOUCH && 'absolute inset-y-0 right-0 z-30 max-w-[85%] pt-20',
      )}
    >
      <div className="font-mono text-[9.5px] uppercase tracking-[0.16em] text-textFaint">Details</div>
      <dl className="mt-4 space-y-3.5">
        {rows.map(([label, value]) => (
          <div key={label}>
            <dt className="text-[10.5px] text-textFaint">{label}</dt>
            <dd className="mt-0.5 break-words text-[12.5px] text-text">{value}</dd>
          </div>
        ))}
      </dl>
    </motion.aside>
  )
}

/** Real pictures, around the current photo. Only a window of them: a strip of
 *  forty thousand would be forty thousand images in the page. */
function Filmstrip({
  photos,
  index,
  base,
  onPick,
}: {
  photos: MediaFile[]
  index: number
  base: string
  onPick: (index: number) => void
}): React.JSX.Element {
  const WINDOW = 30
  const from = Math.max(0, index - WINDOW)
  const to = Math.min(photos.length, index + WINDOW + 1)
  const current = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    current.current?.scrollIntoView({ block: 'nearest', inline: 'center', behavior: 'smooth' })
  }, [index])

  return (
    <div className="flex h-[74px] shrink-0 items-center gap-1.5 overflow-x-auto border-t border-white/[0.05] px-4 [scrollbar-width:none]">
      {photos.slice(from, to).map((p, n) => {
        const i = from + n
        const selected = i === index
        return (
          <button
            key={p.path}
            ref={selected ? current : undefined}
            onClick={() => onPick(i)}
            aria-label={stemOf(p.path)}
            className={cn(
              'relative h-12 w-12 shrink-0 overflow-hidden rounded-[3px] bg-[#141416] transition-[opacity,box-shadow] duration-150',
              selected ? 'opacity-100 ring-2 ring-white/80' : 'opacity-50 hover:opacity-90',
            )}
          >
            {base && (
              <img
                src={thumbUrl(base, p.path, p.mtime, GRID_THUMB)}
                alt=""
                loading="lazy"
                draggable={false}
                className="h-full w-full object-cover"
              />
            )}
          </button>
        )
      })}
    </div>
  )
}

function NavArrow({
  side,
  onClick,
}: {
  side: 'left' | 'right'
  onClick: () => void
}): React.JSX.Element {
  const Icon = side === 'left' ? ChevronLeft : ChevronRight
  return (
    <button
      onClick={onClick}
      aria-label={side === 'left' ? 'Previous photo' : 'Next photo'}
      className={cn(
        'absolute top-1/2 z-10 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-full bg-black/50 text-textDim backdrop-blur transition-colors duration-150 hover:bg-black/75 hover:text-text',
        side === 'left' ? 'left-3' : 'right-3',
      )}
    >
      <Icon size={18} />
    </button>
  )
}

function ToolButton({
  icon: Icon,
  label,
  active,
  small,
  onClick,
}: {
  icon: typeof Info
  label: string
  active?: boolean
  small?: boolean
  onClick?: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        'flex items-center justify-center rounded-md transition-colors duration-150',
        small ? 'h-7 w-7' : TOUCH ? 'h-11 w-11' : 'h-8 w-8',
        active ? 'bg-white/[0.1] text-text' : 'text-textDim hover:bg-white/[0.07] hover:text-text',
      )}
    >
      <Icon size={small ? 13 : TOUCH ? 19 : 15} />
    </button>
  )
}
