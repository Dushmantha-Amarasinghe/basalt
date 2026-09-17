/**
 * Working out why a video is playing silently.
 *
 * Chromium plays a file whose *container* and *video* codec it understands even
 * when it cannot decode the audio track — the picture runs and there is simply
 * no sound, with no error event and nothing in the console. That is the single
 * most confusing failure in a media library, because AC3, E-AC3, DTS and TrueHD
 * are what most MKV and many MP4 rips actually carry, and none of them are in
 * Chromium.
 *
 * There is no supported API for "does this file have an audio track and can you
 * decode it" — `audioTracks` is disabled in Chromium. What there is:
 * `webkitAudioDecodedByteCount`, a counter of audio bytes actually decoded. If
 * the video counter is climbing and the audio counter has not moved after a
 * second of playback, nothing is being decoded to sound.
 *
 * The message deliberately covers both possibilities, because this cannot tell
 * a file with an undecodable audio track from one with no audio track at all,
 * and claiming the wrong one would be worse than naming both.
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

/** What to tell the user when nothing is coming out. */
export const SILENT_MESSAGE =
  'No sound. Either this file has no audio track, or its audio format — ' +
  'often AC3, DTS or TrueHD — is one this window cannot decode. Download it ' +
  'to play elsewhere.'

/** Audio formats the window can decode, for the settings and help text. */
export const DECODABLE_AUDIO = ['AAC', 'MP3', 'Opus', 'Vorbis', 'FLAC']
export const DECODABLE_VIDEO = ['H.264', 'VP8', 'VP9', 'AV1']
