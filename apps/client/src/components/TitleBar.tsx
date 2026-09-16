import { HexMark } from './HexMark'

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
 * Frameless title bar.
 *
 * Apple-style traffic dots, but in **Windows order** (minimise, maximise,
 * close) so muscle memory still works on the platform this actually runs on.
 * Carried over from Frostbyte; only the window API changed, since Tauri
 * replaces the Electron IPC bridge.
 */
export function TitleBar(): React.JSX.Element {
  return (
    <div className="drag relative z-20 flex h-8 items-center justify-between border-b border-line pl-3">
      <div className="flex items-center gap-2">
        <HexMark size={13} className="text-basaltDeep" />
        <span className="text-[10px] font-bold uppercase tracking-[0.3em] text-textDim">
          Basalt
        </span>
      </div>

      <div className="no-drag flex h-full items-center gap-1.5 pr-2.5">
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
        {/* The glyph only appears on hover, so the resting state stays quiet. */}
        <span className="absolute text-[7px] font-black leading-none text-black/50 opacity-0 transition-opacity group-hover:opacity-100">
          {symbol}
        </span>
      </span>
    </button>
  )
}
