import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}

/**
 * Human-readable byte count, binary units.
 *
 * Tuned for a file list: enough precision to compare neighbouring rows, never
 * so much that the column becomes noisy.
 */
export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  if (unit === 0) return `${bytes} B`
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

/**
 * Bytes per second, as a speed.
 *
 * Decimal units, unlike `formatBytes`: link speeds are quoted in megabits and
 * megabytes per second everywhere else, and showing 21.8 MB/s where a transfer
 * window shows 22.9 MiB/s for the same link invites exactly the "why do these
 * disagree" that the throughput work was about.
 */
export function formatRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond < 1) return '—'
  const units = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  let value = bytesPerSecond
  let unit = 0
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000
    unit += 1
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

/**
 * How long ago, from a Unix **seconds** timestamp.
 *
 * Seconds because that is what the host speaks. Passing one of these to
 * `formatDate`, which expects milliseconds, dates everything to January 1970 —
 * a mistake already made once in this project, in the client's file list.
 */
export function formatAgo(unixSeconds: number): string {
  if (!unixSeconds) return 'never'
  return formatDate(unixSeconds * 1000)
}

/** A six-digit PIN, split so it can be read aloud. */
export function groupPin(pin: string): string {
  return pin.length === 6 ? `${pin.slice(0, 3)} ${pin.slice(3)}` : pin
}

/** A countdown in `m:ss`, or `0:00` once it has run out. */
export function formatCountdown(seconds: number): string {
  const clamped = Math.max(0, Math.floor(seconds))
  const minutes = Math.floor(clamped / 60)
  return `${minutes}:${String(clamped % 60).padStart(2, '0')}`
}

/** Relative date for recent items, absolute for older ones. */
export function formatDate(timestamp: number): string {
  const date = new Date(timestamp)
  const now = Date.now()
  const elapsed = now - timestamp

  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour

  if (elapsed < minute) return 'just now'
  if (elapsed < hour) return `${Math.floor(elapsed / minute)}m ago`
  if (elapsed < day) return `${Math.floor(elapsed / hour)}h ago`
  if (elapsed < 7 * day) return `${Math.floor(elapsed / day)}d ago`

  return date.toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}
