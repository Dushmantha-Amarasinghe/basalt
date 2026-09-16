import { useCallback, useEffect, useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  Download,
  FolderPlus,
  Loader2,
  Search,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
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
import { PairingView } from '@/components/PairingView'
import { HexMark } from '@/components/HexMark'
import { api, joinPath } from '@/lib/api'
import { useVault } from '@/lib/useVault'
import { filterKind, recentOf, useLibraryScan } from '@/lib/useLibrary'
import { transferId, useTransfers } from '@/lib/useTransfers'
import { entriesToMedia, isPlayable } from '@/lib/media'
import type { MediaItem } from '@/lib/mockMedia'
import {
  baseName,
  confirmAction,
  localJoin,
  pickFiles,
  pickFolder,
  pickSaveLocation,
} from '@/lib/dialogs'
import { cn, formatBytes } from '@/lib/utils'

const LIBRARY_KEYS: NavKey[] = ['videos', 'music', 'photos']

export function App(): React.JSX.Element {
  const vault = useVault()
  const transfers = useTransfers()

  const [nav, setNav] = useState<NavKey>('files')
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [transfersOpen, setTransfersOpen] = useState(false)
  const [playing, setPlaying] = useState<MediaItem | null>(null)
  const [viewingIndex, setViewingIndex] = useState<number | null>(null)
  const [view, setView] = useState<ViewMode>('details')
  const [sortField, setSortField] = useState<SortField>('name')
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc')
  const [notice, setNotice] = useState<string | null>(null)

  const connected = vault.status?.connected ?? false
  const writable = vault.status?.writable ?? false

  const needsScan = nav === 'recent' || LIBRARY_KEYS.includes(nav)
  const scan = useLibraryScan(needsScan, connected)

  // Which entries this section is showing. Files browses the real directory;
  // everything else draws from the shallow scan.
  const sectionEntries = useMemo(() => {
    if (nav === 'recent') return recentOf(scan.files)
    if (nav === 'starred') return []
    return vault.entries
  }, [nav, scan.files, vault.entries])

  const entries = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const filtered = needle
      ? sectionEntries.filter((e) => e.name.toLowerCase().includes(needle))
      : sectionEntries
    // Recent is already ordered by date and re-sorting it by name would
    // defeat the point of the section.
    if (nav === 'recent' && sortField === 'name') return filtered
    return sortEntries(filtered, sortField, sortDirection)
  }, [sectionEntries, query, nav, sortField, sortDirection])

  const libraryItems = useMemo(() => {
    if (!LIBRARY_KEYS.includes(nav)) return []
    const kind = nav as 'videos' | 'music' | 'photos'
    return entriesToMedia(filterKind(scan.files, kind))
  }, [nav, scan.files])

  const isLibrary = LIBRARY_KEYS.includes(nav)

  // --- selection -----------------------------------------------------------

  const handleSelect = useCallback((id: string, additive: boolean) => {
    setSelected((prev) => {
      if (!additive) return new Set([id])
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const handleOpen = useCallback(
    (entry: Entry) => {
      if (entry.kind === 'dir') {
        setSelected(new Set())
        setNav('files')
        vault.open(entry.id)
        return
      }
      // A file: play it if the player can, otherwise offer to download it.
      const media = entriesToMedia([entry])[0]
      if (isPlayable(entry.name)) setPlaying(media ?? null)
      else void downloadOne(entry)
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [vault.open],
  )

  // --- transfers -----------------------------------------------------------

  const downloadOne = useCallback(
    async (entry: Entry) => {
      const destination = await pickSaveLocation(entry.name)
      if (!destination) return

      const id = transferId()
      transfers.start({
        id,
        kind: 'download',
        name: entry.name,
        path: entry.id,
        total: entry.size,
      })
      try {
        await api.download(entry.id, destination, id)
        transfers.finish(id)
      } catch (e) {
        transfers.finish(id, e instanceof Error ? e.message : String(e))
      }
    },
    [transfers],
  )

  const downloadSelected = useCallback(async () => {
    const chosen = entries.filter((e) => selected.has(e.id) && e.kind === 'file')
    if (chosen.length === 0) return
    if (chosen.length === 1) {
      await downloadOne(chosen[0]!)
      return
    }

    const folder = await pickFolder()
    if (!folder) return
    setTransfersOpen(true)

    // Sequentially: the link is the bottleneck at ~22.7 MB/s, so running
    // several at once would divide the same bandwidth and finish none of them
    // sooner, while making the progress bars useless.
    for (const entry of chosen) {
      const id = transferId()
      transfers.start({
        id,
        kind: 'download',
        name: entry.name,
        path: entry.id,
        total: entry.size,
      })
      try {
        await api.download(entry.id, localJoin(folder, entry.name), id)
        transfers.finish(id)
      } catch (e) {
        transfers.finish(id, e instanceof Error ? e.message : String(e))
      }
    }
  }, [entries, selected, downloadOne, transfers])

  const uploadHere = useCallback(async () => {
    const files = await pickFiles()
    if (files.length === 0) return
    setTransfersOpen(true)

    for (const local of files) {
      const name = baseName(local)
      const id = transferId()
      transfers.start({
        id,
        kind: 'upload',
        name,
        path: joinPath(vault.dir, name),
        total: 0,
      })
      try {
        await api.upload(local, joinPath(vault.dir, name), false, id)
        transfers.finish(id)
      } catch (e) {
        transfers.finish(id, e instanceof Error ? e.message : String(e))
      }
    }
    vault.refresh()
  }, [transfers, vault])

  // --- mutations -----------------------------------------------------------

  const newFolder = useCallback(async () => {
    const name = window.prompt('New folder name')
    if (!name?.trim()) return
    try {
      await api.mkdir(joinPath(vault.dir, name.trim()))
      vault.refresh()
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    }
  }, [vault])

  const deleteSelected = useCallback(async () => {
    const chosen = entries.filter((e) => selected.has(e.id))
    if (chosen.length === 0) return

    const ok = await confirmAction(
      chosen.length === 1
        ? `Delete ${chosen[0]!.name}?`
        : `Delete ${chosen.length} items?`,
      'This cannot be undone',
    )
    if (!ok) return

    for (const entry of chosen) {
      try {
        await api.remove(entry.id, entry.kind === 'dir')
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e))
      }
    }
    setSelected(new Set())
    vault.refresh()
  }, [entries, selected, vault])

  /**
   * Unpairs from the current vault, which sends the app back to the pairing
   * screen.
   *
   * Only this side forgets. The host keeps its record of the device until it
   * is revoked there too, which is the honest split: this app cannot reach
   * into someone else's machine and delete things from it.
   */
  const forgetVault = useCallback(async () => {
    const hostId = vault.status?.hostId
    if (!hostId) return
    const ok = await confirmAction(
      'This device will have to pair again with a new PIN. Nothing on the drive is affected.',
      'Forget this vault?',
    )
    if (!ok) return
    try {
      const next = await api.forgetHost(hostId)
      vault.setStatus({ ...next, hasPaired: false, connected: false })
      setNav('files')
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    }
  }, [vault])

  // --- effects -------------------------------------------------------------

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

  useEffect(() => {
    if (!notice) return undefined
    const timer = setTimeout(() => setNotice(null), 5000)
    return () => clearTimeout(timer)
  }, [notice])

  const selectedSize = useMemo(() => {
    if (selected.size === 0) return 0
    let total = 0
    for (const e of entries) if (selected.has(e.id)) total += e.size
    return total
  }, [selected, entries])

  // --- screens -------------------------------------------------------------

  if (!vault.status) return <Splash />

  if (!connected && !vault.status.hasPaired) {
    return (
      <div className="relative flex h-full flex-col">
        <div className="backdrop" />
        <TitleBar vaultName="Basalt" connected={false} />
        <div className="min-h-0 flex-1">
          <PairingView onPaired={vault.setStatus} />
        </div>
      </div>
    )
  }

  const path = vault.dir ? vault.dir.split('/') : []
  const crumbs = [vault.status.vault ?? 'Vault', ...path]

  return (
    <div className="relative flex h-full flex-col">
      <div className="backdrop" />
      <TitleBar vaultName={vault.status.vault ?? 'Vault'} connected={connected} />

      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        <Sidebar
          active={nav}
          onNavigate={setNav}
          driveUsed={vault.space ? vault.space[1] - vault.space[0] : 0}
          driveTotal={vault.space ? vault.space[1] : 0}
          connected={connected}
          vaultName={vault.status.vault ?? 'Vault'}
        />

        <main className="flex min-w-0 min-h-0 flex-1 flex-col">
          {nav !== 'settings' && (
            <div className="drag flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
              {nav === 'files' ? (
                <Breadcrumbs
                  path={crumbs}
                  onNavigateTo={(index) => {
                    setSelected(new Set())
                    vault.open(path.slice(0, index).join('/'))
                  }}
                />
              ) : (
                <div className="flex items-baseline gap-2.5">
                  <span className="text-sm font-semibold text-text">
                    {TITLES[nav]}
                  </span>
                  <span className="tnum font-mono text-[11px] text-textFaint">
                    {(isLibrary ? libraryItems.length : entries.length).toLocaleString()}
                  </span>
                </div>
              )}

              <div className="flex-1" />

              {nav === 'files' && writable && (
                <>
                  <ToolButton icon={FolderPlus} label="New folder" onClick={newFolder} />
                  <ToolButton icon={Upload} label="Upload files" onClick={uploadHere} />
                </>
              )}
              {selected.size > 0 && (
                <>
                  <ToolButton
                    icon={Download}
                    label="Download selected"
                    onClick={downloadSelected}
                  />
                  {writable && (
                    <ToolButton
                      icon={Trash2}
                      label="Delete selected"
                      onClick={deleteSelected}
                      danger
                    />
                  )}
                </>
              )}

              {!isLibrary && (
                <SortMenu
                  field={sortField}
                  direction={sortDirection}
                  onFieldChange={setSortField}
                  onDirectionChange={setSortDirection}
                />
              )}
              {!isLibrary && <ViewMenu mode={view} onChange={setView} />}

              <div className="no-drag relative shrink-0">
                <Search
                  size={14}
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-textFaint"
                />
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
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
                      onClick={() => setQuery('')}
                      aria-label="Clear search"
                      /*
                        Centred with `inset-y-0` + `my-auto`, deliberately not
                        `top-1/2 -translate-y-1/2`: Framer Motion animates
                        `scale` by writing an inline `transform`, which
                        silently overwrites Tailwind's translate. Auto margins
                        centre without touching transform.
                      */
                      className="absolute inset-y-0 right-1 my-auto flex h-6 w-6 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.06] hover:text-text"
                    >
                      <X size={13} />
                    </motion.button>
                  )}
                </AnimatePresence>
              </div>
            </div>
          )}

          <ConnectionBanner vault={vault} />

          <motion.div
            key={`${nav}-${vault.dir}`}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="min-h-0 flex-1"
          >
            {nav === 'settings' ? (
              <SettingsView
                status={vault.status}
                space={vault.space}
                onForget={forgetVault}
                onRepair={forgetVault}
              />
            ) : isLibrary ? (
              libraryItems.length === 0 ? (
                <EmptyState
                  label={
                    scan.scanning ? 'Looking through the vault…' : `No ${nav} found`
                  }
                />
              ) : (
                <MediaGrid
                  items={libraryItems}
                  shape={nav === 'photos' ? 'square' : 'poster'}
                  onOpen={(item, index) => {
                    if (nav === 'photos') setViewingIndex(index)
                    else setPlaying(item)
                  }}
                />
              )
            ) : entries.length === 0 ? (
              <EmptyState
                label={
                  vault.loading || scan.scanning
                    ? 'Loading…'
                    : query
                      ? `Nothing matches “${query}”`
                      : nav === 'starred'
                        ? 'Nothing starred yet'
                        : 'This folder is empty'
                }
              />
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
        transfers={transfers.transfers}
        open={transfersOpen}
        onToggle={() => setTransfersOpen((v) => !v)}
        onCancel={transfers.cancel}
        onClearDone={transfers.clearDone}
      />

      <AnimatePresence>
        {notice && (
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
            className="pointer-events-none fixed bottom-16 left-1/2 z-[70] -translate-x-1/2 rounded-md border border-danger/30 bg-dangerBg px-3.5 py-2 text-[12px] text-danger shadow-lift"
          >
            {notice}
          </motion.div>
        )}
      </AnimatePresence>

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        entries={entries}
        onNavigate={setNav}
        onOpenEntry={handleOpen}
      />

      <PlayerOverlay item={playing} onClose={() => setPlaying(null)} />

      <ImageViewer
        items={libraryItems}
        index={viewingIndex}
        onIndexChange={setViewingIndex}
        onClose={() => setViewingIndex(null)}
      />
    </div>
  )
}

const TITLES: Record<NavKey, string> = {
  files: 'Vault',
  recent: 'Recent',
  starred: 'Starred',
  videos: 'Videos',
  music: 'Music',
  photos: 'Photos',
  settings: 'Settings',
}

/**
 * The first half-second, before the backend has said whether it is connected.
 *
 * Deliberately not a spinner over an empty window: the mark is already the
 * app's identity, and breathing it reads as "starting" without implying
 * anything is slow.
 */
function Splash(): React.JSX.Element {
  return (
    <div className="relative flex h-full items-center justify-center">
      <div className="backdrop" />
      <motion.span
        animate={{ opacity: [0.35, 1, 0.35] }}
        transition={{ duration: 2.4, repeat: Infinity, ease: 'easeInOut' }}
        className="relative z-10 text-basalt"
      >
        <HexMark size={30} />
      </motion.span>
    </div>
  )
}

/**
 * Shown when the host cannot be reached.
 *
 * The listing underneath is left on screen on purpose. A stale view with an
 * honest banner over it is far more useful than an empty window, and when the
 * host comes back the same folder is still there.
 */
function ConnectionBanner({
  vault,
}: {
  vault: ReturnType<typeof useVault>
}): React.JSX.Element | null {
  const offline =
    vault.error?.kind === 'offline' || vault.error?.kind === 'unpaired'
  if (!offline) return null

  return (
    <motion.div
      initial={{ height: 0, opacity: 0 }}
      animate={{ height: 'auto', opacity: 1 }}
      transition={{ duration: 0.2 }}
      className="shrink-0 overflow-hidden border-b border-line bg-ink2"
    >
      <div className="flex items-center gap-2.5 px-4 py-2">
        <Loader2 size={13} className="shrink-0 animate-spin text-textFaint" />
        <span className="text-[12px] text-textDim">
          {vault.reconnecting
            ? 'Reconnecting…'
            : 'Lost the host. Trying again in the background.'}
        </span>
        <button
          onClick={() => void vault.reconnect()}
          className="ml-auto rounded px-2 py-0.5 text-[11px] text-textDim transition-colors hover:bg-white/[0.05] hover:text-text"
        >
          Retry now
        </button>
      </div>
    </motion.div>
  )
}

function ToolButton({
  icon: Icon,
  label,
  onClick,
  danger,
}: {
  icon: typeof Download
  label: string
  onClick: () => void | Promise<void>
  danger?: boolean
}): React.JSX.Element {
  return (
    <motion.button
      whileTap={{ scale: 0.94 }}
      transition={{ type: 'spring', stiffness: 500, damping: 30 }}
      onClick={() => void onClick()}
      aria-label={label}
      title={label}
      className={cn(
        'no-drag flex h-8 w-8 shrink-0 items-center justify-center rounded-md transition-colors',
        danger
          ? 'text-textDim hover:bg-danger/12 hover:text-danger'
          : 'text-textDim hover:bg-white/[0.05] hover:text-text',
      )}
    >
      <Icon size={15} />
    </motion.button>
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
