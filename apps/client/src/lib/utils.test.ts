import { afterEach, describe, expect, it, vi } from 'vitest'
import { cn, formatBytes, formatDate } from './utils'

describe('formatBytes', () => {
  it('shows raw bytes below a kilobyte, with no decimal point', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1)).toBe('1 B')
    expect(formatBytes(1023)).toBe('1023 B')
  })

  it('steps up a unit at each 1024 boundary', () => {
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1024 ** 2)).toBe('1.0 MB')
    expect(formatBytes(1024 ** 3)).toBe('1.0 GB')
    expect(formatBytes(1024 ** 4)).toBe('1.0 TB')
  })

  it('stops at terabytes rather than inventing a unit', () => {
    expect(formatBytes(4096 * 1024 ** 4)).toBe('4096 TB')
  })

  it('drops the decimal once the number is wide enough without it', () => {
    expect(formatBytes(99 * 1024)).toBe('99.0 KB')
    expect(formatBytes(100 * 1024)).toBe('100 KB')
  })
})

describe('formatDate', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  const at = (now: number, timestamp: number): string => {
    vi.useFakeTimers()
    vi.setSystemTime(now)
    return formatDate(timestamp)
  }

  const now = new Date('2026-06-15T12:00:00Z').getTime()
  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour

  it('describes the last minute as just now', () => {
    expect(at(now, now)).toBe('just now')
    expect(at(now, now - 59_000)).toBe('just now')
  })

  it('counts up through minutes, hours and days', () => {
    expect(at(now, now - 5 * minute)).toBe('5m ago')
    expect(at(now, now - 3 * hour)).toBe('3h ago')
    expect(at(now, now - 2 * day)).toBe('2d ago')
  })

  it('switches to an absolute date after a week', () => {
    const result = at(now, now - 30 * day)
    expect(result).not.toMatch(/ago|just now/)
    expect(result).toMatch(/2026/)
  })
})

describe('cn', () => {
  it('lets a later tailwind class win over an earlier conflicting one', () => {
    expect(cn('px-2', 'px-4')).toBe('px-4')
  })

  it('drops falsy values', () => {
    expect(cn('flex', false && 'hidden', undefined, 'gap-2')).toBe('flex gap-2')
  })
})
