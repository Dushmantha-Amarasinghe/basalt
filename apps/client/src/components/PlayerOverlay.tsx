import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  Gauge,
  Sparkles,
  AlertCircle,
  Check,
  ExternalLink,
  Loader2,
  Maximize2,
  Minimize2,
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
  RotateCcw,
  RotateCw,
  Sun,
  SunDim,
  Volume1,
} from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatDuration } from '@/lib/mockMedia'
import { ApiError, api, inTauri, type SubtitleTrack } from '@/lib/api'
import { chosenFor, mergeDriveSubtitles, otherSubtitles, rememberChosen } from '@/lib/driveSubtitles'
import { onExternalFileDrop, pickSubtitleFile } from '@/lib/dialogs'
import { useAsyncSubscription, useLatest } from '@/lib/useAsyncSubscription'
import { PreviewPicture } from './PreviewPicture'
import { useMpv, type Mpv, type MpvTrack } from '@/lib/useMpv'
import {
  cannotConvert,
  pictureNote,
  rememberCannotConvert,
  rememberStrain,
  sizeFromName,
  sizeName,
  strainsAt,
  whyNotConverted,
  whyNotOptimized,
  type ConversionStatus,
  type NotConverted,
  type PictureHelp,
} from '@/lib/pictureHelp'
import { isAndroid } from '@/lib/platform'
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
import { android } from '@/lib/android'
import { getProperty } from '@/lib/mpvBackend'
import { isMobileShell } from '@/lib/platform'
import { useBack } from '@/mobile/useBack'

/**
 * A phone or tablet: fingers, not a pointer.
 *
 * A tap there shows or hides the controls instead of pausing, a double tap
 * at either side skips, the film takes the whole screen on its own, and the
 * fullscreen button turns the picture instead.
 */
const TOUCH = isMobileShell()

/** Motionless for this long and the controls step aside. */
const CONTROLS_IDLE = 2600

/** Which ways of helping the picture have been explained in this run. */
const pictureSaid = new Set<PictureHelp['mode']>()

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
  previous,
  subtitles = [],
  resolution = null,
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
  /** The episode or track before this one, when there is one. */
  previous?: { path: string; label: string } | null
  /** Subtitle files the host found beside this file. */
  subtitles?: SubtitleTrack[]
  /** The picture size the host measured, when it has. */
  resolution?: { width: number; height: number } | null
}): React.JSX.Element {
  const mpv = useMpv()
  const [failed, setFailed] = useState<string | null>(null)
  const [menu, setMenu] = useState(false)
  const [qualityMenu, setQualityMenu] = useState(false)

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
   * Puts the subtitle or quality menu away. From the picture, the controls go too:
   * that click means back to the film, and they would otherwise sit there
   * for the idle time on top of it.
   */
  const dismissMenu = useCallback((hideControls: boolean) => {
    setMenu(false)
    setQualityMenu(false)
    setOverBar(false)
    if (!hideControls) return
    if (idleTimer.current) clearTimeout(idleTimer.current)
    setShowControls(false)
  }, [])

  const pinned = mpv.paused || menu || qualityMenu || overBar || !mpv.picture

  // On a phone, back puts an open menu away before it closes the film.
  useBack(menu || qualityMenu, () => {
    setMenu(false)
    setQualityMenu(false)
    return true
  })
  const controlsUp = showControls || pinned
  const open = item !== null

  // Any movement anywhere brings them back, including over the controls. A
  // key does too, but from the key handler itself — see there for why.
  useEffect(() => {
    if (!open) return undefined
    keepControls()
    // A tap arrives with a mousemove of its own, which would bring the
    // controls straight back as a tap put them away.
    if (TOUCH) {
      return () => {
        if (idleTimer.current) clearTimeout(idleTimer.current)
      }
    }
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
        const at = resumeAt > 0 ? resumeAt : 0
        // A device that has struggled with pictures this large starts with
        // the host's conversion, when the host can give one: opening the file
        // first only to find it cannot keep up cost seconds every episode.
        // The size the library measured, or failing that the one its name gives.
        const size = resolution ?? sizeFromName(item.id)
        if (inTauri() && size && strainsAt(size)) {
          const check = await api.conversionCheck(item.id).catch(() => null)
          if (cancelled) return
          if (check?.duration) {
            triedConverting.current = item.id
            converting.current = { path: item.id, base: url, at, size }
            await load(url, at, { base: url, duration: check.duration })
            if (!cancelled) playingNow.current = item.id
            return
          }
        }
        await load(url, at)
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

  const finish = useCallback(() => {
    if (!item) return
    // `ended` is a state, not an event, and `item` is in the effect's
    // dependencies — so without a latch one true value re-fires for the next
    // episode, and the one after that, walking the whole series in a second.
    // Each file may hand over exactly once.
    if (advanced.current === item.id || playingNow.current !== item.id) return
    advanced.current = item.id

    const { duration } = latest.current
    if (duration > 0) report.current?.(item.id, duration, duration)
    const { nextUp: next, onPlayNext: play } = advance.current
    if (next && play) play(next.path)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item])

  useEffect(() => {
    if (mpv.ended) finish()
  }, [mpv.ended, finish])

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

  /** The controls up if they were down, and down if they were up. */
  const shown = useRef(showControls)
  shown.current = showControls
  const toggleControls = useCallback(() => {
    if (shown.current) {
      if (idleTimer.current) clearTimeout(idleTimer.current)
      setShowControls(false)
    } else {
      keepControls()
    }
  }, [keepControls])

  /**
   * Taps on a touch screen.
   *
   * One tap shows or hides the controls. Two, at the left or right third,
   * skip ten seconds back or forward, and each further tap on the same side
   * skips another ten, the way every phone player does it. Two in the middle
   * pause. The single tap waits to be sure it was one.
   */
  const lastTap = useRef<{ at: number; x: number } | null>(null)
  const skipping = useRef<{ side: -1 | 1; until: number } | null>(null)
  const [skipped, setSkipped] = useState<{ side: -1 | 1; seconds: number } | null>(null)
  const skippedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const skip = useCallback(
    (side: -1 | 1) => {
      void mpv.seekBy(side * 10)
      void android.haptic('tap')
      skipping.current = { side, until: performance.now() + 700 }
      setSkipped((was) => ({ side, seconds: was?.side === side ? was.seconds + 10 : 10 }))
      if (skippedTimer.current) clearTimeout(skippedTimer.current)
      skippedTimer.current = setTimeout(() => setSkipped(null), 700)
    },
    [mpv],
  )

  const onTap = useCallback(
    (e: React.MouseEvent) => {
      const now = performance.now()
      if (now - swipedAt.current < 350) return
      const width = window.innerWidth
      const side: -1 | 0 | 1 = e.clientX < width / 3 ? -1 : e.clientX > (width * 2) / 3 ? 1 : 0

      const run = skipping.current
      if (run && now < run.until && side === run.side) {
        skip(run.side)
        return
      }

      const before = lastTap.current
      if (before && now - before.at < 300 && Math.abs(before.x - e.clientX) < 80) {
        if (clickTimer.current) {
          clearTimeout(clickTimer.current)
          clickTimer.current = null
        }
        lastTap.current = null
        if (side === 0) void mpv.togglePause()
        else skip(side)
        return
      }

      lastTap.current = { at: now, x: e.clientX }
      if (clickTimer.current) clearTimeout(clickTimer.current)
      clickTimer.current = setTimeout(() => {
        clickTimer.current = null
        toggleControls()
      }, 260)
    },
    [mpv, skip, toggleControls],
  )

  /**
   * Swipes on a touch screen: up and down on the left half sets the screen's
   * brightness, on the right half the phone's media volume, as phone video
   * players do.
   *
   * Only a mostly vertical drag counts, so taps and double taps are left
   * alone, and a swipe never also counts as a tap. Brightness is this
   * window's own and goes back to the phone's when the player closes; volume
   * is the phone's media volume, the one its buttons change, moved in its own
   * steps so the phone never shows its panel on top of the film.
   */
  type Level = { kind: 'brightness' | 'volume'; value: number }
  const levels = useRef<{ brightness: number; volume: number; volumeSteps: number } | null>(null)
  const swipe = useRef<{
    id: number
    x: number
    y: number
    kind: Level['kind']
    from: number
    active: boolean
    last: number
  } | null>(null)
  const swipedAt = useRef(0)
  const [level, setLevel] = useState<Level | null>(null)
  const levelTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const readLevels = useCallback(() => {
    void android
      .playerLevels()
      .then((now) => {
        if (now) levels.current = now
      })
      .catch(() => {})
  }, [])

  const onSwipeStart = useCallback(
    (e: React.PointerEvent) => {
      if (e.pointerType !== 'touch' || !mpv.picture || !levels.current) return
      // Fresh, in case the phone's buttons moved the volume meanwhile; it is
      // back long before a swipe has gone far enough to count.
      readLevels()
      const kind = e.clientX < window.innerWidth / 2 ? 'brightness' : 'volume'
      swipe.current = {
        id: e.pointerId,
        x: e.clientX,
        y: e.clientY,
        kind,
        from: levels.current[kind],
        active: false,
        last: levels.current[kind],
      }
    },
    [mpv.picture, readLevels],
  )

  const onSwipeMove = useCallback((e: React.PointerEvent) => {
    const run = swipe.current
    if (!run || run.id !== e.pointerId) return
    const dx = e.clientX - run.x
    const dy = run.y - e.clientY
    if (!run.active) {
      // Past a small slop, and more up-and-down than sideways.
      if (Math.abs(dy) < 14 || Math.abs(dy) < Math.abs(dx) * 1.3) return
      run.active = true
      run.from = levels.current?.[run.kind] ?? run.from
      run.last = run.from
      run.y = e.clientY
      ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
      if (clickTimer.current) {
        clearTimeout(clickTimer.current)
        clickTimer.current = null
      }
      lastTap.current = null
    }
    // Most of the screen's height takes a level from bottom to top.
    const value = Math.min(1, Math.max(0, run.from + dy / (window.innerHeight * 0.7)))
    if (levelTimer.current) clearTimeout(levelTimer.current)
    if (run.kind === 'brightness') {
      if (Math.abs(value - run.last) < 0.005) return
      run.last = value
      void android.setBrightness(value)
      if (levels.current) levels.current.brightness = value
      setLevel({ kind: 'brightness', value })
    } else {
      const steps = levels.current?.volumeSteps ?? 15
      const stepped = Math.round(value * steps) / steps
      if (stepped !== run.last) {
        run.last = stepped
        void android.setVolume(stepped)
        if (levels.current) levels.current.volume = stepped
      }
      setLevel({ kind: 'volume', value: stepped })
    }
  }, [])

  const onSwipeEnd = useCallback((e: React.PointerEvent) => {
    const run = swipe.current
    if (!run || run.id !== e.pointerId) return
    swipe.current = null
    if (!run.active) return
    swipedAt.current = performance.now()
    if (levelTimer.current) clearTimeout(levelTimer.current)
    levelTimer.current = setTimeout(() => setLevel(null), 700)
  }, [])

  const onDoubleClick = useCallback(() => {
    if (clickTimer.current) {
      clearTimeout(clickTimer.current)
      clickTimer.current = null
    }
    void fullscreenRef.current()
  }, [])

  /**
   * On a phone, the way the screen is held for this film.
   *
   * A film fills the screen by itself: the status and navigation bars go,
   * and the screen turns to suit the picture — sideways for a film, upright
   * for something shot on a phone. The button then turns it the other way.
   * Closing the player gives the screen back as it was.
   */
  const [turned, setTurned] = useState<'landscape' | 'portrait' | null>(null)
  useEffect(() => {
    if (!TOUCH || !open || !mpv.picture) return undefined
    let cancelled = false
    void (async () => {
      const w = Number(await getProperty('dwidth', 'int64').catch(() => 0)) || 0
      const h = Number(await getProperty('dheight', 'int64').catch(() => 0)) || 0
      if (cancelled) return
      const way = w > 0 && h > w ? 'portrait' : 'landscape'
      setTurned(way)
      void android.setOrientation(way)
      void android.setImmersive(true)
    })()
    return () => {
      cancelled = true
    }
  }, [open, mpv.picture, item?.id])
  useEffect(() => {
    if (!TOUCH || open) return
    setTurned(null)
    void android.setImmersive(false)
    void android.setOrientation('auto')
    // The swipe's brightness was the player's; the app has the phone's.
    void android.setBrightness(-1)
    levels.current = null
    setLevel(null)
  }, [open])
  useEffect(() => {
    if (TOUCH && open && mpv.picture) readLevels()
  }, [open, mpv.picture, readLevels])

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
    if (TOUCH) {
      setTurned((was) => {
        const next = was === 'landscape' ? 'portrait' : 'landscape'
        void android.setOrientation(next)
        return next
      })
      return
    }
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

  /**
   * What the host says about this video's subtitles: null while it is being
   * asked. Asked for every video, from Library or from Files; a host too old
   * to know, or one that does not answer soon, leaves the library's list.
   */
  const [fromHost, setFromHost] = useState<{ tracks: SubtitleTrack[]; others: SubtitleTrack[] } | null>(
    null,
  )
  const videoPath = item?.id ?? null
  useEffect(() => {
    setFromHost(null)
    if (!videoPath || !inTauri()) {
      setFromHost({ tracks: [], others: [] })
      return undefined
    }
    let live = true
    const none = { tracks: [], others: [] }
    // Not waited for long: the subtitles chosen as a video opens wait on it.
    const giveUp = setTimeout(() => live && setFromHost((was) => was ?? none), 2500)
    api
      .subtitlesFor(videoPath)
      .then((found) => live && setFromHost(found))
      .catch(() => live && setFromHost(none))
    return () => {
      live = false
      clearTimeout(giveUp)
    }
  }, [videoPath])

  const drive = useMemo(
    () => mergeDriveSubtitles(subtitles, fromHost?.tracks ?? [], videoPath ? chosenFor(videoPath) : []),
    [subtitles, fromHost, videoPath],
  )
  const moreOnDrive = useMemo(() => otherSubtitles(fromHost?.others ?? [], drive), [fromHost, drive])

  const addFromDrive = useCallback(
    async (file: SubtitleTrack, byHand = false) => {
      // Through the proxy: mpv reaches the host the same way the video does.
      const url = await api.mediaUrl(file.path)
      if (url) await mpv.addSubtitle(url)
      savePref({ ...loadPref(), on: true })
      // Picked from the others by hand: offered again next time this plays.
      if (byHand && videoPath) rememberChosen(videoPath, file.path)
    },
    [mpv, videoPath],
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
    // And the host's answer about files on the drive, briefly waited for.
    if (fromHost === null) return
    settled.current = item.id
    const pref = loadPref()
    const subs = mpv.tracks.filter((t) => t.kind === 'sub').map(describeSub)
    const pick = chooseSubtitle(subs, pref)
    if (pick !== null) {
      void mpv.selectSubtitle(pick)
      return
    }
    // Nothing in the file: a matching file beside it, if subtitles are on.
    const beside = subs.length === 0 ? chooseDriveFile(drive, pref) : null
    if (beside) {
      void api.mediaUrl(beside.path).then((url) => {
        if (url) void mpv.addSubtitle(url)
      })
    } else {
      void mpv.selectSubtitle(null)
    }
  }, [item, mpv, mpv.started, mpv.tracks, drive, fromHost])

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

  /**
   * A file this device cannot play as it is: Basalt Host converts it as it is
   * watched, or, when it cannot, the device plays it lighter.
   *
   * The player starts the file itself, as always. When it reports `strain`
   * (decoding in software, and too large for that) the host is asked for a
   * conversion from the same moment. If that does not start, the file is
   * opened again where it was and played lighter, and the note says why.
   */
  const [help, setHelp] = useState<PictureHelp | null>(null)
  /** Arranging a conversion: the original is struggling and about to go. */
  const [preparing, setPreparing] = useState(false)
  /** The file a conversion has been tried for, so it is tried once. */
  const triedConverting = useRef<string | null>(null)
  /** A conversion asked for and not yet seen to start or fail. */
  const converting = useRef<{ path: string; base: string; at: number; size: PictureHelp['size'] } | null>(null)
  /** The original reopened after a failed conversion, to lighten once it plays. */
  const lightenWhenBack = useRef<{ path: string; why: NotConverted | null } | null>(null)

  useEffect(() => {
    setHelp(null)
    setPreparing(false)
    setReconnecting(null)
    setLost(null)
    triedConverting.current = null
    converting.current = null
    lightenWhenBack.current = null
    attempts.current = 0
    lastCut.current = null
    resumedAt.current = null
  }, [item?.id])

  const pictureLoad = mpv.load
  const pictureLighten = mpv.lighten

  /** Which host this is, to remember one that cannot convert. */
  const hostOf = useCallback(async () => (await api.status().catch(() => null))?.hostId ?? '', [])

  /** Back to the original where the conversion was asked for, lighter. */
  const fallBack = useCallback(
    async (asked: NonNullable<typeof converting.current>, status: ConversionStatus | null) => {
      if (converting.current !== asked) return
      converting.current = null
      const why = whyNotConverted(status)
      rememberCannotConvert(await hostOf(), why)
      lightenWhenBack.current = { path: asked.path, why }
      setHelp({ mode: 'lighter', size: asked.size, why })
      await pictureLoad(asked.base, asked.at)
    },
    [hostOf, pictureLoad],
  )

  useEffect(() => {
    if (!item || !mpv.strain || mpv.converted || triedConverting.current === item.id) return
    triedConverting.current = item.id
    const size = mpv.strain
    const at = mpv.position
    const duration = mpv.duration
    const path = item.id
    // Remembered, so the next film this large starts as a conversion.
    rememberStrain(size)
    setPreparing(true)
    void (async () => {
      // A host already known not to convert is not asked again this run: the
      // file is playing, and only needs lightening.
      const host = await hostOf()
      const known = cannotConvert(host)
      if (known) {
        setHelp({ mode: 'lighter', size, why: known })
        setPreparing(false)
        await pictureLighten()
        return
      }
      // Asked first, while the file plays on: a host that cannot convert,
      // or has no room, is then lightened in place rather than switched away
      // from and back, which cost a phone the time to open a 4K film twice.
      try {
        await api.conversionCheck(path).catch(async (e: unknown) => {
          // Busy may only be the last film letting go of its place, a moment
          // after it was closed: asked once more before giving up.
          if (!(e instanceof Error) || !/already converting/i.test(e.message)) throw e
          await new Promise((resolve) => setTimeout(resolve, 1500))
          return api.conversionCheck(path)
        })
      } catch (e) {
        const status = {
          by: null,
          error: e instanceof Error ? e.message : String(e),
          kind: e instanceof ApiError ? e.kind : null,
          at: Date.now(),
        }
        const why = whyNotConverted(status)
        rememberCannotConvert(host, why)
        setHelp({ mode: 'lighter', size, why })
        setPreparing(false)
        await pictureLighten()
        return
      }
      const base = await api.mediaUrl(path)
      if (!base) {
        setPreparing(false)
        return
      }
      const asked = { path, base, at, size }
      converting.current = asked
      const since = Date.now() - 1000
      await pictureLoad(base, at, { base, duration })
      // Opening the conversion now: the screen says so by itself from here.
      setPreparing(false)
      // A refusal is known the moment the host gives it; waiting for the
      // player to give up on the stream cost several seconds more.
      for (let tries = 0; tries < 20 && converting.current === asked; tries++) {
        await new Promise((resolve) => setTimeout(resolve, 300))
        const status = await api.conversionStatus(path).catch(() => null)
        if (!status || status.at < since) continue
        if (status.error) await fallBack(asked, status)
        break
      }
    })()
  }, [item, mpv.strain, mpv.converted, mpv.position, mpv.duration, pictureLoad, pictureLighten, hostOf, fallBack])

  // The conversion started, or did not.
  useEffect(() => {
    const asked = converting.current
    if (!asked || !item || asked.path !== item.id || !mpv.converted) return
    if (mpv.started) {
      converting.current = null
      void api
        .conversionStatus(asked.path)
        .catch(() => null)
        .then((status) => setHelp({ mode: 'converted', size: asked.size, by: status?.by ?? null }))
    } else if (mpv.loadFailed) {
      void api
        .conversionStatus(asked.path)
        .catch(() => null)
        .then((status) => fallBack(asked, status))
    }
  }, [item, mpv.converted, mpv.started, mpv.loadFailed, fallBack])

  /**
   * A conversion that stopped short: one that did not open after a seek, one
   * whose stream ended mid-film (the host went away, or ffmpeg gave up), and
   * one that has sat waiting for more with nothing arriving.
   *
   * Picked up where it was, again and again with longer waits, for as long
   * as the host may only be away for a moment; the file itself, lighter, only
   * when the host says it will not convert. Never "this file could not be
   * opened" for something that only needed a moment, and never the end of
   * the film, which is what a stream that stops looked like before.
   */
  const [reconnecting, setReconnecting] = useState<number | null>(null)
  /** Where it was when the host could not be reached, to try again from. */
  const [lost, setLost] = useState<number | null>(null)
  /** Tries since the conversion last played properly. */
  const attempts = useRef(0)
  /** Dealing with it now, so it is dealt with once. */
  const recovering = useRef(false)
  /** Where it last stopped by itself, to know a film that ends a little short. */
  const lastCut = useRef<number | null>(null)
  /** Where it was last picked up, to know it is playing properly again. */
  const resumedAt = useRef<number | null>(null)

  const recover = useCallback(
    async (cause: 'cut' | 'stalled' | 'failed', at: number, duration: number) => {
      if (!item || recovering.current) return
      const path = item.id
      // Stopped by itself at the same point twice: that is where the film
      // ends, whatever its header said about its length.
      if (cause === 'cut') {
        if (lastCut.current !== null && Math.abs(lastCut.current - at) < 2) {
          finish()
          return
        }
        lastCut.current = at
      }
      recovering.current = true
      setReconnecting(at)
      try {
        const status = await api.conversionStatus(path).catch(() => null)
        // A refusal counts only when it is what stopped it: the status of a
        // conversion that started fine says nothing about this one.
        const why = cause === 'failed' ? whyNotConverted(status) : 'failed'
        const base = await api.mediaUrl(path)
        if (!base) return
        const refused = why === 'unable' || why === 'off' || why === 'slow' || why === 'outdated'
        attempts.current += 1
        if (refused || attempts.current > RECOVER_WAITS.length) {
          if (!refused && status?.kind === 'offline') {
            setReconnecting(null)
            setLost(at)
            setFailed('Lost the connection to Basalt Host. Check that it is running, then try again.')
            return
          }
          // The host will not, or keeps failing: the file itself, from here.
          lightenWhenBack.current = { path, why }
          setHelp((was) => ({ mode: 'lighter', size: was?.size ?? resolution ?? { width: 0, height: 0 }, why }))
          setReconnecting(null)
          // Free before loading, so a load that fails at once is dealt with.
          recovering.current = false
          await pictureLoad(base, at)
          return
        }
        await new Promise((resolve) => setTimeout(resolve, RECOVER_WAITS[attempts.current - 1]))
        if (playingNow.current !== path) return
        resumedAt.current = at
        recovering.current = false
        await pictureLoad(base, at, { base, duration })
      } finally {
        recovering.current = false
      }
    },
    [item, finish, pictureLoad, resolution],
  )

  // Did not open: after a seek, or when picked up again.
  useEffect(() => {
    if (!item || !mpv.converted || !mpv.loadFailed || converting.current) return
    void recover('failed', mpv.position, mpv.duration)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item, mpv.converted, mpv.loadFailed])

  // Stopped mid-film, or waiting with nothing arriving. A moment's grace for
  // the first, since mpv passes through the end of what it has around a
  // seek; long enough for the second that a slow host still catching up is
  // left to, as the position moving starts the wait again.
  useEffect(() => {
    if (!item || !mpv.converted || !mpv.started || mpv.ended || reconnecting !== null) return undefined
    const cut = mpv.atEof
    if (!cut && !mpv.buffering) return undefined
    const at = mpv.position
    const duration = mpv.duration
    const timer = setTimeout(() => void recover(cut ? 'cut' : 'stalled', at, duration), cut ? 1500 : 15000)
    return () => clearTimeout(timer)
  }, [item, mpv.converted, mpv.started, mpv.ended, mpv.atEof, mpv.buffering, mpv.position, mpv.duration, reconnecting, recover])

  // Playing again: said no more, and once it has played on a while, the
  // waits start from the shortest again.
  useEffect(() => {
    if (!mpv.started || !mpv.converted) return
    if (reconnecting !== null && !recovering.current) setReconnecting(null)
    const from = resumedAt.current
    if (from === null || mpv.position > from + 20) {
      attempts.current = 0
      resumedAt.current = null
    }
  }, [mpv.started, mpv.converted, mpv.position, reconnecting])

  // Back on the original after a conversion did not start: lighter, now.
  useEffect(() => {
    const back = lightenWhenBack.current
    if (!back || !item || back.path !== item.id || mpv.converted || !mpv.started) return
    lightenWhenBack.current = null
    void pictureLighten()
  }, [item, mpv.converted, mpv.started, pictureLighten])

  /** Chosen in the menu: the conversion, or the file as it is. */
  const choosePicture = useCallback(
    async (mode: 'converted' | 'original') => {
      if (!item || !help) return
      const base = await api.mediaUrl(item.id)
      if (!base) return
      const at = mpv.position
      if (mode === 'converted') {
        converting.current = { path: item.id, base, at, size: help.size }
        await mpv.load(base, at, { base, duration: mpv.duration })
      } else {
        // On a phone the original only plays at all in the lighter mode.
        lightenWhenBack.current = isAndroid() ? { path: item.id, why: null } : null
        setHelp({ mode: 'lighter', size: help.size, why: null })
        await mpv.load(base, at)
      }
    },
    [item, help, mpv],
  )

  /**
   * Said once a run for each way of helping, the first time it is used: so a
   * softer picture, or a moment's pause as a film starts, does not read as
   * Basalt being slow. Not on every episode after; by then it has been said.
   */
  const [lighterShown, setLighterShown] = useState<[string, string] | null>(null)
  // A menu opened is somebody doing something: the note steps aside for it,
  // rather than sitting over the top of it.
  useEffect(() => {
    if (menu || qualityMenu) setLighterShown(null)
  }, [menu, qualityMenu])
  useEffect(() => {
    if (!help || pictureSaid.has(help.mode)) return undefined
    // The lighter mode is only said once it is actually on.
    if (help.mode === 'lighter' && !mpv.lighter) return undefined
    pictureSaid.add(help.mode)
    setLighterShown(pictureNote(help, TOUCH ? 'phone' : 'computer'))
    const timer = setTimeout(() => setLighterShown(null), 8000)
    return () => clearTimeout(timer)
  }, [help, mpv.lighter])
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

  /** On to the next episode or track, when there is one. */
  const goNext = useCallback(() => {
    if (nextUp && onPlayNext) onPlayNext(nextUp.path)
  }, [nextUp, onPlayNext])

  /**
   * Back: to the start of this one, or — near its start already — to the
   * one before. What every player's previous button does, so a single press
   * never throws away the place in something half watched.
   */
  const goPrevious = useCallback(() => {
    if (mpv.position > RESTART_WITHIN && mpv.duration > 0) {
      void mpv.seekTo(0)
      return
    }
    if (previous && onPlayNext) onPlayNext(previous.path)
    else void mpv.seekTo(0)
  }, [mpv, previous, onPlayNext])

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
        if (menu || qualityMenu) {
          setMenu(false)
          setQualityMenu(false)
        } else void escape()
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
      case 'N':
        goNext()
        break
      case 'P':
        goPrevious()
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
  // A conversion that did not open is handled, by trying again or by the
  // file itself, lighter; only the file itself failing is a problem to show.
  const problem = failed ?? mpv.problem ?? (mpv.converted ? null : mpv.loadFailed)
  /**
   * Between noticing this device cannot keep up and the conversion playing:
   * said as what it is, the host optimizing the film, rather than a player
   * that is slow to start.
   */
  const optimizing = !problem && reconnecting === null && (preparing || (mpv.converted && !mpv.started))

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
          {/* The browser preview's stand-in for mpv's picture. */}
          {!inTauri() && mpv.picture && (
            <PreviewPicture
              path={item.id}
              paused={mpv.paused}
              position={mpv.position}
              subtitles={mpv.subtitleId !== null}
            />
          )}
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
            onClick={TOUCH ? onTap : onSingleClick}
            onDoubleClick={TOUCH ? undefined : onDoubleClick}
            onPointerDown={TOUCH ? onSwipeStart : undefined}
            onPointerMove={TOUCH ? onSwipeMove : undefined}
            onPointerUp={TOUCH ? onSwipeEnd : undefined}
            onPointerCancel={TOUCH ? onSwipeEnd : undefined}
            style={TOUCH ? { touchAction: 'none' } : undefined}
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
                {lost !== null && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation()
                      const at = lost
                      setLost(null)
                      setFailed(null)
                      attempts.current = 0
                      void recover('stalled', at, mpv.duration)
                    }}
                    className="pointer-events-auto mt-5 rounded-full bg-white px-5 py-2 text-[13px] font-medium text-black transition-opacity hover:opacity-90"
                  >
                    Try again
                  </button>
                )}
              </div>
            )}

            {/* The host converting the film for this device: what the wait is. */}
            {optimizing && (
              <div className="pointer-events-none absolute inset-0 z-[1] flex flex-col items-center justify-center bg-black text-center">
                <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                  <Loader2 size={12} className="animate-spin" />
                  optimizing
                </div>
                <div className="mt-3 px-8 text-2xl font-semibold tracking-tight text-text">
                  {item.title}
                </div>
                <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
                <div className="mt-6 max-w-[440px] px-8 text-[12.5px] leading-relaxed text-textDim">
                  Basalt Host is converting this to 1080p so it plays smoothly on this{' '}
                  {TOUCH ? 'phone' : 'computer'}
                  {mpv.position > 5 ? `, from ${formatDuration(mpv.position)}` : ''}.
                </div>
              </div>
            )}

            {/* The conversion stopped short, and is being picked up again. */}
            {!problem && reconnecting !== null && (
              <div className="pointer-events-none absolute inset-0 z-[1] flex flex-col items-center justify-center bg-black text-center">
                <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.24em] text-textFaint">
                  <Loader2 size={12} className="animate-spin" />
                  picking up
                </div>
                <div className="mt-3 px-8 text-2xl font-semibold tracking-tight text-text">
                  {item.title}
                </div>
                <div className="mt-1 text-sm text-textDim">{item.subtitle}</div>
                <div className="mt-6 max-w-[440px] px-8 text-[12.5px] leading-relaxed text-textDim">
                  Basalt Host stopped sending the film for a moment. Picking it up again at{' '}
                  {formatDuration(reconnecting)}.
                </div>
              </div>
            )}

            {/* Until the film is playing, so it is never over a picture. */}
            {!problem && !optimizing && reconnecting === null && !mpv.started && (
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
              {lighterShown && (
                <motion.button
                  type="button"
                  onClick={() => setLighterShown(null)}
                  // Centred by motion rather than a class: motion owns the
                  // transform, and a class translating it was overwritten.
                  initial={{ opacity: 0, x: '-50%', y: -6 }}
                  animate={{ opacity: 1, x: '-50%', y: 0 }}
                  exit={{ opacity: 0, x: '-50%' }}
                  transition={{ duration: 0.2 }}
                  className="absolute left-1/2 top-12 z-20 flex w-[min(92%,26rem)] items-start gap-3 rounded-2xl bg-black/80 px-4 py-3 text-left backdrop-blur"
                >
                  {mpv.converted ? (
                    <Sparkles size={17} className="mt-0.5 shrink-0 text-textDim" />
                  ) : (
                    <Gauge size={17} className="mt-0.5 shrink-0 text-textDim" />
                  )}
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-text">{lighterShown[0]}</span>
                    <span className="mt-0.5 block text-[12px] leading-snug text-textDim">{lighterShown[1]}</span>
                  </span>
                </motion.button>
              )}
            </AnimatePresence>

            <AnimatePresence>
              {dropNote && (
                <motion.div
                  initial={{ opacity: 0, x: '-50%', y: -6 }}
                  animate={{ opacity: 1, x: '-50%', y: 0 }}
                  exit={{ opacity: 0, x: '-50%' }}
                  transition={{ duration: 0.15 }}
                  className="pointer-events-none absolute left-1/2 top-12 z-20 rounded-full bg-black/75 px-4 py-2 text-[12px] text-text backdrop-blur"
                >
                  {dropNote}
                </motion.div>
              )}
            </AnimatePresence>

            {/* What the volume keys just did. */}
            <AnimatePresence>
              {volumeOsd && (
                <motion.div
                  initial={{ opacity: 0, x: '-50%', scale: 0.94 }}
                  animate={{ opacity: 1, x: '-50%', scale: 1 }}
                  exit={{ opacity: 0, x: '-50%' }}
                  transition={{ duration: 0.12 }}
                  className="pointer-events-none absolute left-1/2 top-12 flex items-center gap-2.5 rounded-full bg-black/70 px-4 py-2.5 backdrop-blur"
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

            {/* What is paused, and what is next. */}
            <AnimatePresence>
              {mpv.paused && mpv.picture && !mpv.buffering && !menu && (
                <PausedPanel item={item} mpv={mpv} nextUp={nextUp} onPlayNext={onPlayNext} />
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

            {/* Not when the host is out of reach: another player could not reach it either. */}
            {onOpenExternally && problem && lost === null && (
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
              style={{
                pointerEvents: controlsUp ? 'auto' : 'none',
                ...(TOUCH
                  ? {
                      top: 'calc(var(--inset-top, 0px) + 12px)',
                      right: 'calc(var(--inset-right, 0px) + 12px)',
                    }
                  : {}),
              }}
              onClick={(e) => {
                e.stopPropagation()
                void leave()
              }}
              aria-label="Close player"
              className={cn(
                'no-drag absolute right-4 top-4 z-10 flex items-center justify-center rounded-full bg-black/40 text-textDim backdrop-blur transition-colors hover:bg-black/60 hover:text-text',
                TOUCH ? 'h-11 w-11' : 'h-9 w-9',
              )}
            >
              <X size={TOUCH ? 20 : 16} />
            </motion.button>

            {/* The level a swipe is setting, on the side it is set from. */}
            <AnimatePresence>
              {level && (
                <motion.div
                  key={level.kind}
                  // Centred through framer's transform: its scale animation
                  // replaces a class's translate, which sat the bar low.
                  style={{ y: '-50%' }}
                  initial={{ opacity: 0, scale: 0.94 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                  className={cn(
                    'pointer-events-none absolute top-1/2 flex flex-col items-center gap-3 rounded-full bg-black/60 px-3 py-4 backdrop-blur',
                    level.kind === 'brightness' ? 'left-[10%]' : 'right-[10%]',
                  )}
                >
                  {level.kind === 'brightness' ? (
                    level.value < 0.35 ? <SunDim size={18} /> : <Sun size={18} />
                  ) : level.value === 0 ? (
                    <VolumeX size={18} />
                  ) : level.value < 0.5 ? (
                    <Volume1 size={18} />
                  ) : (
                    <Volume2 size={18} />
                  )}
                  <div className="relative h-28 w-1.5 overflow-hidden rounded-full bg-white/20">
                    <div
                      className="absolute inset-x-0 bottom-0 rounded-full bg-white"
                      style={{ height: `${Math.round(level.value * 100)}%` }}
                    />
                  </div>
                  <span className="tnum font-mono text-[12px] text-text">{Math.round(level.value * 100)}</span>
                </motion.div>
              )}
            </AnimatePresence>

            {/* Where a double tap skipped to, on the side it was tapped. */}
            <AnimatePresence>
              {skipped && (
                <motion.div
                  key={skipped.side}
                  style={{ y: '-50%' }}
                  initial={{ opacity: 0, scale: 0.9 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                  className={cn(
                    'pointer-events-none absolute top-1/2 flex items-center gap-2 rounded-full bg-black/55 px-4 py-2.5 backdrop-blur',
                    skipped.side < 0 ? 'left-[12%]' : 'right-[12%]',
                  )}
                >
                  {skipped.side < 0 ? <RotateCcw size={16} /> : <RotateCw size={16} />}
                  <span className="tnum font-mono text-[13px] text-text">
                    {skipped.side < 0 ? '-' : '+'}
                    {skipped.seconds}s
                  </span>
                </motion.div>
              )}
            </AnimatePresence>
          </div>

          {/*
            With the menu open, a click on the picture means back to the film:
            the menu goes, the controls go with it, and the film plays — it
            carries on if it was playing, and continues if it was paused for
            the menu. Before, the click went through to the picture and
            paused it, and the menu stayed, holding the controls up until
            somebody found the bar and clicked there instead.
          */}
          {(menu || qualityMenu) && (
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
            // A finger on the controls is someone using them.
            onPointerDown={TOUCH ? keepControls : undefined}
            style={{
              pointerEvents: controlsUp ? 'auto' : 'none',
              ...(TOUCH
                ? {
                    paddingBottom: 'calc(var(--inset-bottom, 0px) + 14px)',
                    paddingLeft: 'calc(var(--inset-left, 0px) + 16px)',
                    paddingRight: 'calc(var(--inset-right, 0px) + 16px)',
                  }
                : {}),
            }}
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
                    fromDrive={drive}
                    more={moreOnDrive}
                    onChoose={chooseTrack}
                    onAddFromDrive={(file, byHand) => void addFromDrive(file, byHand)}
                    onAddFromDisk={() => void addFromDisk()}
                    onClose={() => setMenu(false)}
                  />
                </>
              )}
            </AnimatePresence>

            <AnimatePresence>
              {qualityMenu && help && (
                <QualityMenu
                  help={help}
                  converted={mpv.converted}
                  onChoose={(mode) => {
                    setQualityMenu(false)
                    void choosePicture(mode)
                  }}
                  onClose={() => setQualityMenu(false)}
                />
              )}
            </AnimatePresence>

            <Scrubber
              percent={percent}
              onSeek={(fraction) => void mpv.seekTo(fraction * mpv.duration)}
              disabled={mpv.duration === 0}
            />

            <div className="mt-3 flex items-center gap-2">
              {/* Previous and next beside play, the way every player has
                  them. Previous restarts this one first; next is only there
                  to press when there is something after this. */}
              <ControlButton
                icon={SkipBack}
                label={
                  previous && mpv.position <= RESTART_WITHIN
                    ? `Previous: ${previous.label} (Shift+P)`
                    : 'Back to the start (Shift+P)'
                }
                onClick={goPrevious}
              />
              <button
                onClick={() => void mpv.togglePause()}
                disabled={mpv.duration === 0}
                aria-label={mpv.paused ? 'Play' : 'Pause'}
                title={mpv.paused ? 'Play (Space)' : 'Pause (Space)'}
                className="mx-1 flex h-10 w-10 items-center justify-center rounded-full bg-basalt text-ink transition-transform duration-150 hover:scale-105 disabled:opacity-30 disabled:hover:scale-100"
              >
                {mpv.paused ? (
                  <Play size={17} className="ml-0.5 fill-ink" />
                ) : (
                  <Pause size={17} className="fill-ink" />
                )}
              </button>
              <ControlButton
                icon={SkipForward}
                label={nextUp ? `Next: ${nextUp.label} (Shift+N)` : 'Nothing after this'}
                onClick={goNext}
                disabled={!nextUp || !onPlayNext}
              />

              <div className={cn('mx-1.5 h-4 w-px bg-white/[0.1]', TOUCH && 'hidden')} />

              <SeekButton direction={-1} onClick={() => void mpv.seekBy(-10)} />
              <SeekButton direction={1} onClick={() => void mpv.seekBy(10)} />

              <span className="tnum ml-2 whitespace-nowrap font-mono text-[11px] text-textDim">
                {formatDuration(mpv.position)}
                <span className="text-textFaint"> / {formatDuration(mpv.duration)}</span>
              </span>

              <div className="flex-1" />

              {help && (
                <button
                  onClick={() => {
                    setMenu(false)
                    setQualityMenu((open) => !open)
                  }}
                  aria-label="Quality"
                  title="Quality"
                  className={cn(
                    'flex h-8 items-center gap-1.5 rounded-md px-2 font-mono text-[10.5px] transition-colors',
                    qualityMenu ? 'bg-white/[0.08] text-text' : 'text-textDim hover:bg-white/[0.06] hover:text-text',
                  )}
                >
                  {mpv.converted ? <Sparkles size={14} /> : <Gauge size={14} />}
                  {mpv.converted ? '1080p' : sizeName(help.size)}
                </button>
              )}

              <button
                onClick={() => {
                  setQualityMenu(false)
                  setMenu((open) => !open)
                }}
                aria-label="Subtitles"
                title="Subtitles"
                className={cn(
                  'flex h-8 items-center gap-1.5 rounded-md px-2 text-[11px] transition-colors',
                  // Nothing to read along to on a phone playing music.
                  TOUCH && !mpv.picture && 'hidden',
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

              {/* On a phone its own buttons are the volume control. */}
              <div className={cn('group/vol items-center gap-1.5', TOUCH ? 'hidden' : 'flex')}>
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
              {(!TOUCH || mpv.picture) && (
                <ControlButton
                  icon={TOUCH && turned === 'landscape' ? Minimize2 : Maximize2}
                  label={TOUCH ? 'Turn the picture' : 'Fullscreen (f)'}
                  onClick={fullscreen}
                />
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

/**
 * How long to wait before each try at picking a conversion up again: about a
 * minute in all with the tries themselves, long enough for a host computer to
 * restart its app or find the Wi-Fi again.
 */
const RECOVER_WAITS = [1000, 2000, 3000, 5000, 8000, 10000, 12000]

/** The tallest a menu over the controls may be: the screen less the bar. */
const MENU_HEIGHT = 'min(560px, calc(100dvh - var(--inset-top, 0px) - 132px))'

/**
 * Which picture to play: the host's conversion, made for this device, or the
 * file as it is.
 *
 * Its own button beside the subtitles, showing what is playing. As a section
 * at the bottom of the subtitle menu it was where nobody would look for it.
 */
function QualityMenu({
  help,
  converted,
  onChoose,
  onClose,
}: {
  help: PictureHelp
  converted: boolean
  onChoose: (mode: 'converted' | 'original') => void
  onClose: () => void
}): React.JSX.Element {
  const unavailable = help.mode === 'lighter' ? whyNotOptimized(help.why) : null
  const by = help.mode === 'converted' && help.by ? `, on its ${help.by}` : ''
  return (
    <motion.div
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: [0.22, 1, 0.36, 1] }}
      style={{ transformOrigin: 'bottom right', maxHeight: MENU_HEIGHT }}
      className="absolute bottom-full right-4 z-10 mb-3 flex w-[340px] flex-col overflow-hidden rounded-xl border border-white/[0.12] bg-[#141416] shadow-lift"
    >
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.07] pl-4 pr-2">
        <span className="text-[12.5px] font-semibold text-text">Quality</span>
        <button
          onClick={onClose}
          aria-label="Close quality menu"
          className="flex h-7 w-7 items-center justify-center rounded-md text-textFaint transition-colors duration-150 hover:bg-white/[0.07] hover:text-text"
        >
          <X size={14} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1.5">
        <QualityChoice
          icon={Sparkles}
          label="Optimized for this device"
          detail={unavailable ?? `1080p, converted by Basalt Host${by}. Smooth.`}
          active={converted}
          disabled={unavailable !== null}
          onClick={() => onChoose('converted')}
        />
        <QualityChoice
          icon={Gauge}
          label="Original"
          detail={`${sizeName(help.size)}, as the file is. ${
            TOUCH ? 'This phone may stutter, and plays it lighter.' : 'May stutter on this computer.'
          }`}
          active={!converted}
          onClick={() => onChoose('original')}
        />
      </div>
    </motion.div>
  )
}

function QualityChoice({
  icon: Icon,
  label,
  detail,
  active,
  disabled,
  onClick,
}: {
  icon: typeof Sparkles
  label: string
  detail: string
  active: boolean
  disabled?: boolean
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex w-full items-start gap-3 px-4 py-2.5 text-left transition-colors duration-150',
        disabled ? 'opacity-45' : 'hover:bg-white/[0.05]',
      )}
    >
      <Icon size={15} className={cn('mt-0.5 shrink-0', active ? 'text-text' : 'text-textFaint')} />
      <span className="min-w-0 flex-1">
        <span className={cn('block text-[13px]', active ? 'text-text' : 'text-textDim')}>{label}</span>
        <span className="mt-0.5 block text-[11.5px] leading-snug text-textFaint">{detail}</span>
      </span>
      {active && <Check size={14} className="mt-0.5 shrink-0 text-text" />}
    </button>
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
  more,
  onChoose,
  onAddFromDrive,
  onAddFromDisk,
  onClose,
}: {
  mpv: Mpv
  fromDrive: SubtitleTrack[]
  /** Others on the drive that might be meant for this video, by file name. */
  more: SubtitleTrack[]
  /** A subtitle track chosen, or null for off. */
  onChoose: (id: number | null) => void
  onAddFromDrive: (track: SubtitleTrack, byHand: boolean) => void
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
  const others = more.filter((f) => !loadedNames.has(f.path.split('/').pop() ?? f.path))
  // A few at first: a drive can offer dozens, and the menu is for watching.
  const [allOthers, setAllOthers] = useState(false)
  const shownOthers = allOthers ? others : others.slice(0, 6)

  const nudge = (by: number): void => {
    void mpv.setSubtitleDelay(Math.round((mpv.subtitleDelay + by) * 100) / 100)
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 6, scale: 0.98 }}
      transition={{ duration: 0.15, ease: [0.22, 1, 0.36, 1] }}
      // Opaque, not translucent. Over a bright frame a translucent menu washed
      // out to the point where the labels could not be read — and this is a
      // menu used *while* watching, so it is always over a picture.
      // As tall as the screen allows and no taller: on a phone turned on its
      // side a long list ran off the top, title and all. The list scrolls.
      style={{ transformOrigin: 'bottom right', maxHeight: MENU_HEIGHT }}
      className="absolute bottom-full right-4 z-10 mb-3 flex w-[320px] flex-col overflow-hidden rounded-xl border border-white/[0.12] bg-[#141416] shadow-lift"
    >
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.07] pl-4 pr-2">
        <span className="text-[12.5px] font-semibold text-text">Subtitles</span>
        <button
          onClick={onClose}
          aria-label="Close subtitle menu"
          className="flex h-7 w-7 items-center justify-center rounded-md text-textFaint transition-colors duration-150 hover:bg-white/[0.07] hover:text-text"
        >
          <X size={14} />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto py-1.5">
        <Choice label="Off" active={mpv.subtitleId === null} onClick={() => onChoose(null)} />
        {inFile.map((track) => {
          const label = labelOf(describeSub(track))
          // A file loaded from the drive is known to mpv by its file name;
          // the name the drive gave it, such as English, says more.
          const named = track.external
            ? [...fromDrive, ...more].find((f) => (f.path.split('/').pop() ?? f.path) === track.title)
            : undefined
          return (
            <Choice
              key={track.id}
              label={named && named.label !== track.title ? named.label : label.name}
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
                onClick={() => onAddFromDrive(file, false)}
              />
            ))}
          </>
        )}

        {others.length > 0 && (
          <>
            <SectionTitle>More on the drive</SectionTitle>
            {shownOthers.map((file) => (
              <Choice
                key={file.path}
                label={file.label}
                detail={file.path.split('/').slice(0, -1).join('/') || undefined}
                hint="load"
                active={false}
                onClick={() => onAddFromDrive(file, true)}
              />
            ))}
            {!allOthers && others.length > shownOthers.length && (
              <button
                onClick={() => setAllOthers(true)}
                className="w-full px-4 py-1.5 text-left text-[11.5px] text-textFaint transition-colors duration-150 hover:text-text"
              >
                {others.length - shownOthers.length} more
              </button>
            )}
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
  disabled,
}: {
  icon: typeof Play
  label: string
  onClick?: () => void
  disabled?: boolean
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={cn(
        'flex h-8 w-8 items-center justify-center rounded-md text-textDim',
        'transition-colors duration-150 hover:bg-white/[0.06] hover:text-text',
        'disabled:pointer-events-none disabled:opacity-30',
      )}
    >
      <Icon size={16} />
    </button>
  )
}

/** How far into something "previous" restarts it rather than going back. */
const RESTART_WITHIN = 5

/** Ten seconds back or forward: a turning arrow with the number inside. */
function SeekButton({
  direction,
  onClick,
}: {
  direction: 1 | -1
  onClick: () => void
}): React.JSX.Element {
  const Icon = direction === 1 ? RotateCw : RotateCcw
  const label = direction === 1 ? 'Forward 10 seconds (→ is 5)' : 'Back 10 seconds (← is 5)'
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className="relative flex h-8 w-8 items-center justify-center rounded-md text-textDim transition-colors duration-150 hover:bg-white/[0.06] hover:text-text"
    >
      <Icon size={20} strokeWidth={1.6} />
      <span className="absolute inset-0 flex items-center justify-center pt-[1px] font-mono text-[7.5px] font-semibold">
        10
      </span>
    </button>
  )
}

/**
 * What is paused, and what comes next: over the top of a paused picture,
 * like a streaming service's pause screen.
 *
 * Only when paused. While playing, the picture is the point and nothing sits
 * over it but the bar; paused is when somebody looks up and wants to know
 * where they are. "Up next" is only for something with a next — an episode —
 * and never for a film.
 */
function PausedPanel({
  item,
  mpv,
  nextUp,
  onPlayNext,
}: {
  item: MediaItem
  mpv: Mpv
  nextUp?: { path: string; label: string } | null
  onPlayNext?: (path: string) => void
}): React.JSX.Element {
  const sub = mpv.tracks.find((t) => t.kind === 'sub' && t.id === mpv.subtitleId)
  const audio = mpv.tracks.filter((t) => t.kind === 'audio')
  const playingAudio = audio.find((t) => t.id === mpv.audioId)
  const left = Math.max(0, mpv.duration - mpv.position)
  const details = [
    item.subtitle,
    audio.length > 1 && playingAudio ? labelOf(describeSub(playingAudio)).name : '',
    sub ? `${labelOf(describeSub(sub)).name} subtitles` : '',
    mpv.duration > 0 ? (left >= 60 ? `${Math.ceil(left / 60)} min left` : 'Almost done') : '',
  ].filter(Boolean)

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
      className="pointer-events-none absolute inset-x-0 top-0 z-[1] bg-gradient-to-b from-black/80 via-black/40 to-transparent px-8 pb-24 pt-9"
    >
      <div className="flex items-start gap-6">
        <motion.div
          initial={{ y: -6 }}
          animate={{ y: 0 }}
          transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
          className="min-w-0 flex-1"
        >
          <div className="font-mono text-[10px] uppercase tracking-[0.22em] text-textFaint">
            Paused
          </div>
          <div className="mt-2 truncate text-[26px] font-semibold leading-tight tracking-tight text-text">
            {item.title}
          </div>
          {details.length > 0 && (
            <div className="mt-1.5 truncate text-[12.5px] text-textDim">
              {details.join('  ·  ')}
            </div>
          )}
        </motion.div>

        {nextUp && onPlayNext && (
          <motion.button
            initial={{ y: -6, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            transition={{ duration: 0.25, delay: 0.05, ease: [0.22, 1, 0.36, 1] }}
            onClick={(e) => {
              e.stopPropagation()
              onPlayNext(nextUp.path)
            }}
            className="no-drag pointer-events-auto mr-12 flex shrink-0 items-center gap-3 rounded-xl border border-white/[0.12] bg-black/60 py-2.5 pl-4 pr-3 text-left backdrop-blur transition-colors duration-150 hover:border-white/25 hover:bg-black/75"
          >
            <span className="min-w-0">
              <span className="block font-mono text-[9.5px] uppercase tracking-[0.18em] text-textFaint">
                Up next
              </span>
              <span className="mt-0.5 block max-w-[240px] truncate text-[12.5px] font-medium text-text">
                {nextUp.label}
              </span>
            </span>
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-white/[0.1]">
              <SkipForward size={12} className="text-text" />
            </span>
          </motion.button>
        )}
      </div>
    </motion.div>
  )
}

const SUBTITLE_EXTENSIONS = new Set(['srt', 'ass', 'ssa', 'vtt', 'sub', 'sup', 'idx'])

function isSubtitleFile(path: string): boolean {
  const dot = path.lastIndexOf('.')
  return dot > 0 && SUBTITLE_EXTENSIONS.has(path.slice(dot + 1).toLowerCase())
}
