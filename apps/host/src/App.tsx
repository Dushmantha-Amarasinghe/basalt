import { useCallback, useMemo, useState } from 'react'
import { motion } from 'framer-motion'
import { AlertTriangle } from 'lucide-react'
import { api, type DeviceView, type HostStatus } from '@/lib/api'
import { usePoll } from '@/lib/usePoll'
import { DeviceList } from './components/DeviceList'
import { PairingRequests } from './components/PairingRequests'
import { SettingsPanel } from './components/SettingsPanel'
import { Setup } from './components/Setup'
import { TitleBar } from './components/TitleBar'
import { VaultCard } from './components/VaultCard'
import { HexMark } from './components/HexMark'
import { PromptDialog, type PromptRequest } from './components/ui/PromptDialog'

/** How often the dashboard asks for each thing. */
const STATUS_INTERVAL = 2_000
/** Faster: this is where the live speeds come from. */
const DEVICE_INTERVAL = 1_000
/** Faster still: a PIN is on a two-minute clock and someone is waiting. */
const PAIRING_INTERVAL = 900

export function App(): React.JSX.Element {
  const status = usePoll<HostStatus>(useCallback(() => api.status(), []), STATUS_INTERVAL)
  const devices = usePoll<DeviceView[]>(useCallback(() => api.devices(), []), DEVICE_INTERVAL)
  const pairings = usePoll(useCallback(() => api.pendingPairings(), []), PAIRING_INTERVAL)

  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  /** Set when the user asks to share something else, over a live vault. */
  const [reconfiguring, setReconfiguring] = useState(false)

  const list = devices.data ?? []
  const busy = useMemo(
    () => list.some((device) => device.sendRate > 0 || device.receiveRate > 0),
    [list],
  )

  const apply = useCallback(
    (next: HostStatus) => {
      // The command already returned the new status, so the switch moves at
      // once instead of snapping back until the next poll catches up.
      status.set(next)
    },
    [status],
  )

  const chooseVault = useCallback(
    async (path: string, name: string) => {
      apply(await api.chooseVault(path, name))
      setReconfiguring(false)
    },
    [apply],
  )

  if (status.loading && !status.data) return <Splash />

  const current = status.data
  if (!current) return <Splash message={status.error ?? undefined} />

  const needsSetup = !current.vault || reconfiguring

  return (
    <div className="relative flex h-full flex-col">
      <div className="backdrop" />

      <TitleBar
        hostName={current.hostName}
        vaultName={needsSetup ? null : (current.vault?.name ?? null)}
        serving={current.serving}
        busy={busy}
      />

      {/*
        Keyed and faded in, with no exit animation to wait for.
        `AnimatePresence mode="wait"` would be the obvious way to cross-fade
        these, but it holds the outgoing view on screen until its exit
        animation finishes — and this app's window really does get hidden, to
        the tray and at login, which suspends animation frames. A swap that
        cannot complete while nobody is looking is a swap that can be stuck
        when they look again.
      */}
      <main className="relative z-10 min-h-0 flex-1">
        <div key={needsSetup ? 'setup' : 'dashboard'} className="h-full">
          {needsSetup ? (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.18 }}
              className="h-full"
            >
              <Setup hostName={current.hostName} onChosen={chooseVault} />
            </motion.div>
          ) : (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.18 }}
              className="h-full overflow-y-auto"
            >
              <div className="mx-auto flex max-w-[740px] flex-col gap-6 px-8 py-7">
                {!current.serving && current.problem && (
                  <div className="flex items-start gap-3 rounded-md bg-dangerBg px-4 py-3">
                    <span className="mt-0.5 shrink-0 text-danger">
                      <AlertTriangle size={14} />
                    </span>
                    <p className="text-[12.5px] leading-relaxed text-danger">
                      {current.problem}
                    </p>
                  </div>
                )}

                <VaultCard
                  status={current}
                  onChange={() => setReconfiguring(true)}
                  onOpen={() => void api.openVaultFolder().catch(() => {})}
                />

                <section>
                  <PairingRequests
                    requests={pairings.data ?? []}
                    onDeny={(id) => {
                      void api.denyPairing(id).then(() => pairings.refresh())
                    }}
                  />

                  <div className="mb-2.5 mt-1 flex items-baseline justify-between">
                    <h3 className="font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
                      Devices
                    </h3>
                    <span className="tnum font-mono text-[10px] text-textFaint">
                      {list.filter((d) => d.online).length} of {list.length} connected
                    </span>
                  </div>

                  <DeviceList
                    devices={list}
                    onRevoke={(device) =>
                      setPrompt({
                        title: `Remove ${device.name}?`,
                        value: device.name,
                        confirmLabel: 'Remove',
                        select: 'all',
                        // Confirmed by retyping nothing in particular: the
                        // dialog is the confirmation, and the field carries the
                        // name so it is obvious which device is going.
                        onConfirm: () => {
                          void api.revokeDevice(device.id).then(() => {
                            devices.refresh()
                            status.refresh()
                          })
                        },
                      })
                    }
                    onRename={(device) =>
                      setPrompt({
                        title: 'Rename this device',
                        value: device.name,
                        confirmLabel: 'Rename',
                        select: 'all',
                        onConfirm: (name) => {
                          void api
                            .renameDevice(device.id, name)
                            .then(() => devices.refresh())
                        },
                      })
                    }
                    onToggleWritable={(device) => {
                      void api
                        .setDeviceWritable(device.id, !device.writable)
                        .then(() => devices.refresh())
                    }}
                  />
                </section>

                <section>
                  <h3 className="mb-2.5 font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
                    Settings
                  </h3>
                  <SettingsPanel
                    status={current}
                    onRequirePin={(require) => {
                      void api.setRequirePin(require).then((next) => {
                        apply(next)
                        pairings.refresh()
                      })
                    }}
                    onStartWithWindows={(enabled) => {
                      void api.setStartWithWindows(enabled).then(apply)
                    }}
                    onRename={(name) => {
                      void api.setHostName(name).then(apply)
                    }}
                  />
                </section>
              </div>
            </motion.div>
          )}
        </div>
      </main>

      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
    </div>
  )
}

/**
 * The first moment, before the backend has answered.
 *
 * It says something. The client once showed a bare black window here, and the
 * only honest reading of that from the outside was a crash.
 */
function Splash({ message }: { message?: string }): React.JSX.Element {
  return (
    <div className="relative flex h-full flex-col items-center justify-center gap-4">
      <div className="backdrop" />
      <motion.div
        animate={{ opacity: [0.4, 0.9, 0.4] }}
        transition={{ duration: 2.2, repeat: Infinity, ease: 'easeInOut' }}
        className="relative z-10 text-basaltDeep"
      >
        <HexMark size={30} />
      </motion.div>
      <p className="relative z-10 max-w-[320px] text-center text-[12px] leading-relaxed text-textFaint">
        {message ?? 'Starting up…'}
      </p>
    </div>
  )
}
