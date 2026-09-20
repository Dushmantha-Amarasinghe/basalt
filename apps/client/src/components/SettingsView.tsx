import { useEffect, useState } from 'react'
import { motion } from 'framer-motion'
import { HardDrive, Info, Laptop, Shield, Volume2, Wifi, Zap } from 'lucide-react'
import type { Status } from '@/lib/api'
import {
  audioDevices,
  savedAudioDevice,
  setAudioDevice,
  type AudioDevice,
} from '@/lib/useMpv'
import { About } from './About'
import { Dropdown } from './ui/Dropdown'
import { cn, formatBytes } from '@/lib/utils'

/**
 * Settings.
 *
 * Every value here is read from the live connection. Where a setting is not
 * adjustable it is shown as a fact rather than as a switch that does nothing —
 * compression, batching and verification are always on because the
 * measurements left no case for turning them off, and a toggle implying
 * otherwise would be a lie about the software.
 */
/**
 * Which output mpv plays through.
 *
 * Read from mpv rather than from Windows: it is mpv that has to open the
 * device, and its list is the one that can actually be selected.
 */
function AudioOutput(): React.JSX.Element {
  const [devices, setDevices] = useState<AudioDevice[]>([])
  const [chosen, setChosen] = useState(savedAudioDevice)

  useEffect(() => {
    void audioDevices().then(setDevices)
  }, [])

  return (
    <div className="flex items-center justify-between gap-4 px-4 py-2.5">
      <span className="shrink-0 text-[12.5px] text-textDim">Audio output</span>
      <Dropdown
        label="Audio output"
        className="w-[62%] max-w-[420px]"
        value={chosen}
        onChange={(next) => {
          setChosen(next)
          void setAudioDevice(next)
        }}
        options={[
          { value: 'auto', label: 'Automatic' },
          ...devices
            .filter((device) => device.name !== 'auto')
            .map((device) => ({ value: device.name, label: device.description })),
        ]}
      />
    </div>
  )
}

export function SettingsView({
  status,
  space,
  onForget,
  onRepair,
}: {
  status: Status | null
  space: [number, number] | null
  onForget: () => void
  onRepair: () => void
}): React.JSX.Element {
  const [free, total] = space ?? [0, 0]

  return (
    <div className="h-full overflow-y-auto px-8 py-6">
      <div className="mx-auto max-w-[640px] space-y-6">
        <Section icon={HardDrive} title="Vault" hint="The drive this app connects to">
          <Row label="Name" value={status?.vault ?? '—'} />
          <Row label="Host" value={status?.hostName ?? '—'} />
          <Row label="Address" value={status?.address ?? 'not connected'} mono />
          <Row
            label="Space"
            value={total > 0 ? `${formatBytes(free)} free of ${formatBytes(total)}` : '—'}
            mono
          />
          <Row label="Access" value={status?.writable ? 'read and write' : 'read only'} />
          <Note>
            The address is where the host answered today, not something this app
            remembers and depends on. When the router gives it a different one,
            this app finds it again by its pinned identity — which is why you
            were never asked to type one.
          </Note>
          <Action label="Forget this vault" danger onClick={onForget} />
        </Section>

        <Section icon={Info} title="About" hint="Basalt, by Refora Technologies">
          <About product="Basalt" />
        </Section>

        <Section icon={Volume2} title="Playback" hint="Where the sound goes">
          <AudioOutput />
          <Note>
            {/* Worth saying, because the list is mpv's and not Windows's, and
                the names differ enough to be confusing. */}
            Chosen for this app only — it does not change what anything else on
            this machine plays through. <span className="text-textDim">Automatic</span>{' '}
            follows whatever Windows is using at the time.
          </Note>
        </Section>

        <Section icon={Wifi} title="Connection" hint="How this link behaves">
          <Row label="Transport" value="TCP · single connection" mono />
          <Row label="Encryption" value="TLS 1.3 · always on" mono />
          <Row label="Receive buffer" value="2 MiB" mono />
          <Note>
            Both machines are on Wi-Fi, so every byte crosses the air twice —
            once to the router and once back out. A cable to the host would
            roughly double throughput, which is more than any software change
            can offer.
          </Note>
        </Section>

        <Section icon={Zap} title="Transfers" hint="Measured on this hardware">
          <Row label="Compression" value="zstd level 1 · 2.2x on documents" mono />
          <Row label="Small files" value="batched · 7.6x faster" mono />
          <Row label="Verification" value="BLAKE3, every transfer" mono />
          <Row label="Chunk size" value="4 MiB" mono />
          <Note>
            None of these are switches. Compression is skipped automatically for
            anything already compressed, level 9 was measured slower than the
            link itself, and an unverified transfer has no advantage worth
            having.
          </Note>
        </Section>

        <Section icon={Laptop} title="This device" hint="How the host sees you">
          <Row label="Name" value={status?.deviceName ?? '—'} />
          <Note>
            Shown in the host&rsquo;s device list, where this device can be
            revoked at any time.
          </Note>
        </Section>

        <Section icon={Shield} title="Security">
          <Row
            label="Host identity"
            value={status?.hostId ? formatIdentity(status.hostId) : '—'}
            mono
          />
          <Note>
            The host&rsquo;s public key, pinned when you paired. Every connection
            since has had to present exactly this key — a different machine at
            the same address is refused rather than trusted.
          </Note>
          <Action label="Pair with a different vault" onClick={onRepair} />
        </Section>
      </div>
    </div>
  )
}

/** Groups a long hex identity so a person can compare it against a screen. */
function formatIdentity(hostId: string): string {
  return (hostId.match(/.{1,4}/g) ?? [hostId]).slice(0, 4).join(' ') + ' …'
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

function Action({
  label,
  danger,
  onClick,
}: {
  label: string
  danger?: boolean
  onClick?: () => void
}): React.JSX.Element {
  return (
    <div className="px-4 py-2.5">
      <button
        onClick={onClick}
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
