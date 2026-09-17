import { useCallback, useEffect, useRef, useState } from 'react'
import type { Entry } from '@/components/FileList'
import { ApiError, api, onStatus, toEntries, type Status } from './api'
import { useAsyncSubscription } from './useAsyncSubscription'

/**
 * Connection and navigation, in one hook.
 *
 * The rules it enforces, which are easy to get wrong scattered across
 * components:
 *
 * - A listing that arrives after the user has already navigated elsewhere is
 *   discarded. Without that check, clicking quickly through folders leaves you
 *   looking at the contents of one you have already left.
 * - Losing the connection does not clear what is on screen. A stale listing
 *   with a banner over it is far more useful than an empty window, and the
 *   moment the host comes back the same view is still there.
 * - Reconnection backs off. A host that is asleep should not be hammered once
 *   a second forever.
 */

const RETRY_MIN_MS = 2_000
const RETRY_MAX_MS = 30_000

/** Attempts before the interface admits, in words, that nothing is answering. */
export const STARTUP_ATTEMPTS_BEFORE_COMPLAINING = 4

/**
 * How long to wait before asking the backend for its status again.
 *
 * Quick at first, because the usual cause is the backend being a fraction of a
 * second behind the window, then backing off so a genuinely dead backend is not
 * polled forever. It never stops: a first call that fails must not be able to
 * strand the app on its splash screen, which is exactly what it used to do.
 */
export function startupRetryDelay(attempt: number): number {
  return Math.min(100 * 2 ** Math.max(0, attempt - 1), 4000)
}

export interface Vault {
  status: Status | null
  /** Vault-relative directory. `''` is the root. */
  dir: string
  entries: Entry[]
  loading: boolean
  error: ApiError | null
  space: [number, number] | null
  /** Set while the client is trying to get back to a host it knows. */
  reconnecting: boolean
  /** The backend never answered. The window is up but nothing is behind it. */
  startupFailed: boolean

  open: (dir: string) => void
  refresh: () => void
  reconnect: () => Promise<void>
  setStatus: (status: Status) => void
}

export function useVault(): Vault {
  const [status, setStatus] = useState<Status | null>(null)
  const [dir, setDir] = useState('')
  const [entries, setEntries] = useState<Entry[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<ApiError | null>(null)
  const [space, setSpace] = useState<[number, number] | null>(null)
  const [reconnecting, setReconnecting] = useState(false)
  /** Set when the backend has not answered at all, after several attempts. */
  const [startupFailed, setStartupFailed] = useState(false)

  // Which directory the newest request was for. A reply for anything else is
  // stale and must not be shown.
  const wanted = useRef('')
  const retryDelay = useRef(RETRY_MIN_MS)
  const retryTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const load = useCallback(async (target: string) => {
    wanted.current = target
    setLoading(true)
    try {
      const listing = await api.list(target)
      if (wanted.current !== target) return
      setEntries(toEntries(target, listing))
      setError(null)
      retryDelay.current = RETRY_MIN_MS
    } catch (e) {
      if (wanted.current !== target) return
      const err = e instanceof ApiError ? e : new ApiError('error', String(e))
      setError(err)
      // Deliberately not clearing `entries`: keeping the last good listing on
      // screen under a banner is better than an empty window.
    } finally {
      if (wanted.current === target) setLoading(false)
    }
  }, [])

  const open = useCallback(
    (target: string) => {
      setDir(target)
      void load(target)
    },
    [load],
  )

  const refresh = useCallback(() => {
    void load(wanted.current)
  }, [load])

  const reconnect = useCallback(async () => {
    setReconnecting(true)
    try {
      const next = await api.connectSaved()
      setStatus(next)
      setError(null)
      await load(wanted.current)
    } catch {
      // Left to the retry loop below; a failed attempt is the normal case
      // while the host is still waking up.
    } finally {
      setReconnecting(false)
    }
  }, [load])

  /**
   * First load: ask where we stand, then list the root if connected.
   *
   * Retried, because the first version gave up after one failure and the app
   * stayed on its splash screen forever — indistinguishable from a crash. The
   * failure that exposed it was a startup race in the backend, since fixed, but
   * the real fault was here: nothing should be able to leave the interface with
   * no state and no way to get any.
   */
  useEffect(() => {
    let cancelled = false
    let attempt = 0

    const ask = async (): Promise<void> => {
      try {
        const initial = await api.status()
        if (cancelled) return
        setStatus(initial)
        setStartupFailed(false)
        if (initial.connected) {
          void load('')
          api.space().then(setSpace).catch(() => {})
        }
      } catch (e) {
        if (cancelled) return
        attempt += 1
        if (attempt >= STARTUP_ATTEMPTS_BEFORE_COMPLAINING) {
          setStartupFailed(true)
          setError(new ApiError('error', String(e)))
        }
        setTimeout(() => void ask(), startupRetryDelay(attempt))
      }
    }

    void ask()
    return () => {
      cancelled = true
    }
  }, [load])

  // The backend reconnects in the background at startup and pushes the result,
  // so the window can open immediately instead of waiting on the network.
  useAsyncSubscription(
    true,
    useCallback(
      () =>
        onStatus((next) => {
          setStatus(next)
          if (next.connected) {
            void load(wanted.current)
            api.space().then(setSpace).catch(() => {})
          }
        }),
      [load],
    ),
  )

  // Retry while offline, backing off. Anything that is not a transport
  // problem — a missing folder, a denied path — is the user's to resolve and
  // retrying it would just fail the same way.
  useEffect(() => {
    const offline = error?.kind === 'offline' || error?.kind === 'unpaired'
    if (!offline) return undefined

    retryTimer.current = setTimeout(() => {
      retryDelay.current = Math.min(retryDelay.current * 2, RETRY_MAX_MS)
      void reconnect()
    }, retryDelay.current)

    return () => {
      if (retryTimer.current) clearTimeout(retryTimer.current)
    }
  }, [error, reconnect])

  return {
    status,
    dir,
    entries,
    loading,
    error,
    space,
    reconnecting,
    startupFailed,
    open,
    refresh,
    reconnect,
    setStatus,
  }
}
