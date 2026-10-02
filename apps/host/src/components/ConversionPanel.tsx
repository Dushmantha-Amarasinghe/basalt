import { motion } from 'framer-motion'
import { Cpu, Loader2, RefreshCw, Smartphone } from 'lucide-react'
import type { ConversionStatus } from '@/lib/api'
import { cn } from '@/lib/utils'

/** Choices for "at once", besides measuring. */
const BY_HAND = [1, 2, 3, 4, 5, 6]

/**
 * What the switch's row says about converting video, by what is known.
 */
export function conversionDetail(c: ConversionStatus): string {
  if (!c.detected) return 'Looking at what this computer can convert video with…'
  if (!c.available) {
    return 'This computer has nothing to convert video with. A device that can’t play a file plays it in a lighter mode.'
  }
  if (!c.enabled) {
    return 'Off. A device that can’t play a file as it is plays it in a lighter mode instead.'
  }
  if (c.measured === null && c.measuring) {
    return 'Measuring how many devices this computer can convert for at once…'
  }
  if (c.limit === 0) {
    return 'This computer is too slow to convert 4K as it is watched. Devices play it in a lighter mode instead.'
  }
  const on = c.measured ? `, on its ${c.measured.by}` : ''
  return `A phone that can’t play a 4K film gets it converted to 1080p as it watches${on}.`
}

/**
 * How many devices at once, what is being converted now, and measuring again.
 *
 * Under the switch, while converting is on. The number is measured, by
 * converting a demanding 4K sample one, two, three at a time until the
 * machine no longer keeps up, and can be set by hand: lower to keep the
 * machine free for other things, higher on the owner's own say-so.
 */
export function ConversionPanel({
  conversion,
  onAtOnce,
  onMeasure,
}: {
  conversion: ConversionStatus
  onAtOnce: (atOnce: number | null) => void
  onMeasure: () => void
}): React.JSX.Element | null {
  if (!conversion.enabled || !conversion.available) return null
  const measured = conversion.measured
  const auto = conversion.byHand === null

  return (
    <div className="mt-3.5 overflow-hidden rounded-xl border border-line">
      <div className="flex items-center gap-4 px-4 py-3.5">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-white/[0.06]">
          {conversion.measuring ? (
            <Loader2 size={17} className="animate-spin text-textDim" />
          ) : (
            <Cpu size={17} strokeWidth={1.8} className="text-textDim" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-1.5">
            <span className="tnum text-[20px] font-semibold leading-none text-text">{conversion.limit}</span>
            <span className="text-[12px] text-textDim">
              {conversion.limit === 1 ? 'device at once' : 'devices at once'}
            </span>
          </div>
          <p className="mt-1 truncate text-[11px] text-textFaint">
            {conversion.measuring
              ? 'Measuring now: converting a 4K sample, one at a time, then more.'
              : measured
                ? `Measured: ${measured.atOnce} at once, one at ${measured.speed}× real time, on ${measured.by}.`
                : 'Not measured yet.'}
          </p>
        </div>
        <button
          onClick={onMeasure}
          disabled={conversion.measuring}
          className="flex shrink-0 items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[11px] text-textDim transition-colors hover:bg-panel2 hover:text-text disabled:opacity-50"
        >
          <RefreshCw size={11} className={conversion.measuring ? 'animate-spin' : ''} />
          {conversion.measuring ? 'Measuring…' : 'Measure again'}
        </button>
      </div>

      <div className="border-t border-line px-4 py-3">
        <div className="text-[11px] text-textFaint">At once</div>
        <div className="mt-2 flex flex-wrap gap-1.5">
          <Choice active={auto} onClick={() => onAtOnce(null)}>
            Measured{measured ? ` (${measured.atOnce})` : ''}
          </Choice>
          {BY_HAND.map((n) => (
            <Choice key={n} active={conversion.byHand === n} onClick={() => onAtOnce(n)}>
              {n}
            </Choice>
          ))}
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-textFaint">
          {auto
            ? 'As many as this computer was measured to keep up with.'
            : measured && conversion.byHand !== null && conversion.byHand > measured.atOnce
              ? 'More than it was measured to keep up with: pictures may stutter when that many watch at once.'
              : 'Chosen by hand. Fewer leaves this computer freer for other things.'}
        </p>
      </div>

      <div className="border-t border-line px-4 py-3">
        <div className="text-[11px] text-textFaint">Converting now</div>
        {conversion.active.length === 0 ? (
          <p className="mt-1.5 text-[12px] text-textDim">Nothing at the moment.</p>
        ) : (
          <ul className="mt-1.5 space-y-1.5">
            {conversion.active.map((a, i) => (
              <motion.li
                key={`${a.device}-${a.file}-${a.since}-${i}`}
                initial={{ opacity: 0, y: 3 }}
                animate={{ opacity: 1, y: 0 }}
                className="flex items-center gap-2.5 text-[12px]"
              >
                <Smartphone size={13} className="shrink-0 text-textFaint" />
                <span className="shrink-0 text-text">{a.device || 'A device'}</span>
                <span className="min-w-0 truncate text-textDim">{a.file.split('/').pop()}</span>
                <span className="tnum ml-auto shrink-0 font-mono text-[10.5px] text-textFaint">
                  {since(a.since)}
                </span>
              </motion.li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

function Choice({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        'tnum rounded-md border px-2.5 py-1 text-[11.5px] transition-colors duration-150',
        active
          ? 'border-white/25 bg-white/[0.1] text-text'
          : 'border-line text-textDim hover:border-white/[0.14] hover:text-text',
      )}
    >
      {children}
    </button>
  )
}

/** How long ago, briefly: "12 min", "1 h 5 min". */
function since(unixSeconds: number): string {
  const minutes = Math.max(0, Math.floor((Date.now() / 1000 - unixSeconds) / 60))
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes} min`
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`
}
