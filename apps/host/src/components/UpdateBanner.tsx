import { AnimatePresence, motion } from 'framer-motion'
import { ArrowUpCircle, Loader2 } from 'lucide-react'
import { downloadUpdate, installUpdate, offered, useUpdate } from '@/lib/updates'
import { formatBytes } from '@/lib/utils'

/**
 * A newer Basalt Host, at the top of the window where it is seen, rather than
 * at the foot of Settings where nobody scrolls to look.
 *
 * The whole update can be done from here; "What's new" goes down to the
 * release notes in the About section.
 */
export function UpdateBanner(): React.JSX.Element {
  const state = useUpdate()
  const percent =
    state.kind === 'downloading' && state.total > 0 ? Math.round((state.had / state.total) * 100) : 0

  return (
    <AnimatePresence initial={false}>
      {offered(state) && (
        <motion.div
          initial={{ opacity: 0, y: -6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
          className="rounded-lg border border-basalt/25 bg-basalt/[0.06] px-4 py-3"
        >
          <div className="flex items-center gap-3.5">
            <ArrowUpCircle size={16} className="shrink-0 text-basalt" />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-text">
                {state.kind === 'ready'
                  ? `Basalt Host ${state.release.version} is ready to install`
                  : `Basalt Host ${state.release.version} is available`}
              </div>
              <div className="tnum mt-0.5 font-mono text-[10.5px] text-textFaint">
                {formatBytes(state.release.installerBytes)}
                {state.kind === 'ready' && ' · closes, updates and opens again by itself'}
              </div>
            </div>
            <button
              onClick={() =>
                document.getElementById('host-about')?.scrollIntoView({ behavior: 'smooth', block: 'center' })
              }
              className="shrink-0 rounded-md px-2.5 py-1.5 text-[11.5px] text-textFaint transition-colors hover:text-text"
            >
              What’s new
            </button>
            {state.kind === 'ready' ? (
              <button
                onClick={() => void installUpdate()}
                className="shrink-0 rounded-md border border-basalt/40 bg-basalt/15 px-3 py-1.5 text-[11.5px] text-text transition-colors hover:bg-basalt/25"
              >
                Install and restart
              </button>
            ) : (
              <button
                onClick={() => void downloadUpdate()}
                disabled={state.kind === 'downloading'}
                className="flex shrink-0 items-center gap-1.5 rounded-md border border-line bg-ink2 px-3 py-1.5 text-[11.5px] text-textDim transition-colors hover:border-lineBright hover:text-text disabled:opacity-70"
              >
                {state.kind === 'downloading' && <Loader2 size={11} className="animate-spin" />}
                {state.kind === 'downloading' ? `${percent}%` : 'Download'}
              </button>
            )}
          </div>
          {state.kind === 'downloading' && (
            <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-white/10">
              <div
                className="h-full rounded-full bg-basalt transition-[width] duration-200"
                style={{ width: `${percent}%` }}
              />
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  )
}
