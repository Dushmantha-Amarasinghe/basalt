import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MAX_BACKOFF, Poller, backoffDelay } from './poll'

describe('backoffDelay', () => {
  it('uses the plain interval while everything is fine', () => {
    expect(backoffDelay(0, 1_000)).toBe(1_000)
  })

  it('doubles for each consecutive failure', () => {
    expect(backoffDelay(1, 1_000)).toBe(2_000)
    expect(backoffDelay(2, 1_000)).toBe(4_000)
    expect(backoffDelay(3, 1_000)).toBe(8_000)
  })

  // A host that has been down for an hour should still be asked about once
  // every ten seconds, so it reappears promptly when it comes back.
  it('never waits longer than the cap', () => {
    expect(backoffDelay(50, 1_000)).toBe(MAX_BACKOFF)
    expect(backoffDelay(3, 60_000)).toBe(MAX_BACKOFF)
  })
})

describe('Poller', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('fetches immediately rather than waiting for the first interval', async () => {
    const fetch = vi.fn().mockResolvedValue(1)
    const onData = vi.fn()
    const poller = new Poller({ fetch, interval: 1_000, onData, onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)

    expect(onData).toHaveBeenCalledWith(1)
    poller.stop()
  })

  it('keeps polling on the interval', async () => {
    const fetch = vi.fn().mockResolvedValue(1)
    const poller = new Poller({ fetch, interval: 1_000, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(3_100)

    expect(fetch).toHaveBeenCalledTimes(4)
    poller.stop()
  })

  // Rule one: a slow answer must not have the next request pile up behind it.
  it('never has two calls in flight at once', async () => {
    let open = 0
    let peak = 0
    const fetch = vi.fn(async () => {
      open += 1
      peak = Math.max(peak, open)
      await new Promise((resolve) => setTimeout(resolve, 5_000))
      open -= 1
      return 1
    })
    const poller = new Poller({ fetch, interval: 100, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(20_000)

    expect(peak).toBe(1)
    poller.stop()
  })

  // Rule two: the black-window bug. One failure must not end the loop.
  it('carries on after a failure', async () => {
    const fetch = vi
      .fn()
      .mockRejectedValueOnce(new Error('not managed yet'))
      .mockResolvedValue('ready')
    const onData = vi.fn()
    const onError = vi.fn()
    const poller = new Poller({ fetch, interval: 1_000, onData, onError })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)
    expect(onError).toHaveBeenCalledTimes(1)
    expect(onData).not.toHaveBeenCalled()

    // Backed off to 2s, so nothing yet at 1s.
    await vi.advanceTimersByTimeAsync(1_000)
    expect(onData).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1_000)
    expect(onData).toHaveBeenCalledWith('ready')
    poller.stop()
  })

  it('returns to the normal interval once a call succeeds', async () => {
    const fetch = vi
      .fn()
      .mockRejectedValueOnce(new Error('one'))
      .mockRejectedValueOnce(new Error('two'))
      .mockResolvedValue('ready')
    const poller = new Poller({ fetch, interval: 1_000, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)
    expect(poller.failureCount).toBe(1)
    await vi.advanceTimersByTimeAsync(2_000)
    expect(poller.failureCount).toBe(2)
    await vi.advanceTimersByTimeAsync(4_000)
    expect(poller.failureCount).toBe(0)
    poller.stop()
  })

  // Rule three: an answer arriving after unmount must not be written anywhere.
  it('drops an answer that arrives after it was stopped', async () => {
    const onData = vi.fn()
    const onError = vi.fn()
    const fetch = vi.fn(
      () => new Promise((resolve) => setTimeout(() => resolve('late'), 1_000)),
    )
    const poller = new Poller({ fetch, interval: 1_000, onData, onError })

    poller.start()
    poller.stop()
    await vi.advanceTimersByTimeAsync(5_000)

    expect(onData).not.toHaveBeenCalled()
    expect(onError).not.toHaveBeenCalled()
  })

  it('schedules nothing more once stopped', async () => {
    const fetch = vi.fn().mockResolvedValue(1)
    const poller = new Poller({ fetch, interval: 100, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)
    poller.stop()
    const calls = fetch.mock.calls.length

    await vi.advanceTimersByTimeAsync(10_000)
    expect(fetch).toHaveBeenCalledTimes(calls)
  })

  it('refreshes on demand rather than waiting for the interval', async () => {
    const fetch = vi.fn().mockResolvedValue(1)
    const poller = new Poller({ fetch, interval: 10_000, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)
    expect(fetch).toHaveBeenCalledTimes(1)

    poller.refresh()
    await vi.advanceTimersByTimeAsync(0)
    expect(fetch).toHaveBeenCalledTimes(2)
    poller.stop()
  })

  // The subtle one: refreshing mid-call used to clear the pending timer while
  // the early return skipped rescheduling, and the loop stopped for good.
  it('keeps polling when a refresh lands during a call', async () => {
    const fetch = vi.fn(
      () => new Promise((resolve) => setTimeout(() => resolve(1), 500)),
    )
    const poller = new Poller({ fetch, interval: 1_000, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(100)
    poller.refresh()

    await vi.advanceTimersByTimeAsync(5_000)
    expect(fetch.mock.calls.length).toBeGreaterThan(2)
    poller.stop()
  })

  it('ignores a refresh after it was stopped', async () => {
    const fetch = vi.fn().mockResolvedValue(1)
    const poller = new Poller({ fetch, interval: 1_000, onData: vi.fn(), onError: vi.fn() })

    poller.start()
    await vi.advanceTimersByTimeAsync(0)
    poller.stop()
    poller.refresh()
    await vi.advanceTimersByTimeAsync(5_000)

    expect(fetch).toHaveBeenCalledTimes(1)
  })
})
