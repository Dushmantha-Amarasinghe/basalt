import { useCallback, useEffect, useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { ChevronRight, Search, X } from 'lucide-react'
import { TitleBar } from '@/components/TitleBar'
import { useSimulatedThroughput } from '@/components/Sparkline'
import { Sidebar, type NavKey } from '@/components/Sidebar'
import { FileList, type Entry } from '@/components/FileList'
import { CommandPalette } from '@/components/CommandPalette'
import { generateEntries } from '@/lib/mockData'
import { cn, formatBytes } from '@/lib/utils'

/** Row count for the mock data. High on purpose: see `mockData.ts`. */
const MOCK_ENTRIES = 100_000

export function App(): React.JSX.Element {
  const [nav, setNav] = useState<NavKey>('files')
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [path, setPath] = useState<string[]>(['Vault'])
  const [paletteOpen, setPaletteOpen] = useState(false)

  const allEntries = useMemo(() => generateEntries(MOCK_ENTRIES), [])

  // Live link activity. Simulated for now; the shape of the data is what the
  // header renders, so swapping in the real feed later changes nothing here.
  const samples = useSimulatedThroughput()
  const throughput = samples[samples.length - 1] ?? 0

  const entries = useMemo(() => {
    if (!query.trim()) return allEntries
    const needle = query.toLowerCase()
    return allEntries.filter((e) => e.name.toLowerCase().includes(needle))
  }, [allEntries, query])

  const handleSelect = useCallback((id: string, additive: boolean) => {
    setSelected((prev) => {
      if (!additive) return new Set([id])
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const handleOpen = useCallback((entry: Entry) => {
    if (entry.kind === 'dir') {
      setPath((prev) => [...prev, entry.name])
      setSelected(new Set())
    }
  }, [])

  // Ctrl+K anywhere. Registered on the window rather than a focused element so
  // it works no matter what currently has focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen((v) => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const selectedSize = useMemo(() => {
    if (selected.size === 0) return 0
    let total = 0
    for (const e of allEntries) if (selected.has(e.id)) total += e.size
    return total
  }, [selected, allEntries])

  return (
    <div className="relative flex h-full flex-col">
      <div className="backdrop" />
      <TitleBar
        vaultName="Vault"
        connected
        throughput={throughput}
        samples={samples}
      />

      <div className="relative flex min-h-0 flex-1">
        <Sidebar
          active={nav}
          onNavigate={setNav}
          driveUsed={1_842_000_000_000}
          driveTotal={4_000_000_000_000}
          connected
          throughput={throughput}
        />

        <main className="flex min-w-0 flex-1 flex-col">
          <Toolbar
            path={path}
            onNavigateTo={(index) => {
              setPath((prev) => prev.slice(0, index + 1))
              setSelected(new Set())
            }}
            query={query}
            onQueryChange={setQuery}
          />

          {/*
            Keyed on the section so switching views replays the entrance. The
            list itself is virtualised and must not animate per row, so the
            motion lives on the container: content lifts in as a single plane.
          */}
          <motion.div
            key={nav}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="min-h-0 flex-1"
          >
            <FileList
              entries={entries}
              selected={selected}
              onSelect={handleSelect}
              onOpen={handleOpen}
            />
          </motion.div>

          <StatusBar
            total={entries.length}
            filtered={query.trim().length > 0}
            selectedCount={selected.size}
            selectedSize={selectedSize}
            onOpenPalette={() => setPaletteOpen(true)}
          />
        </main>
      </div>

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        entries={allEntries}
        onNavigate={setNav}
        onOpenEntry={handleOpen}
      />
    </div>
  )
}

function Toolbar({
  path,
  onNavigateTo,
  query,
  onQueryChange,
}: {
  path: string[]
  onNavigateTo: (index: number) => void
  query: string
  onQueryChange: (value: string) => void
}): React.JSX.Element {
  return (
    <div className="drag flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
      <nav className="flex min-w-0 items-center gap-1 text-sm">
        {path.map((segment, index) => (
          <div key={`${segment}-${index}`} className="flex min-w-0 items-center">
            {index > 0 && <ChevronRight size={14} className="mx-0.5 shrink-0 text-textFaint" />}
            <button
              onClick={() => onNavigateTo(index)}
              className={cn(
                'no-drag truncate rounded px-1.5 py-0.5 transition-colors',
                index === path.length - 1
                  ? 'font-semibold text-text'
                  : 'text-textDim hover:bg-white/[0.04] hover:text-text',
              )}
            >
              {segment}
            </button>
          </div>
        ))}
      </nav>

      <div className="flex-1" />

      <div className="no-drag relative">
        <Search
          size={14}
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textFaint"
        />
        <input
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder="Search"
          spellCheck={false}
          className="h-8 w-56 rounded-md border border-white/[0.07] bg-ink2 pl-8 pr-8 text-sm text-text placeholder:text-textFaint transition-colors focus:border-white/20"
        />
        <AnimatePresence>
          {query && (
            <motion.button
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.8 }}
              transition={{ duration: 0.12 }}
              onClick={() => onQueryChange('')}
              aria-label="Clear search"
              className="absolute right-2 top-1/2 -translate-y-1/2 text-textFaint hover:text-text"
            >
              <X size={14} />
            </motion.button>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}

function StatusBar({
  total,
  filtered,
  selectedCount,
  selectedSize,
  onOpenPalette,
}: {
  total: number
  filtered: boolean
  selectedCount: number
  selectedSize: number
  onOpenPalette: () => void
}): React.JSX.Element {
  return (
    <div className="flex h-7 shrink-0 items-center gap-3 border-t border-line px-4 font-mono text-[11px] text-textFaint">
      <span className="tnum">
        {total.toLocaleString()} item{total === 1 ? '' : 's'}
        {filtered && ' (filtered)'}
      </span>
      {selectedCount > 0 && (
        <>
          <span className="text-textFaint/40">·</span>
          <span className="tnum text-textDim">
            {selectedCount.toLocaleString()} selected
            {selectedSize > 0 && ` · ${formatBytes(selectedSize)}`}
          </span>
        </>
      )}

      <div className="flex-1" />

      <button
        onClick={onOpenPalette}
        className="flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-colors hover:bg-white/[0.05] hover:text-textDim"
      >
        <kbd className="rounded border border-white/10 px-1 leading-[14px]">Ctrl</kbd>
        <kbd className="rounded border border-white/10 px-1 leading-[14px]">K</kbd>
        <span>commands</span>
      </button>
    </div>
  )
}
