import type { MediaFile } from './api'

/** The name without folders or extension. */
export function stemOf(path: string): string {
  const name = path.split('/').pop() ?? path
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(0, dot) : name
}

/** The extension, upper-case, for a small format badge. */
export function formatOf(path: string): string {
  const dot = path.lastIndexOf('.')
  return dot > 0 ? path.slice(dot + 1).toUpperCase() : ''
}

/** The folder a file is in, or empty at the top of the drive. */
export function folderOf(path: string): string {
  const at = path.lastIndexOf('/')
  return at > 0 ? path.slice(0, at) : ''
}

export interface MonthGroup {
  /** `2026-09`, for keys. */
  key: string
  /** `September 2026`. */
  label: string
  /** Index of the group's first photo in the whole list. */
  start: number
  files: MediaFile[]
}

/**
 * Photos grouped by the month they were last changed, newest first.
 *
 * Changed, not taken: when a photo was taken is inside the file, and reading
 * it would mean opening every photo. For photos copied off a camera or phone
 * the two are usually the same month, and a later version can read the date
 * the camera wrote.
 */
export function groupByMonth(files: MediaFile[]): MonthGroup[] {
  const groups: MonthGroup[] = []
  const formatter = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' })
  files.forEach((file, index) => {
    const date = new Date(file.mtime * 1000)
    const key = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`
    const last = groups[groups.length - 1]
    if (last && last.key === key) {
      last.files.push(file)
    } else {
      groups.push({ key, label: formatter.format(date), start: index, files: [file] })
    }
  })
  return groups
}

export interface TrackInfo {
  title: string
  /** Empty when the folders do not say. */
  artist: string
  album: string
  /** The number the file name starts with, when it has one. */
  number: number | null
}

/** Folder names that are containers rather than an artist or album. */
const GENERIC = new Set(['music', 'songs', 'audio', 'mp3', 'flac', 'my music', 'itunes', 'downloads'])

/**
 * What a track is, read from where it sits.
 *
 * `Music/Artist/Album/01 Title.flac` is how nearly every library is laid out,
 * so the folders are the artist and the album. Reading the tags inside the
 * file would be more exact, and would mean opening every song; this is what
 * the name alone can say.
 */
export function trackInfo(path: string): TrackInfo {
  const parts = path.split('/')
  const stem = stemOf(path)
  const numbered = /^\s*(\d{1,3})\s*(?:[-._)]\s*|\s+)(.+)$/.exec(stem)
  const title = (numbered ? numbered[2]! : stem).trim()
  const number = numbered ? Number(numbered[1]) : null

  const folders = parts.slice(0, -1).filter((p) => !GENERIC.has(p.trim().toLowerCase()))
  const album = folders.length >= 1 ? folders[folders.length - 1]! : ''
  const artist = folders.length >= 2 ? folders[folders.length - 2]! : ''
  return { title: title || stem, artist, album, number }
}
