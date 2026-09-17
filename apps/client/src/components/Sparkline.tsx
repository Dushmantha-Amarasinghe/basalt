import { useEffect, useRef, useState } from 'react'
import {
  IDLE_FLOOR_RATE,
  getCurrent,
  getCurrentMbps,
  getSamples,
  subscribeThroughput,
} from '@/lib/throughput'

/**
 * A live throughput trace, drawn on canvas.
 *
 * The signature element of the app, and deliberately not something Frostbyte
 * has: a compression tool runs a job and finishes, so a static progress bar
 * says everything. A NAS is a connection you leave open, and what you want to
 * feel at a glance is whether data is moving.
 *
 * **This component never re-renders.** It subscribes to the throughput store
 * and paints straight onto the canvas. React is not involved after mount, so
 * the trace updating eight times a second costs nothing elsewhere in the tree.
 */
export function Sparkline({
  width = 56,
  height = 14,
  className,
}: {
  width?: number
  height?: number
  className?: string
}): React.JSX.Element {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return undefined
    const ctx = canvas.getContext('2d')
    if (!ctx) return undefined

    const dpr = window.devicePixelRatio || 1
    canvas.width = width * dpr
    canvas.height = height * dpr
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)

    // Gradient is built once. Rebuilding it per frame is a surprisingly large
    // share of the cost of a small repeated paint.
    const gradient = ctx.createLinearGradient(0, 0, 0, height)
    gradient.addColorStop(0, 'rgba(244,244,245,0.22)')
    gradient.addColorStop(1, 'rgba(244,244,245,0)')

    let frame = 0
    let dirty = true

    const draw = (): void => {
      frame = 0
      if (!dirty) return
      dirty = false

      const samples = getSamples()
      ctx.clearRect(0, 0, width, height)
      if (samples.length < 2) return

      // Scale to the window's own peak so quiet periods still show shape. The
      // floor is a real rate rather than 1, because samples are now bytes per
      // second: without it an idle window of zeros would be divided by 1 and
      // any stray byte would spike to full height.
      let peak = IDLE_FLOOR_RATE
      for (const value of samples) if (value > peak) peak = value

      const step = width / (samples.length - 1)
      const y = (v: number): number => height - (v / peak) * (height - 2) - 1

      ctx.beginPath()
      ctx.moveTo(0, height)
      for (let i = 0; i < samples.length; i += 1) ctx.lineTo(i * step, y(samples[i]!))
      ctx.lineTo(width, height)
      ctx.closePath()
      ctx.fillStyle = gradient
      ctx.fill()

      ctx.beginPath()
      for (let i = 0; i < samples.length; i += 1) {
        const py = y(samples[i]!)
        if (i === 0) ctx.moveTo(0, py)
        else ctx.lineTo(i * step, py)
      }
      ctx.strokeStyle = 'rgba(244,244,245,0.75)'
      ctx.lineWidth = 1
      ctx.lineJoin = 'round'
      ctx.stroke()

      const last = samples[samples.length - 1]!
      ctx.beginPath()
      ctx.arc(width - 1, y(last), 1.6, 0, Math.PI * 2)
      ctx.fillStyle = '#F4F4F5'
      ctx.fill()
    }

    // Repaint on the next animation frame rather than inside the store tick,
    // so painting stays aligned with the compositor and coalesces if several
    // ticks land in one frame.
    const onTick = (): void => {
      dirty = true
      if (frame === 0) frame = requestAnimationFrame(draw)
    }

    draw()
    const unsubscribe = subscribeThroughput(onTick)

    return () => {
      unsubscribe()
      if (frame !== 0) cancelAnimationFrame(frame)
    }
  }, [width, height])

  return (
    <canvas ref={ref} style={{ width, height }} className={className} aria-hidden="true" />
  )
}

/**
 * The numeric readout.
 *
 * Isolated into its own component on purpose: it is the only thing that has to
 * re-render when throughput changes, so it re-renders alone instead of taking
 * the application with it.
 */
export function ThroughputReadout({
  className,
  idleLabel = 'idle',
}: {
  className?: string
  idleLabel?: string
}): React.JSX.Element {
  const [value, setValue] = useState(() => getCurrentMbps())

  useEffect(() => {
    // Text only needs to keep up with the eye, not the data. Updating a few
    // times a second reads as live while cutting re-renders by two thirds.
    let last = 0
    return subscribeThroughput(() => {
      const now = performance.now()
      if (now - last < 320) return
      last = now
      setValue(getCurrentMbps())
    })
  }, [])

  return (
    <span className={className}>
      {value > 0.05 ? `${value.toFixed(1)} MB/s` : idleLabel}
    </span>
  )
}

/**
 * True when the link has been moving data recently. Coarse on purpose.
 *
 * Drives the mark's breathing in the title bar, so it must be steady rather
 * than accurate: flickering on and off between chunks would be worse than not
 * animating at all.
 */
export function useIsActive(thresholdBytesPerSecond = IDLE_FLOOR_RATE): boolean {
  const [active, setActive] = useState(false)

  useEffect(() => {
    let last = 0
    return subscribeThroughput(() => {
      const now = performance.now()
      if (now - last < 600) return
      last = now
      // Any movement in the visible window counts, not just the latest sample.
      // A transfer between chunks reads as zero for an instant, and the mark
      // must not stutter every time that happens.
      const samples = getSamples()
      let recent = getCurrent()
      for (let i = samples.length - 8; i < samples.length; i += 1) {
        if (i >= 0) recent = Math.max(recent, samples[i]!)
      }
      setActive(recent > thresholdBytesPerSecond)
    })
  }, [thresholdBytesPerSecond])

  return active
}
