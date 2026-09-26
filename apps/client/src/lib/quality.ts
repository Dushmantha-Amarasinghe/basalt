import type { LibraryItem, Resolution } from './api'

/** The tags, smallest first. */
export type Quality = 'SD' | 'HD' | 'FHD' | '2K' | '4K' | '8K'

const ORDER: Quality[] = ['SD', 'HD', 'FHD', '2K', '4K', '8K']

/**
 * The tag for a picture size.
 *
 * By width or height, whichever says more. A film cropped to cinema shape is
 * 1920×800 — short, but still a 1080p release — and a 4K one 3840×1600, so
 * height alone would call both a size down. The thresholds sit a little under
 * each standard size, which catches the usual crops and odd encodes.
 */
export function qualityOf(size: Resolution | null | undefined): Quality | null {
  if (!size || size.width <= 0 || size.height <= 0) return null
  const { width: w, height: h } = size
  if (w >= 6000 || h >= 3800) return '8K'
  if (w >= 3200 || h >= 2000) return '4K'
  if (w >= 2300 || h >= 1300) return '2K'
  if (w >= 1800 || h >= 1000) return 'FHD'
  if (w >= 1200 || h >= 700) return 'HD'
  return 'SD'
}

/**
 * The tag for a card: the film's own, or what most of a series' episodes
 * are — so one stray 720p episode does not relabel a 4K series. A tie goes
 * to the better picture.
 */
export function itemQuality(item: LibraryItem): Quality | null {
  if (item.kind === 'film') return qualityOf(item.resolution)
  const counts = new Map<Quality, number>()
  for (const season of item.seasons) {
    for (const episode of season.episodes) {
      const q = qualityOf(episode.resolution)
      if (q) counts.set(q, (counts.get(q) ?? 0) + 1)
    }
  }
  let best: Quality | null = null
  let most = 0
  for (const [q, n] of counts) {
    if (n > most || (n === most && best && ORDER.indexOf(q) > ORDER.indexOf(best))) {
      best = q
      most = n
    }
  }
  return best
}

/** Whether a tag is worth drawing brighter: the ones people look for. */
export function isHighQuality(q: Quality | null): boolean {
  return q === '4K' || q === '8K'
}
