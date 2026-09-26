/**
 * Rows of photos that fill the width exactly, each photo at its own shape.
 *
 * How Google Photos and Windows Photos lay out a library, and the reason a
 * photo grid of squares looks wrong: a square crops every landscape shot to
 * its middle and every portrait one to a band across the face. Here each row
 * takes photos until they would overflow at the target height, then is scaled
 * to fit the width exactly — so rows differ a little in height and nothing
 * is cropped.
 *
 * Pure, and in its own file, because layout arithmetic is exactly the kind of
 * thing that is wrong by a pixel in a way nobody notices until a gap appears.
 */

export interface JustifiedRow {
  /** Index of the first photo in the row. */
  start: number
  /** How many photos the row holds. */
  count: number
  /** The row's height in pixels; every photo in it is this tall. */
  height: number
}

/** Shapes beyond these are treated as these: a panorama would otherwise be
 *  a sliver, and a tall screenshot a column one pixel wide. */
const MIN_ASPECT = 0.5
const MAX_ASPECT = 3

export function clampAspect(aspect: number): number {
  if (!Number.isFinite(aspect) || aspect <= 0) return 4 / 3
  return Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, aspect))
}

/**
 * Lays out photos of the given shapes (width over height) into rows.
 *
 * Each row ends where its height comes closest to `target`: either with the
 * photo that tipped it over the width, which shrinks the row a little, or
 * without it, which grows the row a little. The last row is left at the
 * target rather than stretched: three photos pulled across a whole width
 * would each be enormous.
 */
export function justify(
  aspects: number[],
  width: number,
  target: number,
  gap: number,
): JustifiedRow[] {
  const rows: JustifiedRow[] = []
  if (width <= 0 || target <= 0) return rows

  const heightOf = (sum: number, count: number): number => (width - gap * (count - 1)) / sum

  let start = 0
  let sum = 0
  let i = 0
  while (i < aspects.length) {
    const aspect = clampAspect(aspects[i]!)
    const count = i - start + 1
    if ((sum + aspect) * target + gap * (count - 1) < width) {
      sum += aspect
      i += 1
      continue
    }
    // This photo tips the row over the width. Keep it or leave it for the
    // next row, whichever lands nearer the target height.
    const withIt = heightOf(sum + aspect, count)
    const without = count > 1 ? heightOf(sum, count - 1) : Infinity
    if (Math.abs(withIt - target) <= Math.abs(without - target)) {
      rows.push({ start, count, height: withIt })
      i += 1
    } else {
      rows.push({ start, count: count - 1, height: without })
    }
    start = i
    sum = 0
  }
  if (start < aspects.length) {
    rows.push({ start, count: aspects.length - start, height: target })
  }
  return rows
}
