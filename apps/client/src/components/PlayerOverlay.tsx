import { useEffect, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  Maximize2,
  Pause,
  Play,
  SkipBack,
  SkipForward,
  Subtitles,
  Volume2,
  X,
} from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatDuration } from '@/lib/mockMedia'
import { cn } from '@/lib/utils'

/**
 * Video player overlay.
 *
 * A placeholder surface, not a real player — the shipped app will hand the
 * stream to mpv, because WebView2 cannot decode HEVC, MKV or AC3 and most of a
 * media library is exactly that. What this exists to settle now is the
 * chrome: where the controls sit, how they fade, how the scrubber reads.
 */
export function PlayerOverlay({
  item,
  onClose,
}: {
  item: MediaItem | null
  onClose: () => void
}): React.JSX.Element {
  const [playing, setPlaying] = useState(true)
  const [position, setPosition] = useState(0)
  const duration = item?.duration ?? 0

  // Simulated playback so the scrubber and timecode are live enough to judge.
  useEffect(() => {
    if (!item) return undefined
    setPosition((item.progress ?? 0) * (item.duration ?? 0))
    setPlaying(true)
    return undefined
  }, [item])

  useEffect(() => {
    if (!playing || !item) return undefined
    const id = setInterval(() => {
      setPosition((p) => (p + 1 > duration ? duration : p + 1))
    }, 1000)
    return () => clearInterval(id)
  }, [playing, item, duration])

  useEffect(() => {
    if (!item) return undefined
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
      if (e.key === ' ') {
        e.preventDefault()
        setPlaying((p) => !p)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [item, onClose])

  const percent = duration > 0 ? (position / duration) * 100 : 0

  return (
    <AnimatePresence>
      {item && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          className="fixed inset-0 z-40 flex flex-col bg-black"
        >
          {/* Stand-in for the video surface. */}
          <div
            className="relative flex flex-1 items-center justify-center"
            style={{
              background: `radial-gradient(120% 90% at 50% 40%, ${item.tone[1]} 0%, #050506 70%)`,
            }}
          >
            <motion.div
              initial={{ scale: 0.96, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
              className="text-center"
            >
              <div className="font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                now playing
              </div>
              <div className="mt-3 text-2xl font-semibold tracking-tight text-text">
                {item.title}
              </div>
              <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
              <div className="mt-6 font-mono text-[11px] text-textFaint">
                direct play · no transcode
              </div>
            </motion.div>

            <button
              onClick={onClose}
              aria-label="Close player"
              className="absolute right-4 top-4 flex h-9 w-9 items-center justify-center rounded-full bg-black/40 text-textDim backdrop-blur transition-colors hover:bg-black/60 hover:text-text"
            >
              <X size={16} />
            </button>
          </div>

          {/* Controls */}
          <motion.div
            initial={{ y: 20, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            transition={{ delay: 0.1, duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
            className="shrink-0 border-t border-white/[0.06] bg-ink/95 px-5 py-4 backdrop-blur"
          >
            <div className="group relative h-1 cursor-pointer rounded-full bg-white/[0.1]">
              <div
                className="absolute inset-y-0 left-0 rounded-full bg-basalt"
                style={{ width: `${percent}%` }}
              />
              <div
                className="absolute top-1/2 h-3 w-3 -translate-y-1/2 rounded-full bg-basalt opacity-0 transition-opacity group-hover:opacity-100"
                style={{ left: `calc(${percent}% - 6px)` }}
              />
            </div>

            <div className="mt-3 flex items-center gap-4">
              <ControlButton icon={SkipBack} label="Previous" />
              <button
                onClick={() => setPlaying((p) => !p)}
                aria-label={playing ? 'Pause' : 'Play'}
                className="flex h-10 w-10 items-center justify-center rounded-full bg-basalt text-ink transition-transform hover:scale-105"
              >
                {playing ? (
                  <Pause size={17} className="fill-ink" />
                ) : (
                  <Play size={17} className="ml-0.5 fill-ink" />
                )}
              </button>
              <ControlButton icon={SkipForward} label="Next" />

              <span className="tnum ml-2 font-mono text-[11px] text-textDim">
                {formatDuration(position)}
                <span className="text-textFaint"> / {formatDuration(duration)}</span>
              </span>

              <div className="flex-1" />

              <ControlButton icon={Subtitles} label="Subtitles" />
              <ControlButton icon={Volume2} label="Volume" />
              <ControlButton icon={Maximize2} label="Fullscreen" />
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

function ControlButton({
  icon: Icon,
  label,
}: {
  icon: typeof Play
  label: string
}): React.JSX.Element {
  return (
    <button
      aria-label={label}
      title={label}
      className={cn(
        'flex h-8 w-8 items-center justify-center rounded-md text-textDim',
        'transition-colors hover:bg-white/[0.06] hover:text-text',
      )}
    >
      <Icon size={16} />
    </button>
  )
}
