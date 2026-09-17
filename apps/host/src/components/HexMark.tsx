/**
 * The Basalt mark: interlocking hexagonal columns.
 *
 * Cooling basalt fractures into hexagonal columns — the Giant's Causeway
 * pattern. It reads as stacked storage, and it is the sibling of Frostbyte's
 * snowflake: same monochrome treatment, same weight, different mineral.
 *
 * Drawn with `currentColor` so it inherits whatever the surrounding text is,
 * rather than carrying a colour of its own.
 */
export function HexMark({
  size = 16,
  className,
}: {
  size?: number
  className?: string
}): React.JSX.Element {
  // A flat-top hexagon of radius 1 centred on the origin.
  const hex = (cx: number, cy: number, r: number): string => {
    const points: string[] = []
    for (let i = 0; i < 6; i += 1) {
      const angle = (Math.PI / 3) * i
      points.push(`${(cx + r * Math.cos(angle)).toFixed(3)},${(cy + r * Math.sin(angle)).toFixed(3)}`)
    }
    return points.join(' ')
  }

  // Three columns packed the way basalt actually fractures: one above, two
  // below, sharing edges.
  const r = 5.2
  const dx = r * 1.5
  const dy = r * Math.sqrt(3) * 0.5

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
      aria-hidden="true"
    >
      <polygon
        points={hex(12, 12 - dy, r)}
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
        opacity="0.95"
      />
      <polygon
        points={hex(12 - dx, 12 + dy, r)}
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
        opacity="0.55"
      />
      <polygon
        points={hex(12 + dx, 12 + dy, r)}
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
        opacity="0.55"
      />
    </svg>
  )
}
