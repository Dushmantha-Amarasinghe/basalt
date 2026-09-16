import { describe, expect, it } from 'vitest'
import type { Entry } from './FileList'
import { sortEntries } from './SortMenu'

function entry(
  name: string,
  kind: Entry['kind'],
  size: number,
  modified: number,
): Entry {
  return { id: name, name, kind, size, modified }
}

const dirs = [
  entry('Photos', 'dir', 0, 300),
  entry('Archive', 'dir', 0, 100),
  entry('Music', 'dir', 0, 200),
]

const files = [
  entry('film.mkv', 'file', 2_000_000_000, 500),
  entry('notes.txt', 'file', 1_200, 900),
  entry('song.mp3', 'file', 5_000_000, 700),
]

const all = [...dirs, ...files]

const names = (entries: Entry[]): string[] => entries.map((e) => e.name)

describe('sortEntries', () => {
  it('pins folders above files regardless of field or direction', () => {
    const fields = ['name', 'size', 'modified', 'type'] as const
    const directions = ['asc', 'desc'] as const

    for (const field of fields) {
      for (const direction of directions) {
        const sorted = sortEntries(all, field, direction)
        const firstFile = sorted.findIndex((e) => e.kind === 'file')
        const lastDir = sorted.map((e) => e.kind).lastIndexOf('dir')
        expect(lastDir, `${field}/${direction}`).toBeLessThan(firstFile)
      }
    }
  })

  it('does not mutate the array it is given', () => {
    const input = [...all]
    const before = names(input)
    sortEntries(input, 'size', 'desc')
    expect(names(input)).toEqual(before)
  })

  it('orders names naturally, so file2 precedes file10', () => {
    const numbered = [
      entry('file10.txt', 'file', 1, 1),
      entry('file2.txt', 'file', 1, 1),
      entry('file1.txt', 'file', 1, 1),
    ]
    expect(names(sortEntries(numbered, 'name', 'asc'))).toEqual([
      'file1.txt',
      'file2.txt',
      'file10.txt',
    ])
  })

  it('sorts by size in both directions', () => {
    expect(names(sortEntries(files, 'size', 'asc'))).toEqual([
      'notes.txt',
      'song.mp3',
      'film.mkv',
    ])
    expect(names(sortEntries(files, 'size', 'desc'))).toEqual([
      'film.mkv',
      'song.mp3',
      'notes.txt',
    ])
  })

  it('sorts by modified time, newest first when descending', () => {
    expect(names(sortEntries(files, 'modified', 'desc'))).toEqual([
      'notes.txt',
      'song.mp3',
      'film.mkv',
    ])
  })

  it('groups by extension when sorting by type', () => {
    const mixed = [
      entry('b.txt', 'file', 1, 1),
      entry('a.mkv', 'file', 1, 1),
      entry('a.txt', 'file', 1, 1),
    ]
    expect(names(sortEntries(mixed, 'type', 'asc'))).toEqual([
      'a.mkv',
      'a.txt',
      'b.txt',
    ])
  })

  it('treats a file with no extension as having an empty one', () => {
    const mixed = [entry('readme.md', 'file', 1, 1), entry('LICENSE', 'file', 1, 1)]
    expect(names(sortEntries(mixed, 'type', 'asc'))).toEqual(['LICENSE', 'readme.md'])
  })

  // The bug this guards: folders all report size 0, so sorting by size left
  // them in arrival order and the folder block appeared to shuffle itself
  // every time the sort field changed.
  it('keeps tied entries in a stable, name-defined order', () => {
    for (const field of ['size', 'modified', 'type'] as const) {
      const sorted = sortEntries(dirs, field, 'asc')
      expect(names(sorted), field).toEqual(['Archive', 'Music', 'Photos'])
    }
  })

  it('reverses tied entries when the direction flips', () => {
    expect(names(sortEntries(dirs, 'size', 'desc'))).toEqual([
      'Photos',
      'Music',
      'Archive',
    ])
  })

  it('handles an empty listing', () => {
    expect(sortEntries([], 'name', 'asc')).toEqual([])
  })
})
