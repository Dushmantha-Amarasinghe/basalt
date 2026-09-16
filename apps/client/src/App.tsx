import { useCallback, useEffect, useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Search, X } from 'lucide-react'
import { TitleBar } from '@/components/TitleBar'
import { Breadcrumbs } from '@/components/Breadcrumbs'
import { Sidebar, type NavKey } from '@/components/Sidebar'
import { FileList, type Entry } from '@/components/FileList'
import { EmptyState, ListView, TileView } from '@/components/FileViews'
import { ViewMenu, type ViewMode } from '@/components/ViewMenu'
import {
  SortMenu,
  sortEntries,
  type SortDirection,
  type SortField,
} from '@/components/SortMenu'
import { MediaGrid } from '@/components/MediaGrid'
import { SettingsView } from '@/components/SettingsView'
import { TransfersPanel } from '@/components/TransfersPanel'
import { PlayerOverlay } from '@/components/PlayerOverlay'
import { ImageViewer } from '@/components/ImageViewer'
import { CommandPalette } from '@/components/CommandPalette'
import { generateEntries } from '@/lib/mockData'
import {
  generateMusic,
  generatePhotos,
  generateTransfers,
  generateVideos,
  type MediaItem,
} from '@/lib/mockMedia'
import { formatBytes } from '@/lib/utils'

/** Row count for the mock data. High on purpose: see `mockData.ts`. */
const MOCK_ENTRIES = 100_000

const TITLES: Record<NavKey, string> = {
  files: 'Vault',
  recent: 'Recent',
  starred: 'Starred',
  videos: 'Videos',
  music: 'Music',
  photos: 'Photos',
  settings: 'Settings',
}

export function App(): React.JSX.Element {
  const [nav, setNav] = useState<NavKey>('files')
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [path, setPath] = useState<string[]>(['Vault'])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [transfersOpen, setTransfersOpen] = useState(false)
  const [playing, setPlaying] = useState<MediaItem | null>(null)
  const [viewingIndex, setViewingIndex] = useState<number | null>(null)
  const [view, setView] = useState<ViewMode>('details')
  const [sortField, setSortField] = useState<SortField>('name')
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc')

  const allEntries = useMemo(() => generateEntries(MOCK_ENTRIES), [])
  const videos = useMemo(() => generateVideos(), [])
  const music = useMemo(() => generateMusic(), [])
  const photos = useMemo(() => generatePhotos(), [])
  const transfers = useMemo(() => generateTransfers(), [])

  // Each section draws from the same corpus but presents a different slice, so
  // navigation actually goes somewhere rather than relabelling one list.
  const sectionEntries = useMemo(() => {
    if (nav === 'recent') {
      return [...allEntries]
        .filter((e) => e.kind === 'file')
        .sort((a, b) => b.modified - a.modified)
        .slice(0, 400)
    }
    if (nav === 'starred') {
      // Deterministic pseudo-selection, standing in for real stars.
      return allEntries.filter((_, i) => i % 137 === 3).slice(0, 220)
    }
    return allEntries
  }, [nav, allEntries])

  const entries = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const filtered = needle
      ? sectionEntries.filter((e) => e.name.toLowerCase().includes(needle))
      : sectionEntries
    // Recent is already ordered by date and re-sorting it would defeat the
    // point of the section.
    if (nav === 'recent' && sortField === 'name') return filtered
    return sortEntries(filtered, sortField, sortDirection)
  }, [sectionEntries, query, nav, sortField, sortDirection])

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

  // Clear search and selection when changing section; carrying either over
  // makes the new view look mysteriously empty.
  useEffect(() => {
    setQuery('')
    setSelected(new Set())
  }, [nav])

  const selectedSize = useMemo(() => {
    if (selected.size === 0) return 0
    let total = 0
    for (const e of entries) if (selected.has(e.id)) total += e.size
    return total
  }, [selected, entries])

  const isLibrary = nav === 'videos' || nav === 'music' || nav === 'photos'
  const libraryItems = nav === 'videos' ? videos : nav === 'music' ? music : photos

  return (
    <div className="relative flex h-full flex-col">
      <div className="backdrop" />
      <TitleBar vaultName="Vault" connected />

      {/*
        `min-h-0` lets this shrink below its content height, which is what
        allows the transfers panel to expand without overflowing the window.
        `overflow-hidden` is the backstop: if anything inside still refuses to
        shrink, it clips rather than escaping and painting over the chrome.
      */}
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        <Sidebar
          active={nav}
          onNavigate={setNav}
          driveUsed={1_842_000_000_000}
          driveTotal={4_000_000_000_000}
          connected
        />

        <main className="flex min-w-0 min-h-0 flex-1 flex-col">
          {nav !== 'settings' && (
            <Toolbar
              title={TITLES[nav]}
              path={nav === 'files' ? path : undefined}
              onNavigateTo={(index) => {
                setPath((prev) => prev.slice(0, index + 1))
                setSelected(new Set())
              }}
              query={query}
              onQueryChange={setQuery}
              count={isLibrary ? libraryItems.length : entries.length}
              view={view}
              onViewChange={isLibrary ? undefined : setView}
              sortField={sortField}
              sortDirection={sortDirection}
              onSortFieldChange={isLibrary ? undefined : setSortField}
              onSortDirectionChange={setSortDirection}
            />
          )}

          {/*
            Keyed on the section so switching views replays the entrance. The
            file list is virtualised and must not animate per row, so the motion
            lives on the container: content lifts in as a single plane.
          */}
          <motion.div
            key={nav}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="min-h-0 flex-1"
          >
            {nav === 'settings' ? (
              <SettingsView />
            ) : isLibrary ? (
              <MediaGrid
                items={libraryItems}
                shape={nav === 'photos' ? 'square' : 'poster'}
                onOpen={(item, index) => {
                  // A photo has no timeline, so it gets a viewer rather than a
                  // transport with a scrubber and a play button.
                  if (nav === 'photos') setViewingIndex(index)
                  else setPlaying(item)
                }}
              />
            ) : entries.length === 0 ? (
              <EmptyState label={query ? `Nothing matches “${query}”` : 'Nothing here'} />
            ) : view === 'tiles' ? (
              <TileView
                entries={entries}
                selected={selected}
                onSelect={handleSelect}
                onOpen={handleOpen}
              />
            ) : view === 'list' ? (
              <ListView
                entries={entries}
                selected={selected}
                onSelect={handleSelect}
                onOpen={handleOpen}
              />
            ) : (
              <FileList
                entries={entries}
                selected={selected}
                onSelect={handleSelect}
                onOpen={handleOpen}
              />
            )}
          </motion.div>

          {nav !== 'settings' && !isLibrary && (
            <StatusBar
              total={entries.length}
              filtered={query.trim().length > 0}
              selectedCount={selected.size}
              selectedSize={selectedSize}
              onOpenPalette={() => setPaletteOpen(true)}
            />
          )}
        </main>
      </div>

      <TransfersPanel
        transfers={transfers}
        open={transfersOpen}
        onToggle={() => setTransfersOpen((v) => !v)}
      />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        entries={allEntries}
        onNavigate={setNav}
        onOpenEntry={handleOpen}
      />

      <PlayerOverlay item={playing} onClose={() => setPlaying(null)} />

      <ImageViewer
        items={photos}
        index={viewingIndex}
        onIndexChange={setViewingIndex}
        onClose={() => setViewingIndex(null)}
      />
    </div>
  )
}

function Toolbar({
  title,
  path,
  onNavigateTo,
  query,
  onQueryChange,
  count,
  view,
  onViewChange,
  sortField,
  sortDirection,
  onSortFieldChange,
  onSortDirectionChange,
}: {
  title: string
  path?: string[]
  onNavigateTo: (index: number) => void
  query: string
  onQueryChange: (value: string) => void
  count: number
  view: ViewMode
  onViewChange?: (mode: ViewMode) => void
  sortField: SortField
  sortDirection: SortDirection
  onSortFieldChange?: (field: SortField) => void
  onSortDirectionChange: (direction: SortDirection) => void
}): React.JSX.Element {
  return (
    <div className="drag flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
      {path ? (
        <Breadcrumbs path={path} onNavigateTo={onNavigateTo} />
      ) : (
        <div className="flex items-baseline gap-2.5">
          <span className="text-sm font-semibold text-text">{title}</span>
          <span className="tnum font-mono text-[11px] text-textFaint">
            {count.toLocaleString()}
          </span>
        </div>
      )}

      <div className="flex-1" />

      {onSortFieldChange && (
        <SortMenu
          field={sortField}
          direction={sortDirection}
          onFieldChange={onSortFieldChange}
          onDirectionChange={onSortDirectionChange}
        />
      )}
      {onViewChange && <ViewMenu mode={view} onChange={onViewChange} />}

      {/*
        `shrink-0` on the controls, and nothing else: the breadcrumb trail is
        the only element allowed to give up width, because it is the only one
        that degrades gracefully. A squashed search field is just broken.
      */}
      <div className="no-drag relative shrink-0">
        <Search
          size={14}
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textFaint"
        />
        <input
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder="Search"
          spellCheck={false}
          className="h-8 w-56 rounded-md border border-white/[0.07] bg-ink2 pl-8 pr-9 text-sm text-text placeholder:text-textFaint transition-colors focus:border-white/20"
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
              /*
                Centred with `inset-y-0` + `my-auto`, deliberately **not**
                `top-1/2 -translate-y-1/2`.
                
                Framer Motion animates `scale` by writing an inline
                `transform`, which silently overwrites Tailwind's
                `-translate-y-1/2` — so the button lost its centring the moment
                it animated in. Auto margins centre without touching transform,
                leaving it free for the animation.
              */
              className="absolute inset-y-0 right-1 my-auto flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.06] hover:text-text"
            >
              <X size={13} />
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
