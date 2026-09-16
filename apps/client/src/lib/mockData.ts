import type { Entry } from '@/components/FileList'

/**
 * Stand-in data until the host is wired up.
 *
 * It exists for one reason: the file list has to be judged at the scale it will
 * actually meet. A pretty list of twelve rows proves nothing — the 60 fps
 * target only becomes real at 100,000. So this generates a realistic mix of
 * names, sizes and dates rather than `file1, file2, file3`.
 *
 * Deterministic, so the view does not reshuffle on every hot reload.
 */

const FOLDERS = [
  'Documents',
  'Photos',
  'Music',
  'Films',
  'Projects',
  'Backups',
  'Archive 2024',
  'Scans',
  'Invoices',
  'Screenshots',
]

const WORDS = [
  'report', 'summary', 'draft', 'final', 'notes', 'budget', 'holiday',
  'family', 'contract', 'receipt', 'recording', 'meeting', 'design',
  'render', 'export', 'backup', 'invoice', 'photo', 'scan', 'letter',
]

const EXTENSIONS: { ext: string; min: number; max: number }[] = [
  { ext: 'mkv', min: 800_000_000, max: 4_000_000_000 },
  { ext: 'mp4', min: 200_000_000, max: 2_000_000_000 },
  { ext: 'jpg', min: 800_000, max: 8_000_000 },
  { ext: 'png', min: 200_000, max: 4_000_000 },
  { ext: 'pdf', min: 100_000, max: 20_000_000 },
  { ext: 'docx', min: 20_000, max: 2_000_000 },
  { ext: 'xlsx', min: 15_000, max: 1_000_000 },
  { ext: 'mp3', min: 3_000_000, max: 15_000_000 },
  { ext: 'flac', min: 20_000_000, max: 60_000_000 },
  { ext: 'zip', min: 1_000_000, max: 500_000_000 },
  { ext: 'txt', min: 500, max: 200_000 },
  { ext: 'rs', min: 1_000, max: 80_000 },
  { ext: 'json', min: 500, max: 5_000_000 },
]

/** Deterministic PRNG, so the list is stable across reloads. */
function makeRng(seed: number): () => number {
  let state = seed >>> 0 || 1
  return () => {
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    state >>>= 0
    return state / 0xffffffff
  }
}

export function generateEntries(count: number, seed = 42): Entry[] {
  const rng = makeRng(seed)
  const pick = <T,>(arr: readonly T[]): T => arr[Math.floor(rng() * arr.length)]!
  const entries: Entry[] = []

  const now = Date.now()
  const year = 365 * 24 * 60 * 60 * 1000

  // Folders first, the way a file manager sorts by default.
  const folderCount = Math.min(FOLDERS.length, Math.max(1, Math.floor(count * 0.02)))
  for (let i = 0; i < folderCount; i += 1) {
    entries.push({
      id: `dir-${i}`,
      name: FOLDERS[i]!,
      kind: 'dir',
      size: 0,
      modified: now - rng() * year * 2,
    })
  }

  for (let i = 0; i < count - folderCount; i += 1) {
    const spec = pick(EXTENSIONS)
    const parts = [pick(WORDS)]
    if (rng() > 0.45) parts.push(pick(WORDS))
    if (rng() > 0.7) parts.push(String(2020 + Math.floor(rng() * 6)))

    entries.push({
      id: `file-${i}`,
      name: `${parts.join('-')}-${i}.${spec.ext}`,
      kind: 'file',
      size: Math.floor(spec.min + rng() * (spec.max - spec.min)),
      modified: now - rng() * year * 3,
    })
  }

  return entries
}
