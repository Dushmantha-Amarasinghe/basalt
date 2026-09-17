import { useEffect, useRef } from 'react'
import { onChange, type Change } from './api'
import { useAsyncSubscription } from './useAsyncSubscription'

/**
 * Decides whether a change affects the folder currently on screen.
 *
 * Pure, and exported, because this is the whole of the logic and it deserves a
 * test rather than a rendered component.
 *
 * `resynchronise` always matters: the host is saying it stopped counting, so
 * the only correct response is to reload whatever is showing. A library change
 * never affects a file listing.
 */
export function affects(change: Change, dir: string): boolean {
  const parent = (path: string): string => {
    const cut = path.lastIndexOf('/')
    return cut === -1 ? '' : path.slice(0, cut)
  }

  switch (change.kind) {
    case 'resynchronise':
      return true
    case 'library_changed':
      return false
    case 'renamed':
      // A move touches two folders, and someone looking at either is wrong.
      return parent(change.from) === dir || parent(change.to) === dir
    default:
      return parent(change.path) === dir
  }
}

/** How long to gather changes before reloading. */
const COALESCE_MS = 150

/**
 * Reloads the current folder when the drive changes underneath it.
 *
 * Coalesced: copying twenty files into the folder you are looking at produces
 * twenty changes and should produce one reload, not twenty. The host already
 * batches what it sees, but two batches can still arrive back to back.
 *
 * Everything is read through refs so that neither a changing directory nor an
 * inline callback tears the subscription down and sets it up again — a leak of
 * exactly that shape once turned a single dropped file into eight uploads.
 */
export function useLiveChanges(
  dir: string,
  onRefresh: () => void,
  onLibraryChanged?: () => void,
): void {
  const currentDir = useRef(dir)
  currentDir.current = dir

  const refresh = useRef(onRefresh)
  refresh.current = onRefresh

  const libraryChanged = useRef(onLibraryChanged)
  libraryChanged.current = onLibraryChanged

  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useAsyncSubscription(true, () =>
    onChange((change) => {
      if (change.kind === 'library_changed') {
        libraryChanged.current?.()
        return
      }
      if (!affects(change, currentDir.current)) return

      if (timer.current !== null) clearTimeout(timer.current)
      timer.current = setTimeout(() => {
        timer.current = null
        refresh.current()
      }, COALESCE_MS)
    }),
  )

  // A pending reload after unmount would call into a gone component.
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)
    },
    [],
  )
}
