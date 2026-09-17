/**
 * Live throughput, measured from bytes that actually crossed the link.
 *
 * Two bugs have lived here, and both are worth naming because the shape of the
 * code is a reaction to them.
 *
 * The first: this file used to *generate* a plausible random signal so the
 * trace had something to draw before there was a protocol. Fine as a
 * placeholder, indefensible once the app was real — the header was reporting a
 * speed the software had invented.
 *
 * The second, subtler and worse, because the number looked believable: the
 * backend sampled its byte counter every 250 ms and sent the total, and this
 * file divided whatever had arrived by its own 120 ms tick. Every figure came
 * out about 2.1x too high — 15.5 MB/s was displayed as 35.5. **A rate is a
 * measurement over an interval, and the interval has to travel with it.** So
 * [`recordWindow`] takes both, and nothing here ever infers elapsed time from
 * when an event happened to arrive.
 *
 * The data deliberately lives outside React. An earlier version held it in
 * `useState` inside `App`, which re-rendered the entire application eight times
 * a second for a number in the corner. Now the canvas trace subscribes and
 * paints imperatively without re-rendering at all.
 */

const SAMPLE_COUNT = 48

/**
 * How long without a report before the trace falls back to idle.
 *
 * The backend stops sending once nothing is moving, so something local has to
 * close the trace out — otherwise a finished transfer would leave its last
 * rate frozen on screen.
 */
const IDLE_AFTER_MS = 400

/**
 * The rate below which the link reads as idle, in **bytes per second**.
 *
 * A rate, not a byte count. A few kilobytes arriving inside one short window
 * is a perfectly respectable speed.
 */
const IDLE_FLOOR_RATE = 16 * 1024

const samples = new Float64Array(SAMPLE_COUNT)
const listeners = new Set<() => void>()

let watchdog: ReturnType<typeof setInterval> | null = null
let lastReportAt = 0

function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function push(bytesPerSecond: number): void {
  // Shift left in place. No allocation per sample.
  samples.copyWithin(0, 1)
  samples[SAMPLE_COUNT - 1] =
    bytesPerSecond < IDLE_FLOOR_RATE ? 0 : bytesPerSecond
  for (const listener of listeners) listener()
}

/**
 * Records one measured window: bytes that crossed the link, and how long that
 * took. Both come from the same clock on the same side, which is the whole
 * point.
 */
export function recordWindow(bytes: number, millis: number): void {
  if (!Number.isFinite(bytes) || !Number.isFinite(millis)) return
  if (bytes < 0 || millis <= 0) return
  lastReportAt = now()
  push((bytes / millis) * 1000)
}

/** Clears the trace, for when the connection drops or a view resets. */
export function resetThroughput(): void {
  samples.fill(0)
  for (const listener of listeners) listener()
}

function start(): void {
  if (watchdog !== null) return
  lastReportAt = now()
  watchdog = setInterval(() => {
    // Only when the trace still shows something. A permanently idle app must
    // not repaint the sparkline forever for no reason.
    if (now() - lastReportAt < IDLE_AFTER_MS) return
    if (getCurrent() === 0) return
    push(0)
  }, IDLE_AFTER_MS)
}

function stop(): void {
  if (watchdog !== null && listeners.size === 0) {
    clearInterval(watchdog)
    watchdog = null
  }
}

/** Subscribes to changes. Returns an unsubscribe function. */
export function subscribeThroughput(listener: () => void): () => void {
  listeners.add(listener)
  start()
  return () => {
    listeners.delete(listener)
    stop()
  }
}

/** The live buffer, in bytes per second. Do not mutate; treat as read-only. */
export function getSamples(): Float64Array {
  return samples
}

/** Most recent sample, in bytes per second. */
export function getCurrent(): number {
  return samples[SAMPLE_COUNT - 1] ?? 0
}

/** Most recent sample, in MB/s, which is how it is displayed. */
export function getCurrentMbps(): number {
  return getCurrent() / 1e6
}

export { SAMPLE_COUNT, IDLE_FLOOR_RATE, IDLE_AFTER_MS }
