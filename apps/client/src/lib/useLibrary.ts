import { useCallback, useEffect, useRef, useState } from 'react'
import type { Entry } from '@/components/FileList'
import { api, joinPath, toEntries } from './api'

/**
 * A shallow scan of the vault, used by Recent and the media sections.
 *
 * Two levels deep, not the whole drive. A full recursive walk is what the
 * mirrored SQLite index was for, and that was cut because a directory of
 * 20,000 entries lists in 11 ms — but "cheap per directory" is not "cheap for
 * every directory on a 4 TB drive", and nothing here should spend a minute
 * before showing anything.
 *
 * Two levels covers how people actually arrange a drive: `Films/…`,
 * `Music/Artist/…`, `Photos/2024/…`. Anything deeper is reachable by browsing,
 * which is what Files is for.
 */

const MAX_DIRS = 60
const VIDEO = new Set(['mp4', 'mkv', 'avi', 'mov', 'm4v', 'webm', 'wmv', 'flv', 'ts'])
const AUDIO = new Set(['mp3', 'flac', 'wav', 'm4a', 'aac', 'ogg', 'opus', 'wma'])
const IMAGE = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'bmp', 'heic', 'tiff'])

export type LibraryKind = 'videos' | 'music' | 'photos'

const SETS: Record<LibraryKind, Set<string>> = {
  videos: VIDEO,
  music: AUDIO,
  photos: IMAGE,
}

export function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
}

export function isKind(name: string, kind: LibraryKind): boolean {
  return SETS[kind].has(extensionOf(name))
}

export interface Scan {
  /** Every file found, with `id` holding its full vault path. */
  files: Entry[]
  scanning: boolean
  done: boolean
  rescan: () => void
}

/**
 * Scans once per connection and caches the result.
 *
 * `enabled` is false until something actually needs it, so opening the app on
 * the Files view never pays for a scan the user may not want.
 */
export function useLibraryScan(enabled: boolean, connected: boolean): Scan {
  const [files, setFiles] = useState<Entry[]>([])
  const [scanning, setScanning] = useState(false)
  const [done, setDone] = useState(false)
  const run = useRef(0)

  const scan = useCallback(async () => {
    const generation = ++run.current
    setScanning(true)
    try {
      const root = await api.list('')
      if (run.current !== generation) return

      const found: Entry[] = toEntries('', root).filter((e) => e.kind === 'file')
      const dirs = root.filter((e) => e.kind === 'dir').slice(0, MAX_DIRS)

      // Sequential rather than all at once. The pool holds four connections,
      // and firing sixty requests at it would open and close connections
      // faster than it would return results — the link is the bottleneck, not
      // the request count.
      for (const dir of dirs) {
        if (run.current !== generation) return
        try {
          const listing = await api.list(dir.name)
          found.push(
            ...toEntries(dir.name, listing).filter((e) => e.kind === 'file'),
          )
        } catch {
          // A folder that will not open is not a reason to abandon the scan.
        }
      }

      if (run.current !== generation) return
      setFiles(found)
      setDone(true)
    } finally {
      if (run.current === generation) setScanning(false)
    }
  }, [])

  useEffect(() => {
    if (!enabled || !connected || done || scanning) return
    void scan()
  }, [enabled, connected, done, scanning, scan])

  // A new connection means a possibly different drive.
  useEffect(() => {
    if (!connected) {
      setDone(false)
      setFiles([])
    }
  }, [connected])

  const rescan = useCallback(() => {
    setDone(false)
    setFiles([])
    void scan()
  }, [scan])

  return { files, scanning, done, rescan }
}

/** The files of one media kind, newest first. */
export function filterKind(files: Entry[], kind: LibraryKind): Entry[] {
  return files
    .filter((f) => isKind(f.name, kind))
    .sort((a, b) => b.modified - a.modified)
}

/** The most recently changed files, whatever they are. */
export function recentOf(files: Entry[], limit = 300): Entry[] {
  return [...files].sort((a, b) => b.modified - a.modified).slice(0, limit)
}

export { joinPath }
