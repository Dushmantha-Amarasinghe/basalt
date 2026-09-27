/**
 * What to say when some of an upload did not arrive.
 *
 * One picked file was reported as "1 of 1 did not upload", which is how a
 * folder of several is reported, and the reason was the operating system's
 * own words: "io: Software caused connection abort (os error 103)". Now one
 * file says it could not be uploaded, and a dropped connection says so in
 * plain words.
 */
export function uploadFailure(failed: Array<[string, string]>, uploaded: number): string | null {
  if (failed.length === 0) return null
  const [path, why] = failed[0]!
  const name = path.split('/').pop() || path
  const reason = plainReason(why)
  if (failed.length === 1 && uploaded === 0) return `Couldn't upload ${name}. ${reason}`
  const total = uploaded + failed.length
  const count = failed.length === 1 ? '1 file' : `${failed.length} of ${total} files`
  return `${count} did not upload. ${name}: ${reason}`
}

/** Connection failures, by their messages on Windows, Android and Linux. */
const DROPPED =
  /connection (abort|reset|refused|closed)|broken pipe|os error (103|104|32|10053|10054|10061)|unexpected eof|timed out/i

export function plainReason(why: string): string {
  if (DROPPED.test(why)) return 'The connection to the host dropped. Try again.'
  const text = why.replace(/^io:\s*/i, '').replace(/\s*\(os error \d+\)$/i, '').trim()
  if (!text) return 'Something went wrong.'
  const sentence = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?]$/.test(sentence) ? sentence : `${sentence}.`
}
