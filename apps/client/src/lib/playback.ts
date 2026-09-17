/**
 * What the window can and cannot play, and how to say so.
 *
 * The failure that prompted this: a 2160p x265 MKV with AAC audio played its
 * picture and made no sound at all, with no error and nothing in the console.
 *
 * The reason is specific and worth writing down, because it is not the obvious
 * one. Chromium's Matroska demuxer exists only to serve WebM, which is a subset
 * of Matroska, and it accepts **only** the codecs on WebM's list — VP8, VP9,
 * AV1, Opus and Vorbis. AAC is not on it. So AAC audio inside an `.mkv` is
 * refused by the demuxer even though the very same AAC track inside an `.mp4`
 * plays perfectly. Meanwhile the HEVC video played because Windows had the
 * platform HEVC extension installed. Picture yes, sound no.
 *
 * None of this is fixable in the webview. It needs a real demuxer — mpv — or a
 * host-side remux into MP4. Until then the app's job is to say precisely what
 * happened rather than leave someone checking their volume.
 */

/** Chromium's decoded-byte counters, which are not in the standard lib types. */
interface DecodeCounters {
  webkitAudioDecodedByteCount?: number
  webkitVideoDecodedByteCount?: number
}

export type SoundState = 'unknown' | 'playing' | 'silent'

/** How much video must decode before a still-zero audio counter means anything. */
const VIDEO_BYTES_BEFORE_JUDGING = 64 * 1024

export function readCounters(media: HTMLVideoElement): {
  audio: number
  video: number
} {
  const counters = media as unknown as DecodeCounters
  return {
    audio: counters.webkitAudioDecodedByteCount ?? 0,
    video: counters.webkitVideoDecodedByteCount ?? 0,
  }
}

/**
 * Decides whether sound is reaching the speakers.
 *
 * `unknown` until there is enough evidence — reporting "silent" early, before
 * the first audio packet has been decoded, would flash a warning on every file
 * that plays perfectly well.
 */
export function judgeSound(
  audioBytes: number,
  videoBytes: number,
  playedSeconds: number,
): SoundState {
  if (audioBytes > 0) return 'playing'
  // On a machine with no `webkit*` counters at all, both stay zero forever and
  // this correctly never claims anything.
  if (videoBytes < VIDEO_BYTES_BEFORE_JUDGING) return 'unknown'
  if (playedSeconds < 1) return 'unknown'
  return 'silent'
}

export function extensionOf(path: string): string {
  const dot = path.lastIndexOf('.')
  return dot > 0 ? path.slice(dot + 1).toLowerCase() : ''
}

/** Containers the window demuxes fully. */
const NATIVE_CONTAINERS = new Set([
  'mp4',
  'm4v',
  'm4a',
  'webm',
  'mp3',
  'wav',
  'ogg',
  'opus',
  'flac',
])

/**
 * Containers Chromium parses but only for WebM's codec list.
 *
 * This is the trap: the file opens, the picture may even run, and the audio is
 * silently dropped because AAC is not a WebM codec.
 */
const PARTIAL_CONTAINERS = new Set(['mkv'])

export type Playability = 'full' | 'partial' | 'none'

export function playabilityOf(name: string): Playability {
  const ext = extensionOf(name)
  if (NATIVE_CONTAINERS.has(ext)) return 'full'
  if (PARTIAL_CONTAINERS.has(ext)) return 'partial'
  return 'none'
}

/** Whether it is worth even opening the player for this file. */
export function isPlayable(name: string): boolean {
  return playabilityOf(name) !== 'none'
}

/** Why there is no sound, in terms of this particular file. */
export function silenceMessage(name: string): string {
  const ext = extensionOf(name).toUpperCase()
  if (playabilityOf(name) === 'partial') {
    return (
      `No sound. This window reads the ${ext} container only well enough for ` +
      'WebM, so it plays Opus and Vorbis and drops everything else — AAC, ' +
      'AC3, Dolby Digital Plus (DD+ / E-AC3) and DTS. A 5.1 film almost ' +
      'always carries one of those. Play it in your usual player — it streams, \n' +
      'nothing is downloaded.'
    )
  }
  return (
    'No sound. Either this file has no audio track, or its audio format — ' +
    'often AC3, Dolby Digital Plus or DTS — is one this window cannot ' +
    'decode. Play it in your usual player — it streams, nothing is downloaded.'
  )
}

/** Why the picture is missing too. */
export function unplayableMessage(name: string): string {
  const ext = extensionOf(name).toUpperCase() || 'this file'
  return (
    `This window cannot play ${ext}. Play it in your usual player — it streams, ` +
    'nothing is downloaded.'
  )
}

/** Audio and video formats the window handles, for help text. */
export const DECODABLE_AUDIO = ['AAC', 'MP3', 'Opus', 'Vorbis', 'FLAC']
export const DECODABLE_VIDEO = ['H.264', 'VP8', 'VP9', 'AV1']
