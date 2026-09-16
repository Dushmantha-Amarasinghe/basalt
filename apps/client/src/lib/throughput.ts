/**
 * Live throughput samples, deliberately kept **outside** React.
 *
 * The first version held these in `useState` inside `App`. Every tick — eight
 * times a second, forever — re-rendered the entire application: the sidebar,
 * the toolbar, the file list's callbacks, and a 100,000-element array. That is
 * the whole explanation for the interface feeling sluggish; nothing was slow,
 * it was simply doing all of its work over and over for a number in the corner.
 *
 * Now the data lives in a module-level buffer with a subscription list.
 * Consumers that can paint imperatively (the canvas trace) never re-render at
 * all, and the one component that must show text re-renders alone.
 */

const SAMPLE_COUNT = 48
const TICK_MS = 120

const samples = new Float64Array(SAMPLE_COUNT)
const listeners = new Set<() => void>()

let timer: ReturnType<typeof setInterval> | null = null
let remaining = 0
let target = 0

function tick(): void {
  // Shaped like real transfer traffic: mostly idle, with bursts that ramp and
  // decay. A smooth wave would read as synthetic, and the point of this
  // element is that it looks genuinely live.
  if (remaining <= 0 && Math.random() < 0.18) {
    remaining = 12 + Math.random() * 30
    target = 8 + Math.random() * 15
  }
  if (remaining > 0) remaining -= 1
  else target *= 0.82

  const jitter = (Math.random() - 0.5) * 2.5
  const next = Math.max(0, target + jitter)

  // Shift left in place. No allocation per tick.
  samples.copyWithin(0, 1)
  samples[SAMPLE_COUNT - 1] = next

  for (const listener of listeners) listener()
}

function start(): void {
  if (timer === null) timer = setInterval(tick, TICK_MS)
}

function stop(): void {
  if (timer !== null && listeners.size === 0) {
    clearInterval(timer)
    timer = null
  }
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

/** The live buffer. Do not mutate; treat as read-only. */
export function getSamples(): Float64Array {
  return samples
}

/** Most recent sample, in MB/s. */
export function getCurrent(): number {
  return samples[SAMPLE_COUNT - 1] ?? 0
}

export { SAMPLE_COUNT }
