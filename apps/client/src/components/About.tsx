import { useEffect, useState } from 'react'
import { isAndroid } from '@/lib/platform'
import { AnimatePresence, motion } from 'framer-motion'
import { AlertCircle, ArrowUpCircle, Check, ExternalLink, Github, Loader2 } from 'lucide-react'
import { api, inTauri, type Release } from '@/lib/api'
import { parseNotes } from '@/lib/notes'
import {
  checkForUpdate,
  downloadUpdate,
  installUpdate,
  offered,
  useUpdate,
  type UpdateState,
} from '@/lib/updates'
import { cn, formatBytes } from '@/lib/utils'

export const WEBSITE = 'https://basalt.reforatech.com'
export const REPO = 'https://github.com/refora-technologies/basalt'
export const ISSUES = `${REPO}/issues/new`

/**
 * Who made this, which version it is, and whether there is a newer one.
 *
 * The check itself lives in `lib/updates`, shared with the sidebar and the
 * phone's banner, and runs on its own when the app opens; this is where the
 * whole offer is, release notes and all, and where to check by hand.
 *
 * Downloading and installing are separate presses. The download is verified
 * against the checksum published beside it, and only then is there anything
 * to install; running an installer is the last thing this app does before it
 * closes, so it should never happen as a side effect of a check.
 */
export function About({ product }: { product: string }): React.JSX.Element {
  const [version, setVersion] = useState('')
  const state = useUpdate()

  useEffect(() => {
    void api.appVersion().then(setVersion).catch(() => {})
  }, [])

  const check = (quiet: boolean): Promise<void> => checkForUpdate(quiet)

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

          {offered(state) && (
            <Offer
              release={state.release}
              state={state}
              onDownload={() => void downloadUpdate()}
              onInstall={() => void installUpdate()}
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
          {isAndroid() ? 'Android' : 'Windows'} · GPLv3 · Refora Technologies
        </p>
        <p className="mt-1 font-mono text-[10px] text-textFaint">
          © 2026 Refora Technologies
        </p>
      </div>
    </div>
  )
}

type State = UpdateState

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
  onInstall: () => void
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
            onClick={onInstall}
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
          <Notes notes={release.notes} />
        </div>
      )}
    </div>
  )
}

/**
 * The release notes as written on GitHub, rendered rather than shown raw.
 *
 * Headings, bullets and bold are all release notes use, and all this draws.
 * See `lib/notes` for why there is no Markdown dependency behind this.
 */
function Notes({ notes }: { notes: string }): React.JSX.Element {
  return (
    <div className="space-y-1.5 text-[11.5px] leading-relaxed text-textDim">
      {parseNotes(notes).map((block, at) => {
        const runs = block.spans.map((span, i) => (
          <span
            key={i}
            className={cn(
              span.bold && 'font-semibold text-text',
              span.code && 'rounded bg-white/[0.07] px-1 font-mono text-[10.5px]',
            )}
          >
            {span.text}
          </span>
        ))

        if (block.kind === 'heading') {
          return (
            <div
              key={at}
              className="pt-1.5 text-[10px] font-semibold uppercase tracking-wider text-textFaint first:pt-0"
            >
              {runs}
            </div>
          )
        }
        if (block.kind === 'bullet') {
          return (
            <div key={at} className="flex gap-1.5">
              <span className="shrink-0 text-textFaint">·</span>
              <span className="min-w-0">{runs}</span>
            </div>
          )
        }
        return (
          <p key={at} className="break-words">
            {runs}
          </p>
        )
      })}
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

/**
 * Opens a link in the default browser.
 *
 * Only the addresses listed in the app's capability file may be opened: the
 * opener plugin's `allow-open-url` allows none by itself, which is how these
 * buttons came to do nothing at all. A link added here needs adding there.
 */
async function openExternal(url: string): Promise<void> {
  if (!inTauri()) {
    window.open(url, '_blank')
    return
  }
  const { openUrl } = await import('@tauri-apps/plugin-opener')
  await openUrl(url)
}
