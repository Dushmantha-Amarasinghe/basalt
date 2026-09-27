/**
 * The preview's library, built from `showcase.json`.
 *
 * What the browser preview shows, and what Basalt's videos and screenshots
 * are made from. Every title, name and song is invented, so nothing here can
 * belong to anyone.
 *
 * Pictures are optional. `?assets=<url>` points the preview at a folder of
 * them — posters, photos, thumbnails and short films, drawn by
 * `marketing/art.py` — and without it the preview draws its placeholders, as
 * a host with no pictures does. The file names are computed here and there
 * the same way; see `slug` and `photoFiles`.
 */

import data from './showcase.json'
import type {
  Collections,
  DirEntry,
  LibraryItem,
  MediaFile,
  ProfileView,
  Watched,
} from './api'

const DAY = 86_400
const GB = 1024 ** 3
const MB = 1024 ** 2

function now(): number {
  return Math.floor(Date.now() / 1000)
}

/** A path as a file name: lower case, anything but letters and digits a dash.
 *  Mirrors `slug` in marketing/assets.py. */
export function slug(path: string): string {
  return path
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
}

/** Where the pictures are, when the preview was given some. */
export function showcaseAssets(): string | null {
  if (typeof window === 'undefined') return null
  const base = new URLSearchParams(window.location.search).get('assets')
  if (!base) return null
  return base.endsWith('/') ? base : `${base}/`
}

export const HOST = data.host

// ---------------------------------------------------------------------------
// Films and series
// ---------------------------------------------------------------------------

const pad = (n: number): string => String(n).padStart(2, '0')

function filmPath(f: (typeof data.films)[number]): string {
  const quality = f.res[0]! >= 3840 ? '2160p' : '1080p'
  return `Films/${f.title} (${f.year})/${f.title.replace(/[^A-Za-z0-9]+/g, '.')}.${f.year}.${quality}.mkv`
}

function episodePath(show: (typeof data.series)[number], season: number, episode: number): string {
  const dotted = show.title.replace(/[^A-Za-z0-9]+/g, '.')
  return `Shows/${show.title}/Season ${pad(season)}/${dotted}.S${pad(season)}E${pad(episode)}.1080p.mkv`
}

export function library(): LibraryItem[] {
  const films: LibraryItem[] = data.films.map((f, i) => ({
    id: f.id,
    kind: 'film',
    title: f.title,
    year: f.year,
    path: filmPath(f),
    resolution: { width: f.res[0]!, height: f.res[1]! },
    size: f.size * GB,
    added: now() - DAY * (2 + i * 9),
    seasons: [],
    confidence: 99,
    hasArt: true,
  }))
  const series: LibraryItem[] = data.series.map((s, i) => ({
    id: s.id,
    kind: 'series',
    title: s.title,
    year: s.year,
    path: null,
    size: s.seasons.reduce((n, season) => n + season.episodes.length, 0) * 2.6 * GB,
    added: now() - DAY * (1 + i * 11),
    seasons: s.seasons.map((season) => ({
      number: season.number,
      episodes: season.episodes.map((title, e) => ({
        number: e + 1,
        path: episodePath(s, season.number, e + 1),
        resolution: { width: 1920, height: 1080 },
        title,
        size: 2.6 * GB,
        added: now() - DAY * (1 + i * 11),
      })),
    })),
    confidence: 99,
    hasArt: true,
  }))
  return [...films, ...series]
}

/** How long each thing runs, for the player and for resume points. */
export function durationOf(path: string): number {
  if (path.startsWith('Films/')) return 2 * 3600 + (path.length % 40) * 60
  if (path.startsWith('Shows/')) return 48 * 60 + (path.length % 9) * 60
  if (path.startsWith('Music/')) return 180 + (path.length % 90)
  return 60 + (path.length % 120)
}

export function watching(): Watched[] {
  return data.watching.map((w) => {
    let path: string
    if ('film' in w && w.film) {
      path = filmPath(data.films.find((f) => f.id === w.film)!)
    } else {
      const show = data.series.find((s) => s.id === w.series)!
      path = episodePath(show, w.season!, w.episode!)
    }
    const duration = durationOf(path)
    return {
      path,
      fraction: w.fraction,
      position: Math.round(duration * w.fraction),
      duration,
      updatedAt: now() - w.minutesAgo * 60,
    }
  })
}

// ---------------------------------------------------------------------------
// Photos, videos and music
// ---------------------------------------------------------------------------

/** Camera shapes, cycled through so the grid has portraits, panoramas and
 *  squares among the landscapes. Mirrors SHAPES in marketing/assets.py. */
const SHAPES: Array<[number, number]> = [
  [4000, 3000],
  [3000, 4000],
  [6000, 4000],
  [4000, 4000],
  [4000, 2250],
  [4000, 3000],
  [3000, 4000],
]

export interface ShowcasePhoto extends MediaFile {
  scene: string
  seed: number
}

/** Every photo, with the scene it is drawn from. Mirrors `photo_files` in
 *  marketing/assets.py. */
export function photoFiles(): ShowcasePhoto[] {
  const out: ShowcasePhoto[] = []
  let n = 0
  for (const group of data.photos) {
    group.scenes.forEach((scene, i) => {
      const [width, height] = SHAPES[n % SHAPES.length]!
      out.push({
        path: `${group.dir}/IMG_${2040 + n}.jpg`,
        size: Math.round((2.2 + (n % 5) * 0.4) * MB),
        mtime: now() - DAY * group.day - i * 1800,
        width,
        height,
        scene,
        seed: 500 + n,
      })
      n += 1
    })
  }
  return out
}

function videoFiles(): MediaFile[] {
  return data.videos.map((v) => ({ path: v.path, size: v.size * MB, mtime: now() - DAY * v.day }))
}

function musicFiles(): MediaFile[] {
  return data.music.flatMap((album) =>
    album.tracks.map((title, i) => ({
      path: `Music/${album.artist}/${album.album}/${pad(i + 1)} ${title}.flac`,
      size: Math.round((24 + (i % 7) * 3) * MB),
      mtime: now() - DAY * album.day,
    })),
  )
}

function otherFiles(): MediaFile[] {
  return data.files.map((f) => ({ path: f.path, size: Math.round(f.size * MB), mtime: now() - DAY * f.day }))
}

export function collections(): Collections {
  const photos = photoFiles().map(({ scene: _s, seed: _d, ...file }) => file)
  const videos = videoFiles()
  const music = musicFiles()
  const newest = (a: MediaFile, b: MediaFile): number => b.mtime - a.mtime
  const everything = [...photos, ...videos, ...music, ...otherFiles(), ...libraryFiles()]
  return {
    photos: [...photos].sort(newest),
    videos: [...videos].sort(newest),
    music: [...music].sort(newest),
    recent: everything.sort(newest).slice(0, 40),
    truncated: false,
  }
}

function libraryFiles(): MediaFile[] {
  return library().flatMap((item) =>
    item.kind === 'film'
      ? [{ path: item.path!, size: item.size, mtime: item.added }]
      : item.seasons.flatMap((s) => s.episodes.map((e) => ({ path: e.path, size: e.size, mtime: e.added }))),
  )
}

// ---------------------------------------------------------------------------
// The drive, as folders
// ---------------------------------------------------------------------------

/** One folder's listing, from every file the showcase knows about. */
export function listFolder(dir: string): DirEntry[] {
  const all = [...libraryFiles(), ...photoFiles(), ...videoFiles(), ...musicFiles(), ...otherFiles()]
  const prefix = dir ? `${dir}/` : ''
  const folders = new Map<string, number>()
  const files: DirEntry[] = []
  for (const file of all) {
    if (!file.path.startsWith(prefix)) continue
    const rest = file.path.slice(prefix.length)
    const cut = rest.indexOf('/')
    if (cut === -1) {
      files.push({ name: rest, kind: 'file', size: file.size, mtime: file.mtime, readonly: false })
    } else {
      const name = rest.slice(0, cut)
      folders.set(name, Math.max(folders.get(name) ?? 0, file.mtime))
    }
  }
  const dirs: DirEntry[] = [...folders].map(([name, mtime]) => ({
    name,
    kind: 'dir',
    size: 0,
    mtime,
    readonly: false,
  }))
  return [...dirs, ...files]
}

/** Whether a path is a folder the showcase has. */
export function isFolder(path: string): boolean {
  const parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : ''
  const name = path.slice(path.lastIndexOf('/') + 1)
  return listFolder(parent).some((e) => e.kind === 'dir' && e.name === name)
}

/** A file's own entry, for `stat`. */
export function statFile(path: string): DirEntry | null {
  const parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : ''
  const name = path.slice(path.lastIndexOf('/') + 1)
  return listFolder(parent).find((e) => e.name === name) ?? null
}

export function profiles(): ProfileView[] {
  return data.profiles.map((p, i) => ({
    id: p.id,
    name: p.name,
    color: p.color,
    hasPin: p.pin !== '',
    lastUsed: now() - i * DAY,
  }))
}

export function pins(): Map<string, string> {
  return new Map(data.profiles.filter((p) => p.pin).map((p) => [p.id, p.pin]))
}

// ---------------------------------------------------------------------------
// Pictures and footage, when the preview was given them
// ---------------------------------------------------------------------------

export function posterUrl(id: string): string | null {
  const base = showcaseAssets()
  return base ? `${base}art/${id}.jpg` : null
}

/** A grid picture of a photo or video. */
export function thumbFor(path: string): string {
  const base = showcaseAssets()
  return base ? `${base}thumbs/${slug(path)}.jpg` : ''
}

/** A photo at viewing size. */
export function photoFor(path: string): string {
  const base = showcaseAssets()
  return base ? `${base}photos/${slug(path)}.jpg` : ''
}

/** The film to show behind the player for a path: its own where it has one,
 *  and otherwise one drawn from its scene. Mirrors the footage assets.py
 *  renders. */
export function footageFor(path: string): string | null {
  const base = showcaseAssets()
  if (!base) return null
  const film = data.films.find((f) => filmPath(f) === path)
  if (film) return `${base}footage/${film.id === 'f1' ? 'column-zero' : film.scene}.mp4`
  const show = data.series.find((s) => path.startsWith(`Shows/${s.title}/`))
  if (show) return `${base}footage/${show.id === 's1' ? 'signal-house' : show.scene}.mp4`
  const video = data.videos.find((v) => v.path === path)
  if (video) return `${base}footage/${video.scene}.mp4`
  return null
}
