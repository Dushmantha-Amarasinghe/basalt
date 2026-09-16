import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { motion } from 'framer-motion'
import { Check, LayoutGrid, LayoutList, Rows3 } from 'lucide-react'
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
  const [open, setOpen] = useState(false)
  const [rect, setRect] = useState<DOMRect | null>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const current = VIEW_OPTIONS.find((o) => o.mode === mode) ?? VIEW_OPTIONS[0]!
  const CurrentIcon = current.icon

  const measure = (): void => {
    const el = triggerRef.current
    if (el) setRect(el.getBoundingClientRect())
  }

  useEffect(() => {
    if (!open) return undefined
    const onReposition = (): void => measure()
    window.addEventListener('scroll', onReposition, true)
    window.addEventListener('resize', onReposition)
    return () => {
      window.removeEventListener('scroll', onReposition, true)
      window.removeEventListener('resize', onReposition)
    }
  }, [open])

  return (
    <>
      <button
        ref={triggerRef}
        onClick={() => {
          measure()
          setOpen((v) => !v)
        }}
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

      {open &&
        rect &&
        createPortal(
          <>
            <div className="fixed inset-0 z-[60]" onClick={() => setOpen(false)} />
            <motion.div
              initial={{ opacity: 0, y: -4, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{ duration: 0.11, ease: 'easeOut' }}
              style={{
                position: 'fixed',
                left: Math.max(8, rect.right - 210),
                top: rect.bottom + 6,
                width: 210,
              }}
              className="z-[61] overflow-hidden rounded-md border border-white/10 bg-panel2 p-1 shadow-lift"
            >
              {VIEW_OPTIONS.map((option) => {
                const Icon = option.icon
                const active = option.mode === mode
                return (
                  <button
                    key={option.mode}
                    onClick={() => {
                      onChange(option.mode)
                      setOpen(false)
                    }}
                    className={cn(
                      'flex w-full items-center gap-2.5 rounded px-2.5 py-2 text-left transition-colors hover:bg-white/[0.06]',
                      active ? 'text-text' : 'text-textDim',
                    )}
                  >
                    <Icon size={15} className={active ? 'text-basalt' : 'text-textFaint'} />
                    <span className="flex-1">
                      <span className="block text-[12px]">{option.label}</span>
                      <span className="block text-[10px] text-textFaint">{option.hint}</span>
                    </span>
                    {active && <Check size={12} className="text-textDim" />}
                  </button>
                )
              })}
            </motion.div>
          </>,
          document.body,
        )}
    </>
  )
}
