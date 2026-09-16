import { useEffect } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { ChevronLeft, ChevronRight, Download, Info, Star, X } from 'lucide-react'
import type { MediaItem } from '@/lib/mockMedia'
import { formatBytes } from '@/lib/utils'

/**
 * Photo viewer.
 *
 * Photos were briefly opening the video player, which gave them a scrubber, a
 * play button and a running timecode for something with no timeline at all.
 * A still image needs a different set of affordances: move between shots,
 * see when it was taken, star it, get it off the drive.
 */
export function ImageViewer({
  items,
  index,
  onIndexChange,
  onClose,
}: {
  items: MediaItem[]
  index: number | null
  onIndexChange: (index: number) => void
  onClose: () => void
}): React.JSX.Element {
  const item = index !== null ? items[index] : undefined

  useEffect(() => {
    if (index === null) return undefined
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
      if (e.key === 'ArrowRight' && index < items.length - 1) onIndexChange(index + 1)
      if (e.key === 'ArrowLeft' && index > 0) onIndexChange(index - 1)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [index, items.length, onIndexChange, onClose])

  return (
    <AnimatePresence>
      {item && index !== null && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          // Fully opaque. At 96% the sidebar and grid showed through, which
          // is exactly the distraction a photo viewer exists to remove.
          className="fixed inset-0 z-40 flex flex-col bg-[#060607]"
        >
          {/* Top bar */}
          <div className="flex h-12 shrink-0 items-center gap-3 px-4">
            <span className="text-[13px] font-medium text-text">{item.title}</span>
            <span className="tnum font-mono text-[11px] text-textFaint">
              {item.subtitle} · {formatBytes(item.size)}
            </span>
            <div className="flex-1" />
            <span className="tnum font-mono text-[11px] text-textFaint">
              {index + 1} / {items.length}
            </span>
            <ToolButton icon={Star} label="Star" filled={item.starred} />
            <ToolButton icon={Info} label="Details" />
            <ToolButton icon={Download} label="Download" />
            <ToolButton icon={X} label="Close" onClick={onClose} />
          </div>

          {/* The image itself. A generated tone stands in until real
              thumbnails exist. */}
          <div className="relative flex min-h-0 flex-1 items-center justify-center px-14 pb-6">
            <AnimatePresence mode="wait">
              <motion.div
                key={item.id}
                initial={{ opacity: 0, scale: 0.985 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.985 }}
                transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                className="relative h-full w-full max-w-[1100px] overflow-hidden rounded-lg border border-white/[0.07]"
                style={{
                  background: `linear-gradient(145deg, ${item.tone[0]} 0%, ${item.tone[1]} 100%)`,
                }}
              >
                <svg
                  viewBox="0 0 100 100"
                  className="absolute inset-0 h-full w-full opacity-[0.035]"
                  aria-hidden="true"
                  preserveAspectRatio="xMidYMid slice"
                >
                  <polygon
                    points="50,14 81,32 81,68 50,86 19,68 19,32"
                    fill="none"
                    stroke="#F4F4F5"
                    strokeWidth="1.5"
                  />
                </svg>
              </motion.div>
            </AnimatePresence>

            {index > 0 && (
              <NavArrow side="left" onClick={() => onIndexChange(index - 1)} />
            )}
            {index < items.length - 1 && (
              <NavArrow side="right" onClick={() => onIndexChange(index + 1)} />
            )}
          </div>

          {/* Filmstrip */}
          <div className="flex h-[76px] shrink-0 items-center gap-2 overflow-x-auto border-t border-white/[0.06] px-4">
            {items.map((thumb, i) => (
              <button
                key={thumb.id}
                onClick={() => onIndexChange(i)}
                aria-label={thumb.title}
                className={`h-12 w-12 shrink-0 overflow-hidden rounded border transition-all ${
                  i === index
                    ? 'border-basalt/70 opacity-100 ring-1 ring-basalt/30'
                    : 'border-white/[0.06] opacity-45 hover:opacity-80'
                }`}
                style={{
                  background: `linear-gradient(145deg, ${thumb.tone[0]} 0%, ${thumb.tone[1]} 100%)`,
                }}
              />
            ))}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

function NavArrow({
  side,
  onClick,
}: {
  side: 'left' | 'right'
  onClick: () => void
}): React.JSX.Element {
  const Icon = side === 'left' ? ChevronLeft : ChevronRight
  return (
    <button
      onClick={onClick}
      aria-label={side === 'left' ? 'Previous photo' : 'Next photo'}
      className={`absolute top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-full bg-black/45 text-textDim backdrop-blur transition-colors hover:bg-black/70 hover:text-text ${
        side === 'left' ? 'left-3' : 'right-3'
      }`}
    >
      <Icon size={18} />
    </button>
  )
}

function ToolButton({
  icon: Icon,
  label,
  filled,
  onClick,
}: {
  icon: typeof Star
  label: string
  filled?: boolean
  onClick?: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className="flex h-8 w-8 items-center justify-center rounded-md text-textDim transition-colors hover:bg-white/[0.07] hover:text-text"
    >
      <Icon size={15} className={filled ? 'fill-basalt text-basalt' : undefined} />
    </button>
  )
}
