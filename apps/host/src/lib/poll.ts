/**
 * A polling loop that cannot leave the app staring at nothing.
 *
 * Three rules, each from a bug this project has already had:
 *
 * 1. **Never two calls at once.** The next request is scheduled when the last
 *    one finishes, not on a fixed timer, so a slow answer cannot pile up
 *    behind itself.
 * 2. **A failure is not the end.** The client once showed a black window
 *    forever because its first status call lost a race and nothing ever asked
 *    again. Failures here back off and keep trying.
 * 3. **A stopped poller is silent.** An answer arriving after the component
 *    unmounted is dropped rather than written into state that no longer
 *    exists.
 *
 * The logic lives in a plain class so tests can drive it without rendering
 * anything — the same reason `useAsyncSubscription` was testable in the client.
 */

/** The longest gap between retries, however badly things are going. */
export const MAX_BACKOFF = 10_000

/**
 * How long to wait after `failures` consecutive failures.
 *
 * Doubling, capped: something that is not answering should be asked less
 * often, but the cap means recovery is quick once it comes back rather than
 * the app sulking for a minute after a one-second hiccup.
 */
export function backoffDelay(failures: number, base: number): number {
  if (failures <= 0) return base
  return Math.min(base * 2 ** Math.min(failures, 6), MAX_BACKOFF)
}

export interface PollerOptions<T> {
  fetch: () => Promise<T>
  /** Gap between a finished call and the next one, in milliseconds. */
  interval: number
  onData: (value: T) => void
  onError: (error: unknown) => void
}

export class Poller<T> {
  private timer: ReturnType<typeof setTimeout> | null = null
  private inFlight = false
  private stopped = false
  private failures = 0

  constructor(private readonly options: PollerOptions<T>) {}

  /** Fetches immediately, then keeps going. */
  start(): void {
    this.stopped = false
    void this.tick()
  }

  stop(): void {
    this.stopped = true
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
  }

  /** Asks again now, for after an action that changed something. */
  refresh(): void {
    if (this.stopped) return
    // A call already on its way will deliver a fresh answer and reschedule;
    // interrupting it here would clear the pending timer and stop the loop.
    if (this.inFlight) return
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
    void this.tick()
  }

  /** Consecutive failures, for tests and for deciding what to show. */
  get failureCount(): number {
    return this.failures
  }

  private async tick(): Promise<void> {
    if (this.stopped || this.inFlight) return
    this.inFlight = true
    try {
      const value = await this.options.fetch()
      if (this.stopped) return
      this.failures = 0
      this.options.onData(value)
    } catch (error) {
      if (this.stopped) return
      this.failures += 1
      this.options.onError(error)
    } finally {
      this.inFlight = false
      this.schedule()
    }
  }

  private schedule(): void {
    if (this.stopped) return
    this.timer = setTimeout(() => {
      void this.tick()
    }, backoffDelay(this.failures, this.options.interval))
  }
}
