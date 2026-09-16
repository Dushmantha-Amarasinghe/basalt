import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  SAMPLE_COUNT,
  getCurrent,
  getSamples,
  subscribeThroughput,
} from './throughput'

const TICK_MS = 120

afterEach(() => {
  vi.useRealTimers()
})

describe('throughput store', () => {
  it('starts with a full buffer, so the sparkline never reads past its end', () => {
    expect(getSamples()).toHaveLength(SAMPLE_COUNT)
  })

  it('notifies subscribers on every tick', () => {
    vi.useFakeTimers()
    const listener = vi.fn()
    const unsubscribe = subscribeThroughput(listener)

    vi.advanceTimersByTime(TICK_MS * 3)
    expect(listener).toHaveBeenCalledTimes(3)

    unsubscribe()
  })

  it('stops ticking once the last subscriber leaves', () => {
    vi.useFakeTimers()
    const listener = vi.fn()
    subscribeThroughput(listener)()

    vi.advanceTimersByTime(TICK_MS * 5)
    expect(listener).not.toHaveBeenCalled()
  })

  // Two components subscribe in the real app (the canvas trace and the text
  // readout). If one unmounting killed the timer, the other would silently
  // freeze.
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
    const unsubscribe = subscribeThroughput(() => {})

    vi.advanceTimersByTime(TICK_MS * 10)
    expect(getSamples()).toBe(before)

    unsubscribe()
  })

  it('never produces a negative rate', () => {
    vi.useFakeTimers()
    const unsubscribe = subscribeThroughput(() => {})

    vi.advanceTimersByTime(TICK_MS * 400)
    for (const value of getSamples()) expect(value).toBeGreaterThanOrEqual(0)

    unsubscribe()
  })

  it('reports the newest sample as the current rate', () => {
    vi.useFakeTimers()
    const unsubscribe = subscribeThroughput(() => {})

    vi.advanceTimersByTime(TICK_MS * 20)
    expect(getCurrent()).toBe(getSamples()[SAMPLE_COUNT - 1])

    unsubscribe()
  })
})
