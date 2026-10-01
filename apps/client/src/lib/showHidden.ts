import { useSyncExternalStore } from 'react'

/**
 * Whether lists include Windows' hidden and system items — `desktop.ini`,
 * `Thumbs.db`, the Recycle Bin. Off by default, as in Explorer.
 *
 * Kept on this device, and shared by everything that reads it, so the switch
 * in Settings and the folder on screen can never disagree.
 */
const KEY = 'basalt:show-hidden'
const listeners = new Set<() => void>()

function read(): boolean {
  try {
    return window.localStorage.getItem(KEY) === '1'
  } catch {
    return false
  }
}

let current = typeof window === 'undefined' ? false : read()

export function setShowHidden(on: boolean): void {
  current = on
  try {
    window.localStorage.setItem(KEY, on ? '1' : '0')
  } catch {
    // Remembered for this session, if not beyond it.
  }
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useShowHidden(): boolean {
  return useSyncExternalStore(subscribe, () => current, () => false)
}

/** What a list shows: everything, or everything Explorer would. */
export function visibleEntries<T extends { hidden?: boolean }>(entries: T[], showHidden: boolean): T[] {
  return showHidden ? entries : entries.filter((e) => !e.hidden)
}
