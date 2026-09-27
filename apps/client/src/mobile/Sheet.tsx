import { useCallback, useLayoutEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import type { MenuAction } from '@/components/ui/ContextMenu'
import { cn } from '@/lib/utils'
import { usePresence, useLastShown } from './presence'
import { useBack } from './useBack'

/** How long a sheet takes to rise or fall. */
const SLIDE_MS = 300
/** A drag this far down, or a flick this fast, puts the sheet away. */
const DISMISS_PX = 90
const DISMISS_PX_PER_MS = 0.6

/**
 * A panel that rises from the bottom edge: the phone's menu, and its dialog.
 *
 * Dragged down, or tapped outside, or backed out of, it goes. It sits above
 * the gesture bar and never under it.
 *
 * It moves by CSS transition on `transform` alone, which the WebView hands to
 * its compositor: the slide carries on at full rate whatever the page is
 * doing. It was a Framer Motion spring with `drag`, and every opening began
 * with the library measuring the layout and scroll of the whole page, on the
 * main thread, in the very frames the slide needed — which is where the
 * stutter at the start of every sheet came from.
 *
 * It opens in two steps: its contents are put in the page below the screen,
 * and only on the frame after does it start to rise, so building them never
 * competes with the movement. Closing is the reverse: it falls, and is
 * taken out of the page once it is gone.
 */
export function Sheet({
  open,
  onClose,
  title,
  children,
  tall,
}: {
  open: boolean
  onClose: () => void
  title?: React.ReactNode
  children: React.ReactNode
  /** Room for a long list: most of the screen rather than what it needs. */
  tall?: boolean
}): React.JSX.Element | null {
  useBack(open, () => {
    onClose()
    return true
  })

  const { mounted, visible: shown } = usePresence(open, SLIDE_MS + 40)
  const panel = useRef<HTMLDivElement | null>(null)
  const body = useRef<HTMLDivElement | null>(null)

  // What the sheet last showed, kept while it falls away.
  const shownTitle = useLastShown(title, open)
  const shownChildren = useLastShown(children, open)

  const drag = useSheetDrag(panel, body, onClose)

  // A sheet re-shown mid-drag starts from where it is, not from a stale drag.
  const { reset } = drag
  useLayoutEffect(() => {
    if (shown) reset()
  }, [shown, reset])

  // A short sheet has nothing to scroll, so a finger anywhere on it drags it;
  // left to the browser, a downward swipe there is taken as an attempt to
  // scroll and the drag is cancelled halfway.
  useLayoutEffect(() => {
    const list = body.current
    if (!shown || !list) return
    list.style.touchAction = list.scrollHeight > list.clientHeight + 1 ? '' : 'none'
  }, [shown])

  if (!mounted) return null

  return createPortal(
    <div className={cn('fixed inset-0 z-[80]', !open && 'pointer-events-none')}>
      <div
        className="sheet-fade absolute inset-0 bg-black/55"
        style={{ opacity: shown ? 1 : 0 }}
        onClick={onClose}
      />
      <div
        ref={panel}
        role="dialog"
        className={cn(
          'sheet-slide absolute inset-x-0 bottom-0 mx-auto flex max-w-[640px] flex-col rounded-t-[22px] border-t border-white/[0.08] bg-[#141416] shadow-sheet',
          tall ? 'max-h-[88%]' : 'max-h-[80%]',
        )}
        style={{
          paddingBottom: 'calc(var(--inset-bottom, 0px) + 10px)',
          transform: shown ? 'translate3d(0, 0, 0)' : 'translate3d(0, 100%, 0)',
        }}
        {...drag.handlers}
      >
        <div className="flex shrink-0 touch-none justify-center pb-1 pt-2.5" data-grab="">
          <span className="h-1 w-10 rounded-full bg-white/20" />
        </div>
        {shownTitle && (
          <div className="shrink-0 touch-none px-5 pb-2 pt-1 text-[15px] font-semibold text-text" data-grab="">
            {shownTitle}
          </div>
        )}
        <div ref={body} className="min-h-0 overflow-y-auto overscroll-contain">
          {shownChildren}
        </div>
      </div>
    </div>,
    document.body,
  )
}

/**
 * Dragging a sheet down to put it away.
 *
 * From the handle and title always, and from anywhere on a sheet with nothing
 * to scroll. On a long list the finger is scrolling the list, and the browser
 * will not share that gesture, so there it is the handle. The sheet follows
 * the finger directly, without React, and on letting go either falls away or
 * springs back by the same transition it opened with.
 */
function useSheetDrag(
  panel: React.RefObject<HTMLDivElement | null>,
  body: React.RefObject<HTMLDivElement | null>,
  onClose: () => void,
): {
  handlers: {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void
    onPointerCancel: (e: React.PointerEvent<HTMLDivElement>) => void
  }
  reset: () => void
} {
  const start = useRef<{ y: number; at: number } | null>(null)
  const dragging = useRef(false)
  const last = useRef({ y: 0, at: 0, velocity: 0 })

  const place = (dy: number, animated: boolean): void => {
    const el = panel.current
    if (!el) return
    el.style.transition = animated ? '' : 'none'
    el.style.transform = `translate3d(0, ${Math.max(0, dy)}px, 0)`
  }

  const reset = useCallback(() => {
    start.current = null
    dragging.current = false
    const el = panel.current
    if (el) el.style.transition = ''
  }, [panel])

  const end = (e: React.PointerEvent<HTMLDivElement>, cancelled: boolean): void => {
    const s = start.current
    start.current = null
    if (!s || !dragging.current) return
    dragging.current = false
    const dy = e.clientY - s.y
    if (!cancelled && (dy > DISMISS_PX || last.current.velocity > DISMISS_PX_PER_MS)) {
      // Falls from where the finger left it; the sheet's own closing then
      // carries it the rest of the way.
      const el = panel.current
      if (el) {
        el.style.transition = ''
        el.style.transform = 'translate3d(0, 100%, 0)'
      }
      onClose()
    } else {
      place(0, true)
    }
  }

  return {
    reset,
    handlers: {
      onPointerDown: (e) => {
        if (e.button !== 0) return
        const target = e.target as HTMLElement
        const onGrab = target.closest('[data-grab]') !== null
        const list = body.current
        const scrolls = list !== null && list.scrollHeight > list.clientHeight + 1
        // A list that scrolls keeps its own gestures.
        if (!onGrab && scrolls) return
        start.current = { y: e.clientY, at: e.timeStamp }
        last.current = { y: e.clientY, at: e.timeStamp, velocity: 0 }
      },
      onPointerMove: (e) => {
        const s = start.current
        if (!s) return
        const dy = e.clientY - s.y
        if (!dragging.current) {
          if (dy < -6) {
            // Upward: a scroll, not a drag.
            start.current = null
            return
          }
          if (dy < 8) return
          dragging.current = true
          e.currentTarget.setPointerCapture(e.pointerId)
        }
        const dt = e.timeStamp - last.current.at
        if (dt > 0) {
          last.current = { y: e.clientY, at: e.timeStamp, velocity: (e.clientY - last.current.y) / dt }
        }
        place(dy, false)
      },
      onPointerUp: (e) => end(e, false),
      onPointerCancel: (e) => end(e, true),
    },
  }
}

/** The actions a file or folder offers, as a list in a sheet. */
export function ActionSheet({
  open,
  onClose,
  title,
  subtitle,
  actions,
}: {
  open: boolean
  onClose: () => void
  title?: string
  subtitle?: string
  actions: MenuAction[]
}): React.JSX.Element | null {
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={
        title ? (
          <div className="min-w-0">
            <div className="truncate">{title}</div>
            {subtitle && (
              <div className="mt-0.5 truncate font-mono text-[11px] font-normal text-textFaint">
                {subtitle}
              </div>
            )}
          </div>
        ) : undefined
      }
    >
      <div className="pb-1">
        {actions.map((action) => {
          const Icon = action.icon
          return (
            <div key={action.id}>
              {action.separatorBefore && <div className="mx-5 my-1 h-px bg-white/[0.07]" />}
              <button
                disabled={action.disabled}
                onClick={() => {
                  onClose()
                  void action.run()
                }}
                className={cn(
                  'flex h-[52px] w-full items-center gap-4 px-5 text-left text-[15px] transition-colors active:bg-white/[0.06] disabled:opacity-35',
                  action.danger ? 'text-danger' : 'text-text',
                )}
              >
                {Icon && <Icon size={20} className={action.danger ? 'text-danger' : 'text-textDim'} />}
                <span className="truncate">{action.label.replace(/…$/, '')}</span>
              </button>
            </div>
          )
        })}
      </div>
    </Sheet>
  )
}
