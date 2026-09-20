import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { AlertCircle, ArrowUpCircle, Check, ExternalLink, Github, Loader2 } from 'lucide-react'
import { api, inTauri, type Release } from '@/lib/api'
import { cn, formatBytes } from '@/lib/utils'

export const WEBSITE = 'https://reforatech.com'
export const REPO = 'https://github.com/Dushmantha-Amarasinghe/basalt'
export const ISSUES = `${REPO}/issues/new`

/**
 * Who made this, which version it is, and whether there is a newer one.
 *
 * Checked once on open rather than on a timer. An update is not urgent — it
 * is waiting whenever somebody next looks — and an app that polls a release
 * API in the background is an app making requests nobody asked for.
 *
 * Downloading and installing are separate presses. The download is verified
 * against the checksum published beside it, and only then is there anything
 * to install; running an installer is the last thing this app does before it
 * closes, so it should never happen as a side effect of a check.
 */
export function About({ product }: { product: string }): React.JSX.Element {
  const [version, setVersion] = useState('')
  const [state, setState] = useState<State>({ kind: 'idle' })
  const progress = useRef<() => void>(() => {})

  useEffect(() => {
    void api.appVersion().then(setVersion).catch(() => {})
  }, [])

  const check = useCallback(async (quiet: boolean) => {
    setState(quiet ? { kind: 'idle' } : { kind: 'checking' })
    try {
      const release = await api.checkUpdate()
      setState(release ? { kind: 'available', release } : { kind: 'current' })
    } catch (e) {
      // Only when somebody asked. A background check that cannot reach
      // GitHub is not news, and saying so on every launch would train people
      // to ignore the one time it matters.
      setState(quiet ? { kind: 'idle' } : { kind: 'failed', why: String(e) })
    }
  }, [])

  // One quiet look on open, so the offer is already there when wanted.
  useEffect(() => {
    void check(true)
  }, [check])

  // Progress arrives from the shell as the bytes land.
  useEffect(() => {
    if (!inTauri()) return undefined
    let stop: (() => void) | undefined
    void (async () => {
      const { listen } = await import('@tauri-apps/api/event')
      stop = await listen<[number, number]>('basalt://update-progress', (event) => {
        const [had, total] = event.payload
        setState((s) =>
          s.kind === 'downloading' ? { ...s, had, total } : s,
        )
      })
    })()
    progress.current = () => stop?.()
    return () => stop?.()
  }, [])

  const download = async (release: Release): Promise<void> => {
    setState({ kind: 'downloading', release, had: 0, total: release.installerBytes })
    try {
      const path = await api.downloadUpdate(release)
      setState({ kind: 'ready', release, path })
    } catch (e) {
      setState({ kind: 'failed', why: String(e) })
    }
  }

  return (
    <div className="px-4 py-3.5">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="text-[13px] font-semibold text-text">{product}</div>
          <div className="tnum mt-0.5 font-mono text-[11px] text-textFaint">
            {version ? `v${version}` : '—'}
          </div>
        </div>

        <button
          onClick={() => void check(false)}
          disabled={state.kind === 'checking' || state.kind === 'downloading'}
          className="shrink-0 rounded-md border border-line bg-ink2 px-3 py-1.5 text-[11.5px] text-textDim transition-colors hover:border-lineBright hover:text-text disabled:opacity-40"
        >
          {state.kind === 'checking' ? 'Checking…' : 'Check for updates'}
        </button>
      </div>

      <AnimatePresence mode="wait">
        <motion.div
          key={state.kind}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.14 }}
        >
          {state.kind === 'current' && (
            <Line icon={<Check size={12} className="text-textFaint" />}>
              You’re on the latest version.
            </Line>
          )}

          {state.kind === 'failed' && (
            <Line icon={<AlertCircle size={12} className="text-danger" />} danger>
              {state.why}
            </Line>
          )}

          {(state.kind === 'available' ||
            state.kind === 'downloading' ||
            state.kind === 'ready') && (
            <Offer
              release={state.release}
              state={state}
              onDownload={() => void download(state.release)}
              onInstall={(path) => void api.installUpdate(path).catch(() => {})}
            />
          )}
        </motion.div>
      </AnimatePresence>

      <div className="mt-3.5 border-t border-line pt-3">
        <div className="flex flex-wrap gap-2">
          <Link href={WEBSITE} icon={<ExternalLink size={11} />}>
            Website
          </Link>
          <Link href={REPO} icon={<Github size={11} />}>
            Source code
          </Link>
          <Link href={ISSUES} icon={<AlertCircle size={11} />}>
            Report a problem
          </Link>
        </div>

        <p className="mt-3 font-mono text-[10px] text-textFaint">
          Windows · GPLv3 · Refora Technologies
        </p>
        <p className="mt-1 font-mono text-[10px] text-textFaint">
          © 2026 Refora Technologies
        </p>
      </div>
    </div>
  )
}

type State =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'failed'; why: string }
  | { kind: 'available'; release: Release }
  | { kind: 'downloading'; release: Release; had: number; total: number }
  | { kind: 'ready'; release: Release; path: string }

/** The offer itself: what is new, and what to do about it. */
function Offer({
  release,
  state,
  onDownload,
  onInstall,
}: {
  release: Release
  state: State
  onDownload: () => void
  onInstall: (path: string) => void
}): React.JSX.Element {
  const busy = state.kind === 'downloading'
  const done = state.kind === 'ready'
  const percent =
    state.kind === 'downloading' && state.total > 0
      ? Math.round((state.had / state.total) * 100)
      : 0

  return (
    <div className="mt-3 rounded-md border border-basalt/25 bg-basalt/[0.06] p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 text-[12.5px] text-text">
            <ArrowUpCircle size={13} className="shrink-0 text-basalt" />
            Version {release.version} is available
          </div>
          <div className="tnum mt-0.5 font-mono text-[10px] text-textFaint">
            {formatBytes(release.installerBytes)}
          </div>
        </div>

        {done ? (
          <button
            onClick={() => onInstall((state as { path: string }).path)}
            className="shrink-0 rounded-md border border-basalt/40 bg-basalt/15 px-3 py-1.5 text-[11.5px] text-text transition-colors hover:bg-basalt/25"
          >
            Install and restart
          </button>
        ) : (
          <button
            onClick={onDownload}
            disabled={busy}
            className="flex shrink-0 items-center gap-1.5 rounded-md border border-line bg-ink2 px-3 py-1.5 text-[11.5px] text-textDim transition-colors hover:border-lineBright hover:text-text disabled:opacity-60"
          >
            {busy && <Loader2 size={11} className="animate-spin" />}
            {busy ? `${percent}%` : 'Download'}
          </button>
        )}
      </div>

      {busy && (
        <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-white/10">
          <div
            className="h-full rounded-full bg-basalt transition-[width] duration-200"
            style={{ width: `${percent}%` }}
          />
        </div>
      )}

      {/* What changed, straight from the release. Shown here rather than
          behind a link, because "there is an update" without "and here is
          what it does" is not enough to decide on. */}
      {release.notes && (
        <div className="mt-3 max-h-[180px] overflow-y-auto border-t border-white/[0.07] pt-2.5">
          <pre className="whitespace-pre-wrap break-words font-sans text-[11.5px] leading-relaxed text-textDim">
            {release.notes}
          </pre>
        </div>
      )}
    </div>
  )
}

function Line({
  icon,
  danger,
  children,
}: {
  icon: React.ReactNode
  danger?: boolean
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <div
      className={cn(
        'mt-2 flex items-start gap-1.5 text-[11.5px]',
        danger ? 'text-danger' : 'text-textFaint',
      )}
    >
      <span className="mt-px shrink-0">{icon}</span>
      <span className="min-w-0">{children}</span>
    </div>
  )
}

function Link({
  href,
  icon,
  children,
}: {
  href: string
  icon: React.ReactNode
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <button
      onClick={() => void openExternal(href)}
      className="flex items-center gap-1.5 rounded-md border border-line bg-ink2 px-2.5 py-1.5 text-[11px] text-textDim transition-colors hover:border-lineBright hover:text-text"
    >
      {icon}
      {children}
    </button>
  )
}

async function openExternal(url: string): Promise<void> {
  if (!inTauri()) {
    window.open(url, '_blank')
    return
  }
  const { openUrl } = await import('@tauri-apps/plugin-opener')
  await openUrl(url)
}
