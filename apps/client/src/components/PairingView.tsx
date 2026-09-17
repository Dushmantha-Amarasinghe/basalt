import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  ArrowRight,
  HardDrive,
  Loader2,
  RefreshCw,
  ShieldCheck,
} from 'lucide-react'
import { HexMark } from './HexMark'
import { ApiError, api, type DiscoveredHost, type Status } from '@/lib/api'
import { cn } from '@/lib/utils'

const PIN_LENGTH = 6

/**
 * First contact.
 *
 * **There is no address to type.** The client asks the network which Basalt
 * hosts are there and lists them by name; picking one is the whole of step one.
 * That is the point of the discovery work — an address is something a router
 * changes without telling anyone, and having to go and look it up again is the
 * frustration this app exists to remove.
 *
 * Step two is the PIN, and only when the host is asking for one. The number is
 * on the host's screen, next to the name of this device — so typing it proves
 * the person can see that machine.
 *
 * The identity is shown at both steps on purpose. It is the value that gets
 * pinned, and after this it is never asked about again, so this is the only
 * moment anyone could notice it being wrong.
 */
export function PairingView({
  onPaired,
}: {
  onPaired: (status: Status) => void
}): React.JSX.Element {
  const [hosts, setHosts] = useState<DiscoveredHost[] | null>(null)
  const [scanning, setScanning] = useState(false)
  const [chosen, setChosen] = useState<DiscoveredHost | null>(null)
  const [needsPin, setNeedsPin] = useState(false)
  const [pin, setPin] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Tracks whether this component is still mounted, so a scan that finishes
  // after the user has already paired does not write into state that is gone.
  const live = useRef(true)
  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
    }
  }, [])

  const scan = useCallback(async () => {
    setScanning(true)
    setError(null)
    try {
      const found = await api.discover()
      // The previous list stays on screen until the new one arrives. Clearing
      // it first would make every rescan flash the empty state.
      if (live.current) setHosts(found)
    } catch (e) {
      if (live.current) setError(e instanceof Error ? e.message : String(e))
    } finally {
      if (live.current) setScanning(false)
    }
  }, [])

  useEffect(() => {
    void scan()
  }, [scan])

  /** Picking a host opens a request on it, and asks whether it wants a PIN. */
  const choose = useCallback(
    async (host: DiscoveredHost) => {
      if (busy) return
      setBusy(true)
      setError(null)
      try {
        const wantsPin = await api.beginPairing(host.address)
        if (!live.current) return
        setChosen(host)
        setNeedsPin(wantsPin)
        setPin('')

        // Nothing left to ask. Finish straight away rather than showing an
        // empty PIN screen with a button that only says "continue".
        if (!wantsPin) {
          onPaired(await api.finishPairing(''))
        }
      } catch (e) {
        if (!live.current) return
        setChosen(null)
        setError(
          e instanceof ApiError && e.kind === 'offline'
            ? `${host.hostName} stopped answering. It may have gone to sleep.`
            : e instanceof Error
              ? e.message
              : String(e),
        )
      } finally {
        if (live.current) setBusy(false)
      }
    },
    [busy, onPaired],
  )

  const submitPin = useCallback(
    async (value: string) => {
      if (busy) return
      setBusy(true)
      setError(null)
      try {
        onPaired(await api.finishPairing(value))
      } catch (e) {
        if (!live.current) return
        setPin('')
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (live.current) setBusy(false)
      }
    },
    [busy, onPaired],
  )

  const back = useCallback(() => {
    // Tell the host to take the card down rather than leaving this device's
    // name on its screen for three minutes after a change of mind.
    void api.cancelPairing().catch(() => {})
    setChosen(null)
    setNeedsPin(false)
    setPin('')
    setError(null)
  }, [])

  const picking = !chosen

  return (
    <div className="relative flex h-full flex-col items-center justify-center px-8">
      <div className="backdrop" />

      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
        className="relative z-10 w-full max-w-[400px]"
      >
        <div className="mb-7 flex flex-col items-center text-center">
          <motion.span
            animate={{ opacity: [0.55, 1, 0.55] }}
            transition={{ duration: 4, repeat: Infinity, ease: 'easeInOut' }}
            className="text-basalt"
          >
            <HexMark size={34} />
          </motion.span>
          <h1 className="mt-4 font-display text-[19px] font-semibold tracking-tighter text-text">
            {picking ? 'Choose your vault' : 'Enter the PIN'}
          </h1>
          <p className="mt-1.5 max-w-[320px] text-[12px] leading-relaxed text-textDim">
            {picking
              ? 'Every Basalt host on this network. Nothing to type.'
              : `The six digits showing on ${chosen.hostName}. This happens once.`}
          </p>
        </div>

        {picking ? (
          <HostList
            hosts={hosts}
            scanning={scanning}
            busy={busy}
            onChoose={(host) => void choose(host)}
            onRescan={() => void scan()}
          />
        ) : (
          <motion.div
            initial={{ opacity: 0, x: 12 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
          >
            {needsPin ? (
              <PinInput value={pin} onChange={setPin} onComplete={submitPin} busy={busy} />
            ) : (
              <div className="flex items-center justify-center gap-2 py-4 text-[12px] text-textDim">
                <Loader2 size={13} className="animate-spin" />
                Connecting…
              </div>
            )}

            <div className="mt-6 rounded-lg border border-white/[0.07] bg-panel/70 p-3.5">
              <div className="flex items-center gap-2.5">
                <ShieldCheck size={14} className="shrink-0 text-basaltDeep" />
                <span className="truncate text-[12px] font-medium text-text">
                  {chosen.hostName}
                </span>
                <span className="ml-auto shrink-0 font-mono text-[10px] text-textFaint">
                  {chosen.hostId.slice(0, 8)}
                </span>
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-textDim">
                Sharing <span className="text-text">{chosen.vault}</span>. Check that
                identity matches the one on the host before continuing — after this it
                is trusted permanently and never asked about again.
              </p>
            </div>

            <button
              onClick={back}
              className="mt-3 w-full rounded-md py-1.5 text-[11px] text-textFaint transition-colors hover:text-textDim"
            >
              Choose a different host
            </button>
          </motion.div>
        )}

        <AnimatePresence>
          {error && (
            <motion.p
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              className="mt-4 text-center text-[11px] leading-relaxed text-danger"
            >
              {error}
            </motion.p>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  )
}

/**
 * The hosts on this network.
 *
 * The empty state is the one that matters: somebody staring at it has a host
 * they believe is running, so it says what to check rather than only that
 * nothing was found.
 */
function HostList({
  hosts,
  scanning,
  busy,
  onChoose,
  onRescan,
}: {
  hosts: DiscoveredHost[] | null
  scanning: boolean
  busy: boolean
  onChoose: (host: DiscoveredHost) => void
  onRescan: () => void
}): React.JSX.Element {
  if (hosts === null) {
    return (
      <div className="flex flex-col items-center gap-3 py-8">
        <Loader2 size={16} className="animate-spin text-textFaint" />
        <span className="text-[11.5px] text-textFaint">Looking for hosts…</span>
      </div>
    )
  }

  return (
    <div>
      <AnimatePresence initial={false}>
        {hosts.map((host) => (
          <motion.div
            key={host.hostId}
            layout
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
          >
            <HostRow host={host} busy={busy} onChoose={() => onChoose(host)} />
          </motion.div>
        ))}
      </AnimatePresence>

      {hosts.length === 0 && (
        <div className="rounded-lg border border-dashed border-white/[0.09] px-4 py-7 text-center">
          <p className="text-[12.5px] text-textDim">No hosts on this network.</p>
          <p className="mx-auto mt-2 max-w-[300px] text-[11px] leading-relaxed text-textFaint">
            Check Basalt Host is running on the other machine — look in its
            notification area, not just the taskbar — and that both machines are on
            the same Wi-Fi.
          </p>
        </div>
      )}

      <button
        onClick={onRescan}
        disabled={scanning}
        className="mt-3 flex w-full items-center justify-center gap-2 rounded-md py-2 text-[11px] text-textFaint transition-colors hover:text-textDim disabled:opacity-50"
      >
        <RefreshCw size={11} className={cn(scanning && 'animate-spin')} />
        {scanning ? 'Looking…' : 'Look again'}
      </button>
    </div>
  )
}

function HostRow({
  host,
  busy,
  onChoose,
}: {
  host: DiscoveredHost
  busy: boolean
  onChoose: () => void
}): React.JSX.Element {
  // A host with no drive chosen yet has nothing to offer. Listed anyway,
  // because seeing the machine and being told why it is unavailable beats an
  // empty list and no explanation.
  const ready = host.hasVault
  const disabled = busy || !ready

  return (
    <motion.button
      whileHover={disabled ? undefined : { y: -1 }}
      whileTap={disabled ? undefined : { scale: 0.99 }}
      transition={{ type: 'spring', stiffness: 500, damping: 30 }}
      onClick={disabled ? undefined : onChoose}
      disabled={disabled}
      className={cn(
        'mb-2 flex w-full items-center gap-3 rounded-lg border border-white/[0.07] bg-panel2 px-3.5 py-3 text-left transition-colors',
        disabled ? 'cursor-not-allowed opacity-45' : 'hover:border-white/[0.16]',
      )}
    >
      <span className="shrink-0 text-textFaint">
        <HardDrive size={15} />
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-[13px] font-medium text-text">
            {ready ? host.vault : host.hostName}
          </span>
          {host.paired && (
            <span className="shrink-0 rounded-[4px] border border-white/[0.12] px-1.5 py-[1px] font-mono text-[9px] uppercase tracking-[0.1em] text-textFaint">
              paired
            </span>
          )}
        </div>
        <div className="mt-0.5 flex items-center gap-2 font-mono text-[10.5px] text-textFaint">
          <span className="truncate">{ready ? host.hostName : 'no drive shared yet'}</span>
          <span className="shrink-0">·</span>
          <span className="shrink-0">{host.hostId.slice(0, 8)}</span>
        </div>
      </div>

      {ready && (
        <span className="shrink-0 text-textFaint">
          {host.requiresPin ? (
            <span
              title="This host asks for a PIN"
              className="font-mono text-[9px] uppercase tracking-[0.1em]"
            >
              pin
            </span>
          ) : (
            <ArrowRight size={14} />
          )}
        </span>
      )}
    </motion.button>
  )
}

/**
 * Six boxes that behave like one field.
 *
 * A single text input would be simpler, but a PIN is read off another screen
 * one digit at a time and separated boxes make it obvious where you are. One
 * hidden input does the actual typing so paste, backspace and mobile keyboards
 * all keep working; the boxes are decoration over it.
 */
function PinInput({
  value,
  onChange,
  onComplete,
  busy,
}: {
  value: string
  onChange: (value: string) => void
  onComplete: (value: string) => void
  busy: boolean
}): React.JSX.Element {
  const inputRef = useRef<HTMLInputElement>(null)
  const [focused, setFocused] = useState(false)

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  return (
    <div
      className="relative flex justify-center gap-2"
      onClick={() => inputRef.current?.focus()}
    >
      <input
        ref={inputRef}
        value={value}
        inputMode="numeric"
        autoComplete="one-time-code"
        disabled={busy}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onChange={(e) => {
          const digits = e.target.value.replace(/\D/g, '').slice(0, PIN_LENGTH)
          onChange(digits)
          if (digits.length === PIN_LENGTH) onComplete(digits)
        }}
        className="absolute inset-0 z-10 h-full w-full cursor-default opacity-0"
        aria-label="Pairing PIN"
      />

      {Array.from({ length: PIN_LENGTH }, (_, i) => {
        const filled = i < value.length
        const active = focused && i === value.length && !busy
        return (
          <div
            key={i}
            className={cn(
              'tnum flex h-12 w-11 items-center justify-center rounded-lg border font-mono text-[17px] transition-colors',
              filled
                ? 'border-white/20 bg-panel2 text-text'
                : 'border-white/[0.08] bg-ink2 text-textFaint',
              active && 'border-white/35',
              busy && 'opacity-50',
            )}
          >
            {filled ? (
              <motion.span
                initial={{ opacity: 0, scale: 0.7 }}
                animate={{ opacity: 1, scale: 1 }}
                transition={{ type: 'spring', stiffness: 600, damping: 30 }}
              >
                {value[i]}
              </motion.span>
            ) : active ? (
              <motion.span
                animate={{ opacity: [1, 0.15, 1] }}
                transition={{ duration: 1.1, repeat: Infinity, ease: 'easeInOut' }}
                className="h-4 w-px bg-basalt"
              />
            ) : null}
          </div>
        )
      })}
    </div>
  )
}
