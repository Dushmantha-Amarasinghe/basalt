import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ArrowLeftRight,
  ArrowUpCircle,
  ArrowLeft,
  CheckSquare,
  Clock,
  Copy,
  Download,
  FilePlus2,
  Film,
  FolderOpen,
  FolderPlus,
  Images,
  Library,
  Loader2,
  MoreHorizontal,
  Music,
  Plus,
  Scissors,
  Search,
  Star,
  Trash2,
  Tv,
  Upload,
  Video,
  X,
} from 'lucide-react'
import { Switch } from '@/components/Switch'
import { isWaiting, waitingLabel } from '@/lib/useVault'
import { setShowHidden, useShowHidden } from '@/lib/showHidden'
import { musicItem, type AppModel } from '@/App'
import { HexMark } from '@/components/HexMark'
import { PairingView } from '@/components/PairingView'
import { ProfileGate } from '@/components/ProfileGate'
import { LibraryView } from '@/components/LibraryView'
import { MusicList, PhotoGrid, VideoGrid } from '@/components/MediaViews'
import { PlayerOverlay } from '@/components/PlayerOverlay'
import { ImageViewer } from '@/components/ImageViewer'
import { PromptDialog } from '@/components/ui/PromptDialog'
import { PropertiesDetails } from '@/components/PropertiesPanel'
import { About } from '@/components/About'
import type { NavKey } from '@/components/Sidebar'
import type { Entry } from '@/components/FileList'
import { api, isFinished, parentOf } from '@/lib/api'
import { android, applyInsets, type PhoneFile } from '@/lib/android'
import { profileColor } from '@/lib/useIdentity'
import { fileToEntry } from '@/lib/useCollections'
import { cn, formatBytes } from '@/lib/utils'
import { FilesScreen } from './FilesScreen'
import { Rise } from './presence'
import { isAndroid } from '@/lib/platform'
import { offered, useUpdate, type UpdateState } from '@/lib/updates'
import { ActionSheet, Sheet } from './Sheet'
import { useBack } from './useBack'

type Tab = 'files' | 'library' | 'recent' | 'more'
type LibrarySection = 'movies' | 'series' | 'videos' | 'music' | 'photos'

const SECTIONS: Array<{ key: LibrarySection; label: string; icon: typeof Film }> = [
  { key: 'movies', label: 'Movies', icon: Film },
  { key: 'series', label: 'TV Series', icon: Tv },
  { key: 'videos', label: 'Videos', icon: Video },
  { key: 'music', label: 'Music', icon: Music },
  { key: 'photos', label: 'Photos', icon: Images },
]

/**
 * The phone and tablet app.
 *
 * The same model as the desktop — the same uploads, downloads, library,
 * profiles and player — laid out for a hand rather than a mouse. A phone gets
 * tabs along the bottom, where a thumb reaches; a tablet gets them down the
 * side, where a phone's width would waste the screen.
 */
export function MobileApp({ model }: { model: AppModel }): React.JSX.Element {
  useEffect(() => {
    void applyInsets()
    document.documentElement.classList.add('mobile')
  }, [])

  // Back on the screen: anything the host said while the phone slept may
  // have been lost with its connection, so the watch starts again and asks.
  useEffect(() => {
    const onShow = (): void => {
      if (document.visibilityState === 'visible') void api.rewatch().catch(() => {})
    }
    document.addEventListener('visibilitychange', onShow)
    return () => document.removeEventListener('visibilitychange', onShow)
  }, [])

  const { vault, identity, connected } = model
  // Choosing another drive from More: the drive list, with a way back.
  const [changingDrive, setChangingDrive] = useState(false)
  useBack(changingDrive, () => {
    setChangingDrive(false)
    return true
  })

  if (!vault.status) return <Splash />

  // The drive list: on first use, when the host has removed this device, and
  // when changing drives.
  if ((!connected && !vault.status.hasPaired) || vault.removed || changingDrive) {
    return (
      <Safe>
        <PairingView
          notice={vault.removed}
          onBack={changingDrive ? () => setChangingDrive(false) : undefined}
          currentHostId={changingDrive ? vault.status.hostId : null}
          onPaired={(next) => {
            setChangingDrive(false)
            vault.switchTo(next)
          }}
        />
      </Safe>
    )
  }

  if (connected && identity.supported && !identity.loaded) return <Splash />
  if (
    connected &&
    identity.supported &&
    identity.state &&
    (identity.state.choose || model.signingIn)
  ) {
    return (
      <Safe>
        <div className="h-full overflow-y-auto">
          <ProfileGate
            vaultName={vault.status.vault ?? 'the drive'}
            deviceName={vault.status.deviceName}
            profiles={identity.profiles}
            lastProfile={identity.state.lastProfile}
            ended={identity.state.ended}
            onDone={() => {
              model.setSigningIn(false)
              void identity.refresh()
              void identity.reloadProfiles()
            }}
            onChangeDrive={() => {
              model.setSigningIn(false)
              setChangingDrive(true)
            }}
          />
        </div>
      </Safe>
    )
  }

  return <Shell model={model} onChangeDrive={() => setChangingDrive(true)} />
}

/** Clear of the status bar and the gesture bar. */
function Safe({ children }: { children: React.ReactNode }): React.JSX.Element {
  return (
    <div
      className="relative h-full"
      style={{ paddingTop: 'var(--inset-top, 0px)', paddingBottom: 'var(--inset-bottom, 0px)' }}
    >
      {children}
    </div>
  )
}

function Splash(): React.JSX.Element {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4">
      <HexMark size={40} className="text-basalt" />
      <Loader2 size={16} className="animate-spin text-textFaint" />
    </div>
  )
}

/** Whether the screen is wide enough to put the tabs down the side. */
function useWide(): boolean {
  const [wide, setWide] = useState(() => window.innerWidth >= 600)
  useEffect(() => {
    const onResize = (): void => setWide(window.innerWidth >= 600)
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])
  return wide
}

function Shell({ model, onChangeDrive }: { model: AppModel; onChangeDrive: () => void }): React.JSX.Element {
  const {
    vault,
    nav,
    setNav,
    query,
    setQuery,
    selected,
    setSelected,
    entries,
    actions,
    writable,
    hiddenSections,
    transfers,
    notice,
    setNotice,
    playing,
    viewer,
  } = model
  const wide = useWide()
  const [tab, setTab] = useState<Tab>('files')
  const [section, setSection] = useState<LibrarySection>('movies')
  const [recentMode, setRecentMode] = useState<'recent' | 'starred'>('recent')
  const [searching, setSearching] = useState(false)
  const [menuFor, setMenuFor] = useState<Entry | null>(null)
  const [adding, setAdding] = useState(false)
  const [transfersOpen, setTransfersOpen] = useState(false)
  const [shared, setShared] = useState<PhoneFile[]>([])

  const visibleSections = SECTIONS.filter((s) => !hiddenSections.has(s.key))
  const librarySection = visibleSections.some((s) => s.key === section)
    ? section
    : (visibleSections[0]?.key ?? 'movies')

  // The tab decides which of the model's sections is loaded.
  useEffect(() => {
    const wanted: NavKey =
      tab === 'files'
        ? 'files'
        : tab === 'library'
          ? librarySection
          : tab === 'recent'
            ? recentMode
            : 'settings'
    if (nav !== wanted) setNav(wanted)
  }, [tab, librarySection, recentMode, nav, setNav])

  const selecting = selected.size > 0
  const chosen = entries.filter((e) => selected.has(e.id))

  // Files shared into Basalt from another app: offered for uploading into
  // whichever folder is open, until uploaded or put aside.
  useEffect(() => {
    const take = async (): Promise<void> => {
      const files = await android.takeShared()
      if (files.length > 0) {
        setShared((now) => [...now, ...files])
        setTab('files')
      }
    }
    void take()
    const onVisible = (): void => {
      if (document.visibilityState === 'visible') {
        void take()
        // Back from another player: the stream it was reading is over.
        if (!playing) void android.letGo('playback')
      }
    }
    document.addEventListener('visibilitychange', onVisible)
    let stop: (() => void) | undefined
    void (async () => {
      try {
        const { addPluginListener } = await import('@tauri-apps/api/core')
        const listener = await addPluginListener('basalt-android', 'shared', () => void take())
        stop = () => void listener.unregister()
      } catch {
        // Not on Android.
      }
    })()
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      stop?.()
    }
  }, [playing])

  useKeepAliveForTransfers(model)

  // Back: whatever is on top first.
  useBack(true, () => {
    if (selecting) {
      setSelected(new Set())
      return true
    }
    if (searching) {
      setSearching(false)
      setQuery('')
      return true
    }
    if (tab === 'files' && vault.dir) {
      vault.open(parentOf(vault.dir))
      return true
    }
    if (tab !== 'files') {
      setTab('files')
      return true
    }
    return false
  })

  useEffect(() => {
    if (!notice) return undefined
    const timer = setTimeout(() => setNotice(null), 4500)
    return () => clearTimeout(timer)
  }, [notice, setNotice])

  const goTo = (next: Tab): void => {
    void android.haptic('tap')
    setSelected(new Set())
    setSearching(false)
    setQuery('')
    setTab(next)
  }

  // A newer Basalt: a dot on More, a banner over the other tabs until it is
  // dismissed for that version, and a notification outside the app, once.
  // Each leads to More, where the offer is with its notes.
  const update = useUpdate()
  const [bannerGone, setBannerGone] = useState<string | null>(() => remembered(BANNER_DISMISSED))
  const showUpdate = useCallback(() => {
    setSelected(new Set())
    setSearching(false)
    setQuery('')
    setTab('more')
    setTimeout(
      () => document.getElementById('basalt-update')?.scrollIntoView({ behavior: 'smooth', block: 'center' }),
      350,
    )
  }, [setSelected])
  useUpdateNotification(update)
  useNotificationTaps(showUpdate)

  const active = transfers.active

  return (
    <div
      className="relative flex h-full bg-ink"
      style={{ paddingLeft: 'var(--inset-left, 0px)', paddingRight: 'var(--inset-right, 0px)' }}
    >
      {wide && <Rail tab={tab} onTab={goTo} activeTransfers={active.length} attention={offered(update)} />}

      <div className="flex min-w-0 flex-1 flex-col">
        <div style={{ height: 'var(--inset-top, 0px)' }} className="shrink-0" />

        {selecting ? (
          <SelectionBar
            count={selected.size}
            onClose={() => setSelected(new Set())}
            onSelectAll={() => setSelected(new Set(entries.map((e) => e.id)))}
            onDownload={() => void model.downloadMany(chosen)}
            onCut={writable ? () => {
              actions.cut(chosen.map((e) => e.id))
              setSelected(new Set())
            } : undefined}
            onCopy={() => {
              actions.copy(chosen.map((e) => e.id))
              setSelected(new Set())
            }}
            onDelete={writable ? () => {
              void actions.remove(chosen).then(() => setSelected(new Set()))
            } : undefined}
            onMore={() => chosen[0] && setMenuFor(chosen[0])}
          />
        ) : (
          <TopBar
            model={model}
            tab={tab}
            searching={searching}
            onSearch={() => setSearching(true)}
            onCloseSearch={() => {
              setSearching(false)
              setQuery('')
            }}
            query={query}
            onQuery={setQuery}
          />
        )}

        {tab === 'files' && !selecting && !searching && <Crumbs model={model} />}
        {tab === 'library' && !selecting && visibleSections.length > 1 && (
          <Chips
            items={visibleSections.map((s) => ({ key: s.key, label: s.label }))}
            active={librarySection}
            onChoose={(key) => setSection(key as LibrarySection)}
          />
        )}
        {tab === 'recent' && !selecting && (
          <Chips
            items={[
              { key: 'recent', label: 'Recent' },
              { key: 'starred', label: 'Starred' },
            ]}
            active={recentMode}
            onChoose={(key) => setRecentMode(key as 'recent' | 'starred')}
          />
        )}

        <ConnectionLine model={model} />

        <main className="relative min-h-0 flex-1">
          {tab === 'files' && (
            <FilesScreen
              model={model}
              entries={entries}
              selecting={selecting}
              onActions={setMenuFor}
              scrollKey={model.listKey}
              emptyLabel={
                isWaiting(model.vault.status)
                  ? waitingLabel(model.vault.status)
                  : query
                    ? `Nothing matches “${query}”`
                    : 'This folder is empty'
              }
            />
          )}
          {tab === 'recent' && (
            <FilesScreen
              model={model}
              entries={entries}
              selecting={selecting}
              onActions={setMenuFor}
              scrollKey={model.listKey}
              showPath
              emptyLabel={
                recentMode === 'starred'
                  ? 'Nothing starred yet. Use a file’s ⋮ menu to star it.'
                  : 'Nothing here yet'
              }
            />
          )}
          {tab === 'library' && <LibraryScreen model={model} section={librarySection} wide={wide} />}
          {tab === 'more' && (
            <MoreScreen model={model} onTransfers={() => setTransfersOpen(true)} onChangeDrive={onChangeDrive} />
          )}

          {tab === 'files' && writable && !selecting && (
            <Fab onClick={() => setAdding(true)} raised={active.length > 0 || shared.length > 0 || actions.clipboard !== null} />
          )}
        </main>

        <Rise show={tab === 'files' && actions.clipboard !== null && !selecting}>
          <PasteBar model={model} />
        </Rise>
        <Rise show={shared.length > 0}>
            <SharedBar
              files={shared}
              into={vault.dir}
              vaultName={vault.status?.vault ?? 'the drive'}
              onUpload={() => {
                const files = shared
                setShared([])
                const label = files.length === 1 ? files[0]!.name : `${files.length} files`
                void model.uploadFromPhone(files, [], vault.dir, label)
                setTransfersOpen(true)
              }}
              onDismiss={() => setShared([])}
            />
        </Rise>
        <Rise show={offered(update) && tab !== 'more' && bannerGone !== update.release.version}>
          {offered(update) && (
            <UpdateStrip
              update={update}
              onOpen={showUpdate}
              onDismiss={() => {
                remember(BANNER_DISMISSED, update.release.version)
                setBannerGone(update.release.version)
              }}
            />
          )}
        </Rise>
        <Rise show={active.length > 0}>
          <TransferStrip model={model} onOpen={() => setTransfersOpen(true)} />
        </Rise>

        {!wide && <BottomNav tab={tab} onTab={goTo} activeTransfers={active.length} attention={offered(update)} />}
        {wide && <div style={{ height: 'var(--inset-bottom, 0px)' }} className="shrink-0" />}
      </div>

      <Rise
        show={notice !== null}
        onClick={() => setNotice(null)}
        className="fixed inset-x-4 z-[70] mx-auto max-w-[480px] rounded-xl border border-white/[0.1] bg-[#1c1c1f] px-4 py-3 text-[13.5px] leading-snug text-text shadow-lift"
        style={{ bottom: `calc(var(--inset-bottom, 0px) + ${wide ? 20 : 84}px)` }}
      >
        {notice}
      </Rise>

      <ActionSheet
        open={menuFor !== null}
        onClose={() => setMenuFor(null)}
        title={menuFor?.name}
        subtitle={menuFor ? (menuFor.kind === 'dir' ? 'Folder' : formatBytes(menuFor.size)) : undefined}
        actions={menuFor ? model.entryActions(menuFor) : []}
      />

      <AddSheet
        open={adding}
        onClose={() => setAdding(false)}
        onMedia={() => void model.uploadPicked('media', vault.dir)}
        onFiles={() => void model.uploadPicked('any', vault.dir)}
        onFolder={() => void model.uploadPickedFolder(vault.dir)}
        onNewFolder={model.askNewFolder}
      />

      <TransfersSheet model={model} open={transfersOpen} onClose={() => setTransfersOpen(false)} />

      <MobilePlayer model={model} />

      <ImageViewer
        photos={viewer?.photos ?? []}
        index={model.viewingIndex}
        base={model.mediaBase}
        onIndexChange={(index) => model.setViewer((v) => (v ? { ...v, index } : v))}
        onClose={() => model.setViewer(null)}
        onDownload={(path) => {
          const photo = viewer?.photos.find((p) => p.path === path)
          if (photo) void model.downloadOne(fileToEntry(photo))
        }}
      />
      <BackCloses open={viewer !== null} onBack={() => model.setViewer(null)} />

      {/* What the menu's Properties shows: the same details as the desktop's
          side panel, in a sheet. */}
      <Sheet
        open={model.properties !== null}
        onClose={() => model.setProperties(null)}
        title="Properties"
      >
        {model.properties && (
          <div className="px-5 pb-4 pt-2">
            <PropertiesDetails
              entry={model.properties}
              vaultName={model.vault.status?.vault ?? 'the drive'}
              large
            />
          </div>
        )}
      </Sheet>

      <PromptDialog request={model.prompt} onClose={() => model.setPrompt(null)} />
      {model.confirmDialog}
    </div>
  )
}

/** Back closes something that registers no back handling of its own. */
function BackCloses({ open, onBack }: { open: boolean; onBack: () => void }): null {
  useBack(open, () => {
    onBack()
    return true
  })
  return null
}

// ---------------------------------------------------------------------------
// Bars
// ---------------------------------------------------------------------------

const TABS: Array<{ key: Tab; label: string; icon: typeof Film }> = [
  { key: 'files', label: 'Files', icon: FolderOpen },
  { key: 'library', label: 'Library', icon: Library },
  { key: 'recent', label: 'Recent', icon: Clock },
  { key: 'more', label: 'More', icon: MoreHorizontal },
]

function BottomNav({
  tab,
  onTab,
  activeTransfers,
  attention = false,
}: {
  tab: Tab
  onTab: (tab: Tab) => void
  activeTransfers: number
  /** Something on More wants a look: an update. */
  attention?: boolean
}): React.JSX.Element {
  return (
    <nav
      className="shrink-0 border-t border-white/[0.06] bg-[#0e0e10]"
      style={{ paddingBottom: 'var(--inset-bottom, 0px)' }}
    >
      <div className="mx-auto flex h-16 max-w-[560px]">
        {TABS.map(({ key, label, icon: Icon }) => {
          const on = tab === key
          return (
            <button
              key={key}
              onClick={() => onTab(key)}
              className="relative flex flex-1 flex-col items-center justify-center gap-1"
              aria-current={on ? 'page' : undefined}
            >
              <span
                className={cn(
                  'flex h-8 w-16 items-center justify-center rounded-full transition-colors duration-200',
                  on ? 'bg-white/[0.1]' : 'bg-transparent',
                )}
              >
                <Icon size={21} className={on ? 'text-text' : 'text-textFaint'} />
              </span>
              <span className={cn('text-[11.5px]', on ? 'font-medium text-text' : 'text-textFaint')}>
                {label}
              </span>
              {key === 'more' && (activeTransfers > 0 || attention) && (
                <span className="absolute right-[calc(50%-22px)] top-2.5 h-2 w-2 rounded-full bg-basalt" />
              )}
            </button>
          )
        })}
      </div>
    </nav>
  )
}

function Rail({
  tab,
  onTab,
  activeTransfers,
  attention = false,
}: {
  tab: Tab
  onTab: (tab: Tab) => void
  activeTransfers: number
  attention?: boolean
}): React.JSX.Element {
  return (
    <nav
      className="flex w-[88px] shrink-0 flex-col items-center gap-3 border-r border-white/[0.06] bg-[#0e0e10]"
      style={{ paddingTop: 'calc(var(--inset-top, 0px) + 16px)' }}
    >
      <HexMark size={26} className="mb-4 text-basalt" />
      {TABS.map(({ key, label, icon: Icon }) => {
        const on = tab === key
        return (
          <button
            key={key}
            onClick={() => onTab(key)}
            className="relative flex flex-col items-center gap-1"
            aria-current={on ? 'page' : undefined}
          >
            <span
              className={cn(
                'flex h-8 w-14 items-center justify-center rounded-full transition-colors duration-200',
                on ? 'bg-white/[0.1]' : 'bg-transparent',
              )}
            >
              <Icon size={21} className={on ? 'text-text' : 'text-textFaint'} />
            </span>
            <span className={cn('text-[11.5px]', on ? 'font-medium text-text' : 'text-textFaint')}>
              {label}
            </span>
            {key === 'more' && (activeTransfers > 0 || attention) && (
              <span className="absolute right-3 top-1 h-2 w-2 rounded-full bg-basalt" />
            )}
          </button>
        )
      })}
    </nav>
  )
}

function TopBar({
  model,
  tab,
  searching,
  onSearch,
  onCloseSearch,
  query,
  onQuery,
}: {
  model: AppModel
  tab: Tab
  searching: boolean
  onSearch: () => void
  onCloseSearch: () => void
  query: string
  onQuery: (query: string) => void
}): React.JSX.Element {
  const { vault } = model
  const inFolder = tab === 'files' && vault.dir !== ''
  const title =
    tab === 'files'
      ? inFolder
        ? vault.dir.split('/').pop()!
        : (vault.status?.vault ?? 'Basalt')
      : tab === 'library'
        ? 'Library'
        : tab === 'recent'
          ? 'Recent'
          : 'More'

  if (searching) {
    return (
      <div className="flex h-14 shrink-0 items-center gap-1 px-2">
        <IconButton label="Close search" onClick={onCloseSearch}>
          <ArrowLeft size={21} />
        </IconButton>
        <input
          autoFocus
          value={query}
          onChange={(e) => onQuery(e.target.value)}
          placeholder={tab === 'library' ? 'Search the library' : 'Search this folder'}
          enterKeyHint="search"
          className="min-w-0 flex-1 bg-transparent px-2 text-[16px] text-text placeholder:text-textFaint"
        />
        {query && (
          <IconButton label="Clear" onClick={() => onQuery('')}>
            <X size={19} />
          </IconButton>
        )}
      </div>
    )
  }

  return (
    <div className="flex h-14 shrink-0 items-center gap-1 px-2">
      {inFolder ? (
        <IconButton label="Up a folder" onClick={() => vault.open(parentOf(vault.dir))}>
          <ArrowLeft size={21} />
        </IconButton>
      ) : (
        <span className="w-2" />
      )}
      <h1 className="min-w-0 flex-1 truncate px-1 text-[20px] font-semibold tracking-tight text-text">
        {title}
      </h1>
      {tab !== 'more' && (
        <IconButton label="Search" onClick={onSearch}>
          <Search size={20} />
        </IconButton>
      )}
    </div>
  )
}

function SelectionBar({
  count,
  onClose,
  onSelectAll,
  onDownload,
  onCut,
  onCopy,
  onDelete,
  onMore,
}: {
  count: number
  onClose: () => void
  onSelectAll: () => void
  onDownload: () => void
  onCut?: () => void
  onCopy: () => void
  onDelete?: () => void
  onMore: () => void
}): React.JSX.Element {
  return (
    <div className="flex h-14 shrink-0 items-center gap-0.5 bg-[#18181b] px-2">
      <IconButton label="Stop choosing" onClick={onClose}>
        <X size={21} />
      </IconButton>
      <span className="min-w-0 flex-1 px-1 text-[17px] font-medium text-text">{count}</span>
      <IconButton label="Choose all" onClick={onSelectAll}>
        <CheckSquare size={20} />
      </IconButton>
      <IconButton label="Download" onClick={onDownload}>
        <Download size={20} />
      </IconButton>
      {onCut && (
        <IconButton label="Move" onClick={onCut}>
          <Scissors size={20} />
        </IconButton>
      )}
      <IconButton label="Copy" onClick={onCopy}>
        <Copy size={20} />
      </IconButton>
      {onDelete && (
        <IconButton label="Delete" onClick={onDelete}>
          <Trash2 size={20} />
        </IconButton>
      )}
      {count === 1 && (
        <IconButton label="More" onClick={onMore}>
          <MoreHorizontal size={20} />
        </IconButton>
      )}
    </div>
  )
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string
  onClick: () => void
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <button
      aria-label={label}
      title={label}
      onClick={onClick}
      className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-textDim transition-colors active:bg-white/[0.08] active:text-text"
    >
      {children}
    </button>
  )
}

/** Where in the drive this folder is, and a tap back to any part of it. */
function Crumbs({ model }: { model: AppModel }): React.JSX.Element | null {
  const { vault } = model
  if (!vault.dir) return null
  const parts = vault.dir.split('/')
  return (
    <div className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto px-4 [scrollbar-width:none]">
      <button
        onClick={() => vault.open('')}
        className="shrink-0 rounded-full px-2.5 py-1 text-[12.5px] text-textFaint active:bg-white/[0.06]"
      >
        {vault.status?.vault ?? 'Drive'}
      </button>
      {parts.map((part, i) => (
        <span key={i} className="flex shrink-0 items-center gap-1">
          <span className="text-textFaint">/</span>
          <button
            onClick={() => vault.open(parts.slice(0, i + 1).join('/'))}
            className={cn(
              'rounded-full px-2.5 py-1 text-[12.5px] active:bg-white/[0.06]',
              i === parts.length - 1 ? 'text-text' : 'text-textFaint',
            )}
          >
            {part}
          </button>
        </span>
      ))}
    </div>
  )
}

function Chips({
  items,
  active,
  onChoose,
}: {
  items: Array<{ key: string; label: string }>
  active: string
  onChoose: (key: string) => void
}): React.JSX.Element {
  return (
    <div className="flex h-12 shrink-0 items-center gap-2 overflow-x-auto px-4 [scrollbar-width:none]">
      {items.map((item) => {
        const on = item.key === active
        return (
          <button
            key={item.key}
            onClick={() => {
              void android.haptic('tap')
              onChoose(item.key)
            }}
            className={cn(
              'shrink-0 rounded-full border px-4 py-1.5 text-[13.5px] transition-colors duration-150',
              on
                ? 'border-transparent bg-basalt font-medium text-ink'
                : 'border-white/[0.12] text-textDim active:bg-white/[0.06]',
            )}
          >
            {item.label}
          </button>
        )
      })}
    </div>
  )
}

/** Lost the host, or the drive: said plainly, above the list. */
function ConnectionLine({ model }: { model: AppModel }): React.JSX.Element | null {
  const kind = model.vault.error?.kind
  if (!kind) return null
  const offline = kind === 'offline'
  return (
    <div className="flex shrink-0 items-center gap-2.5 border-y border-white/[0.06] bg-[#141416] px-4 py-2.5">
      <Loader2 size={14} className="shrink-0 animate-spin text-textFaint" />
      <span className="min-w-0 flex-1 text-[12.5px] leading-snug text-textDim">
        {offline
          ? model.vault.reconnecting
            ? 'Reconnecting…'
            : 'Can’t reach the host. Trying again.'
          : (model.vault.error?.message ?? 'Something went wrong')}
      </span>
      <button
        onClick={() => (offline ? void model.vault.reconnect() : model.vault.refresh())}
        className="shrink-0 rounded-full px-3 py-1 text-[12px] text-text active:bg-white/[0.08]"
      >
        Retry
      </button>
    </div>
  )
}

function Fab({ onClick, raised }: { onClick: () => void; raised: boolean }): React.JSX.Element {
  return (
    // Lifted by transform on a wrapper, so its own press shrink still works.
    <div
      className="rise absolute bottom-5 right-5 z-20"
      style={{ transform: raised ? 'translate3d(0, -64px, 0)' : 'translate3d(0, 0, 0)' }}
    >
      <button
        aria-label="Add"
        onClick={() => {
          void android.haptic('tap')
          onClick()
        }}
        className="flex h-14 w-14 items-center justify-center rounded-2xl bg-basalt text-ink shadow-lift transition-transform duration-100 active:scale-95"
      >
        <Plus size={26} />
      </button>
    </div>
  )
}

function AddSheet({
  open,
  onClose,
  onMedia,
  onFiles,
  onFolder,
  onNewFolder,
}: {
  open: boolean
  onClose: () => void
  onMedia: () => void
  onFiles: () => void
  onFolder: () => void
  onNewFolder: () => void
}): React.JSX.Element {
  const items = [
    { id: 'media', label: 'Photos and videos', icon: Images, run: onMedia },
    { id: 'files', label: 'Files', icon: FilePlus2, run: onFiles },
    { id: 'folder', label: 'A whole folder', icon: Upload, run: onFolder },
    { id: 'new', label: 'New folder', icon: FolderPlus, run: onNewFolder, separatorBefore: true },
  ]
  return <ActionSheet open={open} onClose={onClose} title="Add to this folder" actions={items} />
}

function PasteBar({ model }: { model: AppModel }): React.JSX.Element {
  const clip = model.actions.clipboard!
  const n = clip.paths.length
  const what = n === 1 ? (clip.paths[0]!.split('/').pop() ?? '1 item') : `${n} items`
  return (
    <div
      className="mx-3 mb-2 flex items-center gap-2 rounded-2xl border border-white/[0.1] bg-[#1c1c1f] py-2 pl-4 pr-2 shadow-lift"
    >
      <span className="min-w-0 flex-1 truncate text-[13.5px] text-text">
        {clip.mode === 'cut' ? 'Move' : 'Copy'} {what} here?
      </span>
      <button
        onClick={() => model.actions.clearClipboard()}
        className="rounded-full px-3 py-2 text-[13px] text-textDim active:bg-white/[0.08]"
      >
        Cancel
      </button>
      <button
        onClick={() => void model.actions.paste(model.vault.dir)}
        className="rounded-full bg-basalt px-4 py-2 text-[13px] font-medium text-ink"
      >
        {clip.mode === 'cut' ? 'Move here' : 'Paste'}
      </button>
    </div>
  )
}

function SharedBar({
  files,
  into,
  vaultName,
  onUpload,
  onDismiss,
}: {
  files: PhoneFile[]
  into: string
  vaultName: string
  onUpload: () => void
  onDismiss: () => void
}): React.JSX.Element {
  const where = into ? into.split('/').pop() : vaultName
  const what = files.length === 1 ? files[0]!.name : `${files.length} files`
  return (
    <div
      className="mx-3 mb-2 rounded-2xl border border-white/[0.1] bg-[#1c1c1f] p-3 shadow-lift"
    >
      <div className="flex items-start gap-3">
        <Upload size={18} className="mt-0.5 shrink-0 text-textDim" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] text-text">{what}</div>
          <div className="mt-0.5 text-[12px] leading-snug text-textFaint">
            Shared from another app. Open the folder it belongs in, then upload.
          </div>
        </div>
      </div>
      <div className="mt-2.5 flex justify-end gap-2">
        <button
          onClick={onDismiss}
          className="rounded-full px-3 py-2 text-[13px] text-textDim active:bg-white/[0.08]"
        >
          Not now
        </button>
        <button onClick={onUpload} className="rounded-full bg-basalt px-4 py-2 text-[13px] font-medium text-ink">
          Upload to {where}
        </button>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

const BANNER_DISMISSED = 'basalt.update-banner-dismissed'
const NOTIFIED = 'basalt.update-notified'
const ASKED_NOTIFICATIONS = 'basalt.asked-notifications'

function remembered(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function remember(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    // Private storage unavailable: the reminder simply comes back.
  }
}

/** "Basalt 1.4.2 is available", over the other tabs, in the transfer strip's place. */
function UpdateStrip({
  update,
  onOpen,
  onDismiss,
}: {
  update: Extract<UpdateState, { release: unknown }>
  onOpen: () => void
  onDismiss: () => void
}): React.JSX.Element {
  const line =
    update.kind === 'ready'
      ? 'Ready to install'
      : update.kind === 'downloading'
        ? `Downloading… ${update.total > 0 ? Math.round((update.had / update.total) * 100) : 0}%`
        : 'Tap to see what’s new'
  return (
    <div className="relative mx-3 mb-2 flex items-center gap-3 overflow-hidden rounded-2xl border border-basalt/30 bg-[#1c1c1f] py-3 pl-4 pr-2 shadow-lift">
      <button onClick={onOpen} className="flex min-w-0 flex-1 items-center gap-3 text-left">
        <ArrowUpCircle size={19} className="shrink-0 text-basalt" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13.5px] text-text">
            Basalt {update.release.version} is available
          </span>
          <span className="block truncate text-[12px] text-textFaint">{line}</span>
        </span>
      </button>
      <button
        onClick={onDismiss}
        aria-label="Dismiss"
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-textFaint active:bg-white/[0.08]"
      >
        <X size={17} />
      </button>
    </div>
  )
}

/**
 * The notification outside the app, once per version.
 *
 * Android 13 and later ask before an app may notify; the question is put the
 * first time there is an update to tell about, and not again. Remembered
 * either way, so a refusal is not met with a question on every launch.
 */
function useUpdateNotification(update: UpdateState): void {
  const version = update.kind === 'available' ? update.release.version : null
  useEffect(() => {
    if (!version || !isAndroid() || remembered(NOTIFIED) === version) return
    remember(NOTIFIED, version)
    void (async () => {
      if (!remembered(ASKED_NOTIFICATIONS)) {
        remember(ASKED_NOTIFICATIONS, '1')
        await android.requestNotifications().catch(() => null)
      }
      await android.notifyUpdate(version).catch(() => false)
    })()
  }, [version])
}

/** A tap on the update notification opens the update, whether the app was running or not. */
function useNotificationTaps(showUpdate: () => void): void {
  useEffect(() => {
    if (!isAndroid()) return undefined
    const take = async (): Promise<void> => {
      if ((await android.takeAction().catch(() => null)) === 'update') showUpdate()
    }
    void take()
    const onVisible = (): void => {
      if (document.visibilityState === 'visible') void take()
    }
    document.addEventListener('visibilitychange', onVisible)
    let stop: (() => void) | undefined
    void (async () => {
      try {
        const { addPluginListener } = await import('@tauri-apps/api/core')
        const listener = await addPluginListener('basalt-android', 'action', () => void take())
        stop = () => void listener.unregister()
      } catch {
        // Not on Android.
      }
    })()
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      stop?.()
    }
  }, [showUpdate])
}

function TransferStrip({ model, onOpen }: { model: AppModel; onOpen: () => void }): React.JSX.Element {
  const active = model.transfers.active
  const total = active.reduce((sum, t) => sum + t.total, 0)
  const done = active.reduce((sum, t) => sum + t.transferred, 0)
  const fraction = total > 0 ? done / total : 0
  const rate = active.reduce((sum, t) => sum + t.rate, 0)
  const uploads = active.filter((t) => t.kind === 'upload').length
  const label =
    active.length === 1
      ? active[0]!.name
      : `${uploads > 0 ? (uploads === active.length ? 'Uploading' : 'Moving') : 'Downloading'} ${active.length} files`
  return (
    <button
      onClick={onOpen}
      className="relative mx-3 mb-2 block w-[calc(100%-1.5rem)] overflow-hidden rounded-2xl border border-white/[0.1] bg-[#1c1c1f] px-4 py-3 text-left shadow-lift"
    >
      <div className="flex items-center gap-3">
        {active[0]?.kind === 'upload' ? (
          <Upload size={17} className="shrink-0 text-textDim" />
        ) : (
          <Download size={17} className="shrink-0 text-textDim" />
        )}
        <span className="min-w-0 flex-1 truncate text-[13.5px] text-text">{label}</span>
        <span className="tnum shrink-0 font-mono text-[11.5px] text-textDim">
          {Math.round(fraction * 100)}% · {formatBytes(rate)}/s
        </span>
      </div>
      <div className="absolute inset-x-0 bottom-0 h-[3px] bg-white/[0.06]">
        <div className="h-full bg-basalt transition-[width] duration-300" style={{ width: `${fraction * 100}%` }} />
      </div>
    </button>
  )
}

function TransfersSheet({
  model,
  open,
  onClose,
}: {
  model: AppModel
  open: boolean
  onClose: () => void
}): React.JSX.Element {
  const { transfers, savedOnPhone } = model
  return (
    <Sheet open={open} onClose={onClose} title="Transfers" tall>
      {transfers.transfers.length === 0 ? (
        <p className="px-5 pb-6 pt-2 text-[13.5px] text-textFaint">Nothing moving, and nothing moved yet.</p>
      ) : (
        <div className="pb-2">
          {transfers.transfers.map((t) => {
            const saved = savedOnPhone[t.id]
            const fraction = t.total > 0 ? t.transferred / t.total : 0
            return (
              <div key={t.id} className="border-b border-white/[0.05] px-5 py-3.5 last:border-0">
                <div className="flex items-center gap-3">
                  {t.kind === 'upload' ? (
                    <Upload size={17} className="shrink-0 text-textDim" />
                  ) : (
                    <Download size={17} className="shrink-0 text-textDim" />
                  )}
                  <span className="min-w-0 flex-1 truncate text-[14px] text-text">{t.name}</span>
                  {t.status === 'active' && (
                    <button
                      onClick={() => transfers.cancel(t.id)}
                      className="shrink-0 rounded-full px-2.5 py-1 text-[12.5px] text-textDim active:bg-white/[0.08]"
                    >
                      Stop
                    </button>
                  )}
                </div>
                {t.status === 'active' ? (
                  <>
                    <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-white/[0.08]">
                      <div className="h-full rounded-full bg-basalt transition-[width] duration-300" style={{ width: `${fraction * 100}%` }} />
                    </div>
                    <div className="tnum mt-1.5 font-mono text-[11px] text-textFaint">
                      {formatBytes(t.transferred)} of {formatBytes(t.total)} · {formatBytes(t.rate)}/s
                    </div>
                  </>
                ) : (
                  <div
                    className={cn(
                      'mt-1 text-[12px] leading-snug',
                      t.status === 'failed' ? 'text-danger' : 'text-textFaint',
                    )}
                  >
                    {t.status === 'failed'
                      ? (t.error ?? 'Did not finish')
                      : t.status === 'cancelled'
                        ? 'Stopped'
                        : saved
                          ? `Saved to ${saved.shownAs}`
                          : 'Done'}
                  </div>
                )}
                {saved && t.status === 'done' && (
                  <div className="mt-2 flex gap-2">
                    <button
                      onClick={() => void android.openDownload(saved.uri)}
                      className="rounded-full border border-white/[0.12] px-3.5 py-1.5 text-[12.5px] text-text active:bg-white/[0.06]"
                    >
                      Open
                    </button>
                    <button
                      onClick={() => void android.shareDownload(saved.uri)}
                      className="rounded-full border border-white/[0.12] px-3.5 py-1.5 text-[12.5px] text-text active:bg-white/[0.06]"
                    >
                      Share
                    </button>
                  </div>
                )}
              </div>
            )
          })}
          <button
            onClick={transfers.clearDone}
            className="mx-5 mt-2 rounded-full px-3 py-2 text-[13px] text-textDim active:bg-white/[0.08]"
          >
            Clear finished
          </button>
        </div>
      )}
    </Sheet>
  )
}

/**
 * Keeps the app alive while anything is moving, with a notification that
 * says what and how far — and lets go the moment nothing is.
 */
function useKeepAliveForTransfers(model: AppModel): void {
  const active = model.transfers.active
  const total = active.reduce((sum, t) => sum + t.total, 0)
  const done = active.reduce((sum, t) => sum + t.transferred, 0)
  const percent = total > 0 ? Math.floor((done / total) * 100) : -1
  const count = active.length
  const first = active[0]?.name ?? ''
  const [asked, setAsked] = useState(false)

  useEffect(() => {
    if (count === 0) {
      void android.letGo('transfer')
      return
    }
    if (!asked) {
      setAsked(true)
      void android.requestNotifications()
    }
    const title = count === 1 ? first : `${count} transfers`
    void android.keepAlive('transfer', title, percent >= 0 ? `${percent}%` : 'Starting…', percent)
    // Percent, not bytes: the notification is redrawn once per point.
  }, [count, first, percent, asked])
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

function LibraryScreen({
  model,
  section,
  wide,
}: {
  model: AppModel
  section: LibrarySection
  wide: boolean
}): React.JSX.Element {
  const {
    media,
    mediaItems,
    watchedByPath,
    watched,
    playing,
    playPath,
    libraryFiles,
    libraryScanning,
    mediaBase,
    setViewer,
    setPlaying,
    query,
  } = model

  if (section === 'movies' || section === 'series') {
    return (
      // LibraryView scrolls itself. Wrapped in a second scroller with its own
      // bottom padding, the inner one ended short of the screen and cut the
      // posters off above an empty band.
      <div className="h-full">
        <LibraryView
          kind={section === 'movies' ? 'film' : 'series'}
          items={mediaItems}
          enabled={media.enabled}
          scanning={media.scanning}
          watched={watchedByPath}
          continueWatching={watched.continueWatching}
          playing={playing !== null}
          onPlay={(path) => void playPath(path)}
          onForget={watched.forget}
        />
      </div>
    )
  }

  if (libraryFiles.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-10 text-center text-[14px] text-textFaint">
        {libraryScanning
          ? 'Looking through the drive…'
          : query
            ? `Nothing matches “${query}”`
            : `No ${section} on the drive yet`}
      </div>
    )
  }

  if (section === 'photos') {
    return (
      <PhotoGrid
        files={libraryFiles}
        base={mediaBase}
        onOpen={(index) => setViewer({ photos: libraryFiles, index })}
      />
    )
  }
  if (section === 'music') {
    return (
      <MusicList
        files={libraryFiles}
        playing={playing?.id ?? null}
        compact={!wide}
        onPlay={(file) => setPlaying(musicItem(file))}
      />
    )
  }
  return (
    <VideoGrid
      files={libraryFiles}
      base={mediaBase}
      progressOf={(path) => {
        const entry = watchedByPath.get(path)
        return entry && entry.duration > 0 && !isFinished(entry)
          ? entry.position / entry.duration
          : undefined
      }}
      onPlay={(file) => void playPath(file.path)}
    />
  )
}

// ---------------------------------------------------------------------------
// More
// ---------------------------------------------------------------------------

function MoreScreen({
  model,
  onTransfers,
  onChangeDrive,
}: {
  model: AppModel
  onTransfers: () => void
  /** The drive list, to use another drive; this one stays paired. */
  onChangeDrive: () => void
}): React.JSX.Element {
  const showHidden = useShowHidden()
  const { vault, identity, transfers } = model
  const profile = identity.state?.profile ?? null
  const space = vault.space
  const used = space ? space[1] - space[0] : 0
  const total = space ? space[1] : 0

  const signOut = async (): Promise<void> => {
    await api.signOutProfile()
    await identity.reloadProfiles()
    await identity.refresh()
  }

  return (
    <div className="h-full overflow-y-auto px-4 pb-10">
      <div className="mx-auto max-w-[560px] space-y-3 pt-1">
        {identity.supported && (
          <Card>
            <div className="flex items-center gap-3.5">
              {profile ? (
                <span
                  className="flex h-12 w-12 shrink-0 items-center justify-center rounded-full text-[19px] font-semibold text-white"
                  style={{ background: profileColor(profile.color) }}
                >
                  {profile.name.slice(0, 1).toUpperCase()}
                </span>
              ) : (
                <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-full bg-white/[0.06]">
                  <HexMark size={22} className="text-textDim" />
                </span>
              )}
              <div className="min-w-0 flex-1">
                <div className="truncate text-[16px] font-medium text-text">
                  {profile ? profile.name : 'This device'}
                </div>
                <div className="truncate text-[12.5px] text-textFaint">
                  {profile ? 'Your history and stars follow you' : 'Not signed in to a profile'}
                </div>
              </div>
            </div>
            <div className="mt-3.5 flex gap-2">
              {profile ? (
                <>
                  <Pill onClick={() => void signOut()}>Switch profile</Pill>
                  <Pill onClick={() => void signOut()}>Sign out</Pill>
                </>
              ) : (
                <Pill
                  onClick={() => {
                    void identity.reloadProfiles()
                    model.setSigningIn(true)
                  }}
                >
                  Sign in to a profile
                </Pill>
              )}
            </div>
          </Card>
        )}

        <Card>
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="truncate text-[16px] font-medium text-text">
                {vault.status?.vault ?? 'Drive'}
              </div>
              <div className="truncate text-[12.5px] text-textFaint">
                {vault.status?.hostName ?? 'Host'} · {model.connected ? 'connected' : 'not connected'}
              </div>
            </div>
            <span
              className={cn(
                'h-2.5 w-2.5 shrink-0 rounded-full',
                model.connected ? 'bg-emerald-400/80' : 'bg-white/20',
              )}
            />
          </div>
          {total > 0 && (
            <>
              <div className="mt-3.5 h-1.5 overflow-hidden rounded-full bg-white/[0.07]">
                <div className="h-full rounded-full bg-basaltDeep" style={{ width: `${(used / total) * 100}%` }} />
              </div>
              <div className="tnum mt-2 font-mono text-[11.5px] text-textFaint">
                {formatBytes(total - used)} free of {formatBytes(total)}
              </div>
            </>
          )}
          <button
            onClick={onChangeDrive}
            className="mt-3.5 flex w-full items-center justify-center gap-2 rounded-xl border border-white/[0.08] py-2.5 text-[13.5px] text-textDim active:bg-white/[0.06]"
          >
            <ArrowLeftRight size={15} />
            Change drive
          </button>
        </Card>

        <Card>
          <Switch
            label="Show hidden files"
            description="Items Windows keeps out of sight on the host, such as desktop.ini and the Recycle Bin."
            checked={showHidden}
            onChange={setShowHidden}
            className="py-0"
          />
        </Card>

        <Card onClick={onTransfers}>
          <div className="flex items-center gap-3.5">
            <Download size={20} className="shrink-0 text-textDim" />
            <div className="min-w-0 flex-1">
              <div className="text-[15px] text-text">Transfers</div>
              <div className="text-[12.5px] text-textFaint">
                {transfers.active.length > 0
                  ? `${transfers.active.length} under way`
                  : 'Downloads are saved to Download/Basalt'}
              </div>
            </div>
          </div>
        </Card>

        <Card onClick={() => model.setNav('starred')}>
          <div className="flex items-center gap-3.5">
            <Star size={20} className="shrink-0 text-textDim" />
            <div className="text-[15px] text-text">Starred files are under Recent</div>
          </div>
        </Card>

        <div id="basalt-update" className="scroll-mt-4 rounded-2xl border border-white/[0.07] bg-[#141416] p-4">
          <About product="Basalt" />
        </div>

        <button
          onClick={() => void model.forgetVault()}
          className="w-full rounded-2xl border border-danger/25 px-4 py-3.5 text-left text-[14.5px] text-danger active:bg-danger/10"
        >
          Forget this drive
          <div className="mt-0.5 text-[12px] text-textFaint">
            This phone will have to pair again. Nothing on the drive is touched.
          </div>
        </button>
      </div>
    </div>
  )
}

function Card({
  children,
  onClick,
}: {
  children: React.ReactNode
  onClick?: () => void
}): React.JSX.Element {
  const Tag = onClick ? 'button' : 'div'
  return (
    <Tag
      onClick={onClick}
      className={cn(
        'block w-full rounded-2xl border border-white/[0.07] bg-[#141416] p-4 text-left',
        onClick && 'active:bg-white/[0.04]',
      )}
    >
      {children}
    </Tag>
  )
}

function Pill({ children, onClick }: { children: React.ReactNode; onClick: () => void }): React.JSX.Element {
  return (
    <button
      onClick={onClick}
      className="rounded-full border border-white/[0.12] px-3.5 py-2 text-[13px] text-text active:bg-white/[0.06]"
    >
      {children}
    </button>
  )
}

// ---------------------------------------------------------------------------
// Playing
// ---------------------------------------------------------------------------

/** The player, as the desktop has it, closed by back as well as its button. */
function MobilePlayer({ model }: { model: AppModel }): React.JSX.Element {
  const {
    playing,
    setPlaying,
    watched,
    openExternally,
    resumeFor,
    nextAfter,
    previousByPath,
    playPath,
    subtitlesFor,
  } = model
  const close = useCallback(() => {
    setPlaying(null)
    setTimeout(watched.refresh, 300)
  }, [setPlaying, watched.refresh])

  useBack(playing !== null, () => {
    close()
    return true
  })

  // Playing keeps the app alive with the screen off: music between tracks,
  // a film paused under a locked screen.
  useEffect(() => {
    if (!playing) {
      void android.letGo('playback')
      return
    }
    void android.keepAlive('playback', playing.title, playing.subtitle || 'Playing')
  }, [playing])

  const previous = useMemo(
    () => (playing ? (previousByPath.get(playing.id) ?? null) : null),
    [playing, previousByPath],
  )

  return (
    <PlayerOverlay
      item={playing}
      onClose={close}
      onOpenExternally={(path) => void openExternally(path)}
      resumeAt={playing ? resumeFor(playing.id) : 0}
      onProgress={watched.report}
      nextUp={playing ? nextAfter(playing.id) : null}
      previous={previous}
      onPlayNext={(path) => void playPath(path)}
      subtitles={playing ? subtitlesFor(playing.id) : []}
    />
  )
}
