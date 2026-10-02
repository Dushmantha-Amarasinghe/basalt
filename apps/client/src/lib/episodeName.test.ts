import { describe, expect, it } from 'vitest'
import { episodeName } from './episodeName'

describe('an episode named from its file', () => {
  it('is the words between the episode number and the release tags', () => {
    expect(
      episodeName(
        'TV/Signal House/Signal.House.S01E02.Low.Water.2160p.10bit.AMZN.WEB-DL.DDP5.1.HEVC-GROUP.mkv',
        2,
      ),
    ).toBe('Low Water')
    expect(episodeName('Northwind S01E03 - The Long Night [1080p].mkv', 3)).toBe('The Long Night')
    expect(episodeName('northwind.1x04.Harbour.Lights.720p.HDTV.x264.mkv', 4)).toBe('Harbour Lights')
  })

  it('keeps a pilot named after its show', () => {
    expect(episodeName('Copperline.S01E01.Copperline.2160p.10bit.AMZN.WEB-DL.mkv', 1)).toBe('Copperline')
  })

  it('is "Episode N" when the file gives no name', () => {
    expect(episodeName('Northwind.S01E05.1080p.WEB.H264-GROUP.mkv', 5)).toBe('Episode 5')
    expect(episodeName('Northwind S01E06.mkv', 6)).toBe('Episode 6')
    expect(episodeName('random video.mkv', 7)).toBe('Episode 7')
  })
})
