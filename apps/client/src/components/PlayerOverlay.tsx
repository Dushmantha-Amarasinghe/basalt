import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertCircle,
  Maximize2,
  Pause,
  Play,
  SkipBack,
  SkipForward,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatDuration } from '@/lib/mockMedia'
import { api } from '@/lib/api'
import { cn } from '@/lib/utils'

/**
 * The player.
 *
 * The file is not downloaded first. The source is a URL from the local media
 * proxy, and dragging the scrubber makes the browser issue a range request,
 * which becomes a ranged read on the host, which becomes a seek on the drive.
 * That chain is the whole reason `Read` takes an offset.
 *
 * WebView2 is Chromium, so it plays what Chromium plays: H.264/AAC in MP4,
 * WebM, MP3, FLAC. MKV, HEVC and AC3 are common in a real library and it
 * cannot decode them — so rather than showing a black rectangle and letting
 * the user guess, that case is named explicitly. The mpv sidecar that fixes it
 * will take the same URL.
 */
export function PlayerOverlay({
  item,
  onClose,
}: {
  item: MediaItem | null
  onClose: () => void
}): React.JSX.Element {
  const mediaRef = useRef<HTMLVideoElement | null>(null)
  const [url, setUrl] = useState<string | null>(null)
  const [playing, setPlaying] = useState(false)
  const [position, setPosition] = useState(0)
  const [duration, setDuration] = useState(0)
  const [muted, setMuted] = useState(false)
  const [failed, setFailed] = useState(false)

  const isAudio = item ? looksLikeAudio(item.id) : false

  // Resolve the proxy URL whenever a new item opens.
  useEffect(() => {
    if (!item) {
      setUrl(null)
      return
    }
    let cancelled = false
    setUrl(null)
    setFailed(false)
    setPosition(0)
    setDuration(0)
    void api
      .mediaUrl(item.id)
      .then((resolved) => {
        if (!cancelled) setUrl(resolved || null)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
    }
  }, [item])

  const toggle = useCallback(() => {
    const media = mediaRef.current
    if (!media) return
    if (media.paused) void media.play().catch(() => setFailed(true))
    else media.pause()
  }, [])

  const seekTo = useCallback((fraction: number) => {
    const media = mediaRef.current
    if (!media || !Number.isFinite(media.duration)) return
    media.currentTime = Math.max(0, Math.min(1, fraction)) * media.duration
  }, [])

  const skip = useCallback((seconds: number) => {
    const media = mediaRef.current
    if (!media) return
    media.currentTime = Math.max(0, media.currentTime + seconds)
  }, [])

  useEffect(() => {
    if (!item) return undefined
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
      if (e.key === ' ') {
        e.preventDefault()
        toggle()
      }
      if (e.key === 'ArrowRight') skip(10)
      if (e.key === 'ArrowLeft') skip(-10)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [item, onClose, toggle, skip])

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
          <div
            className="relative flex min-h-0 flex-1 items-center justify-center"
            style={{
              background: `radial-gradient(120% 90% at 50% 40%, ${item.tone[1]} 0%, #050506 70%)`,
            }}
          >
            {url && !failed ? (
              <video
                ref={mediaRef}
                src={url}
                autoPlay
                // Audio keeps the poster backdrop visible rather than a strip
                // of black where a picture would be.
                className={cn(
                  'max-h-full max-w-full',
                  isAudio && 'pointer-events-none h-0 w-0 opacity-0',
                )}
                onPlay={() => setPlaying(true)}
                onPause={() => setPlaying(false)}
                onTimeUpdate={(e) => setPosition(e.currentTarget.currentTime)}
                onDurationChange={(e) => {
                  const value = e.currentTarget.duration
                  setDuration(Number.isFinite(value) ? value : 0)
                }}
                onError={() => setFailed(true)}
              />
            ) : null}

            {(isAudio || !url || failed) && (
              <motion.div
                initial={{ scale: 0.96, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
                className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center text-center"
              >
                {failed ? (
                  <>
                    <AlertCircle size={22} className="text-danger" />
                    <div className="mt-4 max-w-[380px] px-6 text-sm text-text">
                      This build cannot decode {extensionOf(item.id) || 'this file'}.
                    </div>
                    <div className="mt-2 max-w-[380px] px-6 text-[12px] leading-relaxed text-textDim">
                      The window plays what Chromium plays — MP4, WebM, MP3, FLAC.
                      Download it to watch in another player for now.
                    </div>
                  </>
                ) : (
                  <>
                    <div className="font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                      {url ? 'now playing' : 'opening'}
                    </div>
                    <div className="mt-3 px-8 text-2xl font-semibold tracking-tight text-text">
                      {item.title}
                    </div>
                    <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
                    <div className="mt-6 font-mono text-[11px] text-textFaint">
                      streaming from the vault · nothing downloaded
                    </div>
                  </>
                )}
              </motion.div>
            )}

            <button
              onClick={onClose}
              aria-label="Close player"
              className="absolute right-4 top-4 flex h-9 w-9 items-center justify-center rounded-full bg-black/40 text-textDim backdrop-blur transition-colors hover:bg-black/60 hover:text-text"
            >
              <X size={16} />
            </button>
          </div>

          <motion.div
            initial={{ y: 20, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            transition={{ delay: 0.1, duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
            className="shrink-0 border-t border-white/[0.06] bg-ink/95 px-5 py-4 backdrop-blur"
          >
            <Scrubber percent={percent} onSeek={seekTo} disabled={duration === 0} />

            <div className="mt-3 flex items-center gap-4">
              <ControlButton icon={SkipBack} label="Back 10s" onClick={() => skip(-10)} />
              <button
                onClick={toggle}
                disabled={!url || failed}
                aria-label={playing ? 'Pause' : 'Play'}
                className="flex h-10 w-10 items-center justify-center rounded-full bg-basalt text-ink transition-transform hover:scale-105 disabled:opacity-30 disabled:hover:scale-100"
              >
                {playing ? (
                  <Pause size={17} className="fill-ink" />
                ) : (
                  <Play size={17} className="ml-0.5 fill-ink" />
                )}
              </button>
              <ControlButton
                icon={SkipForward}
                label="Forward 10s"
                onClick={() => skip(10)}
              />

              <span className="tnum ml-2 font-mono text-[11px] text-textDim">
                {formatDuration(position)}
                <span className="text-textFaint"> / {formatDuration(duration)}</span>
              </span>

              <div className="flex-1" />

              <ControlButton
                icon={muted ? VolumeX : Volume2}
                label={muted ? 'Unmute' : 'Mute'}
                onClick={() => {
                  const media = mediaRef.current
                  if (!media) return
                  media.muted = !media.muted
                  setMuted(media.muted)
                }}
              />
              <ControlButton
                icon={Maximize2}
                label="Fullscreen"
                onClick={() => void mediaRef.current?.requestFullscreen?.()}
              />
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

/** The progress bar, clickable anywhere along its length. */
function Scrubber({
  percent,
  onSeek,
  disabled,
}: {
  percent: number
  onSeek: (fraction: number) => void
  disabled: boolean
}): React.JSX.Element {
  return (
    <div
      role="slider"
      aria-label="Position"
      aria-valuenow={Math.round(percent)}
      aria-valuemin={0}
      aria-valuemax={100}
      tabIndex={0}
      onClick={(e) => {
        if (disabled) return
        const box = e.currentTarget.getBoundingClientRect()
        onSeek((e.clientX - box.left) / box.width)
      }}
      className={cn(
        'group relative h-1 rounded-full bg-white/[0.1]',
        disabled ? 'cursor-default opacity-50' : 'cursor-pointer',
      )}
    >
      {/* A taller invisible target: a 1px bar is far too small to hit. */}
      <div className="absolute -inset-y-2 inset-x-0" />
      <div
        className="absolute inset-y-0 left-0 rounded-full bg-basalt"
        style={{ width: `${percent}%` }}
      />
      <div
        className="absolute top-1/2 h-3 w-3 -translate-y-1/2 rounded-full bg-basalt opacity-0 transition-opacity group-hover:opacity-100"
        style={{ left: `calc(${percent}% - 6px)` }}
      />
    </div>
  )
}

function ControlButton({
  icon: Icon,
  label,
  onClick,
}: {
  icon: typeof Play
  label: string
  onClick?: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
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

function extensionOf(path: string): string {
  const dot = path.lastIndexOf('.')
  return dot > 0 ? path.slice(dot + 1).toUpperCase() : ''
}

function looksLikeAudio(path: string): boolean {
  return ['mp3', 'flac', 'm4a', 'wav', 'ogg', 'opus', 'aac', 'wma'].includes(
    extensionOf(path).toLowerCase(),
  )
}
