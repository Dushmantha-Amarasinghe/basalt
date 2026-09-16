import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { ArrowRight, Loader2, ShieldCheck } from 'lucide-react'
import { HexMark } from './HexMark'
import { ApiError, api, type HostSummary, type Status } from '@/lib/api'
import { cn } from '@/lib/utils'

const PIN_LENGTH = 6

/**
 * First contact.
 *
 * Two steps, because they ask for two different things and merging them would
 * mean typing a PIN at a host that might not be there. Step one finds the host
 * and shows what it is; step two proves the user can see its screen.
 *
 * The identity is shown at both steps on purpose. It is the value that gets
 * pinned, and after this it is never asked about again — so this is the only
 * moment anyone could notice it being wrong.
 */
export function PairingView({
  onPaired,
}: {
  onPaired: (status: Status) => void
}): React.JSX.Element {
  const [address, setAddress] = useState('')
  const [host, setHost] = useState<HostSummary | null>(null)
  const [pin, setPin] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const find = useCallback(async () => {
    const target = address.trim()
    if (!target || busy) return
    setBusy(true)
    setError(null)
    try {
      setHost(await api.probe(target))
    } catch (e) {
      setHost(null)
      setError(
        e instanceof ApiError && e.kind === 'offline'
          ? `Nothing answered at ${target}. Check the address and that the host is running.`
          : e instanceof Error
            ? e.message
            : String(e),
      )
    } finally {
      setBusy(false)
    }
  }, [address, busy])

  const submitPin = useCallback(
    async (value: string) => {
      if (busy) return
      setBusy(true)
      setError(null)
      try {
        onPaired(await api.pair(address.trim(), value))
      } catch (e) {
        setPin('')
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        setBusy(false)
      }
    },
    [address, busy, onPaired],
  )

  return (
    <div className="relative flex h-full flex-col items-center justify-center px-8">
      <div className="backdrop" />

      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
        className="relative z-10 w-full max-w-[380px]"
      >
        <div className="mb-8 flex flex-col items-center text-center">
          <motion.span
            animate={{ opacity: [0.55, 1, 0.55] }}
            transition={{ duration: 4, repeat: Infinity, ease: 'easeInOut' }}
            className="text-basalt"
          >
            <HexMark size={34} />
          </motion.span>
          <h1 className="mt-4 font-display text-[19px] font-semibold tracking-tighter text-text">
            {host ? 'Enter the PIN' : 'Find your vault'}
          </h1>
          <p className="mt-1.5 max-w-[300px] text-[12px] leading-relaxed text-textDim">
            {host
              ? 'The six digits showing on the host. This happens once.'
              : 'The address of the machine holding the drive, shown when you start Basalt Host.'}
          </p>
        </div>

        <AnimatePresence mode="wait" initial={false}>
          {!host ? (
            <motion.div
              key="address"
              initial={{ opacity: 0, x: -12 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -12 }}
              transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            >
              <div className="flex gap-2">
                <input
                  autoFocus
                  value={address}
                  onChange={(e) => setAddress(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && void find()}
                  placeholder="192.168.1.11"
                  spellCheck={false}
                  className="h-10 min-w-0 flex-1 rounded-lg border border-white/[0.09] bg-panel2 px-3.5 font-mono text-[13px] text-text placeholder:text-textFaint transition-colors focus:border-white/25"
                />
                <motion.button
                  whileHover={{ y: -1 }}
                  whileTap={{ scale: 0.985 }}
                  transition={{ type: 'spring', stiffness: 500, damping: 30 }}
                  onClick={() => void find()}
                  disabled={busy || !address.trim()}
                  className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-white/[0.14] bg-panel2 text-text transition-colors hover:bg-white/[0.06] disabled:opacity-35"
                  aria-label="Find this host"
                >
                  {busy ? (
                    <Loader2 size={15} className="animate-spin" />
                  ) : (
                    <ArrowRight size={15} />
                  )}
                </motion.button>
              </div>
            </motion.div>
          ) : (
            <motion.div
              key="pin"
              initial={{ opacity: 0, x: 12 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 12 }}
              transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
            >
              <PinInput value={pin} onChange={setPin} onComplete={submitPin} busy={busy} />
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence>
          {host && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.25, ease: [0.4, 0, 0.2, 1] }}
              className="overflow-hidden"
            >
              <div className="mt-6 rounded-lg border border-white/[0.07] bg-panel/70 p-3.5">
                <div className="flex items-center gap-2.5">
                  <ShieldCheck size={14} className="shrink-0 text-basaltDeep" />
                  <span className="truncate text-[12px] font-medium text-text">
                    {host.hostName}
                  </span>
                  <span className="ml-auto shrink-0 font-mono text-[10px] text-textFaint">
                    {host.hostId.slice(0, 8)}
                  </span>
                </div>
                <p className="mt-2 text-[11px] leading-relaxed text-textDim">
                  Sharing <span className="text-text">{host.vault}</span>. Check that
                  identity matches the one on the host before continuing — after this
                  it is trusted permanently and never asked about again.
                </p>
                {!host.pairingOpen && (
                  <p className="mt-2 text-[11px] leading-relaxed text-danger">
                    This host is not accepting new devices. Press Enter on the host to
                    show a PIN, then try again.
                  </p>
                )}
              </div>

              <button
                onClick={() => {
                  setHost(null)
                  setPin('')
                  setError(null)
                }}
                className="mt-3 w-full rounded-md py-1.5 text-[11px] text-textFaint transition-colors hover:text-textDim"
              >
                Use a different address
              </button>
            </motion.div>
          )}
        </AnimatePresence>

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
