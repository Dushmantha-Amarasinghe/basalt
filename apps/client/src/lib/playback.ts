/**
 * What the player will open.
 *
 * Every media file, because the player is mpv now and mpv decodes all of it.
 * This used to be a much narrower question — which containers Chromium could
 * parse, and which of those it could also get sound out of — and everything
 * that existed to explain the difference to the user has gone with it: the
 * decoded-byte counters that detected silence, the message naming AC3 and
 * DTS, the "partial" playability tier. There is no longer a gap to explain.
 */
export function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
}

export function isMediaFile(name: string): boolean {
  return PLAYER_MEDIA.has(extensionOf(name))
}

/** Extensions worth handing to a media player. */
const PLAYER_MEDIA = new Set([
  // Video containers, including the ones only an external player handles.
  'mp4', 'mkv', 'avi', 'mov', 'm4v', 'webm', 'wmv', 'flv', 'ts', 'm2ts',
  'mpg', 'mpeg', 'vob', 'divx', 'ogv', 'rmvb', 'asf', '3gp',
  // Audio.
  'mp3', 'flac', 'wav', 'm4a', 'aac', 'ogg', 'opus', 'wma', 'aiff', 'alac',
])

