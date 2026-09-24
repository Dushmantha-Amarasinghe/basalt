import { useCallback, useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Check, ChevronLeft, Clapperboard, Loader2, Play, Tv } from 'lucide-react'
import {
  CONFIDENT,
  isFinished,
  type LibraryEpisode,
  type LibraryItem,
  type LibrarySeason,
  type Watched,
} from '@/lib/api'
import { cn, formatBytes } from '@/lib/utils'
import { Poster } from './Poster'
import { subtitleBadge, subtitleTitle } from '@/lib/subtitles'
import { ContinueWatching, resumable } from './ContinueWatching'

/**
 * Films and series, as a wall of posters.
 *
 * Real artwork when the host has downloaded it, and one drawn from the title
 * when it has not — see [`Poster`]. The second is the default, because
 * downloading means telling TMDb every title on the drive and that is opt-in.
 */
export function LibraryView({
  kind,
  items,
  enabled,
  scanning,
  watched,
  continueWatching,
  playing,
  onPlay,
  onForget,
}: {
  kind: 'film' | 'series'
  items: LibraryItem[]
  enabled: boolean
  scanning: boolean
  /** How far through each file, by vault path. */
  watched: Map<string, Watched>
  /** Everything part-watched, newest first, for the row at the top. */
  continueWatching: Watched[]
  /**
   * Whether the player is up.
   *
   * The episode list is a full-screen sheet, and it used to stay on screen
   * over the player — you picked an episode and then watched it from behind
   * the list you picked it from. Hidden rather than closed, so dismissing the
   * player puts you back on the same series where you left off.
   */
  playing: boolean
  onPlay: (path: string) => void
  onForget: (path: string) => void
}): React.JSX.Element {
  const [open, setOpen] = useState<LibraryItem | null>(null)

  /**
   * How far through an item is, for the bar across its poster.
   *
   * For a series that is whichever episode is part-watched — which is what
   * makes a show in Continue watching resume rather than start again.
   */
  const progressOf = useCallback(
    (item: LibraryItem): number => {
      if (item.kind === 'film') {
        return item.path ? (watched.get(item.path)?.fraction ?? 0) : 0
      }
      const partial = item.seasons
        .flatMap((season) => season.episodes)
        .map((episode) => watched.get(episode.path))
        .find((entry) => entry && !isFinished(entry) && entry.fraction > 0)
      return partial?.fraction ?? 0
    },
    [watched],
  )

  // Only what belongs to this section: films on Movies, series on TV Series.
  const carryOn = useMemo(
    () => resumable(items, continueWatching),
    [items, continueWatching],
  )

  // Newest first: what you just added is what you came to watch.
  const ordered = useMemo(
    () => [...items].sort((a, b) => b.added - a.added || a.title.localeCompare(b.title)),
    [items],
  )

  if (!enabled) return <Unavailable kind={kind} />

  if (ordered.length === 0) {
    return scanning ? (
      <Centered>
        <Loader2 size={16} className="animate-spin text-textFaint" />
        <p className="text-[12px] text-textFaint">Looking through the drive…</p>
      </Centered>
    ) : (
      <Centered>
        <span className="text-textFaint">
          {kind === 'film' ? <Clapperboard size={20} /> : <Tv size={20} />}
        </span>
        <p className="text-[13px] text-textDim">
          No {kind === 'film' ? 'films' : 'series'} recognised yet.
        </p>
        <p className="max-w-[320px] text-center text-[11.5px] leading-relaxed text-textFaint">
          Everything on the drive is still in Files. Names like{' '}
          <span className="font-mono">Arrival (2016)</span> or{' '}
          <span className="font-mono">Show/Season 01/S01E01</span> are the ones it
          recognises.
        </p>
      </Centered>
    )
  }

  return (
    <div className="h-full overflow-y-auto px-7 py-6">
      {/* A scan running over an existing library says so without hiding it. */}
      <AnimatePresence>
        {scanning && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            <div className="mb-4 flex items-center gap-2 text-[11.5px] text-textFaint">
              <Loader2 size={12} className="animate-spin" />
              Checking the drive for changes…
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <ContinueWatching entries={carryOn} onPlay={onPlay} onForget={onForget} />

      <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-x-4 gap-y-6">
        {ordered.map((item, index) => (
          <Card
            key={item.id}
            item={item}
            index={index}
            progress={progressOf(item)}
            onOpen={() => {
              if (item.kind === 'series') setOpen(item)
              else if (item.path) onPlay(item.path)
            }}
          />
        ))}
      </div>

      <AnimatePresence>
        {open && !playing && (
          <SeriesSheet
            item={open}
            watched={watched}
            onClose={() => setOpen(null)}
            onPlay={onPlay}
          />
        )}
      </AnimatePresence>
    </div>
  )
}

function Centered({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3">{children}</div>
  )
}

function Unavailable({ kind }: { kind: 'film' | 'series' }): React.JSX.Element {
  return (
    <Centered>
      <span className="text-textFaint">
        {kind === 'film' ? <Clapperboard size={20} /> : <Tv size={20} />}
      </span>
      <p className="text-[13px] text-textDim">Not switched on.</p>
      <p className="max-w-[340px] text-center text-[11.5px] leading-relaxed text-textFaint">
        {/* Said plainly, because it is not this app's decision to make: the
            host is the machine whose drive would be read. */}
        Turn on <span className="text-textDim">Recognise films and series</span> in
        Basalt Host on the machine with the drive.
      </p>
    </Centered>
  )
}

function Card({
  item,
  index,
  progress,
  onOpen,
}: {
  item: LibraryItem
  index: number
  /** 0-1, drawn as a bar across the bottom of the poster. */
  progress: number
  onOpen: () => void
}): React.JSX.Element {
  const episodes = item.seasons.reduce((total, s) => total + s.episodes.length, 0)

  return (
    <motion.button
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      // Staggered, but only over the first screenful: a library of nine
      // hundred must not animate for half a minute.
      transition={{ duration: 0.3, delay: Math.min(index, 18) * 0.02, ease: [0.22, 1, 0.36, 1] }}
      whileHover={{ y: -3 }}
      onClick={onOpen}
      className="group block text-left"
    >
      <div className="relative overflow-hidden rounded-md">
        <Poster
          title={item.title}
          year={item.year ?? undefined}
          id={item.id}
          hasArt={item.hasArt}
        />

        {/* The play affordance appears on hover; the poster is the subject. */}
        <div className="absolute inset-0 flex items-center justify-center bg-black/55 opacity-0 transition-opacity duration-200 group-hover:opacity-100">
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-basalt text-ink">
            <Play size={15} className="ml-0.5" fill="currentColor" />
          </span>
        </div>

        {/* A hairline across the bottom of the poster, the way every
            streaming service does it. Only when there is something to say: an
            empty bar on every card would be noise. */}
        {progress > 0.01 && progress < 0.94 && (
          <div className="absolute inset-x-0 bottom-0 h-[3px] bg-black/60">
            <div
              className="h-full bg-basalt"
              style={{ width: `${Math.round(progress * 100)}%` }}
            />
          </div>
        )}

        {/* Says a subtitle file exists for this, without claiming which
            languages — the player reads that from the file when it opens it. */}
        {subtitleBadge(item) && (
          <span
            title={
              item.kind === 'film'
                ? subtitleTitle(item.subtitles)
                : 'Some episodes have subtitle files on the drive'
            }
            className="absolute right-2 top-2 rounded-[4px] bg-black/70 px-1.5 py-[2px] font-mono text-[8.5px] uppercase tracking-[0.1em] text-textDim"
          >
            {subtitleBadge(item)}
          </span>
        )}

        {item.confidence < CONFIDENT && (
          <span
            title="Recognised from the filename, but not confidently"
            className="absolute left-2 top-2 rounded-[4px] bg-black/70 px-1.5 py-[2px] font-mono text-[8.5px] uppercase tracking-[0.1em] text-textDim"
          >
            a guess
          </span>
        )}
      </div>

      <div className="mt-2 px-0.5">
        <div className="truncate text-[12.5px] font-medium text-text">{item.title}</div>
        <div className="tnum mt-0.5 font-mono text-[10px] text-textFaint">
          {item.kind === 'series'
            ? `${item.seasons.length} ${item.seasons.length === 1 ? 'season' : 'seasons'} · ${episodes} ep`
            : [item.year, formatBytes(item.size)].filter(Boolean).join(' · ')}
        </div>
      </div>
    </motion.button>
  )
}

/** A series opened up: its seasons and episodes. */
function SeriesSheet({
  item,
  watched,
  onClose,
  onPlay,
}: {
  item: LibraryItem
  watched: Map<string, Watched>
  onClose: () => void
  onPlay: (path: string) => void
}): React.JSX.Element {
  const [season, setSeason] = useState<LibrarySeason | undefined>(item.seasons[0])

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.16 }}
      className="fixed inset-0 z-50 bg-ink/95"
      onMouseDown={onClose}
    >
      <motion.div
        initial={{ y: 14 }}
        animate={{ y: 0 }}
        transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
        className="mx-auto flex h-full max-w-[760px] flex-col px-8 py-7"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <button
          onClick={onClose}
          className="mb-5 flex w-fit items-center gap-1.5 rounded-sm px-2 py-1 text-[11.5px] text-textFaint transition-colors hover:text-text"
        >
          <ChevronLeft size={13} />
          Back
        </button>

        <div className="flex gap-5">
          <div className="w-[128px] shrink-0 overflow-hidden rounded-md">
            <Poster
              title={item.title}
              year={item.year ?? undefined}
              id={item.id}
              hasArt={item.hasArt}
            />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-[22px] font-semibold tracking-tighter text-text">
              {item.title}
            </h2>
            <div className="tnum mt-1 font-mono text-[11px] text-textFaint">
              {[item.year, `${item.seasons.length} ${item.seasons.length === 1 ? 'season' : 'seasons'}`, formatBytes(item.size)]
                .filter(Boolean)
                .join(' · ')}
            </div>

            {item.seasons.length > 1 && (
              <div className="mt-4 flex flex-wrap gap-1.5">
                {item.seasons.map((s) => (
                  <button
                    key={s.number}
                    onClick={() => setSeason(s)}
                    className={cn(
                      'rounded-sm px-2.5 py-1 font-mono text-[11px] transition-colors',
                      s.number === season?.number
                        ? 'basalt-edge text-text'
                        : 'text-textFaint hover:bg-panel2 hover:text-textDim',
                    )}
                  >
                    {s.number === 0 ? 'Specials' : `S${String(s.number).padStart(2, '0')}`}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="mt-6 min-h-0 flex-1 overflow-y-auto fade-bottom">
          <div className="flex flex-col gap-1 pb-6">
            {season?.episodes.map((episode) => (
              <EpisodeRow
                key={episode.path}
                episode={episode}
                watched={watched.get(episode.path)}
                onPlay={() => onPlay(episode.path)}
              />
            ))}
          </div>
        </div>
      </motion.div>
    </motion.div>
  )
}

function EpisodeRow({
  episode,
  watched,
  onPlay,
}: {
  episode: LibraryEpisode
  watched: Watched | undefined
  onPlay: () => void
}): React.JSX.Element {
  const done = watched ? isFinished(watched) : false
  const part = watched && !done ? watched.fraction : 0

  return (
    <button
      onClick={onPlay}
      className="group relative flex items-center gap-3 overflow-hidden rounded-md border border-line bg-panel px-3.5 py-2.5 text-left transition-colors hover:bg-panel2"
    >
      <span className="tnum w-7 shrink-0 font-mono text-[12px] text-textFaint">
        {episode.number === 0 ? '—' : String(episode.number).padStart(2, '0')}
      </span>
      <span
        className={cn(
          'min-w-0 flex-1 truncate text-[12.5px]',
          // Watched episodes recede rather than vanish: the list is still the
          // whole season, and which ones are done is the useful part.
          done ? 'text-textFaint' : 'text-text',
        )}
      >
        {episode.title ?? nameOf(episode.path)}
      </span>

      {(episode.subtitles?.length ?? 0) > 0 && (
        <span
          title={subtitleTitle(episode.subtitles)}
          className="shrink-0 rounded-[3px] border border-line px-1 py-[1px] font-mono text-[8.5px] uppercase tracking-[0.1em] text-textFaint"
        >
          sub
        </span>
      )}

      {done && <Check size={12} className="shrink-0 text-textFaint" />}
      {part > 0 && (
        <span className="tnum shrink-0 font-mono text-[10px] text-textDim">
          {Math.round(part * 100)}%
        </span>
      )}

      <span className="tnum shrink-0 font-mono text-[10.5px] text-textFaint">
        {formatBytes(episode.size)}
      </span>
      <span className="shrink-0 text-textFaint opacity-0 transition-opacity group-hover:opacity-100">
        <Play size={12} fill="currentColor" />
      </span>

      {part > 0 && (
        <span
          className="absolute inset-x-0 bottom-0 h-[2px] bg-basalt/70"
          style={{ width: `${Math.round(part * 100)}%` }}
        />
      )}
    </button>
  )
}

function nameOf(path: string): string {
  const file = path.split('/').pop() ?? path
  const dot = file.lastIndexOf('.')
  return dot > 0 ? file.slice(0, dot) : file
}
