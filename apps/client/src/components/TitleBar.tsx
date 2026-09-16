import { motion } from 'framer-motion'
import { HexMark } from './HexMark'
import { Sparkline } from './Sparkline'
import { cn } from '@/lib/utils'

/**
 * True when running inside the Tauri shell rather than a plain browser.
 *
 * The window controls are the one part of the UI that cannot work without the
 * native shell, and `getCurrentWindow()` throws outright when the Tauri
 * internals are absent — which takes the whole app down with it. Guarding here
 * means the interface can be built and reviewed in an ordinary browser, which
 * is a far faster loop than rebuilding the desktop binary for every change.
 */
function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/** Window control, or a no-op outside the desktop shell. */
async function windowAction(action: 'minimize' | 'toggleMaximize' | 'close'): Promise<void> {
  if (!inTauri()) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await getCurrentWindow()[action]()
}

/**
 * The title bar shows the **connection**, not the application's own name.
 *
 * Frostbyte puts "FROSTBYTE" here because a compression tool is a thing you
 * open, use and close — the app is the subject. Basalt is the opposite: the
 * subject is the drive at the other end of the room, and the question you have
 * on opening the window is always "is it there, and is anything moving?".
 *
 * So the wordmark is gone. In its place: the vault's name, whether it is
 * reachable, and a live trace of what is actually crossing the link.
 */
export function TitleBar({
  vaultName,
  connected,
  throughput,
  samples,
}: {
  vaultName: string
  connected: boolean
  throughput: number
  samples: number[]
}): React.JSX.Element {
  const active = connected && throughput > 0.5

  return (
    <div className="drag relative z-20 flex h-9 items-center gap-2.5 border-b border-line px-3">
      {/*
        The mark breathes only while data is moving. Idle it is perfectly
        still — an animation that never stops stops meaning anything.
      */}
      <motion.div
        animate={active ? { opacity: [0.55, 1, 0.55] } : { opacity: 0.55 }}
        transition={
          active
            ? { duration: 2.4, repeat: Infinity, ease: 'easeInOut' }
            : { duration: 0.4 }
        }
        className="text-basaltDeep"
      >
        <HexMark size={14} />
      </motion.div>

      <div className="flex items-baseline gap-2">
        <span className="text-[12px] font-semibold tracking-tight text-text">
          {vaultName}
        </span>
        {!connected && (
          <span className="font-mono text-[10px] uppercase tracking-[0.14em] text-textFaint">
            offline
          </span>
        )}
      </div>

      <div className="flex-1" />

      {/* The live trace. Sits next to the window controls because it is
          ambient information, not something you act on. */}
      {connected && (
        <div className="no-drag flex items-center gap-2 pr-1">
          <Sparkline samples={samples} width={56} height={14} />
          <span
            className={cn(
              'tnum w-[68px] text-right font-mono text-[10px] tabular-nums transition-colors',
              active ? 'text-textDim' : 'text-textFaint',
            )}
          >
            {throughput > 0.05 ? `${throughput.toFixed(1)} MB/s` : 'idle'}
          </span>
        </div>
      )}

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
        <Dot
          onClick={() => void windowAction('close')}
          color="#FF5F57"
          symbol="×"
          label="Close"
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
