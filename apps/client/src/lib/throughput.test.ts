import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  IDLE_FLOOR_RATE,
  SAMPLE_COUNT,
  TICK_MS,
  getCurrent,
  getCurrentMbps,
  getSamples,
  recordBytes,
  resetThroughput,
  subscribeThroughput,
} from './throughput'

afterEach(() => {
  vi.useRealTimers()
  resetThroughput()
})

/** Subscribes, runs the body, and always unsubscribes. */
function whileSubscribed(body: (listener: ReturnType<typeof vi.fn>) => void): void {
  const listener = vi.fn()
  const unsubscribe = subscribeThroughput(listener)
  try {
    body(listener)
  } finally {
    unsubscribe()
  }
}

describe('throughput store', () => {
  it('starts with a full buffer, so the sparkline never reads past its end', () => {
    expect(getSamples()).toHaveLength(SAMPLE_COUNT)
  })

  it('reads as idle until something actually moves', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      vi.advanceTimersByTime(TICK_MS * 20)
      expect(getCurrent()).toBe(0)
      expect(getCurrentMbps()).toBe(0)
    })
  })

  // The bug this replaced: the store generated a plausible random signal, so
  // the header reported a speed the app had invented.
  it('reports a rate derived from the bytes it was given', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      // 1 MB across one 120 ms tick is a little over 8 MB/s.
      recordBytes(1_000_000)
      vi.advanceTimersByTime(TICK_MS)

      const expected = 1_000_000 / (TICK_MS / 1000)
      expect(getCurrent()).toBeCloseTo(expected, -3)
      expect(getCurrentMbps()).toBeCloseTo(expected / 1e6, 1)
    })
  })

  it('falls back to idle once the bytes stop', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordBytes(5_000_000)
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBeGreaterThan(0)

      vi.advanceTimersByTime(TICK_MS * 2)
      expect(getCurrent()).toBe(0)
    })
  })

  it('ignores a trickle rather than showing a speed for nothing', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      // The floor is a rate, so the test has to reason in rates: a quarter of
      // the floor over one tick, which is a few hundred bytes.
      recordBytes((IDLE_FLOOR_RATE / 4) * (TICK_MS / 1000))
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBe(0)
    })
  })

  it('shows a rate that is only just above the floor', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordBytes(IDLE_FLOOR_RATE * 2 * (TICK_MS / 1000))
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBeGreaterThan(IDLE_FLOOR_RATE)
    })
  })

  // A resumed transfer restarts its byte count from the resume offset, and a
  // negative delta must not drag the trace below zero.
  it('ignores negative and nonsense deltas', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordBytes(-5_000_000)
      recordBytes(Number.NaN)
      recordBytes(Number.POSITIVE_INFINITY)
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBe(0)
      for (const value of getSamples()) expect(value).toBeGreaterThanOrEqual(0)
    })
  })

  it('accumulates everything recorded within one tick', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordBytes(400_000)
      recordBytes(400_000)
      recordBytes(200_000)
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBeCloseTo(1_000_000 / (TICK_MS / 1000), -3)
    })
  })

  it('notifies subscribers on every tick', () => {
    vi.useFakeTimers()
    whileSubscribed((listener) => {
      vi.advanceTimersByTime(TICK_MS * 3)
      expect(listener).toHaveBeenCalledTimes(3)
    })
  })

  it('stops ticking once the last subscriber leaves', () => {
    vi.useFakeTimers()
    const listener = vi.fn()
    subscribeThroughput(listener)()
    vi.advanceTimersByTime(TICK_MS * 5)
    expect(listener).not.toHaveBeenCalled()
  })

  // Two components subscribe in the real app (the canvas trace and the text
  // readout). If one unmounting killed the timer, the other would freeze.
  it('keeps ticking while any subscriber remains', () => {
    vi.useFakeTimers()
    const a = vi.fn()
    const b = vi.fn()
    const unsubA = subscribeThroughput(a)
    const unsubB = subscribeThroughput(b)

    unsubA()
    vi.advanceTimersByTime(TICK_MS * 2)
    expect(b).toHaveBeenCalledTimes(2)
    unsubB()
  })

  it('reuses one buffer rather than allocating per tick', () => {
    vi.useFakeTimers()
    const before = getSamples()
    whileSubscribed(() => {
      recordBytes(1_000_000)
      vi.advanceTimersByTime(TICK_MS * 10)
      expect(getSamples()).toBe(before)
    })
  })

  it('clears on demand, for when the connection drops', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordBytes(9_000_000)
      vi.advanceTimersByTime(TICK_MS)
      expect(getCurrent()).toBeGreaterThan(0)

      resetThroughput()
      for (const value of getSamples()) expect(value).toBe(0)
    })
  })
})
