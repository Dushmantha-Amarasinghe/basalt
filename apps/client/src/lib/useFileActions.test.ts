import { describe, expect, it } from 'vitest'
import { nameOf, uniqueName, wouldNest } from './useFileActions'

describe('nameOf', () => {
  it('takes the last segment', () => {
    expect(nameOf('films/2024/a.mkv')).toBe('a.mkv')
    expect(nameOf('a.mkv')).toBe('a.mkv')
    expect(nameOf('')).toBe('')
  })
})

describe('wouldNest', () => {
  // Without this, dragging a folder onto one of its own children asks the host
  // to copy a tree into its own subtree, which does not terminate.
  it('catches a folder being put inside itself', () => {
    expect(wouldNest('films', 'films')).toBe(true)
    expect(wouldNest('films', 'films/2024')).toBe(true)
    expect(wouldNest('films', 'films/2024/summer')).toBe(true)
  })

  it('allows anything that is genuinely elsewhere', () => {
    expect(wouldNest('films', 'docs')).toBe(false)
    expect(wouldNest('films', '')).toBe(false)
    expect(wouldNest('films/2024', 'films')).toBe(false)
  })

  // The trap a naive `startsWith` falls into: `films-old` is not inside
  // `films`, but its path does start with those characters.
  it('does not confuse a sibling whose name shares a prefix', () => {
    expect(wouldNest('films', 'films-old')).toBe(false)
    expect(wouldNest('films', 'filmsomething/deep')).toBe(false)
  })
})

describe('uniqueName', () => {
  it('leaves a free name alone', () => {
    expect(uniqueName('a.txt', new Set())).toBe('a.txt')
    expect(uniqueName('a.txt', new Set(['b.txt']))).toBe('a.txt')
  })

  it('numbers a collision before the extension', () => {
    expect(uniqueName('a.txt', new Set(['a.txt']))).toBe('a (2).txt')
    expect(uniqueName('a.txt', new Set(['a.txt', 'a (2).txt']))).toBe('a (3).txt')
  })

  it('numbers a folder with no extension', () => {
    expect(uniqueName('Films', new Set(['Films']))).toBe('Films (2)')
  })

  it('keeps only the last extension, so a double one survives', () => {
    expect(uniqueName('archive.tar.gz', new Set(['archive.tar.gz']))).toBe(
      'archive.tar (2).gz',
    )
  })

  // A leading dot is a hidden file, not an extension. Splitting on it would
  // produce ". (2)gitignore".
  it('treats a leading dot as part of the name', () => {
    expect(uniqueName('.gitignore', new Set(['.gitignore']))).toBe(
      '.gitignore (2)',
    )
  })

  it('keeps climbing past a long run of collisions', () => {
    const taken = new Set(['a.txt', ...Array.from({ length: 50 }, (_, i) => `a (${i + 2}).txt`)])
    expect(uniqueName('a.txt', taken)).toBe('a (52).txt')
  })

  it('never returns a name that is already taken', () => {
    const taken = new Set(['x', 'x (2)', 'x (3)', 'x (4)'])
    expect(taken.has(uniqueName('x', taken))).toBe(false)
  })
})
