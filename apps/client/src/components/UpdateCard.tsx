import { AnimatePresence, motion } from 'framer-motion'
import { ArrowUpCircle, Loader2 } from 'lucide-react'
import { downloadUpdate, installUpdate, offered, useUpdate } from '@/lib/updates'
import { formatBytes } from '@/lib/utils'

/**
 * A newer Basalt, where it is seen: in the sidebar, above the drive.
 *
 * Only there while an update is on offer, and the whole of it can be done from
 * here: download, watch it come in, restart into it. "What's new" goes to
 * Settings, where the release notes are.
 */
export function UpdateCard({ onWhatsNew }: { onWhatsNew: () => void }): React.JSX.Element {
  const state = useUpdate()

  return (
    <AnimatePresence initial={false}>
      {offered(state) && (
        <motion.div
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: 'auto' }}
          exit={{ opacity: 0, height: 0 }}
          transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
          className="overflow-hidden"
        >
          <div className="mb-2 rounded-lg border border-basalt/25 bg-basalt/[0.06] p-2.5">
            <div className="flex items-center gap-1.5 text-[12px] font-medium text-text">
              <ArrowUpCircle size={13} className="shrink-0 text-basalt" />
              {state.kind === 'ready' ? 'Update ready' : 'Update available'}
            </div>
            <div className="tnum mt-0.5 font-mono text-[10px] text-textFaint">
              Version {state.release.version} · {formatBytes(state.release.installerBytes)}
            </div>

            {state.kind === 'downloading' && (
              <div className="mt-2 h-1 overflow-hidden rounded-full bg-white/10">
                <div
                  className="h-full rounded-full bg-basalt transition-[width] duration-200"
                  style={{
                    width: `${state.total > 0 ? Math.round((state.had / state.total) * 100) : 0}%`,
                  }}
                />
              </div>
            )}

            <div className="mt-2 flex items-center gap-1.5">
              {state.kind === 'ready' ? (
                <button
                  onClick={() => void installUpdate()}
                  className="flex-1 rounded-md border border-basalt/40 bg-basalt/15 px-2 py-1.5 text-[11px] text-text transition-colors hover:bg-basalt/25"
                >
                  Restart to update
                </button>
              ) : (
                <button
                  onClick={() => void downloadUpdate()}
                  disabled={state.kind === 'downloading'}
                  className="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-line bg-ink2 px-2 py-1.5 text-[11px] text-textDim transition-colors hover:border-lineBright hover:text-text disabled:opacity-70"
                >
                  {state.kind === 'downloading' && <Loader2 size={11} className="animate-spin" />}
                  {state.kind === 'downloading'
                    ? `${state.total > 0 ? Math.round((state.had / state.total) * 100) : 0}%`
                    : 'Download'}
                </button>
              )}
              <button
                onClick={onWhatsNew}
                className="shrink-0 rounded-md px-2 py-1.5 text-[11px] text-textFaint transition-colors hover:text-text"
              >
                What’s new
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
