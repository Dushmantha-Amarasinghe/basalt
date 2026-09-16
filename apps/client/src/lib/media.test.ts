import { describe, expect, it } from 'vitest'
import type { Entry } from '@/components/FileList'
import { entriesToMedia, entryToMedia, isPlayable } from './media'
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

describe('isPlayable', () => {
  it('accepts what the window can actually decode', () => {
    for (const name of ['a.mp4', 'a.M4V', 'a.webm', 'a.mp3', 'a.flac']) {
      expect(isPlayable(name), name).toBe(true)
    }
  })

  // Naming these honestly is the point: showing a black rectangle for an MKV
  // and letting the user work it out would be worse than saying so.
  it('refuses the formats that need a real decoder', () => {
    for (const name of ['a.mkv', 'a.avi', 'a.mov', 'a.wmv', 'a.txt', 'noext']) {
      expect(isPlayable(name), name).toBe(false)
    }
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
