import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type LibraryItem, type LibraryResponse } from './api'

export interface MediaLibrary {
  /** False when the host has the feature switched off. */
  enabled: boolean
  scanning: boolean
  films: LibraryItem[]
  series: LibraryItem[]
  loading: boolean
  error: string | null
  refresh: () => void
}

/**
 * The host's index of films and series.
 *
 * Fetched once and then only when the host says it changed, which is what the
 * revision is for: the answer to "anything new?" is a few bytes when there is
 * not. No polling — the watch already tells this client when to ask.
 */
export function useMediaLibrary(connected: boolean): MediaLibrary {
  const [state, setState] = useState<{
    enabled: boolean
    scanning: boolean
    items: LibraryItem[]
  }>({ enabled: false, scanning: false, items: [] })
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const revision = useRef(0)
  const live = useRef(true)
  const inFlight = useRef(false)

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const load = useCallback(async () => {
    if (!connected || inFlight.current) return
    inFlight.current = true
    setLoading(true)
    try {
      const response: LibraryResponse = await api.library(revision.current)
      if (!live.current) return
      revision.current = response.revision
      setState((previous) => ({
        enabled: response.enabled,
        scanning: response.scanning,
        // No items means "you already have them", not "there are none".
        items: response.items ?? previous.items,
      }))
      setError(null)
    } catch (e) {
      if (live.current) setError(e instanceof Error ? e.message : String(e))
    } finally {
      inFlight.current = false
      if (live.current) setLoading(false)
    }
  }, [connected])

  useEffect(() => {
    if (!connected) {
      // A fresh connection may be a different host with a different library,
      // so the revision cannot carry over.
      revision.current = 0
      setState({ enabled: false, scanning: false, items: [] })
      return
    }
    void load()
  }, [connected, load])

  // A scan in progress is the one time polling is right: the host has nothing
  // to announce until it finishes, and the screen is saying "scanning".
  useEffect(() => {
    if (!state.scanning) return
    const timer = setInterval(() => void load(), 1_500)
    return () => clearInterval(timer)
  }, [state.scanning, load])

  return {
    enabled: state.enabled,
    scanning: state.scanning,
    films: state.items.filter((item) => item.kind === 'film'),
    series: state.items.filter((item) => item.kind === 'series'),
    loading,
    error,
    refresh: useCallback(() => void load(), [load]),
  }
}
