import { describe, expect, it } from 'vitest'
import type { LibraryItem } from './api'
import { withEmptyLists } from './useMediaLibrary'
import { resumable } from '@/components/ContinueWatching'
import { subtitleBadge } from './subtitles'

/**
 * A film exactly as a host that omits empty lists sends one.
 *
 * Typed through `unknown` on purpose: the point of these tests is what
 * arrives over the wire, which does not have to match the declared type —
 * and if it did, there would have been no bug to fix.
 */
const wireFilm = (extra: Record<string, unknown> = {}): LibraryItem =>
  ({
    id: 'f1',
    kind: 'film',
    title: 'Night Harbour',
    year: 2024,
    path: 'Movies/Night Harbour (2024).mkv',
    size: 95_165_174,
    added: 1_789_936_039,
    confidence: 90,
    hasArt: false,
    ...extra,
  }) as unknown as LibraryItem

describe('withEmptyLists', () => {
  it('gives a film the empty seasons list the host left out', () => {
    const [film] = withEmptyLists([wireFilm()])
    expect(film?.seasons).toEqual([])
    expect(film?.subtitles).toEqual([])
  })

  it('leaves everything else exactly as it came', () => {
    const [film] = withEmptyLists([wireFilm()])
    expect(film?.title).toBe('Night Harbour')
    expect(film?.year).toBe(2024)
    expect(film?.path).toBe('Movies/Night Harbour (2024).mkv')
    expect(film?.hasArt).toBe(false)
  })

  it('does not flatten a series that already has its seasons', () => {
    const series = {
      id: 's1',
      kind: 'series',
      title: 'Northwind',
      size: 1,
      added: 1,
      confidence: 88,
      hasArt: true,
      seasons: [
        {
          number: 1,
          episodes: [{ number: 1, path: 'a.mkv', size: 1, added: 1 }],
        },
      ],
    } as unknown as LibraryItem

    const [normalised] = withEmptyLists([series])
    expect(normalised?.seasons).toHaveLength(1)
    expect(normalised?.seasons[0]?.episodes).toHaveLength(1)
  })

  it('fills a season that arrived without its episodes', () => {
    const odd = {
      id: 's2',
      kind: 'series',
      title: 'Empty',
      size: 1,
      added: 1,
      confidence: 50,
      hasArt: false,
      seasons: [{ number: 1 }],
    } as unknown as LibraryItem

    expect(withEmptyLists([odd])[0]?.seasons[0]?.episodes).toEqual([])
  })

  it('reads nothing out of nothing', () => {
    expect(withEmptyLists([])).toEqual([])
  })
})

/**
 * The crash itself, at the two places that took the screen down.
 *
 * Opening Movies against a host that omitted `seasons` threw out of render,
 * so the page showed as a bare black window. Both of these read `seasons`
 * without checking, and both run for films.
 */
describe('a film from a host that omits empty lists', () => {
  it('can be counted for Continue watching without throwing', () => {
    expect(() => resumable(withEmptyLists([wireFilm()]), [])).not.toThrow()
  })

  it('can have its subtitle badge worked out without throwing', () => {
    const [film] = withEmptyLists([wireFilm()])
    expect(subtitleBadge(film!)).toBeNull()
  })

  it('counts its episodes as none rather than throwing', () => {
    const [film] = withEmptyLists([wireFilm()])
    expect(film!.seasons.reduce((n, s) => n + s.episodes.length, 0)).toBe(0)
  })
})
