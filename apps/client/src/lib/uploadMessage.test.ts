import { describe, expect, it } from 'vitest'
import { plainReason, uploadFailure } from './uploadMessage'

describe('uploadFailure', () => {
  /// The report from a phone: one picked video, and the system's own words.
  it('says one file could not be uploaded, in plain words', () => {
    expect(
      uploadFailure(
        [['Videos/VID_0001.mp4', 'io: Software caused connection abort (os error 103)']],
        0,
      ),
    ).toBe("Couldn't upload VID_0001.mp4. The connection to the host dropped. Try again.")
  })

  it('counts a folder that partly arrived', () => {
    expect(uploadFailure([['Trip/b.jpg', 'already exists']], 9)).toBe(
      '1 file did not upload. b.jpg: Already exists.',
    )
    expect(
      uploadFailure(
        [
          ['Trip/b.jpg', 'x'],
          ['Trip/c.jpg', 'x'],
        ],
        8,
      ),
    ).toBe('2 of 10 files did not upload. b.jpg: X.')
  })

  it('says nothing when everything arrived', () => {
    expect(uploadFailure([], 3)).toBeNull()
  })
})

describe('plainReason', () => {
  it('recognises a dropped connection on every system', () => {
    for (const why of [
      'io: Software caused connection abort (os error 103)',
      'io: An established connection was aborted by the software in your host machine. (os error 10053)',
      'io: Connection reset by peer (os error 104)',
      'io: Broken pipe (os error 32)',
    ]) {
      expect(plainReason(why)).toBe('The connection to the host dropped. Try again.')
    }
  })

  it('tidies anything else into a sentence', () => {
    expect(plainReason('io: Permission denied (os error 13)')).toBe('Permission denied.')
    expect(plainReason('the phone would not say how big it is')).toBe(
      'The phone would not say how big it is.',
    )
  })
})
