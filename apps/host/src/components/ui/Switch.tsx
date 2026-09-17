import { motion } from 'framer-motion'
import { cn } from '@/lib/utils'

/**
 * A setting that is on or off.
 *
 * The knob is animated with a spring rather than a CSS transition so an
 * interrupted toggle — two clicks in quick succession — carries its velocity
 * instead of snapping back to the start.
 */
export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean
  onChange: (next: boolean) => void
  disabled?: boolean
  label: string
}): React.JSX.Element {
  return (
    <button
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative h-[22px] w-[38px] shrink-0 rounded-full border transition-colors',
        checked ? 'border-lineBright bg-basalt/90' : 'border-line bg-panel2',
        disabled ? 'cursor-not-allowed opacity-40' : 'cursor-pointer',
      )}
    >
      <motion.span
        layout
        transition={{ type: 'spring', stiffness: 520, damping: 34 }}
        className={cn(
          'absolute top-[2px] h-[16px] w-[16px] rounded-full',
          checked ? 'left-[19px] bg-ink' : 'left-[2px] bg-textDim',
        )}
      />
    </button>
  )
}
