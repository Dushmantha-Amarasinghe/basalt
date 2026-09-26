import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { POPOVER } from '@/lib/motion'
import { AnimatePresence, motion } from 'framer-motion'
import { Check, ChevronDown } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface Option {
  value: string
  label: string
}

/**
 * A list to choose from, drawn by this app.
 *
 * Not a `<select>`. The element itself can be styled, but the list it opens
 * is drawn by Windows and cannot be — so picking an audio output meant a grey
 * system menu with a scrollbar landing in the middle of a dark window. In an
 * app whose whole point is that it looks considered, that is the one control
 * that gives the game away.
 *
 * Keyboard behaviour is the part people notice only when it is missing:
 * Escape closes, Enter and Space open, and the arrows move through the list
 * without opening it — which is what a `<select>` does.
 */
export function Dropdown({
  value,
  options,
  onChange,
  label,
  className,
}: {
  value: string
  options: Option[]
  onChange: (value: string) => void
  /** For screen readers, since the button shows the value rather than a name. */
  label: string
  className?: string
}): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement | null>(null)
  const button = useRef<HTMLButtonElement | null>(null)
  const list = useRef<HTMLUListElement | null>(null)
  const chosen = options.find((option) => option.value === value)

  /**
   * Where to draw the list, in viewport coordinates.
   *
   * The list is portalled to the body rather than left beside the button,
   * because the settings cards are rounded and therefore clip their
   * contents — the first version showed two entries and cut the rest off at
   * the edge of the panel. Nothing inside a card can overflow it, so the
   * list has to live outside and be told where to go.
   */
  const [at, setAt] = useState({ left: 0, top: 0, width: 0, above: false })

  useLayoutEffect(() => {
    if (!open || !button.current) return
    const rect = button.current.getBoundingClientRect()
    const room = window.innerHeight - rect.bottom
    // Flip upwards when the list would run off the bottom.
    const above = room < 220 && rect.top > room
    setAt({
      left: rect.left,
      top: above ? rect.top : rect.bottom + 6,
      width: rect.width,
      above,
    })
  }, [open])

  // Closing on an outside click is what every menu does and what anyone
  // tries first.
  useEffect(() => {
    if (!open) return undefined
    const away = (e: MouseEvent): void => {
      const target = e.target as Node
      // The list is portalled out of `box`, so it has to be asked separately
      // — otherwise `mousedown` on an option counts as "outside", the list
      // unmounts, and the click that would have chosen it never lands.
      if (box.current?.contains(target) || list.current?.contains(target)) return
      setOpen(false)
    }
    document.addEventListener('mousedown', away)
    return () => document.removeEventListener('mousedown', away)
  }, [open])

  const step = (by: number): void => {
    const index = options.findIndex((option) => option.value === value)
    const next = options[Math.min(options.length - 1, Math.max(0, index + by))]
    if (next) onChange(next.value)
  }

  return (
    <div ref={box} className={cn('relative', className)}>
      <button
        type="button"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') setOpen(false)
          if (e.key === 'ArrowDown' && !open) {
            e.preventDefault()
            step(1)
          }
          if (e.key === 'ArrowUp' && !open) {
            e.preventDefault()
            step(-1)
          }
        }}
        ref={button}
        className={cn(
          'flex w-full items-center justify-between gap-2 rounded-md border px-2.5 py-1.5',
          'font-mono text-[11.5px] text-text transition-colors',
          open ? 'border-lineBright bg-ink2' : 'border-line bg-ink2 hover:border-lineBright',
        )}
      >
        <span className="min-w-0 truncate text-left">{chosen?.label ?? value}</span>
        <ChevronDown
          size={13}
          className={cn(
            'shrink-0 text-textFaint transition-transform',
            open && 'rotate-180',
          )}
        />
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.ul
              ref={list}
              role="listbox"
              initial={{ opacity: 0, y: at.above ? 4 : -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: at.above ? 4 : -4 }}
              transition={POPOVER.transition}
              style={{
                left: at.left,
                top: at.above ? undefined : at.top,
                bottom: at.above ? window.innerHeight - at.top + 6 : undefined,
                width: Math.max(at.width, 280),
              }}
              // Opaque, like the player's menu: this sits over whatever the
              // page is showing and a translucent list is a list you squint at.
              className="fixed z-[80] max-h-[320px] overflow-y-auto rounded-lg border border-white/15 bg-[#151517] py-1 shadow-lift"
            >
              {options.map((option) => (
                <li key={option.value}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={option.value === value}
                    onClick={() => {
                      onChange(option.value)
                      setOpen(false)
                    }}
                    className={cn(
                      'flex w-full items-center gap-2 px-2.5 py-1.5 text-left font-mono text-[11.5px] transition-colors',
                      option.value === value
                        ? 'bg-white/[0.07] text-text'
                        : 'text-textDim hover:bg-white/[0.04] hover:text-text',
                    )}
                  >
                    <Check
                      size={12}
                      className={cn(
                        'shrink-0',
                        option.value === value ? 'text-basalt' : 'opacity-0',
                      )}
                    />
                    <span className="min-w-0 truncate">{option.label}</span>
                  </button>
                </li>
              ))}
            </motion.ul>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </div>
  )
}
