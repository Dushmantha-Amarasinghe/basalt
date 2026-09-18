import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertCircle,
  ExternalLink,
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
import {
  judgeSound,
  playabilityOf,
  readCounters,
  silenceMessage,
  unplayableMessage,
  type SoundState,
} from '@/lib/playback'
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
  onOpenExternally,
  resumeAt = 0,
  onProgress,
  nextUp,
  onPlayNext,
}: {
  item: MediaItem | null
  onClose: () => void
  /** Hand the file to the system's own player. */
  onOpenExternally?: (path: string) => void
  /** Seconds to start from. Zero starts at the beginning. */
  resumeAt?: number
  /** Called as playback advances, and once when it stops. */
  onProgress?: (path: string, position: number, duration: number) => void
  /** The episode after this one, when there is one. */
  nextUp?: { path: string; label: string } | null
  onPlayNext?: (path: string) => void
}): React.JSX.Element {
  const mediaRef = useRef<HTMLVideoElement | null>(null)
  const [url, setUrl] = useState<string | null>(null)
  const [playing, setPlaying] = useState(false)
  const [position, setPosition] = useState(0)
  const [duration, setDuration] = useState(0)
  const [muted, setMuted] = useState(false)
  const [volume, setVolume] = useState(1)
  const [failed, setFailed] = useState(false)
  const [sound, setSound] = useState<SoundState>('unknown')

  /** True once this file has been seeked to its resume point. */
  const resumed = useRef(false)
  const latest = useRef({ path: '', position: 0, duration: 0 })
  const report = useRef(onProgress)
  report.current = onProgress

  // Reported on a timer rather than on every `timeupdate`, which fires about
  // four times a second and would be four network calls a second.
  useEffect(() => {
    if (!item) return
    const timer = setInterval(() => {
      const { path, position, duration } = latest.current
      if (path && duration > 0) report.current?.(path, position, duration)
    }, 10_000)

    return () => {
      clearInterval(timer)
      // One last report on the way out. This is the important one: it is the
      // position somebody actually stopped at.
      const { path, position, duration } = latest.current
      if (path && duration > 0) report.current?.(path, position, duration)
    }
  }, [item])

  useEffect(() => {
    resumed.current = false
    latest.current = { path: item?.id ?? '', position: 0, duration: 0 }
  }, [item])

  const isAudio = item ? looksLikeAudio(item.id) : false
  // A container the window half-understands: it will open and may show a
  // picture, but any audio that is not Opus or Vorbis is quietly dropped.
  const partial = item ? playabilityOf(item.id) === 'partial' : false

  // Resolve the proxy URL whenever a new item opens.
  useEffect(() => {
    if (!item) {
      setUrl(null)
      return
    }
    let cancelled = false
    setUrl(null)
    setFailed(false)
    setSound('unknown')
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

  // Chromium plays a file it can only partly decode without saying so: the
  // picture runs and there is no sound and no error. Watching the decoded-byte
  // counters is the only way to notice, so the app can say what happened
  // instead of leaving the user to wonder whether it is their volume.
  useEffect(() => {
    if (!url || failed || !item) return undefined
    const started = Date.now()
    const timer = setInterval(() => {
      const media = mediaRef.current
      if (!media || media.paused) return
      const { audio, video } = readCounters(media)
      const verdict = judgeSound(audio, video, (Date.now() - started) / 1000)
      if (verdict !== 'unknown') {
        setSound(verdict)
        clearInterval(timer)
      }
    }, 500)
    return () => clearInterval(timer)
  }, [url, failed, item])

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
                onTimeUpdate={(e) => {
                  const at = e.currentTarget.currentTime
                  setPosition(at)
                  latest.current = {
                    path: item?.id ?? '',
                    position: at,
                    duration: e.currentTarget.duration || 0,
                  }
                }}
                onDurationChange={(e) => {
                  const value = e.currentTarget.duration
                  const total = Number.isFinite(value) ? value : 0
                  setDuration(total)

                  // Seek once, and only once the duration is known — before
                  // that the element refuses to move. Never right at the end,
                  // which would drop someone into the credits of something
                  // they had just finished.
                  if (!resumed.current && total > 0 && resumeAt > 0 && resumeAt < total - 10) {
                    resumed.current = true
                    e.currentTarget.currentTime = resumeAt
                  }
                }}
                onEnded={() => {
                  // Mark it finished before moving on, so it leaves Continue
                  // watching rather than sitting there at 99%.
                  if (item && latest.current.duration > 0) {
                    report.current?.(
                      item.id,
                      latest.current.duration,
                      latest.current.duration,
                    )
                  }
                  if (nextUp && onPlayNext) onPlayNext(nextUp.path)
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
                    <div className="mt-4 max-w-[400px] px-6 text-[13px] leading-relaxed text-text">
                      {unplayableMessage(item.id)}
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

            {/*
              The picture is running and nothing is coming out. Chromium gives
              no error for this, so without saying it here the user is left
              checking their own volume.
            */}
            <AnimatePresence>
              {sound === 'silent' && (
                <motion.div
                  initial={{ opacity: 0, y: -8 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -8 }}
                  className="absolute inset-x-0 top-0 flex justify-center px-16 pt-4"
                >
                  <div className="flex max-w-[520px] items-start gap-2.5 rounded-md border border-danger/25 bg-dangerBg/95 px-3.5 py-2.5 backdrop-blur">
                    <AlertCircle size={14} className="mt-px shrink-0 text-danger" />
                    <p className="text-[12px] leading-relaxed text-danger">
                      {silenceMessage(item.id)}
                    </p>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>

            {onOpenExternally && (sound === 'silent' || failed || partial) && (
              <motion.button
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                onClick={() => onOpenExternally(item.id)}
                className="absolute bottom-6 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-md border border-white/[0.16] bg-panel2/95 px-3.5 py-2 text-[12px] text-text backdrop-blur transition-colors hover:bg-white/[0.08]"
              >
                <ExternalLink size={13} />
                Play in your player
              </motion.button>
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

              <div className="group/vol flex items-center gap-1.5">
                <ControlButton
                  icon={muted || volume === 0 ? VolumeX : Volume2}
                  label={muted ? 'Unmute' : 'Mute'}
                  onClick={() => {
                    const media = mediaRef.current
                    if (!media) return
                    media.muted = !media.muted
                    setMuted(media.muted)
                  }}
                />
                {/*
                  A real slider rather than a mute toggle alone: when someone
                  reports no sound, the first thing they need is to rule out
                  the volume, and a control that only mutes cannot do that.
                */}
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.02}
                  value={muted ? 0 : volume}
                  aria-label="Volume"
                  onChange={(e) => {
                    const next = Number(e.target.value)
                    const media = mediaRef.current
                    setVolume(next)
                    setMuted(next === 0)
                    if (media) {
                      media.volume = next
                      media.muted = next === 0
                    }
                  }}
                  className="h-1 w-0 cursor-pointer appearance-none rounded-full bg-white/[0.14] opacity-0 transition-all duration-200 accent-basalt group-hover/vol:w-20 group-hover/vol:opacity-100"
                />
              </div>
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

function looksLikeAudio(path: string): boolean {
  const dot = path.lastIndexOf('.')
  const ext = dot > 0 ? path.slice(dot + 1).toLowerCase() : ''
  return ['mp3', 'flac', 'm4a', 'wav', 'ogg', 'opus', 'aac', 'wma'].includes(ext)
}
