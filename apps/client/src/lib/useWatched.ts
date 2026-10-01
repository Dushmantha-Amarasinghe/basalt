import { useCallback, useEffect, useRef, useState } from 'react'
import { api, inProgress, type Watched } from './api'

/** How often to sync while something is playing. */
const WHILE_PLAYING_MS = 15_000
/** And while it is not, to pick up what an external player has read. */
const IDLE_MS = 20_000

export interface WatchedLibrary {
  /** Everything watched, newest first. */
  all: Watched[]
  /** Only what is part-watched — what Continue watching shows. */
  continueWatching: Watched[]
  /** How far through one file, if it has been started. */
  of: (path: string) => Watched | undefined
  /** Records a position. Seconds, not a fraction: this knows the duration. */
  report: (path: string, position: number, duration: number) => void
  /** Drops a file from the list entirely. */
  forget: (path: string) => void
  refresh: () => void
}

/**
 * Where everything has been watched to, kept on the host.
 *
 * Polled rather than pushed, and slowly, because the interesting source is an
 * *external* player: it never tells anyone anything, and the only signal is
 * what the media proxy has seen it read. That arrives whenever the host is
 * asked, so asking on a timer is the mechanism rather than a shortcut.
 */
/**
 * `who` is whose history this is — a profile's id, or empty for the device —
 * so a change of profile shows the new person's list straight away.
 */
export function useWatched(host: string | null, who = ''): WatchedLibrary {
  const [all, setAll] = useState<Watched[]>([])
  const live = useRef(true)
  /** Whose list a plain poll is out for, so a second is not sent alongside. */
  const inFlight = useRef<string | null>(null)
  /** Whose list on which host is wanted now, for discarding a late answer. */
  const current = useRef(`${host ?? ''}|${who}`)
  current.current = `${host ?? ''}|${who}`
  /** Set while the in-app player is running, to poll a little faster. */
  const playing = useRef(false)

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const sync = useCallback(
    async (update?: Watched, forget?: string) => {
      if (!host) return
      const asked = current.current
      // A report must never be dropped for being concurrent with a poll, so
      // only the plain polls skip — and only for a poll out for this same
      // list: one still out to the last host must not stop this one asking.
      if (inFlight.current === asked && !update && !forget) return
      inFlight.current = asked
      try {
        const entries = await api.watchProgress(update, forget)
        // Another drive's or another person's list: not to be shown here.
        if (live.current && current.current === asked) setAll(entries)
      } catch {
        // The host being briefly unreachable costs a resume point being a few
        // seconds stale, which is not worth telling anyone about.
      } finally {
        if (inFlight.current === asked) inFlight.current = null
      }
    },
    [host],
  )

  useEffect(() => {
    // Someone else's list, or another drive's, is never shown while this one
    // loads.
    setAll([])
    if (!host) return
    void sync()
    const timer = setInterval(() => {
      void sync()
    }, playing.current ? WHILE_PLAYING_MS : IDLE_MS)
    return () => clearInterval(timer)
  }, [host, sync, who])

  const report = useCallback(
    (path: string, position: number, duration: number) => {
      if (!path || !Number.isFinite(duration) || duration <= 0) return
      playing.current = true
      void sync({
        path,
        fraction: Math.min(1, Math.max(0, position / duration)),
        position,
        duration,
        // The host stamps this; the value sent is ignored.
        updatedAt: 0,
      })
    },
    [sync],
  )

  return {
    all,
    continueWatching: all.filter(inProgress),
    of: useCallback((path: string) => all.find((w) => w.path === path), [all]),
    report,
    forget: useCallback((path: string) => void sync(undefined, path), [sync]),
    refresh: useCallback(() => void sync(), [sync]),
  }
}
