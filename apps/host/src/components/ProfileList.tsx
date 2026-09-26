import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { KeyRound, Laptop, MoreHorizontal, Trash2, UserRound } from 'lucide-react'
import type { ProfileSummary } from '@/lib/api'
import { cn, formatAgo } from '@/lib/utils'

/**
 * The household's profiles, and which devices are signed in to each.
 *
 * Names only. A PIN is never shown here, nor anywhere else: the host keeps a
 * hash of it and nothing more. What the host's owner can do is what a
 * forgotten PIN needs — clear it, so the next sign-in chooses a new one — and
 * remove a profile outright, with its history and stars.
 */

/** Avatar colours, muted to sit in a graphite interface. Mirrors the client. */
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
  return PROFILE_COLORS[((index % PROFILE_COLORS.length) + PROFILE_COLORS.length) % PROFILE_COLORS.length]!
}

export function Avatar({
  name,
  color,
  size = 40,
}: {
  name: string
  color: number
  size?: number
}): React.JSX.Element {
  const tone = profileColor(color)
  return (
    <span
      className="flex shrink-0 items-center justify-center rounded-full font-semibold text-white"
      style={{
        width: size,
        height: size,
        fontSize: size * 0.4,
        background: `linear-gradient(145deg, ${tone}, ${tone}B0)`,
        boxShadow: `inset 0 1px 0 rgba(255,255,255,0.22), 0 0 0 1px rgba(255,255,255,0.06)`,
      }}
      aria-hidden
    >
      {initialOf(name)}
    </span>
  )
}

function initialOf(name: string): string {
  return (name.trim()[0] ?? '?').toUpperCase()
}

export function ProfileList({
  profiles,
  onResetPin,
  onRemove,
}: {
  profiles: ProfileSummary[]
  onResetPin: (profile: ProfileSummary) => void
  onRemove: (profile: ProfileSummary) => void
}): React.JSX.Element {
  if (profiles.length === 0) {
    return (
      <div className="flex items-center gap-4 rounded-lg border border-dashed border-line px-5 py-5">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white/[0.04] text-textFaint">
          <UserRound size={18} />
        </span>
        <div>
          <p className="text-[13px] text-textDim">No profiles yet.</p>
          <p className="mt-1 max-w-[460px] text-[11.5px] leading-relaxed text-textFaint">
            Anyone on a paired device can make one when they open Basalt. A profile keeps its
            own watch history and stars, and takes them from one device to the next.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 gap-2">
      <AnimatePresence initial={false}>
        {profiles.map((profile) => (
          <motion.div
            key={profile.id}
            layout
            initial={{ opacity: 0, scale: 0.98 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.98 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
          >
            <ProfileCard
              profile={profile}
              onResetPin={() => onResetPin(profile)}
              onRemove={() => onRemove(profile)}
            />
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  )
}

function ProfileCard({
  profile,
  onResetPin,
  onRemove,
}: {
  profile: ProfileSummary
  onResetPin: () => void
  onRemove: () => void
}): React.JSX.Element {
  const [menu, setMenu] = useState(false)
  const now = Date.now() / 1000
  const active = profile.devices.filter((d) => now - d.lastUsed < 10 * 60).length

  return (
    <div className="relative h-full rounded-lg glass px-4 py-3.5">
      <div className="flex items-center gap-3">
        <div className="relative">
          <Avatar name={profile.name} color={profile.color} />
          {active > 0 && (
            <span
              title="In use now"
              className="absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-panel bg-basalt"
            />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] font-semibold text-text">{profile.name}</div>
          <div className="mt-0.5 truncate font-mono text-[10px] text-textFaint">
            {profile.lastUsed > 0 ? `active ${formatAgo(profile.lastUsed)}` : 'not used yet'}
          </div>
        </div>
        <button
          onClick={() => setMenu((open) => !open)}
          aria-label={`Options for ${profile.name}`}
          className="flex h-7 w-7 items-center justify-center rounded-md text-textFaint transition-colors duration-150 hover:bg-white/[0.06] hover:text-text"
        >
          <MoreHorizontal size={15} />
        </button>
      </div>

      {!profile.hasPin && (
        <div className="mt-3 flex items-center gap-2 rounded-md bg-white/[0.04] px-2.5 py-1.5 text-[11px] text-textDim">
          <KeyRound size={12} className="shrink-0 text-textFaint" />
          PIN cleared. The next sign-in chooses a new one.
        </div>
      )}

      <div className="mt-3 border-t border-line pt-2.5">
        <div className="font-mono text-[9.5px] uppercase tracking-[0.16em] text-textFaint">
          {profile.devices.length === 0
            ? 'Not signed in anywhere'
            : `Signed in on ${profile.devices.length} ${profile.devices.length === 1 ? 'device' : 'devices'}`}
        </div>
        {profile.devices.length > 0 && (
          <ul className="mt-2 space-y-1.5">
            {profile.devices.map((device, index) => (
              <li key={`${device.name}-${index}`} className="flex items-center gap-2 text-[12px]">
                <Laptop size={12} className="shrink-0 text-textFaint" />
                <span className="min-w-0 flex-1 truncate text-textDim">{device.name}</span>
                <span
                  className={cn(
                    'shrink-0 rounded-full px-1.5 py-px font-mono text-[9px]',
                    device.remembered
                      ? 'bg-white/[0.06] text-textDim'
                      : 'border border-line text-textFaint',
                  )}
                  title={
                    device.remembered
                      ? 'Stays signed in on this device'
                      : 'Signed in until the app closes'
                  }
                >
                  {device.remembered ? 'remembered' : 'this session'}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <AnimatePresence>
        {menu && (
          <>
            <div className="fixed inset-0 z-10" onClick={() => setMenu(false)} />
            <motion.div
              initial={{ opacity: 0, scale: 0.97, y: -4 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.98 }}
              transition={{ duration: 0.13, ease: [0.22, 1, 0.36, 1] }}
              style={{ transformOrigin: 'top right' }}
              className="absolute right-3 top-11 z-20 w-[190px] overflow-hidden rounded-md border border-white/10 bg-panel2 p-1 shadow-lift"
            >
              <MenuItem
                icon={KeyRound}
                label="Reset PIN"
                onClick={() => {
                  setMenu(false)
                  onResetPin()
                }}
              />
              <MenuItem
                icon={Trash2}
                label="Remove profile"
                danger
                onClick={() => {
                  setMenu(false)
                  onRemove()
                }}
              />
            </motion.div>
          </>
        )}
      </AnimatePresence>
    </div>
  )
}

function MenuItem({
  icon: Icon,
  label,
  danger,
  onClick,
}: {
  icon: typeof Trash2
  label: string
  danger?: boolean
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className={cn(
        'flex w-full items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[12px] transition-colors duration-100',
        danger ? 'text-danger hover:bg-dangerBg' : 'text-textDim hover:bg-white/[0.06] hover:text-text',
      )}
    >
      <Icon size={13} />
      {label}
    </button>
  )
}
