import { describe, expect, it } from 'vitest'
import { joinPath, parentOf, toEntries, type DirEntry } from './api'

function entry(name: string, kind: 'dir' | 'file', mtime: number): DirEntry {
  return { name, kind, size: 100, mtime, readonly: false }
}

describe('toEntries', () => {
  // The bug this guards: the host speaks Unix seconds and everything in the
  // interface is milliseconds. Passing one through as the other silently dates
  // every file to January 1970.
  it('converts the host seconds into interface milliseconds', () => {
    const [converted] = toEntries('', [entry('a.txt', 'file', 1_700_000_000)])
    expect(converted!.modified).toBe(1_700_000_000_000)
    expect(new Date(converted!.modified).getUTCFullYear()).toBe(2023)
  })

  it('keeps dates before 1970 negative rather than clamping them', () => {
    const [converted] = toEntries('', [entry('old.txt', 'file', -86_400)])
    expect(converted!.modified).toBe(-86_400_000)
  })

  it('gives each entry its full vault path as an id', () => {
    expect(toEntries('films/2024', [entry('a.mkv', 'file', 0)])[0]!.id).toBe(
      'films/2024/a.mkv',
    )
  })

  // Selection is keyed on the id, so two files of the same name in different
  // folders must not collide.
  it('distinguishes files of the same name in different folders', () => {
    const a = toEntries('films', [entry('a.mkv', 'file', 0)])[0]!
    const b = toEntries('docs', [entry('a.mkv', 'file', 0)])[0]!
    expect(a.id).not.toBe(b.id)
  })

  it('does not prefix a separator at the root', () => {
    expect(toEntries('', [entry('a.txt', 'file', 0)])[0]!.id).toBe('a.txt')
  })

  it('carries kind and size through unchanged', () => {
    const [dir] = toEntries('', [entry('films', 'dir', 0)])
    expect(dir!.kind).toBe('dir')
    expect(dir!.size).toBe(100)
  })

  it('handles an empty listing', () => {
    expect(toEntries('films', [])).toEqual([])
  })
})

describe('joinPath', () => {
  it('joins below the root', () => {
    expect(joinPath('films', 'a.mkv')).toBe('films/a.mkv')
    expect(joinPath('films/2024', 'a.mkv')).toBe('films/2024/a.mkv')
  })

  it('does not put a separator in front at the root', () => {
    expect(joinPath('', 'a.mkv')).toBe('a.mkv')
  })
})

describe('parentOf', () => {
  it('walks one level up', () => {
    expect(parentOf('films/2024/a.mkv')).toBe('films/2024')
    expect(parentOf('films/a.mkv')).toBe('films')
  })

  it('stops at the root', () => {
    expect(parentOf('a.mkv')).toBe('')
    expect(parentOf('')).toBe('')
  })
})
