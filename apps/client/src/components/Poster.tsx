import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { api } from '@/lib/api'


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

/**
 * Posters already fetched, so scrolling back up costs nothing.
 *
 * Module-level rather than React state: the same film appears on the grid and
 * again in the series sheet, and both should draw from one fetch. Entries are
 * data URLs of about 50 KB, so a library of a thousand would be 50 MB — which
 * is why only what has actually been looked at is ever in here.
 */
const fetched = new Map<string, string | null>()

/** Fetches a poster once, however many components ask for it. */
function useArtwork(id: string | undefined, hasArt: boolean): string | null {
  const [url, setUrl] = useState<string | null>(() =>
    id ? (fetched.get(id) ?? null) : null,
  )

  useEffect(() => {
    // Asking for a poster the host has already said it does not have is a
    // round trip guaranteed to come back empty.
    if (!id || !hasArt || fetched.has(id)) return

    let live = true
    void api
      .art(id)
      .then((data) => {
        fetched.set(id, data)
        if (live) setUrl(data)
      })
      // A poster that will not load is not worth a message anywhere; the
      // generated one takes over.
      .catch(() => fetched.set(id, null))

    return () => {
      live = false
    }
  }, [id, hasArt])

  return url
}

/**
 * A poster: the real artwork when the host has it, otherwise one drawn from
 * the title.
 *
 * The generated one is a deliberate answer rather than a placeholder, and it
 * is what everyone sees until a TMDb key is supplied on the host:
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
export function Poster({
  title,
  year,
  id,
  hasArt = false,
}: {
  title: string
  year?: number
  /** The library id, when this is a library item. */
  id?: string
  /** Whether the host has a poster for it. */
  hasArt?: boolean
}): React.JSX.Element {
  const artwork = useArtwork(id, hasArt)
  const hash = hashOf(title.toLowerCase())

  if (artwork) {
    return (
      <motion.img
        src={artwork}
        alt=""
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.25 }}
        className="aspect-[2/3] w-full object-cover"
        draggable={false}
      />
    )
  }

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
