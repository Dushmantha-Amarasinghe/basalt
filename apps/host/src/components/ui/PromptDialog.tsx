import { useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion } from 'framer-motion'

export interface PromptRequest {
  title: string
  /** Prefilled value. */
  value: string
  confirmLabel: string
  /**
   * Characters to select on open. `stem` selects everything before the last
   * dot, which is what every file manager does on rename — nobody wants to
   * retype `.mkv`.
   */
  select?: 'all' | 'stem'
  onConfirm: (value: string) => void
}

/**
 * A small modal for naming things.
 *
 * `window.prompt` would be four lines instead of this, and it is disabled
 * outright in some webviews, renders in the OS font, and cannot be styled — in
 * an app whose whole point is that it looks considered, it is the one dialog
 * that would give the game away.
 */
export function PromptDialog({
  request,
  onClose,
}: {
  request: PromptRequest | null
  onClose: () => void
}): React.JSX.Element {
  return createPortal(
    <AnimatePresence>
      {request && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="fixed inset-0 z-[90] flex items-center justify-center bg-black/55 backdrop-blur-[2px]"
          onMouseDown={onClose}
        >
          {/*
            Keyed on the request, so each one mounts a fresh card whose state
            already holds the right value. Seeding it from an effect instead
            meant the field was briefly empty, autofocus selected nothing, and
            the caret ended up at the end — so renaming `holiday.mkv` made you
            retype the extension.
          */}
          <Card
            key={`${request.title}:${request.value}`}
            request={request}
            onClose={onClose}
          />
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  )
}

function Card({
  request,
  onClose,
}: {
  request: PromptRequest
  onClose: () => void
}): React.JSX.Element {
  const [value, setValue] = useState(request.value)
  const opened = useRef(false)

  /**
   * Focuses the field and selects the part worth replacing.
   *
   * A callback ref rather than `autoFocus` plus an `onFocus` handler: the ref
   * runs during commit with the node already in the document and its value
   * already set, which is the only point where both are guaranteed. The
   * earlier versions of this — an effect, then autofocus — each managed to run
   * at a moment when one or the other was not true, and left the caret at the
   * end so renaming `holiday.mkv` meant retyping the extension.
   */
  const focusInput = (input: HTMLInputElement | null): void => {
    if (!input || opened.current) return
    opened.current = true
    input.focus()
    const dot = request.value.lastIndexOf('.')
    // Only a real extension, not the leading dot of a hidden file.
    if (request.select === 'stem' && dot > 0) input.setSelectionRange(0, dot)
    else input.select()
  }

  const submit = (): void => {
    const trimmed = value.trim()
    if (!trimmed) return
    request.onConfirm(trimmed)
    onClose()
  }

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.97, y: 6 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.97, y: 6 }}
      transition={{ duration: 0.16, ease: [0.22, 1, 0.36, 1] }}
      // Stops a click inside the card reaching the backdrop's dismiss.
      onMouseDown={(e) => e.stopPropagation()}
      className="w-[360px] rounded-lg border border-white/10 bg-panel2 p-4 shadow-lift"
    >
      <h2 className="text-[13px] font-semibold text-text">{request.title}</h2>

      <input
        ref={focusInput}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            submit()
          }
          if (e.key === 'Escape') {
            e.preventDefault()
            onClose()
          }
        }}
        spellCheck={false}
        className="mt-3 h-9 w-full rounded-md border border-white/[0.09] bg-ink2 px-3 text-[13px] text-text transition-colors focus:border-white/25"
      />

      <div className="mt-4 flex justify-end gap-2">
        <button
          onClick={onClose}
          className="rounded-md px-3 py-1.5 text-[12px] text-textDim transition-colors hover:bg-white/[0.05] hover:text-text"
        >
          Cancel
        </button>
        <motion.button
          whileTap={{ scale: 0.97 }}
          onClick={submit}
          disabled={!value.trim()}
          className="rounded-md border border-white/[0.16] bg-panel px-3 py-1.5 text-[12px] text-text transition-colors hover:bg-white/[0.06] disabled:opacity-35"
        >
          {request.confirmLabel}
        </motion.button>
      </div>
    </motion.div>
  )
}
