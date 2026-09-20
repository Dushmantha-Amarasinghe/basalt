import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertCircle,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  Loader2,
  Maximize2,
  Pause,
  Play,
  Plus,
  SkipBack,
  SkipForward,
  Subtitles,
  Volume2,
  VolumeX,
  X,
} from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatDuration } from '@/lib/mockMedia'
import { api, type SubtitleTrack } from '@/lib/api'
import { pickSubtitleFile } from '@/lib/dialogs'
import { useMpv, type Mpv } from '@/lib/useMpv'
import { cn } from '@/lib/utils'

/** Motionless for this long and the controls step aside. */
const CONTROLS_IDLE = 2600

/** Where mpv draws subtitles with the controls up, and without. */
const SUBTITLES_ABOVE_CONTROLS = 96
const SUBTITLES_AT_REST = 22

/**
 * The player.
 *
 * Nothing is downloaded. The source is a URL from the local media proxy, and
 * seeking becomes a range request, which becomes a ranged read on the host,
 * which becomes a seek on the drive. That chain is why `Read` takes an offset.
 *
 * The picture is **mpv**, drawn into the native window behind this page — see
 * [`useMpv`] for why a `<video>` element could not do the job. Everything
 * here is ordinary HTML composited over the top of it, which is why the area
 * where the video belongs is deliberately left transparent.
 */
export function PlayerOverlay({
  item,
  onClose,
  onOpenExternally,
  resumeAt = 0,
  onProgress,
  nextUp,
  onPlayNext,
  subtitles = [],
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
  /** Subtitle files the host found beside this file. */
  subtitles?: SubtitleTrack[]
}): React.JSX.Element {
  const mpv = useMpv()
  const [failed, setFailed] = useState<string | null>(null)
  const [menu, setMenu] = useState(false)

  /**
   * Whether the controls are on screen.
   *
   * They sit over the picture, and the bottom of the picture is where the
   * subtitles are — so leaving them up permanently costs exactly the part of
   * the frame you are reading. They come back on any movement and go away
   * again after a pause in it, which is what every player does.
   *
   * Never hidden while paused, while the subtitle menu is open, or while the
   * pointer is resting on the bar itself: each of those means somebody is
   * looking at the controls rather than the film.
   */
  const [showControls, setShowControls] = useState(true)
  const [overBar, setOverBar] = useState(false)
  /**
   * When the volume last changed by key, so it can be shown.
   *
   * Without something on screen a five per cent step is nearly inaudible,
   * and a control you cannot tell is working is indistinguishable from one
   * that is not — which is how this was reported.
   */
  const [volumeOsd, setVolumeOsd] = useState(false)
  const volumeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const flashVolume = useCallback(() => {
    setVolumeOsd(true)
    if (volumeTimer.current) clearTimeout(volumeTimer.current)
    volumeTimer.current = setTimeout(() => setVolumeOsd(false), 1200)
  }, [])
  const idleTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const keepControls = useCallback(() => {
    setShowControls(true)
    if (idleTimer.current) clearTimeout(idleTimer.current)
    idleTimer.current = setTimeout(() => setShowControls(false), CONTROLS_IDLE)
  }, [])

  const pinned = mpv.paused || menu || overBar || !mpv.picture
  const controlsUp = showControls || pinned

  // Any movement anywhere brings them back, including over the controls.
  useEffect(() => {
    if (!item) return undefined
    keepControls()
    window.addEventListener('mousemove', keepControls)
    window.addEventListener('keydown', keepControls)
    return () => {
      window.removeEventListener('mousemove', keepControls)
      window.removeEventListener('keydown', keepControls)
      if (idleTimer.current) clearTimeout(idleTimer.current)
    }
  }, [item, keepControls])

  const latest = useRef({ path: '', position: 0, duration: 0 })
  const report = useRef(onProgress)
  report.current = onProgress

  /**
   * Which file mpv is actually playing, as opposed to which one is selected.
   *
   * They differ for a moment on every change of episode, and writing during
   * that moment recorded the outgoing film's position against the incoming
   * film's path — so the next episode began already part-watched, at a time
   * nobody had reached.
   */
  const playingNow = useRef<string | null>(null)
  if (playingNow.current === (item?.id ?? null)) {
    latest.current = {
      path: item?.id ?? '',
      position: mpv.position,
      duration: mpv.duration,
    }
  }

  // Reported on a timer rather than on every tick, which would be several
  // network calls a second.
  useEffect(() => {
    if (!item) return
    const mine = item.id
    const tell = (): void => {
      const { path, position, duration } = latest.current
      if (path === mine && duration > 0) report.current?.(path, position, duration)
    }
    const timer = setInterval(tell, 10_000)

    return () => {
      clearInterval(timer)
      // One last report on the way out, and the important one: it is the
      // position somebody actually stopped at.
      tell()
    }
  }, [item])

  // Open the file whenever a new one is chosen, and stop when the player closes.
  const load = mpv.load
  const stop = mpv.stop
  useEffect(() => {
    if (!item) {
      void stop()
      return
    }
    let cancelled = false
    setFailed(null)
    setMenu(false)
    playingNow.current = null
    advanced.current = null
    void api
      .mediaUrl(item.id)
      .then(async (url) => {
        if (cancelled) return
        if (!url) {
          setFailed('This file could not be opened for streaming.')
          return
        }
        await load(url, resumeAt > 0 ? resumeAt : 0)
        if (!cancelled) playingNow.current = item.id
      })
      .catch((e: unknown) => {
        if (!cancelled) setFailed(String(e))
      })
    return () => {
      cancelled = true
    }
    // `resumeAt` deliberately absent: it changes as the position is reported
    // back, and depending on it would reload the file mid-playback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item, load, stop])

  // Finished: mark it watched so it leaves Continue watching rather than
  // sitting at 99%, then go on to the next episode if there is one.
  const advance = useRef({ nextUp, onPlayNext })
  advance.current = { nextUp, onPlayNext }
  /** The file this player has already moved on from. */
  const advanced = useRef<string | null>(null)

  useEffect(() => {
    if (!mpv.ended || !item) return
    // `ended` is a state, not an event, and `item` is in these dependencies —
    // so without a latch one true value re-fires for the next episode, and
    // the one after that, walking the whole series in a second. Each file may
    // hand over exactly once.
    if (advanced.current === item.id || playingNow.current !== item.id) return
    advanced.current = item.id

    const { duration } = latest.current
    if (duration > 0) report.current?.(item.id, duration, duration)
    const { nextUp: next, onPlayNext: play } = advance.current
    if (next && play) play(next.path)
  }, [mpv.ended, item])

  /**
   * Click pauses, double click goes fullscreen — without doing both.
   *
   * A double click delivers two `click` events before the `dblclick`, so the
   * naive wiring toggled pause twice on the way to fullscreen. Net zero, but
   * visibly janky: the film stopped and started under the cursor. The single
   * click waits long enough to find out which it was.
   */
  const clickTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const onSingleClick = useCallback(() => {
    if (clickTimer.current) return
    clickTimer.current = setTimeout(() => {
      clickTimer.current = null
      void mpv.togglePause()
    }, 220)
  }, [mpv])

  const onDoubleClick = useCallback(() => {
    if (clickTimer.current) {
      clearTimeout(clickTimer.current)
      clickTimer.current = null
    }
    void fullscreenRef.current()
  }, [])

  const fullscreen = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const window = getCurrentWindow()
      await window.setFullscreen(!(await window.isFullscreen()))
    } catch {
      // Not in the shell, or the window refused; neither is worth an error.
    }
  }, [])

  /**
   * Subtitles step up out of the way of the controls.
   *
   * mpv draws them a little above the bottom edge, which is exactly where
   * the control bar sits — so bringing the controls up covered the line
   * somebody was reading. They drop back down as soon as the bar does.
   */
  const lift = mpv.setSubtitleMargin
  useEffect(() => {
    if (!item) return
    void lift(controlsUp ? SUBTITLES_ABOVE_CONTROLS : SUBTITLES_AT_REST)
  }, [item, controlsUp, lift])

  const fullscreenRef = useRef(fullscreen)
  fullscreenRef.current = fullscreen

  // Leaving fullscreen is what Escape means while fullscreen; closing the
  // player from there would drop the window back to its old size *and* end
  // the film, which is two surprises for one key.
  const escape = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const window = getCurrentWindow()
      if (await window.isFullscreen()) {
        await window.setFullscreen(false)
        return
      }
    } catch {
      // Not in the shell; fall through and close.
    }
    onClose()
  }, [onClose])

  /** Closing from fullscreen must not leave the window fullscreen. */
  const leave = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const window = getCurrentWindow()
      if (await window.isFullscreen()) await window.setFullscreen(false)
    } catch {
      // Not in the shell.
    }
    onClose()
  }, [onClose])

  useEffect(() => {
    if (!item) return undefined
    const onKey = (e: KeyboardEvent): void => {
      // Typing in the subtitle-offset box is not a player shortcut.
      const target = e.target as HTMLElement | null
      if (target?.closest('input, textarea, [contenteditable="true"]')) return

      switch (e.key) {
        case 'Escape':
          void escape()
          break
        case ' ':
        case 'k':
          e.preventDefault()
          void mpv.togglePause()
          break
        case 'ArrowRight':
          e.preventDefault()
          void mpv.seekBy(e.shiftKey ? 60 : 5)
          break
        case 'ArrowLeft':
          e.preventDefault()
          void mpv.seekBy(e.shiftKey ? -60 : -5)
          break
        case 'ArrowUp':
          e.preventDefault()
          void mpv.nudgeVolume(5)
          flashVolume()
          break
        case 'ArrowDown':
          e.preventDefault()
          void mpv.nudgeVolume(-5)
          flashVolume()
          break
        // mpv's own keys for this, because anyone who wants frame stepping
        // already knows them.
        case '.':
          e.preventDefault()
          void mpv.stepFrame(1)
          break
        case ',':
          e.preventDefault()
          void mpv.stepFrame(-1)
          break
        case 'm':
          void mpv.toggleMute()
          break
        case 'f':
          void fullscreen()
          break
        default:
          break
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [item, escape, mpv, fullscreen, flashVolume])

  const percent = mpv.duration > 0 ? (mpv.position / mpv.duration) * 100 : 0
  const problem = failed ?? mpv.problem

  return (
    <AnimatePresence>
      {item && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          className="fixed inset-0 z-40"
        >
          {/*
            Transparent, and that is not a style choice: mpv is drawing behind
            this page, and anything painted here would cover the film. Only
            the bars above and below are opaque.
          */}
          {/*
            Black until there is a frame behind it.

            This area is transparent so mpv, which draws behind the page, can
            show through — but for the second or two before the first frame
            there is nothing back there, and a transparent hole over a hidden
            app shows the desktop. On screen that read as two windows: the
            controls in one, whatever was behind them in the other.
          */}
          <div
            onClick={onSingleClick}
            onDoubleClick={onDoubleClick}
            className={cn(
              'absolute inset-0',
              !mpv.picture && 'bg-black',
              // The cursor goes with the controls: a pointer resting over a
              // film is as much of an intrusion as the bar underneath it.
              controlsUp ? 'cursor-pointer' : 'cursor-none',
            )}
          >
            {problem && (
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center bg-black text-center">
                <AlertCircle size={22} className="text-danger" />
                <div className="mt-4 max-w-[420px] px-6 text-[13px] leading-relaxed text-text">
                  {problem}
                </div>
              </div>
            )}

            {/* Only until the first frame, so it is never over a picture. */}
            {!problem && !mpv.picture && (
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center bg-black text-center">
                <div className="font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                  opening
                </div>
                <div className="mt-3 px-8 text-2xl font-semibold tracking-tight text-text">
                  {item.title}
                </div>
                <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
                <div className="mt-6 font-mono text-[11px] text-textFaint">
                  streaming from the vault · nothing downloaded
                </div>
              </div>
            )}

            {/* Stalled on the network. Without this it is indistinguishable
                from the player having died. */}
            <AnimatePresence>
              {mpv.buffering && mpv.picture && (
                <motion.div
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="pointer-events-none absolute inset-0 flex items-center justify-center"
                >
                  <span className="flex items-center gap-2 rounded-full bg-black/60 px-3.5 py-2 backdrop-blur">
                    <Loader2 size={14} className="animate-spin text-textDim" />
                    <span className="font-mono text-[11px] text-textDim">buffering</span>
                  </span>
                </motion.div>
              )}
            </AnimatePresence>

            {/* What the volume keys just did. */}
            <AnimatePresence>
              {volumeOsd && (
                <motion.div
                  initial={{ opacity: 0, scale: 0.94 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.12 }}
                  className="pointer-events-none absolute left-1/2 top-12 flex -translate-x-1/2 items-center gap-2.5 rounded-full bg-black/70 px-4 py-2.5 backdrop-blur"
                >
                  {mpv.muted || mpv.volume === 0 ? (
                    <VolumeX size={15} className="text-textDim" />
                  ) : (
                    <Volume2 size={15} className="text-textDim" />
                  )}
                  <div className="h-1 w-28 overflow-hidden rounded-full bg-white/15">
                    <div
                      className="h-full rounded-full bg-basalt"
                      style={{ width: `${Math.min(100, (mpv.volume / 130) * 100)}%` }}
                    />
                  </div>
                  <span className="tnum w-9 text-right font-mono text-[11px] text-text">
                    {Math.round(mpv.volume)}
                  </span>
                </motion.div>
              )}
            </AnimatePresence>

            {/* A paused film shows nothing else; this says it is paused. */}
            <AnimatePresence>
              {mpv.paused && mpv.picture && !mpv.buffering && (
                <motion.div
                  initial={{ opacity: 0, scale: 0.9 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.9 }}
                  transition={{ duration: 0.14 }}
                  className="pointer-events-none absolute inset-0 flex items-center justify-center"
                >
                  <span className="flex h-16 w-16 items-center justify-center rounded-full bg-black/55 backdrop-blur">
                    <Pause size={24} className="fill-text text-text" />
                  </span>
                </motion.div>
              )}
            </AnimatePresence>

            {onOpenExternally && problem && (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onOpenExternally(item.id)
                }}
                className="absolute bottom-6 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-md border border-white/[0.16] bg-panel2/95 px-3.5 py-2 text-[12px] text-text backdrop-blur transition-colors hover:bg-white/[0.08]"
              >
                <ExternalLink size={13} />
                Play in your player
              </button>
            )}

            {/*
              Somewhere to hold the window by.

              The app's title bar is the drag handle, and the player hides it
              along with the rest of the app — so while a film was open the
              window could not be moved at all. This is a strip of the same
              height in the same place, and it comes and goes with the
              controls so there is no dead band across the top of a film
              nobody is currently touching.
            */}
            {controlsUp && (
              <div className="drag absolute inset-x-0 top-0 h-9" />
            )}

            <motion.button
              initial={false}
              animate={{ opacity: controlsUp ? 1 : 0 }}
              transition={{ duration: 0.22 }}
              style={{ pointerEvents: controlsUp ? 'auto' : 'none' }}
              onClick={(e) => {
                e.stopPropagation()
                void leave()
              }}
              aria-label="Close player"
              className="no-drag absolute right-4 top-4 z-10 flex h-9 w-9 items-center justify-center rounded-full bg-black/40 text-textDim backdrop-blur transition-colors hover:bg-black/60 hover:text-text"
            >
              <X size={16} />
            </motion.button>
          </div>

          <motion.div
            initial={false}
            animate={{ y: controlsUp ? 0 : 28, opacity: controlsUp ? 1 : 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            onMouseEnter={() => setOverBar(true)}
            onMouseLeave={() => setOverBar(false)}
            style={{ pointerEvents: controlsUp ? 'auto' : 'none' }}
            // Over the picture rather than beside it. As a row in a column it
            // took a strip of the window permanently, so a film was letterboxed
            // above its own controls whether or not anyone wanted them.
            className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-ink via-ink/92 to-transparent px-5 pb-4 pt-10"
          >
            <AnimatePresence>
              {menu && (
                <>
                  {/* Clicking anywhere else puts it away, which is what every
                      menu does and what anyone will try first. Behind the
                      menu itself, so the menu still takes its own clicks. */}
                  <div
                    className="fixed inset-0 z-[5]"
                    onClick={() => setMenu(false)}
                  />
                  <SubtitleMenu
                    mpv={mpv}
                    fromDrive={subtitles}
                    onClose={() => setMenu(false)}
                  />
                </>
              )}
            </AnimatePresence>

            <Scrubber
              percent={percent}
              onSeek={(fraction) => void mpv.seekTo(fraction * mpv.duration)}
              disabled={mpv.duration === 0}
            />

            <div className="mt-3 flex items-center gap-4">
              <ControlButton
                icon={SkipBack}
                label="Back 10s"
                onClick={() => void mpv.seekBy(-10)}
              />
              <button
                onClick={() => void mpv.togglePause()}
                disabled={mpv.duration === 0}
                aria-label={mpv.paused ? 'Play' : 'Pause'}
                className="flex h-10 w-10 items-center justify-center rounded-full bg-basalt text-ink transition-transform hover:scale-105 disabled:opacity-30 disabled:hover:scale-100"
              >
                {mpv.paused ? (
                  <Play size={17} className="ml-0.5 fill-ink" />
                ) : (
                  <Pause size={17} className="fill-ink" />
                )}
              </button>
              <ControlButton
                icon={SkipForward}
                label="Forward 10s"
                onClick={() => void mpv.seekBy(10)}
              />

              {/* Frame stepping, which is the thing a `<video>` element could
                  only ever approximate by nudging `currentTime`. */}
              <div className="ml-1 flex items-center">
                <ControlButton
                  icon={ChevronLeft}
                  label="Previous frame (,)"
                  onClick={() => void mpv.stepFrame(-1)}
                />
                <ControlButton
                  icon={ChevronRight}
                  label="Next frame (.)"
                  onClick={() => void mpv.stepFrame(1)}
                />
              </div>

              <span className="tnum ml-1 font-mono text-[11px] text-textDim">
                {formatDuration(mpv.position)}
                <span className="text-textFaint"> / {formatDuration(mpv.duration)}</span>
              </span>

              <div className="flex-1" />

              <button
                onClick={() => setMenu((open) => !open)}
                aria-label="Subtitles"
                title="Subtitles"
                className={cn(
                  'flex h-8 items-center gap-1.5 rounded-md px-2 text-[11px] transition-colors',
                  mpv.subtitleId !== null
                    ? 'bg-white/[0.08] text-text'
                    : 'text-textDim hover:bg-white/[0.06] hover:text-text',
                )}
              >
                <Subtitles size={16} />
                {mpv.subtitleDelay !== 0 && (
                  <span className="tnum font-mono text-[10px]">
                    {mpv.subtitleDelay > 0 ? '+' : ''}
                    {mpv.subtitleDelay.toFixed(1)}s
                  </span>
                )}
              </button>

              <div className="group/vol flex items-center gap-1.5">
                <ControlButton
                  icon={mpv.muted || mpv.volume === 0 ? VolumeX : Volume2}
                  label={mpv.muted ? 'Unmute' : 'Mute'}
                  onClick={() => void mpv.toggleMute()}
                />
                {/*
                  A real slider rather than a mute toggle alone: when someone
                  reports no sound, the first thing they need is to rule out
                  the volume, and a control that only mutes cannot do that.
                */}
                <input
                  type="range"
                  min={0}
                  max={130}
                  step={1}
                  value={mpv.muted ? 0 : mpv.volume}
                  aria-label="Volume"
                  onChange={(e) => void mpv.setVolume(Number(e.target.value))}
                  className="h-1 w-0 cursor-pointer appearance-none rounded-full bg-white/[0.14] opacity-0 transition-all duration-200 accent-basalt group-hover/vol:w-20 group-hover/vol:opacity-100"
                />
              </div>
              <ControlButton icon={Maximize2} label="Fullscreen (f)" onClick={fullscreen} />
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

/**
 * Which subtitles, and whether they are in time with the sound.
 *
 * The offset is here rather than buried in a settings screen because it is
 * needed *while watching* — a subtitle file from one release against a video
 * from another drifts, and the only way to correct it is to watch and nudge.
 */
function SubtitleMenu({
  mpv,
  fromDrive,
  onClose,
}: {
  mpv: Mpv
  fromDrive: SubtitleTrack[]
  onClose: () => void
}): React.JSX.Element {
  const inFile = mpv.tracks.filter((t) => t.kind === 'sub')
  const audio = mpv.tracks.filter((t) => t.kind === 'audio')

  const addFromDrive = async (track: SubtitleTrack): Promise<void> => {
    // Through the proxy: mpv reaches the host the same way the video does.
    const url = await api.mediaUrl(track.path)
    if (url) await mpv.addSubtitle(url)
  }

  const addFromDisk = async (): Promise<void> => {
    const path = await pickSubtitleFile()
    if (path) await mpv.addSubtitle(path)
  }

  const nudge = (by: number): void => {
    void mpv.setSubtitleDelay(Math.round((mpv.subtitleDelay + by) * 100) / 100)
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 8 }}
      transition={{ duration: 0.14 }}
      // Opaque, not translucent. Over a bright frame the old `/98` plus a
      // blur washed out to the point where the labels and the offset could
      // not be read at all — and this is a menu you use *while* watching,
      // which means it is always over a picture.
      className="absolute bottom-full right-4 z-10 mb-2 w-[300px] overflow-hidden rounded-lg border border-white/15 bg-[#151517] shadow-lift"
    >
      <div className="max-h-[260px] overflow-y-auto py-1.5">
        <Choice
          label="Off"
          active={mpv.subtitleId === null}
          onClick={() => void mpv.selectSubtitle(null)}
        />
        {inFile.map((track) => (
          <Choice
            key={track.id}
            label={track.label}
            hint={track.external ? 'file' : undefined}
            active={mpv.subtitleId === track.id}
            onClick={() => void mpv.selectSubtitle(track.id)}
          />
        ))}

        {/* Files the host found beside the video. Loaded on demand rather
            than all at once: a season folder can hold a dozen languages and
            handing every one of them to mpv before anybody asks is work
            nobody wanted. */}
        {fromDrive.length > 0 && (
          <>
            <div className="mt-1 px-3 py-1 font-mono text-[9.5px] uppercase tracking-[0.14em] text-textFaint">
              on the drive
            </div>
            {fromDrive.map((track) => (
              <Choice
                key={track.path}
                label={track.label}
                hint="load"
                active={false}
                onClick={() => void addFromDrive(track)}
              />
            ))}
          </>
        )}
      </div>

      {/* Only when there is a choice to make. One audio track needs no menu. */}
      {audio.length > 1 && (
        <div className="border-t border-white/[0.07] py-1.5">
          <div className="px-3 py-1 font-mono text-[9.5px] uppercase tracking-[0.14em] text-textFaint">
            audio
          </div>
          {audio.map((track) => (
            <Choice
              key={track.id}
              label={track.label}
              active={mpv.audioId === track.id}
              onClick={() => void mpv.selectAudio(track.id)}
            />
          ))}
        </div>
      )}

      <div className="border-t border-white/[0.07] px-3 py-2.5">
        <button
          onClick={() => void addFromDisk()}
          className="flex w-full items-center gap-2 rounded-md px-1 py-1 text-left text-[12px] text-textDim transition-colors hover:text-text"
        >
          <Plus size={13} />
          Add a subtitle file…
        </button>

        <div className="mt-2.5 flex items-center justify-between">
          <span className="text-[11px] text-textFaint">Sync</span>
          <div className="flex items-center gap-1">
            <Nudge label="−0.5s" onClick={() => nudge(-0.5)} />
            <Nudge label="−0.1s" onClick={() => nudge(-0.1)} />
            <span className="tnum w-[52px] text-center font-mono text-[11px] text-text">
              {mpv.subtitleDelay > 0 ? '+' : ''}
              {mpv.subtitleDelay.toFixed(1)}s
            </span>
            <Nudge label="+0.1s" onClick={() => nudge(0.1)} />
            <Nudge label="+0.5s" onClick={() => nudge(0.5)} />
          </div>
        </div>
        <p className="mt-1.5 text-[10.5px] leading-relaxed text-textFaint">
          {/* Which way is which is genuinely hard to remember, so it says. */}
          Plus if the subtitles are early, minus if they are late.
        </p>
      </div>

      <button
        onClick={onClose}
        aria-label="Close subtitle menu"
        className="absolute right-1.5 top-1.5 rounded p-1 text-textFaint transition-colors hover:text-text"
      >
        <X size={12} />
      </button>
    </motion.div>
  )
}

function Choice({
  label,
  hint,
  active,
  onClick,
}: {
  label: string
  hint?: string
  active: boolean
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className={cn(
        'flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left text-[12px] transition-colors',
        active ? 'bg-white/[0.07] text-text' : 'text-textDim hover:bg-white/[0.04]',
      )}
    >
      <span className="min-w-0 truncate">{label}</span>
      {hint && <span className="shrink-0 font-mono text-[9.5px] text-textFaint">{hint}</span>}
    </button>
  )
}

function Nudge({
  label,
  onClick,
}: {
  label: string
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className="rounded px-1.5 py-1 font-mono text-[10.5px] text-textDim transition-colors hover:bg-white/[0.07] hover:text-text"
    >
      {label}
    </button>
  )
}

/**
 * The progress bar: click anywhere, or drag along it.
 *
 * Dragging matters more than it sounds. Clicking alone means finding a moment
 * by guessing at it repeatedly, and every real player lets you scrub — the
 * pointer is captured so the drag keeps working past the ends of the bar.
 */
function Scrubber({
  percent,
  onSeek,
  disabled,
}: {
  percent: number
  onSeek: (fraction: number) => void
  disabled: boolean
}): React.JSX.Element {
  const fractionAt = (element: HTMLElement, clientX: number): number => {
    const box = element.getBoundingClientRect()
    return Math.max(0, Math.min(1, (clientX - box.left) / box.width))
  }

  return (
    <div
      role="slider"
      aria-label="Position"
      aria-valuenow={Math.round(percent)}
      aria-valuemin={0}
      aria-valuemax={100}
      tabIndex={0}
      onPointerDown={(e) => {
        if (disabled) return
        // Seek first, capture second. The other order loses the seek
        // entirely whenever `setPointerCapture` throws — which it does for
        // some synthesised pointers — and a timeline that ignores a click is
        // worse than one that cannot be dragged.
        onSeek(fractionAt(e.currentTarget, e.clientX))
        try {
          e.currentTarget.setPointerCapture(e.pointerId)
        } catch {
          // Dragging will not follow the pointer outside the bar. Clicking
          // still works, which is the part that matters.
        }
      }}
      onPointerMove={(e) => {
        // Only while the button is held; `buttons` is the reliable test,
        // because a plain move over the bar must not seek.
        if (disabled || e.buttons !== 1) return
        onSeek(fractionAt(e.currentTarget, e.clientX))
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
