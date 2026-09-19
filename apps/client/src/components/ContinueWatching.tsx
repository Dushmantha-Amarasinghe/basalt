import { motion } from 'framer-motion'
import { Play, X } from 'lucide-react'
import type { LibraryItem, Watched } from '@/lib/api'
import { Poster } from './Poster'

/** One thing part-watched, resolved back to what it belongs to. */
export interface Resumable {
  watched: Watched
  /** The library item it came from, when the index recognises it. */
  item: LibraryItem
  /** `S01E03`, for an episode. Empty for a film. */
  episode: string
}

/**
 * Turns watch progress back into something worth showing.
 *
 * Progress is stored by *file*, because a file is what gets watched. The
 * library is organised by *title*. This is the join: it finds which item each
 * part-watched file belongs to, and drops anything the index no longer
 * recognises — a resume point for a deleted film is not worth a card.
 *
 * At most one card per series. Somebody midway through episode three has one
 * thing to carry on with, not a row of the same show.
 */
export function resumable(items: LibraryItem[], watched: Watched[]): Resumable[] {
  const byPath = new Map<string, { item: LibraryItem; episode: string }>()
  for (const item of items) {
    if (item.path) byPath.set(item.path, { item, episode: '' })
    for (const season of item.seasons) {
      for (const episode of season.episodes) {
        byPath.set(episode.path, {
          item,
          episode: `S${String(season.number).padStart(2, '0')}E${String(
            episode.number,
          ).padStart(2, '0')}`,
        })
      }
    }
  }

  const seen = new Set<string>()
  const out: Resumable[] = []
  // `watched` arrives newest first, so the first match for a series is the one
  // most recently touched — which is the episode to carry on from.
  for (const entry of watched) {
    const found = byPath.get(entry.path)
    if (!found || seen.has(found.item.id)) continue
    seen.add(found.item.id)
    out.push({ watched: entry, item: found.item, episode: found.episode })
  }
  return out
}

/** How long is left, in words. */
function remaining(watched: Watched): string {
  if (watched.duration <= 0) return `${Math.round(watched.fraction * 100)}% through`
  const left = Math.max(0, watched.duration - watched.position)
  const minutes = Math.round(left / 60)
  if (minutes < 1) return 'nearly done'
  if (minutes < 60) return `${minutes} min left`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ${minutes % 60}m left`
}

/**
 * The row at the top of the library: what you were in the middle of.
 *
 * Wide cards rather than posters, because this row answers "what was I
 * watching" and the answer wants a title, an episode and how long is left —
 * none of which fit under a poster.
 */
export function ContinueWatching({
  entries,
  onPlay,
  onForget,
}: {
  entries: Resumable[]
  onPlay: (path: string) => void
  onForget: (path: string) => void
}): React.JSX.Element | null {
  if (entries.length === 0) return null

  return (
    <section className="mb-7">
      <h3 className="mb-1 font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
        Continue watching
      </h3>

      {/* `pt-2` is not decoration: the remove button hangs six pixels above
          each card, and setting `overflow-x` forces `overflow-y` to clip as
          well — so without room made for it the button was sliced in half
          along its top edge. */}
      <div className="flex gap-3 overflow-x-auto pb-1 pt-2">
        {entries.map(({ watched, item, episode }, index) => (
          <motion.div
            key={watched.path}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{
              duration: 0.28,
              delay: Math.min(index, 8) * 0.03,
              ease: [0.22, 1, 0.36, 1],
            }}
            className="group relative w-[228px] shrink-0"
          >
            <button
              onClick={() => onPlay(watched.path)}
              className="flex w-full items-stretch gap-3 overflow-hidden rounded-md border border-line bg-panel text-left transition-colors hover:bg-panel2"
            >
              <div className="w-[56px] shrink-0">
                <Poster
                  title={item.title}
                  year={item.year ?? undefined}
                  id={item.id}
                  hasArt={item.hasArt}
                />
              </div>

              <div className="flex min-w-0 flex-1 flex-col justify-center py-2.5 pr-2">
                <div className="truncate text-[12.5px] font-medium text-text">
                  {item.title}
                </div>
                <div className="tnum mt-0.5 truncate font-mono text-[10px] text-textFaint">
                  {episode ? `${episode} · ` : ''}
                  {remaining(watched)}
                </div>

                <div className="mt-2 h-[3px] w-full overflow-hidden rounded-full bg-ink2">
                  <div
                    className="h-full rounded-full bg-basalt"
                    style={{ width: `${Math.round(watched.fraction * 100)}%` }}
                  />
                </div>
              </div>

              {/* The play affordance sits over the poster on hover, matching
                  the grid below it. */}
              <span className="pointer-events-none absolute left-0 top-0 flex h-full w-[56px] items-center justify-center bg-black/55 opacity-0 transition-opacity group-hover:opacity-100">
                <Play size={13} className="ml-0.5 text-basalt" fill="currentColor" />
              </span>
            </button>

            <button
              onClick={() => onForget(watched.path)}
              title="Remove from Continue watching"
              aria-label={`Remove ${item.title} from Continue watching`}
              className="absolute -right-1.5 -top-1.5 rounded-full border border-line bg-panel2 p-1 text-textFaint opacity-0 transition-opacity hover:text-text group-hover:opacity-100 focus:opacity-100"
            >
              <X size={10} />
            </button>
          </motion.div>
        ))}
      </div>
    </section>
  )
}
