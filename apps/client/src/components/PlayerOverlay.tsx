import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertCircle,
  Check,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  Loader2,
  Maximize2,
  Music,
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
import { onExternalFileDrop, pickSubtitleFile } from '@/lib/dialogs'
import { useAsyncSubscription, useLatest } from '@/lib/useAsyncSubscription'
import { useMpv, type Mpv, type MpvTrack } from '@/lib/useMpv'
import {
  chooseDriveFile,
  chooseSubtitle,
  describeTrack,
  labelOf,
  loadPref,
  prefFor,
  savePref,
} from '@/lib/subtitleChoice'
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

  /**
   * Puts the subtitle menu away. From the picture, the controls go too:
   * that click means back to the film, and they would otherwise sit there
   * for the idle time on top of it.
   */
  const dismissMenu = useCallback((hideControls: boolean) => {
    setMenu(false)
    setOverBar(false)
    if (!hideControls) return
    if (idleTimer.current) clearTimeout(idleTimer.current)
    setShowControls(false)
  }, [])

  const pinned = mpv.paused || menu || overBar || !mpv.picture
  const controlsUp = showControls || pinned
  const open = item !== null

  // Any movement anywhere brings them back, including over the controls. A
  // key does too, but from the key handler itself — see there for why.
  useEffect(() => {
    if (!open) return undefined
    keepControls()
    window.addEventListener('mousemove', keepControls)
    return () => {
      window.removeEventListener('mousemove', keepControls)
      if (idleTimer.current) clearTimeout(idleTimer.current)
    }
  }, [open, keepControls])

  /**
   * Everything behind the player goes, not just the app's own view.
   *
   * Hiding the app's wrapper was not enough: the file list sets `visibility`
   * on each of its rows, which beats an inherited `hidden`, so the rows of the
   * folder a film was opened from were drawn across the film. Menus and
   * dialogs are outside the wrapper altogether. The rule this switches on
   * hides every element on the page but the player's own, and nothing can
   * override it.
   *
   * The page stays opaque black until there is a picture to show through it.
   * See-through any earlier is see-through onto nothing: the desktop, or the
   * app, behind the controls while the film is still opening.
   */
  useEffect(() => {
    if (!open) return undefined
    document.body.classList.add('player-open')
    return () => document.body.classList.remove('player-open')
  }, [open])
  useEffect(() => {
    if (!open || !mpv.picture) return undefined
    document.body.classList.add('player-live')
    return () => document.body.classList.remove('player-live')
  }, [open, mpv.picture])

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

  /** Whether the window was maximised before it went fullscreen. */
  const wasMaximised = useRef(false)

  /**
   * Fullscreen, including from a maximised window.
   *
   * A maximised window refuses to go fullscreen — the call is accepted and
   * simply does nothing, which is exactly how it was reported: the button
   * worked from a normal window and did nothing from a maximised one. So it
   * is unmaximised first, and put back on the way out, because coming out of
   * fullscreen into a small window when you started maximised is its own
   * small annoyance.
   */
  const fullscreen = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const window = getCurrentWindow()

      if (await window.isFullscreen()) {
        await window.setFullscreen(false)
        if (wasMaximised.current) {
          wasMaximised.current = false
          await window.maximize()
        }
        return
      }

      wasMaximised.current = await window.isMaximized()
      if (wasMaximised.current) await window.unmaximize()
      await window.setFullscreen(true)
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
      if (await getCurrentWindow().isFullscreen()) {
        // Through the same path, so the window is put back the way it was.
        await fullscreenRef.current()
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
      if (await getCurrentWindow().isFullscreen()) await fullscreenRef.current()
    } catch {
      // Not in the shell.
    }
    onClose()
  }, [onClose])

  /**
   * A subtitle track chosen, or none — and remembered for the next video.
   */
  const chooseTrack = useCallback(
    (id: number | null) => {
      void mpv.selectSubtitle(id)
      const track = id === null ? null : mpv.tracks.find((t) => t.id === id)
      savePref(prefFor(track ? describeSub(track) : null))
    },
    [mpv],
  )

  const addFromDrive = useCallback(
    async (file: SubtitleTrack) => {
      // Through the proxy: mpv reaches the host the same way the video does.
      const url = await api.mediaUrl(file.path)
      if (url) await mpv.addSubtitle(url)
      savePref({ ...loadPref(), on: true })
    },
    [mpv],
  )

  const addFromDisk = useCallback(async () => {
    const path = await pickSubtitleFile()
    if (path) {
      await mpv.addSubtitle(path)
      savePref({ ...loadPref(), on: true })
    }
  }, [mpv])

  /**
   * Subtitles as they were last left, when a video opens.
   *
   * Once per video, after its tracks are known, and never again for it — so
   * whatever is chosen from the menu while watching is not undone.
   */
  const settled = useRef<string | null>(null)
  useEffect(() => {
    if (!item || !mpv.started || settled.current === item.id) return
    // The track list arrives just after the picture does.
    if (mpv.tracks.length === 0) return
    settled.current = item.id
    const pref = loadPref()
    const subs = mpv.tracks.filter((t) => t.kind === 'sub').map(describeSub)
    const pick = chooseSubtitle(subs, pref)
    if (pick !== null) {
      void mpv.selectSubtitle(pick)
      return
    }
    // Nothing in the file: a matching file beside it, if subtitles are on.
    const beside = subs.length === 0 ? chooseDriveFile(subtitles, pref) : null
    if (beside) {
      void api.mediaUrl(beside.path).then((url) => {
        if (url) void mpv.addSubtitle(url)
      })
    } else {
      void mpv.selectSubtitle(null)
    }
  }, [item, mpv, mpv.started, mpv.tracks, subtitles])

  /**
   * Subtitle files dropped on the video, as any player takes them.
   *
   * Only subtitle files: anything else dropped here is said to be the wrong
   * kind rather than uploaded, which is what the drive underneath would have
   * done with it — the app's own drop handling stands aside while a video is
   * open.
   */
  const [dropping, setDropping] = useState(false)
  const [dropNote, setDropNote] = useState<string | null>(null)
  const dropTarget = useLatest({ mpv })
  const subscribeToDrops = useCallback(
    () =>
      onExternalFileDrop({
        onEnter: () => setDropping(true),
        onOver: () => {},
        onLeave: () => setDropping(false),
        onDrop: (paths) => {
          setDropping(false)
          const subs = paths.filter(isSubtitleFile)
          if (subs.length === 0) {
            setDropNote('Only subtitle files can be dropped on a video.')
            return
          }
          void (async () => {
            for (const path of subs) await dropTarget.current.mpv.addSubtitle(path)
            savePref({ ...loadPref(), on: true })
            setDropNote(subs.length === 1 ? 'Subtitles added.' : `${subs.length} subtitle files added.`)
          })()
        },
      }),
    [dropTarget],
  )
  useAsyncSubscription(open, subscribeToDrops)
  useEffect(() => {
    if (!dropNote) return undefined
    const timer = setTimeout(() => setDropNote(null), 2200)
    return () => clearTimeout(timer)
  }, [dropNote])

  /** C turns subtitles on and off, as on YouTube. */
  const toggleSubtitles = useCallback(() => {
    if (mpv.subtitleId !== null) {
      chooseTrack(null)
      return
    }
    const subs = mpv.tracks.filter((t) => t.kind === 'sub').map(describeSub)
    const pick = chooseSubtitle(subs, { ...loadPref(), on: true })
    if (pick !== null) chooseTrack(pick)
  }, [mpv, chooseTrack])

  /**
   * The keys, and the one listener that hears them.
   *
   * Every key acts on its first press, controls up or not, and brings the
   * controls up as well. It used to take two presses whenever the controls
   * were hidden, and there were two separate reasons:
   *
   * - Showing the controls and acting on the key were two listeners, and the
   *   acting one was re-attached whenever the player re-rendered. The first
   *   listener showing the controls *was* a re-render — React runs it between
   *   the two listeners — so the second was detached before its turn came,
   *   and the key only brought the controls up. Now there is one listener,
   *   attached once, reading the current handler from a ref.
   * - Keys aimed at an input were left alone, so a text field could be typed
   *   in, and the volume slider is an input. After using the slider it kept
   *   focus, so Space did nothing and the arrows nudged the slider by a single
   *   step instead of the volume by five.
   */
  const onKey = useRef<(e: KeyboardEvent) => void>(() => {})
  onKey.current = (e: KeyboardEvent): void => {
    const target = e.target as HTMLElement | null
    const typing = target?.closest(
      'textarea, [contenteditable="true"], input:not([type="range"])',
    )
    if (typing) return

    let handled = true
    switch (e.key) {
      case 'Escape':
        // The menu first, if it is open: Escape putting away the thing in
        // front of you is universal, and ending the film instead is not.
        if (menu) setMenu(false)
        else void escape()
        break
      case ' ':
      case 'k':
        void mpv.togglePause()
        break
      case 'ArrowRight':
        void mpv.seekBy(e.shiftKey ? 60 : 5)
        break
      case 'ArrowLeft':
        void mpv.seekBy(e.shiftKey ? -60 : -5)
        break
      case 'ArrowUp':
        void mpv.nudgeVolume(5)
        flashVolume()
        break
      case 'ArrowDown':
        void mpv.nudgeVolume(-5)
        flashVolume()
        break
      // mpv's own keys for this, because anyone who wants frame stepping
      // already knows them.
      case '.':
        void mpv.stepFrame(1)
        break
      case ',':
        void mpv.stepFrame(-1)
        break
      case 'm':
        void mpv.toggleMute()
        break
      case 'c':
        toggleSubtitles()
        break
      case 'f':
        void fullscreen()
        break
      default:
        handled = false
        break
    }
    // A player key, and only that. Without this a focused control would act
    // on it too — Space on the last button pressed, the arrows on the slider
    // — so one press did two things.
    if (handled) e.preventDefault()
    keepControls()
  }

  useEffect(() => {
    if (!open) return undefined
    const listener = (e: KeyboardEvent): void => onKey.current(e)
    window.addEventListener('keydown', listener)
    return () => window.removeEventListener('keydown', listener)
  }, [open])

  const percent = mpv.duration > 0 ? (mpv.position / mpv.duration) * 100 : 0
  const problem = failed ?? mpv.problem ?? mpv.loadFailed

  return (
    <AnimatePresence>
      {item && (
        <motion.div
          // Solid from its first frame. It used to fade in, and for those
          // frames the half-hidden app and the desktop showed through it.
          initial={false}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          data-player=""
          className={cn('fixed inset-0 z-40', !mpv.picture && 'bg-black')}
        >
          {/*
            The stage: black, until there is a picture behind it.

            Once there is, this area is transparent, and that is not a style
            choice — mpv is drawing behind this page, and anything painted
            here would cover the film. Until then there is nothing back there,
            and a transparent hole shows whatever is behind the window. On
            screen that read as two windows: the controls in one, the app or
            the desktop in the other. So the player is built on black and
            only opens up once frames are reaching the screen — which
            `picture` is careful to wait for.
          */}
          <div
            onClick={onSingleClick}
            onDoubleClick={onDoubleClick}
            // A pointer over the picture is not over the bar, whatever the
            // bar last heard. Its `mouseleave` never comes when the pointer
            // leaves by way of a window on top — the file picker behind "Add
            // a subtitle file" — and the controls were then held up, as if
            // hovered, until the pointer went back over the bar and out.
            onMouseMove={() => setOverBar(false)}
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

            {/* Until the film is playing, so it is never over a picture. */}
            {!problem && !mpv.started && (
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center bg-black text-center">
                <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                  <Loader2 size={12} className="animate-spin" />
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

            {/* Playing, with nothing to show: music. Still the black stage,
                never a see-through window with a clock running in it. */}
            {!problem && mpv.started && !mpv.picture && (
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center bg-black text-center">
                <span className="flex h-16 w-16 items-center justify-center rounded-full bg-white/[0.06]">
                  <Music size={24} className="text-textDim" />
                </span>
                <div className="mt-5 px-8 text-2xl font-semibold tracking-tight text-text">
                  {item.title}
                </div>
                <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
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

            {/* A subtitle file on its way in. */}
            <AnimatePresence>
              {dropping && (
                <motion.div
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.12 }}
                  className="pointer-events-none absolute inset-4 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-white/40 bg-black/55"
                >
                  <div className="flex flex-col items-center gap-2 text-center">
                    <Subtitles size={26} className="text-text" />
                    <span className="text-[14px] font-medium text-text">Drop to add subtitles</span>
                    <span className="text-[11.5px] text-textDim">.srt, .ass, .ssa, .vtt, .sub or .sup</span>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>

            <AnimatePresence>
              {dropNote && (
                <motion.div
                  initial={{ opacity: 0, y: -6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                  className="pointer-events-none absolute left-1/2 top-12 z-20 -translate-x-1/2 rounded-full bg-black/75 px-4 py-2 text-[12px] text-text backdrop-blur"
                >
                  {dropNote}
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

          {/*
            With the menu open, a click on the picture means back to the film:
            the menu goes, the controls go with it, and the film plays — it
            carries on if it was playing, and continues if it was paused for
            the menu. Before, the click went through to the picture and
            paused it, and the menu stayed, holding the controls up until
            somebody found the bar and clicked there instead.
          */}
          {menu && (
            <div
              className="absolute inset-0"
              onClick={() => {
                dismissMenu(true)
                if (mpv.paused) void mpv.setPaused(false)
              }}
            />
          )}

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
                  {/* Clicking elsewhere on the bar puts it away. Behind the
                      menu itself, so the menu still takes its own clicks. The
                      picture has its own catcher, over the stage — `fixed`
                      here only ever covered the bar, because the bar moves
                      and a moving parent is what `fixed` is measured from. */}
                  <div
                    className="absolute inset-0 z-[5]"
                    onClick={() => dismissMenu(false)}
                  />
                  <SubtitleMenu
                    mpv={mpv}
                    fromDrive={subtitles}
                    onChoose={chooseTrack}
                    onAddFromDrive={(file) => void addFromDrive(file)}
                    onAddFromDisk={() => void addFromDisk()}
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

              {/* Straight on to the next episode, without waiting for the
                  end of this one — the credits, most of the time. */}
              {nextUp && onPlayNext && (
                <button
                  onClick={() => onPlayNext(nextUp.path)}
                  title={nextUp.label}
                  className="flex h-8 items-center gap-1 rounded-md px-2 text-[11px] text-textDim transition-colors hover:bg-white/[0.06] hover:text-text"
                >
                  {mpv.started && !mpv.picture ? 'Next track' : 'Next episode'}
                  <ChevronRight size={14} />
                </button>
              )}

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
 * Subtitles, audio, and whether the subtitles are in time with the sound.
 *
 * A header with the close button in it, rather than a button floated over the
 * first row — where it sat on top of "Off" and its highlight. Each track is
 * named by its language, with SDH and Forced as tags, and the chosen one has a
 * tick rather than a filled row, so the list reads as a list.
 *
 * The offset is here rather than buried in a settings screen because it is
 * needed *while watching* — a subtitle file from one release against a video
 * from another drifts, and the only way to correct it is to watch and nudge.
 */
function SubtitleMenu({
  mpv,
  fromDrive,
  onChoose,
  onAddFromDrive,
  onAddFromDisk,
  onClose,
}: {
  mpv: Mpv
  fromDrive: SubtitleTrack[]
  /** A subtitle track chosen, or null for off. */
  onChoose: (id: number | null) => void
  onAddFromDrive: (track: SubtitleTrack) => void
  onAddFromDisk: () => void
  onClose: () => void
}): React.JSX.Element {
  const inFile = mpv.tracks.filter((t) => t.kind === 'sub')
  const audio = mpv.tracks.filter((t) => t.kind === 'audio')
  // A file loaded from the drive shows up as a track once loaded, so it is
  // offered under "On the drive" only until then.
  const loadedNames = new Set(inFile.filter((t) => t.external).map((t) => t.title))
  const notLoaded = fromDrive.filter(
    (f) => !loadedNames.has(f.path.split('/').pop() ?? f.path),
  )

  const nudge = (by: number): void => {
    void mpv.setSubtitleDelay(Math.round((mpv.subtitleDelay + by) * 100) / 100)
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: [0.22, 1, 0.36, 1] }}
      style={{ transformOrigin: 'bottom right' }}
      // Opaque, not translucent. Over a bright frame a translucent menu washed
      // out to the point where the labels could not be read — and this is a
      // menu used *while* watching, so it is always over a picture.
      className="absolute bottom-full right-4 z-10 mb-3 w-[320px] overflow-hidden rounded-xl border border-white/[0.12] bg-[#141416] shadow-lift"
    >
      <div className="flex h-10 items-center justify-between border-b border-white/[0.07] pl-4 pr-2">
        <span className="text-[12.5px] font-semibold text-text">Subtitles</span>
        <button
          onClick={onClose}
          aria-label="Close subtitle menu"
          className="flex h-7 w-7 items-center justify-center rounded-md text-textFaint transition-colors duration-150 hover:bg-white/[0.07] hover:text-text"
        >
          <X size={14} />
        </button>
      </div>

      <div className="max-h-[280px] overflow-y-auto py-1.5">
        <Choice label="Off" active={mpv.subtitleId === null} onClick={() => onChoose(null)} />
        {inFile.map((track) => {
          const label = labelOf(describeSub(track))
          return (
            <Choice
              key={track.id}
              label={label.name}
              detail={label.detail}
              tags={label.tags}
              hint={track.external ? 'file' : undefined}
              active={mpv.subtitleId === track.id}
              onClick={() => onChoose(track.id)}
            />
          )
        })}

        {notLoaded.length > 0 && (
          <>
            <SectionTitle>On the drive</SectionTitle>
            {notLoaded.map((file) => (
              <Choice
                key={file.path}
                label={file.label}
                hint="load"
                active={false}
                onClick={() => onAddFromDrive(file)}
              />
            ))}
          </>
        )}

        {/* Only when there is a choice to make. One audio track needs no menu. */}
        {audio.length > 1 && (
          <>
            <SectionTitle>Audio</SectionTitle>
            {audio.map((track) => {
              const label = labelOf(describeSub(track))
              return (
                <Choice
                  key={track.id}
                  label={label.name}
                  detail={label.detail}
                  active={mpv.audioId === track.id}
                  onClick={() => void mpv.selectAudio(track.id)}
                />
              )
            })}
          </>
        )}
      </div>

      <div className="border-t border-white/[0.07] px-4 py-3">
        <button
          onClick={onAddFromDisk}
          className="flex w-full items-center gap-2 text-left text-[12px] text-textDim transition-colors duration-150 hover:text-text"
        >
          <Plus size={13} />
          Add a subtitle file
          <span className="ml-auto text-[10.5px] text-textFaint">or drop one on the video</span>
        </button>

        <div className="mt-3 flex items-center gap-2">
          <span className="w-9 text-[11px] text-textFaint">Sync</span>
          <div className="flex flex-1 items-center justify-between rounded-lg bg-white/[0.04] p-0.5">
            <Nudge label="−0.5" onClick={() => nudge(-0.5)} />
            <Nudge label="−0.1" onClick={() => nudge(-0.1)} />
            <button
              onClick={() => void mpv.setSubtitleDelay(0)}
              title="Back to 0"
              className="tnum w-[54px] rounded-md py-1 text-center font-mono text-[11px] text-text transition-colors duration-150 hover:bg-white/[0.06]"
            >
              {mpv.subtitleDelay > 0 ? '+' : ''}
              {mpv.subtitleDelay.toFixed(1)}s
            </button>
            <Nudge label="+0.1" onClick={() => nudge(0.1)} />
            <Nudge label="+0.5" onClick={() => nudge(0.5)} />
          </div>
        </div>
        <p className="mt-2 text-[10.5px] leading-relaxed text-textFaint">
          {/* Which way is which is genuinely hard to remember, so it says. */}
          Plus if the subtitles are early, minus if they are late.
        </p>
      </div>
    </motion.div>
  )
}

/** An mpv track as the subtitle rules read one. */
function describeSub(track: MpvTrack): ReturnType<typeof describeTrack> {
  return describeTrack({
    id: track.id,
    lang: track.lang,
    title: track.title,
    forced: track.forced,
    isDefault: track.isDefault,
    hearingImpaired: track.hearingImpaired,
    external: track.external,
  })
}

function SectionTitle({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="mt-1.5 px-4 pb-1 pt-2 font-mono text-[9.5px] uppercase tracking-[0.16em] text-textFaint">
      {children}
    </div>
  )
}

function Choice({
  label,
  detail,
  tags = [],
  hint,
  active,
  onClick,
}: {
  label: string
  detail?: string
  tags?: string[]
  hint?: string
  active: boolean
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className={cn(
        'flex w-full items-center gap-2.5 px-4 py-[7px] text-left text-[12.5px] transition-colors duration-100',
        active ? 'text-text' : 'text-textDim hover:bg-white/[0.04] hover:text-text',
      )}
    >
      <Check
        size={13}
        className={cn('shrink-0 transition-opacity duration-150', active ? 'opacity-100' : 'opacity-0')}
      />
      <span className="min-w-0 truncate">{label}</span>
      {detail && <span className="min-w-0 truncate text-[11.5px] text-textFaint">{detail}</span>}
      {tags.map((tag) => (
        <span
          key={tag}
          className="shrink-0 rounded border border-white/[0.12] px-1 py-px font-mono text-[9px] tracking-wide text-textDim"
        >
          {tag}
        </span>
      ))}
      {hint && (
        <span className="ml-auto shrink-0 font-mono text-[9.5px] text-textFaint">{hint}</span>
      )}
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
      className="rounded-md px-2 py-1 font-mono text-[10.5px] text-textDim transition-colors duration-150 hover:bg-white/[0.07] hover:text-text"
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

const SUBTITLE_EXTENSIONS = new Set(['srt', 'ass', 'ssa', 'vtt', 'sub', 'sup', 'idx'])

function isSubtitleFile(path: string): boolean {
  const dot = path.lastIndexOf('.')
  return dot > 0 && SUBTITLE_EXTENSIONS.has(path.slice(dot + 1).toLowerCase())
}
