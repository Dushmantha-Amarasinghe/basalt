/**
 * The bridge to the Rust host.
 *
 * Every call goes through `call()`, which falls back to sample data when the
 * app is running in a plain browser rather than inside Tauri. That is not a
 * convenience: it is what lets the whole interface be opened, resized and
 * clicked through without a drive plugged in and a laptop on the network.
 *
 * **These types mirror `basalt-host::ui`.** Tauri converts command *arguments*
 * from camelCase to snake_case automatically but does nothing to what comes
 * back, so every field below is the camelCase name that crate's `#[serde]`
 * attributes produce. Those names are asserted by Rust tests; if one changes
 * there and not here, the field silently becomes `undefined`.
 */

export interface VaultView {
  path: string
  name: string
  free: number
  total: number
  /** False once the drive has been unplugged. */
  available: boolean
}

/** How the media index is getting on. */
export interface LibraryStatus {
  enabled: boolean
  /** True while a scan runs, so the screen can say so rather than look empty. */
  scanning: boolean
  films: number
  series: number
  /** Items the parser was unsure about, worth a person's eye. */
  uncertain: number
  /** Items with a poster downloaded. */
  withArt: number
  /** Whether poster downloads are switched on. */
  posters: boolean
  /** Whether a TMDb key is set. The key itself never leaves the host. */
  hasKey: boolean
  /** Unix seconds of the last completed scan, zero if never. */
  scannedAt: number
}

export interface HostStatus {
  hostId: string
  hostName: string
  port: number
  requirePin: boolean
  startWithWindows: boolean
  vault: VaultView | null
  addresses: string[]
  deviceCount: number
  library: LibraryStatus
  /** Whether each device sees its own watch history rather than a shared one. */
  progressPerDevice: boolean
  serving: boolean
  /** Why sharing stopped, when it has. */
  problem: string | null
}

export interface DriveView {
  path: string
  /** Unambiguous in a list: `Films (E:)`. */
  name: string
  /** The bare volume label, empty when it has none. A better default name for
   *  the share itself — the drive letter is this machine's business. */
  label: string
  kind: 'fixed' | 'removable' | 'network' | 'cdrom' | 'other'
  free: number
  total: number
  /** False for an empty card reader slot or a disconnected network drive. */
  ready: boolean
}

export interface DeviceView {
  id: string
  name: string
  /** Unix seconds, as the host reports them. */
  pairedAt: number
  lastSeen: number
  writable: boolean
  online: boolean
  connections: number
  sent: number
  received: number
  /** Bytes per second, measured over the interval between two polls. */
  sendRate: number
  receiveRate: number
}

export interface PairingView {
  id: string
  deviceName: string
  /** Null when the host is not asking for a PIN. */
  pin: string | null
  secondsLeft: number
}

export type ErrorKind = 'notfound' | 'denied' | 'pairing' | 'exists' | 'error'

export class ApiError extends Error {
  readonly kind: ErrorKind

  constructor(kind: ErrorKind, message: string) {
    super(message)
    this.name = 'ApiError'
    this.kind = kind
  }
}

/** True when running inside the Tauri shell rather than a plain browser. */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) return mock<T>(command, args)

  const { invoke } = await import('@tauri-apps/api/core')
  try {
    return await invoke<T>(command, args)
  } catch (raw) {
    // Errors arrive as `{ kind, message }` so the interface can branch on a
    // tag instead of matching on prose.
    if (raw && typeof raw === 'object' && 'kind' in raw && 'message' in raw) {
      const e = raw as { kind: ErrorKind; message: string }
      throw new ApiError(e.kind, e.message)
    }
    throw new ApiError('error', String(raw))
  }
}

// ---------------------------------------------------------------------------
// Commands
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
  appVersion: (): Promise<string> => call('app_version'),
  /** A newer release, or null when this is the newest. */
  checkUpdate: (): Promise<Release | null> => call('check_update'),
  /** Fetches and verifies an installer, returning where it landed. */
  downloadUpdate: (release: Release): Promise<string> =>
    call('download_update', { release }),
  /** Runs the installer and closes this app so it can be replaced. */
  installUpdate: (path: string): Promise<void> => call('install_update', { path }),

  status: (): Promise<HostStatus> => call('status'),
  listDrives: (): Promise<DriveView[]> => call('list_drives'),
  chooseVault: (path: string, name: string): Promise<HostStatus> =>
    call('choose_vault', { path, name }),
  setHostName: (name: string): Promise<HostStatus> => call('set_host_name', { name }),

  devices: (): Promise<DeviceView[]> => call('devices'),
  revokeDevice: (id: string): Promise<boolean> => call('revoke_device', { id }),
  renameDevice: (id: string, name: string): Promise<boolean> =>
    call('rename_device', { id, name }),
  setDeviceWritable: (id: string, writable: boolean): Promise<boolean> =>
    call('set_device_writable', { id, writable }),

  pendingPairings: (): Promise<PairingView[]> => call('pending_pairings'),
  denyPairing: (id: string): Promise<boolean> => call('deny_pairing', { id }),
  setRequirePin: (require: boolean): Promise<HostStatus> =>
    call('set_require_pin', { require }),

  setStartWithWindows: (enabled: boolean): Promise<HostStatus> =>
    call('set_start_with_windows', { enabled }),
  setLibraryEnabled: (enabled: boolean): Promise<HostStatus> =>
    call('set_library_enabled', { enabled }),
  rescanLibrary: (): Promise<HostStatus> => call('rescan_library'),
  setPosters: (enabled: boolean): Promise<HostStatus> =>
    call('set_posters', { enabled }),
  setTmdbKey: (key: string): Promise<HostStatus> => call('set_tmdb_key', { key }),
  setProgressPerDevice: (enabled: boolean): Promise<HostStatus> =>
    call('set_progress_per_device', { enabled }),
  openVaultFolder: (): Promise<void> => call('open_vault_folder'),
  /** Which build this is — the commit and the day it was made. */
  buildInfo: (): Promise<string> => call('build_info'),
  openLogFolder: (): Promise<void> => call('open_log_folder'),
}

/** Opens the native folder picker, for sharing a folder rather than a drive. */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri()) return null
  const { open } = await import('@tauri-apps/plugin-dialog')
  const chosen = await open({ directory: true, multiple: false })
  return typeof chosen === 'string' ? chosen : null
}

// ---------------------------------------------------------------------------
// Sample data, for reviewing the interface in a browser
// ---------------------------------------------------------------------------

const GB = 1024 ** 3

/** Mutable so the browser preview responds to clicks like the real thing. */
const sample: {
  status: HostStatus
  devices: DeviceView[]
  pending: PairingView[]
  drives: DriveView[]
} = {
  status: {
    hostId: 'a3f9c1e27b48d05f6a1c9e83b4d72f10c5e6a9b8d3f4172c8e5a6b9d0f3c7e21',
    hostName: 'STUDY-LAPTOP',
    port: 7742,
    requirePin: true,
    startWithWindows: false,
    vault: null,
    addresses: ['192.168.1.90'],
    deviceCount: 2,
    library: {
      enabled: false,
      scanning: false,
      films: 0,
      series: 0,
      uncertain: 0,
      withArt: 0,
      posters: false,
      hasKey: false,
      scannedAt: 0,
    },
    progressPerDevice: false,
    serving: true,
    problem: null,
  },
  devices: [
    {
      id: 'aa11',
      name: 'FST',
      pairedAt: Math.floor(Date.now() / 1000) - 86_400 * 9,
      lastSeen: Math.floor(Date.now() / 1000) - 12,
      writable: true,
      online: true,
      connections: 2,
      sent: 41 * GB,
      received: 2.4 * GB,
      sendRate: 21_800_000,
      receiveRate: 14_000,
    },
    {
      id: 'bb22',
      name: 'Living room PC',
      pairedAt: Math.floor(Date.now() / 1000) - 86_400 * 31,
      lastSeen: Math.floor(Date.now() / 1000) - 86_400 * 2,
      writable: false,
      online: false,
      connections: 0,
      sent: 3.1 * GB,
      received: 0,
      sendRate: 0,
      receiveRate: 0,
    },
  ],
  pending: [
    {
      id: 'req-1',
      deviceName: 'Kitchen tablet',
      pin: '169241',
      secondsLeft: 104,
    },
  ],
  drives: [
    { path: 'C:\\', name: 'Windows (C:)', label: 'Windows', kind: 'fixed', free: 74 * GB, total: 476 * GB, ready: true },
    { path: 'D:\\', name: 'Storage (D:)', label: 'Storage', kind: 'fixed', free: 512 * GB, total: 1863 * GB, ready: true },
    { path: 'E:\\', name: 'Films (E:)', label: 'Films', kind: 'removable', free: 1204 * GB, total: 3726 * GB, ready: true },
    { path: 'F:\\', name: 'Removable Disk (F:)', label: '', kind: 'removable', free: 0, total: 0, ready: false },
  ],
}

function mock<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  // The one command that does not resolve at once in the app either: it
  // resolves when the download has finished, having reported progress along
  // the way. Handled here rather than in `answer` so the preview keeps that
  // shape, because a bar that never fills is not a bar anybody can review.
  if (command === 'download_update') {
    return mockDownload(args?.release as Release) as Promise<T>
  }

  const answer = (): unknown => {
    switch (command) {
      case 'status':
        // `?missing` shows the drive as unplugged, which is otherwise only
        // reachable by pulling a real drive out of a real machine.
        if (previewFlag('missing') && sample.status.vault) {
          return {
            ...sample.status,
            vault: { ...sample.status.vault, free: 0, total: 0, available: false },
          }
        }
        return sample.status
      case 'list_drives':
        return sample.drives
      case 'choose_vault': {
        const drive = sample.drives.find((d) => d.path === args?.path)
        sample.status.vault = {
          path: String(args?.path ?? ''),
          name: String(args?.name || drive?.name || 'Vault'),
          free: drive?.free ?? 400 * GB,
          total: drive?.total ?? 1000 * GB,
          available: true,
        }
        return sample.status
      }
      case 'set_host_name':
        sample.status.hostName = String(args?.name ?? '')
        return sample.status
      case 'devices':
        return sample.devices
      case 'revoke_device':
        sample.devices = sample.devices.filter((d) => d.id !== args?.id)
        sample.status.deviceCount = sample.devices.length
        return true
      case 'rename_device': {
        const device = sample.devices.find((d) => d.id === args?.id)
        if (device) device.name = String(args?.name ?? '')
        return Boolean(device)
      }
      case 'set_device_writable': {
        const device = sample.devices.find((d) => d.id === args?.id)
        if (device) device.writable = Boolean(args?.writable)
        return Boolean(device)
      }
      case 'pending_pairings':
        return sample.pending
      case 'deny_pairing':
        sample.pending = sample.pending.filter((p) => p.id !== args?.id)
        return true
      case 'set_require_pin':
        sample.status.requirePin = Boolean(args?.require)
        sample.pending = []
        return sample.status
      case 'set_start_with_windows':
        sample.status.startWithWindows = Boolean(args?.enabled)
        return sample.status
      case 'set_library_enabled':
        sample.status.library = Boolean(args?.enabled)
          ? {
              enabled: true,
              scanning: false,
              films: 42,
              series: 7,
              uncertain: 3,
              withArt: sample.status.library.posters ? 46 : 0,
              posters: sample.status.library.posters,
              hasKey: sample.status.library.hasKey,
              scannedAt: Math.floor(Date.now() / 1000),
            }
          : {
              enabled: false,
              scanning: false,
              films: 0,
              series: 0,
              uncertain: 0,
              withArt: 0,
              posters: sample.status.library.posters,
              hasKey: sample.status.library.hasKey,
              scannedAt: 0,
            }
        return sample.status
      case 'rescan_library':
        sample.status.library.scanning = true
        // Finishes on its own, so the preview shows the scanning state and
        // then the result, as the real thing does.
        setTimeout(() => {
          sample.status.library.scanning = false
          sample.status.library.scannedAt = Math.floor(Date.now() / 1000)
        }, 2500)
        return sample.status
      case 'set_posters':
        sample.status.library.posters = Boolean(args?.enabled)
        sample.status.library.withArt = sample.status.library.posters
          ? sample.status.library.films + sample.status.library.series
          : 0
        return sample.status
      case 'set_progress_per_device':
        sample.status.progressPerDevice = Boolean(args?.enabled)
        return sample.status
      case 'set_tmdb_key':
        sample.status.library.hasKey = String(args?.key ?? '').trim().length > 0
        return sample.status
      case 'build_info':
        return 'preview · not a real build'
      case 'app_version':
        return MOCK_VERSION
      case 'check_update':
        return previewFlag('update') ? MOCK_RELEASE : null
      case 'install_update':
        return undefined
      case 'open_log_folder':
        return undefined
      case 'open_vault_folder':
        return undefined
      default:
        throw new ApiError('error', `no sample data for ${command}`)
    }
  }

  // A tick of latency, so loading states are visible in the browser rather
  // than resolving before React has painted them.
  //
  // Cloned, because the real thing crosses a process boundary and is
  // deserialised fresh every time. Handing back the same object twice let
  // React skip a render that the desktop app would always do, which made the
  // preview behave differently from the app for no reason that was visible.
  return new Promise((resolve) =>
    setTimeout(() => resolve(structuredClone(answer()) as T), 60),
  )
}

/** The version the preview claims to be. */
const MOCK_VERSION = '1.0.0'

/**
 * `?update` in the preview offers one.
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
    '* **Two drives at once**, shared as one.',
    '* **Per-device access**, so a device can be given read-only.',
    '',
    '## Fixed',
    '',
    '* A scan no longer stalls on a folder the drive refuses to list.',
  ].join('\n'),
  pageUrl: 'https://example.test/releases/v1.1.0',
  installerName: 'Basalt-Host-1.1.0-setup.exe',
  installerUrl: 'https://example.test/Basalt-Host-1.1.0-setup.exe',
  installerBytes: 5_200_000,
  checksumUrl: 'https://example.test/Basalt-Host-1.1.0-setup.exe.sha256',
}

function previewFlag(name: string): boolean {
  return (
    typeof window !== 'undefined' &&
    new URLSearchParams(window.location.search).has(name)
  )
}

/** Fills the bar over a couple of seconds, then resolves like the real one. */
async function mockDownload(release: Release): Promise<string> {
  const total = release.installerBytes
  const steps = 12
  for (let tick = 1; tick <= steps; tick += 1) {
    await new Promise((resolve) => setTimeout(resolve, 160))
    window.dispatchEvent(
      new CustomEvent('basalt://update-progress', {
        detail: [Math.min(Math.ceil((total / steps) * tick), total), total],
      }),
    )
  }
  return `C:\Users\preview\Downloads\${release.installerName}`
}
