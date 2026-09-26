import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type Collections, type MediaFile } from './api'

/**
 * Every video, song and photo on the drive, as the host sorted them.
 *
 * Replaces the scan each device used to run for itself: the top of the drive
 * and one folder down, once per connection. That missed anything deeper and
 * anything added after connecting — a photo uploaded to the root sat in Files
 * and never reached Photos. The host walks the whole drive and keeps the lists
 * current; this fetches them when it says they changed.
 *
 * A host from before collections existed cannot answer, and `unsupported`
 * says so, so the sections can fall back to the old scan rather than go
 * empty while the two ends are being updated.
 */
export interface CollectionsState {
  collections: Collections
  scanning: boolean
  loaded: boolean
  unsupported: boolean
  refresh: () => void
}

const EMPTY: Collections = { videos: [], music: [], photos: [], recent: [], truncated: false }

export function useCollections(connected: boolean): CollectionsState {
  const [collections, setCollections] = useState<Collections>(EMPTY)
  const [scanning, setScanning] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [unsupported, setUnsupported] = useState(false)
  const revision = useRef(0)
  const inFlight = useRef(false)
  const live = useRef(true)

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const load = useCallback(async () => {
    if (!connected || inFlight.current) return
    inFlight.current = true
    try {
      const response = await api.collections(revision.current)
      if (!live.current) return
      revision.current = response.revision
      setScanning(response.scanning)
      // Nothing back means "you already have them", not "there are none".
      if (response.collections) setCollections(withLists(response.collections))
      setUnsupported(false)
      setLoaded(true)
    } catch {
      if (!live.current) return
      // A host from before collections cannot answer: it refuses, or hangs
      // up. Either way the sections fall back to scanning for themselves,
      // and a later answer — once the host is updated — switches them back.
      setUnsupported(true)
      setLoaded(true)
    } finally {
      inFlight.current = false
    }
  }, [connected])

  useEffect(() => {
    if (!connected) {
      revision.current = 0
      setCollections(EMPTY)
      setLoaded(false)
      setScanning(false)
      return
    }
    void load()
  }, [connected, load])

  // While the host is walking, it has nothing to announce until it finishes.
  useEffect(() => {
    if (!scanning) return undefined
    const timer = setInterval(() => void load(), 2_000)
    return () => clearInterval(timer)
  }, [scanning, load])

  return {
    collections,
    scanning,
    loaded,
    unsupported,
    refresh: useCallback(() => void load(), [load]),
  }
}

/** A host that leaves an empty list out is answered as if it sent one. */
function withLists(c: Partial<Collections>): Collections {
  return {
    videos: c.videos ?? [],
    music: c.music ?? [],
    photos: c.photos ?? [],
    recent: c.recent ?? [],
    truncated: c.truncated ?? false,
  }
}

/** A collection file as a listing entry, for views built on entries. */
export function fileToEntry(file: MediaFile): {
  id: string
  name: string
  kind: 'file'
  size: number
  modified: number
} {
  return {
    id: file.path,
    name: file.path.split('/').pop() ?? file.path,
    kind: 'file',
    size: file.size,
    modified: file.mtime * 1000,
  }
}
