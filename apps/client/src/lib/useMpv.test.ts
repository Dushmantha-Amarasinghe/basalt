import { describe, expect, it } from 'vitest'
import { hasEnded } from './useMpv'

describe('hasEnded', () => {
  /// The two bugs this exists for, both reported from real use: stepping a
  /// frame jumped to the next episode, and so did clicking the middle of the
  /// timeline. Both were mpv reporting `eof-reached` away from the end.
  it('is not the end just because mpv says so mid-file', () => {
    expect(hasEnded(true, 600, 3600)).toBe(false)
    expect(hasEnded(true, 1800, 3600)).toBe(false)
    expect(hasEnded(true, 4.3, 3707)).toBe(false)
  })

  it('is the end at the end, allowing a little slack', () => {
    expect(hasEnded(true, 3600, 3600)).toBe(true)
    expect(hasEnded(true, 3599, 3600)).toBe(true)
    // Just outside the slack is still playing.
    expect(hasEnded(true, 3596, 3600)).toBe(false)
  })

  it('is never the end while nothing is loaded', () => {
    // Between files mpv reports eof with no duration, which is what fired
    // the instant an episode opened.
    expect(hasEnded(true, 0, 0)).toBe(false)
    expect(hasEnded(true, 120, 0)).toBe(false)
  })

  it('takes only a real true, not anything truthy', () => {
    expect(hasEnded(false, 3600, 3600)).toBe(false)
    expect(hasEnded(null, 3600, 3600)).toBe(false)
    expect(hasEnded(undefined, 3600, 3600)).toBe(false)
    expect(hasEnded('yes', 3600, 3600)).toBe(false)
  })
})
