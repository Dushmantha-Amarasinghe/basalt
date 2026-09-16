import { useState } from 'react'
import { motion } from 'framer-motion'
import { Check, HardDrive, Laptop, Shield, Wifi, Zap } from 'lucide-react'
import { cn } from '@/lib/utils'

/**
 * Settings.
 *
 * The numbers shown here are the ones Phase 0 actually measured on this
 * hardware, not invented defaults — 22.7 MB/s, 2.3 ms, zstd level 1, a 2 MiB
 * receive buffer. Seeing real values in the mock keeps the design honest about
 * how much room these rows need.
 */
export function SettingsView(): React.JSX.Element {
  return (
    <div className="h-full overflow-y-auto px-8 py-6">
      <div className="mx-auto max-w-[640px] space-y-6">
        <Section icon={HardDrive} title="Vault" hint="The drive this app connects to">
          <Row label="Name" value="Vault" />
          <Row label="Host" value="192.168.1.10" mono />
          {/* A JSX string literal, so the backslash is not an escape. */}
          <Row label="Drive" value={'D:\\ · 3.6 TB'} mono />
          <Row label="Paired" value="16 Sept 2026" />
          <Action label="Forget this vault" danger />
        </Section>

        <Section icon={Wifi} title="Connection" hint="Measured on this link">
          <Row label="Throughput" value="22.7 MB/s" mono />
          <Row label="Round trip" value="2.3 ms" mono />
          <Row label="Encryption" value="TLS 1.3 · always on" mono />
          <Note>
            Both machines are on Wi-Fi, so every byte crosses the air twice. A
            cable to the vault would roughly double this.
          </Note>
        </Section>

        <Section icon={Zap} title="Transfers" hint="How data moves">
          <Toggle label="Compress before sending" defaultOn hint="2.2x measured on documents" />
          <Toggle label="Batch small files" defaultOn hint="7.6x faster than one at a time" />
          <Toggle label="Verify after transfer" defaultOn hint="BLAKE3 checksum" />
          <Select
            label="Compression level"
            value="1 — fastest"
            options={['1 — fastest', '3 — balanced', '6 — smaller']}
          />
          <Row label="Receive buffer" value="2 MiB" mono />
        </Section>

        <Section icon={Laptop} title="This device" hint="Local behaviour">
          <Toggle label="Start with Windows" />
          <Toggle label="Keep folders available offline" hint="Uses local disk space" />
          <Select label="Cache size" value="20 GB" options={['5 GB', '20 GB', '50 GB', 'Unlimited']} />
          <Action label="Clear cache" />
        </Section>

        <Section icon={Shield} title="Security">
          <Row label="Device key" value="SHA256:a41f…9c2e" mono />
          <Toggle label="Require confirmation before deleting" defaultOn />
          <Action label="Re-pair with vault" />
        </Section>
      </div>
    </div>
  )
}

function Section({
  icon: Icon,
  title,
  hint,
  children,
}: {
  icon: typeof HardDrive
  title: string
  hint?: string
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
      className="glass overflow-hidden rounded-lg"
    >
      <div className="flex items-center gap-2.5 border-b border-line px-4 py-3">
        <Icon size={15} className="text-textDim" />
        <span className="text-sm font-semibold tracking-tight text-text">{title}</span>
        {hint && <span className="ml-auto text-[11px] text-textFaint">{hint}</span>}
      </div>
      <div className="divide-y divide-white/[0.04]">{children}</div>
    </motion.section>
  )
}

function Row({
  label,
  value,
  mono,
}: {
  label: string
  value: string
  mono?: boolean
}): React.JSX.Element {
  return (
    <div className="flex items-center justify-between px-4 py-2.5">
      <span className="text-[13px] text-textDim">{label}</span>
      <span className={cn('text-[13px] text-text', mono && 'font-mono text-[12px]')}>
        {value}
      </span>
    </div>
  )
}

function Toggle({
  label,
  hint,
  defaultOn,
}: {
  label: string
  hint?: string
  defaultOn?: boolean
}): React.JSX.Element {
  const [on, setOn] = useState(Boolean(defaultOn))
  return (
    <div className="flex items-center justify-between gap-4 px-4 py-2.5">
      <div className="min-w-0">
        <div className="text-[13px] text-text">{label}</div>
        {hint && <div className="mt-0.5 text-[11px] text-textFaint">{hint}</div>}
      </div>
      <button
        onClick={() => setOn((v) => !v)}
        role="switch"
        aria-checked={on}
        aria-label={label}
        className={cn(
          'relative flex h-5 w-9 shrink-0 items-center rounded-full border px-0.5 transition-colors duration-150',
          on ? 'border-white/25 bg-white/20' : 'border-white/10 bg-white/5',
        )}
      >
        <motion.span
          initial={false}
          animate={{ x: on ? 16 : 0 }}
          transition={{ type: 'spring', stiffness: 700, damping: 38 }}
          className="h-3.5 w-3.5 rounded-full"
          style={{ backgroundColor: on ? '#ffffff' : '#6b6b72' }}
        />
      </button>
    </div>
  )
}

function Select({
  label,
  value,
  options,
}: {
  label: string
  value: string
  options: string[]
}): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const [current, setCurrent] = useState(value)

  return (
    <div className="relative flex items-center justify-between px-4 py-2.5">
      <span className="text-[13px] text-textDim">{label}</span>
      <button
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'rounded-md border bg-panel2 px-3 py-1.5 text-[12px] text-text transition-colors',
          open ? 'border-white/20' : 'border-white/10 hover:border-white/20',
        )}
      >
        {current}
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <motion.div
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            transition={{ duration: 0.1, ease: 'easeOut' }}
            className="absolute right-4 top-full z-20 mt-1 w-[180px] overflow-hidden rounded-md border border-white/10 bg-panel2 shadow-lift"
          >
            {options.map((option) => (
              <button
                key={option}
                onClick={() => {
                  setCurrent(option)
                  setOpen(false)
                }}
                className={cn(
                  'flex w-full items-center justify-between px-3 py-2 text-left text-[12px] transition-colors hover:bg-white/[0.06]',
                  option === current ? 'bg-white/5 text-text' : 'text-textDim',
                )}
              >
                {option}
                {option === current && <Check size={12} className="text-textDim" />}
              </button>
            ))}
          </motion.div>
        </>
      )}
    </div>
  )
}

function Action({ label, danger }: { label: string; danger?: boolean }): React.JSX.Element {
  return (
    <div className="px-4 py-2.5">
      <button
        className={cn(
          'rounded-md border px-3 py-1.5 text-[12px] transition-colors',
          danger
            ? 'border-danger/25 bg-dangerBg text-danger hover:border-danger/50'
            : 'border-white/10 text-textDim hover:border-white/20 hover:text-text',
        )}
      >
        {label}
      </button>
    </div>
  )
}

function Note({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="px-4 py-2.5 text-[11px] leading-relaxed text-textFaint">{children}</div>
  )
}
