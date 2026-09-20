import { describe, expect, it } from 'vitest'
import type { LibraryItem } from './api'
import { episodesWithSubtitles, subtitleBadge, subtitleCount } from './subtitles'

function film(subs: string[]): LibraryItem {
  return {
    id: 'f', kind: 'film', title: 'Arrival', year: 2016, path: 'a.mkv',
    size: 1, added: 1, seasons: [], confidence: 90, hasArt: false,
    subtitles: subs.map((label) => ({ path: `${label}.srt`, label })),
  }
}

function series(withSubs: number, total: number): LibraryItem {
  return {
    id: 's', kind: 'series', title: 'Show', size: 1, added: 1, confidence: 90,
    hasArt: false,
    seasons: [
      {
        number: 1,
        episodes: Array.from({ length: total }, (_, i) => ({
          number: i + 1,
          path: `e${i}.mkv`,
          size: 1,
          added: 1,
          subtitles: i < withSubs ? [{ path: `e${i}.srt`, label: 'English' }] : [],
        })),
      },
    ],
  }
}

describe('subtitleBadge', () => {
  it('says nothing when there is nothing to say', () => {
    // An empty badge on every card is noise, the same reasoning as the
    // progress bar that only appears part-way through something.
    expect(subtitleBadge(film([]))).toBeNull()
    expect(subtitleBadge(series(0, 10))).toBeNull()
  })

  it('counts a film’s languages', () => {
    expect(subtitleBadge(film(['English']))).toBe('SUB')
    expect(subtitleBadge(film(['English', 'French']))).toBe('SUB 2')
  })

  it('reports how much of a series is covered, not how many files', () => {
    // "SUB 30" on a series would read as thirty languages. What is worth
    // knowing is whether the episodes are covered.
    expect(subtitleBadge(series(10, 10))).toBe('SUB')
    expect(subtitleBadge(series(3, 10))).toBe('SUB 3/10')
  })

  it('treats a missing field as none, since the host omits it when empty', () => {
    const bare = { ...film([]), subtitles: undefined }
    expect(subtitleCount(bare)).toBe(0)
    expect(subtitleBadge(bare)).toBeNull()
  })
})

describe('episodesWithSubtitles', () => {
  it('counts across every season', () => {
    expect(episodesWithSubtitles(series(4, 10))).toEqual([4, 10])
  })
})
