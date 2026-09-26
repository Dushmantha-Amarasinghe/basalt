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
      className={cn(
        'inline-flex shrink-0 items-center rounded-[4px] font-mono font-semibold leading-none tracking-[0.06em]',
        size === 'md' ? 'px-1.5 py-[3px] text-[9px]' : 'px-1 py-[2px] text-[8.5px]',
        high
          ? 'bg-basalt text-ink'
          : 'bg-black/70 text-text ring-1 ring-inset ring-white/15',
        className,
      )}
    >
      {quality}
    </span>
  )
}
