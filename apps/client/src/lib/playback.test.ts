import { describe, expect, it } from 'vitest'
import { judgeSound } from './playback'

describe('judgeSound', () => {
  it('says nothing until enough video has decoded', () => {
    expect(judgeSound(0, 0, 0)).toBe('unknown')
    expect(judgeSound(0, 1000, 5)).toBe('unknown')
  })

  it('says nothing in the first second, however much video has decoded', () => {
    // A file whose audio simply has not started decoding yet must not be
    // accused of being silent.
    expect(judgeSound(0, 10_000_000, 0.4)).toBe('unknown')
  })

  it('reports sound as soon as any audio decodes', () => {
    expect(judgeSound(1, 0, 0)).toBe('playing')
    expect(judgeSound(5000, 10_000_000, 10)).toBe('playing')
  })

  // The case this exists for: an MKV with AC3 audio. Chromium plays the
  // picture, decodes no audio, and reports no error at all.
  it('reports silence once video is decoding and audio still is not', () => {
    expect(judgeSound(0, 10_000_000, 2)).toBe('silent')
  })

  // On a platform without the webkit counters both stay at zero forever, and
  // claiming silence there would be a warning on every working file.
  it('never claims silence when no counters are available', () => {
    for (const seconds of [1, 10, 600]) {
      expect(judgeSound(0, 0, seconds)).toBe('unknown')
    }
  })
})
