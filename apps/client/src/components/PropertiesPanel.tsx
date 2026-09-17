import { useEffect, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { X } from 'lucide-react'
import type { Entry } from './FileList'
import { iconFor } from './FileList'
import { api, parentOf } from '@/lib/api'
import { formatBytes, formatDate } from '@/lib/utils'

/**
 * What a file actually is.
 *
 * Every value is read back from the host when the panel opens rather than
 * taken from the listing, because a listing can be minutes old — the point of
 * opening properties is usually to check something has changed. A folder gets
 * its contents counted, which the listing deliberately does not do.
 */
export function PropertiesPanel({
  entry,
  vaultName,
  onClose,
}: {
  entry: Entry | null
  vaultName: string
  onClose: () => void
}): React.JSX.Element {
  const [size, setSize] = useState<number | null>(null)
  const [modified, setModified] = useState<number | null>(null)
  const [contents, setContents] = useState<{ files: number; folders: number } | null>(
    null,
  )
  const [readonly, setReadonly] = useState(false)

  useEffect(() => {
    if (!entry) return
    let cancelled = false
    setSize(null)
    setModified(null)
    setContents(null)
    setReadonly(false)

    void (async () => {
      try {
        const fresh = await api.stat(entry.id)
        if (cancelled) return
        setSize(fresh.size)
        setModified(fresh.mtime * 1000)
        setReadonly(fresh.readonly)
      } catch {
        // Fall back to what the listing already knew rather than showing
        // nothing: a stale figure beats an empty panel.
        if (!cancelled) {
          setSize(entry.size)
          setModified(entry.modified)
        }
      }

      if (entry.kind === 'dir') {
        try {
          const listing = await api.list(entry.id)
          if (cancelled) return
          setContents({
            files: listing.filter((e) => e.kind === 'file').length,
            folders: listing.filter((e) => e.kind === 'dir').length,
          })
        } catch {
          // A folder that will not list still has a name and a date worth
          // showing.
        }
      }
    })()

    return () => {
      cancelled = true
    }
  }, [entry])

  useEffect(() => {
    if (!entry) return undefined
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [entry, onClose])

  const Icon = entry ? iconFor(entry) : null
  const folder = entry ? parentOf(entry.id) : ''

  return (
    <AnimatePresence>
      {entry && (
        <motion.aside
          initial={{ x: 320, opacity: 0 }}
          animate={{ x: 0, opacity: 1 }}
          exit={{ x: 320, opacity: 0 }}
          transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
          className="fixed bottom-0 right-0 top-9 z-[60] flex w-[300px] flex-col border-l border-line bg-panel shadow-lift"
        >
          <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-4">
            <span className="text-[12px] font-semibold text-text">Properties</span>
            <div className="flex-1" />
            <button
              onClick={onClose}
              aria-label="Close properties"
              className="flex h-7 w-7 items-center justify-center rounded text-textFaint transition-colors hover:bg-white/[0.06] hover:text-text"
            >
              <X size={14} />
            </button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
            <div className="flex flex-col items-center text-center">
              {Icon && (
                <Icon
                  size={40}
                  strokeWidth={1.2}
                  className={
                    entry.kind === 'dir' ? 'text-basaltDeep' : 'text-textFaint'
                  }
                />
              )}
              <p className="mt-3 break-all text-[13px] font-medium text-text">
                {entry.name}
              </p>
            </div>

            <div className="mt-5 space-y-px">
              <Row label="Kind" value={describeKind(entry)} />
              <Row
                label="Size"
                value={
                  entry.kind === 'dir'
                    ? contents
                      ? `${contents.files} file${contents.files === 1 ? '' : 's'}, ${contents.folders} folder${contents.folders === 1 ? '' : 's'}`
                      : 'counting…'
                    : size === null
                      ? '…'
                      : `${formatBytes(size)} (${size.toLocaleString()} bytes)`
                }
              />
              <Row
                label="Modified"
                value={modified === null ? '…' : formatDate(modified)}
              />
              <Row label="Where" value={folder ? `${vaultName}/${folder}` : vaultName} />
              <Row label="Full path" value={entry.id} mono />
              {readonly && <Row label="Attributes" value="read-only" />}
            </div>

            {entry.kind === 'dir' && contents && (
              <p className="mt-4 text-[11px] leading-relaxed text-textFaint">
                Counts what is directly inside this folder. Adding up a whole
                tree means walking every subfolder on the drive, which is not
                something a panel should do while you wait.
              </p>
            )}
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  )
}

function describeKind(entry: Entry): string {
  if (entry.kind === 'dir') return 'Folder'
  const dot = entry.name.lastIndexOf('.')
  if (dot <= 0) return 'File'
  return `${entry.name.slice(dot + 1).toUpperCase()} file`
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
    <div className="flex gap-3 rounded px-1 py-1.5">
      <span className="w-[74px] shrink-0 text-[11px] text-textFaint">{label}</span>
      <span
        className={
          mono
            ? 'min-w-0 flex-1 break-all font-mono text-[10px] text-textDim'
            : 'min-w-0 flex-1 break-words text-[11px] text-text'
        }
      >
        {value}
      </span>
    </div>
  )
}
