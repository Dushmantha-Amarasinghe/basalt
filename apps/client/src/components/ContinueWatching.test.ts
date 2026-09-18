import { describe, expect, it } from 'vitest'
import { resumable } from './ContinueWatching'
import type { LibraryItem, Watched } from '@/lib/api'

function film(id: string, title: string, path: string): LibraryItem {
  return {
    id,
    kind: 'film',
    title,
    year: 2016,
    path,
    size: 1,
    added: 1,
    seasons: [],
    confidence: 90,
    hasArt: false,
  }
}

function series(id: string, title: string, episodes: string[]): LibraryItem {
  return {
    id,
    kind: 'series',
    title,
    year: 2008,
    path: null,
    size: 1,
    added: 1,
    seasons: [
      {
        number: 1,
        episodes: episodes.map((path, i) => ({
          number: i + 1,
          path,
          title: null,
          size: 1,
          added: 1,
        })),
      },
    ],
    confidence: 95,
    hasArt: false,
  }
}

function watched(path: string, fraction: number, updatedAt: number): Watched {
  return { path, fraction, position: 0, duration: 0, updatedAt }
}

describe('resumable', () => {
  it('matches a part-watched file back to its film', () => {
    const items = [film('f1', 'Arrival', 'Films/Arrival.mkv')]
    const found = resumable(items, [watched('Films/Arrival.mkv', 0.4, 10)])

    expect(found).toHaveLength(1)
    expect(found[0]!.item.title).toBe('Arrival')
    expect(found[0]!.episode).toBe('')
  })

  it('matches an episode back to its series and names it', () => {
    const items = [series('s1', 'Breaking Bad', ['a.mkv', 'b.mkv'])]
    const found = resumable(items, [watched('b.mkv', 0.3, 10)])

    expect(found[0]!.item.title).toBe('Breaking Bad')
    expect(found[0]!.episode).toBe('S01E02')
  })

  // One thing to carry on with, not a row of the same show.
  it('shows one card per series, the most recent', () => {
    const items = [series('s1', 'Breaking Bad', ['a.mkv', 'b.mkv', 'c.mkv'])]
    const found = resumable(items, [
      watched('c.mkv', 0.2, 30),
      watched('b.mkv', 0.5, 20),
      watched('a.mkv', 0.9, 10),
    ])

    expect(found).toHaveLength(1)
    expect(found[0]!.episode).toBe('S01E03')
  })

  // A resume point for something deleted is not worth a card.
  it('drops progress for a file the library no longer knows', () => {
    const items = [film('f1', 'Arrival', 'Films/Arrival.mkv')]
    const found = resumable(items, [watched('Films/Gone.mkv', 0.4, 10)])
    expect(found).toEqual([])
  })

  it('keeps the order it was given, which is newest first', () => {
    const items = [
      film('f1', 'Arrival', 'a.mkv'),
      film('f2', 'Dune', 'd.mkv'),
    ]
    const found = resumable(items, [watched('d.mkv', 0.2, 30), watched('a.mkv', 0.5, 10)])
    expect(found.map((r) => r.item.title)).toEqual(['Dune', 'Arrival'])
  })

  it('handles an empty library and empty progress', () => {
    expect(resumable([], [])).toEqual([])
    expect(resumable([], [watched('a.mkv', 0.5, 1)])).toEqual([])
    expect(resumable([film('f1', 'Arrival', 'a.mkv')], [])).toEqual([])
  })

  // Two shows are two cards; the per-series limit must not become a global one.
  it('shows a card for each different series', () => {
    const items = [
      series('s1', 'Breaking Bad', ['bb1.mkv']),
      series('s2', 'The Wire', ['tw1.mkv']),
    ]
    const found = resumable(items, [watched('bb1.mkv', 0.4, 20), watched('tw1.mkv', 0.6, 10)])
    expect(found.map((r) => r.item.title)).toEqual(['Breaking Bad', 'The Wire'])
  })
})
