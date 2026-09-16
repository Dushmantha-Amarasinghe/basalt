import { useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  ArrowRight,
  Clock,
  CornerDownLeft,
  FolderOpen,
  Image as ImageIcon,
  Music,
  Search,
  Settings2,
  Star,
  Video,
} from 'lucide-react'
import type { Entry } from './FileList'
import { cn, formatBytes } from '@/lib/utils'
import type { NavKey } from './Sidebar'

interface Command {
  id: string
  label: string
  hint?: string
  icon: typeof FolderOpen
  run: () => void
}

/**
 * Ctrl+K palette.
 *
 * The one place in the app where motion is allowed to be theatrical. Everything
 * else is deliberately restrained — rows must not animate, the list must hold
 * 60 fps — so this carries the personality: it drops in with a spring, the
 * backdrop blurs behind it, and the selection glides between rows on a shared
 * layout transition.
 */
export function CommandPalette({
  open,
  onClose,
  entries,
  onNavigate,
  onOpenEntry,
}: {
  open: boolean
  onClose: () => void
  entries: Entry[]
  onNavigate: (key: NavKey) => void
  onOpenEntry: (entry: Entry) => void
}): React.JSX.Element {
  const [query, setQuery] = useState('')
  const [cursor, setCursor] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const navCommands: Command[] = useMemo(
    () => [
      { id: 'nav-files', label: 'Go to Files', icon: FolderOpen, run: () => onNavigate('files') },
      { id: 'nav-recent', label: 'Go to Recent', icon: Clock, run: () => onNavigate('recent') },
      { id: 'nav-starred', label: 'Go to Starred', icon: Star, run: () => onNavigate('starred') },
      { id: 'nav-videos', label: 'Go to Videos', icon: Video, run: () => onNavigate('videos') },
      { id: 'nav-music', label: 'Go to Music', icon: Music, run: () => onNavigate('music') },
      { id: 'nav-photos', label: 'Go to Photos', icon: ImageIcon, run: () => onNavigate('photos') },
      {
        id: 'nav-settings',
        label: 'Go to Settings',
        icon: Settings2,
        run: () => onNavigate('settings'),
      },
    ],
    [onNavigate],
  )

  // Files are searched too, but capped: the palette is for jumping somewhere,
  // not for browsing, and an unbounded list would make it feel like the main
  // view rather than a shortcut.
  const results = useMemo(() => {
    const needle = query.trim().toLowerCase()

    const commands = needle
      ? navCommands.filter((c) => c.label.toLowerCase().includes(needle))
      : navCommands.slice(0, 4)

    const files: Command[] = needle
      ? entries
          .filter((e) => e.name.toLowerCase().includes(needle))
          .slice(0, 8)
          .map((e) => ({
            id: `file-${e.id}`,
            label: e.name,
            hint: e.kind === 'dir' ? 'Folder' : formatBytes(e.size),
            icon: e.kind === 'dir' ? FolderOpen : Search,
            run: () => onOpenEntry(e),
          }))
      : []

    return [...commands, ...files]
  }, [query, navCommands, entries, onOpenEntry])

  // Reset whenever it opens, so it never reappears mid-search.
  useEffect(() => {
    if (open) {
      setQuery('')
      setCursor(0)
      // Focus after the entrance has begun, or the browser scrolls to it.
      const id = setTimeout(() => inputRef.current?.focus(), 40)
      return () => clearTimeout(id)
    }
    return undefined
  }, [open])

  useEffect(() => {
    setCursor(0)
  }, [query])

  useEffect(() => {
    if (!open) return undefined
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      } else if (e.key === 'ArrowDown') {
        e.preventDefault()
        setCursor((c) => Math.min(results.length - 1, c + 1))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setCursor((c) => Math.max(0, c - 1))
      } else if (e.key === 'Enter') {
        e.preventDefault()
        results[cursor]?.run()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, results, cursor, onClose])

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.14 }}
          className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[14vh] backdrop-blur-[2px]"
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, y: -12, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 460, damping: 32 }}
            onClick={(e) => e.stopPropagation()}
            className="w-[520px] overflow-hidden rounded-lg border border-white/[0.09] bg-panel2 shadow-lift"
          >
            <div className="flex items-center gap-3 border-b border-line px-4">
              <Search size={15} className="shrink-0 text-textFaint" />
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search files or jump to…"
                spellCheck={false}
                className="h-12 flex-1 bg-transparent text-[15px] text-text placeholder:text-textFaint focus:outline-none"
              />
              <kbd className="shrink-0 rounded border border-white/10 px-1.5 py-0.5 font-mono text-[10px] text-textFaint">
                ESC
              </kbd>
            </div>

            <div className="max-h-[320px] overflow-y-auto p-1.5">
              {results.length === 0 ? (
                <div className="px-3 py-8 text-center text-sm text-textFaint">
                  Nothing matches “{query}”
                </div>
              ) : (
                results.map((item, index) => (
                  <PaletteRow
                    key={item.id}
                    item={item}
                    active={index === cursor}
                    onHover={() => setCursor(index)}
                    onRun={() => {
                      item.run()
                      onClose()
                    }}
                  />
                ))
              )}
            </div>

            <div className="flex items-center gap-3 border-t border-line px-3 py-2 font-mono text-[10px] text-textFaint">
              <Hint keys="↑↓" label="navigate" />
              <Hint keys="↵" label="open" icon={CornerDownLeft} />
              <div className="flex-1" />
              <span className="tnum">{results.length} results</span>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

function PaletteRow({
  item,
  active,
  onHover,
  onRun,
}: {
  item: Command
  active: boolean
  onHover: () => void
  onRun: () => void
}): React.JSX.Element {
  const Icon = item.icon
  return (
    <button
      onMouseMove={onHover}
      onClick={onRun}
      className="relative flex w-full items-center gap-3 rounded px-3 py-2 text-left text-sm"
    >
      {/* Shared layout highlight, the same trick as the sidebar: the selection
          slides between rows rather than blinking on and off. */}
      {active && (
        <motion.span
          layoutId="palette-active"
          className="absolute inset-0 rounded bg-white/[0.07] ring-1 ring-inset ring-white/[0.12]"
          transition={{ type: 'spring', stiffness: 620, damping: 40 }}
        />
      )}
      <Icon
        size={15}
        className={cn('relative z-10 shrink-0', active ? 'text-basalt' : 'text-textFaint')}
      />
      <span className={cn('relative z-10 flex-1 truncate', active ? 'text-text' : 'text-textDim')}>
        {item.label}
      </span>
      {item.hint && (
        <span className="relative z-10 shrink-0 font-mono text-[10px] text-textFaint">
          {item.hint}
        </span>
      )}
      {active && <ArrowRight size={13} className="relative z-10 shrink-0 text-textFaint" />}
    </button>
  )
}

function Hint({
  keys,
  label,
  icon: Icon,
}: {
  keys: string
  label: string
  icon?: typeof FolderOpen
}): React.JSX.Element {
  return (
    <span className="flex items-center gap-1.5">
      <kbd className="rounded border border-white/10 px-1 py-0.5 leading-none">
        {Icon ? <Icon size={9} /> : keys}
      </kbd>
      {label}
    </span>
  )
}
