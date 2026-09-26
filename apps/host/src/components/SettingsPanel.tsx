import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Check, Film, Image as ImageIcon, Music, Pencil, RefreshCw, Tv, Video } from 'lucide-react'
import type { HostStatus, Sections } from '@/lib/api'
import { About } from './About'
import { Switch } from './ui/Switch'
import { cn, formatAgo } from '@/lib/utils'

/**
 * The settings this app has.
 *
 * Everything else a network share usually asks for — addresses, share names,
 * user accounts, firewall rules, permissions — is either decided by the
 * protocol or not a decision at all.
 *
 * Two of the four are off by default and say what turning them on means,
 * because both do something on the user's behalf that they did not ask for at
 * install time: one lets strangers on the network read the drive, the other
 * reads every folder on it.
 */
export function SettingsPanel({
  status,
  onRequirePin,
  onStartWithWindows,
  onLibrary,
  onRescan,
  onPosters,
  onTmdbKey,
  onSections,
  onRename,
  build,
  onOpenLog,
}: {
  status: HostStatus
  onRequirePin: (require: boolean) => void
  onStartWithWindows: (enabled: boolean) => void
  onLibrary: (enabled: boolean) => void
  onRescan: () => void
  onPosters: (enabled: boolean) => void
  onTmdbKey: (key: string) => void
  onSections: (sections: Sections) => void
  onRename: (name: string) => void
  /** Which build this is, for telling one install from another. */
  build: string
  onOpenLog: () => void
}): React.JSX.Element {
  const [editingName, setEditingName] = useState(false)
  const [draft, setDraft] = useState(status.hostName)

  const commitName = (): void => {
    const name = draft.trim()
    setEditingName(false)
    if (name && name !== status.hostName) onRename(name)
    else setDraft(status.hostName)
  }

  return (
    <div className="rounded-lg glass divide-y divide-line">
      <Row
        title="Ask for a PIN when pairing"
        detail={
          status.requirePin
            ? 'A new device shows up here with a number to type. Nobody joins without someone at this machine.'
            : 'Anyone on this network who finds this machine can read the drive without being let in.'
        }
        warn={!status.requirePin}
        control={
          <Switch
            checked={status.requirePin}
            onChange={onRequirePin}
            label="Ask for a PIN when pairing"
          />
        }
      />

      <Row
        title="Recognise films and series"
        detail={
          status.library.enabled
            ? libraryDetail(status)
            : 'Off. Your devices see folders and files exactly as they are on the drive.'
        }
        control={
          <Switch
            checked={status.library.enabled}
            onChange={onLibrary}
            label="Recognise films and series"
          />
        }
        extra={
          status.library.enabled ? (
            <>
              <button
                onClick={onRescan}
                disabled={status.library.scanning}
                className="mt-2.5 flex items-center gap-1.5 rounded-sm px-2 py-1 text-[11px] text-textFaint transition-colors hover:bg-panel2 hover:text-textDim disabled:opacity-50"
              >
                <RefreshCw size={11} className={status.library.scanning ? 'animate-spin' : ''} />
                {status.library.scanning ? 'Scanning…' : 'Scan again'}
              </button>
              <Artwork status={status} onPosters={onPosters} onSave={onTmdbKey} />
            </>
          ) : null
        }
      />

      <Row
        title="Sections on your devices"
        detail="What each device lists under Library. Hiding a section only tidies the sidebar; every file stays reachable under Files."
        extra={
          <SectionPicker
            sections={status.sections}
            library={status.library}
            onChange={onSections}
          />
        }
      />

      <Row
        title="Start when Windows starts"
        detail="Opens in the notification area at login, so the drive is there before you go looking for it."
        control={
          <Switch
            checked={status.startWithWindows}
            onChange={onStartWithWindows}
            label="Start when Windows starts"
          />
        }
      />

      <Row
        title="This machine's name"
        detail="What your devices see in their list, before they pair."
        control={
          editingName ? (
            <div className="flex items-center gap-1.5">
              <input
                autoFocus
                value={draft}
                maxLength={48}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') commitName()
                  if (e.key === 'Escape') {
                    setDraft(status.hostName)
                    setEditingName(false)
                  }
                }}
                onBlur={commitName}
                className="w-[160px] rounded-sm border border-lineBright bg-ink2 px-2 py-1 font-mono text-[12px] text-text outline-none"
              />
              <button
                onMouseDown={(e) => e.preventDefault()}
                onClick={commitName}
                aria-label="Save the name"
                className="rounded-sm p-1.5 text-textDim transition-colors hover:bg-panel2 hover:text-text"
              >
                <Check size={13} />
              </button>
            </div>
          ) : (
            <button
              onClick={() => {
                setDraft(status.hostName)
                setEditingName(true)
              }}
              className="flex items-center gap-2 rounded-sm px-2 py-1 font-mono text-[12px] text-textDim transition-colors hover:bg-panel2 hover:text-text"
            >
              {status.hostName}
              <Pencil size={11} />
            </button>
          )
        }
      />

      <div className="px-5 py-3.5">
        <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
          Reachable at
        </div>
        <div className="tnum mt-1.5 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11.5px] text-textDim">
          {status.addresses.length > 0 ? (
            status.addresses.map((address) => (
              <span key={address}>
                {address}
                <span className="text-textFaint">:{status.port}</span>
              </span>
            ))
          ) : (
            <span className="text-textFaint">no network connection</span>
          )}
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-textFaint">
          {/* Shown because it is occasionally useful to know, and never
              because anyone has to type it. */}
          For your information only — your devices find this machine by themselves, and keep
          finding it when the address changes.
        </p>

        {/* Which build this is, and where it writes its log. Both exist
            because "am I running the new one?" and "where do I look when it
            misbehaves?" each cost a round trip to answer once. */}
        <div className="tnum mt-3 border-t border-line pt-3 font-mono text-[10px] text-textFaint">
          <div>{build || 'build unknown'}</div>
          <button
            onClick={onOpenLog}
            className="mt-1 underline decoration-dotted underline-offset-2 transition-colors hover:text-textDim"
          >
            open the log folder
          </button>
        </div>
      </div>

      <About product="Basalt Host" />
    </div>
  )
}

/**
 * What the index found, or what it is doing.
 *
 * A scan in progress says so rather than reporting zero, which would read as
 * "nothing on your drive" at exactly the moment that is least likely to be
 * true.
 */
function libraryDetail(status: HostStatus): string {
  const { scanning, films, series, uncertain, scannedAt } = status.library
  if (scanning) return 'Scanning the drive…'
  if (films === 0 && series === 0) {
    return 'Nothing recognised yet. Films and series show up in their own sections on your devices.'
  }

  const parts = [
    `${films} ${films === 1 ? 'film' : 'films'}`,
    `${series} ${series === 1 ? 'series' : 'series'}`,
  ]
  const found = `${parts.join(' and ')}, last checked ${formatAgo(scannedAt)}.`
  return uncertain > 0
    ? `${found} ${uncertain} ${uncertain === 1 ? 'is a guess' : 'are guesses'}.`
    : found
}

/**
 * The poster downloads, and the key that unlocks them.
 *
 * Folded under the library switch rather than given a row of its own, because
 * it is a refinement of that setting and meaningless without it.
 *
 * The key is never displayed once saved. The host reports whether one is set,
 * not what it is, so there is nothing here that could read it back out.
 */
function Artwork({
  status,
  onPosters,
  onSave,
}: {
  status: HostStatus
  onPosters: (enabled: boolean) => void
  onSave: (key: string) => void
}): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState('')
  const { posters, hasKey, withArt, films, series } = status.library
  const total = films + series

  return (
    <div className="mt-2.5 border-t border-line pt-2.5">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[12px] text-textDim">Download posters</div>
          <p className="mt-0.5 text-[11px] leading-relaxed text-textFaint">
            {/* Said plainly, because this is the actual cost of the switch and
                no key is required any more to make somebody think about it. */}
            {posters
              ? `Sending each recognised title to a lookup service. ${withArt} of ${total} have artwork.`
              : 'Off. Covers are drawn from the title. Turning this on sends each recognised title to a lookup service.'}
          </p>
        </div>
        <div className="mt-0.5 shrink-0">
          <Switch checked={posters} onChange={onPosters} label="Download posters" />
        </div>
      </div>

      {!posters ? null : !open ? (
        <button
          onClick={() => {
            setDraft('')
            setOpen(true)
          }}
          className="mt-1.5 flex items-center gap-1.5 rounded-sm px-2 py-1 text-[11px] text-textFaint transition-colors hover:bg-panel2 hover:text-textDim"
        >
          <ImageIcon size={11} />
          {hasKey ? 'A TMDb key is saved' : 'Add a TMDb key for more coverage…'}
        </button>
      ) : (
        <motion.div
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: 'auto' }}
          transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
          className="overflow-hidden"
        >
          <div className="mt-1 rounded-md border border-line bg-ink2 p-3">
            <p className="text-[11px] leading-relaxed text-textFaint">
              {/* The key is genuinely optional now. Saying so matters: asking
                  for one when none is needed is how an app trains people to
                  paste credentials they were never required to have. */}
              Posters already work without this. A free TMDb key is only
              consulted for titles the default source has never heard of, so it
              widens coverage and nothing more.
            </p>
            <div className="mt-2.5 flex items-center gap-1.5">
              <input
                autoFocus
                type="password"
                value={draft}
                placeholder={hasKey ? 'a key is saved — paste a new one to replace it' : 'TMDb API key'}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    onSave(draft)
                    setOpen(false)
                  }
                  if (e.key === 'Escape') setOpen(false)
                }}
                className="min-w-0 flex-1 rounded-sm border border-line bg-panel px-2 py-1.5 font-mono text-[11.5px] text-text outline-none transition-colors placeholder:text-textFaint/60 focus:border-lineBright"
              />
              <button
                onClick={() => {
                  onSave(draft)
                  setOpen(false)
                }}
                className="rounded-sm px-2.5 py-1.5 text-[11px] text-textDim transition-colors hover:bg-panel2 hover:text-text"
              >
                Save
              </button>
              <button
                onClick={() => setOpen(false)}
                className="rounded-sm px-2 py-1.5 text-[11px] text-textFaint transition-colors hover:text-textDim"
              >
                Cancel
              </button>
            </div>
            {hasKey && (
              <button
                onClick={() => {
                  onSave('')
                  setOpen(false)
                }}
                className="mt-2 text-[10.5px] text-textFaint transition-colors hover:text-danger"
              >
                Remove the key
              </button>
            )}
          </div>
        </motion.div>
      )}
    </div>
  )
}

function Row({
  title,
  detail,
  control,
  warn,
  extra,
}: {
  title: string
  detail: string
  control?: React.ReactNode
  warn?: boolean
  /** Rendered under the detail, for a control the row itself cannot hold. */
  extra?: React.ReactNode
}): React.JSX.Element {
  return (
    <div className="flex items-start gap-4 px-5 py-4">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-text">{title}</div>
        <AnimatePresence mode="wait" initial={false}>
          <motion.p
            key={detail}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            className={
              warn
                ? 'mt-1 text-[11.5px] leading-relaxed text-danger'
                : 'mt-1 text-[11.5px] leading-relaxed text-textFaint'
            }
          >
            {detail}
          </motion.p>
        </AnimatePresence>
        {extra}
      </div>
      <div className="mt-0.5 shrink-0">{control}</div>
    </div>
  )
}

interface SectionCard {
  key: keyof Sections
  label: string
  icon: typeof Film
  /** How much is in it, in words: "42 films". */
  amount: (library: HostStatus['library']) => string
  /** Filled by recognising films and series, which can be switched off. */
  recognised?: boolean
}

const SECTION_CARDS: SectionCard[] = [
  { key: 'movies', label: 'Movies', icon: Film, recognised: true, amount: (l) => count(l.films, 'film') },
  { key: 'series', label: 'TV Series', icon: Tv, recognised: true, amount: (l) => count(l.series, 'series', 'series') },
  { key: 'videos', label: 'Videos', icon: Video, amount: (l) => count(l.videos, 'video') },
  { key: 'music', label: 'Music', icon: Music, amount: (l) => count(l.music, 'song') },
  { key: 'photos', label: 'Photos', icon: ImageIcon, amount: (l) => count(l.photos, 'photo') },
]

function count(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`
}

/**
 * One card per section: what it is, how much is in it, and whether devices
 * show it.
 *
 * Cards rather than a row of chips, because each carries more than a name —
 * a section with nothing in it is worth knowing about before deciding to show
 * it, and Movies and TV Series are empty until films are being recognised.
 * The whole card is the control; the small switch in its corner only says
 * which way it is set.
 */
function SectionPicker({
  sections,
  library,
  onChange,
}: {
  sections: Sections
  library: HostStatus['library']
  onChange: (sections: Sections) => void
}): React.JSX.Element {
  const shown = SECTION_CARDS.filter((c) => sections[c.key]).length

  return (
    <div className="mt-3.5">
      <div className="grid grid-cols-5 gap-2">
        {SECTION_CARDS.map((card) => {
          const on = sections[card.key]
          const waiting = card.recognised && !library.enabled
          const Icon = card.icon
          return (
            <motion.button
              key={card.key}
              type="button"
              role="switch"
              aria-checked={on}
              aria-label={`Show ${card.label} on devices`}
              onClick={() => onChange({ ...sections, [card.key]: !on })}
              whileTap={{ scale: 0.97 }}
              transition={{ type: 'spring', stiffness: 600, damping: 32 }}
              className={cn(
                'group relative flex flex-col items-start overflow-hidden rounded-xl border px-3 pb-3 pt-3 text-left transition-[background-color,border-color] duration-200',
                on
                  ? 'border-white/[0.16] bg-white/[0.055] hover:border-white/25'
                  : 'border-line bg-transparent hover:border-white/[0.12] hover:bg-white/[0.02]',
              )}
            >
              {/* A soft light from above on the ones that are shown. */}
              <span
                aria-hidden
                className={cn(
                  'pointer-events-none absolute inset-x-0 top-0 h-12 bg-gradient-to-b from-white/[0.06] to-transparent transition-opacity duration-300',
                  on ? 'opacity-100' : 'opacity-0',
                )}
              />

              <div className="relative flex w-full items-start justify-between">
                <span
                  className={cn(
                    'flex h-8 w-8 items-center justify-center rounded-lg transition-colors duration-200',
                    on ? 'bg-white/[0.1] text-text' : 'bg-white/[0.03] text-textFaint',
                  )}
                >
                  <Icon size={15} strokeWidth={1.8} />
                </span>
                <MiniSwitch on={on} />
              </div>

              <span
                className={cn(
                  'relative mt-3 text-[12.5px] font-medium transition-colors duration-200',
                  on ? 'text-text' : 'text-textDim',
                )}
              >
                {card.label}
              </span>
              <span className="tnum relative mt-0.5 truncate font-mono text-[10px] text-textFaint">
                {!on ? 'Hidden' : waiting ? 'Recognition off' : card.amount(library)}
              </span>
            </motion.button>
          )
        })}
      </div>
      <p className="mt-2.5 text-[11px] text-textFaint">
        {shown === SECTION_CARDS.length
          ? 'Every section is shown.'
          : shown === 0
            ? 'Devices show Files, Recent and Starred only.'
            : `${shown} of ${SECTION_CARDS.length} sections shown. Changes reach connected devices at once.`}
      </p>
    </div>
  )
}

/** The switch in a card's corner: an indicator, not a second control. */
function MiniSwitch({ on }: { on: boolean }): React.JSX.Element {
  return (
    <span
      aria-hidden
      className={cn(
        'relative h-[14px] w-[24px] shrink-0 rounded-full border transition-colors duration-200',
        on ? 'border-transparent bg-basalt/90' : 'border-line bg-panel2',
      )}
    >
      <motion.span
        layout
        transition={{ type: 'spring', stiffness: 560, damping: 34 }}
        className={cn(
          'absolute top-[2px] h-[8px] w-[8px] rounded-full',
          on ? 'right-[2px] bg-ink' : 'left-[2px] bg-textFaint',
        )}
      />
    </span>
  )
}
