import { useCallback, useMemo, useRef, useState } from 'react'
import { api, onBytes, onTransfer, type TransferEvent } from './api'
import { recordWindow } from './throughput'
import { useAsyncSubscription } from './useAsyncSubscription'

/**
 * The transfer queue, fed by events from the backend.
 *
 * Progress arrives as events rather than being polled because the Rust side
 * already knows exactly when a chunk lands, and asking it every 100 ms would
 * be both slower to update and more work. The backend throttles to about eight
 * events a second, which is a sensible rate for a progress bar and well under
 * anything React would struggle with.
 */

export interface Transfer {
  id: string
  kind: 'download' | 'upload'
  name: string
  path: string
  transferred: number
  total: number
  status: 'active' | 'done' | 'failed' | 'cancelled'
  /** Bytes per second. */
  rate: number
  error?: string
}

/** Completed transfers kept on screen before they are dropped. */
const KEEP_DONE = 12

export interface Transfers {
  transfers: Transfer[]
  active: Transfer[]
  /** Combined rate of everything in flight, in MB/s. */
  totalRate: number
  start: (transfer: Omit<Transfer, 'transferred' | 'rate' | 'status'>) => void
  finish: (id: string, error?: string) => void
  cancel: (id: string) => void
  clearDone: () => void
}

export function useTransfers(): Transfers {
  const [transfers, setTransfers] = useState<Transfer[]>([])
  const seen = useRef(new Map<string, number>())

  // The throughput trace is fed from the backend's own byte counter, which
  // sees everything on the link including streamed video. Transfer events are
  // only used to draw the queue.
  useAsyncSubscription(
    true,
    useCallback(() => onBytes((w) => recordWindow(w.bytes, w.millis)), []),
  )

  useAsyncSubscription(
    true,
    useCallback(
      () =>
        onTransfer((event: TransferEvent) => {
          if (event.status === 'done') seen.current.delete(event.id)
          else seen.current.set(event.id, event.transferred)

          setTransfers((prev) => {
            const index = prev.findIndex((t) => t.id === event.id)
            const next: Transfer = {
              id: event.id,
              kind: event.kind,
              name: event.name,
              path: event.path,
              transferred: event.transferred,
              total: event.total,
              status: event.status === 'done' ? 'done' : 'active',
              rate: event.rate,
            }
            if (index === -1) return [next, ...prev]
            // Replace in place so the row does not jump to the top on every
            // tick.
            const copy = [...prev]
            copy[index] = { ...copy[index], ...next }
            return copy
          })
        }),
      [],
    ),
  )

  const start = useCallback(
    (transfer: Omit<Transfer, 'transferred' | 'rate' | 'status'>) => {
      setTransfers((prev) => [
        { ...transfer, transferred: 0, rate: 0, status: 'active' },
        ...prev,
      ])
    },
    [],
  )

  const finish = useCallback((id: string, error?: string) => {
    seen.current.delete(id)
    setTransfers((prev) => {
      const updated = prev.map((t) =>
        t.id === id
          ? {
              ...t,
              status: error ? ('failed' as const) : ('done' as const),
              error,
              rate: 0,
              transferred: error ? t.transferred : t.total,
            }
          : t,
      )
      // Old completed rows are dropped rather than accumulating forever; the
      // queue is a view of what is happening, not a log.
      const finished = updated.filter((t) => t.status !== 'active')
      if (finished.length <= KEEP_DONE) return updated

      const keep = new Set(finished.slice(0, KEEP_DONE).map((t) => t.id))
      return updated.filter((t) => t.status === 'active' || keep.has(t.id))
    })
  }, [])

  const cancel = useCallback((id: string) => {
    void api.cancelTransfer(id)
    seen.current.delete(id)
    setTransfers((prev) =>
      prev.map((t) => (t.id === id ? { ...t, status: 'cancelled', rate: 0 } : t)),
    )
  }, [])

  const clearDone = useCallback(() => {
    setTransfers((prev) => prev.filter((t) => t.status === 'active'))
  }, [])

  // Memoised so this object keeps its identity between renders. A fresh
  // object each time made every `useCallback` that depended on it fresh too,
  // which is how an effect meant to run once ended up running on every render.
  return useMemo(() => {
    const active = transfers.filter((t) => t.status === 'active')
    return {
      transfers,
      active,
      totalRate: active.reduce((sum, t) => sum + t.rate, 0) / 1e6,
      start,
      finish,
      cancel,
      clearDone,
    }
  }, [transfers, start, finish, cancel, clearDone])
}

/** A short, unique id for one transfer. */
export function transferId(): string {
  return `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`
}
