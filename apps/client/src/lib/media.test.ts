import { describe, expect, it } from 'vitest'
import type { Entry } from '@/components/FileList'
import type { LibraryItem } from './api'
import { entriesToMedia, entryToMedia, nextEpisodes } from './media'
import { baseName, localJoin } from './dialogs'

function file(id: string): Entry {
  return {
    id,
    name: id.split('/').pop()!,
    kind: 'file',
    size: 4242,
    modified: 1_700_000_000_000,
  }
}

describe('entryToMedia', () => {
  it('drops the extension from the title and shows the folder underneath', () => {
    const media = entryToMedia(file('films/2024/Holiday.mp4'))
    expect(media.title).toBe('Holiday')
    expect(media.subtitle).toBe('films/2024')
    expect(media.size).toBe(4242)
  })

  it('labels a file at the root as being in the vault', () => {
    expect(entryToMedia(file('a.mp4')).subtitle).toBe('Vault')
  })

  it('keeps the full path as the id, so opening knows which file it is', () => {
    expect(entryToMedia(file('films/a.mp4')).id).toBe('films/a.mp4')
  })

  it('leaves a name with no extension alone', () => {
    expect(entryToMedia(file('LICENSE')).title).toBe('LICENSE')
  })

  // The same film must look the same every time the grid is drawn, or the
  // library would reshuffle its colours on every render.
  it('gives a file the same tones every time', () => {
    const a = entryToMedia(file('films/a.mkv'))
    const b = entryToMedia(file('films/a.mkv'))
    expect(a.tone).toEqual(b.tone)
  })

  it('gives different files different tones', () => {
    const tones = new Set(
      Array.from({ length: 40 }, (_, i) => entryToMedia(file(`f${i}.mkv`)).tone.join()),
    )
    expect(tones.size).toBeGreaterThan(20)
  })

  it('stays monochrome, as the palette requires', () => {
    for (let i = 0; i < 50; i += 1) {
      for (const tone of entryToMedia(file(`f${i}.mkv`)).tone) {
        const [r, g, b] = [1, 3, 5].map((at) => parseInt(tone.slice(at, at + 2), 16))
        // Red and green equal, blue a touch above: a grey, never a hue.
        expect(r).toBe(g)
        expect(b! - r!).toBeLessThanOrEqual(2)
      }
    }
  })

  it('converts a whole list', () => {
    expect(entriesToMedia([file('a.mp4'), file('b.mp4')])).toHaveLength(2)
  })
})


describe('local paths', () => {
  it('takes the last segment whichever separator is used', () => {
    expect(baseName('C:\\Users\\me\\film.mkv')).toBe('film.mkv')
    expect(baseName('/home/me/film.mkv')).toBe('film.mkv')
    expect(baseName('film.mkv')).toBe('film.mkv')
  })

  it('joins with the separator the folder already uses', () => {
    expect(localJoin('C:\\Users\\me', 'a.txt')).toBe('C:\\Users\\me\\a.txt')
    expect(localJoin('/home/me', 'a.txt')).toBe('/home/me/a.txt')
  })

  it('does not double the separator', () => {
    expect(localJoin('C:\\Users\\me\\', 'a.txt')).toBe('C:\\Users\\me\\a.txt')
    expect(localJoin('/home/me/', 'a.txt')).toBe('/home/me/a.txt')
  })
})

describe('nextEpisodes', () => {
  const show = (title: string, paths: string[]): LibraryItem => ({
    id: title, kind: 'series', title, size: 1, added: 1, confidence: 90, hasArt: false,
    seasons: [
      { number: 1, episodes: paths.map((path, i) => ({ number: i + 1, path, size: 1, added: 1 })) },
    ],
  })

  it('plays the next episode of the same series', () => {
    const next = nextEpisodes([show('Northwind', ['a1.mkv', 'a2.mkv'])])
    expect(next.get('a1.mkv')?.path).toBe('a2.mkv')
    expect(next.get('a1.mkv')?.label).toBe('Northwind · S01E02')
  })

  it('stops at the end of a series instead of starting another', () => {
    // The bug: one flat list over every series meant finishing the last
    // episode of one show started an unrelated one, ten seconds in.
    const next = nextEpisodes([show('Northwind', ['a1.mkv']), show('The Quiet Coast', ['b1.mkv'])])
    expect(next.get('a1.mkv')).toBeUndefined()
    expect(next.get('b1.mkv')).toBeUndefined()
  })

  it('crosses a season boundary, which is where autoplay is most wanted', () => {
    const across: LibraryItem = {
      id: 's', kind: 'series', title: 'Show', size: 1, added: 1, confidence: 90, hasArt: false,
      seasons: [
        { number: 1, episodes: [{ number: 10, path: 's1e10.mkv', size: 1, added: 1 }] },
        { number: 2, episodes: [{ number: 1, path: 's2e01.mkv', size: 1, added: 1 }] },
      ],
    }
    expect(nextEpisodes([across]).get('s1e10.mkv')?.path).toBe('s2e01.mkv')
  })
})
