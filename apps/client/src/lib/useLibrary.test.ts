import { describe, expect, it } from 'vitest'
import type { Entry } from '@/components/FileList'
import { extensionOf, filterKind, isKind, recentOf } from './useLibrary'

function file(id: string, modified = 0): Entry {
  return {
    id,
    name: id.split('/').pop()!,
    kind: 'file',
    size: 1000,
    modified,
  }
}

describe('extensionOf', () => {
  it('reads the extension in lower case', () => {
    expect(extensionOf('a.MKV')).toBe('mkv')
    expect(extensionOf('holiday.2024.mp4')).toBe('mp4')
  })

  it('returns nothing for a file without one', () => {
    expect(extensionOf('LICENSE')).toBe('')
    expect(extensionOf('')).toBe('')
  })

  // A leading dot is a hidden file, not an extension: `.gitignore` is named
  // `.gitignore`, it is not a `gitignore` file.
  it('does not treat a leading dot as an extension', () => {
    expect(extensionOf('.gitignore')).toBe('')
  })
})

describe('isKind', () => {
  it('recognises video, audio and images', () => {
    expect(isKind('a.mkv', 'videos')).toBe(true)
    expect(isKind('a.MP4', 'videos')).toBe(true)
    expect(isKind('a.flac', 'music')).toBe(true)
    expect(isKind('a.jpeg', 'photos')).toBe(true)
  })

  it('does not put a file in the wrong section', () => {
    expect(isKind('a.mkv', 'music')).toBe(false)
    expect(isKind('a.mp3', 'videos')).toBe(false)
    expect(isKind('a.txt', 'photos')).toBe(false)
    expect(isKind('README', 'videos')).toBe(false)
  })
})

describe('filterKind', () => {
  const files = [
    file('films/a.mkv', 300),
    file('docs/notes.txt', 400),
    file('music/song.mp3', 100),
    file('photos/holiday.jpg', 200),
    file('films/b.mp4', 500),
  ]

  it('keeps only the matching kind', () => {
    expect(filterKind(files, 'videos').map((f) => f.name)).toEqual([
      'b.mp4',
      'a.mkv',
    ])
    expect(filterKind(files, 'music').map((f) => f.name)).toEqual(['song.mp3'])
  })

  it('puts the newest first', () => {
    const sorted = filterKind(files, 'videos')
    expect(sorted[0]!.modified).toBeGreaterThan(sorted[1]!.modified)
  })

  it('returns nothing rather than failing when there is no match', () => {
    expect(filterKind([file('a.txt')], 'videos')).toEqual([])
    expect(filterKind([], 'photos')).toEqual([])
  })
})

describe('recentOf', () => {
  it('sorts newest first', () => {
    const files = [file('a', 100), file('b', 300), file('c', 200)]
    expect(recentOf(files).map((f) => f.name)).toEqual(['b', 'c', 'a'])
  })

  it('caps the list', () => {
    const many = Array.from({ length: 500 }, (_, i) => file(`f${i}`, i))
    expect(recentOf(many, 10)).toHaveLength(10)
    expect(recentOf(many, 10)[0]!.modified).toBe(499)
  })

  it('does not mutate what it is given', () => {
    const files = [file('a', 100), file('b', 300)]
    const before = files.map((f) => f.name)
    recentOf(files)
    expect(files.map((f) => f.name)).toEqual(before)
  })
})
