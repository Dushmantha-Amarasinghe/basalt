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

export interface HostSummary {
  hostId: string
  hostName: string
  vault: string
  pairingOpen: boolean
}

export interface TransferEvent {
  id: string
  kind: 'download' | 'upload'
  name: string
  path: string
  transferred: number
  total: number
  status: 'active' | 'done' | 'failed'
  /** Bytes per second, averaged over the transfer so far. */
  rate: number
}

export type ErrorKind =
  | 'offline'
  | 'notfound'
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

export const api = {
  status: () => call<Status>('status'),
  probe: (address: string) => call<HostSummary>('probe', { address }),
  pair: (address: string, pin: string) => call<Status>('pair', { address, pin }),
  connectSaved: () => call<Status>('connect_saved'),
  connectTo: (hostId: string, address?: string) =>
    call<Status>('connect_to', { hostId, address: address ?? null }),
  disconnect: () => call<Status>('disconnect'),
  forgetHost: (hostId: string) => call<Status>('forget_host', { hostId }),

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
  upload: (local: string, remote: string, overwrite: boolean, id: string) =>
    call<number>('upload', { local, remote, overwrite, id }),
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

let mockEntries: Entry[] | null = null

async function mock<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case 'status':
    case 'connect_saved':
    case 'pair':
    case 'connect_to':
      return MOCK_STATUS as T
    case 'disconnect':
    case 'forget_host':
      return { ...MOCK_STATUS, connected: false } as T
    case 'probe':
      return {
        hostId: 'demo0000',
        hostName: 'Demo Host',
        vault: 'Vault',
        pairingOpen: true,
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
    case 'make_dir':
    case 'rename_entry':
    case 'remove_entry':
      return undefined as T
    default:
      return undefined as T
  }
}
