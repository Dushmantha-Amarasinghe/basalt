import { describe, expect, it, vi } from 'vitest'

/*
 * These test the *shape* of the async subscribe/unsubscribe dance directly,
 * without React. The bug they guard against needed no rendering to exist: it
 * was that a cleanup running before the registration resolved could not undo
 * it, so every listener registered that way lived forever.
 *
 * What that looked like in the app: dragging one file into the window started
 * eight simultaneous uploads of it, one per listener accumulated since launch.
 */

/** The broken pattern, kept so the test can show it failing. */
function subscribeNaively(register: () => Promise<() => void>): () => void {
  let stop: (() => void) | undefined
  void register().then((fn) => {
    stop = fn
  })
  return () => stop?.()
}

/** The pattern `useAsyncSubscription` implements. */
function subscribeSafely(register: () => Promise<() => void>): () => void {
  let stop: (() => void) | undefined
  let cancelled = false
  void register().then((fn) => {
    if (cancelled) {
      fn()
      return
    }
    stop = fn
  })
  return () => {
    cancelled = true
    // Cleared as it is called, so unsubscribing twice is harmless.
    const fn = stop
    stop = undefined
    fn?.()
  }
}

/** A registration that resolves later, like Tauri's `listen`. */
function deferredRegistration(): {
  register: () => Promise<() => void>
  resolveAll: () => Promise<void>
  live: () => number
} {
  let live = 0
  const pending: (() => void)[] = []

  return {
    register: () =>
      new Promise((resolve) => {
        pending.push(() => {
          live += 1
          resolve(() => {
            live -= 1
          })
        })
      }),
    resolveAll: async () => {
      while (pending.length > 0) pending.shift()!()
      // Let the `.then` callbacks run.
      await Promise.resolve()
      await Promise.resolve()
    },
    live: () => live,
  }
}

describe('the naive pattern', () => {
  it('leaks a listener when cleanup beats the registration', async () => {
    const reg = deferredRegistration()

    // Subscribe and immediately tear down, as a re-rendering effect does.
    const cleanup = subscribeNaively(reg.register)
    cleanup()
    await reg.resolveAll()

    // This is the bug, demonstrated.
    expect(reg.live()).toBe(1)
  })

  it('leaks one listener per render', async () => {
    const reg = deferredRegistration()
    for (let render = 0; render < 8; render += 1) {
      const cleanup = subscribeNaively(reg.register)
      cleanup()
    }
    await reg.resolveAll()

    // Eight listeners, so one drop becomes eight uploads.
    expect(reg.live()).toBe(8)
  })
})

describe('useAsyncSubscription pattern', () => {
  it('undoes a registration that arrives after cleanup', async () => {
    const reg = deferredRegistration()
    const cleanup = subscribeSafely(reg.register)
    cleanup()
    await reg.resolveAll()

    expect(reg.live()).toBe(0)
  })

  it('leaves nothing behind however many times it re-subscribes', async () => {
    const reg = deferredRegistration()
    for (let render = 0; render < 20; render += 1) {
      const cleanup = subscribeSafely(reg.register)
      cleanup()
    }
    await reg.resolveAll()

    expect(reg.live()).toBe(0)
  })

  it('keeps exactly one listener while it is still mounted', async () => {
    const reg = deferredRegistration()
    const cleanup = subscribeSafely(reg.register)
    await reg.resolveAll()

    expect(reg.live()).toBe(1)
    cleanup()
    expect(reg.live()).toBe(0)
  })

  it('unsubscribes normally when the registration resolved first', async () => {
    const reg = deferredRegistration()
    const cleanup = subscribeSafely(reg.register)
    await reg.resolveAll()
    cleanup()
    await reg.resolveAll()

    expect(reg.live()).toBe(0)
  })

  it('tolerates cleanup being called twice', async () => {
    const reg = deferredRegistration()
    const cleanup = subscribeSafely(reg.register)
    await reg.resolveAll()

    cleanup()
    expect(() => cleanup()).not.toThrow()
    expect(reg.live()).toBe(0)
  })
})

describe('the consequence, as the user saw it', () => {
  it('delivers one event to one handler rather than one per render', async () => {
    const handlers = new Set<() => void>()
    const register = (handler: () => void) => {
      handlers.add(handler)
      return Promise.resolve(() => {
        handlers.delete(handler)
      })
    }

    const onDrop = vi.fn()
    // Twenty renders of a component that subscribes correctly.
    let cleanup = (): void => {}
    for (let render = 0; render < 20; render += 1) {
      cleanup()
      cleanup = subscribeSafely(() => register(onDrop))
      await Promise.resolve()
    }

    // One drop.
    for (const handler of handlers) handler()

    expect(handlers.size).toBe(1)
    expect(onDrop).toHaveBeenCalledTimes(1)
    cleanup()
  })
})
