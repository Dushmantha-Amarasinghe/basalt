import { useEffect, useState } from 'react'
import { api } from './api'
import * as showcase from './showcase'

/** The long side of a grid thumbnail. Mirrors the host. */
export const GRID_THUMB = 320
/** The long side of a picture made for viewing. Mirrors the host. */
export const VIEW_THUMB = 1600

/**
 * Which host the pictures are from, in every thumbnail URL.
 *
 * The media proxy lives as long as the app, so its address is the same for
 * every host. Without this, the browser's cache would hand one drive's
 * picture to another drive's file of the same name and date.
 */
let hostTag = ''

/**
 * The start of every media URL, fetched once per host.
 *
 * Empty until known, and empty in the browser preview — where every tile
 * shows its placeholder, which is also what a host without pictures shows.
 * `host` is the id of the host connected to, or null.
 */
export function useMediaBase(host: string | null): string {
  const [base, setBase] = useState('')
  useEffect(() => {
    hostTag = host ? host.slice(0, 12) : ''
    setBase('')
    if (!host) return undefined
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
  }, [host])
  return base
}

/** The file itself, as a URL an `<img>` or the player can open. */
export function fileUrl(base: string, path: string): string {
  if (!base && showcase.showcaseAssets()) return showcase.photoFor(path)
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
  // The browser preview, given showcase pictures: see `showcase.ts`.
  if (!base && showcase.showcaseAssets()) {
    return size > GRID_THUMB ? showcase.photoFor(path) : showcase.thumbFor(path)
  }
  const tag = hostTag ? `&h=${hostTag}` : ''
  return base ? `${base}${encodeURIComponent(path)}?thumb=${size}&v=${mtime}${tag}` : ''
}

/** Photos the webview can show as they are. Everything else goes through
 *  the host, which turns it into a JPEG. */
const DISPLAYABLE = new Set(['jpg', 'jpeg', 'jfif', 'png', 'gif', 'webp', 'avif', 'bmp'])

export function isDisplayable(path: string): boolean {
  const dot = path.lastIndexOf('.')
  return dot > 0 && DISPLAYABLE.has(path.slice(dot + 1).toLowerCase())
}
