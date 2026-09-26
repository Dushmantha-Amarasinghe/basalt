import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlertTriangle,
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
import { MusicList, PhotoGrid, VideoGrid, sortTracks } from '@/components/MediaViews'
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
import { useConfirm } from '@/components/ui/ConfirmDialog'
import {
  api,
  isFinished,
  joinPath,
  parentOf,
  type MediaFile,
  type SubtitleTrack,
} from '@/lib/api'
import { fileToEntry, useCollections } from '@/lib/useCollections'
import { useMediaBase } from '@/lib/thumbs'
import { stemOf, trackInfo } from '@/lib/mediaInfo'
import { useVault } from '@/lib/useVault'
import { filterKind, isKind, recentOf, useLibraryScan } from '@/lib/useLibrary'
import { transferId, useTransfers } from '@/lib/useTransfers'
import { nameOf, useFileActions } from '@/lib/useFileActions'
import { useAsyncSubscription, useLatest } from '@/lib/useAsyncSubscription'
import { useStars } from '@/lib/useStars'
import { entriesToMedia, isMediaFile, nextEpisodes } from '@/lib/media'
import type { MediaItem } from '@/lib/mockMedia'
import {
  baseName,
  folderUnder,
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
  /** The photo viewer: the photos it steps through, and which one is open. */
  const [viewer, setViewer] = useState<{ photos: MediaFile[]; index: number } | null>(null)
  const viewingIndex = viewer?.index ?? null
  const [view, setView] = useState<ViewMode>('details')
  const [sortField, setSortField] = useState<SortField>('name')
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc')
  const [notice, setNotice] = useState<string | null>(null)
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const [properties, setProperties] = useState<Entry | null>(null)
  const [dropActive, setDropActive] = useState(false)
  /** The folder an external drag is hovering, or null for the one that is open. */
  const [dropInto, setDropInto] = useState<string | null>(null)

  const connected = vault.status?.connected ?? false
  const writable = vault.status?.writable ?? false

  const { confirm, dialog: confirmDialog } = useConfirm()
  const actions = useFileActions({
    onChanged: vault.refresh,
    onError: setNotice,
    confirm,
  })

  const stars = useStars(vault.status?.hostId, nav === 'starred')

  const collections = useCollections(connected)
  const mediaBase = useMediaBase(connected)
  // Only for a host too old to sort the drive itself.
  const needsScan =
    collections.unsupported && (nav === 'recent' || LIBRARY_KEYS.includes(nav))
  const scan = useLibraryScan(needsScan, connected)

  const media = useMediaLibrary(connected)

  /** Sections the host's owner chose not to show. */
  const hiddenSections = useMemo(() => {
    const s = media.sections
    const hidden = new Set<NavKey>()
    if (!s.movies) hidden.add('movies')
    if (!s.series) hidden.add('series')
    if (!s.videos) hidden.add('videos')
    if (!s.music) hidden.add('music')
    if (!s.photos) hidden.add('photos')
    return hidden
  }, [media.sections])

  // Looking at a section the host has just hidden: back to Files.
  useEffect(() => {
    if (hiddenSections.has(nav)) setNav('files')
  }, [hiddenSections, nav])
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
  /**
   * What follows each episode, for autoplay.
   *
   * Built per series — see `nextEpisodes`. It used to be one flat list over
   * every series in the library, which meant finishing a show started an
   * unrelated one.
   */
  const nextByPath = useMemo(() => nextEpisodes(media.series), [media.series])

  /**
   * The subtitle files the host found beside a given video.
   *
   * Built from the index rather than asked for, because the index is already
   * here and a per-file question would be a round trip at the moment somebody
   * pressed play.
   */
  const subtitlesByPath = useMemo(() => {
    const map = new Map<string, SubtitleTrack[]>()
    for (const film of media.films) {
      if (film.path && film.subtitles?.length) map.set(film.path, film.subtitles)
    }
    for (const series of media.series) {
      for (const season of series.seasons) {
        for (const episode of season.episodes) {
          if (episode.subtitles?.length) map.set(episode.path, episode.subtitles)
        }
      }
    }
    return map
  }, [media.films, media.series])

  /**
   * What the player calls a file the library knows: the film's own name, or
   * the show and the episode — not the release name off the file, which is
   * what the opening card used to show.
   */
  const namesByPath = useMemo(() => {
    const map = new Map<string, { title: string; subtitle: string }>()
    for (const film of media.films) {
      if (!film.path) continue
      map.set(film.path, { title: film.title, subtitle: film.year ? String(film.year) : '' })
    }
    for (const series of media.series) {
      for (const season of series.seasons) {
        for (const episode of season.episodes) {
          const code = `S${pad2(season.number)}E${pad2(episode.number)}`
          map.set(episode.path, {
            title: series.title,
            subtitle: episode.title ? `${code} · ${episode.title}` : code,
          })
        }
      }
    }
    return map
  }, [media.films, media.series])

  const subtitlesFor = useCallback(
    (path: string): SubtitleTrack[] => subtitlesByPath.get(path) ?? [],
    [subtitlesByPath],
  )

  /**
   * Where to start a file, which is not simply where it was left.
   *
   * Something already watched starts again. Dropping straight into the last
   * ten seconds of a film somebody finished is not resuming, it is showing
   * them the credits — and it is exactly what happens when a stray progress
   * record says the position is the duration.
   */
  const resumeFor = useCallback(
    (path: string): number => {
      const entry = watchedByPath.get(path)
      if (!entry) return 0
      return isFinished(entry) ? 0 : entry.position
    },
    [watchedByPath],
  )

  /**
   * The track after each one, within its album, for music to play on.
   *
   * Within the album, not the whole library: an album finishing is the
   * natural place to stop, and running on into whatever sorts next is a
   * shuffle nobody asked for.
   */
  const nextTrack = useMemo(() => {
    const next = new Map<string, { path: string; label: string }>()
    const tracks = sortTracks(collections.collections.music)
    for (let i = 0; i < tracks.length - 1; i++) {
      const here = tracks[i]!
      const after = tracks[i + 1]!
      if (parentOf(here.path) === parentOf(after.path)) {
        next.set(here.path, { path: after.path, label: trackInfo(after.path).title })
      }
    }
    return next
  }, [collections.collections.music])

  const nextAfter = useCallback(
    (path: string): { path: string; label: string } | null =>
      nextByPath.get(path) ?? nextTrack.get(path) ?? null,
    [nextByPath, nextTrack],
  )

  // The drive is the truth: whatever changes it, the folder on screen reloads
  // and the index is asked again. Nothing here polls.
  const refreshLibrary = useCallback(() => {
    media.refresh()
    collections.refresh()
  }, [media, collections])
  useLiveChanges(vault.dir, vault.refresh, refreshLibrary)

  const sectionEntries = useMemo(() => {
    if (nav === 'recent') {
      return collections.unsupported
        ? recentOf(scan.files)
        : collections.collections.recent.map(fileToEntry)
    }
    if (nav === 'starred') return stars.entries
    return vault.entries
  }, [nav, scan.files, stars.entries, vault.entries, collections])

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

  /**
   * The files of the section on screen, as the host sorted them — or, from a
   * host too old to, as the old scan found them — narrowed by the search box.
   */
  const libraryFiles = useMemo<MediaFile[]>(() => {
    if (!isLibrary) return []
    const kind = nav as 'videos' | 'music' | 'photos'
    const all: MediaFile[] = collections.unsupported
      ? filterKind(scan.files, kind).map((e) => ({
          path: e.id,
          size: e.size,
          mtime: Math.floor(e.modified / 1000),
        }))
      : collections.collections[kind]
    const needle = query.trim().toLowerCase()
    const found = needle ? all.filter((f) => f.path.toLowerCase().includes(needle)) : all
    return kind === 'music' ? sortTracks(found) : found
  }, [isLibrary, nav, collections, scan.files, query])

  const libraryScanning = collections.unsupported ? scan.scanning : !collections.loaded || collections.scanning

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
          const outcome = await api.upload(local, joinPath(into, name), false, id)
          const failed = outcome?.failed ?? []
          if (failed.length > 0) {
            // A folder that mostly arrived is not a failed transfer, but the
            // files that did not have to be named somewhere.
            const [first, why] = failed[0]!
            transfers.finish(
              id,
              `${failed.length} of ${outcome.files + failed.length} files did not upload. ${nameOf(first)}: ${why}`,
            )
          } else {
            transfers.finish(id)
          }
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
      // A photo opens in the viewer, stepping through the photos beside it.
      // It used to be downloaded, like any file the app did not play.
      if (isKind(entry.name, 'photos')) {
        // Sizes from the host's sort where it has them: a folder listing
        // does not carry them, and the viewer lays a photo out by its shape.
        const known = new Map(collections.collections.photos.map((p) => [p.path, p]))
        const photos = entries
          .filter((e) => e.kind === 'file' && isKind(e.name, 'photos'))
          .map((e) => ({
            path: e.id,
            size: e.size,
            mtime: Math.floor(e.modified / 1000),
            width: known.get(e.id)?.width,
            height: known.get(e.id)?.height,
          }))
        const index = Math.max(0, photos.findIndex((p) => p.path === entry.id))
        setViewer({ photos, index })
        return
      }
      const media = entriesToMedia([entry])[0]
      if (isMediaFile(entry.name)) setPlaying(media ?? null)
      else void downloadOne(entry)
    },
    [vault, downloadOne, entries, collections],
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
        // A song — the next track, played on from the last — is named the
        // way the music list names it, not by its file name.
        if (item && isKind(entry.name, 'music')) {
          setPlaying(musicItem({ path, size: entry.size, mtime: entry.mtime }))
        } else if (item) {
          setPlaying({ ...item, ...namesByPath.get(path) })
        }
      } catch {
        setNotice(`${nameOf(path)} is not on the drive any more.`)
        media.refresh()
      }
    },
    [media, namesByPath],
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

      // Only when opening means something other than downloading. For a
      // file that is not media, "Open" and "Download…" would be the same
      // action listed twice.
      const canOpen = entry.kind === 'dir' || isMediaFile(entry.name)
      if (canOpen && !many) {
        items.push({
          id: 'open',
          label: entry.kind === 'dir' ? 'Open' : 'Play',
          icon: entry.kind === 'dir' ? FolderOpen : PlayCircle,
          run: () => openEntry(entry),
        })
      }

      // Only for media. This used to be offered for every file, so a PDF and
      // a zip archive were both advertised as something to watch — and the
      // external player would be launched and left with nothing to do.
      if (!many && entry.kind === 'file' && isMediaFile(entry.name)) {
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
      onSelectSet: (ids) => {
        setSelected(ids)
        // Shift-click after a drag box extends from where the box began.
        anchor.current = entries.find((e) => ids.has(e.id))?.id ?? null
      },
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
    // The player and the photo viewer have the keyboard to themselves. These
    // stayed live behind them, so Delete while watching deleted the files
    // still selected in the folder underneath, Enter reopened one, and
    // Backspace walked the hidden list up a folder.
    if (playing || viewingIndex !== null) return undefined
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
    playing,
    viewingIndex,
  ])

  // Anything floating over the app is put away when a film starts, rather
  // than left open behind it to be found again afterwards.
  const closeMenu = menu.close
  useEffect(() => {
    if (!playing) return
    setPaletteOpen(false)
    closeMenu()
  }, [playing, closeMenu])

  // --- files dragged in from Explorer --------------------------------------

  // Read through refs so the subscription below can be established once.
  // Depending on these directly re-ran the effect on every render, and because
  // the unsubscribe arrives asynchronously the old listener was never removed —
  // so a single drop started one upload per render that had happened since the
  // app opened.
  const latestUpload = useLatest(uploadPaths)
  const latestDir = useLatest(vault.dir)
  // While a video plays, a drop belongs to the player — subtitles — and is
  // not an upload into the folder hidden behind it.
  const latestPlaying = useLatest(playing !== null)

  const subscribeToDrops = useCallback(
    () =>
      onExternalFileDrop({
        onEnter: () => {
          if (!latestPlaying.current) setDropActive(true)
        },
        // Hovering a folder aims at that folder; anywhere else means the
        // folder currently open. Both are shown before letting go, because
        // "which folder did that just go into" is not a question anyone should
        // have to answer by going and looking.
        onOver: (x, y) => {
          if (!latestPlaying.current) setDropInto(folderUnder(x, y))
        },
        onLeave: () => {
          setDropActive(false)
          setDropInto(null)
        },
        onDrop: (paths, x, y) => {
          // Read from the drop's own position rather than from the last `over`:
          // they are normally the same, but a drop that arrives without a
          // preceding hover would otherwise use a stale target.
          setDropActive(false)
          setDropInto(null)
          if (latestPlaying.current) return
          const into = folderUnder(x, y) ?? latestDir.current
          void latestUpload.current(paths, into)
        },
      }),
    [latestUpload, latestDir, latestPlaying],
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
    if (!hostId) {
      // Never a silent return. This button has now failed to do anything
      // twice, for two unrelated reasons, and both times the only symptom was
      // a click that produced nothing at all. Whatever goes wrong next, it
      // says so on screen.
      setNotice('There is no paired vault to forget.')
      return
    }
    // Inside the try, all of it. With the confirmation outside, anything that
    // went wrong in it escaped this callback entirely and the button did
    // nothing at all, silently.
    try {
      const ok = await confirm({
        title: 'Forget this vault?',
        message:
          'This device will have to pair again with a new PIN. Nothing on the drive is affected.',
        confirmLabel: 'Forget it',
        danger: true,
      })
      if (!ok) return

      const next = await api.forgetHost(hostId)
      vault.adopt({ ...next, hasPaired: false, connected: false })
      setNav('files')
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e))
    }
  }, [vault, confirm])

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
          <PairingView onPaired={vault.adopt} />
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
    dropHighlight: dropInto,
    handlers,
    onBackgroundContextMenu: (event: { clientX: number; clientY: number }) => {
      setSelected(new Set())
      menu.open(event, backgroundActions())
    },
  }

  return (
    <div className="relative flex h-full flex-col">
      {/*
        Everything behind the player stops drawing while it is up.

        mpv renders into the native window *behind* this page, so the page has
        to be genuinely transparent for the picture to reach the screen — and
        a transparent `body` is not enough while the app's own sidebar, grid
        and backdrop are still painting over the top of it. The first build
        showed exactly that: the controls worked, the clock ran, and what you
        saw where the film should be was the library.

        `visibility: hidden` rather than unmounting: the view underneath keeps
        its scroll position and its state, so closing the player puts you back
        where you were.
      */}
      {/*
        And inert: nothing behind the player can be focused, clicked or typed
        into. The file a film was opened from used to keep focus, so keys
        meant for the player reached the list as well.
      */}
      <div
        className="contents"
        style={{ visibility: playing ? 'hidden' : 'visible' }}
        inert={playing !== null}
      >
      <div className="backdrop" />
      <TitleBar vaultName={vault.status.vault ?? 'Vault'} connected={connected} />

      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        <Sidebar
          hidden={hiddenSections}
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
                        ? libraryFiles.length
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
              libraryFiles.length === 0 ? (
                <EmptyState
                  label={
                    libraryScanning
                      ? 'Looking through the drive…'
                      : query
                        ? `Nothing matches “${query}”`
                        : `No ${nav} found`
                  }
                />
              ) : nav === 'photos' ? (
                <PhotoGrid
                  files={libraryFiles}
                  base={mediaBase}
                  onOpen={(index) => setViewer({ photos: libraryFiles, index })}
                />
              ) : nav === 'music' ? (
                <MusicList
                  files={libraryFiles}
                  playing={playing?.id ?? null}
                  onPlay={(file) => setPlaying(musicItem(file))}
                />
              ) : (
                <VideoGrid
                  files={libraryFiles}
                  base={mediaBase}
                  progressOf={(path) => {
                    const entry = watchedByPath.get(path)
                    return entry && entry.duration > 0 && !isFinished(entry)
                      ? entry.position / entry.duration
                      : undefined
                  }}
                  onPlay={(file) => void playPath(file.path)}
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
                    // Not yet connected is not an empty drive. For the first
                    // seconds after launch this said "This folder is empty",
                    // which reads as the drive having been wiped.
                    !connected
                      ? vault.error
                        ? 'Not connected to the host'
                        : 'Connecting to the host…'
                      : vault.loading || scan.scanning
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

            <DropOverlay active={dropActive && nav === 'files'} dir={dropInto ?? vault.dir} />
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

      </div>

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
        resumeAt={playing ? resumeFor(playing.id) : 0}
        onProgress={watched.report}
        nextUp={playing ? nextAfter(playing.id) : null}
        onPlayNext={(path) => void playPath(path)}
        subtitles={playing ? subtitlesFor(playing.id) : []}
      />

      <ImageViewer
        photos={viewer?.photos ?? []}
        index={viewingIndex}
        base={mediaBase}
        onIndexChange={(index) => setViewer((v) => (v ? { ...v, index } : v))}
        onClose={() => setViewer(null)}
        onDownload={(path) => {
          const photo = viewer?.photos.find((p) => p.path === path)
          if (photo) void downloadOne(fileToEntry(photo))
        }}
      />

      <PropertiesPanel
        entry={properties}
        vaultName={vault.status.vault ?? 'Vault'}
        onClose={() => setProperties(null)}
      />

      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      {confirmDialog}
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
          <div className="max-w-[80%] text-center">
            <Upload size={26} className="mx-auto text-basaltDeep" />
            <p className="mt-3 text-sm text-text">
              Drop to upload into {dir ? nameOf(dir) : 'the vault'}
            </p>
            {/* The whole path, not just the last part of it.
                A folder called `Season 1` is three of those on this drive, and
                a name alone cannot tell you which one you are about to drop
                into — which is the entire question being asked at the moment
                somebody is holding files over a window. */}
            <p className="tnum mt-1.5 break-all font-mono text-[11px] text-textFaint">
              {dir ? `/${dir}` : '/'}
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
/**
 * Why the folder on screen is not what the drive holds right now.
 *
 * Every listing failure is said here. Only losing the host used to be: any
 * other failure — the host's drive unplugged, a folder it would not open —
 * left an empty folder on screen and no word about why, which reads as the
 * drive having been wiped.
 */
function ConnectionBanner({
  vault,
}: {
  vault: ReturnType<typeof useVault>
}): React.JSX.Element | null {
  const kind = vault.error?.kind
  if (!kind) return null

  const offline = kind === 'offline' || kind === 'unpaired'
  const waiting = kind === 'unavailable'
  const text = offline
    ? vault.reconnecting
      ? 'Reconnecting…'
      : 'Lost the host. Trying again in the background.'
    : waiting
      ? `${capitalise(vault.error?.message ?? 'The drive is not connected')}. It will be back here as soon as it is plugged in again.`
      : capitalise(vault.error?.message ?? 'Something went wrong')

  return (
    <motion.div
      initial={{ height: 0, opacity: 0 }}
      animate={{ height: 'auto', opacity: 1 }}
      transition={{ duration: 0.2 }}
      className="shrink-0 overflow-hidden border-b border-line bg-ink2"
    >
      <div className="flex items-center gap-2.5 px-4 py-2">
        {offline || waiting ? (
          <Loader2 size={13} className="shrink-0 animate-spin text-textFaint" />
        ) : (
          <AlertTriangle size={13} className="shrink-0 text-danger" />
        )}
        <span className={cn('text-[12px]', offline || waiting ? 'text-textDim' : 'text-danger')}>
          {text}
        </span>
        <button
          onClick={() => (offline ? void vault.reconnect() : vault.refresh())}
          className="ml-auto shrink-0 rounded px-2 py-0.5 text-[11px] text-textDim transition-colors hover:bg-white/[0.05] hover:text-text"
        >
          {offline ? 'Retry now' : waiting ? 'Check now' : 'Try again'}
        </button>
      </div>
    </motion.div>
  )
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function pad2(n: number): string {
  return String(n).padStart(2, '0')
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

/** A song as the player shows it: its title, and who and what it is from. */
function musicItem(file: MediaFile): MediaItem {
  const base = entriesToMedia([fileToEntry(file)])[0]!
  const info = trackInfo(file.path)
  return {
    ...base,
    title: info.title || stemOf(file.path),
    subtitle: [info.artist, info.album].filter(Boolean).join(' · ') || 'Music',
  }
}
