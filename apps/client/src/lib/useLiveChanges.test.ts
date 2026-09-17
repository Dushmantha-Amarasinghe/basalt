import { describe, expect, it } from 'vitest'
import { affects } from './useLiveChanges'
import type { Change } from './api'

const created = (path: string): Change => ({ kind: 'created', path })
const removed = (path: string): Change => ({ kind: 'removed', path })
const renamed = (from: string, to: string): Change => ({ kind: 'renamed', from, to })

describe('affects', () => {
  it('matters when it happened in the folder on screen', () => {
    expect(affects(created('films/a.mkv'), 'films')).toBe(true)
    expect(affects(removed('films/a.mkv'), 'films')).toBe(true)
  })

  it('does not matter when it happened somewhere else', () => {
    expect(affects(created('docs/a.txt'), 'films')).toBe(false)
  })

  it('handles the root, where there is no separator', () => {
    expect(affects(created('a.mkv'), '')).toBe(true)
    expect(affects(created('films/a.mkv'), '')).toBe(false)
  })

  // A file two levels down does not change what the current folder lists.
  it('ignores a change deeper than the folder on screen', () => {
    expect(affects(created('films/2024/a.mkv'), 'films')).toBe(false)
  })

  // A move changes two listings, and someone looking at either one is wrong.
  it('matters at both ends of a move', () => {
    expect(affects(renamed('films/a.mkv', 'archive/a.mkv'), 'films')).toBe(true)
    expect(affects(renamed('films/a.mkv', 'archive/a.mkv'), 'archive')).toBe(true)
    expect(affects(renamed('films/a.mkv', 'archive/a.mkv'), 'docs')).toBe(false)
  })

  it('always matters when the host says it stopped counting', () => {
    expect(affects({ kind: 'resynchronise' }, 'anywhere')).toBe(true)
    expect(affects({ kind: 'resynchronise' }, '')).toBe(true)
  })

  // The library is a different screen; reloading a file listing for it would
  // be work for nothing.
  it('never reloads a file listing for a library change', () => {
    expect(affects({ kind: 'library_changed' }, '')).toBe(false)
    expect(affects({ kind: 'library_changed' }, 'films')).toBe(false)
  })
})
