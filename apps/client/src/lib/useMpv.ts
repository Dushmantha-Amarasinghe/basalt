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
 * `body` is normally opaque, and the player makes it see-through while it is
 * open. The controls are ordinary HTML over the top.
 *
 * **One instance for the life of the app.** `init` is expensive and there is
 * one window; opening a second file is a `loadfile`, not a second player.
 */

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
   * Whether mpv has a frame on screen yet.
   *
   * The player area is transparent so the video behind it shows through, and
   * before the first frame there is nothing behind it — so it has to stay
   * black until this turns true, or there is a second or two where the
   * controls float over the desktop and it reads as two windows.
   */
  picture: boolean
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
  ['dwidth', 'int64', 'none'],
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
  picture: false,
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
                const position = Number(data) || 0
                // Also the signal that there is a picture.
                //
                // `dwidth` alone was not enough: an observed property only
                // reports *changes*, and the next episode of a series is the
                // same resolution as the one before it. So nothing fired, and
                // the "opening" card sat on top of a film that was already
                // playing behind it — but only ever on autoplay, which is
                // what made it look like a different bug.
                const picture = s.picture || (position > 0 && s.duration > 0)
                return { ...s, position, picture }
              }
              case 'duration':
                return { ...s, duration: Number(data) || 0 }
              case 'volume':
                return { ...s, volume: Number(data) || 0 }
              case 'mute':
                return { ...s, muted: Boolean(data) }
              case 'eof-reached':
                return { ...s, ended: hasEnded(data, s.position, s.duration) }
              case 'paused-for-cache':
                return { ...s, buffering: data === true }
              case 'dwidth':
                // Non-null once a frame has actually been decoded and the
                // output is configured. Until then the window behind this
                // page is empty, and anything transparent shows the desktop.
                return { ...s, picture: Number(data) > 0 }
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

  const load = useCallback(
    async (url: string, startAt: number) => {
      if (!inTauri()) return
      // Transparent only while something is playing. The rest of the app is
      // opaque graphite and has no business showing the desktop through it.
      document.body.style.background = 'transparent'
      setState((s) => ({
      ...s,
      ended: false,
      picture: false,
      tracks: [],
      position: startAt,
      duration: 0,
    }))
      const options = startAt > 1 ? `start=${startAt.toFixed(3)}` : ''
      await mpv.command('loadfile', options ? [url, 'replace', '0', options] : [url])
      loaded.current = true
    },
    [],
  )

  const stop = useCallback(async () => {
    document.body.style.background = ''
    if (!inTauri() || !loaded.current) return
    loaded.current = false
    try {
      await mpv.command('stop', [])
    } catch {
      // Closing a player that already stopped is not a failure.
    }
    setState((s) => ({
      ...s,
      position: 0,
      duration: 0,
      ended: false,
      picture: false,
      tracks: [],
    }))
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
    toggleMute,
    selectSubtitle,
    selectAudio,
    addSubtitle,
    setSubtitleDelay,
    setSubtitleMargin,
  }
}
