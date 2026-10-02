// @vitest-environment jsdom
//
// The app opened while its host was not sharing. It used to try once, at
// startup, and stop: the window showed an empty folder for good, and sharing
// again on the host changed nothing until the app was restarted. These follow
// it through to the host coming back, and the folder with it.

import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const host = vi.hoisted(() => ({
  /** Attempts to connect that fail before the host is sharing again. */
  downFor: 0,
  attempts: 0,
  listed: 0,
  /** The app was connected when the host went: its status says so still. */
  wasConnected: false,
  /** The next listing hangs for five seconds, then fails as unanswered. */
  hangNext: false,
}))

const offline = {
  connected: false,
  connecting: false,
  hostId: 'host-a',
  hostName: 'Laptop',
  vault: 'Films drive',
  writable: false,
  address: null,
  hasPaired: true,
  deviceName: 'Phone',
}
const online = { ...offline, connected: true, writable: true, address: '10.0.0.2:7742' }

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api')>()
  return {
    ...actual,
    onStatus: () => Promise.resolve(() => {}),
    onRemoved: () => Promise.resolve(() => {}),
    api: {
      ...actual.api,
      status: () =>
        Promise.resolve(host.wasConnected || host.attempts > host.downFor ? online : offline),
      connectSaved: () => {
        host.attempts += 1
        return host.attempts > host.downFor
          ? Promise.resolve(online)
          : Promise.reject(new actual.ApiError('offline', 'the host did not answer'))
      },
      list: () => {
        host.listed += 1
        if (host.hangNext) {
          host.hangNext = false
          return new Promise((_, reject) =>
            setTimeout(() => reject(new actual.ApiError('offline', 'did not answer')), 5_000),
          )
        }
        if (host.wasConnected && host.attempts <= host.downFor) {
          return Promise.reject(new actual.ApiError('offline', 'connection reset'))
        }
        return Promise.resolve([
          { name: 'Holiday 2026.mp4', kind: 'file', size: 10, mtime: 1, readonly: false },
        ])
      },
      space: () => Promise.resolve([1, 2]),
    },
  }
})

import { isWaiting, useVault, waitingLabel } from './useVault'

beforeEach(() => {
  host.downFor = 0
  host.attempts = 0
  host.listed = 0
  host.wasConnected = false
  host.hangNext = false
  vi.useFakeTimers({ shouldAdvanceTime: true })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('a host that was not sharing when the app opened', () => {
  it('is tried again until it answers, and its folder appears', async () => {
    host.downFor = 3
    const { result } = renderHook(() => useVault())

    await waitFor(() => expect(result.current.status?.hasPaired).toBe(true))
    expect(result.current.error?.kind).toBe('offline')

    // Each failure schedules the next attempt, not just the first.
    for (let i = 0; i < 6 && !result.current.status?.connected; i++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000)
      })
    }

    expect(host.attempts).toBe(4)
    await waitFor(() => expect(result.current.status?.connected).toBe(true))
    await waitFor(() => expect(result.current.entries.map((e) => e.name)).toEqual(['Holiday 2026.mp4']))
    expect(result.current.error).toBeNull()
  })

  it('never waits long between attempts', async () => {
    host.downFor = 100
    const { result } = renderHook(() => useVault())
    await waitFor(() => expect(result.current.error?.kind).toBe('offline'))
    for (let second = 0; second < 60; second++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000)
      })
    }
    // Two, four, eight, then every ten seconds: about eight in a minute.
    expect(host.attempts).toBeGreaterThanOrEqual(6)
  })
})

describe('a host lost while the app was using it', () => {
  it('is retried until it answers, though the status says connected throughout', async () => {
    host.wasConnected = true
    host.downFor = 3
    const { result } = renderHook(() => useVault())
    await waitFor(() => expect(result.current.error?.kind).toBe('offline'))
    for (let second = 0; second < 40 && host.attempts <= host.downFor; second++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000)
      })
    }
    expect(host.attempts).toBe(4)
    await waitFor(() => expect(result.current.error).toBeNull())
    await waitFor(() => expect(result.current.entries.map((e) => e.name)).toEqual(['Holiday 2026.mp4']))
  })
})

describe('a request that was waiting when the host came back', () => {
  it('does not say the host is unreachable once it has been reached', async () => {
    // Down at startup; the first retry reaches it.
    host.downFor = 0
    const { result } = renderHook(() => useVault())
    await waitFor(() => expect(result.current.error?.kind).toBe('offline'))
    // A listing asked for while the host is away, left waiting.
    host.hangNext = true
    act(() => result.current.refresh())
    // The host is reached in the meantime...
    for (let second = 0; second < 4 && !result.current.status?.connected; second++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000)
      })
    }
    await waitFor(() => expect(result.current.status?.connected).toBe(true))
    await waitFor(() => expect(result.current.error).toBeNull())
    // ...and only then does the old listing give up.
    for (let second = 0; second < 6; second++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000)
      })
      expect(result.current.error).toBeNull()
    }
  })
})

describe('two reasons to retry at once', () => {
  it('make one attempt, not two racing each other', async () => {
    host.downFor = 100
    const { result } = renderHook(() => useVault())
    await waitFor(() => expect(result.current.error?.kind).toBe('offline'))
    const before = host.attempts
    await act(async () => {
      void result.current.reconnect()
      void result.current.reconnect()
      void result.current.reconnect()
    })
    expect(host.attempts).toBe(before + 1)
  })
})

describe('choosing another drive while waiting', () => {
  it('stops the retries for the one left behind', async () => {
    host.downFor = 100
    const { result } = renderHook(() => useVault())
    await waitFor(() => expect(result.current.error?.kind).toBe('offline'))
    act(() => result.current.switchTo({ ...online, hostId: 'host-b', vault: 'Other drive' }))
    const before = host.attempts
    for (let second = 0; second < 30; second++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000)
      })
    }
    expect(result.current.error).toBeNull()
    expect(host.attempts).toBe(before)
  })
})

describe('what the window says meanwhile', () => {
  it('is waiting only once an attempt has failed, not while one runs', () => {
    expect(isWaiting(offline)).toBe(true)
    expect(isWaiting({ ...offline, connecting: true })).toBe(false)
    expect(isWaiting(online)).toBe(false)
    expect(isWaiting({ ...offline, hasPaired: false, hostId: null })).toBe(false)
  })

  it('names the drive and the computer it is waiting for', () => {
    expect(waitingLabel(offline)).toBe(
      'Waiting for Films drive. It opens here by itself as soon as Laptop is sharing again.',
    )
  })
})
