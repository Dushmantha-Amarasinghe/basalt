import { describe, expect, it } from 'vitest'
import { isReturn, scrollKeyOf } from './useScrollMemory'

const files = (folder: string, search = ''): string => scrollKeyOf('files', folder, search)

describe('where a file list starts', () => {
  // The bug this guards: the list kept its scroll offset from one folder to
  // the next, so a folder opened from halfway down the drive opened halfway
  // down too.
  it('starts a folder opened from its parent at the top', () => {
    expect(isReturn(files(''), files('Films'))).toBe(false)
    expect(isReturn(files('Films'), files('Films/Classics'))).toBe(false)
  })

  it('starts at the top even in a folder visited before, going down into it', () => {
    expect(isReturn(files('Photos'), files('Films'))).toBe(false)
  })

  it('puts a folder gone back up to where it was left', () => {
    expect(isReturn(files('Films'), files(''))).toBe(true)
    expect(isReturn(files('Films/Classics/Noir'), files('Films'))).toBe(true)
  })

  it('does not take a folder with a longer name for a parent', () => {
    // "Films 2" is not inside "Films", so this is a new folder, not a return.
    expect(isReturn(files('Films 2'), files('Films'))).toBe(false)
  })

  it('starts a new search at the top and comes back when it is cleared', () => {
    expect(isReturn(files('Films'), files('Films', 'arr'))).toBe(false)
    expect(isReturn(files('Films', 'arr'), files('Films', 'arri'))).toBe(false)
    expect(isReturn(files('Films', 'arri'), files('Films'))).toBe(true)
  })

  it('keeps each section where it was when switching between them', () => {
    expect(isReturn(files('Films'), scrollKeyOf('recent', '', ''))).toBe(true)
    expect(isReturn(scrollKeyOf('starred', '', ''), files('Films'))).toBe(true)
  })

  it('puts a list that appears afresh where it was', () => {
    expect(isReturn(null, files('Films'))).toBe(true)
  })

  it('reads back a key whose search contains the separator', () => {
    expect(isReturn(files('Films', 'a|b'), files('Films'))).toBe(true)
  })
})
