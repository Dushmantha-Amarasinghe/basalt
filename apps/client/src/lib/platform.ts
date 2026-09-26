/**
 * Which app this is: the desktop window, or the phone and tablet app.
 *
 * Decided once, from the webview itself. Android's says so in its user
 * agent. The browser preview can ask for the mobile app with `?mobile`, so
 * its screens can be worked on at a desk.
 */
export function isMobileShell(): boolean {
  if (typeof navigator === 'undefined') return false
  if (/Android/i.test(navigator.userAgent)) return true
  if (typeof location !== 'undefined' && new URLSearchParams(location.search).has('mobile')) {
    return true
  }
  return false
}

/** Whether this is the Android app itself, not a preview of it. */
export function isAndroid(): boolean {
  return typeof navigator !== 'undefined' && /Android/i.test(navigator.userAgent)
}
