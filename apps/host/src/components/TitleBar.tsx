import { motion } from 'framer-motion'
import { HexMark } from './HexMark'
import { cn } from '@/lib/utils'

/**
 * True when running inside the Tauri shell rather than a plain browser.
 *
 * `getCurrentWindow()` throws outright when the Tauri internals are absent,
 * which would take the whole app down. Guarding here means the interface can be
 * opened and reviewed in an ordinary browser — a far faster loop than
 * rebuilding the desktop binary for every change.
 */
function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

async function windowAction(action: 'minimize' | 'toggleMaximize' | 'close'): Promise<void> {
  if (!inTauri()) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow()[action]()
}

/**
 * The title bar says what this machine is sharing, and whether it is live.
 *
 * The client's version shows the connection because the subject there is the
 * drive across the room. Here the subject is this machine: the question on
 * opening the window is "is my drive still being shared, and is anyone using
 * it?".
 */
export function TitleBar({
  hostName,
  vaultName,
  serving,
  busy,
}: {
  hostName: string
  vaultName: string | null
  serving: boolean
  /** True while at least one device has something in flight. */
  busy: boolean
}): React.JSX.Element {
  const alive = serving && busy

  return (
    <div className="drag relative z-20 flex h-9 shrink-0 items-center gap-2.5 border-b border-line px-3">
      {/*
        The mark breathes only while data is actually moving. Idle it is
        perfectly still — an animation that never stops stops meaning anything.
      */}
      <motion.div
        animate={alive ? { opacity: [0.55, 1, 0.55] } : { opacity: 0.55 }}
        transition={
          alive ? { duration: 2.4, repeat: Infinity, ease: 'easeInOut' } : { duration: 0.4 }
        }
        className="text-basaltDeep"
      >
        <HexMark size={14} />
      </motion.div>

      <div className="flex min-w-0 items-baseline gap-2">
        <span className="truncate text-[12px] font-semibold tracking-tight text-text">
          {vaultName ?? hostName}
        </span>
        <span
          className={cn(
            'shrink-0 font-mono text-[10px] uppercase tracking-[0.14em]',
            serving ? 'text-textFaint' : 'text-danger',
          )}
        >
          {serving ? (vaultName ? 'sharing' : 'ready') : 'not sharing'}
        </span>
      </div>

      <div className="flex-1" />

      <div className="no-drag flex h-full items-center gap-1.5">
        <Dot
          onClick={() => void windowAction('minimize')}
          color="#FFBD2E"
          symbol="−"
          label="Minimise"
        />
        <Dot
          onClick={() => void windowAction('toggleMaximize')}
          color="#28C840"
          symbol="⤢"
          label="Maximise"
        />
        {/*
          Closing hides the window; the drive stays shared and the tray icon
          brings it back. Said in the tooltip because a close button that does
          not close is a surprise, and a surprise about whether your files are
          still being served is the wrong kind.
        */}
        <Dot
          onClick={() => void windowAction('close')}
          color="#FF5F57"
          symbol="×"
          label="Hide — sharing continues in the background"
        />
      </div>
    </div>
  )
}

function Dot({
  onClick,
  color,
  symbol,
  label,
}: {
  onClick: () => void
  color: string
  symbol: string
  label: string
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className="no-drag group flex h-6 w-6 items-center justify-center"
    >
      <span
        className="relative flex h-3 w-3 items-center justify-center rounded-full transition-transform group-hover:scale-110"
        style={{ backgroundColor: color }}
      >
        <span className="absolute text-[7px] font-black leading-none text-black/50 opacity-0 transition-opacity group-hover:opacity-100">
          {symbol}
        </span>
      </span>
    </button>
  )
}
