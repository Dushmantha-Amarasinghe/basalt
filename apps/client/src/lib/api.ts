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

import packageInfo from '../../package.json'
import type { Entry } from '@/components/FileList'
import * as showcase from './showcase'
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
  /** The picture size, measured by the host. Absent until it knows. */
  resolution?: Resolution | null
}

/** A video's picture size in pixels. Mirrors the Rust. */
export interface Resolution {
  width: number
  height: number
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
  /** A film's picture size. For a series, each episode has its own. */
  resolution?: Resolution | null
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

/** A profile, as any device sees it: never its PIN. Mirrors the Rust. */
export interface ProfileView {
  id: string
  name: string
  /** Which avatar colour, 0 to 7. */
  color: number
  /** False after the host reset the PIN: signing in chooses a new one. */
  hasPin: boolean
  lastUsed: number
}

/** Who is using this device. */
export interface IdentityState {
  profile: ProfileView | null
  /** Ask who is using the device. */
  choose: boolean
  /** Signed out by the host since the app last looked. */
  ended: boolean
  /** The profile last signed in to here, shown first. */
  lastProfile: string | null
}

export interface Star {
  path: string
  name: string
  kind: 'file' | 'dir'
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
  /** The host removed this device; its pairing is gone here too. */
  | 'removed'
  | 'wronghost'
  | 'incompatible'
  | 'pairing'
  /** A profile sign-in the host has ended. */
  | 'signedout'
  | 'unsupported'
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
  /** Watches afresh, for an app coming back to the screen. */
  rewatch: () => call<void>('rewatch'),
  forgetHost: (hostId: string) => call<Status>('forget_host', { hostId }),

  library: (knownRevision: number) =>
    call<LibraryResponse>('library', { knownRevision }),
  identity: () => call<IdentityState>('identity'),
  profiles: () => call<ProfileView[]>('profiles'),
  createProfile: (name: string, pin: string, color: number, remember: boolean) =>
    call<ProfileView>('create_profile', { name, pin, color, remember }),
  signInProfile: (id: string, pin: string, remember: boolean) =>
    call<ProfileView>('sign_in_profile', { id, pin, remember }),
  signOutProfile: () => call<void>('sign_out_profile'),
  continueAsDevice: (always: boolean) => call<void>('continue_as_device', { always }),
  /** The signed-in profile's stars, replaced first when `set` is given. */
  profileStars: (set?: Star[]) => call<Star[]>('profile_stars', { set: set ?? null }),
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
  /** Which app this is: the desktop window, or the Android app. */
  platform: () => call<'desktop' | 'android' | 'ios'>('platform'),
  /** Files from the phone — picked, shared in, or a whole folder — as one
   *  upload. Android only. */
  uploadFromPhone: (
    files: Array<{ uri: string; rel: string; size: number; mtime: number }>,
    folders: string[],
    into: string,
    label: string,
    id: string,
  ) => call<UploadOutcome>('upload_from_phone', { files, folders, into, label, id }),
  /** A file saved into the phone's Downloads/Basalt. Android only. */
  downloadToPhone: (remote: string, id: string) =>
    call<{ uri: string; shownAs: string }>('download_to_phone', { remote, id }),
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

/**
 * Told when the host turns out to have removed this device while the app was
 * closed. The message names the host and the drive.
 */
export async function onRemoved(handler: (message: string) => void): Promise<() => void> {
  if (!inTauri()) return () => {}
  const { listen } = await import('@tauri-apps/api/event')
  return listen<string>('basalt://removed', (e) => handler(e.payload))
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
  hostId: '5c1e8d2a9b7f4e03',
  hostName: showcase.HOST.name,
  vault: showcase.HOST.vault,
  writable: true,
  address: '127.0.0.1:7742',
  hasPaired: true,
  // A name from the showcase household, as the videos show it.
  deviceName:
    typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('mobile')
      ? "Maya's phone"
      : "Maya's laptop",
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
/** The version the preview claims to be: this build's own. */
const MOCK_VERSION: string = packageInfo.version

/**
 * `?update` in the preview offers one, for the same reason as `?unpaired`.
 *
 * An update offer is by definition something the app shows on a day nobody
 * chose, so without this the panel could only ever be reviewed by publishing
 * a release — which is a poor moment to discover the notes do not fit.
 */
const MOCK_RELEASE: Release = {
  version: '9.9.0',
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
  pageUrl: 'https://example.test/releases/v9.9.0',
  installerName: 'Basalt-Client-9.9.0-setup.exe',
  installerUrl: 'https://example.test/Basalt-Client-9.9.0-setup.exe',
  installerBytes: 35_600_000,
  checksumUrl: 'https://example.test/Basalt-Client-9.9.0-setup.exe.sha256',
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
    hostId: 'a83f0c6d21e94b7a5f1c2d3e4b5a6978',
    hostName: 'STUDY-LAPTOP',
    vault: 'Backups',
    address: '192.168.1.42:7742',
    requiresPin: false,
    hasVault: true,
    paired: true,
  },
  {
    hostId: '5c1e8d2a9b7f4e03c6a1f2e3d4c5b6a7',
    hostName: showcase.HOST.name,
    vault: showcase.HOST.vault,
    address: '192.168.1.90:7742',
    requiresPin: true,
    hasVault: true,
    paired: false,
  },
  {
    hostId: 'e27b94f0c3a15d68e9f0a1b2c3d4e5f6',
    hostName: 'OFFICE-PC',
    vault: 'Vault',
    address: '192.168.1.17:7742',
    requiresPin: true,
    hasVault: false,
    paired: false,
  },
]

/** The showcase library: invented films and series. See `showcase.ts`. */
const MOCK_LIBRARY: LibraryItem[] = showcase.library()

/** Some things part-watched, so Continue watching has something to draw. */
const mockWatched = new Map<string, Watched>(showcase.watching().map((w) => [w.path, w]))

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
      // Artwork only when the preview was pointed at some with `?assets`.
      // Without, it draws posters from the title, as a host with no poster
      // source does.
      return showcase.posterUrl(String(args?.id ?? '')) as T
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
      // The showcase drive, folder by folder. `?many` gives the root a
      // hundred thousand generated entries instead, so the virtualised list
      // and the sort menu can be exercised as against a large real drive.
      if (!previewFlag('many')) return showcase.listFolder((args?.path as string) ?? '') as T
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
      const known = showcase.statFile(path)
      if (known) return known as T
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
      // Something to open, so the player goes ahead; the preview's player
      // shows the showcase's footage for it. See `useMpv`.
      return `showcase:${String(args?.path ?? '')}` as T
    case 'media_base':
      // No thumbnails in the browser preview: tiles show their placeholder,
      // which is what a host without pictures looks like too.
      return '' as T
    case 'identity':
      return { ...mockIdentity } as T
    case 'profiles':
      return mockProfiles.map((p) => ({ ...p })) as T
    case 'create_profile': {
      const profile: ProfileView = {
        id: `p${mockProfiles.length + 1}`,
        name: String(args?.name ?? '').trim(),
        color: Number(args?.color ?? 0),
        hasPin: true,
        lastUsed: Math.floor(Date.now() / 1000),
      }
      mockProfiles.push(profile)
      mockPins.set(profile.id, String(args?.pin ?? ''))
      mockIdentity = { profile, choose: false, ended: false, lastProfile: profile.id }
      return profile as T
    }
    case 'sign_in_profile': {
      const profile = mockProfiles.find((p) => p.id === args?.id)
      const pin = String(args?.pin ?? '')
      if (!profile) throw new ApiError('notfound', 'that profile is not there any more')
      if (profile.hasPin && mockPins.get(profile.id) !== pin) {
        throw new ApiError('denied', 'that PIN is not right')
      }
      if (!profile.hasPin) {
        mockPins.set(profile.id, pin)
        profile.hasPin = true
      }
      mockIdentity = { profile, choose: false, ended: false, lastProfile: profile.id }
      return profile as T
    }
    case 'sign_out_profile':
      mockIdentity = { profile: null, choose: true, ended: false, lastProfile: mockIdentity.lastProfile }
      return undefined as T
    case 'continue_as_device':
      mockIdentity = { profile: null, choose: false, ended: false, lastProfile: mockIdentity.lastProfile }
      return undefined as T
    case 'profile_stars': {
      const set = args?.set as Star[] | null | undefined
      if (set) mockProfileStars = set
      return mockProfileStars as T
    }
    case 'collections':
      return {
        revision: 1,
        scanning: false,
        collections: args?.knownRevision === 1 ? null : showcase.collections(),
      } as T
    case 'make_dir':
    case 'rename_entry':
    case 'remove_entry':
      return undefined as T
    default:
      return undefined as T
  }
}

// Preview profiles, from the showcase. Maya's PIN is 1234; Sam and Leo have
// none yet, so signing in as either chooses one. `?device` opens as the
// device, skipping the choice.
const mockProfiles: ProfileView[] = showcase.profiles()
const mockPins = showcase.pins()
let mockProfileStars: Star[] = []
let mockIdentity: IdentityState = {
  profile: null,
  choose: !(typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('device')),
  ended: false,
  lastProfile: 'p1',
}
