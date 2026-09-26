/**
 * What the app asks of Android itself, through the Basalt plugin.
 *
 * Only in the Android app; everything here does nothing — and returns
 * nothing — anywhere else, including the browser preview, so screens can
 * call it without checking where they are.
 */
import { inTauri } from './api'
import { isAndroid } from './platform'

/** A file on the phone, as Android describes one it has handed over. */
export interface PhoneFile {
  uri: string
  name: string
  /** Bytes; negative when the phone could not say. */
  size: number
  /** Unix seconds, or 0. */
  mtime: number
  mime?: string
  /** Where it sits inside a picked folder: `Sub/name`. */
  rel?: string
}

export interface Insets {
  top: number
  bottom: number
  left: number
  right: number
  keyboard: number
}

async function plugin<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!inTauri() || !isAndroid()) return null
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(`plugin:basalt-android|${command}`, args ?? {})
}

export const android = {
  /** Photos and videos, or any files, from the phone's own picker. */
  pickFiles: async (kind: 'any' | 'media' | 'image' | 'video' | 'audio' | 'subtitle' = 'any') =>
    (await plugin<{ files: PhoneFile[] }>('pick_files', { kind, multiple: kind !== 'subtitle' }))
      ?.files ?? [],

  /** A folder, to upload with everything in it. */
  pickFolder: async () => {
    const picked = await plugin<{ uri?: string; name?: string }>('pick_folder')
    return picked?.uri ? { uri: picked.uri, name: picked.name ?? 'Folder' } : null
  },

  /** Every file and folder inside a picked folder, relative to it. */
  listFolder: async (uri: string) =>
    (await plugin<{ files: PhoneFile[]; folders: string[] }>('list_folder', { uri })) ?? {
      files: [],
      folders: [],
    },

  /** Files shared into Basalt from another app, once each. */
  takeShared: async () => (await plugin<{ files: PhoneFile[] }>('take_shared'))?.files ?? [],

  /** A stream, handed to another player: VLC, MX Player. */
  openWith: (url: string, mime: string, title: string) =>
    plugin<void>('open_with', { url, mime, title }),

  openDownload: (uri: string) => plugin<void>('open_download', { uri }),
  shareDownload: (uri: string) => plugin<void>('share_download', { uri }),

  keepAlive: (reason: 'transfer' | 'playback', title: string, text: string, progress = -1) =>
    plugin<void>('keep_alive', { reason, title, text, progress }),
  letGo: (reason: 'transfer' | 'playback') => plugin<void>('let_go', { reason }),
  requestNotifications: () => plugin<{ granted: boolean }>('request_notifications'),

  setImmersive: (on: boolean) => plugin<void>('set_immersive', { on }),
  setOrientation: (mode: 'landscape' | 'portrait' | 'auto') =>
    plugin<void>('set_orientation', { mode }),
  /** Back at the top of the app: to the home screen, as every app does. */
  minimize: () => plugin<void>('minimize'),
  haptic: (kind: 'tap' | 'long' | 'confirm' = 'tap') => plugin<void>('haptic', { kind }),
  insets: () => plugin<Insets>('insets'),

  canInstallApks: async () => (await plugin<{ can: boolean }>('can_install_apks'))?.can ?? false,
  openInstallSettings: () => plugin<void>('open_install_settings'),
  installApk: (path: string) => plugin<void>('install_apk', { path }),
}

/** Applies the system bars' sizes as CSS variables, now and on every change. */
export async function applyInsets(): Promise<void> {
  const insets = await android.insets().catch(() => null)
  if (!insets) return
  const style = document.documentElement.style
  style.setProperty('--inset-top', `${insets.top}px`)
  style.setProperty('--inset-bottom', `${insets.bottom}px`)
  style.setProperty('--inset-left', `${insets.left}px`)
  style.setProperty('--inset-right', `${insets.right}px`)
  style.setProperty('--keyboard', `${insets.keyboard}px`)
}
