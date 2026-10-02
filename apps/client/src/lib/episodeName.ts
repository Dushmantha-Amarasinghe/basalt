/**
 * An episode's name, read off its file when the library has no title for it.
 *
 * The file is usually a release name: `Signal.House.S01E02.Low.Water.
 * 2160p.10bit.AMZN.WEB-DL.DDP5.1.HEVC-GROUP.mkv`. Shown whole, every row
 * of a season said the same long string with one number changed. What sits
 * between the episode number and the release tags is the episode's own name,
 * "Low Water", and that is what is shown; a file that gives none
 * is "Episode 2".
 */

/** Where a release name stops being a name and starts listing its source. */
const RELEASE_TAG =
  /^(2160p|1440p|1080p|720p|576p|480p|4k|uhd|8k|10bit|8bit|hdr|hdr10|dv|dovi|web|webdl|webrip|web-dl|bluray|blu-ray|brrip|bdrip|remux|hdtv|dvdrip|amzn|nf|dsnp|hmax|max|atvp|pcok|hulu|x264|x265|h264|h265|hevc|avc|av1|ddp\d?|dd\d?|aac\d?|ac3|eac3|dts|truehd|atmos|proper|repack|internal|multi)$/i

export function episodeName(path: string, number: number): string {
  const file = path.split('/').pop() ?? path
  const stem = file.replace(/\.[^.]+$/, '')
  const fallback = number > 0 ? `Episode ${number}` : stem

  const marker = /(?:^|[\s._-])(?:s\d{1,2}[\s._-]?e\d{1,3}(?:[\s._-]?e\d{1,3})*|\d{1,2}x\d{2,3})(?=$|[\s._-])/i.exec(stem)
  if (!marker) return fallback

  // Bracketed tags go first: `[1080p]`, `[GROUP]`.
  const after = stem.slice(marker.index + marker[0].length).replace(/\[[^\]]*\]/g, ' ')
  const words: string[] = []
  for (const word of after.split(/[\s._]+/).filter(Boolean)) {
    // A group name glued on with a dash, as in `HEVC-GROUP`, ends it too.
    const bare = word.split('-')[0] ?? word
    if (RELEASE_TAG.test(word) || RELEASE_TAG.test(bare) || /^\d+(\.\d)?$/.test(word)) break
    words.push(word)
  }
  const name = words.join(' ').replace(/^[-–—\s]+|[-–—\s]+$/g, '').trim()
  return name || fallback
}
