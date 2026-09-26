import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type IdentityState, type ProfileView } from './api'

/**
 * Who is using this device: a profile, or the device on its own.
 *
 * Asked after connecting, and again every so often, so a profile removed or
 * signed out on the host is noticed without anybody having to do anything.
 *
 * A host from before profiles cannot list any, and then there is nothing to
 * choose: the device simply carries on as itself, as it always did.
 */
export interface Identity {
  state: IdentityState | null
  /** Profiles on the host, once asked. */
  profiles: ProfileView[]
  /** False for a host that has no profiles to offer at all. */
  supported: boolean
  loaded: boolean
  refresh: () => Promise<void>
  /** Loads the host's list again, for the chooser. */
  reloadProfiles: () => Promise<void>
}

const RECHECK_MS = 30_000

export function useIdentity(connected: boolean): Identity {
  const [state, setState] = useState<IdentityState | null>(null)
  const [profiles, setProfiles] = useState<ProfileView[]>([])
  const [supported, setSupported] = useState(true)
  const [loaded, setLoaded] = useState(false)
  const live = useRef(true)

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const reloadProfiles = useCallback(async () => {
    try {
      const list = await api.profiles()
      if (!live.current) return
      setProfiles(list)
      setSupported(true)
    } catch {
      if (live.current) setSupported(false)
    }
  }, [])

  const refresh = useCallback(async () => {
    try {
      const next = await api.identity()
      if (live.current) setState(next)
    } catch {
      // Not connected: asked again when it is.
    }
  }, [])

  useEffect(() => {
    if (!connected) {
      setState(null)
      setLoaded(false)
      return undefined
    }
    let cancelled = false
    void Promise.all([refresh(), reloadProfiles()]).then(() => {
      if (!cancelled) setLoaded(true)
    })
    const timer = setInterval(() => void refresh(), RECHECK_MS)
    return () => {
      cancelled = true
      clearInterval(timer)
    }
  }, [connected, refresh, reloadProfiles])

  return { state, profiles, supported, loaded, refresh, reloadProfiles }
}

/** Avatar colours, muted to sit in a graphite interface. Mirrors the host. */
export const PROFILE_COLORS = [
  '#7384D8',
  '#4E9EA0',
  '#6BA674',
  '#C4A157',
  '#CC7E68',
  '#C27391',
  '#957AC9',
  '#8A8F98',
]

export function profileColor(index: number): string {
  const n = PROFILE_COLORS.length
  return PROFILE_COLORS[((index % n) + n) % n]!
}

/** Four to eight digits. Mirrors the host. */
export function validPin(pin: string): boolean {
  return /^[0-9]{4,8}$/.test(pin)
}
