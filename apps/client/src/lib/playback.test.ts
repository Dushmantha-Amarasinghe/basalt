import { describe, expect, it } from 'vitest'
import {
  isPlayable,
  judgeSound,
  playabilityOf,
  silenceMessage,
  unplayableMessage,
} from './playback'

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

  // The exact case that prompted all of this: an MKV whose AAC track Chromium
  // refuses because AAC is not on WebM's codec list. Picture runs, no sound,
  // no error anywhere.
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

describe('playabilityOf', () => {
  it('knows what the window handles completely', () => {
    for (const name of ['a.mp4', 'a.M4V', 'a.webm', 'a.mp3', 'a.flac', 'a.wav']) {
      expect(playabilityOf(name), name).toBe('full')
    }
  })

  // MKV is the interesting one: Chromium opens it, because WebM is a Matroska
  // subset, and then silently drops anything that is not a WebM codec.
  it('marks matroska as only half-understood', () => {
    expect(playabilityOf('a.mkv')).toBe('partial')
    expect(playabilityOf('A.MKV')).toBe('partial')
  })

  it('rules out the containers it cannot open at all', () => {
    for (const name of ['a.avi', 'a.mov', 'a.wmv', 'a.flv', 'a.ts', 'noext']) {
      expect(playabilityOf(name), name).toBe('none')
    }
  })

  it('opens the player for anything it can show something of', () => {
    expect(isPlayable('a.mp4')).toBe(true)
    expect(isPlayable('a.mkv')).toBe(true)
    expect(isPlayable('a.avi')).toBe(false)
  })
})

describe('the messages', () => {
  // The old message blamed AC3 and DTS for every silence, which is wrong for
  // the common case — an MKV with a perfectly ordinary AAC track.
  it('explains a silent matroska as a container limit, not a codec one', () => {
    const message = silenceMessage('films/holiday.mkv')
    expect(message).toContain('MKV')
    expect(message).toMatch(/Opus/)
    expect(message).toMatch(/AAC/)
  })

  it('falls back to naming the likely audio formats for anything else', () => {
    const message = silenceMessage('films/holiday.mp4')
    expect(message).toMatch(/AC3|DTS/)
    expect(message).not.toContain('WebM')
  })

  it('always says what to do next', () => {
    for (const name of ['a.mkv', 'a.mp4', 'a.avi']) {
      expect(silenceMessage(name), name).toContain('your usual player')
    }
    expect(unplayableMessage('a.avi')).toContain('your usual player')
  })

  it('names the format it cannot play', () => {
    expect(unplayableMessage('films/a.avi')).toContain('AVI')
  })

  it('does not say "THIS FILE file" when there is no extension', () => {
    expect(unplayableMessage('README')).toContain('this file')
    expect(unplayableMessage('README')).not.toContain('undefined')
  })
})
