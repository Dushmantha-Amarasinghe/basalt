import { describe, expect, it } from 'vitest'
import { formatAgo, formatBytes, formatCountdown, formatRate, groupPin } from './utils'

describe('formatRate', () => {
  it('reads in decimal units, matching how links are quoted', () => {
    expect(formatRate(1_000)).toBe('1.0 KB/s')
    expect(formatRate(22_700_000)).toBe('22.7 MB/s')
  })

  it('drops the decimal once the number is wide', () => {
    expect(formatRate(120_000_000)).toBe('120 MB/s')
  })

  // An idle device should read as idle, not as "0.0 B/s", which looks like a
  // stalled transfer rather than nothing happening.
  it('shows a dash rather than zero when nothing is moving', () => {
    expect(formatRate(0)).toBe('—')
    expect(formatRate(0.4)).toBe('—')
  })

  it('does not print NaN or Infinity', () => {
    expect(formatRate(Number.NaN)).toBe('—')
    expect(formatRate(Number.POSITIVE_INFINITY)).toBe('—')
  })
})

describe('formatBytes', () => {
  it('uses binary units for sizes', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1024 ** 3)).toBe('1.0 GB')
  })
})

describe('formatAgo', () => {
  // The host speaks Unix seconds; treating one as milliseconds dates every
  // device to 1970, which has happened once already in this project.
  it('reads the host seconds as seconds', () => {
    const tenMinutesAgo = Math.floor(Date.now() / 1000) - 600
    expect(formatAgo(tenMinutesAgo)).toBe('10m ago')
  })

  it('calls a device that has never connected never', () => {
    expect(formatAgo(0)).toBe('never')
  })
})

describe('groupPin', () => {
  it('splits six digits so they can be read out', () => {
    expect(groupPin('169241')).toBe('169 241')
  })

  it('leaves anything else alone', () => {
    expect(groupPin('1692')).toBe('1692')
    expect(groupPin('')).toBe('')
  })
})

describe('formatCountdown', () => {
  it('counts in minutes and seconds', () => {
    expect(formatCountdown(125)).toBe('2:05')
    expect(formatCountdown(9)).toBe('0:09')
  })

  it('stops at zero rather than going negative', () => {
    expect(formatCountdown(0)).toBe('0:00')
    expect(formatCountdown(-5)).toBe('0:00')
  })
})
