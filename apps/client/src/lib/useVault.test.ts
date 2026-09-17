import { describe, expect, it } from 'vitest'
import {
  STARTUP_ATTEMPTS_BEFORE_COMPLAINING,
  startupRetryDelay,
} from './useVault'

/*
 * The bug these guard against: the app asked the backend for its status once,
 * and if that single call failed it never asked again — leaving a dark window
 * with a faint mark in the middle, indistinguishable from a crash. The trigger
 * was a startup race in the backend, but the fault was that one failure could
 * strand the interface with no state and no way to get any.
 */

describe('startupRetryDelay', () => {
  it('retries almost immediately at first', () => {
    // The usual cause is the backend being a fraction of a second behind the
    // window, so the first retry has to be quick enough to be invisible.
    expect(startupRetryDelay(1)).toBe(100)
    expect(startupRetryDelay(2)).toBe(200)
  })

  it('backs off so a dead backend is not polled forever', () => {
    expect(startupRetryDelay(3)).toBe(400)
    expect(startupRetryDelay(5)).toBe(1600)
  })

  it('stops growing at a few seconds', () => {
    for (const attempt of [10, 50, 1000]) {
      expect(startupRetryDelay(attempt)).toBe(4000)
    }
  })

  it('never returns zero or a negative wait, whatever it is given', () => {
    for (const attempt of [-5, 0, 1, 7]) {
      expect(startupRetryDelay(attempt)).toBeGreaterThan(0)
    }
  })

  it('complains in words before the waits get long', () => {
    // Whatever the threshold is, the user must be told something before they
    // have sat looking at a blank window for seconds on end.
    let elapsed = 0
    for (let i = 1; i <= STARTUP_ATTEMPTS_BEFORE_COMPLAINING; i += 1) {
      elapsed += startupRetryDelay(i)
    }
    expect(elapsed).toBeLessThan(2000)
  })
})
