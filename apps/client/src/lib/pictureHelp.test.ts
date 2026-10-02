import { describe, expect, it } from 'vitest'
import { cannotConvert, pictureNote, rememberCannotConvert, sizeName, whyNotConverted } from './pictureHelp'
import { convertedUrl, seekWithinConversion } from './useMpv'

const fourK = { width: 3840, height: 1920 }

describe('why the host did not convert', () => {
  it('tells a busy host from one that cannot, and from one too old to', () => {
    expect(
      whyNotConverted({ by: null, error: 'this host is already converting as much as it can', kind: 'unavailable', at: 0 }),
    ).toBe('busy')
    expect(whyNotConverted({ by: null, error: 'this host has no way to convert video', kind: 'unavailable', at: 0 })).toBe(
      'unable',
    )
    expect(whyNotConverted({ by: null, error: 'update Basalt Host', kind: 'unsupported', at: 0 })).toBe('outdated')
    expect(whyNotConverted({ by: null, error: 'the video could not be converted: x', kind: 'error', at: 0 })).toBe('failed')
    expect(whyNotConverted(null)).toBe('failed')
    expect(
      whyNotConverted({ by: null, error: 'video conversion is switched off on this host', kind: 'unavailable', at: 0 }),
    ).toBe('off')
    expect(
      whyNotConverted({
        by: null,
        error: "this host's computer is too slow to convert video as it is watched",
        kind: 'unavailable',
        at: 0,
      }),
    ).toBe('slow')
  })
})

describe('a host that cannot convert', () => {
  it('is remembered when it cannot or is too old, and not when it was only busy', () => {
    rememberCannotConvert('host-a', 'unable')
    rememberCannotConvert('host-b', 'busy')
    rememberCannotConvert('host-c', 'outdated')
    expect(cannotConvert('host-a')).toBe('unable')
    expect(cannotConvert('host-b')).toBeNull()
    expect(cannotConvert('host-c')).toBe('outdated')
    expect(cannotConvert('host-d')).toBeNull()
    rememberCannotConvert('host-e', 'off')
    expect(cannotConvert('host-e')).toBeNull()
    rememberCannotConvert('host-f', 'slow')
    expect(cannotConvert('host-f')).toBe('slow')
  })
})

describe('the note', () => {
  it('says the host is converting, and on what', () => {
    const [title, text] = pictureNote({ mode: 'converted', size: fourK, by: 'NVIDIA graphics' }, 'phone')
    expect(title).toBe('Converted by Basalt Host')
    expect(text).toContain('This phone can’t play 4K smoothly')
    expect(text).toContain('on its NVIDIA graphics')
  })

  it('says why it is playing lighter instead', () => {
    const [title, text] = pictureNote({ mode: 'lighter', size: fourK, why: 'busy' }, 'phone')
    expect(title).toBe('Playing in a lighter mode')
    expect(text).toContain('decode 4K video in hardware')
    expect(text).toContain('already converting for other devices')
    const [, outdated] = pictureNote({ mode: 'lighter', size: fourK, why: 'outdated' }, 'computer')
    expect(outdated).toContain('This computer')
    expect(outdated).toContain('needs updating')
  })

  it('names sizes as people do', () => {
    expect(sizeName(fourK)).toBe('4K')
    expect(sizeName({ width: 2560, height: 1440 })).toBe('1440p')
  })
})

describe('seeking in a conversion', () => {
  it('stays within what has arrived, and starts again for anything further', () => {
    // Started at 100 s, now at 130, arrived up to 190.
    expect(seekWithinConversion(150, 100, 130, 190)).toBe(true)
    expect(seekWithinConversion(120, 100, 130, 190)).toBe(true)
    expect(seekWithinConversion(300, 100, 130, 190)).toBe(false)
    expect(seekWithinConversion(90, 100, 130, 190)).toBe(false)
    expect(seekWithinConversion(105, 100, 130, 190)).toBe(false)
  })

  it('asks for a conversion from the film’s own time', () => {
    expect(convertedUrl('http://127.0.0.1:5/t/Films/a.mkv', 2490.5)).toBe(
      'http://127.0.0.1:5/t/Films/a.mkv?convert=2490.500',
    )
    expect(convertedUrl('x', -3)).toBe('x?convert=0.000')
  })
})
