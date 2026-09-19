import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  ClipboardPaste,
  Copy,
  Download,
  ExternalLink,
  FolderOpen,
  FolderPlus,
  Files,
  Info,
  Loader2,
  PlayCircle,
  RefreshCw,
  Scissors,
  Search,
  SquarePen,
  Star,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import { TitleBar } from '@/components/TitleBar'
import { Breadcrumbs } from '@/components/Breadcrumbs'
import { Sidebar, type NavKey } from '@/components/Sidebar'
import { LibraryView } from '@/components/LibraryView'
import { useMediaLibrary } from '@/lib/useMediaLibrary'
import { useLiveChanges } from '@/lib/useLiveChanges'
import { useWatched } from '@/lib/useWatched'
import { FileList, type Entry, type RowHandlers } from '@/components/FileList'
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
import { PropertiesPanel } from '@/components/PropertiesPanel'
import { useContextMenu, type MenuAction } from '@/components/ui/ContextMenu'
import { PromptDialog, type PromptRequest } from '@/components/ui/PromptDialog'
import { api, joinPath, parentOf } from '@/lib/api'
import { useVault } from '@/lib/useVault'
import { filterKind, recentOf, useLibraryScan } from '@/lib/useLibrary'
import { transferId, useTransfers } from '@/lib/useTransfers'
import { nameOf, useFileActions } from '@/lib/useFileActions'
import { useAsyncSubscription, useLatest } from '@/lib/useAsyncSubscription'
import { useStars } from '@/lib/useStars'
import { entriesToMedia, isPlayable } from '@/lib/media'
import type { MediaItem } from '@/lib/mockMedia'
import {
  baseName,
  localJoin,
  onExternalFileDrop,
  pickFiles,
  pickFolder,
  pickSaveLocation,
} from '@/lib/dialogs'
import { cn, formatBytes } from '@/lib/utils'

const LIBRARY_KEYS: NavKey[] = ['videos', 'music', 'photos']

/** The sections served by the host's media index rather than a folder scan. */
const MEDIA_KEYS: NavKey[] = ['movies', 'series']

const TITLES: Record<NavKey, string> = {
  files: 'Vault',
  recent: 'Recent',
  starred: 'Starred',
  movies: 'Movies',
  series: 'TV Series',
  videos: 'Videos',
  music: 'Music',
  photos: 'Photos',
  settings: 'Settings',
}

export function App(): React.JSX.Element {
  const vault = useVault()
  const transfers = useTransfers()
  const menu = useContextMenu()

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
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const [properties, setProperties] = useState<Entry | null>(null)
  const [dropActive, setDropActive] = useState(false)

  const connected = vault.status?.connected ?? false
  const writable = vault.status?.writable ?? false

  const actions = useFileActions({ onChanged: vault.refresh, onError: setNotice })

  const stars = useStars(vault.status?.hostId, nav === 'starred')

  const needsScan = nav === 'recent' || LIBRARY_KEYS.includes(nav)
  const scan = useLibraryScan(needsScan, connected)

  const media = useMediaLibrary(connected)
  const isMedia = MEDIA_KEYS.includes(nav)

  const watched = useWatched(connected)
  const watchedByPath = useMemo(
    () => new Map(watched.all.map((entry) => [entry.path, entry])),
    [watched.all],
  )

  /**
   * Every episode in order, so the player knows what comes next.
   *
   * Flattened across seasons: the episode after the last one of season one is
   * the first of season two, and stopping at a season boundary is exactly the
   * point at which autoplay is most wanted.
   */
  const episodeOrder = useMemo(() => {
    const order: { path: string; label: string }[] = []
    for (const series of media.series) {
      for (const season of series.seasons) {
        for (const episode of season.episodes) {
          order.push({
            path: episode.path,
            label: `${series.title} · S${String(season.number).padStart(2, '0')}E${String(
              episode.number,
            ).padStart(2, '0')}`,
          })
        }
      }
    }
    return order
  }, [media.series])

  const nextAfter = useCallback(
    (path: string): { path: string; label: string } | null => {
      const at = episodeOrder.findIndex((e) => e.path === path)
      return at >= 0 ? (episodeOrder[at + 1] ?? null) : null
    },
    [episodeOrder],
  )

  // The drive is the truth: whatever changes it, the folder on screen reloads
  // and the index is asked again. Nothing here polls.
  useLiveChanges(vault.dir, vault.refresh, media.refresh)

  const sectionEntries = useMemo(() => {
    if (nav === 'recent') return recentOf(scan.files)
    if (nav === 'starred') return stars.entries
    return vault.entries
  }, [nav, scan.files, stars.entries, vault.entries])

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

  const isLibrary = LIBRARY_KEYS.includes(nav)

  const libraryItems = useMemo(() => {
    if (!isLibrary) return []
    return entriesToMedia(filterKind(scan.files, nav as 'videos' | 'music' | 'photos'))
  }, [isLibrary, nav, scan.files])

  /** The films or series on screen, filtered by the search box. */
  const mediaItems = useMemo(() => {
    const items = nav === 'movies' ? media.films : media.series
    const needle = query.trim().toLowerCase()
    return needle ? items.filter((i) => i.title.toLowerCase().includes(needle)) : items
  }, [nav, media.films, media.series, query])

  /** The entries the next action applies to: the selection, or what was clicked. */
  const targetsFor = useCallback(
    (entry?: Entry): Entry[] => {
      if (entry && !selected.has(entry.id)) return [entry]
      const chosen = entries.filter((e) => selected.has(e.id))
      return chosen.length > 0 ? chosen : entry ? [entry] : []
    },
    [entries, selected],
  )

  // --- selection -----------------------------------------------------------

  // The row a range-select measures from, kept out of state so anchoring never
  // causes a render of its own.
  const anchor = useRef<string | null>(null)

  const handleSelect = useCallback(
    (id: string, modifiers: { additive: boolean; range: boolean }) => {
      if (modifiers.range && anchor.current) {
        const from = entries.findIndex((e) => e.id === anchor.current)
        const to = entries.findIndex((e) => e.id === id)
        if (from !== -1 && to !== -1) {
          const [lo, hi] = from < to ? [from, to] : [to, from]
          setSelected(new Set(entries.slice(lo, hi + 1).map((e) => e.id)))
          return
        }
      }
      anchor.current = id
      setSelected((prev) => {
        if (!modifiers.additive) return new Set([id])
        const next = new Set(prev)
        if (next.has(id)) next.delete(id)
        else next.add(id)
        return next
      })
    },
    [entries],
  )

  // --- transfers -----------------------------------------------------------

  const downloadOne = useCallback(
    async (entry: Entry) => {
      const destination = await pickSaveLocation(entry.name)
      if (!destination) return
      setTransfersOpen(true)

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

  const downloadMany = useCallback(
    async (chosen: Entry[]) => {
      const files = chosen.filter((e) => e.kind === 'file')
      if (files.length === 0) {
        setNotice('Folders cannot be downloaded yet — open one and take the files.')
        return
      }
      if (files.length === 1) {
        await downloadOne(files[0]!)
        return
      }

      const folder = await pickFolder()
      if (!folder) return
      setTransfersOpen(true)

      // Sequentially: the link is the bottleneck at ~22.7 MB/s, so running
      // several at once would divide the same bandwidth and finish none of
      // them sooner, while making every progress bar useless.
      for (const entry of files) {
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
    },
    [downloadOne, transfers],
  )

  /**
   * Uploads already in flight, keyed by source and destination.
   *
   * Belt and braces against starting the same upload twice. The listener leak
   * that made a single drop start eight identical uploads is fixed at its
   * source, but a user can also double-drop, and two copies of the same file
   * racing for one destination is not something to find out about at the
   * commit.
   */
  const inFlight = useRef(new Set<string>())

  const uploadPaths = useCallback(
    async (paths: string[], into: string) => {
      if (paths.length === 0) return

      const fresh = paths.filter((local) => !inFlight.current.has(`${local}->${into}`))
      if (fresh.length === 0) {
        setNotice('That is already uploading.')
        return
      }
      for (const local of fresh) inFlight.current.add(`${local}->${into}`)
      setTransfersOpen(true)

      for (const local of fresh) {
        const name = baseName(local)
        const id = transferId()
        transfers.start({
          id,
          kind: 'upload',
          name,
          path: joinPath(into, name),
          total: 0,
        })
        try {
          await api.upload(local, joinPath(into, name), false, id)
          transfers.finish(id)
        } catch (e) {
          transfers.finish(id, e instanceof Error ? e.message : String(e))
        } finally {
          inFlight.current.delete(`${local}->${into}`)
        }
      }
      vault.refresh()
    },
    [transfers, vault],
  )

  const uploadHere = useCallback(async () => {
    await uploadPaths(await pickFiles(), vault.dir)
  }, [uploadPaths, vault.dir])

  // --- opening -------------------------------------------------------------

  /**
   * Hands a file to a player that can decode it.
   *
   * Streamed over a local URL when VLC, mpv or similar is installed, so a 3 GB
   * episode starts at once and nothing lands on this disk. A transfer row is
   * only opened for the fallback, where the file really is being copied — a
   * progress bar for something that is streaming would be a lie.
   */
  const openExternally = useCallback(
    async (path: string) => {
      const id = transferId()
      try {
        const result = await api.openExternally(path, id)
        if (result.streamed) {
          setNotice(`Streaming to ${result.player}. Nothing is being downloaded.`)
        } else {
          transfers.finish(id)
        }
      } catch (e) {
        transfers.finish(id, e instanceof Error ? e.message : String(e))
        setNotice(e instanceof Error ? e.message : String(e))
      }
    },
    [transfers],
  )

  const openEntry = useCallback(
    (entry: Entry) => {
      if (entry.kind === 'dir') {
        setSelected(new Set())
        anchor.current = null
        setNav('files')
        vault.open(entry.id)
        return
      }
      const media = entriesToMedia([entry])[0]
      if (isPlayable(entry.name)) setPlaying(media ?? null)
      else void downloadOne(entry)
    },
    [vault, downloadOne],
  )

  /**
   * Plays a path the media index gave us.
   *
   * The index knows a path, not a listing entry, so this stats the file to
   * build one — which also means a film deleted since the last scan fails with
   * something the user can read rather than an empty player.
   */
  const playPath = useCallback(
    async (path: string) => {
      try {
        const entry = await api.stat(path)
        const item = entriesToMedia([
          {
            id: path,
            name: entry.name,
            kind: 'file',
            size: entry.size,
            modified: entry.mtime * 1000,
          },
        ])[0]
        if (item) setPlaying(item)
      } catch {
        setNotice(`${nameOf(path)} is not on the drive any more.`)
        media.refresh()
      }
    },
    [media],
  )

  // --- prompts -------------------------------------------------------------

  const askRename = useCallback(
    (entry: Entry) => {
      setPrompt({
        title: `Rename ${entry.kind === 'dir' ? 'folder' : 'file'}`,
        value: entry.name,
        confirmLabel: 'Rename',
        select: 'stem',
        onConfirm: (name) => void actions.rename(entry.id, name),
      })
    },
    [actions],
  )

  const askNewFolder = useCallback(() => {
    setPrompt({
      title: 'New folder',
      value: 'New folder',
      confirmLabel: 'Create',
      select: 'all',
      onConfirm: (name) => void actions.newFolder(vault.dir, name),
    })
  }, [actions, vault.dir])

  // --- the context menu ----------------------------------------------------

  const backgroundActions = useCallback((): MenuAction[] => {
    const items: MenuAction[] = []
    if (writable) {
      items.push({
        id: 'new-folder',
        label: 'New folder',
        icon: FolderPlus,
        run: askNewFolder,
      })
      items.push({
        id: 'upload',
        label: 'Upload files here…',
        icon: Upload,
        run: uploadHere,
      })
      items.push({
        id: 'paste',
        label: 'Paste',
        icon: ClipboardPaste,
        shortcut: 'Ctrl+V',
        separatorBefore: true,
        disabled: !actions.canPasteInto(vault.dir),
        run: () => void actions.paste(vault.dir),
      })
    }
    items.push({
      id: 'refresh',
      label: 'Refresh',
      icon: RefreshCw,
      shortcut: 'F5',
      separatorBefore: items.length > 0,
      run: vault.refresh,
    })
    return items
  }, [writable, askNewFolder, uploadHere, actions, vault.dir, vault.refresh])

  const entryActions = useCallback(
    (entry: Entry): MenuAction[] => {
      const chosen = targetsFor(entry)
      const many = chosen.length > 1
      const paths = chosen.map((e) => e.id)
      const label = many ? `${chosen.length} items` : entry.name

      const items: MenuAction[] = []

      // Only when opening means something other than downloading. For a file
      // the window cannot play, "Open" and "Download…" would be the same
      // action listed twice.
      const canOpen = entry.kind === 'dir' || isPlayable(entry.name)
      if (canOpen && !many) {
        items.push({
          id: 'open',
          label: entry.kind === 'dir' ? 'Open' : 'Play',
          icon: entry.kind === 'dir' ? FolderOpen : PlayCircle,
          run: () => openEntry(entry),
        })
      }

      if (!many && entry.kind === 'file') {
        items.push({
          id: 'open-external',
          label: 'Play in your player',
          icon: ExternalLink,
          run: () => void openExternally(entry.id),
        })
      }

      items.push({
        id: 'download',
        label: many ? `Download ${label}…` : 'Download…',
        icon: Download,
        separatorBefore: items.length > 0,
        // A folder has no download path yet; saying so beats a dead entry.
        disabled: chosen.every((e) => e.kind === 'dir'),
        run: () => void downloadMany(chosen),
      })

      if (writable) {
        items.push(
          {
            id: 'cut',
            label: 'Cut',
            icon: Scissors,
            shortcut: 'Ctrl+X',
            separatorBefore: true,
            run: () => actions.cut(paths),
          },
          {
            id: 'copy',
            label: 'Copy',
            icon: Copy,
            shortcut: 'Ctrl+C',
            run: () => actions.copy(paths),
          },
          {
            id: 'paste-into',
            label: 'Paste into folder',
            icon: ClipboardPaste,
            disabled: entry.kind !== 'dir' || many || !actions.canPasteInto(entry.id),
            run: () => void actions.paste(entry.id),
          },
          {
            id: 'duplicate',
            label: 'Duplicate',
            icon: Files,
            disabled: many,
            run: () => void actions.duplicate(entry.id),
          },
          {
            id: 'rename',
            label: 'Rename…',
            icon: SquarePen,
            shortcut: 'F2',
            separatorBefore: true,
            disabled: many,
            run: () => askRename(entry),
          },
          {
            id: 'delete',
            label: many ? `Delete ${label}` : 'Delete',
            icon: Trash2,
            shortcut: 'Del',
            danger: true,
            run: () => void actions.remove(chosen),
          },
        )
      }

      items.push({
        id: 'star',
        label: chosen.every((e) => stars.isStarred(e.id))
          ? many
            ? 'Remove stars'
            : 'Remove star'
          : many
            ? `Star ${label}`
            : 'Star',
        icon: Star,
        separatorBefore: true,
        run: () => stars.toggle(chosen),
      })

      items.push({
        id: 'properties',
        label: 'Properties',
        icon: Info,
        separatorBefore: true,
        disabled: many,
        run: () => setProperties(entry),
      })

      return items
    },
    [
      targetsFor,
      writable,
      openEntry,
      openExternally,
      downloadMany,
      actions,
      askRename,
      stars,
    ],
  )

  // --- row handlers, as one stable object ----------------------------------

  const handlers = useMemo<RowHandlers>(
    () => ({
      onSelect: handleSelect,
      onOpen: openEntry,
      onContextMenu: (entry, event) => {
        // Right-clicking outside the selection targets that row instead, which
        // is what every file manager does and what stops an accidental delete
        // of something the user had forgotten was selected.
        if (!selected.has(entry.id)) {
          setSelected(new Set([entry.id]))
          anchor.current = entry.id
        }
        menu.open(event, entryActions(entry))
      },
      onDownload: (entry) => void downloadOne(entry),
      onDragStart: (entry) => {
        if (selected.has(entry.id)) return entries.filter((e) => selected.has(e.id)).map((e) => e.id)
        setSelected(new Set([entry.id]))
        return [entry.id]
      },
      onDropInto: (entry, paths) => void actions.moveInto(paths, entry.id),
    }),
    [handleSelect, openEntry, selected, menu, entryActions, downloadOne, entries, actions],
  )

  const cutPaths = useMemo(
    () =>
      actions.clipboard?.mode === 'cut'
        ? new Set(actions.clipboard.paths)
        : undefined,
    [actions.clipboard],
  )

  // --- keyboard ------------------------------------------------------------

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const target = e.target as HTMLElement | null
      // Never steal a shortcut from a field the user is typing in.
      const typing =
        target?.tagName === 'INPUT' ||
        target?.tagName === 'TEXTAREA' ||
        target?.isContentEditable
      const ctrl = e.ctrlKey || e.metaKey

      if (ctrl && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setPaletteOpen((v) => !v)
        return
      }
      if (typing || nav === 'settings' || prompt) return

      const chosen = entries.filter((en) => selected.has(en.id))

      if (ctrl && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        setSelected(new Set(entries.map((en) => en.id)))
        return
      }
      if (ctrl && e.key.toLowerCase() === 'c' && chosen.length > 0) {
        actions.copy(chosen.map((en) => en.id))
        return
      }
      if (ctrl && e.key.toLowerCase() === 'x' && chosen.length > 0 && writable) {
        actions.cut(chosen.map((en) => en.id))
        return
      }
      if (ctrl && e.key.toLowerCase() === 'v' && writable) {
        void actions.paste(vault.dir)
        return
      }
      if (e.key === 'Delete' && chosen.length > 0 && writable) {
        void actions.remove(chosen)
        return
      }
      if (e.key === 'F2' && chosen.length === 1 && writable) {
        askRename(chosen[0]!)
        return
      }
      if (e.key === 'F5') {
        e.preventDefault()
        vault.refresh()
        return
      }
      if (e.key === 'Escape') {
        setSelected(new Set())
        return
      }
      if (e.key === 'Enter' && chosen.length === 1) {
        openEntry(chosen[0]!)
        return
      }
      // Backspace goes up a folder, as it does in Explorer.
      if (e.key === 'Backspace' && nav === 'files' && vault.dir) {
        vault.open(parentOf(vault.dir))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [
    entries,
    selected,
    actions,
    writable,
    vault,
    nav,
    prompt,
    askRename,
    openEntry,
  ])

  // --- files dragged in from Explorer --------------------------------------

  // Read through refs so the subscription below can be established once.
  // Depending on these directly re-ran the effect on every render, and because
  // the unsubscribe arrives asynchronously the old listener was never removed —
  // so a single drop started one upload per render that had happened since the
  // app opened.
  const latestUpload = useLatest(uploadPaths)
  const latestDir = useLatest(vault.dir)

  const subscribeToDrops = useCallback(
    () =>
      onExternalFileDrop({
        onEnter: () => setDropActive(true),
        onLeave: () => setDropActive(false),
        onDrop: (paths) => {
          setDropActive(false)
          void latestUpload.current(paths, latestDir.current)
        },
      }),
    [latestUpload, latestDir],
  )

  useAsyncSubscription(writable, subscribeToDrops)

  // --- effects -------------------------------------------------------------

  useEffect(() => {
    setQuery('')
    setSelected(new Set())
    anchor.current = null
  }, [nav])

  useEffect(() => {
    setSelected(new Set())
    anchor.current = null
  }, [vault.dir])

  useEffect(() => {
    if (!notice) return undefined
    const timer = setTimeout(() => setNotice(null), 6000)
    return () => clearTimeout(timer)
  }, [notice])

  const forgetVault = useCallback(async () => {
    const hostId = vault.status?.hostId
    if (!hostId) return
    // Inside the try, all of it. With the confirmation outside, anything that
    // went wrong in it escaped this callback entirely and the button did
    // nothing at all, silently.
    try {
      const { confirmAction } = await import('@/lib/dialogs')
      const ok = await confirmAction(
        'This device will have to pair again with a new PIN. Nothing on the drive is affected.',
        'Forget this vault?',
      )
      if (!ok) return

      const next = await api.forgetHost(hostId)
      vault.setStatus({ ...next, hasPaired: false, connected: false })
      setNav('files')
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    }
  }, [vault])

  const selectedSize = useMemo(() => {
    if (selected.size === 0) return 0
    let total = 0
    for (const e of entries) if (selected.has(e.id)) total += e.size
    return total
  }, [selected, entries])

  // --- screens -------------------------------------------------------------

  if (!vault.status) return <Splash failed={vault.startupFailed} />

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
  const chosenEntries = entries.filter((e) => selected.has(e.id))

  const viewProps = {
    entries,
    selected,
    cutPaths,
    handlers,
    onBackgroundContextMenu: (event: { clientX: number; clientY: number }) => {
      setSelected(new Set())
      menu.open(event, backgroundActions())
    },
  }

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
                  onNavigateTo={(index) => vault.open(path.slice(0, index).join('/'))}
                  onDropInto={(index, paths) =>
                    void actions.moveInto(paths, path.slice(0, index).join('/'))
                  }
                />
              ) : (
                <div className="flex items-baseline gap-2.5">
                  <span className="text-sm font-semibold text-text">{TITLES[nav]}</span>
                  <span className="tnum font-mono text-[11px] text-textFaint">
                    {/* Whatever this screen is actually showing. Reading the
                        file count on a Movies screen reported 100,000 items
                        next to three films. */}
                    {(isMedia
                      ? mediaItems.length
                      : isLibrary
                        ? libraryItems.length
                        : entries.length
                    ).toLocaleString()}
                  </span>
                </div>
              )}

              <div className="flex-1" />

              {nav === 'files' && writable && (
                <>
                  <ToolButton icon={FolderPlus} label="New folder" onClick={askNewFolder} />
                  <ToolButton icon={Upload} label="Upload files" onClick={uploadHere} />
                </>
              )}
              {selected.size > 0 && (
                <>
                  <ToolButton
                    icon={Download}
                    label="Download selected"
                    onClick={() => downloadMany(chosenEntries)}
                  />
                  {writable && (
                    <ToolButton
                      icon={Trash2}
                      label="Delete selected"
                      onClick={() => actions.remove(chosenEntries)}
                      danger
                    />
                  )}
                </>
              )}

              {/* Neither applies to a wall of posters, which is always newest
                  first and always the same shape. A control that does nothing
                  is worse than no control. */}
              {!isLibrary && !isMedia && (
                <SortMenu
                  field={sortField}
                  direction={sortDirection}
                  onFieldChange={setSortField}
                  onDirectionChange={setSortDirection}
                />
              )}
              {!isLibrary && !isMedia && <ViewMenu mode={view} onChange={setView} />}

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
            className="relative min-h-0 flex-1"
          >
            {nav === 'settings' ? (
              <SettingsView
                status={vault.status}
                space={vault.space}
                onForget={forgetVault}
                onRepair={forgetVault}
              />
            ) : isMedia ? (
              <LibraryView
                kind={nav === 'movies' ? 'film' : 'series'}
                items={mediaItems}
                enabled={media.enabled}
                scanning={media.scanning}
                watched={watchedByPath}
                continueWatching={watched.continueWatching}
                playing={playing !== null}
                onPlay={(path) => void playPath(path)}
                onForget={watched.forget}
              />
            ) : isLibrary ? (
              libraryItems.length === 0 ? (
                <EmptyState
                  label={scan.scanning ? 'Looking through the vault…' : `No ${nav} found`}
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
              <div
                className="h-full"
                onContextMenu={(e) => {
                  e.preventDefault()
                  menu.open(e, backgroundActions())
                }}
              >
                <EmptyState
                  label={
                    vault.loading || scan.scanning
                      ? 'Loading…'
                      : query
                        ? `Nothing matches “${query}”`
                        : nav === 'starred'
                          ? stars.loading
                            ? 'Loading…'
                            : 'Nothing starred yet. Right-click a file and choose Star.'
                          : 'This folder is empty'
                  }
                />
              </div>
            ) : view === 'tiles' ? (
              <TileView {...viewProps} />
            ) : view === 'list' ? (
              <ListView {...viewProps} />
            ) : (
              <FileList {...viewProps} />
            )}

            <DropOverlay active={dropActive && nav === 'files'} dir={vault.dir} />
          </motion.div>

          {nav !== 'settings' && !isLibrary && !isMedia && (
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
            onClick={() => setNotice(null)}
            className="fixed bottom-16 left-1/2 z-[70] max-w-[440px] -translate-x-1/2 cursor-pointer rounded-md border border-danger/30 bg-dangerBg px-3.5 py-2 text-[12px] leading-relaxed text-danger shadow-lift"
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
        onOpenEntry={openEntry}
      />

      <PlayerOverlay
        item={playing}
        onClose={() => {
          setPlaying(null)
          // The player reports one last position as it unmounts; picking the
          // list up straight after means Continue watching is right by the
          // time anyone looks at it.
          setTimeout(watched.refresh, 300)
        }}
        onOpenExternally={(path) => void openExternally(path)}
        resumeAt={playing ? (watchedByPath.get(playing.id)?.position ?? 0) : 0}
        onProgress={watched.report}
        nextUp={playing ? nextAfter(playing.id) : null}
        onPlayNext={(path) => void playPath(path)}
      />

      <ImageViewer
        items={libraryItems}
        index={viewingIndex}
        onIndexChange={setViewingIndex}
        onClose={() => setViewingIndex(null)}
      />

      <PropertiesPanel
        entry={properties}
        vaultName={vault.status.vault ?? 'Vault'}
        onClose={() => setProperties(null)}
      />

      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      {menu.node}
    </div>
  )
}

/**
 * The first half-second, before the backend has said whether it is connected.
 *
 * Deliberately not a spinner over an empty window: the mark is already the
 * app's identity, and breathing it reads as "starting" without implying
 * anything is slow.
 *
 * It says so in words after a moment, though. A dark window with a faint mark
 * in the middle is, at a glance, indistinguishable from a crashed application —
 * which is exactly how a startup bug here was first reported.
 */
function Splash({ failed }: { failed: boolean }): React.JSX.Element {
  const [slow, setSlow] = useState(false)

  useEffect(() => {
    const timer = setTimeout(() => setSlow(true), 1500)
    return () => clearTimeout(timer)
  }, [])

  return (
    <div className="relative flex h-full flex-col items-center justify-center gap-5">
      <div className="backdrop" />
      <motion.span
        animate={{ opacity: [0.35, 1, 0.35] }}
        transition={{ duration: 2.4, repeat: Infinity, ease: 'easeInOut' }}
        className="relative z-10 text-basalt"
      >
        <HexMark size={30} />
      </motion.span>

      <AnimatePresence>
        {(slow || failed) && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            className="relative z-10 max-w-[320px] text-center"
          >
            <p className="text-[12px] leading-relaxed text-textDim">
              {failed
                ? 'Basalt is running but its backend is not answering. Still trying — if this does not clear, close the window and open it again.'
                : 'Starting…'}
            </p>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

/** Shown while files from Explorer are being dragged over the window. */
function DropOverlay({
  active,
  dir,
}: {
  active: boolean
  dir: string
}): React.JSX.Element {
  return (
    <AnimatePresence>
      {active && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="pointer-events-none absolute inset-2 z-30 flex items-center justify-center rounded-lg border-2 border-dashed border-basalt/40 bg-ink/70 backdrop-blur-[1px]"
        >
          <div className="text-center">
            <Upload size={26} className="mx-auto text-basaltDeep" />
            <p className="mt-3 text-sm text-text">
              Drop to upload into {dir ? nameOf(dir) : 'the vault'}
            </p>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
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
  const offline = vault.error?.kind === 'offline' || vault.error?.kind === 'unpaired'
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
