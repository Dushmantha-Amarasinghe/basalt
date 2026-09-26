import { useCallback, useEffect, useState } from 'react'
import type { Entry } from '@/components/FileList'
import { api } from './api'

/**
 * Starred files, kept on this device.
 *
 * Deliberately local rather than stored on the host. A star is a note about how
 * *you* use the drive, not a property of the file — two people sharing a vault
 * should not be rearranging each other's favourites, and nothing on the drive
 * should have to change because someone clicked a star.
 *
 * Keyed by host id, so pairing with a second vault does not show the first
 * one's stars against paths that may not exist there.
 */

const KEY_PREFIX = 'basalt:stars:'

/** What is remembered, so the list can be drawn before anything is fetched. */
interface StarRecord {
  path: string
  name: string
  kind: 'dir' | 'file'
}

function keyFor(hostId: string | null | undefined): string | null {
  return hostId ? `${KEY_PREFIX}${hostId}` : null
}

function load(hostId: string | null | undefined): StarRecord[] {
  const key = keyFor(hostId)
  if (!key) return []
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (r): r is StarRecord =>
        typeof r === 'object' &&
        r !== null &&
        typeof (r as StarRecord).path === 'string' &&
        typeof (r as StarRecord).name === 'string',
    )
  } catch {
    // Corrupt storage is not worth failing over; the worst case is a lost
    // list of favourites.
    return []
  }
}

function save(hostId: string | null | undefined, records: StarRecord[]): void {
  const key = keyFor(hostId)
  if (!key) return
  try {
    window.localStorage.setItem(key, JSON.stringify(records))
  } catch {
    // Storage full or disabled. Stars stop persisting; nothing else breaks.
  }
}

export interface Stars {
  /** Paths, for testing membership while drawing a list. */
  paths: Set<string>
  /** The starred items, with fresh metadata once it has been fetched. */
  entries: Entry[]
  loading: boolean
  isStarred: (path: string) => boolean
  toggle: (entries: Entry[]) => void
  /** Re-reads size and date from the host, dropping anything since deleted. */
  refresh: () => Promise<void>
}

/**
 * Stars are kept on the host for a profile, so they follow the person from
 * device to device, and on the device for a device on its own, as before.
 * `profileId` says which.
 */
export function useStars(
  hostId: string | null | undefined,
  active: boolean,
  profileId: string | null = null,
): Stars {
  const [records, setRecords] = useState<StarRecord[]>(() => (profileId ? [] : load(hostId)))
  const [entries, setEntries] = useState<Entry[]>([])
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    setEntries([])
    if (!profileId) {
      setRecords(load(hostId))
      return undefined
    }
    setRecords([])
    let cancelled = false
    void api
      .profileStars()
      .then((stars) => {
        if (!cancelled) setRecords(stars)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [hostId, profileId])

  /** Keeps a changed list wherever this identity keeps it. */
  const persist = useCallback(
    (next: StarRecord[]) => {
      if (profileId) {
        void api
          .profileStars(next.map((r) => ({ path: r.path, name: r.name, kind: r.kind })))
          .catch(() => {})
      } else {
        save(hostId, next)
      }
    },
    [hostId, profileId],
  )

  const paths = new Set(records.map((r) => r.path))

  const toggle = useCallback(
    (chosen: Entry[]) => {
      setRecords((prev) => {
        const known = new Set(prev.map((r) => r.path))
        // If any of the chosen items is not starred, the whole group becomes
        // starred; otherwise the whole group is unstarred. Toggling each one
        // individually would leave a mixed selection half-starred, which is
        // never what anyone means.
        const adding = chosen.some((e) => !known.has(e.id))

        const next = adding
          ? [
              ...prev,
              ...chosen
                .filter((e) => !known.has(e.id))
                .map((e) => ({ path: e.id, name: e.name, kind: e.kind })),
            ]
          : prev.filter((r) => !chosen.some((e) => e.id === r.path))

        persist(next)
        return next
      })
      // The starred view is rebuilt from the host next time it opens.
      setEntries([])
    },
    [persist],
  )

  const refresh = useCallback(async () => {
    if (records.length === 0) {
      setEntries([])
      return
    }
    setLoading(true)
    try {
      const found: Entry[] = []
      const alive: StarRecord[] = []
      for (const record of records) {
        try {
          const fresh = await api.stat(record.path)
          alive.push(record)
          found.push({
            id: record.path,
            name: fresh.name,
            kind: fresh.kind,
            size: fresh.size,
            modified: fresh.mtime * 1000,
          })
        } catch {
          // A star pointing at something deleted is dropped rather than shown
          // as a row that fails whenever it is touched.
        }
      }
      if (alive.length !== records.length) {
        setRecords(alive)
        persist(alive)
      }
      setEntries(found)
    } finally {
      setLoading(false)
    }
  }, [records, persist])

  // Fetched when the section is opened, not before: nobody should pay for a
  // round trip per star while browsing files.
  useEffect(() => {
    if (!active || entries.length > 0 || records.length === 0) return
    void refresh()
  }, [active, entries.length, records.length, refresh])

  return {
    paths,
    entries,
    loading,
    isStarred: (path: string) => paths.has(path),
    toggle,
    refresh,
  }
}

/** Exported for tests: what a toggle does to a set of records. */
export function nextStars(
  current: { path: string }[],
  chosen: { id: string; name: string; kind: 'dir' | 'file' }[],
): { path: string }[] {
  const known = new Set(current.map((r) => r.path))
  const adding = chosen.some((e) => !known.has(e.id))
  return adding
    ? [
        ...current,
        ...chosen
          .filter((e) => !known.has(e.id))
          .map((e) => ({ path: e.id, name: e.name, kind: e.kind })),
      ]
    : current.filter((r) => !chosen.some((e) => e.id === r.path))
}
