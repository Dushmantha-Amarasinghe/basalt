/**
 * Native file dialogs.
 *
 * Loaded on demand and stubbed outside the desktop shell, for the same reason
 * `api.ts` mocks its calls: the interface has to stay openable in a plain
 * browser, and an import that only resolves inside Tauri would break that at
 * module load rather than at the moment of use.
 */

import { inTauri } from './api'

/** Where to save a file the user is downloading. */
export async function pickSaveLocation(
  defaultName: string,
): Promise<string | null> {
  if (!inTauri()) return null
  const { save } = await import('@tauri-apps/plugin-dialog')
  return (await save({ defaultPath: defaultName })) ?? null
}

/** A folder to download several files into. */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri()) return null
  const { open } = await import('@tauri-apps/plugin-dialog')
  const chosen = await open({ directory: true, multiple: false })
  return typeof chosen === 'string' ? chosen : null
}

/** Files to upload. */
export async function pickFiles(): Promise<string[]> {
  if (!inTauri()) return []
  const { open } = await import('@tauri-apps/plugin-dialog')
  const chosen = await open({ multiple: true })
  if (Array.isArray(chosen)) return chosen
  return typeof chosen === 'string' ? [chosen] : []
}

/**
 * Confirms something destructive.
 *
 * Throws rather than returning false when the dialog cannot be shown. Those are
 * different things and the difference matters: a failure returned as "no" would
 * make a button that quietly does nothing, which is exactly what happened when
 * `dialog:allow-confirm` was missing from the capability file — Forget this
 * vault, Pair with a different vault and Delete all became dead buttons with no
 * message anywhere. Whatever else it does, this must never answer "yes" on
 * failure.
 */
export async function confirmAction(
  message: string,
  title: string,
): Promise<boolean> {
  if (!inTauri()) return window.confirm(`${title}\n\n${message}`)
  const { confirm } = await import('@tauri-apps/plugin-dialog')
  try {
    return await confirm(message, { title, kind: 'warning' })
  } catch (e) {
    throw new Error(`Could not ask you to confirm this: ${e}`)
  }
}

/**
 * Files dragged into the window from Explorer.
 *
 * This cannot be done with HTML drag-and-drop: a webview is given a `File`
 * object with no path, and the upload needs a real path on disk to read from.
 * Tauri intercepts the drop at the window level and hands over the actual
 * paths, which is the only way this works at all.
 *
 * Returns an unsubscribe function; a no-op outside the desktop shell.
 */
export async function onExternalFileDrop(handlers: {
  onEnter: () => void
  /** Where the pointer is, in CSS pixels, as it moves over the window. */
  onOver: (x: number, y: number) => void
  onLeave: () => void
  onDrop: (paths: string[], x: number, y: number) => void
}): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { getCurrentWebview } = await import('@tauri-apps/api/webview')
  const webview = getCurrentWebview()

  // Tauri reports **physical** pixels; everything in the document is in CSS
  // pixels. On a scaled display — which is most laptops — skipping this
  // conversion aims the hit test at roughly half the intended position, so it
  // would work on exactly the machines nobody tests on.
  const toCss = (p?: { x: number; y: number }): [number, number] => {
    const ratio = window.devicePixelRatio || 1
    return p ? [p.x / ratio, p.y / ratio] : [-1, -1]
  }

  return webview.onDragDropEvent((event) => {
    const payload = event.payload as {
      type: string
      paths?: string[]
      position?: { x: number; y: number }
    }
    const [x, y] = toCss(payload.position)

    if (payload.type === 'enter') handlers.onEnter()
    else if (payload.type === 'over') {
      handlers.onEnter()
      handlers.onOver(x, y)
    } else if (payload.type === 'leave') handlers.onLeave()
    else if (payload.type === 'drop') handlers.onDrop(payload.paths ?? [], x, y)
  })
}

/**
 * The folder the pointer is over, or null for "wherever we are now".
 *
 * Read from the document rather than from React state because the position
 * arrives from the window, not from a React event — there is no hovered
 * element to consult, only a coordinate. Rows advertise themselves with
 * `data-drop-dir`, so this stays correct however the list is being drawn.
 */
export function folderUnder(x: number, y: number): string | null {
  if (typeof document === 'undefined' || x < 0 || y < 0) return null
  const element = document.elementFromPoint(x, y)
  const row = element?.closest('[data-drop-dir]')
  return row?.getAttribute('data-drop-dir') ?? null
}

/** The last segment of a local path, whichever separator it uses. */
export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return cut === -1 ? path : path.slice(cut + 1)
}

/** Joins a local path with a name, keeping the platform's separator. */
export function localJoin(dir: string, name: string): string {
  const sep = dir.includes('\\') ? '\\' : '/'
  return dir.endsWith(sep) ? `${dir}${name}` : `${dir}${sep}${name}`
}
