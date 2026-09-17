import { motion } from 'framer-motion'
import { AlertTriangle, ExternalLink, Repeat } from 'lucide-react'
import type { HostStatus } from '@/lib/api'
import { cn, formatBytes } from '@/lib/utils'

/**
 * What is being shared, and how much room is left on it.
 *
 * The gauge is the only large graphic in the app because it is the only number
 * anyone checks repeatedly. Everything else here answers a question you ask
 * once.
 */
export function VaultCard({
  status,
  onChange,
  onOpen,
}: {
  status: HostStatus
  onChange: () => void
  onOpen: () => void
}): React.JSX.Element | null {
  const vault = status.vault
  if (!vault) return null

  const used = vault.total > 0 ? (vault.total - vault.free) / vault.total : 0

  return (
    <div className="rounded-lg glass p-5">
      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-[18px] font-semibold tracking-tighter text-text">
              {vault.name}
            </h2>
            {!vault.available && (
              <span className="flex shrink-0 items-center gap-1 rounded-[4px] bg-dangerBg px-1.5 py-[2px] font-mono text-[9px] uppercase tracking-[0.1em] text-danger">
                <AlertTriangle size={9} />
                unplugged
              </span>
            )}
          </div>
          <p className="mt-1 truncate font-mono text-[11px] text-textFaint" title={vault.path}>
            {vault.path}
          </p>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <IconButton icon={<ExternalLink size={12} />} label="Open in Explorer" onClick={onOpen} />
          <IconButton icon={<Repeat size={12} />} label="Share a different drive" onClick={onChange} />
        </div>
      </div>

      {vault.total > 0 ? (
        <>
          <div className="mt-5 h-[6px] w-full overflow-hidden rounded-full bg-ink2">
            <motion.div
              className="h-full rounded-full bg-basaltDeep"
              initial={{ width: 0 }}
              animate={{ width: `${Math.round(used * 100)}%` }}
              transition={{ duration: 0.7, ease: [0.22, 1, 0.36, 1] }}
            />
          </div>
          <div className="tnum mt-2.5 flex items-baseline justify-between font-mono text-[11px]">
            <span className="text-textDim">{formatBytes(vault.free)} free</span>
            <span className="text-textFaint">{formatBytes(vault.total)} total</span>
          </div>
        </>
      ) : (
        <p className="mt-5 font-mono text-[11px] text-textFaint">
          {/* Some USB enclosures decline to report a size. Not worth an error —
              the share works regardless — but the gauge would be a lie. */}
          this volume does not report its size
        </p>
      )}

      {!vault.available && (
        <p className="mt-4 rounded-sm bg-dangerBg px-3 py-2 text-[12px] leading-relaxed text-danger">
          This drive is not connected any more. Devices can still find this machine, but
          nothing can be read until you plug it back in.
        </p>
      )}
    </div>
  )
}

function IconButton({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode
  label: string
  onClick: () => void
}): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className={cn(
        'rounded-sm p-2 text-textFaint transition-colors hover:bg-panel2 hover:text-text',
      )}
    >
      {icon}
    </button>
  )
}
