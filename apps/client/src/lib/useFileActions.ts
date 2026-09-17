import { useCallback, useState } from 'react'
import type { Entry } from '@/components/FileList'
import { ApiError, api, joinPath, parentOf } from './api'
import { confirmAction } from './dialogs'

/**
 * Every operation the file browser can perform on the vault, in one place.
 *
 * Gathered here rather than spread across the context menu, the toolbar, the
 * keyboard handler and the drag target, because all four must behave
 * identically — and because each one has a rule that is easy to get wrong
 * separately: a name that would collide, a folder moved inside itself, a
 * delete that needs confirming.
 */

export type ClipboardMode = 'copy' | 'cut'

export interface Clipboard {
  mode: ClipboardMode
  /** Full vault paths. */
  paths: string[]
}

export interface FileActions {
  clipboard: Clipboard | null
  busy: boolean

  copy: (paths: string[]) => void
  cut: (paths: string[]) => void
  clearClipboard: () => void
  /** Whether pasting into this folder would do anything. */
  canPasteInto: (dir: string) => boolean
  paste: (dir: string) => Promise<void>

  rename: (path: string, name: string) => Promise<void>
  remove: (entries: Entry[]) => Promise<void>
  /** Moves entries into a folder. Used by drag-and-drop and by cut/paste. */
  moveInto: (paths: string[], dir: string) => Promise<void>
  newFolder: (dir: string, name: string) => Promise<void>
  duplicate: (path: string) => Promise<void>
}

/** The last segment of a vault path. */
export function nameOf(path: string): string {
  const cut = path.lastIndexOf('/')
  return cut === -1 ? path : path.slice(cut + 1)
}

/**
 * Whether moving or copying `path` into `dir` would put it inside itself.
 *
 * Without this check, dragging a folder onto one of its own children asks the
 * host to copy a tree into its own subtree — which does not terminate until
 * the drive is full. The host refuses it too; catching it here means the user
 * gets told rather than watching an operation fail halfway.
 */
export function wouldNest(path: string, dir: string): boolean {
  return dir === path || dir.startsWith(`${path}/`)
}

/** A name that does not collide with anything already in the folder. */
export function uniqueName(name: string, taken: Set<string>): string {
  if (!taken.has(name)) return name

  const dot = name.lastIndexOf('.')
  // A leading dot is a hidden file, not an extension, so `.gitignore` must
  // become `.gitignore (2)` rather than `. (2)gitignore`.
  const [stem, extension] =
    dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, '']

  for (let n = 2; n < 10_000; n += 1) {
    const candidate = `${stem} (${n})${extension}`
    if (!taken.has(candidate)) return candidate
  }
  // Nobody has 10,000 copies; this is here so the loop cannot fall through to
  // a name that collides.
  return `${stem} (${Date.now()})${extension}`
}

export function useFileActions({
  onChanged,
  onError,
}: {
  /** Called after anything that changes the drive, to refresh the listing. */
  onChanged: () => void
  onError: (message: string) => void
}): FileActions {
  const [clipboard, setClipboard] = useState<Clipboard | null>(null)
  const [busy, setBusy] = useState(false)

  const fail = useCallback(
    (e: unknown, what: string) => {
      const message =
        e instanceof ApiError || e instanceof Error ? e.message : String(e)
      onError(`${what}: ${message}`)
    },
    [onError],
  )

  /** The names already present in a folder, so collisions can be avoided. */
  const takenIn = useCallback(async (dir: string): Promise<Set<string>> => {
    try {
      return new Set((await api.list(dir)).map((e) => e.name))
    } catch {
      // A folder that will not list is a problem the operation itself will
      // report; assuming it is empty here just means no renaming happens.
      return new Set()
    }
  }, [])

  const copy = useCallback((paths: string[]) => {
    if (paths.length > 0) setClipboard({ mode: 'copy', paths })
  }, [])

  const cut = useCallback((paths: string[]) => {
    if (paths.length > 0) setClipboard({ mode: 'cut', paths })
  }, [])

  const clearClipboard = useCallback(() => setClipboard(null), [])

  const canPasteInto = useCallback(
    (dir: string): boolean => {
      if (!clipboard) return false
      return clipboard.paths.some((p) => !wouldNest(p, dir))
    },
    [clipboard],
  )

  const paste = useCallback(
    async (dir: string) => {
      if (!clipboard || busy) return
      setBusy(true)
      try {
        const taken = await takenIn(dir)
        for (const source of clipboard.paths) {
          if (wouldNest(source, dir)) {
            onError(`${nameOf(source)} cannot go inside itself`)
            continue
          }
          // A cut back into the same folder is a no-op, not an error.
          if (clipboard.mode === 'cut' && parentOf(source) === dir) continue

          const name = uniqueName(nameOf(source), taken)
          taken.add(name)
          const target = joinPath(dir, name)
          try {
            if (clipboard.mode === 'copy') await api.copy(source, target)
            else await api.rename(source, target)
          } catch (e) {
            fail(e, `Could not paste ${nameOf(source)}`)
          }
        }
        // A cut is spent once pasted; a copy can be pasted again elsewhere,
        // which is what every file manager does and what people expect.
        if (clipboard.mode === 'cut') setClipboard(null)
        onChanged()
      } finally {
        setBusy(false)
      }
    },
    [clipboard, busy, takenIn, onChanged, onError, fail],
  )

  const moveInto = useCallback(
    async (paths: string[], dir: string) => {
      if (busy) return
      setBusy(true)
      try {
        const taken = await takenIn(dir)
        for (const source of paths) {
          if (wouldNest(source, dir)) {
            onError(`${nameOf(source)} cannot go inside itself`)
            continue
          }
          if (parentOf(source) === dir) continue

          const name = uniqueName(nameOf(source), taken)
          taken.add(name)
          try {
            await api.rename(source, joinPath(dir, name))
          } catch (e) {
            fail(e, `Could not move ${nameOf(source)}`)
          }
        }
        onChanged()
      } finally {
        setBusy(false)
      }
    },
    [busy, takenIn, onChanged, onError, fail],
  )

  const rename = useCallback(
    async (path: string, name: string) => {
      const trimmed = name.trim()
      if (!trimmed || trimmed === nameOf(path)) return
      try {
        await api.rename(path, joinPath(parentOf(path), trimmed))
        onChanged()
      } catch (e) {
        fail(e, `Could not rename ${nameOf(path)}`)
      }
    },
    [onChanged, fail],
  )

  const remove = useCallback(
    async (entries: Entry[]) => {
      if (entries.length === 0 || busy) return

      const folders = entries.filter((e) => e.kind === 'dir').length
      const ok = await confirmAction(
        entries.length === 1
          ? `Delete ${entries[0]!.name}?${folders ? ' Everything inside it goes too.' : ''}`
          : `Delete ${entries.length} items?${folders ? ` ${folders} are folders, and everything inside them goes too.` : ''}`,
        'This cannot be undone',
      )
      if (!ok) return

      setBusy(true)
      try {
        for (const entry of entries) {
          try {
            await api.remove(entry.id, entry.kind === 'dir')
          } catch (e) {
            fail(e, `Could not delete ${entry.name}`)
          }
        }
        onChanged()
      } finally {
        setBusy(false)
      }
    },
    [busy, onChanged, fail],
  )

  const newFolder = useCallback(
    async (dir: string, name: string) => {
      const trimmed = name.trim()
      if (!trimmed) return
      try {
        const taken = await takenIn(dir)
        await api.mkdir(joinPath(dir, uniqueName(trimmed, taken)))
        onChanged()
      } catch (e) {
        fail(e, 'Could not create the folder')
      }
    },
    [takenIn, onChanged, fail],
  )

  const duplicate = useCallback(
    async (path: string) => {
      const dir = parentOf(path)
      try {
        const taken = await takenIn(dir)
        await api.copy(path, joinPath(dir, uniqueName(nameOf(path), taken)))
        onChanged()
      } catch (e) {
        fail(e, `Could not duplicate ${nameOf(path)}`)
      }
    },
    [takenIn, onChanged, fail],
  )

  return {
    clipboard,
    busy,
    copy,
    cut,
    clearClipboard,
    canPasteInto,
    paste,
    rename,
    remove,
    moveInto,
    newFolder,
    duplicate,
  }
}
