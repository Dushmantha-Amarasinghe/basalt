import { useEffect, useMemo, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { FolderOpen, HardDrive, Network, RefreshCw, Usb } from 'lucide-react'
import { api, pickFolder, type DriveView } from '@/lib/api'
import { cn, formatBytes } from '@/lib/utils'
import { HexMark } from './HexMark'

/**
 * The first screen: which drive?
 *
 * One question, answered from a list rather than from a file picker. A picker
 * is still here, below the fold, because sharing a folder rather than a whole
 * drive is a reasonable thing to want — but it is the exception, and the
 * common case should not cost a dialog and a directory tree.
 *
 * No network settings, no share names, no permissions. Choosing a drive is the
 * entire setup, which is the product requirement this app exists to satisfy.
 */
export function Setup({
  hostName,
  onChosen,
}: {
  hostName: string
  onChosen: (path: string, name: string) => Promise<void>
}): React.JSX.Element {
  const [drives, setDrives] = useState<DriveView[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = async (): Promise<void> => {
    setDrives(await api.listDrives())
  }

  useEffect(() => {
    void load()
  }, [])

  const chosen = useMemo(
    () => drives?.find((drive) => drive.path === selected) ?? null,
    [drives, selected],
  )

  const share = async (): Promise<void> => {
    if (!selected) return
    setBusy(true)
    setError(null)
    try {
      await onChosen(selected, name.trim() || chosen?.name || selected)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const browse = async (): Promise<void> => {
    const folder = await pickFolder()
    if (!folder) return
    setSelected(folder)
    setName(folder.split(/[\\/]/).filter(Boolean).pop() ?? folder)
    // A picked folder is not in the drive list, so it gets its own card.
    setDrives((current) =>
      current && current.some((d) => d.path === folder)
        ? current
        : [
            ...(current ?? []),
            {
              path: folder,
              name: folder,
              label: folder.split(/[\\/]/).filter(Boolean).pop() ?? folder,
              kind: 'other',
              free: 0,
              total: 0,
              ready: true,
            },
          ],
    )
  }

  return (
    <div className="flex h-full flex-col items-center overflow-y-auto px-8 py-10">
      <div className="w-full max-w-[680px]">
        <header className="mb-8 flex items-start gap-4">
          <div className="mt-1 text-basaltDeep">
            <HexMark size={26} />
          </div>
          <div>
            <h1 className="text-[22px] font-semibold tracking-tighter text-text">
              Choose a drive to share
            </h1>
            <p className="mt-1.5 text-[13px] leading-relaxed text-textDim">
              Your other devices will find{' '}
              <span className="font-mono text-textDim">{hostName}</span> on this network by
              themselves. There is nothing else to set up — no addresses, no accounts.
            </p>
          </div>
        </header>

        <div className="mb-3 flex items-center justify-between">
          <span className="font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
            Drives on this machine
          </span>
          <button
            onClick={() => void load()}
            className="flex items-center gap-1.5 rounded-sm px-2 py-1 text-[11px] text-textFaint transition-colors hover:bg-panel2 hover:text-textDim"
          >
            <RefreshCw size={11} />
            Rescan
          </button>
        </div>

        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {drives === null
            ? [0, 1, 2, 3].map((i) => (
                <div key={i} className="h-[72px] animate-pulse rounded-md bg-panel/60" />
              ))
            : drives.map((drive) => (
                <DriveCard
                  key={drive.path}
                  drive={drive}
                  selected={drive.path === selected}
                  onSelect={() => {
                    setSelected(drive.path)
                    // The label, not the list name: your other devices have no
                    // use for this machine's drive letters.
                    setName(drive.label || drive.name)
                  }}
                />
              ))}
        </div>

        <button
          onClick={() => void browse()}
          className="mt-3 flex w-full items-center justify-center gap-2 rounded-md border border-dashed border-line py-3 text-[12px] text-textFaint transition-colors hover:border-lineBright hover:text-textDim"
        >
          <FolderOpen size={13} />
          Or pick a single folder instead
        </button>

        <AnimatePresence>
          {chosen && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
              className="overflow-hidden"
            >
              <div className="mt-6 rounded-md glass p-4">
                <label className="block font-mono text-[10px] uppercase tracking-[0.16em] text-textFaint">
                  What your devices will call it
                </label>
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  maxLength={48}
                  className="mt-2 w-full rounded-sm border border-line bg-ink2 px-3 py-2 text-[14px] text-text outline-none transition-colors focus:border-lineBright"
                />
                <p className="mt-2 text-[11px] text-textFaint">
                  Shown on every paired device instead of the drive letter.
                </p>
              </div>

              {error && (
                <p className="mt-3 rounded-sm bg-dangerBg px-3 py-2 text-[12px] text-danger">
                  {error}
                </p>
              )}

              <button
                onClick={() => void share()}
                disabled={busy || !chosen.ready}
                className={cn(
                  'mt-4 w-full rounded-md py-3 text-[13px] font-semibold tracking-tight transition-colors',
                  busy || !chosen.ready
                    ? 'cursor-not-allowed bg-panel2 text-textFaint'
                    : 'bg-basalt text-ink hover:bg-white',
                )}
              >
                {busy ? 'Starting…' : `Share ${name.trim() || chosen.name}`}
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}

function DriveIcon({ kind }: { kind: DriveView['kind'] }): React.JSX.Element {
  if (kind === 'removable') return <Usb size={15} />
  if (kind === 'network') return <Network size={15} />
  if (kind === 'other') return <FolderOpen size={15} />
  return <HardDrive size={15} />
}

function DriveCard({
  drive,
  selected,
  onSelect,
}: {
  drive: DriveView
  selected: boolean
  onSelect: () => void
}): React.JSX.Element {
  const used = drive.total > 0 ? (drive.total - drive.free) / drive.total : 0

  return (
    <button
      onClick={drive.ready ? onSelect : undefined}
      disabled={!drive.ready}
      className={cn(
        'group relative overflow-hidden rounded-md border p-3 text-left transition-colors',
        selected ? 'basalt-edge border-transparent' : 'border-line bg-panel hover:bg-panel2',
        !drive.ready && 'cursor-not-allowed opacity-45 hover:bg-panel',
      )}
    >
      <div className="flex items-center gap-2.5">
        <span className={cn('shrink-0', selected ? 'text-text' : 'text-textFaint')}>
          <DriveIcon kind={drive.kind} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-text">
          {drive.name}
        </span>
      </div>

      {drive.ready ? (
        <>
          <div className="mt-3 h-[3px] w-full overflow-hidden rounded-full bg-ink2">
            <div
              className="h-full rounded-full bg-basaltDim transition-[width] duration-500"
              style={{ width: `${Math.round(used * 100)}%` }}
            />
          </div>
          <div className="tnum mt-2 font-mono text-[10px] text-textFaint">
            {formatBytes(drive.free)} free of {formatBytes(drive.total)}
          </div>
        </>
      ) : (
        <div className="mt-3 font-mono text-[10px] text-textFaint">
          {/* An empty card reader slot is worth listing: a user who expects to
              see E: and does not would otherwise have no idea why. */}
          nothing in this slot
        </div>
      )}
    </button>
  )
}
