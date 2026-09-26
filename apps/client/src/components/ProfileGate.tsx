import { useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { ArrowLeft, Check, Laptop, Loader2, Plus } from 'lucide-react'
import { api, ApiError, inTauri, type ProfileView } from '@/lib/api'
import { PROFILE_COLORS, profileColor, validPin } from '@/lib/useIdentity'
import { EASE_OUT } from '@/lib/motion'
import { cn } from '@/lib/utils'
import { HexMark } from './HexMark'

/**
 * Who is using Basalt: a profile, or this device on its own.
 *
 * Shown after connecting, until somebody chooses — or never, once a profile
 * is remembered or the device is set to carry on as itself.
 *
 * **Two paths, one look.** The device path is one click and changes nothing
 * about how Basalt has always worked: straight in, with a history kept on
 * this device. A profile is a name, a colour and a PIN, and in return its
 * history and stars follow it to every device in the house. The profile last
 * used here sits first, so signing back in after signing out is a tap and a
 * PIN.
 */

type Step =
  | { kind: 'choose' }
  | { kind: 'pin'; profile: ProfileView }
  | { kind: 'create' }

export function ProfileGate({
  vaultName,
  deviceName,
  profiles,
  lastProfile,
  ended,
  onDone,
}: {
  vaultName: string
  deviceName: string
  profiles: ProfileView[]
  lastProfile: string | null
  /** The host signed a profile out since the app last looked. */
  ended: boolean
  /** Signed in, or carrying on as the device. */
  onDone: () => void
}): React.JSX.Element {
  const [step, setStep] = useState<Step>(() => previewStep(profiles))
  const [always, setAlways] = useState(false)
  const [busy, setBusy] = useState(false)

  // The one last used here first; the rest as the host lists them.
  const ordered = useMemo(() => {
    const first = profiles.find((p) => p.id === lastProfile)
    return first ? [first, ...profiles.filter((p) => p.id !== lastProfile)] : profiles
  }, [profiles, lastProfile])

  const asDevice = async (): Promise<void> => {
    setBusy(true)
    try {
      await api.continueAsDevice(always)
      onDone()
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="relative flex h-full flex-col items-center justify-center overflow-y-auto px-8 py-10">
      <div className="backdrop" />
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={step.kind === 'pin' ? `pin-${step.profile.id}` : step.kind}
          initial={{ opacity: 0, y: 10, scale: 0.99 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: -8, scale: 0.99 }}
          transition={{ duration: 0.22, ease: EASE_OUT }}
          className="relative z-10 w-full max-w-[560px]"
        >
          {step.kind === 'choose' && (
            <>
              <div className="mb-8 flex flex-col items-center text-center">
                <HexMark size={30} className="text-basalt" />
                <h1 className="mt-4 font-display text-[22px] font-semibold tracking-tight text-text">
                  Who is using {vaultName}?
                </h1>
                <p className="mt-2 max-w-[400px] text-[12.5px] leading-relaxed text-textDim">
                  Choose your profile and your watch history and stars come with you, on any
                  device. Or carry on as this device, straight in.
                </p>
                {ended && (
                  <p className="mt-3 rounded-full bg-white/[0.05] px-3 py-1 text-[11.5px] text-textDim">
                    Your profile was signed out on the host.
                  </p>
                )}
              </div>

              <div className="flex flex-wrap justify-center gap-4">
                {ordered.map((profile, index) => (
                  <ProfileTile
                    key={profile.id}
                    index={index}
                    name={profile.name}
                    color={profile.color}
                    hint={profile.id === lastProfile ? 'Last used here' : undefined}
                    onClick={() => setStep({ kind: 'pin', profile })}
                  />
                ))}
                <motion.button
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.3, delay: Math.min(ordered.length, 8) * 0.04, ease: EASE_OUT }}
                  whileHover={{ y: -3 }}
                  whileTap={{ scale: 0.97 }}
                  onClick={() => setStep({ kind: 'create' })}
                  className="group flex w-[104px] flex-col items-center"
                >
                  <span className="flex h-[76px] w-[76px] items-center justify-center rounded-full border border-dashed border-white/20 text-textFaint transition-colors duration-200 group-hover:border-white/40 group-hover:text-text">
                    <Plus size={24} strokeWidth={1.6} />
                  </span>
                  <span className="mt-2.5 text-[12.5px] text-textDim transition-colors group-hover:text-text">
                    Add profile
                  </span>
                </motion.button>
              </div>

              <div className="my-8 flex items-center gap-3">
                <span className="h-px flex-1 bg-line" />
                <span className="font-mono text-[10px] uppercase tracking-[0.2em] text-textFaint">or</span>
                <span className="h-px flex-1 bg-line" />
              </div>

              <div className="mx-auto max-w-[440px]">
                <button
                  onClick={() => void asDevice()}
                  disabled={busy}
                  className="group flex w-full items-center gap-3.5 rounded-xl border border-white/[0.09] bg-panel/80 px-4 py-3.5 text-left transition-[background-color,border-color] duration-200 hover:border-white/20 hover:bg-panel2 disabled:opacity-60"
                >
                  <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-white/[0.06] text-textDim transition-colors group-hover:text-text">
                    {busy ? <Loader2 size={17} className="animate-spin" /> : <Laptop size={17} />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-[13px] font-medium text-text">
                      Continue as this device
                    </span>
                    <span className="mt-0.5 block truncate text-[11.5px] text-textFaint">
                      {deviceName} · history and stars stay on this device
                    </span>
                  </span>
                  <span className="shrink-0 text-textFaint transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-textDim">
                    →
                  </span>
                </button>
                <Checkbox
                  checked={always}
                  onChange={setAlways}
                  label="Always continue as this device, without asking"
                  className="mt-3 justify-center"
                />
              </div>
            </>
          )}

          {step.kind === 'pin' && (
            <PinStep
              profile={step.profile}
              onBack={() => setStep({ kind: 'choose' })}
              onDone={onDone}
            />
          )}

          {step.kind === 'create' && (
            <CreateStep
              taken={profiles.map((p) => p.name.toLowerCase())}
              onBack={() => setStep({ kind: 'choose' })}
              onDone={onDone}
            />
          )}
        </motion.div>
      </AnimatePresence>
    </div>
  )
}

function ProfileTile({
  index,
  name,
  color,
  hint,
  onClick,
}: {
  index: number
  name: string
  color: number
  hint?: string
  onClick: () => void
}): React.JSX.Element {
  return (
    <motion.button
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, delay: Math.min(index, 8) * 0.04, ease: EASE_OUT }}
      whileHover={{ y: -3 }}
      whileTap={{ scale: 0.97 }}
      onClick={onClick}
      className="group flex w-[104px] flex-col items-center outline-none"
    >
      <span className="rounded-full ring-2 ring-transparent ring-offset-4 ring-offset-ink transition-[box-shadow] duration-200 group-hover:ring-white/50 group-focus-visible:ring-white/70">
        <Avatar name={name} color={color} size={76} />
      </span>
      <span className="mt-2.5 max-w-full truncate text-[13px] font-medium text-textDim transition-colors group-hover:text-text">
        {name}
      </span>
      <span className="mt-0.5 h-3.5 font-mono text-[9.5px] text-textFaint">{hint ?? ''}</span>
    </motion.button>
  )
}

export function Avatar({
  name,
  color,
  size = 40,
}: {
  name: string
  color: number
  size?: number
}): React.JSX.Element {
  const tone = profileColor(color)
  return (
    <span
      aria-hidden
      className="flex shrink-0 select-none items-center justify-center rounded-full font-semibold text-white"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.4),
        background: `linear-gradient(145deg, ${tone}, ${tone}A8)`,
        boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.22), 0 0 0 1px rgba(255,255,255,0.06)',
      }}
    >
      {(name.trim()[0] ?? '?').toUpperCase()}
    </span>
  )
}

function PinStep({
  profile,
  onBack,
  onDone,
}: {
  profile: ProfileView
  onBack: () => void
  onDone: () => void
}): React.JSX.Element {
  // A profile whose PIN the host cleared chooses a new one here.
  const choosing = !profile.hasPin
  const [pin, setPin] = useState('')
  const [confirm, setConfirm] = useState('')
  const [remember, setRemember] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [shake, setShake] = useState(0)

  const submit = async (): Promise<void> => {
    if (!validPin(pin)) {
      setError('A PIN is 4 to 8 digits.')
      return
    }
    if (choosing && pin !== confirm) {
      setError('The two PINs are not the same.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      await api.signInProfile(profile.id, pin, remember)
      onDone()
    } catch (e) {
      setError(e instanceof ApiError || e instanceof Error ? capitalise(e.message) : String(e))
      setPin('')
      setShake((n) => n + 1)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="mx-auto flex max-w-[360px] flex-col items-center text-center">
      <BackButton onClick={onBack} />
      <Avatar name={profile.name} color={profile.color} size={84} />
      <h2 className="mt-4 text-[19px] font-semibold tracking-tight text-text">{profile.name}</h2>
      <p className="mt-1 text-[12px] text-textDim">
        {choosing ? 'This PIN was reset on the host. Choose a new one.' : 'Enter your PIN'}
      </p>

      <motion.div
        key={shake}
        animate={shake ? { x: [0, -9, 8, -6, 4, 0] } : undefined}
        transition={{ duration: 0.36 }}
        className="mt-6 w-full"
      >
        <PinField value={pin} onChange={setPin} onEnter={() => void submit()} autoFocus />
        {choosing && (
          <div className="mt-3">
            <PinField
              value={confirm}
              onChange={setConfirm}
              onEnter={() => void submit()}
              placeholder="Again, to be sure"
            />
          </div>
        )}
      </motion.div>

      <div className="mt-2 h-5 text-[11.5px] text-danger">{error}</div>

      <Checkbox
        checked={remember}
        onChange={setRemember}
        label="Keep me signed in on this device"
        className="mt-1"
      />

      <button
        onClick={() => void submit()}
        disabled={busy || pin.length < 4}
        className="mt-5 flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-basalt text-[13px] font-medium text-ink transition-opacity duration-150 disabled:opacity-40"
      >
        {busy && <Loader2 size={14} className="animate-spin" />}
        {choosing ? 'Set PIN and sign in' : 'Sign in'}
      </button>
    </div>
  )
}

function CreateStep({
  taken,
  onBack,
  onDone,
}: {
  taken: string[]
  onBack: () => void
  onDone: () => void
}): React.JSX.Element {
  const [name, setName] = useState('')
  const [color, setColor] = useState(() => Math.floor(Math.random() * PROFILE_COLORS.length))
  const [pin, setPin] = useState('')
  const [confirm, setConfirm] = useState('')
  const [remember, setRemember] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const trimmed = name.trim()
  const clash = taken.includes(trimmed.toLowerCase())
  const ready = trimmed.length > 0 && !clash && validPin(pin) && pin === confirm

  const submit = async (): Promise<void> => {
    if (!trimmed) return setError('Give the profile a name.')
    if (clash) return setError('There is already a profile with that name.')
    if (!validPin(pin)) return setError('A PIN is 4 to 8 digits.')
    if (pin !== confirm) return setError('The two PINs are not the same.')
    setBusy(true)
    setError(null)
    try {
      await api.createProfile(trimmed, pin, color, remember)
      onDone()
    } catch (e) {
      setError(e instanceof Error ? capitalise(e.message) : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="mx-auto flex max-w-[380px] flex-col items-center text-center">
      <BackButton onClick={onBack} />
      <motion.div key={`${color}-${trimmed[0] ?? ''}`} initial={{ scale: 0.92 }} animate={{ scale: 1 }}>
        <Avatar name={trimmed || '?'} color={color} size={84} />
      </motion.div>
      <h2 className="mt-4 text-[19px] font-semibold tracking-tight text-text">New profile</h2>
      <p className="mt-1 max-w-[320px] text-[12px] leading-relaxed text-textDim">
        Its watch history and stars follow it to every device in the house.
      </p>

      <div className="mt-6 w-full space-y-4 text-left">
        <label className="block">
          <span className="text-[11px] text-textFaint">Name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={32}
            autoFocus
            placeholder="Maya"
            className="mt-1.5 h-10 w-full rounded-lg border border-white/[0.1] bg-ink2 px-3 text-[13.5px] text-text outline-none transition-colors placeholder:text-textFaint focus:border-white/30"
          />
          {clash && <span className="mt-1 block text-[11px] text-danger">That name is taken.</span>}
        </label>

        <div>
          <span className="text-[11px] text-textFaint">Colour</span>
          <div className="mt-2 flex justify-between">
            {PROFILE_COLORS.map((tone, index) => (
              <button
                key={tone}
                type="button"
                onClick={() => setColor(index)}
                aria-label={`Colour ${index + 1}`}
                className={cn(
                  'flex h-8 w-8 items-center justify-center rounded-full transition-transform duration-150 hover:scale-110',
                  color === index && 'ring-2 ring-white/80 ring-offset-2 ring-offset-ink',
                )}
                style={{ background: tone }}
              >
                {color === index && <Check size={13} className="text-white" strokeWidth={3} />}
              </button>
            ))}
          </div>
        </div>

        <div>
          <span className="text-[11px] text-textFaint">PIN · 4 to 8 digits</span>
          <div className="mt-1.5 space-y-2.5">
            <PinField value={pin} onChange={setPin} onEnter={() => void submit()} />
            <PinField
              value={confirm}
              onChange={setConfirm}
              onEnter={() => void submit()}
              placeholder="Again, to be sure"
            />
          </div>
        </div>
      </div>

      <div className="mt-3 h-5 text-[11.5px] text-danger">{error}</div>

      <Checkbox
        checked={remember}
        onChange={setRemember}
        label="Keep me signed in on this device"
      />

      <button
        onClick={() => void submit()}
        disabled={busy || !ready}
        className="mt-5 flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-basalt text-[13px] font-medium text-ink transition-opacity duration-150 disabled:opacity-40"
      >
        {busy && <Loader2 size={14} className="animate-spin" />}
        Create profile
      </button>
    </div>
  )
}

/**
 * A PIN field that shows dots, one per digit and room for eight.
 *
 * A real input underneath, so typing, pasting and the number pad all work
 * as they do everywhere else; the dots are only how it looks.
 */
function PinField({
  value,
  onChange,
  onEnter,
  autoFocus,
  placeholder = 'PIN',
}: {
  value: string
  onChange: (value: string) => void
  onEnter: () => void
  autoFocus?: boolean
  placeholder?: string
}): React.JSX.Element {
  const input = useRef<HTMLInputElement | null>(null)
  const [focused, setFocused] = useState(autoFocus ?? false)

  return (
    <div
      onClick={() => input.current?.focus()}
      className={cn(
        'relative flex h-12 w-full cursor-text items-center justify-center gap-3 rounded-lg border bg-ink2 transition-colors duration-150',
        focused ? 'border-white/30' : 'border-white/[0.1]',
      )}
    >
      <input
        ref={input}
        value={value}
        inputMode="numeric"
        autoComplete="off"
        autoFocus={autoFocus}
        aria-label={placeholder}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onChange={(e) => onChange(e.target.value.replace(/[^0-9]/g, '').slice(0, 8))}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onEnter()
        }}
        className="absolute inset-0 h-full w-full cursor-text opacity-0"
      />
      {value.length === 0 ? (
        <span className="pointer-events-none text-[12.5px] text-textFaint">{placeholder}</span>
      ) : (
        Array.from({ length: Math.max(4, value.length) }, (_, i) => (
          <motion.span
            key={i}
            initial={false}
            animate={{ scale: i < value.length ? 1 : 0.6 }}
            transition={{ type: 'spring', stiffness: 700, damping: 28 }}
            className={cn(
              'pointer-events-none h-2.5 w-2.5 rounded-full',
              i < value.length ? 'bg-text' : 'bg-white/[0.12]',
            )}
          />
        ))
      )}
    </div>
  )
}

function Checkbox({
  checked,
  onChange,
  label,
  className,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  label: string
  className?: string
}): React.JSX.Element {
  return (
    <label className={cn('flex cursor-pointer select-none items-center gap-2.5', className)}>
      <button
        type="button"
        role="checkbox"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={cn(
          'flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors duration-150',
          checked ? 'border-transparent bg-basalt text-ink' : 'border-white/25 bg-transparent',
        )}
      >
        {checked && <Check size={11} strokeWidth={3} />}
      </button>
      <span className="text-[12px] text-textDim" onClick={() => onChange(!checked)}>
        {label}
      </span>
    </label>
  )
}

function BackButton({ onClick }: { onClick: () => void }): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className="mb-6 flex items-center gap-1.5 self-start rounded-md px-2 py-1 text-[12px] text-textFaint transition-colors duration-150 hover:bg-white/[0.05] hover:text-text"
    >
      <ArrowLeft size={13} />
      Back
    </button>
  )
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/**
 * In the browser preview only: `?gate=pin`, `?gate=reset` or `?gate=create`
 * opens straight on that step, so each can be looked at without clicking.
 */
function previewStep(profiles: ProfileView[]): Step {
  if (inTauri() || typeof window === 'undefined') return { kind: 'choose' }
  const gate = new URLSearchParams(window.location.search).get('gate')
  const pinned = profiles.find((p) => p.hasPin)
  const reset = profiles.find((p) => !p.hasPin)
  if (gate === 'create') return { kind: 'create' }
  if (gate === 'pin' && pinned) return { kind: 'pin', profile: pinned }
  if (gate === 'reset' && reset) return { kind: 'pin', profile: reset }
  return { kind: 'choose' }
}
