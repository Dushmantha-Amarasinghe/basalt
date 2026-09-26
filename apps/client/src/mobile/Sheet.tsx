import { useRef } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion, type PanInfo } from 'framer-motion'
import type { MenuAction } from '@/components/ui/ContextMenu'
import { cn } from '@/lib/utils'
import { useBack } from './useBack'

/**
 * A panel that rises from the bottom edge: the phone's menu, and its dialog.
 *
 * Dragged down, or tapped outside, or backed out of, it goes. It sits above
 * the gesture bar and never under it.
 */
export function Sheet({
  open,
  onClose,
  title,
  children,
  tall,
}: {
  open: boolean
  onClose: () => void
  title?: React.ReactNode
  children: React.ReactNode
  /** Room for a long list: most of the screen rather than what it needs. */
  tall?: boolean
}): React.JSX.Element {
  useBack(open, () => {
    onClose()
    return true
  })
  const panel = useRef<HTMLDivElement | null>(null)

  const onDragEnd = (_: unknown, info: PanInfo): void => {
    if (info.offset.y > 90 || info.velocity.y > 600) onClose()
  }

  return createPortal(
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-[80]">
          <motion.div
            className="absolute inset-0 bg-black/55"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            onClick={onClose}
          />
          <motion.div
            ref={panel}
            role="dialog"
            className={cn(
              'absolute inset-x-0 bottom-0 mx-auto flex max-w-[640px] flex-col rounded-t-[22px] border-t border-white/[0.08] bg-[#141416] shadow-lift',
              tall ? 'max-h-[88%]' : 'max-h-[80%]',
            )}
            style={{ paddingBottom: 'calc(var(--inset-bottom, 0px) + 10px)' }}
            initial={{ y: '100%' }}
            animate={{ y: 0 }}
            exit={{ y: '100%' }}
            transition={{ type: 'spring', stiffness: 520, damping: 44 }}
            drag="y"
            dragConstraints={{ top: 0, bottom: 0 }}
            dragElastic={{ top: 0, bottom: 0.6 }}
            onDragEnd={onDragEnd}
          >
            <div className="flex shrink-0 justify-center pb-1 pt-2.5">
              <span className="h-1 w-10 rounded-full bg-white/20" />
            </div>
            {title && (
              <div className="shrink-0 px-5 pb-2 pt-1 text-[15px] font-semibold text-text">
                {title}
              </div>
            )}
            <div className="min-h-0 overflow-y-auto overscroll-contain">{children}</div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  )
}

/** The actions a file or folder offers, as a list in a sheet. */
export function ActionSheet({
  open,
  onClose,
  title,
  subtitle,
  actions,
}: {
  open: boolean
  onClose: () => void
  title?: string
  subtitle?: string
  actions: MenuAction[]
}): React.JSX.Element {
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={
        title ? (
          <div className="min-w-0">
            <div className="truncate">{title}</div>
            {subtitle && (
              <div className="mt-0.5 truncate font-mono text-[11px] font-normal text-textFaint">
                {subtitle}
              </div>
            )}
          </div>
        ) : undefined
      }
    >
      <div className="pb-1">
        {actions.map((action) => {
          const Icon = action.icon
          return (
            <div key={action.id}>
              {action.separatorBefore && <div className="mx-5 my-1 h-px bg-white/[0.07]" />}
              <button
                disabled={action.disabled}
                onClick={() => {
                  onClose()
                  void action.run()
                }}
                className={cn(
                  'flex h-[52px] w-full items-center gap-4 px-5 text-left text-[15px] transition-colors active:bg-white/[0.06] disabled:opacity-35',
                  action.danger ? 'text-danger' : 'text-text',
                )}
              >
                {Icon && <Icon size={20} className={action.danger ? 'text-danger' : 'text-textDim'} />}
                <span className="truncate">{action.label.replace(/…$/, '')}</span>
              </button>
            </div>
          )
        })}
      </div>
    </Sheet>
  )
}
