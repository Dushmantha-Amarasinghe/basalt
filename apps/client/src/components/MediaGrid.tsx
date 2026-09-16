import { motion } from 'framer-motion'
import { Play, Star } from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatDuration } from '@/lib/mockMedia'
import { cn, formatBytes } from '@/lib/utils'

type Shape = 'poster' | 'square'

/**
 * Poster grid for the library views.
 *
 * A deliberately different rhythm from the file list: the list is dense, mono
 * and information-first, this is spacious and image-first. Same palette, same
 * motion language, different job — which is what stops the library feeling
 * like the file browser with bigger icons.
 *
 * Tiles stagger in, but only the first two rows. Beyond that the delay would
 * outlast the user's patience, and anything offscreen is animating for nobody.
 */
export function MediaGrid({
  items,
  shape = 'poster',
  onOpen,
}: {
  items: MediaItem[]
  shape?: Shape
  onOpen?: (item: MediaItem, index: number) => void
}): React.JSX.Element {
  return (
    <div className="h-full overflow-y-auto px-5 py-4">
      <div
        className={cn(
          'grid gap-4',
          shape === 'poster'
            ? 'grid-cols-[repeat(auto-fill,minmax(150px,1fr))]'
            : 'grid-cols-[repeat(auto-fill,minmax(130px,1fr))]',
        )}
      >
        {items.map((item, index) => (
          <Tile
            key={item.id}
            item={item}
            shape={shape}
            index={index}
            onOpen={onOpen}
          />
        ))}
      </div>
    </div>
  )
}

function Tile({
  item,
  shape,
  index,
  onOpen,
}: {
  item: MediaItem
  shape: Shape
  index: number
  onOpen?: (item: MediaItem, index: number) => void
}): React.JSX.Element {
  return (
    <motion.button
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{
        duration: 0.32,
        delay: Math.min(index, 14) * 0.022,
        ease: [0.22, 1, 0.36, 1],
      }}
      whileHover={{ y: -3 }}
      onClick={() => onOpen?.(item, index)}
      className="group text-left"
    >
      <div
        className={cn(
          'relative overflow-hidden rounded-md border border-white/[0.07] transition-colors group-hover:border-white/20',
          shape === 'poster' ? 'aspect-[2/3]' : 'aspect-square',
        )}
        style={{
          background: `linear-gradient(145deg, ${item.tone[0]} 0%, ${item.tone[1]} 100%)`,
        }}
      >
        {/* Faint hex watermark, the same motif as the app mark. Ties the
            placeholder art to the identity instead of leaving flat rectangles. */}
        <HexWatermark />

        {item.starred && (
          <Star
            size={13}
            className="absolute right-2 top-2 fill-basalt text-basalt drop-shadow"
          />
        )}

        {item.duration !== undefined && (
          <span className="tnum absolute bottom-2 right-2 rounded bg-black/65 px-1.5 py-0.5 font-mono text-[10px] text-text backdrop-blur-sm">
            {formatDuration(item.duration)}
          </span>
        )}

        {/* Play affordance on hover. */}
        <div className="absolute inset-0 flex items-center justify-center bg-black/35 opacity-0 transition-opacity duration-200 group-hover:opacity-100">
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-basalt/90">
            <Play size={15} className="ml-0.5 fill-ink text-ink" />
          </span>
        </div>

        {/* Resume position, for anything part-watched. */}
        {item.progress !== undefined && (
          <div className="absolute inset-x-0 bottom-0 h-[3px] bg-black/50">
            <div
              className="h-full bg-basalt"
              style={{ width: `${item.progress * 100}%` }}
            />
          </div>
        )}
      </div>

      <div className="mt-2 px-0.5">
        <div className="truncate text-[13px] font-medium text-text group-hover:text-basalt">
          {item.title}
        </div>
        <div className="mt-0.5 flex items-baseline justify-between gap-2">
          <span className="truncate text-[11px] text-textFaint">{item.subtitle}</span>
          <span className="tnum shrink-0 font-mono text-[10px] text-textFaint">
            {formatBytes(item.size)}
          </span>
        </div>
      </div>
    </motion.button>
  )
}

/** Very low-contrast hexagon, echoing the app mark. */
function HexWatermark(): React.JSX.Element {
  return (
    <svg
      viewBox="0 0 100 100"
      className="absolute inset-0 h-full w-full opacity-[0.045]"
      aria-hidden="true"
      preserveAspectRatio="xMidYMid slice"
    >
      <polygon
        points="50,14 81,32 81,68 50,86 19,68 19,32"
        fill="none"
        stroke="#F4F4F5"
        strokeWidth="2"
      />
    </svg>
  )
}
