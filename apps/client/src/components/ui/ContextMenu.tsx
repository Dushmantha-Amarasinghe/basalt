import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { motion } from 'framer-motion'
import { cn } from '@/lib/utils'

export interface MenuAction {
  id: string
  label: string
  icon?: React.ComponentType<{ size?: number; className?: string }>
  /** Shown right-aligned, e.g. `Ctrl+C`. Display only; the real binding lives
   *  with the keyboard handler. */
  shortcut?: string
  danger?: boolean
  disabled?: boolean
  /** Draws a hairline above this item. */
  separatorBefore?: boolean
  run: () => void | Promise<void>
}

const ITEM_HEIGHT = 30
const SEPARATOR_HEIGHT = 9
const PADDING = 8
const WIDTH = 216

/**
 * A menu anchored to a point rather than to a trigger.
 *
 * One instance for the whole application rather than one per row: a file list
 * renders a hundred rows and would otherwise carry a hundred menus, each with
 * its own listeners, for a thing only one of them can show at a time.
 *
 * Portalled to `<body>` because the list is inside several `overflow-hidden`
 * containers, and flipped rather than clipped when it would run off an edge —
 * a context menu that opens half off the screen is worse than no menu.
 */
export function useContextMenu(): {
  open: (event: { clientX: number; clientY: number }, actions: MenuAction[]) => void
  close: () => void
  node: React.ReactNode
} {
  const [state, setState] = useState<{
    x: number
    y: number
    actions: MenuAction[]
  } | null>(null)

  const open = useCallback(
    (event: { clientX: number; clientY: number }, actions: MenuAction[]) => {
      if (actions.length === 0) return
      setState({ x: event.clientX, y: event.clientY, actions })
    },
    [],
  )

  const close = useCallback(() => setState(null), [])

  return {
    open,
    close,
    node: state ? (
      <ContextMenuSurface
        x={state.x}
        y={state.y}
        actions={state.actions}
        onClose={close}
      />
    ) : null,
  }
}

function ContextMenuSurface({
  x,
  y,
  actions,
  onClose,
}: {
  x: number
  y: number
  actions: MenuAction[]
  onClose: () => void
}): React.JSX.Element {
  const ref = useRef<HTMLDivElement>(null)
  const [focused, setFocused] = useState(-1)

  const enabled = actions.filter((a) => !a.disabled)

  const runAt = useCallback(
    (action: MenuAction) => {
      onClose()
      void action.run()
    },
    [onClose],
  )

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
        return
      }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        setFocused((current) => {
          const step = e.key === 'ArrowDown' ? 1 : -1
          const next = current + step
          if (next < 0) return enabled.length - 1
          if (next >= enabled.length) return 0
          return next
        })
        return
      }
      if (e.key === 'Enter' && focused >= 0) {
        e.preventDefault()
        const action = enabled[focused]
        if (action) runAt(action)
      }
    }
    // Capture, so the file list's own key handling never sees these first.
    window.addEventListener('keydown', onKey, true)
    // Scrolling under an open menu leaves it pointing at the wrong row.
    window.addEventListener('scroll', onClose, true)
    window.addEventListener('resize', onClose)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('scroll', onClose, true)
      window.removeEventListener('resize', onClose)
    }
  }, [enabled, focused, onClose, runAt])

  // Estimated rather than measured, so the first paint is already in the right
  // place. Measuring would mean one frame in the wrong position, which reads as
  // a flicker every single time the menu opens.
  const height =
    PADDING +
    actions.length * ITEM_HEIGHT +
    actions.filter((a) => a.separatorBefore).length * SEPARATOR_HEIGHT

  const left = Math.min(Math.max(6, x), window.innerWidth - WIDTH - 6)
  const top =
    y + height > window.innerHeight - 6 ? Math.max(6, y - height) : y

  return createPortal(
    <>
      <div
        className="fixed inset-0 z-[80]"
        onMouseDown={onClose}
        onContextMenu={(e) => {
          // A second right-click closes this one rather than stacking another.
          e.preventDefault()
          onClose()
        }}
      />
      <motion.div
        ref={ref}
        role="menu"
        initial={{ opacity: 0, scale: 0.97 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.09, ease: 'easeOut' }}
        style={{
          position: 'fixed',
          left,
          top,
          width: WIDTH,
          transformOrigin: 'top left',
        }}
        className="z-[81] overflow-hidden rounded-md border border-white/10 bg-panel2 p-1 shadow-lift"
      >
        {actions.map((action) => {
          const Icon = action.icon
          const index = enabled.indexOf(action)
          return (
            <div key={action.id}>
              {action.separatorBefore && (
                <div className="my-1 h-px bg-white/[0.07]" />
              )}
              <button
                role="menuitem"
                disabled={action.disabled}
                onMouseEnter={() => setFocused(index)}
                onClick={() => runAt(action)}
                className={cn(
                  'flex h-[30px] w-full items-center gap-2.5 rounded px-2.5 text-left text-[12px] transition-colors',
                  action.disabled
                    ? 'cursor-default text-textFaint/50'
                    : action.danger
                      ? 'text-danger hover:bg-danger/12'
                      : 'text-textDim hover:bg-white/[0.06] hover:text-text',
                  !action.disabled && index === focused && 'bg-white/[0.06]',
                )}
              >
                {Icon ? (
                  <Icon size={14} className="shrink-0" />
                ) : (
                  <span className="w-[14px] shrink-0" />
                )}
                <span className="min-w-0 flex-1 truncate">{action.label}</span>
                {action.shortcut && (
                  <span className="shrink-0 font-mono text-[10px] text-textFaint">
                    {action.shortcut}
                  </span>
                )}
              </button>
            </div>
          )
        })}
      </motion.div>
    </>,
    document.body,
  )
}

/** Where a menu of this shape will actually be drawn, exported for tests. */
export function menuPosition(
  x: number,
  y: number,
  itemCount: number,
  separatorCount: number,
  viewport: { width: number; height: number },
): { left: number; top: number } {
  const height =
    PADDING + itemCount * ITEM_HEIGHT + separatorCount * SEPARATOR_HEIGHT
  return {
    left: Math.min(Math.max(6, x), viewport.width - WIDTH - 6),
    top: y + height > viewport.height - 6 ? Math.max(6, y - height) : y,
  }
}

export { WIDTH as CONTEXT_MENU_WIDTH, ITEM_HEIGHT, SEPARATOR_HEIGHT, PADDING }
