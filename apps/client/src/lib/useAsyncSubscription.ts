import { useEffect, useRef } from 'react'

/**
 * Subscribing to something whose registration is asynchronous.
 *
 * Tauri's `listen` returns a promise for the unsubscribe function, and the
 * obvious way to use it is wrong in a way that is invisible until it is not:
 *
 * ```ts
 * useEffect(() => {
 *   let stop
 *   void listen(...).then((fn) => { stop = fn })
 *   return () => stop?.()          // stop is still undefined
 * }, [handler])                    // handler is new every render
 * ```
 *
 * Two faults compound. The cleanup runs synchronously while `stop` is still
 * `undefined`, so it removes nothing; and a dependency that changes identity
 * every render makes the effect re-run every render. Together they add one
 * permanent listener per render.
 *
 * What that looked like: dragging a file into the window started **eight
 * simultaneous uploads of the same file**, one for each listener that had piled
 * up since the app opened.
 *
 * This hook fixes both. It remembers whether cleanup already happened and
 * unsubscribes the moment the registration resolves, and it takes the handler
 * through a ref so the subscription is established exactly once.
 */

/** A ref that always holds the newest value, without causing a re-render. */
export function useLatest<T>(value: T): React.RefObject<T> {
  const ref = useRef(value)
  ref.current = value
  return ref
}

/**
 * Registers an asynchronous subscription once, and tears it down correctly.
 *
 * `subscribe` must be stable — wrap it in `useCallback` with no dependencies
 * and have it read anything changeable out of a [`useLatest`] ref. That is what
 * guarantees one listener rather than one per render.
 */
export function useAsyncSubscription(
  enabled: boolean,
  subscribe: () => Promise<() => void>,
): void {
  const latest = useLatest(subscribe)

  useEffect(() => {
    if (!enabled) return undefined

    let stop: (() => void) | undefined
    let cancelled = false

    void latest.current().then((fn) => {
      // The effect was torn down while registration was still in flight.
      // Undoing it here is the whole point: otherwise this listener lives
      // forever and every future event is handled one extra time.
      if (cancelled) {
        fn()
        return
      }
      stop = fn
    })

    return () => {
      cancelled = true
      // Cleared as it is called, so unsubscribing twice is harmless. React
      // will not do that, but an unlisten called twice is not something to
      // find out about the hard way.
      const fn = stop
      stop = undefined
      fn?.()
    }
  }, [enabled, latest])
}
