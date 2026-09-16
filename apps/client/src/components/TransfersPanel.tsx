import { AnimatePresence, motion } from 'framer-motion'
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  Pause,
  Play,
  X,
} from 'lucide-react'
import type { Transfer } from '@/lib/mockMedia'
import { cn, formatBytes } from '@/lib/utils'

/**
 * The transfer queue, as a panel that rises from the bottom edge.
 *
 * This is the screen a NAS client is really about, and the one Frostbyte has no
 * equivalent of: its queue is a list of jobs to run, this is a list of things
 * currently crossing a link that might stall, resume, or be paused.
 *
 * Collapsed it is a single summary bar, so it can stay open permanently without
 * stealing room from the files.
 */
export function TransfersPanel({
  transfers,
  open,
  onToggle,
}: {
  transfers: Transfer[]
  open: boolean
  onToggle: () => void
}): React.JSX.Element {
  const active = transfers.filter((t) => t.status === 'active')
  const queued = transfers.filter((t) => t.status === 'queued')
  const done = transfers.filter((t) => t.status === 'done')

  const totalSpeed = active.reduce((sum, t) => sum + t.speed, 0)
  const remainingBytes = [...active, ...queued].reduce(
    (sum, t) => sum + (t.bytes - t.transferred),
    0,
  )
  const etaSeconds = totalSpeed > 0 ? remainingBytes / (totalSpeed * 1e6) : 0

  return (
    <div className="shrink-0 border-t border-line bg-ink2/80 backdrop-blur-sm">
      <button
        onClick={onToggle}
        className="flex h-9 w-full items-center gap-3 px-4 text-left transition-colors hover:bg-white/[0.02]"
      >
        <motion.span
          animate={{ rotate: open ? 0 : 180 }}
          transition={{ duration: 0.2 }}
          className="text-textFaint"
        >
          <ChevronDown size={14} />
        </motion.span>

        <span className="text-xs font-semibold text-text">Transfers</span>

        {active.length > 0 ? (
          <span className="tnum font-mono text-[11px] text-textDim">
            {active.length} active · {totalSpeed.toFixed(1)} MB/s
            {etaSeconds > 0 && ` · ${formatEta(etaSeconds)} left`}
          </span>
        ) : (
          <span className="font-mono text-[11px] text-textFaint">idle</span>
        )}

        <div className="flex-1" />

        {queued.length > 0 && (
          <Pill label={`${queued.length} queued`} />
        )}
        {done.length > 0 && <Pill label={`${done.length} done`} muted />}
      </button>

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
              {transfers.map((t) => (
                <TransferRow key={t.id} transfer={t} />
              ))}
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
        'rounded-full border px-2 py-0.5 font-mono text-[10px]',
        muted
          ? 'border-white/[0.06] text-textFaint'
          : 'border-white/[0.12] text-textDim',
      )}
    >
      {label}
    </span>
  )
}

function TransferRow({ transfer }: { transfer: Transfer }): React.JSX.Element {
  const percent = transfer.bytes > 0 ? (transfer.transferred / transfer.bytes) * 100 : 0
  const isActive = transfer.status === 'active'
  const isDone = transfer.status === 'done'

  return (
    <div className="group flex items-center gap-3 rounded-md px-2 py-2 transition-colors hover:bg-white/[0.03]">
      <span
        className={cn(
          'flex h-6 w-6 shrink-0 items-center justify-center rounded',
          isDone ? 'text-textFaint' : 'text-textDim',
        )}
      >
        {isDone ? (
          <Check size={13} />
        ) : transfer.direction === 'down' ? (
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
              isDone ? 'text-textFaint' : 'text-text',
            )}
          >
            {transfer.name}
          </span>
          <span className="tnum ml-auto shrink-0 font-mono text-[10px] text-textFaint">
            {formatBytes(transfer.transferred)} / {formatBytes(transfer.bytes)}
          </span>
        </div>

        {!isDone && (
          <div className="mt-1.5 h-[3px] overflow-hidden rounded-full bg-white/[0.06]">
            <motion.div
              className={cn(
                'h-full rounded-full',
                isActive ? 'bg-basalt' : 'bg-basaltDim',
              )}
              initial={false}
              animate={{ width: `${percent}%` }}
              transition={{ duration: 0.4, ease: 'easeOut' }}
            />
          </div>
        )}
      </div>

      <span className="tnum w-[70px] shrink-0 text-right font-mono text-[10px] text-textFaint">
        {isActive ? `${transfer.speed.toFixed(1)} MB/s` : transfer.status}
      </span>

      {/* Row controls, revealed on hover like the file list. */}
      <span className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
        {!isDone && (
          <IconButton
            icon={isActive ? Pause : Play}
            label={isActive ? 'Pause' : 'Resume'}
          />
        )}
        <IconButton icon={X} label="Cancel" />
      </span>
    </div>
  )
}

function IconButton({
  icon: Icon,
  label,
}: {
  icon: typeof Pause
  label: string
}): React.JSX.Element {
  return (
    <button
      aria-label={label}
      title={label}
      className="flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.07] hover:text-text"
    >
      <Icon size={12} />
    </button>
  )
}

function formatEta(seconds: number): string {
  if (seconds < 60) return `${Math.ceil(seconds)}s`
  if (seconds < 3600) return `${Math.ceil(seconds / 60)}m`
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.ceil((seconds % 3600) / 60)
  return `${hours}h ${minutes}m`
}
