import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  X,
} from 'lucide-react'
import type { Transfer } from '@/lib/useTransfers'
import { cn, formatBytes } from '@/lib/utils'

/**
 * The transfer queue, as a panel that rises from the bottom edge.
 *
 * This is the screen a NAS client is really about, and the one Frostbyte has no
 * equivalent of: its queue is a list of jobs to run, this is a list of things
 * currently crossing a link that might stall or be cancelled.
 *
 * Collapsed it is a single summary bar, so it can stay open permanently without
 * stealing room from the files.
 */
export function TransfersPanel({
  transfers,
  open,
  onToggle,
  onCancel,
  onClearDone,
}: {
  transfers: Transfer[]
  open: boolean
  onToggle: () => void
  onCancel: (id: string) => void
  onClearDone: () => void
}): React.JSX.Element {
  const active = transfers.filter((t) => t.status === 'active')
  const done = transfers.filter((t) => t.status === 'done')
  const failed = transfers.filter(
    (t) => t.status === 'failed' || t.status === 'cancelled',
  )

  const totalRate = active.reduce((sum, t) => sum + t.rate, 0)
  const remaining = active.reduce((sum, t) => sum + (t.total - t.transferred), 0)
  const etaSeconds = totalRate > 0 ? remaining / totalRate : 0

  return (
    <div className="shrink-0 border-t border-line bg-ink2/80 backdrop-blur-sm">
      <div className="flex h-9 w-full items-center gap-3 px-4">
        <button
          onClick={onToggle}
          className="-mx-2 flex min-w-0 flex-1 items-center gap-3 rounded px-2 py-1 text-left transition-colors hover:bg-white/[0.02]"
        >
          <motion.span
            animate={{ rotate: open ? 0 : 180 }}
            transition={{ duration: 0.2 }}
            className="shrink-0 text-textFaint"
          >
            <ChevronDown size={14} />
          </motion.span>

          <span className="shrink-0 text-xs font-semibold text-text">Transfers</span>

          {active.length > 0 ? (
            <span className="tnum truncate font-mono text-[11px] text-textDim">
              {active.length} active · {(totalRate / 1e6).toFixed(1)} MB/s
              {etaSeconds > 0 && ` · ${formatEta(etaSeconds)} left`}
            </span>
          ) : (
            <span className="font-mono text-[11px] text-textFaint">idle</span>
          )}
        </button>

        {done.length > 0 && <Pill label={`${done.length} done`} muted />}
        {failed.length > 0 && <Pill label={`${failed.length} failed`} />}
        {(done.length > 0 || failed.length > 0) && (
          <button
            onClick={onClearDone}
            className="shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] text-textFaint transition-colors hover:bg-white/[0.05] hover:text-textDim"
          >
            clear
          </button>
        )}
      </div>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.25, ease: [0.4, 0, 0.2, 1] }}
            className="overflow-hidden"
          >
            {/*
              Capped against the viewport, not a fixed 232px. In a short window
              a fixed cap let this panel claim more height than was left, which
              squeezed the file list past zero and broke the whole layout.
              `min()` keeps it to a third of the window however small that gets.
            */}
            <div
              className="overflow-y-auto border-t border-line px-2 py-1.5"
              style={{ maxHeight: 'min(232px, 32vh)' }}
            >
              {transfers.length === 0 ? (
                <p className="px-2 py-4 text-center text-[11px] text-textFaint">
                  Nothing transferring. Downloads and uploads appear here.
                </p>
              ) : (
                transfers.map((t) => (
                  <TransferRow key={t.id} transfer={t} onCancel={onCancel} />
                ))
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

function Pill({ label, muted }: { label: string; muted?: boolean }): React.JSX.Element {
  return (
    <span
      className={cn(
        'shrink-0 rounded-full border px-2 py-0.5 font-mono text-[10px]',
        muted
          ? 'border-white/[0.06] text-textFaint'
          : 'border-danger/25 text-danger',
      )}
    >
      {label}
    </span>
  )
}

function TransferRow({
  transfer,
  onCancel,
}: {
  transfer: Transfer
  onCancel: (id: string) => void
}): React.JSX.Element {
  const percent =
    transfer.total > 0 ? (transfer.transferred / transfer.total) * 100 : 0
  const isActive = transfer.status === 'active'
  const isDone = transfer.status === 'done'
  const isBad = transfer.status === 'failed' || transfer.status === 'cancelled'

  return (
    <div className="group flex items-center gap-3 rounded-md px-2 py-2 transition-colors hover:bg-white/[0.03]">
      <span
        className={cn(
          'flex h-6 w-6 shrink-0 items-center justify-center rounded',
          isBad ? 'text-danger' : isDone ? 'text-textFaint' : 'text-textDim',
        )}
      >
        {isBad ? (
          <AlertCircle size={13} />
        ) : isDone ? (
          <Check size={13} />
        ) : transfer.kind === 'download' ? (
          <ArrowDown size={13} />
        ) : (
          <ArrowUp size={13} />
        )}
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span
            className={cn(
              'truncate text-[13px]',
              isDone || isBad ? 'text-textFaint' : 'text-text',
            )}
            title={transfer.path}
          >
            {transfer.name}
          </span>
          {transfer.total > 0 && (
            <span className="tnum ml-auto shrink-0 font-mono text-[10px] text-textFaint">
              {formatBytes(transfer.transferred)} / {formatBytes(transfer.total)}
            </span>
          )}
        </div>

        {isActive && (
          <div className="mt-1.5 h-[3px] overflow-hidden rounded-full bg-white/[0.06]">
            <motion.div
              className="h-full rounded-full bg-basalt"
              initial={false}
              animate={{ width: `${percent}%` }}
              transition={{ duration: 0.4, ease: 'easeOut' }}
            />
          </div>
        )}

        {isBad && transfer.error && (
          <p className="mt-1 truncate text-[10px] text-danger" title={transfer.error}>
            {transfer.error}
          </p>
        )}
      </div>

      <span className="tnum w-[70px] shrink-0 text-right font-mono text-[10px] text-textFaint">
        {isActive ? `${(transfer.rate / 1e6).toFixed(1)} MB/s` : transfer.status}
      </span>

      {/* Row controls, revealed on hover like the file list. */}
      <span className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        {isActive && (
          <button
            onClick={() => onCancel(transfer.id)}
            aria-label="Cancel"
            title="Cancel"
            className="flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.07] hover:text-text"
          >
            <X size={12} />
          </button>
        )}
      </span>
    </div>
  )
}

function formatEta(seconds: number): string {
  if (seconds < 60) return `${Math.ceil(seconds)}s`
  if (seconds < 3600) return `${Math.ceil(seconds / 60)}m`
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.ceil((seconds % 3600) / 60)
  return `${hours}h ${minutes}m`
}
