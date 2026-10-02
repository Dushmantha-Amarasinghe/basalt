import type { SubtitleTrack } from './api'

/**
 * The subtitle files on the drive for one video, from the library and from
 * the host, as one list.
 *
 * The library only knows about the films and episodes it filed, and only as
 * of its last scan. The host is asked as well whenever a video opens, so a
 * video played from Files gets its subtitles, and so does one whose subtitle
 * arrived since. The two are merged with the library's first, without
 * repeating a file both name.
 */
export function mergeDriveSubtitles(
  library: SubtitleTrack[],
  fromHost: SubtitleTrack[],
  chosenBefore: string[],
): SubtitleTrack[] {
  const merged: SubtitleTrack[] = []
  const seen = new Set<string>()
  const add = (track: SubtitleTrack): void => {
    if (seen.has(track.path)) return
    seen.add(track.path)
    merged.push(track)
  }
  library.forEach(add)
  fromHost.forEach(add)
  // Picked by hand for this video before: offered again, named by its file.
  for (const path of chosenBefore) add({ path, label: path.split('/').pop() ?? path })
  return merged
}

/** Others offered for choosing by hand: whatever is not already offered. */
export function otherSubtitles(others: SubtitleTrack[], offered: SubtitleTrack[]): SubtitleTrack[] {
  const taken = new Set(offered.map((t) => t.path))
  return others.filter((o) => !taken.has(o.path))
}

const KEY = 'basalt:chosen-subtitles'
/** Videos remembered, so the store cannot grow without end. */
const KEEP = 300

type Chosen = Record<string, string[]>

function read(): Chosen {
  try {
    return JSON.parse(window.localStorage.getItem(KEY) ?? '{}') as Chosen
  } catch {
    return {}
  }
}

/** The subtitle files chosen by hand for a video before. */
export function chosenFor(video: string): string[] {
  return read()[video] ?? []
}

/** Remembers a subtitle file chosen by hand for a video. */
export function rememberChosen(video: string, subtitle: string): void {
  const all = read()
  const list = (all[video] ?? []).filter((p) => p !== subtitle)
  delete all[video]
  all[video] = [subtitle, ...list].slice(0, 5)
  const names = Object.keys(all)
  for (const old of names.slice(0, Math.max(0, names.length - KEEP))) delete all[old]
  try {
    window.localStorage.setItem(KEY, JSON.stringify(all))
  } catch {
    // Not remembered, but used this time.
  }
}
