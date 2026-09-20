import type { Entry } from '@/components/FileList'
import type { MediaItem } from './mockMedia'
import { parentOf, type LibraryItem } from './api'

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

export { isMediaFile } from './playback'

/**
 * What to play after each episode, never leaving the series it belongs to.
 *
 * Flat *within* one series, so the episode after the last of season one is
 * the first of season two — a season boundary is exactly where autoplay earns
 * its keep. Across series, nothing: the episode after a finale is not the
 * pilot of whatever happens to sort next.
 *
 * That was the bug. The order was one flat list over every series, so ten
 * seconds into Alice in Borderland the player moved itself to Alien Earth.
 */
export function nextEpisodes(
  series: LibraryItem[],
): Map<string, { path: string; label: string }> {
  const next = new Map<string, { path: string; label: string }>()

  for (const show of series) {
    const run = show.seasons.flatMap((season) =>
      season.episodes.map((episode) => ({
        path: episode.path,
        label: `${show.title} · S${String(season.number).padStart(2, '0')}E${String(
          episode.number,
        ).padStart(2, '0')}`,
      })),
    )
    for (let i = 0; i < run.length - 1; i++) {
      next.set(run[i]!.path, run[i + 1]!)
    }
  }
  return next
}
