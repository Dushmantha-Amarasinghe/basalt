import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  ArrowDown,
  ArrowUp,
  Laptop,
  MoreHorizontal,
  Pencil,
  ShieldOff,
  Trash2,
} from 'lucide-react'
import type { DeviceView } from '@/lib/api'
import { cn, formatAgo, formatBytes, formatRate } from '@/lib/utils'

/**
 * The paired devices, and what each is doing right now.
 *
 * Two numbers per device, both measured rather than inferred: cumulative bytes
 * since the host started, and a rate computed over the actual interval between
 * two polls. Nothing here divides by an assumed tick length, which is how the
 * client once came to display 35 MB/s over a 22.7 MB/s link.
 */
export function DeviceList({
  devices,
  onRevoke,
  onRename,
  onToggleWritable,
}: {
  devices: DeviceView[]
  onRevoke: (device: DeviceView) => void
  onRename: (device: DeviceView) => void
  onToggleWritable: (device: DeviceView) => void
}): React.JSX.Element {
  if (devices.length === 0) {
    return (
      <div className="rounded-md border border-dashed border-line px-4 py-10 text-center">
        <p className="text-[13px] text-textDim">No devices yet.</p>
        <p className="mx-auto mt-1.5 max-w-[360px] text-[11.5px] leading-relaxed text-textFaint">
          Open Basalt on another machine on this network. It will list this drive by name —
          there is no address to type.
        </p>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-1.5">
      <AnimatePresence initial={false}>
        {devices.map((device) => (
          <motion.div
            key={device.id}
            layout
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0, height: 0, marginBottom: 0 }}
            transition={{ duration: 0.2 }}
          >
            <DeviceRow
              device={device}
              onRevoke={() => onRevoke(device)}
              onRename={() => onRename(device)}
              onToggleWritable={() => onToggleWritable(device)}
            />
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  )
}

function DeviceRow({
  device,
  onRevoke,
  onRename,
  onToggleWritable,
}: {
  device: DeviceView
  onRevoke: () => void
  onRename: () => void
  onToggleWritable: () => void
}): React.JSX.Element {
  const [menuOpen, setMenuOpen] = useState(false)
  const moving = device.sendRate > 0 || device.receiveRate > 0

  return (
    <div
      className={cn(
        'group relative flex items-center gap-3 rounded-md border border-line px-3.5 py-3 transition-colors',
        // Background only. `border-line/60` would *replace* the token's own
        // 0.07 alpha with 0.6 rather than scaling it, drawing the offline row
        // with a border nine times brighter than the connected one.
        device.online ? 'bg-panel2' : 'bg-panel/50',
      )}
    >
      <span className={cn('shrink-0', device.online ? 'text-text' : 'text-textFaint')}>
        <Laptop size={15} />
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span
            className={cn(
              'truncate text-[13px] font-medium',
              device.online ? 'text-text' : 'text-textDim',
            )}
          >
            {device.name}
          </span>

          {/*
            Online means an open connection, not a recent timestamp. A laptop
            that shut its lid an hour ago still has a recent `lastSeen`, and
            calling that online would be a lie the user would act on.
          */}
          {device.online ? (
            <span className="flex items-center gap-1.5 font-mono text-[9.5px] uppercase tracking-[0.14em] text-textFaint">
              <span
                className={cn(
                  'h-1.5 w-1.5 rounded-full bg-basalt',
                  moving && 'animate-breathe',
                )}
              />
              connected
            </span>
          ) : (
            <span className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-textFaint">
              {formatAgo(device.lastSeen)}
            </span>
          )}

          {!device.writable && (
            <span
              title="This device can read but not change anything"
              className="rounded-[4px] border border-line px-1.5 py-[1px] font-mono text-[9px] uppercase tracking-[0.1em] text-textFaint"
            >
              read only
            </span>
          )}
        </div>

        <div className="tnum mt-1 flex items-center gap-3 font-mono text-[10.5px] text-textFaint">
          <span title="Sent to this device">↑ {formatBytes(device.sent)}</span>
          <span title="Received from this device">↓ {formatBytes(device.received)}</span>
        </div>
      </div>

      {/* The live rates. Fixed width so the column does not jitter as digits
          change, and hidden entirely when nothing is moving. */}
      <div className="tnum hidden w-[104px] shrink-0 flex-col items-end gap-0.5 font-mono text-[11px] sm:flex">
        <Rate icon={<ArrowUp size={9} />} value={device.sendRate} />
        <Rate icon={<ArrowDown size={9} />} value={device.receiveRate} />
      </div>

      <div className="relative shrink-0">
        <button
          onClick={() => setMenuOpen((open) => !open)}
          aria-label={`Options for ${device.name}`}
          className={cn(
            'rounded-sm p-1.5 transition-colors',
            menuOpen
              ? 'bg-panel2 text-text'
              : 'text-textFaint opacity-0 hover:bg-panel2 hover:text-text group-hover:opacity-100 focus:opacity-100',
          )}
        >
          <MoreHorizontal size={14} />
        </button>

        <AnimatePresence>
          {menuOpen && (
            <>
              {/* Catches the next click anywhere, so the menu closes the way
                  every other menu does. */}
              <div className="fixed inset-0 z-40" onClick={() => setMenuOpen(false)} />
              <motion.div
                initial={{ opacity: 0, scale: 0.96, y: -4 }}
                animate={{ opacity: 1, scale: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.96, y: -4 }}
                transition={{ duration: 0.12 }}
                className="absolute right-0 top-8 z-50 w-[188px] origin-top-right overflow-hidden rounded-md border border-lineBright bg-panel2 py-1 shadow-lift"
              >
                <MenuItem
                  icon={<Pencil size={12} />}
                  label="Rename"
                  onClick={() => {
                    setMenuOpen(false)
                    onRename()
                  }}
                />
                <MenuItem
                  icon={<ShieldOff size={12} />}
                  label={device.writable ? 'Make read only' : 'Allow changes'}
                  onClick={() => {
                    setMenuOpen(false)
                    onToggleWritable()
                  }}
                />
                <div className="my-1 h-px bg-line" />
                <MenuItem
                  icon={<Trash2 size={12} />}
                  label="Remove this device"
                  danger
                  onClick={() => {
                    setMenuOpen(false)
                    onRevoke()
                  }}
                />
              </motion.div>
            </>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}

function Rate({ icon, value }: { icon: React.ReactNode; value: number }): React.JSX.Element {
  const idle = value < 1
  return (
    <span
      className={cn(
        'flex items-center gap-1 transition-colors',
        idle ? 'text-textFaint/40' : 'text-textDim',
      )}
    >
      {icon}
      {formatRate(value)}
    </span>
  )
}

function MenuItem({
  icon,
  label,
  onClick,
  danger,
}: {
  icon: React.ReactNode
  label: string
  onClick: () => void
  danger?: boolean
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className={cn(
        'flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-[12px] transition-colors',
        danger
          ? 'text-danger hover:bg-dangerBg'
          : 'text-textDim hover:bg-panel hover:text-text',
      )}
    >
      <span className="shrink-0">{icon}</span>
      {label}
    </button>
  )
}
