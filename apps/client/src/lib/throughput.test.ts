import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  IDLE_AFTER_MS,
  IDLE_FLOOR_RATE,
  SAMPLE_COUNT,
  getCurrent,
  getCurrentMbps,
  getReadoutRate,
  getSamples,
  recordWindow,
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
    expect(getCurrent()).toBe(0)
    expect(getCurrentMbps()).toBe(0)
  })

  // The first bug: the store generated a plausible random signal, so the header
  // reported a speed the app had invented.
  it('reports a rate derived from the bytes it was given', () => {
    recordWindow(1_000_000, 100)
    expect(getCurrent()).toBeCloseTo(10_000_000, -3)
    expect(getCurrentMbps()).toBeCloseTo(10, 1)
  })

  /*
   * The second bug, and the reason `recordWindow` takes two arguments.
   *
   * The backend measured bytes over 250 ms and this file divided them by its
   * own 120 ms tick, so a real 15.5 MB/s was displayed as 35.5 — over a link
   * whose measured ceiling is 22.7. A rate is a measurement over an interval,
   * and the interval has to travel with it.
   */
  it('uses the interval it was given, not the one it might have assumed', () => {
    recordWindow(4_000_000, 250)
    const quarterSecond = getCurrent()

    resetThroughput()
    recordWindow(4_000_000, 120)
    const eighthSecond = getCurrent()

    expect(quarterSecond).toBeCloseTo(16_000_000, -4)
    expect(eighthSecond).toBeGreaterThan(quarterSecond)
    // The exact ratio that was being displayed.
    expect(eighthSecond / quarterSecond).toBeCloseTo(250 / 120, 2)
  })

  it('never reports a rate the link could not produce', () => {
    // 22.7 MB/s is the measured ceiling. Reporting a window honestly can never
    // exceed what actually arrived in it.
    recordWindow(2_800_000, 125)
    expect(getCurrentMbps()).toBeLessThan(23)
  })

  it('ignores a trickle rather than showing a speed for nothing', () => {
    recordWindow((IDLE_FLOOR_RATE / 4) * 0.125, 125)
    expect(getCurrent()).toBe(0)
  })

  it('shows a rate that is only just above the floor', () => {
    recordWindow(IDLE_FLOOR_RATE * 2 * 0.125, 125)
    expect(getCurrent()).toBeGreaterThan(IDLE_FLOOR_RATE)
  })

  it('ignores nonsense rather than dividing by it', () => {
    recordWindow(1_000_000, 0)
    expect(getCurrent()).toBe(0)
    recordWindow(1_000_000, -50)
    expect(getCurrent()).toBe(0)
    recordWindow(-1_000_000, 125)
    expect(getCurrent()).toBe(0)
    recordWindow(Number.NaN, 125)
    expect(getCurrent()).toBe(0)
    recordWindow(1_000_000, Number.POSITIVE_INFINITY)
    expect(getCurrent()).toBe(0)

    for (const value of getSamples()) expect(value).toBeGreaterThanOrEqual(0)
  })

  it('notifies subscribers when a window arrives', () => {
    whileSubscribed((listener) => {
      recordWindow(1_000_000, 125)
      recordWindow(1_000_000, 125)
      expect(listener).toHaveBeenCalledTimes(2)
    })
  })

  // The backend stops sending once nothing is moving, so something local has to
  // close the trace out — otherwise a finished transfer leaves its last rate
  // frozen on screen.
  it('falls back to idle when the reports stop', () => {
    vi.useFakeTimers()
    whileSubscribed(() => {
      recordWindow(5_000_000, 125)
      expect(getCurrent()).toBeGreaterThan(0)

      vi.advanceTimersByTime(IDLE_AFTER_MS * 3)
      expect(getCurrent()).toBe(0)
    })
  })

  it('does not keep repainting once it has settled at idle', () => {
    vi.useFakeTimers()
    whileSubscribed((listener) => {
      recordWindow(5_000_000, 125)
      vi.advanceTimersByTime(IDLE_AFTER_MS * 2)
      const afterSettling = listener.mock.calls.length

      vi.advanceTimersByTime(IDLE_AFTER_MS * 20)
      expect(listener.mock.calls.length).toBe(afterSettling)
    })
  })

  it('stops its watchdog once the last subscriber leaves', () => {
    vi.useFakeTimers()
    const listener = vi.fn()
    subscribeThroughput(listener)()
    vi.advanceTimersByTime(IDLE_AFTER_MS * 5)
    expect(listener).not.toHaveBeenCalled()
  })

  // Two components subscribe in the real app (the canvas trace and the text
  // readout). If one unmounting killed the timer, the other would freeze.
  it('keeps working while any subscriber remains', () => {
    const a = vi.fn()
    const b = vi.fn()
    const unsubA = subscribeThroughput(a)
    const unsubB = subscribeThroughput(b)

    unsubA()
    recordWindow(1_000_000, 125)
    expect(b).toHaveBeenCalledTimes(1)
    unsubB()
  })

  it('reuses one buffer rather than allocating per sample', () => {
    const before = getSamples()
    for (let i = 0; i < 10; i += 1) recordWindow(1_000_000, 125)
    expect(getSamples()).toBe(before)
  })

  it('clears on demand, for when the connection drops', () => {
    recordWindow(9_000_000, 125)
    expect(getCurrent()).toBeGreaterThan(0)
    resetThroughput()
    for (const value of getSamples()) expect(value).toBe(0)
  })
})

describe('the readout', () => {
  /** Reports arriving the way the backend sends them: every eighth second. */
  function feed(bytesPerSecond: number, seconds: number): void {
    for (let i = 0; i < seconds * 8; i += 1) {
      vi.advanceTimersByTime(125)
      recordWindow(bytesPerSecond / 8, 125)
    }
  }

  it('reads the speed over the last two seconds', () => {
    vi.useFakeTimers()
    feed(20_000_000, 5)
    expect(getReadoutRate() / 1e6).toBeCloseTo(20, 0)
  })

  /** The panel and the title bar used to disagree because this was the last
   *  single report; it now moves with the link over a couple of seconds. */
  it('follows a change within the window rather than on the next report', () => {
    vi.useFakeTimers()
    feed(20_000_000, 5)
    feed(30_000_000, 3)
    expect(getReadoutRate() / 1e6).toBeCloseTo(30, 0)
  })

  it('counts a stall between reports as slowness', () => {
    vi.useFakeTimers()
    feed(20_000_000, 3)
    vi.advanceTimersByTime(1000)
    expect(getReadoutRate() / 1e6).toBeLessThan(15)
  })

  it('reads nothing once nothing has moved for the whole window', () => {
    vi.useFakeTimers()
    feed(20_000_000, 2)
    vi.advanceTimersByTime(2500)
    expect(getReadoutRate()).toBe(0)
  })

  it('measures a transfer younger than the window over its own life', () => {
    vi.useFakeTimers()
    feed(20_000_000, 0.5)
    expect(getReadoutRate() / 1e6).toBeCloseTo(20, 0)
  })

  it('is cleared with everything else', () => {
    vi.useFakeTimers()
    feed(20_000_000, 1)
    resetThroughput()
    expect(getReadoutRate()).toBe(0)
  })
})
