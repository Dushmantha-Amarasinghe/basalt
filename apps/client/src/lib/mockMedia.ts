/**
 * Placeholder data for the library and transfer views.
 *
 * Deterministic, like the file list. Poster art is generated as a pair of
 * graphite tones rather than fetched, so the layout can be judged without any
 * network and without pretending we have real thumbnails yet.
 */

export interface MediaItem {
  id: string
  title: string
  subtitle: string
  /** Seconds, for video and music. */
  duration?: number
  size: number
  /** Two graphite stops for the placeholder poster. */
  tone: [string, string]
  progress?: number
  starred?: boolean
}

export interface Transfer {
  id: string
  name: string
  direction: 'down' | 'up'
  bytes: number
  transferred: number
  speed: number
  status: 'active' | 'queued' | 'done' | 'paused'
}

function makeRng(seed: number): () => number {
  let state = seed >>> 0 || 1
  return () => {
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    state >>>= 0
    return state / 0xffffffff
  }
}

/** Graphite tone pairs. Never colour — the palette stays monochrome. */
function tonePair(rng: () => number): [string, string] {
  const base = 18 + Math.floor(rng() * 26)
  const lift = 8 + Math.floor(rng() * 16)
  const hex = (v: number): string => v.toString(16).padStart(2, '0')
  return [
    `#${hex(base)}${hex(base)}${hex(base + 2)}`,
    `#${hex(base + lift)}${hex(base + lift)}${hex(base + lift + 2)}`,
  ]
}

const FILM_WORDS = [
  'Northern', 'Silent', 'Last', 'Winter', 'Hollow', 'Iron', 'Distant',
  'Broken', 'Amber', 'Quiet', 'Long', 'Salt', 'Glass', 'Ember',
]
const FILM_NOUNS = [
  'Shore', 'Harvest', 'Passage', 'Circuit', 'Reckoning', 'Signal', 'Country',
  'Archive', 'Tide', 'Season', 'Highway', 'Vault', 'Current',
]

const ARTISTS = [
  'Låpsley', 'Bonobo', 'Nils Frahm', 'Portico Quartet', 'Kiasmos',
  'Ólafur Arnalds', 'Floating Points', 'Jon Hopkins', 'Emancipator',
]

export function generateVideos(count = 36, seed = 7): MediaItem[] {
  const rng = makeRng(seed)
  return Array.from({ length: count }, (_, i) => {
    const title = `The ${FILM_WORDS[Math.floor(rng() * FILM_WORDS.length)]} ${
      FILM_NOUNS[Math.floor(rng() * FILM_NOUNS.length)]
    }`
    const watched = rng()
    return {
      id: `vid-${i}`,
      title,
      subtitle: `${2014 + Math.floor(rng() * 12)} · ${rng() > 0.5 ? '2160p' : '1080p'}`,
      duration: Math.floor(2400 + rng() * 5400),
      size: Math.floor(1.2e9 + rng() * 6e9),
      tone: tonePair(rng),
      // Only some have been started, so "continue watching" has something to
      // show without every tile carrying a bar.
      progress: watched > 0.62 ? rng() * 0.85 + 0.05 : undefined,
      starred: rng() > 0.82,
    }
  })
}

export function generateMusic(count = 40, seed = 11): MediaItem[] {
  const rng = makeRng(seed)
  return Array.from({ length: count }, (_, i) => ({
    id: `mus-${i}`,
    title: `${FILM_WORDS[Math.floor(rng() * FILM_WORDS.length)]} ${
      FILM_NOUNS[Math.floor(rng() * FILM_NOUNS.length)]
    }`,
    subtitle: ARTISTS[Math.floor(rng() * ARTISTS.length)]!,
    duration: Math.floor(140 + rng() * 300),
    size: Math.floor(6e6 + rng() * 5e7),
    tone: tonePair(rng),
    starred: rng() > 0.8,
  }))
}

export function generatePhotos(count = 60, seed = 13): MediaItem[] {
  const rng = makeRng(seed)
  const places = ['Lisbon', 'Kandy', 'Reykjavík', 'Kyoto', 'Galle', 'Oslo', 'Porto', 'Ella']
  return Array.from({ length: count }, (_, i) => ({
    id: `pho-${i}`,
    title: `${places[Math.floor(rng() * places.length)]}-${1000 + i}.jpg`,
    subtitle: `${2019 + Math.floor(rng() * 7)}`,
    size: Math.floor(1.5e6 + rng() * 9e6),
    tone: tonePair(rng),
    starred: rng() > 0.88,
  }))
}

export function generateTransfers(seed = 17): Transfer[] {
  const rng = makeRng(seed)
  const names = [
    'The Northern Shore (2160p).mkv',
    'Photos 2025-06.zip',
    'Projects/basalt-backup.tar',
    'Kandy-1043.jpg',
    'Invoices Q3.pdf',
    'Kiasmos - Blurred.flac',
  ]
  const statuses: Transfer['status'][] = ['active', 'active', 'queued', 'queued', 'done', 'done']

  return names.map((name, i) => {
    const bytes = Math.floor(2e7 + rng() * 4e9)
    const status = statuses[i]!
    const fraction = status === 'done' ? 1 : status === 'queued' ? 0 : 0.15 + rng() * 0.7
    return {
      id: `tr-${i}`,
      name,
      direction: rng() > 0.35 ? 'down' : 'up',
      bytes,
      transferred: Math.floor(bytes * fraction),
      speed: status === 'active' ? 4 + rng() * 18 : 0,
      status,
    }
  })
}

/** Seconds to `1:23:45` / `4:07`. */
export function formatDuration(seconds: number): string {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  return `${m}:${String(s).padStart(2, '0')}`
}
