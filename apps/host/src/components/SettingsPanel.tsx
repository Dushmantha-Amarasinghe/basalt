import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Check, Pencil } from 'lucide-react'
import type { HostStatus } from '@/lib/api'
import { Switch } from './ui/Switch'

/**
 * The three settings this app has.
 *
 * Deliberately three. Everything else a network share usually asks for —
 * addresses, share names, user accounts, firewall rules, permissions — is
 * either decided by the protocol or not a decision at all.
 */
export function SettingsPanel({
  status,
  onRequirePin,
  onStartWithWindows,
  onRename,
}: {
  status: HostStatus
  onRequirePin: (require: boolean) => void
  onStartWithWindows: (enabled: boolean) => void
  onRename: (name: string) => void
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
      </div>
    </div>
  )
}

function Row({
  title,
  detail,
  control,
  warn,
}: {
  title: string
  detail: string
  control: React.ReactNode
  warn?: boolean
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
      </div>
      <div className="mt-0.5 shrink-0">{control}</div>
    </div>
  )
}
