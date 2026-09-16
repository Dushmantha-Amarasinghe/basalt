import type { Entry } from '@/components/FileList'
import type { MediaItem } from './mockMedia'
import { parentOf } from './api'

/**
 * Turns a real file into something the media grid can draw.
 *
 * The grid was built around posters, and there are no thumbnails yet — the
 * host generating them is a phase of its own. Rather than leave every tile
 * blank, each file gets a pair of graphite tones derived from its name, so a
 * folder of films is visually varied and the same film always looks the same.
 *
 * Deliberately monochrome, like everything else: the tones are lightness
 * steps, never hues.
 */

/** A stable small hash of a string. FNV-1a, which is plenty for picking tones. */
function hash(text: string): number {
  let value = 0x811c9dc5
  for (let i = 0; i < text.length; i += 1) {
    value ^= text.charCodeAt(i)
    value = Math.imul(value, 0x01000193)
  }
  return value >>> 0
}

function tonePair(seed: number): [string, string] {
  const base = 18 + (seed % 26)
  const lift = 8 + ((seed >>> 8) % 16)
  const hex = (v: number): string => Math.min(255, v).toString(16).padStart(2, '0')
  return [
    `#${hex(base)}${hex(base)}${hex(base + 2)}`,
    `#${hex(base + lift)}${hex(base + lift)}${hex(base + lift + 2)}`,
  ]
}

function stripExtension(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(0, dot) : name
}

export function entryToMedia(entry: Entry): MediaItem {
  const folder = parentOf(entry.id)
  return {
    // The vault path, so opening a tile knows exactly which file it is.
    id: entry.id,
    title: stripExtension(entry.name),
    subtitle: folder || 'Vault',
    size: entry.size,
    tone: tonePair(hash(entry.id)),
  }
}

export function entriesToMedia(entries: Entry[]): MediaItem[] {
  return entries.map(entryToMedia)
}

/** Whether the built-in player is likely to be able to decode this. */
export function isPlayable(name: string): boolean {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''
  // WebView2 is Chromium, so this is Chromium's list. MKV, HEVC and AC3 are
  // the common gaps, and they need a real decoder rather than a better guess —
  // that is what the mpv sidecar is for, later.
  return ['mp4', 'm4v', 'webm', 'mp3', 'm4a', 'wav', 'ogg', 'opus', 'flac'].includes(
    ext,
  )
}
