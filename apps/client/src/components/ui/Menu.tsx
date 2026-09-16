import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { motion } from 'framer-motion'
import { Check } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface MenuItem<T extends string> {
  value: T
  label: string
  hint?: string
  icon?: React.ComponentType<{ size?: number; className?: string }>
}

/**
 * A dropdown that escapes its container.
 *
 * Extracted because View, Sort and every Settings select were each
 * reimplementing the same three awkward parts: portalling to `<body>` to
 * escape a clipping ancestor, positioning from the trigger's rect, and
 * repositioning on scroll and resize. Three copies of that is three places for
 * the flip-up logic to drift.
 *
 * It renders outside the React tree it is written in, so it is unaffected by
 * any `overflow-hidden` above it — which is what the settings cards and the
 * toolbar both have.
 */
export function Menu<T extends string>({
  trigger,
  items,
  value,
  onSelect,
  align = 'right',
  width = 210,
  groups,
}: {
  trigger: (props: { open: boolean; onClick: () => void; ref: React.Ref<HTMLButtonElement> }) => React.ReactNode
  items: MenuItem<T>[]
  value: T | T[]
  onSelect: (value: T) => void
  align?: 'left' | 'right'
  width?: number
  /** Optional index at which to draw a separator, like Explorer's sort menu. */
  groups?: number[]
}): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const [rect, setRect] = useState<DOMRect | null>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const measure = (): void => {
    const el = triggerRef.current
    if (el) setRect(el.getBoundingClientRect())
  }

  useEffect(() => {
    if (!open) return undefined
    const onReposition = (): void => measure()
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') setOpen(false)
    }
    // Capture phase, so scrolling an inner pane repositions it too.
    window.addEventListener('scroll', onReposition, true)
    window.addEventListener('resize', onReposition)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('scroll', onReposition, true)
      window.removeEventListener('resize', onReposition)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const selected = Array.isArray(value) ? value : [value]
  const estimatedHeight = items.length * 38 + 12
  const flipUp = rect ? window.innerHeight - rect.bottom < estimatedHeight + 12 : false

  // Clamp horizontally so a menu near the window edge never hangs off it.
  const left = rect
    ? Math.min(
        Math.max(8, align === 'right' ? rect.right - width : rect.left),
        window.innerWidth - width - 8,
      )
    : 0

  return (
    <>
      {trigger({
        open,
        ref: triggerRef,
        onClick: () => {
          measure()
          setOpen((v) => !v)
        },
      })}

      {open &&
        rect &&
        createPortal(
          <>
            <div className="fixed inset-0 z-[60]" onClick={() => setOpen(false)} />
            <motion.div
              initial={{ opacity: 0, y: flipUp ? 4 : -4, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{ duration: 0.11, ease: 'easeOut' }}
              style={{
                position: 'fixed',
                left,
                width,
                ...(flipUp
                  ? { bottom: window.innerHeight - rect.top + 6 }
                  : { top: rect.bottom + 6 }),
                maxHeight: 'min(60vh, 420px)',
              }}
              className="z-[61] overflow-y-auto rounded-md border border-white/10 bg-panel2 p-1 shadow-lift"
            >
              {items.map((item, index) => {
                const Icon = item.icon
                const active = selected.includes(item.value)
                return (
                  <div key={item.value}>
                    {groups?.includes(index) && (
                      <div className="my-1 h-px bg-white/[0.07]" />
                    )}
                    <button
                      onClick={() => {
                        onSelect(item.value)
                        setOpen(false)
                      }}
                      className={cn(
                        'flex w-full items-center gap-2.5 rounded px-2.5 py-2 text-left transition-colors hover:bg-white/[0.06]',
                        active ? 'text-text' : 'text-textDim',
                      )}
                    >
                      {Icon ? (
                        <Icon size={15} className={active ? 'text-basalt' : 'text-textFaint'} />
                      ) : (
                        <span className="w-[15px]" />
                      )}
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[12px]">{item.label}</span>
                        {item.hint && (
                          <span className="block truncate text-[10px] text-textFaint">
                            {item.hint}
                          </span>
                        )}
                      </span>
                      {active && <Check size={12} className="shrink-0 text-textDim" />}
                    </button>
                  </div>
                )
              })}
            </motion.div>
          </>,
          document.body,
        )}
    </>
  )
}
