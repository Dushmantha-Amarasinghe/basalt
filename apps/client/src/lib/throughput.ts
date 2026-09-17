/**
 * Live throughput, measured from bytes that actually crossed the link.
 *
 * The first version of this file generated a plausible-looking random signal so
 * the trace had something to draw before there was a protocol. That was fine as
 * a placeholder and indefensible once the app was real: the header and the
 * sidebar were reporting a speed the software had invented. Every number here
 * now comes from [`recordBytes`], which is called with the exact byte counts
 * reported by a transfer in progress.
 *
 * The data deliberately lives **outside** React. An earlier version held it in
 * `useState` inside `App`, which re-rendered the entire application eight times
 * a second — the sidebar, the toolbar, and a 100,000-element array — for a
 * number in the corner. Now the canvas trace subscribes and paints imperatively
 * without re-rendering at all, and the one component that must show text
 * re-renders alone.
 */

const SAMPLE_COUNT = 48
const TICK_MS = 120

/**
 * The rate below which the link reads as idle, in **bytes per second**.
 *
 * A rate, not a byte count — the distinction matters, because a few kilobytes
 * arriving inside one 120 ms tick is a perfectly respectable 34 KB/s.
 */
const IDLE_FLOOR_RATE = 16 * 1024

const samples = new Float64Array(SAMPLE_COUNT)
const listeners = new Set<() => void>()

let timer: ReturnType<typeof setInterval> | null = null
/** Bytes recorded since the last tick. */
let pending = 0
let lastTickAt = 0

/** Now, from whichever clock is available. Tests run without `performance`. */
function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function tick(): void {
  const at = now()
  // Real elapsed time rather than the nominal interval: a busy main thread or a
  // backgrounded window can delay the timer considerably, and dividing by 120 ms
  // regardless would report a speed several times higher than the truth.
  const elapsed = Math.max(1, at - lastTickAt)
  lastTickAt = at

  const bytesPerSecond = (pending / elapsed) * 1000
  pending = 0

  // Shift left in place. No allocation per tick.
  samples.copyWithin(0, 1)
  samples[SAMPLE_COUNT - 1] = bytesPerSecond < IDLE_FLOOR_RATE ? 0 : bytesPerSecond

  for (const listener of listeners) listener()
}

function start(): void {
  if (timer === null) {
    lastTickAt = now()
    timer = setInterval(tick, TICK_MS)
  }
}

function stop(): void {
  if (timer !== null && listeners.size === 0) {
    clearInterval(timer)
    timer = null
  }
}

/**
 * Records bytes that have crossed the link.
 *
 * Called with a delta, not a total. Negative values are ignored rather than
 * subtracted: a resumed transfer restarts its count from the resume offset, and
 * treating that as negative throughput would show the trace dipping below zero.
 */
export function recordBytes(delta: number): void {
  if (!Number.isFinite(delta) || delta <= 0) return
  pending += delta
}

/** Clears the trace, for when the connection drops or a view resets. */
export function resetThroughput(): void {
  samples.fill(0)
  pending = 0
  for (const listener of listeners) listener()
}

/** Subscribes to ticks. Returns an unsubscribe function. */
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

export { SAMPLE_COUNT, TICK_MS, IDLE_FLOOR_RATE }
