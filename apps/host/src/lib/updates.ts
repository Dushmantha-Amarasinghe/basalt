import { useSyncExternalStore } from 'react'
import { api, inTauri, type Release } from '@/lib/api'

/**
 * Whether there is a newer Basalt Host, shared by everything that shows it.
 *
 * One check, one download and one install, however many places offer the
 * update: the banner at the top of the window and Settings read the same
 * state, so starting a download in one shows its progress in the other, and
 * nothing asks GitHub twice.
 *
 * Checked when the app opens and then every twelve hours while it stays open,
 * which on a PC left running can be weeks. That is two small requests a day to
 * GitHub's release list, and nothing else: no file is downloaded until
 * someone presses Download, and nothing is installed until they press
 * Install.
 */

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'failed'; why: string }
  | { kind: 'available'; release: Release }
  | { kind: 'downloading'; release: Release; had: number; total: number }
  | { kind: 'ready'; release: Release; path: string }

const EVERY = 12 * 60 * 60 * 1000

let state: UpdateState = { kind: 'idle' }
let lastChecked = 0
const listeners = new Set<() => void>()

function set(next: UpdateState): void {
  state = next
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** The current state, for a component. */
export function useUpdate(): UpdateState {
  return useSyncExternalStore(subscribe, () => state)
}

/** Whether an update is on offer, in any of its stages. */
export function offered(s: UpdateState): s is Extract<UpdateState, { release: Release }> {
  return s.kind === 'available' || s.kind === 'downloading' || s.kind === 'ready'
}

/**
 * Asks GitHub. `quiet` is the automatic check: a failure there is not news,
 * and saying so on every launch would train people to ignore the one time it
 * matters. A download under way or finished is never replaced by a check.
 */
export async function checkForUpdate(quiet: boolean): Promise<void> {
  if (state.kind === 'downloading' || state.kind === 'ready' || state.kind === 'checking') return
  const before = state
  if (!quiet) set({ kind: 'checking' })
  try {
    const release = await api.checkUpdate()
    lastChecked = Date.now()
    set(release ? { kind: 'available', release } : { kind: 'current' })
  } catch (e) {
    set(quiet ? before : { kind: 'failed', why: String(e) })
  }
}

let started = false

/** Checks now, and again every twelve hours or on returning after as long. */
export function startUpdateChecks(): void {
  if (started) return
  started = true
  void checkForUpdate(true)
  setInterval(() => void checkForUpdate(true), EVERY)
  // The host spends most of its life hidden in the tray; coming back into
  // view after a long time away is as good a moment as a timer's.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && Date.now() - lastChecked > EVERY) {
      void checkForUpdate(true)
    }
  })
}

let listening: Promise<void> | null = null

/** Progress arrives from the shell as the bytes land. */
function listenForProgress(): Promise<void> {
  if (listening) return listening
  const onProgress = ([had, total]: [number, number]): void => {
    if (state.kind === 'downloading') set({ ...state, had, total })
  }
  listening = (async () => {
    // In a browser preview the same payload arrives as a window event, so the
    // bar can be reviewed without a release to download.
    if (!inTauri()) {
      window.addEventListener('basalt://update-progress', (event) =>
        onProgress((event as CustomEvent<[number, number]>).detail),
      )
      return
    }
    const { listen } = await import('@tauri-apps/api/event')
    await listen<[number, number]>('basalt://update-progress', (event) => onProgress(event.payload))
  })()
  return listening
}

/** Downloads the offered release, verified against its published checksum. */
export async function downloadUpdate(): Promise<void> {
  if (state.kind !== 'available') return
  const release = state.release
  await listenForProgress()
  set({ kind: 'downloading', release, had: 0, total: release.installerBytes })
  try {
    const path = await api.downloadUpdate(release)
    set({ kind: 'ready', release, path })
  } catch (e) {
    set({ kind: 'failed', why: String(e) })
  }
}

/** Runs the downloaded, verified installer; the host steps aside for it. */
export async function installUpdate(): Promise<void> {
  if (state.kind !== 'ready') return
  await api.installUpdate(state.path).catch(() => {})
}
