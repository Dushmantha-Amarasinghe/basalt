import { useEffect, useRef, useState } from 'react'

/**
 * A live throughput trace, drawn on canvas.
 *
 * This is the signature element of the app, and it is deliberately not
 * something Frostbyte has: a compression tool runs a job and finishes, so a
 * static progress bar says everything. A NAS is a connection you leave open,
 * and the thing you want to feel at a glance is whether data is moving.
 *
 * Canvas rather than React: this repaints several times a second forever, and
 * re-rendering a component tree at that rate would burn frames the file list
 * needs. The canvas is written to directly and React never sees the updates.
 */
export function Sparkline({
  samples,
  width = 64,
  height = 16,
  className,
}: {
  samples: number[]
  width?: number
  height?: number
  className?: string
}): React.JSX.Element {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = window.devicePixelRatio || 1
    canvas.width = width * dpr
    canvas.height = height * dpr
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, width, height)

    if (samples.length < 2) return

    // Scale to the window's own peak so quiet periods still show shape, with a
    // floor so a flat idle line does not get amplified into noise.
    const peak = Math.max(1, ...samples)
    const step = width / (samples.length - 1)
    const y = (v: number): number => height - (v / peak) * (height - 2) - 1

    // Filled area underneath, fading downward.
    const gradient = ctx.createLinearGradient(0, 0, 0, height)
    gradient.addColorStop(0, 'rgba(244,244,245,0.22)')
    gradient.addColorStop(1, 'rgba(244,244,245,0)')

    ctx.beginPath()
    ctx.moveTo(0, height)
    samples.forEach((v, i) => ctx.lineTo(i * step, y(v)))
    ctx.lineTo(width, height)
    ctx.closePath()
    ctx.fillStyle = gradient
    ctx.fill()

    // The trace itself.
    ctx.beginPath()
    samples.forEach((v, i) => {
      if (i === 0) ctx.moveTo(0, y(v))
      else ctx.lineTo(i * step, y(v))
    })
    ctx.strokeStyle = 'rgba(244,244,245,0.75)'
    ctx.lineWidth = 1
    ctx.lineJoin = 'round'
    ctx.stroke()

    // A dot on the leading edge, so the eye has something to track.
    const last = samples[samples.length - 1]!
    ctx.beginPath()
    ctx.arc(width - 1, y(last), 1.6, 0, Math.PI * 2)
    ctx.fillStyle = '#F4F4F5'
    ctx.fill()
  }, [samples, width, height])

  return (
    <canvas
      ref={ref}
      style={{ width, height }}
      className={className}
      aria-hidden="true"
    />
  )
}

/**
 * Simulated throughput until the host is connected.
 *
 * Shaped like real transfer traffic rather than a sine wave: mostly idle, with
 * bursts that ramp up and decay. A smooth wave would look synthetic, and the
 * point of this element is that it reads as genuinely live.
 */
export function useSimulatedThroughput(sampleCount = 48): number[] {
  const [samples, setSamples] = useState<number[]>(() =>
    new Array(sampleCount).fill(0),
  )

  useEffect(() => {
    // State carried between ticks. Refs are unnecessary: the interval closure
    // owns these and nothing else reads them.
    let remaining = 0
    let target = 0

    const id = setInterval(() => {
      if (remaining <= 0 && Math.random() < 0.18) {
        // Start a burst: a transfer beginning.
        remaining = 12 + Math.random() * 30
        target = 8 + Math.random() * 15
      }
      if (remaining > 0) {
        remaining -= 1
      } else {
        // Decay toward idle rather than dropping to zero, the way a real
        // connection tails off.
        target *= 0.82
      }

      const jitter = (Math.random() - 0.5) * 2.5
      setSamples((prev) => [...prev.slice(1), Math.max(0, target + jitter)])
    }, 120)

    return () => clearInterval(id)
  }, [])

  return samples
}
