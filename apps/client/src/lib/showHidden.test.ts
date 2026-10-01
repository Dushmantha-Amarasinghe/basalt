import { describe, expect, it } from 'vitest'
import { toEntries } from './api'
import { visibleEntries } from './showHidden'

describe('hidden files', () => {
  const listing = toEntries('', [
    { name: 'desktop.ini', kind: 'file', size: 1, mtime: 0, readonly: false, hidden: true },
    { name: 'Photos', kind: 'dir', size: 0, mtime: 0, readonly: false },
    // A host from before the flag: never hidden.
    { name: 'notes.txt', kind: 'file', size: 1, mtime: 0, readonly: false },
  ])

  it('carries the flag from the host into the list', () => {
    expect(listing.find((e) => e.name === 'desktop.ini')?.hidden).toBe(true)
    expect(listing.find((e) => e.name === 'Photos')?.hidden).toBeUndefined()
  })

  it('leaves hidden items out unless asked, as Explorer does', () => {
    expect(visibleEntries(listing, false).map((e) => e.name)).toEqual(['Photos', 'notes.txt'])
    expect(visibleEntries(listing, true)).toHaveLength(3)
  })
})
