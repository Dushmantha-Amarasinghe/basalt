import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

/**
 * Something that comes and goes with a CSS transition rather than a
 * JavaScript animation.
 *
 * `mounted` is whether it is in the page at all; `visible` is whether it is
 * in its shown state. Coming in, it is put in the page in its hidden state
 * first and only switched to shown two frames later, so the browser has drawn
 * where it starts from and building it never competes with the movement.
 * Going out, it switches to hidden at once and leaves the page when the
 * transition has had time to finish.
 */
export function usePresence(show: boolean, exitMs: number): { mounted: boolean; visible: boolean } {
  const [mounted, setMounted] = useState(show)
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    if (show) {
      setMounted(true)
      let second = 0
      const first = requestAnimationFrame(() => {
        second = requestAnimationFrame(() => setVisible(true))
      })
      return () => {
        cancelAnimationFrame(first)
        cancelAnimationFrame(second)
      }
    }
    setVisible(false)
    const gone = setTimeout(() => setMounted(false), exitMs)
    return () => clearTimeout(gone)
  }, [show, exitMs])

  return { mounted, visible }
}

/**
 * Keeps what was last shown while it animates away.
 *
 * Whatever it was about is usually cleared in the same moment it is hidden —
 * the clipboard emptied, the notice dismissed — and something going blank as
 * it leaves reads as a glitch.
 */
export function useLastShown<T>(value: T, show: boolean): T {
  const last = useRef(value)
  if (show) last.current = value
  return last.current
}

/**
 * Rises into place from a little below, fading in, and sinks away again.
 *
 * For the bars above the tabs and the notice. Transform and opacity only, so
 * the compositor runs it.
 */
export function Rise({
  show,
  children,
  className,
  style,
  onClick,
}: {
  show: boolean
  children: React.ReactNode
  className?: string
  style?: React.CSSProperties
  onClick?: () => void
}): React.JSX.Element | null {
  const { mounted, visible } = usePresence(show, 240)
  const content = useLastShown(children, show)
  if (!mounted) return null
  return (
    <div
      onClick={onClick}
      className={cn('rise', !show && 'pointer-events-none', className)}
      style={{
        ...style,
        opacity: visible ? 1 : 0,
        transform: visible ? 'translate3d(0, 0, 0)' : 'translate3d(0, 24px, 0)',
      }}
    >
      {content}
    </div>
  )
}
