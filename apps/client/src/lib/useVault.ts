import { useCallback, useEffect, useRef, useState } from 'react'
import type { Entry } from '@/components/FileList'
import { ApiError, api, onStatus, toEntries, type Status } from './api'

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

  // First load: ask where we stand, then list the root if connected.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const initial = await api.status()
        if (cancelled) return
        setStatus(initial)
        if (initial.connected) {
          void load('')
          api.space().then(setSpace).catch(() => {})
        }
      } catch (e) {
        if (!cancelled) setError(new ApiError('error', String(e)))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [load])

  // The backend reconnects in the background at startup and pushes the result,
  // so the window can open immediately instead of waiting on the network.
  useEffect(() => {
    let stop: (() => void) | undefined
    void onStatus((next) => {
      setStatus(next)
      if (next.connected) {
        void load(wanted.current)
        api.space().then(setSpace).catch(() => {})
      }
    }).then((fn) => {
      stop = fn
    })
    return () => stop?.()
  }, [load])

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
    open,
    refresh,
    reconnect,
    setStatus,
  }
}
