import { AnimatePresence, motion } from 'framer-motion'
import { Laptop, X } from 'lucide-react'
import type { PairingView } from '@/lib/api'
import { formatCountdown, groupPin } from '@/lib/utils'

/**
 * Devices asking to be let in.
 *
 * With the PIN on, this is where the number lives: the client shows the drive,
 * asks for a PIN, and the person reads it off this screen. That direction
 * matters — the host knows *who* is asking and can say so, which the older
 * design (open a window, fetch a PIN, then go to the client) could not.
 *
 * With the PIN off, a request grants itself and never appears here at all.
 */
export function PairingRequests({
  requests,
  onDeny,
}: {
  requests: PairingView[]
  onDeny: (id: string) => void
}): React.JSX.Element {
  return (
    <AnimatePresence initial={false}>
      {requests.map((request) => (
        <motion.div
          key={request.id}
          layout
          initial={{ opacity: 0, y: -8, height: 0 }}
          animate={{ opacity: 1, y: 0, height: 'auto' }}
          exit={{ opacity: 0, y: -8, height: 0 }}
          transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
          className="overflow-hidden"
        >
          <div className="mb-2 flex items-center gap-4 rounded-md border border-lineBright bg-panel2 p-4">
            <span className="text-textDim">
              <Laptop size={16} />
            </span>

            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-semibold tracking-tight text-text">
                {request.deviceName}
              </div>
              <div className="mt-0.5 text-[11px] text-textDim">
                {request.pin
                  ? 'wants to connect. Type this on that device:'
                  : 'wants to connect, and no PIN is being asked for.'}
              </div>
            </div>

            {request.pin && (
              <div className="text-right">
                {/*
                  Large, monospace and split in threes: this number gets read
                  out loud across a room.
                */}
                <div className="tnum font-mono text-[24px] font-semibold leading-none tracking-[0.06em] text-text">
                  {groupPin(request.pin)}
                </div>
                <div className="tnum mt-1.5 font-mono text-[10px] text-textFaint">
                  expires in {formatCountdown(request.secondsLeft)}
                </div>
              </div>
            )}

            <button
              onClick={() => onDeny(request.id)}
              title="Refuse this device"
              aria-label={`Refuse ${request.deviceName}`}
              className="shrink-0 rounded-sm p-1.5 text-textFaint transition-colors hover:bg-dangerBg hover:text-danger"
            >
              <X size={14} />
            </button>
          </div>
        </motion.div>
      ))}
    </AnimatePresence>
  )
}
