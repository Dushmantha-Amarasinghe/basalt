import { useEffect, useRef } from 'react'
import { android } from '@/lib/android'
import { isAndroid, isMobileShell } from '@/lib/platform'

/**
 * Android's back gesture, handled the way a phone user expects.
 *
 * Whatever is on top goes first: the player, then a photo, then a sheet,
 * then selection, then up a folder, then back to Files. Only at the very top
 * does back leave the app — and then it goes to the home screen, as every app
 * does, rather than closing and losing its place.
 *
 * Each screen registers what back means for it while it is showing; the most
 * recently registered handler that says it handled the gesture wins.
 */
type Handler = () => boolean

const stack: Array<{ id: number; handler: React.RefObject<Handler> }> = []
let nextId = 0
let listening = false

async function listen(): Promise<void> {
  if (listening) return
  if (!isAndroid()) {
    // The browser preview of the phone app has no back button: Escape is
    // back there, so the same screens can be walked through at a desk.
    if (!isMobileShell()) return
    listening = true
    window.addEventListener('keydown', (e) => {
      if (e.key !== 'Escape') return
      for (let i = stack.length - 1; i >= 0; i--) {
        if (stack[i]!.handler.current()) return
      }
    })
    return
  }
  listening = true
  const { onBackButtonPress } = await import('@tauri-apps/api/app')
  await onBackButtonPress(() => {
    for (let i = stack.length - 1; i >= 0; i--) {
      if (stack[i]!.handler.current()) return
    }
    void android.minimize()
  })
}

/** While `active`, back runs `handler` first; it returns whether it handled it. */
export function useBack(active: boolean, handler: Handler): void {
  const ref = useRef<Handler>(handler)
  ref.current = handler
  useEffect(() => {
    if (!active) return undefined
    void listen()
    const id = ++nextId
    stack.push({ id, handler: ref })
    return () => {
      const at = stack.findIndex((entry) => entry.id === id)
      if (at >= 0) stack.splice(at, 1)
    }
  }, [active])
}
