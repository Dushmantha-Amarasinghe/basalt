import { describe, expect, it } from 'vitest'
import { folderOf, formatOf, groupByMonth, stemOf, trackInfo } from './mediaInfo'

const at = (y: number, m: number, d = 10): number => Math.floor(new Date(y, m - 1, d).getTime() / 1000)

describe('groupByMonth', () => {
  it('groups consecutive photos of one month, keeping their place', () => {
    const files = [
      { path: 'a.jpg', size: 1, mtime: at(2026, 9, 20) },
      { path: 'b.jpg', size: 1, mtime: at(2026, 9, 2) },
      { path: 'c.jpg', size: 1, mtime: at(2026, 8, 30) },
      { path: 'd.jpg', size: 1, mtime: at(2025, 8, 1) },
    ]
    const groups = groupByMonth(files)
    expect(groups.map((g) => g.key)).toEqual(['2026-09', '2026-08', '2025-08'])
    expect(groups.map((g) => g.start)).toEqual([0, 2, 3])
    expect(groups[0]!.files.map((f) => f.path)).toEqual(['a.jpg', 'b.jpg'])
    expect(groups[0]!.label).toMatch(/2026/)
  })

  it('is empty for no photos', () => {
    expect(groupByMonth([])).toEqual([])
  })
})

describe('trackInfo', () => {
  it('reads artist and album from the folders and the number off the name', () => {
    expect(trackInfo('Music/The Quiet Coast/Harbour Lights/01 Opening.flac')).toEqual({
      title: 'Opening',
      artist: 'The Quiet Coast',
      album: 'Harbour Lights',
      number: 1,
    })
  })

  it('understands the usual ways of numbering a track', () => {
    expect(trackInfo('A/B/03 - Low Tide.mp3').title).toBe('Low Tide')
    expect(trackInfo('A/B/03. Low Tide.mp3').title).toBe('Low Tide')
    expect(trackInfo('A/B/03_Low Tide.mp3').number).toBe(3)
    // A title that is only a number is still a title.
    expect(trackInfo('A/B/1999.mp3').title).toBe('1999')
  })

  it('does not call a container folder an artist', () => {
    expect(trackInfo('Music/Harbour Lights/02 Tides.mp3')).toMatchObject({
      artist: '',
      album: 'Harbour Lights',
    })
    expect(trackInfo('song.mp3')).toMatchObject({ title: 'song', artist: '', album: '' })
  })
})

describe('names', () => {
  it('splits a path into its parts', () => {
    expect(stemOf('Photos/2024/IMG_0001.HEIC')).toBe('IMG_0001')
    expect(formatOf('Photos/2024/IMG_0001.HEIC')).toBe('HEIC')
    expect(folderOf('Photos/2024/IMG_0001.HEIC')).toBe('Photos/2024')
    expect(folderOf('top.png')).toBe('')
  })
})
