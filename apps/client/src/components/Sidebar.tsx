import { motion } from 'framer-motion'
import {
  Clapperboard,
  Clock,
  FolderOpen,
  HardDrive,
  Image,
  Music,
  Settings2,
  Star,
  Tv,
  Video,
} from 'lucide-react'
import { cn, formatBytes } from '@/lib/utils'

export type NavKey =
  | 'files'
  | 'recent'
  | 'starred'
  | 'movies'
  | 'series'
  | 'videos'
  | 'music'
  | 'photos'
  | 'settings'

const NAV: { key: NavKey; label: string; icon: typeof FolderOpen }[] = [
  { key: 'files', label: 'Files', icon: FolderOpen },
  { key: 'recent', label: 'Recent', icon: Clock },
  { key: 'starred', label: 'Starred', icon: Star },
]

/**
 * Films and series come first because they are the reason most people open
 * this — and they sit above the raw media sections rather than replacing them,
 * since anything the index did not recognise is still findable there.
 */
const LIBRARY: { key: NavKey; label: string; icon: typeof FolderOpen }[] = [
  { key: 'movies', label: 'Movies', icon: Clapperboard },
  { key: 'series', label: 'TV Series', icon: Tv },
  { key: 'videos', label: 'Videos', icon: Video },
  { key: 'music', label: 'Music', icon: Music },
  { key: 'photos', label: 'Photos', icon: Image },
]

export function Sidebar({
  active,
  onNavigate,
  driveUsed,
  driveTotal,
  connected,
  vaultName = 'Vault',
  hidden,
}: {
  /** Library sections the host's owner has chosen not to show. */
  hidden?: ReadonlySet<NavKey>
  active: NavKey
  onNavigate: (key: NavKey) => void
  driveUsed: number
  driveTotal: number
  connected: boolean
  vaultName?: string
}): React.JSX.Element {
  const usedPercent = driveTotal > 0 ? (driveUsed / driveTotal) * 100 : 0

  return (
    <aside className="z-10 flex w-[210px] shrink-0 flex-col border-r border-line px-3 py-4">
      {/*
        `min-h-0` plus `overflow-y-auto` is what stops a short window from
        pushing the drive card and Settings out of the bottom of the sidebar.
        Without it this column simply overflows its parent and the whole layout
        visibly breaks.
      */}
      <div className="fade-bottom flex min-h-0 flex-1 flex-col overflow-y-auto">
      <nav className="flex flex-col gap-1">
        {NAV.map((item) => (
          <NavItem
            key={item.key}
            navKey={item.key}
            label={item.label}
            icon={item.icon}
            active={active}
            onNavigate={onNavigate}
          />
        ))}
      </nav>

      {LIBRARY.some((item) => !hidden?.has(item.key)) && <SectionLabel>Library</SectionLabel>}
      <nav className="flex flex-col gap-1">
        {LIBRARY.filter((item) => !hidden?.has(item.key)).map((item) => (
          <NavItem
            key={item.key}
            navKey={item.key}
            label={item.label}
            icon={item.icon}
            active={active}
            onNavigate={onNavigate}
          />
        ))}
      </nav>
      <div className="min-h-4 flex-1" />
      </div>

      <DriveStatus
        connected={connected}
        used={driveUsed}
        total={driveTotal}
        usedPercent={usedPercent}
        vaultName={vaultName}
      />

      <nav className="mt-1 flex flex-col gap-1">
        <NavItem
          navKey="settings"
          label="Settings"
          icon={Settings2}
          active={active}
          onNavigate={onNavigate}
        />
      </nav>
    </aside>
  )
}

function SectionLabel({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="mt-5 mb-2 px-3 font-mono text-[10px] uppercase tracking-[0.18em] text-textFaint">
      {children}
    </div>
  )
}

function NavItem({
  navKey,
  label,
  icon: Icon,
  active,
  onNavigate,
}: {
  navKey: NavKey
  label: string
  icon: typeof FolderOpen
  active: NavKey
  onNavigate: (key: NavKey) => void
}): React.JSX.Element {
  const isActive = active === navKey

  return (
    <button
      onClick={() => onNavigate(navKey)}
      className={cn(
        'no-drag relative flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium transition-colors',
        isActive ? 'text-basalt' : 'text-textDim hover:bg-white/[0.03] hover:text-text',
      )}
    >
      {/*
        The shared-element highlight. `layoutId` makes Framer Motion animate the
        pill between items rather than cross-fading two of them, which is the
        detail that makes the navigation feel physical.
      */}
      {isActive && (
        <motion.span
          layoutId="nav-active"
          className="absolute inset-0 rounded-md bg-basalt/10 ring-1 ring-inset ring-basalt/25"
          transition={{ type: 'spring', stiffness: 500, damping: 34 }}
        />
      )}
      <Icon size={18} className="relative z-10" />
      <span className="relative z-10">{label}</span>
    </button>
  )
}

function DriveStatus({
  connected,
  used,
  total,
  usedPercent,
  vaultName,
}: {
  connected: boolean
  used: number
  total: number
  usedPercent: number
  vaultName: string
}): React.JSX.Element {
  return (
    <div className="glass rounded-md px-3 py-3">
      <div className="flex items-center gap-2">
        <HardDrive size={14} className="shrink-0 text-textDim" />
        <span className="truncate text-xs font-semibold text-text">{vaultName}</span>
        {/*
          Connection shows as a word, not a coloured dot. The palette is
          deliberately monochrome, and a green pip was the one thing breaking
          it for information the text already carries.
        */}
        <span className="ml-auto shrink-0 font-mono text-[9px] uppercase tracking-[0.14em] text-textFaint">
          {connected ? 'online' : 'offline'}
        </span>
      </div>

      <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-white/[0.06]">
        <div
          className="h-full rounded-full bg-basaltDeep transition-[width] duration-500"
          style={{ width: `${Math.min(100, usedPercent)}%` }}
        />
      </div>

      <div className="mt-2 flex items-baseline justify-between gap-2 whitespace-nowrap">
        <span className="tnum font-mono text-[10px] text-textFaint">
          {usedOfTotal(used, total)}
        </span>
        {/* Free space rather than a speed. The speed lives in the title bar;
            two readouts of one number were two chances to disagree. */}
        {total > 0 && (
          <span className="tnum font-mono text-[10px] text-textDim">
            {formatBytes(Math.max(0, total - used))} free
          </span>
        )}
      </div>
    </div>
  )
}

/**
 * `439 / 500 GB` rather than `439 GB / 500 GB` when both share a unit — the
 * card is narrow, and the longer form pushed the free space onto a second
 * line.
 */
function usedOfTotal(used: number, total: number): string {
  const u = formatBytes(used)
  const t = formatBytes(total)
  const [uNumber, uUnit] = u.split(' ')
  const [, tUnit] = t.split(' ')
  return uUnit && uUnit === tUnit ? `${uNumber} / ${t}` : `${u} / ${t}`
}
