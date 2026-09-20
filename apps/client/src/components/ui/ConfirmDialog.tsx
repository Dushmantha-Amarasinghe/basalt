import { useCallback, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion } from 'framer-motion'
import { AlertTriangle } from 'lucide-react'

export interface ConfirmRequest {
  title: string
  message: string
  /** What the confirming button says. Defaults to a plain `Continue`. */
  confirmLabel?: string
  /** Draws the action as destructive. */
  danger?: boolean
}

/**
 * A yes/no question, asked in the app's own voice.
 *
 * This replaces the platform dialog. That one worked, but it arrived in the
 * system font on a white card with a yellow warning triangle, in the middle of
 * an app that is otherwise entirely dark and set in its own type — and it said
 * "OK", which answers a different question from the one being asked. An app
 * that looks considered everywhere except the moment it asks permission looks
 * borrowed at exactly the wrong moment.
 *
 * It also removes a whole class of failure. The platform dialog is a plugin
 * call behind a permission grant, and when that grant went missing the button
 * silently did nothing — twice. Nothing here can be un-granted.
 */
export function ConfirmDialog({
  request,
  onSettle,
}: {
  request: ConfirmRequest | null
  onSettle: (ok: boolean) => void
}): React.JSX.Element {
  return createPortal(
    <AnimatePresence>
      {request && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="fixed inset-0 z-[95] flex items-center justify-center bg-black/55 backdrop-blur-[2px]"
          onMouseDown={() => onSettle(false)}
        >
          <Card request={request} onSettle={onSettle} />
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  )
}

function Card({
  request,
  onSettle,
}: {
  request: ConfirmRequest
  onSettle: (ok: boolean) => void
}): React.JSX.Element {
  return (
    <motion.div
      role="alertdialog"
      aria-modal
      initial={{ opacity: 0, scale: 0.97, y: 6 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.97, y: 6 }}
      transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
      onMouseDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onSettle(false)
        if (e.key === 'Enter') onSettle(true)
      }}
      className="w-[380px] rounded-lg border border-white/10 bg-panel2 p-4 shadow-lift"
    >
      <div className="flex items-start gap-3">
        {request.danger && (
          <span className="mt-0.5 shrink-0 text-danger">
            <AlertTriangle size={15} />
          </span>
        )}
        <div className="min-w-0">
          <h2 className="text-[13px] font-semibold text-text">{request.title}</h2>
          <p className="mt-1.5 text-[12px] leading-relaxed text-textDim">
            {request.message}
          </p>
        </div>
      </div>

      <div className="mt-4 flex justify-end gap-2">
        <button
          onClick={() => onSettle(false)}
          className="rounded-md px-3 py-1.5 text-[12px] text-textDim transition-colors hover:bg-white/[0.05] hover:text-text"
        >
          Cancel
        </button>
        <motion.button
          // Focused on open, so Enter and Escape both do the obvious thing
          // without anyone reaching for the mouse.
          ref={(button) => button?.focus()}
          whileTap={{ scale: 0.97 }}
          onClick={() => onSettle(true)}
          className={
            request.danger
              ? 'rounded-md border border-danger/40 bg-dangerBg px-3 py-1.5 text-[12px] text-danger transition-colors hover:bg-danger/20'
              : 'rounded-md border border-white/[0.16] bg-panel px-3 py-1.5 text-[12px] text-text transition-colors hover:bg-white/[0.06]'
          }
        >
          {request.confirmLabel ?? 'Continue'}
        </motion.button>
      </div>
    </motion.div>
  )
}

/**
 * Turns the dialog into something an `async` callback can simply await.
 *
 * The two callers are both in the middle of doing something — deleting files,
 * forgetting a vault — and neither wants to be split into a "start" and a
 * "finish" half around a render. So the promise is held open and settled by
 * whichever button is pressed.
 */
export function useConfirm(): {
  confirm: (request: ConfirmRequest) => Promise<boolean>
  dialog: React.JSX.Element
} {
  const [request, setRequest] = useState<ConfirmRequest | null>(null)
  // In a ref rather than in state: resolving inside a state updater would run
  // the side effect twice under StrictMode's double invocation.
  const pending = useRef<((ok: boolean) => void) | null>(null)

  const confirm = useCallback(
    (next: ConfirmRequest) =>
      new Promise<boolean>((resolve) => {
        // A second question while one is open would strand the first promise
        // for ever, and an awaited promise that never settles is a menu item
        // that hangs rather than one that fails.
        pending.current?.(false)
        pending.current = resolve
        setRequest(next)
      }),
    [],
  )

  const settle = useCallback((ok: boolean) => {
    const resolve = pending.current
    pending.current = null
    setRequest(null)
    resolve?.(ok)
  }, [])

  return {
    confirm,
    dialog: <ConfirmDialog request={request} onSettle={settle} />,
  }
}
