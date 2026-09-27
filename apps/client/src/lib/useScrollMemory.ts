import { useCallback, useLayoutEffect, useRef } from 'react'
import type { VListHandle } from 'virtua'

/** Where each list was left, for the life of the app. */
const remembered = new Map<string, number>()
/** Enough to walk back up a deep folder tree; not every folder ever opened. */
const KEEP = 200

/**
 * The key for a file list: the section, the folder its entries came from,
 * and the search. `|` is the separator because no Windows file name can
 * contain one.
 */
export function scrollKeyOf(section: string, folder: string, search: string): string {
  return `${section}|${folder}|${search}`
}

function parse(key: string): { section: string; folder: string; search: string } {
  const first = key.indexOf('|')
  const second = key.indexOf('|', first + 1)
  return {
    section: key.slice(0, first),
    folder: key.slice(first + 1, second),
    search: key.slice(second + 1),
  }
}

/**
 * Whether showing `next` after `prev` is coming back to it, and so should be
 * where it was left, rather than going somewhere new, which starts at the
 * top.
 *
 * Back up the tree is coming back; down into a folder is somewhere new, even
 * a folder visited before, as in Explorer. A new search is new; clearing it
 * is coming back. Another section, or the list appearing afresh — the tab
 * switched back to, the view changed — is coming back to where that was.
 */
export function isReturn(prev: string | null, next: string): boolean {
  if (prev === null) return true
  const a = parse(prev)
  const b = parse(next)
  if (a.section !== b.section) return true
  if (a.folder !== b.folder) {
    return b.folder === '' ? a.folder !== '' : a.folder.startsWith(`${b.folder}/`)
  }
  return b.search === ''
}

/**
 * Where a file list is scrolled to.
 *
 * The list is one component whichever folder it shows, so it kept its scroll
 * offset from one folder to the next: scrolled halfway down the drive, a
 * folder opened halfway down too, as if the scroll had carried over. Now a
 * folder opened starts at the top, a folder gone back up to is where it was
 * left, and the same folder changing under the list — a refresh, a file
 * arriving — does not move it. See `isReturn` for the rest.
 *
 * `key` (from `scrollKeyOf`) has to change when the new folder's entries
 * arrive, not when the folder is asked for — until then the old folder is
 * still what is on screen.
 *
 * Give the list `ref` and `onScroll`. The ref is a callback, because some
 * lists only appear once they know their width, after the folder is already
 * showing; one that appears late is put where it belongs too. `handle` is the
 * list, for anything else that needs to ask where it is.
 */
export function useScrollMemory(key: string): {
  ref: (list: VListHandle | null) => void
  onScroll: (offset: number) => void
  handle: React.RefObject<VListHandle | null>
} {
  const handle = useRef<VListHandle | null>(null)
  const offset = useRef(0)
  const shown = useRef<string | null>(null)

  const ref = useCallback((list: VListHandle | null) => {
    handle.current = list
    if (list && offset.current > 0) list.scrollTo(offset.current)
  }, [])

  useLayoutEffect(() => {
    const target = isReturn(shown.current, key) ? (remembered.get(key) ?? 0) : 0
    shown.current = key
    offset.current = target
    handle.current?.scrollTo(target)
    return () => {
      remembered.delete(key)
      remembered.set(key, offset.current)
      if (remembered.size > KEEP) {
        const oldest = remembered.keys().next().value
        if (oldest !== undefined) remembered.delete(oldest)
      }
    }
  }, [key])

  const onScroll = useCallback((at: number) => {
    offset.current = at
  }, [])

  return { ref, onScroll, handle }
}
