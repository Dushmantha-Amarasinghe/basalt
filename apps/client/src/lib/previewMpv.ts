import { useCallback, useEffect, useRef, useState } from 'react'
import type { Mpv, MpvState, MpvTrack } from './useMpv'
import * as showcase from './showcase'

/**
 * The player, in a plain browser.
 *
 * mpv only exists inside the app, so in the browser preview the player used
 * to open and wait for ever. This keeps its clock instead: the file opens,
 * plays, pauses, seeks and changes volume as the real one does, and the
 * picture behind the page is the showcase's footage, played by
 * `PreviewPicture` in the player. It is what the preview, and Basalt's
 * videos, show when something plays.
 */

/** A film's tracks, as a release often carries them. */
const FILM_TRACKS: MpvTrack[] = [
  { id: 1, kind: 'audio', label: 'English', external: false, lang: 'eng', title: '', forced: false, isDefault: true, hearingImpaired: false },
  { id: 2, kind: 'audio', label: 'Spanish', external: false, lang: 'spa', title: '', forced: false, isDefault: false, hearingImpaired: false },
  { id: 1, kind: 'sub', label: 'English', external: false, lang: 'eng', title: '', forced: false, isDefault: false, hearingImpaired: false },
  { id: 2, kind: 'sub', label: 'English SDH', external: false, lang: 'eng', title: 'SDH', forced: false, isDefault: false, hearingImpaired: true },
  { id: 3, kind: 'sub', label: 'Spanish', external: false, lang: 'spa', title: '', forced: false, isDefault: false, hearingImpaired: false },
  { id: 4, kind: 'sub', label: 'French', external: false, lang: 'fre', title: '', forced: false, isDefault: false, hearingImpaired: false },
]

const IDLE: MpvState = {
  ready: true,
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
  lighter: null,
  strain: null,
  converted: false,
}

/** How long opening takes, so the opening card is seen as it is in the app. */
const OPENING_MS = 700

export function usePreviewMpv(): Mpv {
  const [state, setState] = useState<MpvState>(IDLE)
  const opening = useRef<ReturnType<typeof setTimeout> | null>(null)

  // The clock, four times a second while playing.
  useEffect(() => {
    if (!state.started || state.paused || state.ended) return undefined
    const timer = setInterval(() => {
      setState((s) => {
        const position = Math.min(s.duration, s.position + 0.25)
        return { ...s, position, ended: s.duration > 0 && position >= s.duration }
      })
    }, 250)
    return () => clearInterval(timer)
  }, [state.started, state.paused, state.ended])

  const load = useCallback(async (url: string, startAt: number) => {
    const path = url.replace(/^showcase:/, '')
    const sound = /\.(mp3|flac|wav|m4a|aac|ogg|opus|wma)$/i.test(path)
    if (opening.current) clearTimeout(opening.current)
    setState((s) => ({
      ...IDLE,
      volume: s.volume,
      muted: s.muted,
      position: startAt,
      duration: showcase.durationOf(path),
    }))
    opening.current = setTimeout(() => {
      setState((s) => ({
        ...s,
        started: true,
        picture: !sound && showcase.footageFor(path) !== null,
        tracks: sound ? [] : FILM_TRACKS,
        audioId: sound ? null : 1,
        // `?lighter` shows the lighter mode, as a phone gives it a 4K film.
        lighter:
          !sound && new URLSearchParams(window.location.search).has('lighter')
            ? { width: 3840, height: 1920 }
            : null,
      }))
    }, OPENING_MS)
  }, [])

  const stop = useCallback(async () => {
    if (opening.current) clearTimeout(opening.current)
    setState((s) => ({ ...IDLE, volume: s.volume, muted: s.muted }))
  }, [])

  const seekTo = useCallback(async (seconds: number) => {
    setState((s) => ({ ...s, position: Math.max(0, Math.min(s.duration, seconds)), ended: false }))
  }, [])

  return {
    ...state,
    load,
    lighten: useCallback(async () => {
      setState((s) => ({ ...s, lighter: { width: 3840, height: 1920 } }))
    }, []),
    stop,
    togglePause: useCallback(async () => setState((s) => ({ ...s, paused: !s.paused })), []),
    setPaused: useCallback(async (paused: boolean) => setState((s) => ({ ...s, paused })), []),
    seekTo,
    seekBy: useCallback(
      async (by: number) =>
        setState((s) => ({ ...s, position: Math.max(0, Math.min(s.duration, s.position + by)) })),
      [],
    ),
    stepFrame: useCallback(async () => setState((s) => ({ ...s, paused: true })), []),
    setVolume: useCallback(async (volume: number) => setState((s) => ({ ...s, volume })), []),
    nudgeVolume: useCallback(
      async (by: number) =>
        setState((s) => ({ ...s, volume: Math.max(0, Math.min(130, s.volume + by)) })),
      [],
    ),
    toggleMute: useCallback(async () => setState((s) => ({ ...s, muted: !s.muted })), []),
    selectSubtitle: useCallback(async (id: number | null) => setState((s) => ({ ...s, subtitleId: id })), []),
    selectAudio: useCallback(async (id: number) => setState((s) => ({ ...s, audioId: id })), []),
    addSubtitle: useCallback(async (path: string) => {
      setState((s) => {
        const id = Math.max(0, ...s.tracks.filter((t) => t.kind === 'sub').map((t) => t.id)) + 1
        const title = path.split(/[\\/]/).pop() ?? path
        return {
          ...s,
          subtitleId: id,
          tracks: [
            ...s.tracks,
            { id, kind: 'sub', label: title, external: true, lang: '', title, forced: false, isDefault: false, hearingImpaired: false },
          ],
        }
      })
    }, []),
    setSubtitleDelay: useCallback(async (seconds: number) => setState((s) => ({ ...s, subtitleDelay: seconds })), []),
    setSubtitleMargin: useCallback(async () => {}, []),
  }
}

/** Lines shown as subtitles over the showcase's footage, in turn. */
const LINES = [
  'We lost the signal forty minutes ago.',
  'Then it came back. Stronger.',
  'Whoever is out there, they know we are listening.',
  'Bring us closer. Slowly.',
  'Look at the bands. Something is moving under them.',
  'That is not weather.',
]

/** The subtitle line for a moment, or nothing between lines. */
export function previewLine(position: number): string | null {
  const slot = Math.floor(position / 4)
  if (position % 4 > 3.3) return null
  return LINES[slot % LINES.length]!
}
