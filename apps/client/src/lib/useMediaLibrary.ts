import { useCallback, useEffect, useRef, useState } from 'react'
import { ALL_SECTIONS, api, type LibraryItem, type LibraryResponse, type Sections } from './api'

export interface MediaLibrary {
  /** False when the host has the feature switched off. */
  enabled: boolean
  scanning: boolean
  films: LibraryItem[]
  series: LibraryItem[]
  loading: boolean
  error: string | null
  /** Which sections the host's owner wants shown. */
  sections: Sections
  refresh: () => void
}

/**
 * Fills in the list fields a host leaves out when they are empty.
 *
 * `seasons` is declared as always present and read as such — every card on
 * the Movies screen runs `item.seasons.reduce(...)` to count episodes. A host
 * that omits the field for a film, because serde was told to skip an empty
 * list, therefore does not cost one card: it throws a TypeError out of render
 * and takes the whole page down with it. That is exactly what happened —
 * opening Movies showed a black window.
 *
 * Fixed on the host too, so the field is always sent. Normalised here as well
 * because a client that blanks its window when a host words a reply slightly
 * differently is a client with a bug, whatever the host does.
 */
export function withEmptyLists(items: LibraryItem[]): LibraryItem[] {
  return items.map((item) => ({
    ...item,
    seasons: (item.seasons ?? []).map((season) => ({
      ...season,
      episodes: season.episodes ?? [],
    })),
    subtitles: item.subtitles ?? [],
  }))
}

/**
 * The host's index of films and series.
 *
 * Fetched once and then only when the host says it changed, which is what the
 * revision is for: the answer to "anything new?" is a few bytes when there is
 * not. No polling — the watch already tells this client when to ask.
 *
 * `host` is the id of the host connected to, or null. Everything here belongs
 * to one host: a different one starts from nothing. It used to be keyed on
 * being connected at all, and changing drive goes from one host to another
 * without ever disconnecting — so the old drive's films stayed on screen, and
 * the old revision was sent to the new host, which could answer "nothing
 * changed" and leave them there.
 */
export function useMediaLibrary(host: string | null): MediaLibrary {
  const [state, setState] = useState<{
    enabled: boolean
    scanning: boolean
    items: LibraryItem[]
    sections: Sections
  }>({ enabled: false, scanning: false, items: [], sections: ALL_SECTIONS })
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const revision = useRef(0)
  const live = useRef(true)
  /** The host a request is out to, so a second one is not sent alongside. */
  const inFlight = useRef<string | null>(null)
  /** The host connected to now, for discarding a late answer from another. */
  const current = useRef(host)
  current.current = host

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const load = useCallback(async () => {
    const asked = current.current
    if (!asked || inFlight.current === asked) return
    inFlight.current = asked
    setLoading(true)
    try {
      const response: LibraryResponse = await api.library(revision.current)
      // Answered by the host before a drive change: not this library.
      if (!live.current || current.current !== asked) return
      revision.current = response.revision
      setState((previous) => ({
        enabled: response.enabled,
        scanning: response.scanning,
        // No items means "you already have them", not "there are none".
        items: response.items ? withEmptyLists(response.items) : previous.items,
        // Always sent, changed or not: a host that only changed which
        // sections to show has no new items to send.
        sections: { ...ALL_SECTIONS, ...(response.sections ?? {}) },
      }))
      setError(null)
    } catch (e) {
      if (live.current && current.current === asked) {
        setError(e instanceof Error ? e.message : String(e))
      }
    } finally {
      if (inFlight.current === asked) inFlight.current = null
      if (live.current && current.current === asked) setLoading(false)
    }
  }, [])

  useEffect(() => {
    // Another host, or none: nothing of the last library carries over, the
    // revision least of all.
    revision.current = 0
    inFlight.current = null
    setState({ enabled: false, scanning: false, items: [], sections: ALL_SECTIONS })
    setError(null)
    setLoading(false)
    if (host) void load()
  }, [host, load])

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
    sections: state.sections,
    refresh: useCallback(() => void load(), [load]),
  }
}
