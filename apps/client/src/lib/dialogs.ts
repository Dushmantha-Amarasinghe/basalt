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

/** Confirms something destructive. */
export async function confirmAction(
  message: string,
  title: string,
): Promise<boolean> {
  if (!inTauri()) return window.confirm(`${title}\n\n${message}`)
  const { confirm } = await import('@tauri-apps/plugin-dialog')
  return confirm(message, { title, kind: 'warning' })
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
