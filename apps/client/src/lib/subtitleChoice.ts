/**
 * Which subtitles to show, and what to call them.
 *
 * mpv only turns subtitles on by itself when the file marks a track as the
 * default, which most releases do not — so they started off every time, and
 * turning them on for one episode did nothing for the next. Now the choice is
 * remembered: on means the next video opens with the closest match to what
 * was chosen (same language, same kind — SDH or plain), and off stays off.
 * Forced subtitles, which carry only the lines in another language, show
 * either way, as they do on every streaming service.
 *
 * The labels were mpv's raw fields: a track with no title showed its language
 * tag, `en-US`, and one titled `SDH` showed just that, with no language at
 * all. Each track is now named by its language, with SDH and Forced as tags.
 */

export interface SubTrack {
  id: number
  /** The file's language tag, as written: `en`, `eng`, `en-US`. */
  lang: string
  title: string
  forced: boolean
  isDefault: boolean
  /** For the deaf and hard of hearing: sounds and speakers as well as words. */
  sdh: boolean
  /** Loaded from a separate file rather than carried in the video. */
  external: boolean
}

export interface SubtitlePref {
  on: boolean
  /** The language last chosen, when on. */
  lang: string | null
  /** Whether the track last chosen was SDH. */
  sdh: boolean
}

/** No choice made yet: subtitles on, in whatever the file has. */
export const FIRST_TIME: SubtitlePref = { on: true, lang: null, sdh: false }

const KEY = 'basalt.subtitles'

export function loadPref(): SubtitlePref {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return FIRST_TIME
    const parsed = JSON.parse(raw) as Partial<SubtitlePref>
    return {
      on: parsed.on !== false,
      lang: typeof parsed.lang === 'string' ? parsed.lang : null,
      sdh: parsed.sdh === true,
    }
  } catch {
    return FIRST_TIME
  }
}

export function savePref(pref: SubtitlePref): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(pref))
  } catch {
    // Storage off or full: the choice still holds for this video.
  }
}

/** Three-letter codes that are not the two-letter ones spelt out. */
const BIBLIOGRAPHIC: Record<string, string> = {
  eng: 'en', spa: 'es', fre: 'fr', fra: 'fr', ger: 'de', deu: 'de', ita: 'it', por: 'pt',
  jpn: 'ja', kor: 'ko', chi: 'zh', zho: 'zh', rus: 'ru', ara: 'ar', hin: 'hi', dut: 'nl',
  nld: 'nl', swe: 'sv', nor: 'no', nob: 'nb', dan: 'da', fin: 'fi', pol: 'pl', tur: 'tr',
  gre: 'el', ell: 'el', heb: 'he', tha: 'th', vie: 'vi', ind: 'id', may: 'ms', msa: 'ms',
  cze: 'cs', ces: 'cs', hun: 'hu', rum: 'ro', ron: 'ro', ukr: 'uk', sin: 'si', tam: 'ta',
  tel: 'te', ben: 'bn', urd: 'ur', per: 'fa', fas: 'fa', bul: 'bg', hrv: 'hr', srp: 'sr',
  slo: 'sk', slk: 'sk', slv: 'sl', est: 'et', lav: 'lv', lit: 'lt', ice: 'is', isl: 'is',
  cat: 'ca', baq: 'eu', eus: 'eu', glg: 'gl', fil: 'fil', tgl: 'tl', mal: 'ml', kan: 'kn',
  mar: 'mr', guj: 'gu', pan: 'pa', nep: 'ne',
}

/** A language tag as a canonical one Intl understands: `eng` → `en`. */
export function normaliseLang(tag: string): string {
  const clean = tag.trim().replace(/_/g, '-')
  if (!clean || /^(und|unk|mis|zxx|none)$/i.test(clean)) return ''
  const [primary, ...rest] = clean.split('-')
  const two = BIBLIOGRAPHIC[primary!.toLowerCase()] ?? primary!.toLowerCase()
  return [two, ...rest].join('-')
}

/** The language, in the reader's own language: `en-US` → `English (United States)`. */
export function languageName(tag: string): string {
  const code = normaliseLang(tag)
  if (!code) return ''
  try {
    const names = new Intl.DisplayNames(undefined, { type: 'language' })
    // A language Intl does not know comes back as its own code, sometimes
    // dressed up — `xx-nonsense` as "xx (NONSENSE)". Known means the first
    // part has a name of its own.
    const primary = code.split('-')[0]!
    const primaryName = names.of(primary)
    if (!primaryName || primaryName.toLowerCase() === primary.toLowerCase()) return ''
    return names.of(code) ?? primaryName
  } catch {
    return ''
  }
}

/** The first part of a language tag, for comparing `en-US` with `eng`. */
function primaryOf(tag: string | null | undefined): string {
  return normaliseLang(tag ?? '').split('-')[0] ?? ''
}

const SDH_WORDS = /\b(sdh|cc|hearing[ -]impaired|hi)\b/i
const FORCED_WORDS = /\bforced\b/i

/** Reads a track the way it should be read, from what mpv reports. */
export function describeTrack(raw: {
  id: number
  lang?: string
  title?: string
  forced?: boolean
  isDefault?: boolean
  hearingImpaired?: boolean
  external?: boolean
}): SubTrack {
  const title = (raw.title ?? '').trim()
  return {
    id: raw.id,
    lang: raw.lang ?? '',
    title,
    forced: raw.forced === true || FORCED_WORDS.test(title),
    isDefault: raw.isDefault === true,
    sdh: raw.hearingImpaired === true || SDH_WORDS.test(title),
    external: raw.external === true,
  }
}

export interface TrackLabel {
  /** The language, or the title when there is no language. */
  name: string
  tags: string[]
  /** Whatever else the title says: `Commentary`, `Signs & Songs`. */
  detail: string
}

export function labelOf(track: SubTrack): TrackLabel {
  const language = languageName(track.lang)
  const tags: string[] = []
  if (track.sdh) tags.push('SDH')
  if (track.forced) tags.push('Forced')

  // The title, minus what is already said: the language, SDH, Forced.
  const rest = track.title
    .replace(SDH_WORDS, '')
    .replace(FORCED_WORDS, '')
    .replace(/[[\](){}]/g, ' ')
    .replace(/\s*[-|·,:]\s*$/g, '')
    .replace(/^\s*[-|·,:]\s*/g, '')
    .replace(/\s+/g, ' ')
    .trim()
  const restIsLanguage =
    !rest ||
    rest.toLowerCase() === track.lang.toLowerCase() ||
    (language !== '' &&
      (rest.toLowerCase() === language.toLowerCase() || languageName(rest) === language))

  const name = language || (restIsLanguage ? '' : rest) || languageName(rest) || `Track ${track.id}`
  const detail = language && !restIsLanguage ? rest : ''
  return { name, tags, detail }
}

/**
 * The track to show when a video opens, or null for none.
 *
 * Off: only a forced track, which carries the lines a viewer could not
 * otherwise follow. On: the closest to the last choice — its language first,
 * then whether it was SDH — and after that whatever the file marks as its
 * default, then simply the first.
 */
export function chooseSubtitle(tracks: SubTrack[], pref: SubtitlePref): number | null {
  if (tracks.length === 0) return null
  const forced = tracks.filter((t) => t.forced)
  if (!pref.on) {
    const matching = forced.find((t) => pref.lang && primaryOf(t.lang) === primaryOf(pref.lang))
    return (matching ?? forced[0])?.id ?? null
  }

  const full = tracks.filter((t) => !t.forced)
  const pool = full.length > 0 ? full : forced
  const wanted = primaryOf(pref.lang)
  let best: SubTrack | null = null
  let bestScore = -1
  for (const track of pool) {
    const score =
      (wanted && primaryOf(track.lang) === wanted ? 8 : 0) +
      (track.sdh === pref.sdh ? 2 : 0) +
      (track.isDefault ? 1 : 0)
    if (score > bestScore) {
      best = track
      bestScore = score
    }
  }
  return best?.id ?? null
}

/**
 * A subtitle file from beside the video, for when the video carries none.
 *
 * Files are known only by the label the host read from their names —
 * `English`, `Spanish forced` — so the language is matched by name.
 */
export function chooseDriveFile<T extends { label: string }>(
  files: T[],
  pref: SubtitlePref,
): T | null {
  if (!pref.on || files.length === 0) return null
  const plain = files.filter((f) => !FORCED_WORDS.test(f.label))
  const pool = plain.length > 0 ? plain : files
  const wanted = pref.lang ? languageName(pref.lang).split(' (')[0]!.toLowerCase() : ''
  const inLanguage = wanted ? pool.filter((f) => f.label.toLowerCase().includes(wanted)) : []
  const candidates = inLanguage.length > 0 ? inLanguage : pool
  return candidates.find((f) => SDH_WORDS.test(f.label) === pref.sdh) ?? candidates[0] ?? null
}

/** What choosing a track says about the next video. */
export function prefFor(track: SubTrack | null): SubtitlePref {
  if (!track) return { on: false, lang: null, sdh: false }
  return { on: true, lang: track.lang || null, sdh: track.sdh }
}
