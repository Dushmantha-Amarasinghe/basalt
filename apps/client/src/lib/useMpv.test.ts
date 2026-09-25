import { describe, expect, it } from 'vitest'
import { hasEnded, judgeOpening, OPENING, type OpeningProbe, type OpeningWatch } from './useMpv'

describe('judgeOpening', () => {
  const url = 'http://127.0.0.1:5000/media/Night.Harbour.mkv'
  const probe = (p: Partial<OpeningProbe>): OpeningProbe => ({
    path: url,
    position: 0,
    width: 1280,
    paused: false,
    idle: false,
    ...p,
  })
  /** Feeds probes in order, 80 ms apart, and returns every verdict. */
  const run = (probes: Partial<OpeningProbe>[]): string[] => {
    let watch: OpeningWatch = OPENING
    return probes.map((p, i) => {
      const next = judgeOpening(watch, probe(p), i * 80, url)
      watch = next.watch
      return next.verdict
    })
  }

  it('waits for the clock to move before trusting the picture', () => {
    expect(run([{ position: 0 }, { position: 0.04 }, { position: 0.2 }])).toEqual([
      'waiting',
      'waiting',
      'video',
    ])
  })

  /// Straight after `loadfile` the clock is still the old file's, and it is
  /// moving. Trusting it made the page see-through over a frame that was not
  /// there yet.
  it('ignores the clock of the file being replaced', () => {
    expect(
      run([
        { path: 'http://127.0.0.1:5000/media/Earlier.Episode.mkv', position: 1500 },
        { path: 'http://127.0.0.1:5000/media/Earlier.Episode.mkv', position: 1500.3 },
        { path: null, position: null },
        { position: 0 },
      ]),
    ).toEqual(['waiting', 'waiting', 'waiting', 'waiting'])
  })

  it('counts a resumed start from where it resumed', () => {
    expect(run([{ position: 1234.5 }, { position: 1234.6 }, { position: 1234.8 }])).toEqual([
      'waiting',
      'waiting',
      'video',
    ])
  })

  it('never says video for a file with nothing to show', () => {
    expect(run([{ position: 0, width: 0 }, { position: 0.3, width: 0 }])).toEqual([
      'waiting',
      'sound',
    ])
  })

  it('shows a file that opens paused once it has held still a moment', () => {
    const verdicts = run(Array.from({ length: 7 }, () => ({ position: 12, paused: true })))
    expect(verdicts.slice(0, 5)).toEqual(Array(5).fill('waiting'))
    expect(verdicts.at(-1)).toBe('video')
  })

  it('waits as long as the network takes', () => {
    const verdicts = run(Array.from({ length: 100 }, () => ({ position: null })))
    expect(new Set(verdicts)).toEqual(new Set(['waiting']))
  })

  it('gives up on a file mpv went idle instead of opening', () => {
    const verdicts = run(Array.from({ length: 45 }, () => ({ path: null, position: null, idle: true })))
    expect(verdicts.slice(0, 30)).toEqual(Array(30).fill('waiting'))
    expect(verdicts.at(-1)).toBe('failed')
  })
})

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

  /// Dragged or skipped to the end, mpv stops there paused and never raises
  /// the flag. That is the end, and the next episode has to come.
  it('is the end when stopped paused at the end without the flag', () => {
    expect(hasEnded(false, 180, 180, true)).toBe(true)
    expect(hasEnded(null, 3599, 3600, true)).toBe(true)
  })

  it('is not the end for a pause anywhere else', () => {
    expect(hasEnded(false, 1800, 3600, true)).toBe(false)
    expect(hasEnded(false, 3590, 3600, true)).toBe(false)
    expect(hasEnded(false, 0, 0, true)).toBe(false)
  })

  it('takes only a real true, not anything truthy', () => {
    expect(hasEnded(false, 3600, 3600)).toBe(false)
    expect(hasEnded(null, 3600, 3600)).toBe(false)
    expect(hasEnded(undefined, 3600, 3600)).toBe(false)
    expect(hasEnded('yes', 3600, 3600)).toBe(false)
  })
})
