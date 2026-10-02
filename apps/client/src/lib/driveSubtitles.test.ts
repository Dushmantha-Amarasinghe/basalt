// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { chosenFor, mergeDriveSubtitles, otherSubtitles, rememberChosen } from './driveSubtitles'

const t = (path: string, label = 'English') => ({ path, label })

beforeEach(() => window.localStorage.clear())

describe('subtitles on the drive for one video', () => {
  it('puts the library first and repeats nothing the host also names', () => {
    const merged = mergeDriveSubtitles(
      [t('Films/A/A.en.srt')],
      [t('Films/A/A.en.srt'), t('Downloads/A.2016.es.srt', 'Spanish')],
      [],
    )
    expect(merged.map((m) => m.path)).toEqual(['Films/A/A.en.srt', 'Downloads/A.2016.es.srt'])
  })

  it('offers again a file chosen by hand before, named by its file', () => {
    const merged = mergeDriveSubtitles([], [], ['Elsewhere/odd name.srt'])
    expect(merged).toEqual([{ path: 'Elsewhere/odd name.srt', label: 'odd name.srt' }])
  })

  it('leaves out of the others anything already offered', () => {
    const others = otherSubtitles([t('a.srt'), t('b.srt')], [t('a.srt')])
    expect(others.map((o) => o.path)).toEqual(['b.srt'])
  })
})

describe('a subtitle chosen by hand', () => {
  it('is remembered for that video, newest first, and only that video', () => {
    rememberChosen('Films/A.mkv', 'x.srt')
    rememberChosen('Films/A.mkv', 'y.srt')
    rememberChosen('Films/A.mkv', 'x.srt')
    expect(chosenFor('Films/A.mkv')).toEqual(['x.srt', 'y.srt'])
    expect(chosenFor('Films/B.mkv')).toEqual([])
  })
})
