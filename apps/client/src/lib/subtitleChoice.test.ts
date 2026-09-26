import { describe, expect, it } from 'vitest'
import {
  FIRST_TIME,
  chooseDriveFile,
  chooseSubtitle,
  describeTrack,
  labelOf,
  languageName,
  normaliseLang,
  prefFor,
} from './subtitleChoice'

const track = (id: number, fields: Parameters<typeof describeTrack>[0] extends infer T ? Omit<T & object, 'id'> : never) =>
  describeTrack({ id, ...fields })

describe('labels', () => {
  /// The two tracks in the report: one named only `en-US`, one named `SDH`.
  it('names a track by its language, with SDH as a tag', () => {
    const plain = labelOf(track(1, { lang: 'en-US' }))
    expect(plain.name).toMatch(/English/)
    expect(plain.tags).toEqual([])

    const sdh = labelOf(track(2, { lang: 'eng', title: 'SDH' }))
    expect(sdh.name).toBe('English')
    expect(sdh.tags).toEqual(['SDH'])
    expect(sdh.detail).toBe('')
  })

  it('keeps what a title says beyond the language', () => {
    const commentary = labelOf(track(3, { lang: 'en', title: 'English - Commentary' }))
    expect(commentary.name).toBe('English')
    expect(commentary.detail).toMatch(/Commentary/)
  })

  it('falls back to the title, then the number', () => {
    expect(labelOf(track(4, { title: 'Signs & Songs' })).name).toBe('Signs & Songs')
    expect(labelOf(track(5, {})).name).toBe('Track 5')
    expect(labelOf(track(6, { lang: 'und' })).name).toBe('Track 6')
  })

  it('reads the forms language tags come in', () => {
    expect(normaliseLang('eng')).toBe('en')
    expect(normaliseLang('en_GB')).toBe('en-GB')
    expect(normaliseLang('und')).toBe('')
    expect(languageName('spa')).toBe('Spanish')
    expect(languageName('xx-nonsense')).toBe('')
  })

  it('spots SDH and forced in titles as well as flags', () => {
    expect(describeTrack({ id: 1, title: 'English [CC]' }).sdh).toBe(true)
    expect(describeTrack({ id: 1, hearingImpaired: true }).sdh).toBe(true)
    expect(describeTrack({ id: 1, title: 'English (Forced)' }).forced).toBe(true)
    // "Hindi" contains "hi", which is not the SDH abbreviation inside a word.
    expect(describeTrack({ id: 1, title: 'Hindi' }).sdh).toBe(false)
  })
})

describe('choosing a track', () => {
  const tracks = [
    track(1, { lang: 'spa' }),
    track(2, { lang: 'eng', title: 'SDH' }),
    track(3, { lang: 'eng' }),
    track(4, { lang: 'eng', title: 'Forced', forced: true }),
  ]

  it('shows subtitles the first time a file has them', () => {
    expect(chooseSubtitle(tracks, FIRST_TIME)).not.toBeNull()
    expect(chooseSubtitle([track(9, { lang: 'fre' })], FIRST_TIME)).toBe(9)
  })

  it('follows the last choice: language, then SDH or plain', () => {
    expect(chooseSubtitle(tracks, { on: true, lang: 'en', sdh: false })).toBe(3)
    expect(chooseSubtitle(tracks, { on: true, lang: 'en-US', sdh: true })).toBe(2)
    expect(chooseSubtitle(tracks, { on: true, lang: 'es', sdh: false })).toBe(1)
  })

  it('stays off when turned off, except for forced lines', () => {
    expect(chooseSubtitle(tracks, { on: false, lang: null, sdh: false })).toBe(4)
    expect(chooseSubtitle(tracks.slice(0, 3), { on: false, lang: null, sdh: false })).toBeNull()
  })

  it('never picks a forced track while a full one exists', () => {
    expect(chooseSubtitle(tracks, { on: true, lang: 'en', sdh: false })).not.toBe(4)
  })

  it('has nothing to choose from nothing', () => {
    expect(chooseSubtitle([], FIRST_TIME)).toBeNull()
  })

  it('turns a choice into a preference for next time', () => {
    expect(prefFor(null)).toEqual({ on: false, lang: null, sdh: false })
    expect(prefFor(tracks[1]!)).toEqual({ on: true, lang: 'eng', sdh: true })
  })
})

describe('subtitle files beside the video', () => {
  const files = [
    { path: 'a.es.srt', label: 'Spanish' },
    { path: 'a.en.forced.srt', label: 'English forced' },
    { path: 'a.en.srt', label: 'English' },
  ]

  it('picks the language chosen before, and not the forced one', () => {
    expect(chooseDriveFile(files, { on: true, lang: 'eng', sdh: false })?.path).toBe('a.en.srt')
  })

  it('picks something when no language was chosen yet', () => {
    expect(chooseDriveFile(files, FIRST_TIME)).not.toBeNull()
  })

  it('loads nothing while subtitles are off', () => {
    expect(chooseDriveFile(files, { on: false, lang: null, sdh: false })).toBeNull()
  })
})
