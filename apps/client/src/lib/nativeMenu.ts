/**
 * Stops the webview offering its own right-click menu.
 *
 * WebView2 shows a browser menu — Back, Refresh, Save as, Print, More tools —
 * on every right click the app does not handle itself. In a browser that is
 * correct. In a desktop app it advertises what the window really is, offers
 * Refresh as a way to throw away the app's state, and sits over the top of the
 * app's own menu, which is the one with the actual commands in it.
 *
 * Done here in one place rather than by adding `preventDefault` to every
 * element: the components that *do* have a menu already prevent it, and the
 * bug is precisely everywhere they do not. A listener on the document runs
 * after React's, which are delegated at the root, so this cannot stop the
 * app's own menus from opening first.
 *
 * **Text fields keep theirs.** Right-clicking an input in any application
 * offers cut, copy and paste, and removing that would cost something real to
 * fix something cosmetic. Those menus carry no browser commands.
 */
export function suppressNativeContextMenu(): () => void {
  const handler = (event: MouseEvent): void => {
    const target = event.target as HTMLElement | null
    if (target?.closest('input, textarea, [contenteditable="true"]')) return
    event.preventDefault()
  }

  document.addEventListener('contextmenu', handler)
  return () => document.removeEventListener('contextmenu', handler)
}

/**
 * Whether a right click at this element would still get the platform menu.
 *
 * Exported for the test, which has no DOM to dispatch events into.
 */
export function keepsNativeMenu(tag: string, editable = false): boolean {
  return ['input', 'textarea'].includes(tag.toLowerCase()) || editable
}
