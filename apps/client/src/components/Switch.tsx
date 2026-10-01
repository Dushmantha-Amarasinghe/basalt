import { cn } from '@/lib/utils'

/**
 * An on/off setting: a label, a line saying what it does, and the switch.
 *
 * One button for all of it, so a tap anywhere on the row flips it — on a phone
 * the switch alone is a small target.
 */
export function Switch({
  label,
  description,
  checked,
  onChange,
  className,
}: {
  label: string
  description?: string
  checked: boolean
  onChange: (checked: boolean) => void
  className?: string
}): React.JSX.Element {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn('flex w-full items-center gap-4 rounded-md py-2 text-left', className)}
    >
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] text-text">{label}</span>
        {description && (
          <span className="mt-0.5 block text-[11.5px] leading-snug text-textFaint">{description}</span>
        )}
      </span>
      <span
        className={cn(
          'relative h-[22px] w-[38px] shrink-0 rounded-full transition-colors duration-150',
          checked ? 'bg-basalt' : 'bg-white/15',
        )}
      >
        <span
          className={cn(
            'absolute top-[3px] h-4 w-4 rounded-full transition-transform duration-150',
            checked ? 'translate-x-[19px] bg-ink' : 'translate-x-[3px] bg-white/80',
          )}
        />
      </span>
    </button>
  )
}
