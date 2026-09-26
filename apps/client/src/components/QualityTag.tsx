import { isHighQuality, type Quality } from '@/lib/quality'
import { cn } from '@/lib/utils'

const TITLES: Record<Quality, string> = {
  SD: 'Standard definition',
  HD: 'HD · 720p',
  FHD: 'Full HD · 1080p',
  '2K': '2K · 1440p',
  '4K': '4K Ultra HD · 2160p',
  '8K': '8K · 4320p',
}

/**
 * HD, FHD, 2K or 4K, as a small tag.
 *
 * 4K and 8K are drawn light on dark-filled, everything else dark with a
 * hairline — the tag people scan a library for stands out without the rest
 * turning into a row of stickers.
 */
export function QualityTag({
  quality,
  size = 'md',
  className,
}: {
  quality: Quality | null
  /** `md` over a poster, `sm` inline in a row of text. */
  size?: 'sm' | 'md'
  className?: string
}): React.JSX.Element | null {
  if (!quality) return null
  const high = isHighQuality(quality)
  return (
    <span
      title={TITLES[quality]}
      // Centred on the capitals, not the line. A line of text keeps room
      // below for letters that hang under it, which "4K" has none of, so
      // centring the line left the letters sitting high in the chip. The
      // text box is trimmed to the capitals' own height and the chip is a
      // fixed height around it, so what is centred is exactly the ink. The
      // trim is on an inner block because it does not apply to a flex
      // container's own text.
      className={cn(
        'inline-flex shrink-0 items-center justify-center rounded-[4px] font-mono font-semibold leading-none tracking-[0.06em]',
        size === 'md' ? 'h-[17px] px-[6px] text-[9.5px]' : 'h-[14px] px-1 text-[8.5px]',
        high
          ? 'bg-basalt text-ink'
          : 'bg-black/70 text-text ring-1 ring-inset ring-white/15',
        className,
      )}
    >
      <span className="block [text-box:trim-both_cap_alphabetic]">{quality}</span>
    </span>
  )
}
