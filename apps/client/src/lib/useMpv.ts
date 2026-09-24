import { useCallback, useEffect, useRef, useState } from 'react'
import * as mpv from 'tauri-plugin-libmpv-api'
import { inTauri } from './api'

/**
 * The player, which is mpv rendering behind the window.
 *
 * **Why not a `<video>` element.** WebView2 is Chromium, and Chromium refuses
 * most of what is on a real drive. Measured on this project's own library:
 * two of four titles are HEVC, which WebView2 will not decode without a paid
 * Store extension; two carry E-AC3, which it cannot decode at all; and the
 * one with AAC is in an MKV, where Chromium applies WebM's codec list and
 * drops the audio anyway. Every title was unplayable, for three different
 * reasons. mpv decodes all of it, and the host does not have to transcode —
 * which is what makes several devices watching several films at once cost the
 * host nothing but bytes.
 *
 * **How it gets on screen.** mpv draws into the native window *behind* the
 * webview, so the page has to get out of the way: the window is transparent,
 * `body` is normally opaque, and the player makes it see-through once there
 * is a `picture` to see. The controls are ordinary HTML over the top.
 *
 * **One instance for the life of the app.** `init` is expensive and there is
 * one window; opening a second file is a `loadfile`, not a second player.
 */

/** Where the sound goes. */
export interface AudioDevice {
  /** mpv's own name for it, which is what gets stored. */
  name: string
  description: string
}

/** Remembered across runs, and applied to mpv as it starts. */
const DEVICE_KEY = 'basalt.audioDevice'

export function savedAudioDevice(): string {
  try {
    return localStorage.getItem(DEVICE_KEY) ?? 'auto'
  } catch {
    return 'auto'
  }
}

/**
 * The outputs mpv can see, for the Settings list.
 *
 * Standalone rather than part of the hook: Settings has no player and should
 * not start one, and mpv is already running by the time anybody opens it.
 */
export async function audioDevices(): Promise<AudioDevice[]> {
  if (!inTauri()) return []
  // Read one indexed string at a time, never as a `node`.
  //
  // Asking for `audio-device-list` whole, as a node, segfaults the wrapper
  // outright — the app died the moment Settings opened. Strings cross that
  // boundary safely, which is how the track list is read too.
  const read = async (name: string): Promise<string> => {
    try {
      return String((await mpv.getProperty(name, 'string')) ?? '')
    } catch {
      return ''
    }
  }

  const count = Number(await read('audio-device-list/count')) || 0
  const devices: AudioDevice[] = []
  for (let i = 0; i < count; i++) {
    const name = await read(`audio-device-list/${i}/name`)
    if (!name) continue
    const description = await read(`audio-device-list/${i}/description`)
    devices.push({ name, description: description || name })
  }
  return devices
}

/**
 * One property, or null when mpv has no value for it right now.
 *
 * Never with `node`, for the reason above. Unavailable properties — the
 * clock before a file opens, the video size of a file with no video — are an
 * ordinary answer here, not an error.
 */
async function readProperty(
  name: string,
  format: 'string' | 'flag' | 'int64' | 'double',
): Promise<unknown> {
  try {
    return await mpv.getProperty(name, format)
  } catch {
    return null
  }
}

/** Switches output, and remembers it for next time. */
export async function setAudioDevice(name: string): Promise<void> {
  try {
    localStorage.setItem(DEVICE_KEY, name)
  } catch {
    // Private mode, or storage full. The choice still applies to this run.
  }
  if (inTauri()) await mpv.command('set', ['audio-device', name])
}

/** What the interface needs to draw itself, mirrored out of mpv. */
export interface MpvState {
  ready: boolean
  /** Set when mpv could not start at all — the player then says so. */
  problem: string | null
  paused: boolean
  position: number
  duration: number
  volume: number
  muted: boolean
  /** True once the file has played to the end. */
  ended: boolean
  /**
   * True while playback is stalled waiting for data.
   *
   * Worth its own state: a film that has run out of buffer looks identical
   * to a player that has crashed, and the difference matters over Wi-Fi.
   */
  buffering: boolean
  /**
   * Whether the file has started playing: opened, and its clock moving.
   *
   * Until then the player shows what it is opening, on black.
   */
  started: boolean
  /**
   * Whether mpv has video on screen, which is when the page may go
   * see-through.
   *
   * The player area is transparent so the video behind it shows through, and
   * before the first frame there is nothing behind it — so it stays black
   * until this turns true, or the controls float over the desktop and it
   * reads as two windows. False for the whole of a file with no video in it,
   * which is music: there is never anything behind the page to show.
   */
  picture: boolean
  /** Set when the file itself could not be opened. */
  loadFailed: string | null
  /** Subtitle and audio tracks inside the file, as mpv reports them. */
  tracks: MpvTrack[]
  /** The selected subtitle track id, or null for none. */
  subtitleId: number | null
  /** The selected audio track id. */
  audioId: number | null
  /** Seconds the subtitles are shifted by. Positive shows them later. */
  subtitleDelay: number
}

export interface MpvTrack {
  id: number
  kind: 'sub' | 'audio'
  label: string
  /** True for a track loaded from a separate file rather than the video. */
  external: boolean
}

const OBSERVED = [
  ['pause', 'flag'],
  ['time-pos', 'double', 'none'],
  ['duration', 'double', 'none'],
  ['volume', 'double', 'none'],
  ['mute', 'flag'],
  ['eof-reached', 'flag', 'none'],
  ['paused-for-cache', 'flag', 'none'],
  ['track-list/count', 'int64', 'none'],
  ['sid', 'int64', 'none'],
  ['aid', 'int64', 'none'],
  ['sub-delay', 'double', 'none'],
] as const satisfies mpv.MpvObservableProperty[]

/**
 * Whether the file has genuinely finished.
 *
 * `eof-reached` on its own is not "the film ended". mpv trips it whenever the
 * demuxer runs out of what it has — while idle, between files, and
 * transiently around a seek or a frame step. Trusting it cost two reported
 * bugs: stepping one frame jumped to the next episode, and so did clicking
 * the middle of the timeline.
 *
 * So the position has to agree. Something that has really ended sits within a
 * couple of seconds of its own duration, and nothing in the middle of a film
 * can pass that.
 */
export function hasEnded(flag: unknown, position: number, duration: number): boolean {
  if (flag !== true || duration <= 0) return false
  return position >= duration - END_SLACK
}

/** How close to the duration still counts as the end. */
const END_SLACK = 2

/** One look at mpv while a file is opening. */
export interface OpeningProbe {
  /** What mpv has open, or null while it has nothing. */
  path: string | null
  /** The playback clock, or null before there is one. */
  position: number | null
  /** Width of the video on screen; zero for a file with no video. */
  width: number
  paused: boolean
  /** True while mpv has no file at all. */
  idle: boolean
}

/** What has been seen so far of the file being opened. */
export interface OpeningWatch {
  /** The clock when it was first readable for this file. */
  first: number | null
  /** When that was, in milliseconds. */
  since: number
  /** Since when mpv has sat idle instead of opening anything. */
  idleSince: number | null
}

export const OPENING: OpeningWatch = { first: null, since: 0, idleSince: null }

/**
 * Whether the file being opened is on screen yet.
 *
 * The page goes see-through when this says `video`, so the one thing it must
 * never do is say so early: that is the moment the window shows whatever is
 * behind it. So neither of the obvious signals is used on its own. `dwidth`
 * is set when the output is configured, which is before a frame is drawn, and
 * as a property observer it does not fire at all when the next episode has
 * the same size as the last. A readable clock does not mean a frame either —
 * and straight after `loadfile` the clock still belongs to the file being
 * replaced.
 *
 * What does mean it: this file, by path, with its clock having moved on from
 * where it first stood. By then frames have been going to the screen for a
 * while. A file paused as it opens never moves, so being held for a moment
 * with a readable clock counts too — mpv shows the first frame of a paused
 * file.
 *
 * `sound` is a file that plays with nothing to show, and `failed` is mpv
 * having gone idle instead of opening it.
 */
export function judgeOpening(
  watch: OpeningWatch,
  probe: OpeningProbe,
  now: number,
  url: string,
): { watch: OpeningWatch; verdict: 'waiting' | 'video' | 'sound' | 'failed' } {
  if (probe.path !== url) {
    const idleSince = probe.idle ? (watch.idleSince ?? now) : null
    const failed = idleSince !== null && now - idleSince >= OPEN_FAILED_AFTER_MS
    return { watch: { ...OPENING, idleSince }, verdict: failed ? 'failed' : 'waiting' }
  }
  if (probe.position === null) return { watch: { ...watch, idleSince: null }, verdict: 'waiting' }
  if (watch.first === null) {
    return { watch: { first: probe.position, since: now, idleSince: null }, verdict: 'waiting' }
  }

  const moved = Math.abs(probe.position - watch.first) >= OPEN_MOVED_S
  const held = probe.paused && now - watch.since >= OPEN_HELD_MS
  if (!moved && !held) return { watch, verdict: 'waiting' }
  return { watch, verdict: probe.width > 0 ? 'video' : 'sound' }
}

/** How far the clock has to move before the picture is trusted. */
const OPEN_MOVED_S = 0.15
/** How long a paused file has to sit with a clock before it is. */
const OPEN_HELD_MS = 400
/** How long mpv may sit idle after `loadfile` before the file has failed. */
const OPEN_FAILED_AFTER_MS = 3000
/** How often an opening file is looked at. */
const PROBE_MS = 80

const EMPTY: MpvState = {
  ready: false,
  problem: null,
  paused: false,
  position: 0,
  duration: 0,
  volume: 100,
  muted: false,
  ended: false,
  buffering: false,
  started: false,
  picture: false,
  loadFailed: null,
  tracks: [],
  subtitleId: null,
  audioId: null,
  subtitleDelay: 0,
}

export interface Mpv extends MpvState {
  /** Opens a file or URL. */
  load: (url: string, startAt: number) => Promise<void>
  /** Stops playback and lets the window go opaque again. */
  stop: () => Promise<void>
  togglePause: () => Promise<void>
  setPaused: (paused: boolean) => Promise<void>
  seekTo: (seconds: number) => Promise<void>
  seekBy: (seconds: number) => Promise<void>
  /** One frame forward, or back. Exact — this is why mpv is here. */
  stepFrame: (direction: 1 | -1) => Promise<void>
  setVolume: (volume: number) => Promise<void>
  /** Nudges the volume, letting mpv do the arithmetic and the clamping. */
  nudgeVolume: (by: number) => Promise<void>
  toggleMute: () => Promise<void>
  selectSubtitle: (id: number | null) => Promise<void>
  selectAudio: (id: number) => Promise<void>
  /** Loads a subtitle file and selects it. */
  addSubtitle: (path: string) => Promise<void>
  setSubtitleDelay: (seconds: number) => Promise<void>
  /** Lifts the subtitles off the bottom edge, in pixels. */
  setSubtitleMargin: (pixels: number) => Promise<void>
}

export function useMpv(): Mpv {
  const [state, setState] = useState<MpvState>(EMPTY)
  const started = useRef(false)
  const loaded = useRef(false)
  /** mpv's own end-of-file flag, as last reported. */
  const eof = useRef<unknown>(false)

  /**
   * Every write goes through `set`, never `setProperty`.
   *
   * The plugin's `setProperty` has no format argument, so it sends a JS number
   * as a double — and mpv rejects a double for an int64 property. `sid` fails
   * that way, silently enough that a subtitle menu would simply never work.
   * `set` takes strings and mpv parses them, which is correct for every
   * property regardless of its type.
   */
  const set = useCallback(async (name: string, value: string | number | boolean) => {
    await mpv.command('set', [name, String(value)])
  }, [])

  const readTracks = useCallback(async () => {
    const count = Number(await mpv.getProperty('track-list/count', 'int64')) || 0
    const tracks: MpvTrack[] = []
    for (let i = 0; i < count; i++) {
      const kind = String(await mpv.getProperty(`track-list/${i}/type`, 'string') ?? '')
      if (kind !== 'sub' && kind !== 'audio') continue

      const id = Number(await mpv.getProperty(`track-list/${i}/id`, 'int64')) || 0
      const title = String(await mpv.getProperty(`track-list/${i}/title`, 'string') ?? '')
      const lang = String(await mpv.getProperty(`track-list/${i}/lang`, 'string') ?? '')
      const external =
        String(await mpv.getProperty(`track-list/${i}/external`, 'string') ?? '') === 'yes'

      // Titles are often more useful than codes — a file can carry `English`,
      // `English (SDH)` and `English (forced)`, which `en` three times does
      // not distinguish.
      const label = title || lang || `Track ${id}`
      tracks.push({ id, kind, label, external })
    }
    setState((s) => ({ ...s, tracks }))
  }, [])

  // One init for the life of the window.
  useEffect(() => {
    if (!inTauri() || started.current) return
    started.current = true

    let stop: (() => void) | undefined
    void (async () => {
      try {
        await mpv.init({
          initialOptions: {
            vo: 'gpu-next',
            hwdec: 'auto-safe',
            // Hold the last frame instead of closing, so finishing a file is
            // a state this code decides what to do with rather than mpv
            // tearing the window down underneath it.
            'keep-open': 'yes',
            // mpv's own keys and mouse handling would fight the interface's.
            'input-default-bindings': 'no',
            'input-vo-keyboard': 'no',
            osc: 'no',
            'osd-level': 0,
            // The volume slider goes to 130, and without this mpv silently
            // clamps at 100 — so the top third of the control did nothing.
            // Amplification earns its place on quietly mastered films.
            'volume-max': 130,
            // Whatever was chosen in Settings last time, or let mpv decide.
            'audio-device': savedAudioDevice(),
          },
          observedProperties: OBSERVED,
        })
        setState((s) => ({ ...s, ready: true }))

        stop = await mpv.observeProperties(OBSERVED, (event) => {
          const { name, data } = event as { name: string; data: unknown }
          setState((s) => {
            switch (name) {
              case 'pause':
                return { ...s, paused: Boolean(data) }
              case 'time-pos': {
                // The end is judged again as the clock moves, not only when
                // mpv flags it. Skipping past the end sets the flag before the
                // clock catches up, so judged then it was never the end, and
                // the flag does not change again — the episode sat finished
                // at 3:00 of 3:00 and the next one never started.
                const position = Number(data) || 0
                return { ...s, position, ended: hasEnded(eof.current, position, s.duration) }
              }
              case 'duration':
                return { ...s, duration: Number(data) || 0 }
              case 'volume':
                return { ...s, volume: Number(data) || 0 }
              case 'mute':
                return { ...s, muted: Boolean(data) }
              case 'eof-reached':
                eof.current = data
                return { ...s, ended: hasEnded(data, s.position, s.duration) }
              case 'paused-for-cache':
                return { ...s, buffering: data === true }
              case 'sid':
                return { ...s, subtitleId: data === null ? null : Number(data) }
              case 'aid':
                return { ...s, audioId: data === null ? null : Number(data) }
              case 'sub-delay':
                return { ...s, subtitleDelay: Number(data) || 0 }
              default:
                return s
            }
          })
          // The track list is only complete once the file is open, and its
          // length changing is the signal that it is.
          if (name === 'track-list/count') void readTracks()
        })
      } catch (e) {
        setState((s) => ({ ...s, problem: String(e) }))
      }
    })()

    return () => stop?.()
  }, [readTracks])

  /**
   * Which `load` is current. Each one watches its own file open, and stops
   * watching the moment another load or a stop replaces it.
   */
  const generation = useRef(0)

  /** Looks at mpv until the file is on screen, has failed, or is replaced. */
  const watchOpening = useCallback(async (url: string, mine: number) => {
    let watch = OPENING
    while (generation.current === mine) {
      await new Promise((resolve) => setTimeout(resolve, PROBE_MS))
      if (generation.current !== mine) return

      const [path, position, width, paused, idle] = await Promise.all([
        readProperty('path', 'string'),
        readProperty('time-pos', 'double'),
        readProperty('dwidth', 'int64'),
        readProperty('pause', 'flag'),
        readProperty('idle-active', 'flag'),
      ])
      const probe: OpeningProbe = {
        path: typeof path === 'string' && path ? path : null,
        position: typeof position === 'number' ? position : null,
        width: Number(width) || 0,
        paused: paused === true,
        idle: idle === true,
      }
      const next = judgeOpening(watch, probe, performance.now(), url)
      watch = next.watch
      if (next.verdict === 'waiting') continue
      if (generation.current !== mine) return

      if (next.verdict === 'failed') {
        setState((s) => ({ ...s, loadFailed: 'This file could not be opened.' }))
      } else {
        setState((s) => ({ ...s, started: true, picture: next.verdict === 'video' }))
      }
      return
    }
  }, [])

  const load = useCallback(
    async (url: string, startAt: number) => {
      if (!inTauri()) return
      const mine = ++generation.current
      eof.current = false
      setState((s) => ({
        ...s,
        ended: false,
        started: false,
        picture: false,
        loadFailed: null,
        tracks: [],
        position: startAt,
        duration: 0,
      }))
      try {
        // A file that is chosen is a file somebody wants to watch. Pause
        // survives `loadfile`, so without this the next episode after a
        // paused one opened paused, looking like it had not loaded.
        await set('pause', 'no')
        const options = startAt > 1 ? `start=${startAt.toFixed(3)}` : ''
        await mpv.command('loadfile', options ? [url, 'replace', '0', options] : [url])
        loaded.current = true
      } catch (e) {
        if (generation.current === mine) setState((s) => ({ ...s, loadFailed: String(e) }))
        return
      }
      void watchOpening(url, mine)
    },
    [set, watchOpening],
  )

  const stop = useCallback(async () => {
    generation.current++
    eof.current = false
    setState((s) => ({
      ...s,
      position: 0,
      duration: 0,
      ended: false,
      started: false,
      picture: false,
      loadFailed: null,
      tracks: [],
    }))
    if (!inTauri() || !loaded.current) return
    loaded.current = false
    try {
      await mpv.command('stop', [])
    } catch {
      // Closing a player that already stopped is not a failure.
    }
  }, [])

  const setPaused = useCallback(
    async (paused: boolean) => {
      if (inTauri()) await set('pause', paused ? 'yes' : 'no')
    },
    [set],
  )

  const togglePause = useCallback(async () => {
    if (inTauri()) await mpv.command('cycle', ['pause'])
  }, [])

  const seekTo = useCallback(async (seconds: number) => {
    if (!inTauri()) return
    // `absolute` and `exact`: a keyframe seek would land somewhere near the
    // scrubber rather than under it, and near is what makes resuming feel
    // approximate.
    await mpv.command('seek', [seconds.toFixed(3), 'absolute+exact'])
  }, [])

  const seekBy = useCallback(async (seconds: number) => {
    if (inTauri()) await mpv.command('seek', [String(seconds), 'relative'])
  }, [])

  const stepFrame = useCallback(async (direction: 1 | -1) => {
    if (!inTauri()) return
    // Stepping implies stopping; mpv pauses itself on `frame-step`, and doing
    // it here keeps the interface's idea of pause in step from the first key.
    await mpv.command(direction === 1 ? 'frame-step' : 'frame-back-step', [])
  }, [])

  const setVolume = useCallback(
    async (volume: number) => {
      if (inTauri()) await set('volume', Math.round(volume))
    },
    [set],
  )

  /**
   * Volume by delta, computed inside mpv.
   *
   * Reading our own mirrored `volume` and adding to it looks equivalent and
   * is not: that mirror is fed by a property observer, and an observer only
   * reports *changes*. Anything that failed to move the volume — asking for
   * more than `volume-max`, most obviously — left the mirror stale, so every
   * later press recomputed the same rejected number and the key did nothing
   * at all. `add` has no such problem, and mpv clamps it properly.
   */
  const nudgeVolume = useCallback(async (by: number) => {
    if (inTauri()) await mpv.command('add', ['volume', String(by)])
  }, [])

  const toggleMute = useCallback(async () => {
    if (inTauri()) await mpv.command('cycle', ['mute'])
  }, [])

  const selectSubtitle = useCallback(
    async (id: number | null) => {
      if (inTauri()) await set('sid', id === null ? 'no' : id)
    },
    [set],
  )

  const selectAudio = useCallback(
    async (id: number) => {
      if (inTauri()) await set('aid', id)
    },
    [set],
  )

  const addSubtitle = useCallback(async (path: string) => {
    if (!inTauri()) return
    // `select` makes it the active track, which is what someone who just
    // chose a file expects to happen.
    await mpv.command('sub-add', [path, 'select'])
  }, [])

  const setSubtitleDelay = useCallback(
    async (seconds: number) => {
      if (inTauri()) await set('sub-delay', seconds.toFixed(2))
    },
    [set],
  )

  const setSubtitleMargin = useCallback(
    async (pixels: number) => {
      if (inTauri()) await set('sub-margin-y', Math.round(pixels))
    },
    [set],
  )

  return {
    ...state,
    load,
    stop,
    togglePause,
    setPaused,
    seekTo,
    seekBy,
    stepFrame,
    setVolume,
    nudgeVolume,
    toggleMute,
    selectSubtitle,
    selectAudio,
    addSubtitle,
    setSubtitleDelay,
    setSubtitleMargin,
  }
}
