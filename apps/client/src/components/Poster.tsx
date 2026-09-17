/**
 * A poster, generated from the title.
 *
 * There is no artwork on the drive and nothing has been downloaded, so this
 * draws one. That is a deliberate answer rather than a placeholder waiting to
 * be replaced:
 *
 * - It needs no network, no API key, and tells no third party what is on
 *   somebody's drive — which matters, because a list of filenames is a list of
 *   what you watch.
 * - A wall of identical grey rectangles is genuinely harder to scan than a
 *   wall of distinct ones. Deriving the colours from the title means the same
 *   film looks the same every time, so the grid becomes memorable by position
 *   and shade even before you read a word.
 *
 * Monochrome, like everything else here: the hue is fixed and only lightness
 * moves, so a poster never fights the interface it sits in.
 */

/** A small, stable hash. Same title, same poster, on every machine. */
function hashOf(text: string): number {
  let hash = 2166136261
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

/** Up to two initials, for the watermark behind the title. */
function initialsOf(title: string): string {
  const words = title
    .split(/[\s:–-]+/)
    .filter((word) => /[a-z0-9]/i.test(word))
    .slice(0, 2)
  return words.map((word) => word[0]!.toUpperCase()).join('') || '?'
}

export function Poster({
  title,
  year,
}: {
  title: string
  year?: number
}): React.JSX.Element {
  const hash = hashOf(title.toLowerCase())

  // Two greys a fixed distance apart, so contrast is the same on every card
  // however the hash falls. 10–26% keeps every poster darker than the text
  // that sits on it.
  const top = 10 + (hash % 17)
  const bottom = Math.max(4, top - 7)
  const angle = 120 + (hash % 5) * 15
  // A faint cool or warm cast, barely there, so cards differ without colour
  // entering the palette properly.
  const tint = (hash >> 8) % 2 === 0 ? 220 : 30

  return (
    <div
      className="relative flex aspect-[2/3] w-full items-end overflow-hidden"
      style={{
        background: `linear-gradient(${angle}deg, hsl(${tint} 6% ${top}%), hsl(${tint} 4% ${bottom}%))`,
      }}
      aria-hidden
    >
      {/* The initials, large and very dim: texture rather than information.
          The title is written underneath the card in full. */}
      <span
        className="pointer-events-none absolute -right-2 -top-6 select-none font-display font-bold leading-none text-white/[0.055]"
        style={{ fontSize: '104px' }}
      >
        {initialsOf(title)}
      </span>

      {/* A hairline top edge, matching every other surface in the app. */}
      <span className="pointer-events-none absolute inset-0 shadow-[inset_0_1px_0_rgba(255,255,255,0.07)]" />

      <div className="relative w-full bg-gradient-to-t from-black/70 to-transparent px-3 pb-3 pt-8">
        <div className="line-clamp-3 text-[12.5px] font-semibold leading-snug tracking-tight text-text/90">
          {title}
        </div>
        {year !== undefined && (
          <div className="tnum mt-0.5 font-mono text-[10px] text-textFaint">{year}</div>
        )}
      </div>
    </div>
  )
}
