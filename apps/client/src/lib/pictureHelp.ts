/**
 * What the player says when this device cannot play a file as it is.
 *
 * Two ways out, in order: Basalt Host converts it as it is watched, or, when
 * it cannot, the device plays it lighter. Each is said once, in plain words,
 * with the reason: so a softer picture, or a moment's pause at the start,
 * does not read as Basalt being slow for no reason.
 */

/** How a conversion turned out, as the media proxy reports it. */
export interface ConversionStatus {
  by: string | null
  error: string | null
  kind: string | null
  /** When, in milliseconds since 1970. */
  at: number
}

/**
 * Hosts that cannot convert, by id, for the rest of this run: asking again
 * for every episode cost each one several seconds before it played. A host
 * that was only busy is asked again next time.
 */
const cannot = new Map<string, NotConverted>()

export function rememberCannotConvert(host: string, why: NotConverted): void {
  // Switched off is not remembered: it can be switched back on at any time,
  // and asking costs a moment.
  if (why === 'unable' || why === 'outdated' || why === 'slow') cannot.set(host, why)
}

export function cannotConvert(host: string): NotConverted | null {
  return cannot.get(host) ?? null
}

/** Why the host did not convert, when it did not. */
export type NotConverted = 'unable' | 'off' | 'slow' | 'busy' | 'outdated' | 'failed'

export function whyNotConverted(status: ConversionStatus | null): NotConverted {
  if (!status) return 'failed'
  if (status.kind === 'unsupported') return 'outdated'
  if (status.kind === 'unavailable') {
    const said = status.error ?? ''
    if (/already converting/i.test(said)) return 'busy'
    if (/switched off/i.test(said)) return 'off'
    if (/too slow/i.test(said)) return 'slow'
    return 'unable'
  }
  return 'failed'
}

/** What is helping the picture along, and what it is helping with. */
export type PictureHelp =
  | { mode: 'converted'; size: Size; by: string | null }
  | { mode: 'lighter'; size: Size; why: NotConverted | null }

export interface Size {
  width: number
  height: number
}

/** A picture size in the words people use for it. */
export function sizeName(size: Size): string {
  return size.width >= 3200 ? '4K' : `${size.height}p`
}

/** The note's title and text. */
export function pictureNote(help: PictureHelp, device: 'phone' | 'computer'): [string, string] {
  const size = sizeName(help.size)
  if (help.mode === 'converted') {
    const on = help.by ? `, on its ${help.by}` : ''
    return [
      'Converted by Basalt Host',
      `This ${device} can’t play ${size} smoothly, so Basalt Host converts it to 1080p as you watch${on}.`,
    ]
  }
  const because: Record<NotConverted, string> = {
    unable: ', and Basalt Host can’t convert video on its computer',
    off: ', and video conversion is switched off in Basalt Host',
    slow: ', and Basalt Host’s computer is too slow to convert it as you watch',
    busy: ', and Basalt Host is already converting for other devices',
    outdated: ', and Basalt Host needs updating to convert video',
    failed: ', and Basalt Host couldn’t convert this file',
  }
  return [
    'Playing in a lighter mode',
    `This ${device} can’t decode ${size} video in hardware${help.why ? because[help.why] : ''}, so Basalt plays it lighter to keep the picture in step with the sound.`,
  ]
}
