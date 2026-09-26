/**
 * The bridge to the Rust client.
 *
 * Every call goes through `call()`, which falls back to mock data when the app
 * is running in a plain browser rather than inside Tauri. That is not a
 * convenience: it is what lets the whole interface be opened, resized, clicked
 * through and measured in a browser without a host on the network — which is
 * how the layout and performance work has been done all along.
 *
 * Errors arrive as `{ kind, message }` and are rethrown as `ApiError`, so the
 * UI can branch on `kind` instead of matching on prose.
 */

import type { Entry } from '@/components/FileList'
import { generateEntries } from './mockData'

export interface DirEntry {
  name: string
  kind: 'dir' | 'file'
  size: number
  /** Unix seconds, as the host reports them. */
  mtime: number
  readonly: boolean
}

export interface Status {
  connected: boolean
  hostId: string | null
  hostName: string | null
  vault: string | null
  writable: boolean
  address: string | null
  /** Whether this device has ever paired. Distinguishes "host asleep" from
   *  "never set up", which are different screens. */
  hasPaired: boolean
  deviceName: string
}

/**
 * A host found on the network.
 *
 * This is what replaced typing an address. The address is still here because it
 * is worth showing in small type, but nobody enters one: the identity is what
 * gets pinned, and the address is only how this device reached it today.
 */
export interface DiscoveredHost {
  hostId: string
  hostName: string
  vault: string
  address: string
  requiresPin: boolean
  /** False until somebody has chosen a drive on that machine. */
  hasVault: boolean
  /** Whether this device has already paired with it. */
  paired: boolean
}

export interface TransferEvent {
  id: string
  kind: 'download' | 'upload'
  name: string
  path: string
  transferred: number
  total: number
  status: 'active' | 'done' | 'failed'
  /** Bytes per second now, over the last couple of seconds. */
  rate: number
  /** Bytes per second over a longer window, for the time left. */
  etaRate: number
}

/**
 * Something that happened on the drive.
 *
 * Reported by the host watching the filesystem, so a file deleted in Explorer
 * arrives exactly like one deleted here. `resynchronise` means the host stopped
 * counting — too many changes at once, or the watch reconnected — and the only
 * correct response is to reload whatever is on screen.
 */
export type Change =
  | { kind: 'created'; path: string }
  | { kind: 'removed'; path: string }
  | { kind: 'modified'; path: string }
  | { kind: 'renamed'; from: string; to: string }
  | { kind: 'resynchronise' }
  | { kind: 'library_changed' }

/** A subtitle file the host found beside a film or an episode. */
export interface SubtitleTrack {
  /** Vault-relative path. */
  path: string
  /** `English`, `Spanish forced`, or `Subtitles` when the name says nothing. */
  label: string
}

export interface LibraryEpisode {
  number: number
  path: string
  title?: string | null
  size: number
  added: number
  /** Absent rather than empty when there are none — the host omits the field. */
  subtitles?: SubtitleTrack[]
}

export interface LibrarySeason {
  number: number
  episodes: LibraryEpisode[]
}

/** A film, or a series with its seasons. */
export interface LibraryItem {
  id: string
  kind: 'film' | 'series'
  title: string
  year?: number | null
  /** The file to play, for a film. */
  path?: string | null
  size: number
  /** Unix seconds of the newest file in this item. */
  added: number
  seasons: LibrarySeason[]
  /** Subtitle files beside a film. Empty for a series — its episodes carry
   *  their own, and a series-level list would mean nothing. */
  subtitles?: SubtitleTrack[]
  /** 0-100. Below CONFIDENT the interface offers a correction rather than
   *  asserting the match. */
  confidence: number
  /** Whether the host has a poster for this item. Saves asking for one that
   *  is not there — a library of five hundred would otherwise be five hundred
   *  requests that all come back empty. */
  hasArt: boolean
}

export interface LibraryResponse {
  revision: number
  enabled: boolean
  scanning: boolean
  /** Absent when the revision asked for is still current. */
  items?: LibraryItem[] | null
  /** Which sections the host's owner wants shown. Absent from an older host. */
  sections?: Sections
}

/** What an upload did. For a folder, some of its files may not have arrived. */
export interface UploadOutcome {
  bytes: number
  files: number
  /** Vault path and reason, for each file that did not arrive. */
  failed: Array<[string, string]>
}

/** The library sections this device shows. Mirrors the Rust. */
export interface Sections {
  movies: boolean
  series: boolean
  videos: boolean
  music: boolean
  photos: boolean
}

export const ALL_SECTIONS: Sections = {
  movies: true,
  series: true,
  videos: true,
  music: true,
  photos: true,
}

/** One file in a collection. Mirrors the Rust. */
export interface MediaFile {
  /** Vault-relative. */
  path: string
  size: number
  /** Unix seconds. */
  mtime: number
  /** Photos only: pixels as the photo is meant to be seen. */
  width?: number | null
  height?: number | null
}

/** Media on the drive, sorted by the host, newest first. */
export interface Collections {
  videos: MediaFile[]
  music: MediaFile[]
  photos: MediaFile[]
  recent: MediaFile[]
  truncated: boolean
}

export interface CollectionsResponse {
  revision: number
  scanning: boolean
  /** Absent when the revision asked for is still current. */
  collections?: Collections | null
}

/** Below this, a match is a guess worth showing the user. Mirrors the Rust. */
export const CONFIDENT = 70

/**
 * How far through a file somebody got.
 *
 * A fraction rather than a timestamp, because the in-app player knows seconds
 * and an external one gives away only how far through the file it has read.
 * Storing a fraction makes both the same kind of answer.
 */
export interface Watched {
  /** The file itself: an episode is watched, a series is not. */
  path: string
  /** 0-1 through the file. Always present. */
  fraction: number
  /** Seconds in. Zero when only a byte offset was observable. */
  position: number
  /** Total seconds. Zero when unknown. */
  duration: number
  updatedAt: number
}

/** Past this, it counts as watched. Credits run long. Mirrors the Rust. */
export const FINISHED_AT = 0.94
/** Before this, there is nothing worth resuming. Mirrors the Rust. */
export const STARTED_AFTER = 0.01

export function isFinished(watched: Watched): boolean {
  return watched.fraction >= FINISHED_AT
}

export function inProgress(watched: Watched): boolean {
  return watched.fraction > STARTED_AFTER && !isFinished(watched)
}

export type ErrorKind =
  | 'offline'
  | 'notfound'
  /** The host is there, but the drive it shares is not connected. */
  | 'unavailable'
  | 'denied'
  | 'exists'
  | 'notempty'
  | 'unpaired'
  | 'wronghost'
  | 'incompatible'
  | 'pairing'
  | 'error'

export class ApiError extends Error {
  constructor(
    readonly kind: ErrorKind,
    message: string,
  ) {
    super(message)
    this.name = 'ApiError'
  }
}

/** Whether the app is running inside the desktop shell. */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/* eslint-disable @typescript-eslint/no-explicit-any */
type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<any>

let invokeFn: Invoke | null = null

async function getInvoke(): Promise<Invoke> {
  if (!invokeFn) {
    const mod = await import('@tauri-apps/api/core')
    invokeFn = mod.invoke as Invoke
  }
  return invokeFn
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) return mock<T>(cmd, args)
  const invoke = await getInvoke()
  try {
    return (await invoke(cmd, args)) as T
  } catch (raw: unknown) {
    const e = raw as { kind?: string; message?: string }
    throw new ApiError(
      (e?.kind as ErrorKind) ?? 'error',
      e?.message ?? String(raw),
    )
  }
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/** A release newer than the one running. Mirrors `basalt_update::Release`. */
export interface Release {
  version: string
  /** The release notes, as written on GitHub. */
  notes: string
  pageUrl: string
  installerName: string
  installerUrl: string
  installerBytes: number
  checksumUrl: string | null
}

export const api = {
  /** Which version this is, as the release tags spell it. */
  appVersion: (): Promise<string> => call<string>('app_version'),
  /** A newer release, or null when this is the newest. */
  checkUpdate: (): Promise<Release | null> => call<Release | null>('check_update'),
  /** Fetches and verifies an installer, returning where it landed. */
  downloadUpdate: (release: Release): Promise<string> =>
    call<string>('download_update', { release }),
  /** Runs the installer and closes this app so it can be replaced. */
  installUpdate: (path: string): Promise<void> => call<void>('install_update', { path }),

  status: () => call<Status>('status'),
  /** Every host answering on this network. Takes about a second. */
  discover: () => call<DiscoveredHost[]>('discover'),
  /** Asks a host to pair. Resolves to whether it wants a PIN. */
  beginPairing: (address: string) => call<boolean>('begin_pairing', { address }),
  /** Completes it. Pass an empty string when no PIN was asked for. */
  finishPairing: (pin: string) => call<Status>('finish_pairing', { pin }),
  cancelPairing: () => call<void>('cancel_pairing'),

  connectSaved: () => call<Status>('connect_saved'),
  connectTo: (hostId: string, address?: string) =>
    call<Status>('connect_to', { hostId, address: address ?? null }),
  disconnect: () => call<Status>('disconnect'),
  forgetHost: (hostId: string) => call<Status>('forget_host', { hostId }),

  library: (knownRevision: number) =>
    call<LibraryResponse>('library', { knownRevision }),
  /** Every video, song and photo on the drive, sorted by the host. */
  collections: (knownRevision: number) =>
    call<CollectionsResponse>('collections', { knownRevision }),
  /** The start of every media URL. A percent-encoded path goes on the end. */
  mediaBase: () => call<string>('media_base'),
  /** Poster bytes for one item, as a data URL the interface can hand to an
   *  `<img>`. Null when the host has none. */
  art: (id: string) => call<string | null>('library_art', { id }),
  /** Reports a position and reads back everything watched, in one trip. */
  watchProgress: (update?: Watched, forget?: string) =>
    call<Watched[]>('watch_progress', {
      update: update ?? null,
      forget: forget ?? null,
    }),

  list: (path: string) => call<DirEntry[]>('list_dir', { path }),
  stat: (path: string) => call<DirEntry>('stat_entry', { path }),
  copy: (from: string, to: string) => call<void>('copy_entry', { from, to }),
  space: () => call<[number, number]>('space'),
  mkdir: (path: string) => call<void>('make_dir', { path }),
  rename: (from: string, to: string) => call<void>('rename_entry', { from, to }),
  remove: (path: string, recursive: boolean) =>
    call<void>('remove_entry', { path, recursive }),
  mediaUrl: (path: string) => call<string>('media_url', { path }),

  download: (remote: string, local: string, id: string) =>
    call<number>('download', { remote, local, id }),
  /** A file, or a folder with everything in it. */
  upload: (local: string, remote: string, overwrite: boolean, id: string) =>
    call<UploadOutcome>('upload', { local, remote, overwrite, id }),
  cancelTransfer: (id: string) => call<boolean>('cancel_transfer', { id }),
  /**
   * Hands a file to a player that can decode it — streamed over a local URL
   * where possible, copied out only when nothing streaming-capable is
   * installed.
   */
  openExternally: (remote: string, id: string) =>
    call<OpenResult>('open_externally', { remote, id }),
  /** The name of an installed player that can stream, if there is one. */
  externalPlayer: () => call<string | null>('external_player'),
}

/** Subscribes to transfer progress. Returns an unsubscribe function. */
export async function onTransfer(
  handler: (event: TransferEvent) => void,
): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  const stop = await listen<TransferEvent>('basalt://transfer', (e) =>
    handler(e.payload),
  )
  return stop
}

/** Bytes that crossed the link, and the interval they were measured over. */
export interface OpenResult {
  player: string
  /** False when the file had to be copied out first. */
  streamed: boolean
}

export interface ByteWindow {
  bytes: number
  millis: number
}

/**
 * Subscribes to bytes crossing the link.
 *
 * Separate from transfer progress because most of what moves is not a
 * transfer: streaming a film runs through the media proxy and would otherwise
 * leave the throughput trace flat while the link is saturated.
 *
 * The interval comes with the bytes deliberately — deriving it from when the
 * event arrived is what made every displayed speed roughly twice the truth.
 */
export async function onBytes(
  handler: (window: ByteWindow) => void,
): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  const stop = await listen<ByteWindow>('basalt://bytes', (e) =>
    handler(e.payload),
  )
  return stop
}

/**
 * Subscribes to changes on the drive.
 *
 * Returns an unsubscribe function. Registered asynchronously, so the caller
 * has to handle a cleanup that runs before registration finishes — see
 * `useAsyncSubscription`, which exists because ignoring that once turned one
 * dropped file into eight uploads.
 */
export async function onChange(
  handler: (change: Change) => void,
): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<Change>('basalt://change', (e) => handler(e.payload))
}

/** Subscribes to connection changes pushed by the backend. */
export async function onStatus(
  handler: (status: Status) => void,
): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  const stop = await listen<Status>('basalt://status', (e) => handler(e.payload))
  return stop
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/** Turns a host listing into the shape the list components already use. */
export function toEntries(dir: string, entries: DirEntry[]): Entry[] {
  return entries.map((e) => ({
    // The full vault path is the identity: two files can share a name in
    // different folders, and selection state is keyed on this.
    id: dir ? `${dir}/${e.name}` : e.name,
    name: e.name,
    kind: e.kind,
    size: e.size,
    // The host speaks Unix seconds; everything in the interface is
    // milliseconds, and mixing the two silently shows dates in 1970.
    modified: e.mtime * 1000,
  }))
}

/** Joins a vault path, tolerating the empty root. */
export function joinPath(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name
}

/** The parent of a vault path, or `''` at the root. */
export function parentOf(path: string): string {
  const cut = path.lastIndexOf('/')
  return cut === -1 ? '' : path.slice(0, cut)
}

// ---------------------------------------------------------------------------
// Browser fallback
// ---------------------------------------------------------------------------

/**
 * Stand-in responses for running outside the desktop shell.
 *
 * Kept small and obviously fake — a demo vault, not a simulation. Anything
 * cleverer would invite testing behaviour here that has never run against a
 * real host.
 */
const MOCK_STATUS: Status = {
  connected: true,
  hostId: 'demo0000',
  hostName: 'Demo Host',
  vault: 'Vault',
  writable: true,
  address: '127.0.0.1:7742',
  hasPaired: true,
  deviceName: 'Browser',
}

/**
 * `?unpaired` in the preview opens on the pairing screen.
 *
 * Without this the screen is unreachable in a browser, because the stand-in
 * status is always connected — and pairing is by design the one screen a user
 * sees once and never again, so it would otherwise be the least reviewable
 * part of the app rather than the most.
 */
function previewIsUnpaired(): boolean {
  return previewFlag('unpaired')
}

function previewFlag(name: string): boolean {
  return (
    typeof window !== 'undefined' &&
    new URLSearchParams(window.location.search).has(name)
  )
}

/** The version the preview claims to be. */
const MOCK_VERSION = '1.0.0'

/**
 * `?update` in the preview offers one, for the same reason as `?unpaired`.
 *
 * An update offer is by definition something the app shows on a day nobody
 * chose, so without this the panel could only ever be reviewed by publishing
 * a release — which is a poor moment to discover the notes do not fit.
 */
const MOCK_RELEASE: Release = {
  version: '1.1.0',
  notes: [
    '## New',
    '',
    '* **Subtitle search** — find a line of dialogue and jump to it.',
    '* **Two drives at once**, if the host has two.',
    '',
    '## Fixed',
    '',
    '* Seeking in a file still being written no longer stalls the player.',
  ].join('\n'),
  pageUrl: 'https://example.test/releases/v1.1.0',
  installerName: 'Basalt-Client-1.1.0-setup.exe',
  installerUrl: 'https://example.test/Basalt-Client-1.1.0-setup.exe',
  installerBytes: 35_600_000,
  checksumUrl: 'https://example.test/Basalt-Client-1.1.0-setup.exe.sha256',
}

/**
 * Three hosts, covering the three rows the list has to draw.
 *
 * In the order `basalt_client::ui::sort_hosts` would return them — already
 * paired first, then ones with a drive, then by name. The preview teaching a
 * different order from the app would be worse than no preview.
 */
const MOCK_HOSTS: DiscoveredHost[] = [
  {
    hostId: 'known111aa11bb22cc33dd44ee55ff66',
    hostName: 'ATTIC-PC',
    vault: 'Backups',
    address: '192.168.1.42:7742',
    requiresPin: false,
    hasVault: true,
    paired: true,
  },
  {
    hostId: 'demo0000aa11bb22cc33dd44ee55ff66',
    hostName: 'STUDY-LAPTOP',
    vault: 'Films',
    address: '192.168.1.90:7742',
    requiresPin: true,
    hasVault: true,
    paired: false,
  },
  {
    hostId: 'empty222aa11bb22cc33dd44ee55ff66',
    hostName: 'DESKTOP-4F2A',
    vault: 'Vault',
    address: '192.168.1.17:7742',
    requiresPin: true,
    hasVault: false,
    paired: false,
  },
]

/** A small library, covering both kinds and an uncertain match. */
const MOCK_LIBRARY: LibraryItem[] = [
  {
    id: 'f1',
    kind: 'film',
    title: 'Arrival',
    year: 2016,
    path: 'Films/Arrival (2016)/Arrival.2016.2160p.mkv',
    size: 24 * 1024 ** 3,
    added: Math.floor(Date.now() / 1000) - 86_400 * 3,
    seasons: [],
    confidence: 95,
    hasArt: false,
  },
  {
    id: 'f2',
    kind: 'film',
    title: 'Blade Runner 2049',
    year: 2017,
    path: 'Films/Blade Runner 2049 (2017).mkv',
    size: 31 * 1024 ** 3,
    added: Math.floor(Date.now() / 1000) - 86_400 * 30,
    seasons: [],
    confidence: 92,
    hasArt: false,
  },
  {
    id: 'f3',
    kind: 'film',
    title: 'Holiday Footage',
    year: null,
    path: 'Films/Holiday Footage.mkv',
    size: 2 * 1024 ** 3,
    added: Math.floor(Date.now() / 1000) - 86_400 * 200,
    seasons: [],
    confidence: 55,
    hasArt: false,
  },
  {
    id: 's1',
    kind: 'series',
    title: 'Breaking Bad',
    year: 2008,
    path: null,
    size: 180 * 1024 ** 3,
    added: Math.floor(Date.now() / 1000) - 86_400 * 12,
    seasons: [
      {
        number: 1,
        episodes: Array.from({ length: 7 }, (_, i) => ({
          number: i + 1,
          path: `Shows/Breaking Bad/Season 01/S01E0${i + 1}.mkv`,
          title: null,
          size: 3 * 1024 ** 3,
          added: Math.floor(Date.now() / 1000) - 86_400 * 12,
        })),
      },
      {
        number: 2,
        episodes: Array.from({ length: 13 }, (_, i) => ({
          number: i + 1,
          path: `Shows/Breaking Bad/Season 02/S02E${String(i + 1).padStart(2, '0')}.mkv`,
          title: null,
          size: 3 * 1024 ** 3,
          added: Math.floor(Date.now() / 1000) - 86_400 * 11,
        })),
      },
    ],
    confidence: 95,
    hasArt: false,
  },
  {
    id: 's2',
    kind: 'series',
    title: 'The Wire',
    year: 2002,
    path: null,
    size: 90 * 1024 ** 3,
    added: Math.floor(Date.now() / 1000) - 86_400 * 60,
    seasons: [
      {
        number: 1,
        episodes: Array.from({ length: 13 }, (_, i) => ({
          number: i + 1,
          path: `Shows/The Wire/Season 01/S01E${String(i + 1).padStart(2, '0')}.mkv`,
          title: null,
          size: 2 * 1024 ** 3,
          added: Math.floor(Date.now() / 1000) - 86_400 * 60,
        })),
      },
    ],
    confidence: 95,
    hasArt: false,
  },
]

/** Two things part-watched, so Continue watching has something to draw. */
const mockWatched = new Map<string, Watched>([
  [
    'Films/Blade Runner 2049 (2017).mkv',
    {
      path: 'Films/Blade Runner 2049 (2017).mkv',
      fraction: 0.42,
      position: 4_200,
      duration: 9_780,
      updatedAt: Math.floor(Date.now() / 1000) - 3_600,
    },
  ],
  [
    'Shows/Breaking Bad/Season 01/S01E03.mkv',
    {
      path: 'Shows/Breaking Bad/Season 01/S01E03.mkv',
      fraction: 0.71,
      position: 2_130,
      duration: 3_000,
      updatedAt: Math.floor(Date.now() / 1000) - 600,
    },
  ],
])

let mockEntries: Entry[] | null = null

async function mock<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case 'app_version':
      return MOCK_VERSION as T
    case 'check_update':
      // Slow on purpose, like `discover`: the panel has a "Checking…" state
      // and an instant answer would hide it.
      await new Promise((resolve) => setTimeout(resolve, 700))
      return (previewFlag('update') ? MOCK_RELEASE : null) as T
    case 'download_update': {
      // Progress arrives as an event in the app, so the preview emits the
      // same event rather than resolving straight to a finished download.
      const release = args?.release as Release
      const total = release.installerBytes
      for (let had = 0; had < total; had += Math.ceil(total / 12)) {
        await new Promise((resolve) => setTimeout(resolve, 160))
        window.dispatchEvent(
          new CustomEvent('basalt://update-progress', {
            detail: [Math.min(had, total), total],
          }),
        )
      }
      return `C:\\Users\\preview\\Downloads\\${release.installerName}` as T
    }
    case 'install_update':
      return undefined as T
    case 'status':
      return (
        previewIsUnpaired()
          ? { ...MOCK_STATUS, connected: false, hasPaired: false }
          : MOCK_STATUS
      ) as T
    case 'connect_saved':
    case 'connect_to':
      return MOCK_STATUS as T
    case 'disconnect':
    case 'forget_host':
      return { ...MOCK_STATUS, connected: false } as T
    case 'discover':
      // A slow answer on purpose: the real scan waits out a broadcast window
      // of about a second, and a list that appears instantly in the preview
      // would hide whatever the waiting state looks like.
      await new Promise((resolve) => setTimeout(resolve, 900))
      return MOCK_HOSTS as T
    case 'begin_pairing':
      return MOCK_HOSTS.some(
        (host) => host.address === args?.address && host.requiresPin,
      ) as T
    case 'finish_pairing':
      return MOCK_STATUS as T
    case 'cancel_pairing':
      return undefined as T
    case 'watch_progress': {
      const update = args?.update as Watched | null | undefined
      if (update?.path) {
        mockWatched.set(update.path, {
          ...update,
          updatedAt: Math.floor(Date.now() / 1000),
        })
      }
      const forget = args?.forget as string | null | undefined
      if (forget) mockWatched.delete(forget)
      return [...mockWatched.values()].sort((a, b) => b.updatedAt - a.updatedAt) as T
    }
    case 'library_art':
      // No sample artwork: the preview shows the generated posters, which is
      // also what anyone without a TMDb key sees.
      return null as T
    case 'library':
      return {
        revision: 1,
        enabled: true,
        scanning: false,
        // Only when the caller does not already have revision 1, exactly as
        // the host behaves — so the preview exercises the same path.
        items: args?.knownRevision === 1 ? null : MOCK_LIBRARY,
      } as T
    case 'space':
      return [1_842_000_000_000, 4_000_000_000_000] as T
    case 'list_dir': {
      // One large generated listing at the root, so the virtualised list and
      // the sort menu are exercised exactly as they are against a real drive.
      if (!mockEntries) mockEntries = generateEntries(100_000)
      const path = (args?.path as string) ?? ''
      const entries: DirEntry[] = (path ? mockEntries.slice(0, 40) : mockEntries).map(
        (e) => ({
          name: e.name,
          kind: e.kind,
          size: e.size,
          mtime: Math.floor(e.modified / 1000),
          readonly: false,
        }),
      )
      return entries as T
    }
    case 'stat_entry': {
      const path = (args?.path as string) ?? ''
      const name = path.split('/').pop() ?? path
      return {
        name,
        kind: name.includes('.') ? 'file' : 'dir',
        size: 1024 * 1024,
        mtime: Math.floor(Date.now() / 1000),
        readonly: false,
      } as T
    }
    case 'copy_entry':
      return undefined as T
    case 'open_externally':
      return { player: 'VLC', streamed: true } as T
    case 'external_player':
      return 'VLC' as T
    case 'media_url':
      return '' as T
    case 'media_base':
      // No thumbnails in the browser preview: tiles show their placeholder,
      // which is what a host without pictures looks like too.
      return '' as T
    case 'collections':
      return {
        revision: 1,
        scanning: false,
        collections: args?.knownRevision === 1 ? null : mockCollections(),
      } as T
    case 'make_dir':
    case 'rename_entry':
    case 'remove_entry':
      return undefined as T
    default:
      return undefined as T
  }
}

/** A small spread of every kind, so each section has something to show. */
function mockCollections(): Collections {
  const now = Math.floor(Date.now() / 1000)
  const day = 86_400
  const photos: MediaFile[] = Array.from({ length: 48 }, (_, i) => {
    const portrait = i % 3 === 1
    const square = i % 7 === 0
    return {
      path: `Photos/${2026 - Math.floor(i / 20)}/Trip/IMG_${String(4100 + i).padStart(4, '0')}.jpg`,
      size: 2_400_000 + i * 31_000,
      mtime: now - i * day * 4,
      width: square ? 3000 : portrait ? 3000 : 4000,
      height: square ? 3000 : portrait ? 4000 : 3000,
    }
  })
  const videos: MediaFile[] = Array.from({ length: 14 }, (_, i) => ({
    path: `Home Videos/Clip ${String(i + 1).padStart(2, '0')}.mp4`,
    size: 180_000_000 + i * 9_000_000,
    mtime: now - i * day * 6,
  }))
  const music: MediaFile[] = Array.from({ length: 24 }, (_, i) => ({
    path: `Music/The Quiet Coast/Harbour Lights/${String(i + 1).padStart(2, '0')} Track ${i + 1}.flac`,
    size: 28_000_000 + i * 400_000,
    mtime: now - i * day,
  }))
  const recent = [...photos.slice(0, 6), ...videos.slice(0, 4), ...music.slice(0, 4)].sort(
    (a, b) => b.mtime - a.mtime,
  )
  return { videos, music, photos, recent, truncated: false }
}
