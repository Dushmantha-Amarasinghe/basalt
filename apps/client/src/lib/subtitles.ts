import type { LibraryItem, SubtitleTrack } from './api'

/**
 * Whether anything on the drive will subtitle this.
 *
 * Only counts the files the host found beside the video. Tracks *inside* the
 * file are not counted and deliberately so: the host would have to demux
 * every video on the drive to know, and it would be guessing at something the
 * player reads exactly when it opens the file. So a badge here means "there
 * is a subtitle file for this", and the player may well offer more once it
 * has the video open.
 */
export function subtitleCount(item: LibraryItem): number {
  if (item.kind === 'film') return item.subtitles?.length ?? 0
  // A series is subtitled if any of its episodes is. Counting every episode's
  // would read as a number of languages, which it is not.
  return item.seasons.reduce(
    (total, season) =>
      total + season.episodes.filter((e) => (e.subtitles?.length ?? 0) > 0).length,
    0,
  )
}

/** How many of a series' episodes have a subtitle file. */
export function episodesWithSubtitles(item: LibraryItem): [number, number] {
  const all = item.seasons.flatMap((s) => s.episodes)
  return [all.filter((e) => (e.subtitles?.length ?? 0) > 0).length, all.length]
}

/**
 * The badge for a card, or nothing when there is nothing to say.
 *
 * An empty badge on every card is noise, which is the same reasoning as the
 * progress bar that only appears part-way through something.
 */
export function subtitleBadge(item: LibraryItem): string | null {
  if (item.kind === 'film') {
    const count = item.subtitles?.length ?? 0
    if (count === 0) return null
    return count === 1 ? 'SUB' : `SUB ${count}`
  }

  const [withSubs, total] = episodesWithSubtitles(item)
  if (withSubs === 0) return null
  return withSubs === total ? 'SUB' : `SUB ${withSubs}/${total}`
}

/** What to say about a subtitle list on hover. */
export function subtitleTitle(tracks: SubtitleTrack[] | undefined): string {
  if (!tracks || tracks.length === 0) return ''
  return `Subtitles: ${tracks.map((t) => t.label).join(', ')}`
}
