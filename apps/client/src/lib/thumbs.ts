import { useEffect, useState } from 'react'
import { api } from './api'

/** The long side of a grid thumbnail. Mirrors the host. */
export const GRID_THUMB = 320
/** The long side of a picture made for viewing. Mirrors the host. */
export const VIEW_THUMB = 1600

/**
 * The start of every media URL, fetched once per connection.
 *
 * Empty until known, and empty in the browser preview — where every tile
 * shows its placeholder, which is also what a host without pictures shows.
 */
export function useMediaBase(connected: boolean): string {
  const [base, setBase] = useState('')
  useEffect(() => {
    if (!connected) {
      setBase('')
      return undefined
    }
    let cancelled = false
    void api
      .mediaBase()
      .then((b) => {
        if (!cancelled) setBase(b ?? '')
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [connected])
  return base
}

/** The file itself, as a URL an `<img>` or the player can open. */
export function fileUrl(base: string, path: string): string {
  return base ? `${base}${encodeURIComponent(path)}` : ''
}

/**
 * A picture of a video or photo, made by the host.
 *
 * The modification time rides along so a changed file is a new URL: the
 * page caches these for a week, and without it an edited photo would show
 * its old picture for that long.
 */
export function thumbUrl(base: string, path: string, mtime: number, size = GRID_THUMB): string {
  return base ? `${base}${encodeURIComponent(path)}?thumb=${size}&v=${mtime}` : ''
}

/** Photos the webview can show as they are. Everything else goes through
 *  the host, which turns it into a JPEG. */
const DISPLAYABLE = new Set(['jpg', 'jpeg', 'jfif', 'png', 'gif', 'webp', 'avif', 'bmp'])

export function isDisplayable(path: string): boolean {
  const dot = path.lastIndexOf('.')
  return dot > 0 && DISPLAYABLE.has(path.slice(dot + 1).toLowerCase())
}
