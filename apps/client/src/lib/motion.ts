/**
 * How things move, in one place.
 *
 * Menus, dialogs and panels each picked their own timing, and some had no
 * way out at all — a context menu vanished in one frame while a dialog beside
 * it faded for a sixth of a second. These are the timings everything shares
 * now: quick enough to never be waited on, long enough to be seen.
 */

/** Fast out, gentle landing. The curve the whole app uses. */
export const EASE_OUT = [0.22, 1, 0.36, 1] as const

/** Something small appearing under the pointer: a menu, a dropdown. */
export const POPOVER = {
  initial: { opacity: 0, scale: 0.97 },
  animate: { opacity: 1, scale: 1 },
  exit: { opacity: 0, scale: 0.98 },
  transition: { duration: 0.13, ease: EASE_OUT },
} as const
