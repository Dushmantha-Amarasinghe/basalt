import { useEffect, useMemo, useRef, useState } from 'react'
import { VList } from 'virtua'
import { Play, Volume2 } from 'lucide-react'
import type { MediaFile } from '@/lib/api'
import { justify } from '@/lib/justify'
import { folderOf, formatOf, groupByMonth, stemOf, trackInfo } from '@/lib/mediaInfo'
import { thumbUrl } from '@/lib/thumbs'
import { cn, formatBytes } from '@/lib/utils'

/**
 * Videos, Photos and Music, each drawn as what it is.
 *
 * They were one grid of identical poster-shaped placeholders — photos with a
 * play button, songs as film posters. Each now has its own shape: videos as
 * wide frames with a picture from inside them, photos at their own proportions
 * in rows like a photo library, music as a list of tracks.
 *
 * All three are virtualised, the same way the file list is, so a drive with
 * forty thousand photos scrolls as smoothly as one with forty. Nothing inside
 * a tile animates with JavaScript: a tile fades its picture in when it loads
 * and lifts a little on hover, both in CSS, which is what keeps a fast scroll
 * at full frame rate.
 */

/** The width of an element, kept current as the window changes. */
function useWidth(): [React.RefObject<HTMLDivElement | null>, number] {
  const ref = useRef<HTMLDivElement | null>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const el = ref.current
    if (!el) return undefined
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(Math.floor(entry.contentRect.width))
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  return [ref, width]
}

/**
 * A picture from the host, over a placeholder that is there from the start.
 *
 * The placeholder is not a loading state to be replaced: it is what the tile
 * is until a picture arrives, and what it stays if none does — a video the
 * host cannot picture, or a host without the means to.
 */
export function Thumb({
  src,
  alt,
  className,
  children,
}: {
  src: string
  alt: string
  className?: string
  children?: React.ReactNode
}): React.JSX.Element {
  const [loaded, setLoaded] = useState(false)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    setLoaded(false)
    setFailed(false)
  }, [src])

  return (
    <div className={cn('relative overflow-hidden bg-[#141416]', className)}>
      {!loaded && <HexMark />}
      {src && !failed && (
        <img
          src={src}
          alt={alt}
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={() => setLoaded(true)}
          onError={() => setFailed(true)}
          className={cn(
            'absolute inset-0 h-full w-full object-cover transition-[opacity,transform] duration-300 ease-out group-hover:scale-[1.03]',
            loaded ? 'opacity-100' : 'opacity-0',
          )}
        />
      )}
      {children}
    </div>
  )
}

function HexMark(): React.JSX.Element {
  return (
    <svg
      viewBox="0 0 100 100"
      className="absolute inset-0 h-full w-full opacity-[0.05]"
      aria-hidden="true"
      preserveAspectRatio="xMidYMid meet"
    >
      <polygon
        points="50,22 74,36 74,64 50,78 26,64 26,36"
        fill="none"
        stroke="#F4F4F5"
        strokeWidth="1.6"
      />
    </svg>
  )
}

// ---------------------------------------------------------------------------
// Videos
// ---------------------------------------------------------------------------

const VIDEO_MIN_TILE = 250
const VIDEO_GAP = 18
const VIDEO_TEXT = 50

export function VideoGrid({
  files,
  base,
  progressOf,
  onPlay,
}: {
  files: MediaFile[]
  base: string
  /** How far through a video is, 0 to 1, when it has been started. */
  progressOf?: (path: string) => number | undefined
  onPlay: (file: MediaFile) => void
}): React.JSX.Element {
  const [ref, width] = useWidth()
  const inner = Math.max(0, width - 40)
  const columns = Math.max(1, Math.floor((inner + VIDEO_GAP) / (VIDEO_MIN_TILE + VIDEO_GAP)))
  const tile = columns > 0 ? (inner - VIDEO_GAP * (columns - 1)) / columns : 0
  const rows = Math.ceil(files.length / columns)

  return (
    <div ref={ref} className="h-full">
      {width > 0 && (
        <VList style={{ height: '100%' }} count={rows}>
          {(row) => (
            <div
              key={row}
              className="flex px-5"
              style={{ gap: VIDEO_GAP, paddingTop: row === 0 ? 18 : 0, paddingBottom: VIDEO_GAP }}
            >
              {files.slice(row * columns, row * columns + columns).map((file) => {
                const progress = progressOf?.(file.path)
                return (
                  <button
                    key={file.path}
                    onClick={() => onPlay(file)}
                    className="group text-left"
                    style={{ width: tile }}
                    title={file.path}
                  >
                    <Thumb
                      src={thumbUrl(base, file.path, file.mtime)}
                      alt={stemOf(file.path)}
                      className="aspect-video rounded-md ring-1 ring-inset ring-white/[0.06] transition-shadow duration-200 group-hover:ring-white/20"
                    >
                      <div className="absolute inset-0 flex items-center justify-center bg-black/30 opacity-0 transition-opacity duration-200 group-hover:opacity-100">
                        <span className="flex h-11 w-11 items-center justify-center rounded-full bg-white/90 shadow-lift">
                          <Play size={16} className="ml-0.5 fill-ink text-ink" />
                        </span>
                      </div>
                      {progress !== undefined && (
                        <div className="absolute inset-x-0 bottom-0 h-[3px] bg-black/50">
                          <div className="h-full bg-basalt" style={{ width: `${progress * 100}%` }} />
                        </div>
                      )}
                    </Thumb>
                    <div style={{ height: VIDEO_TEXT }} className="pt-2">
                      <div className="truncate text-[12.5px] font-medium text-text">
                        {stemOf(file.path)}
                      </div>
                      <div className="mt-0.5 flex gap-2 font-mono text-[10.5px] text-textFaint">
                        <span className="min-w-0 truncate">{folderOf(file.path) || 'Vault'}</span>
                        <span className="ml-auto shrink-0">{formatBytes(file.size)}</span>
                      </div>
                    </div>
                  </button>
                )
              })}
            </div>
          )}
        </VList>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Photos
// ---------------------------------------------------------------------------

const PHOTO_GAP = 4

type PhotoRow =
  | { kind: 'month'; key: string; label: string; count: number }
  | { kind: 'photos'; key: string; start: number; files: MediaFile[]; height: number }

export function PhotoGrid({
  files,
  base,
  onOpen,
}: {
  files: MediaFile[]
  base: string
  /** Opens the viewer at a photo's place in `files`. */
  onOpen: (index: number) => void
}): React.JSX.Element {
  const [ref, width] = useWidth()
  const inner = Math.max(0, width - 40)
  // Smaller rows on a narrow window, so a row still holds a few photos.
  const target = inner < 700 ? 150 : inner < 1200 ? 190 : 220

  const rows = useMemo<PhotoRow[]>(() => {
    if (inner <= 0) return []
    const out: PhotoRow[] = []
    for (const group of groupByMonth(files)) {
      out.push({ kind: 'month', key: `m-${group.key}`, label: group.label, count: group.files.length })
      const aspects = group.files.map((f) => (f.width && f.height ? f.width / f.height : 4 / 3))
      for (const row of justify(aspects, inner, target, PHOTO_GAP)) {
        out.push({
          kind: 'photos',
          key: `p-${group.key}-${row.start}`,
          start: group.start + row.start,
          files: group.files.slice(row.start, row.start + row.count),
          height: row.height,
        })
      }
    }
    return out
  }, [files, inner, target])

  return (
    <div ref={ref} className="h-full">
      {width > 0 && (
        <VList style={{ height: '100%' }} count={rows.length}>
          {(i) => {
            const row = rows[i]!
            if (row.kind === 'month') {
              return (
                <div key={row.key} className="flex items-baseline gap-2.5 px-5 pb-2.5 pt-5">
                  <span className="text-[14px] font-semibold tracking-tight text-text">
                    {row.label}
                  </span>
                  <span className="font-mono text-[10.5px] text-textFaint">{row.count}</span>
                </div>
              )
            }
            return (
              <div
                key={row.key}
                className="flex px-5"
                style={{ gap: PHOTO_GAP, height: row.height, marginBottom: PHOTO_GAP }}
              >
                {row.files.map((file, n) => {
                  const aspect = file.width && file.height ? file.width / file.height : 4 / 3
                  return (
                    <button
                      key={file.path}
                      onClick={() => onOpen(row.start + n)}
                      className="group relative shrink-0 overflow-hidden rounded-[3px] outline-none focus-visible:ring-2 focus-visible:ring-white/60"
                      style={{
                        width: Math.min(3, Math.max(0.5, aspect)) * row.height,
                        height: row.height,
                      }}
                      title={stemOf(file.path)}
                    >
                      <Thumb
                        src={thumbUrl(base, file.path, file.mtime)}
                        alt={stemOf(file.path)}
                        className="h-full w-full"
                      >
                        <div className="absolute inset-0 bg-white/0 transition-colors duration-200 group-hover:bg-white/[0.06]" />
                      </Thumb>
                    </button>
                  )
                })}
              </div>
            )
          }}
        </VList>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Music
// ---------------------------------------------------------------------------

const TRACK_ROW = 44

/** Tracks in the order an album is played: artist, album, number, title. */
export function sortTracks(files: MediaFile[]): MediaFile[] {
  const info = new Map(files.map((f) => [f.path, trackInfo(f.path)]))
  return [...files].sort((a, b) => {
    const x = info.get(a.path)!
    const y = info.get(b.path)!
    return (
      x.artist.localeCompare(y.artist) ||
      x.album.localeCompare(y.album) ||
      (x.number ?? 1e9) - (y.number ?? 1e9) ||
      x.title.localeCompare(y.title)
    )
  })
}

export function MusicList({
  files,
  playing,
  onPlay,
}: {
  /** Already in playing order. */
  files: MediaFile[]
  /** The path playing now, to mark it. */
  playing: string | null
  onPlay: (file: MediaFile) => void
}): React.JSX.Element {
  return (
    <div className="flex h-full flex-col px-3">
      <div className="grid h-8 shrink-0 grid-cols-[40px_minmax(0,2.2fr)_minmax(0,1.3fr)_minmax(0,1.3fr)_64px_72px] items-center gap-3 border-b border-line px-2 font-mono text-[9.5px] uppercase tracking-[0.16em] text-textFaint">
        <span className="text-right">#</span>
        <span>Title</span>
        <span>Artist</span>
        <span>Album</span>
        <span>Format</span>
        <span className="text-right">Size</span>
      </div>
      <VList style={{ flex: 1 }} count={files.length} itemSize={TRACK_ROW}>
        {(i) => {
          const file = files[i]!
          const info = trackInfo(file.path)
          const current = file.path === playing
          return (
            <button
              key={file.path}
              onClick={() => onPlay(file)}
              style={{ height: TRACK_ROW }}
              className={cn(
                'group grid w-full grid-cols-[40px_minmax(0,2.2fr)_minmax(0,1.3fr)_minmax(0,1.3fr)_64px_72px] items-center gap-3 rounded-md px-2 text-left transition-colors duration-150',
                current ? 'bg-white/[0.07]' : 'hover:bg-white/[0.04]',
              )}
            >
              <span className="flex justify-end font-mono text-[11px] text-textFaint">
                {current ? (
                  <Volume2 size={13} className="text-text" />
                ) : (
                  <>
                    <span className="group-hover:hidden">{info.number ?? i + 1}</span>
                    <Play size={12} className="hidden fill-text text-text group-hover:block" />
                  </>
                )}
              </span>
              <span className={cn('truncate text-[13px]', current ? 'font-medium text-text' : 'text-text')}>
                {info.title}
              </span>
              <span className="truncate text-[12px] text-textDim">{info.artist || '—'}</span>
              <span className="truncate text-[12px] text-textDim">{info.album || '—'}</span>
              <span className="font-mono text-[10px] text-textFaint">{formatOf(file.path)}</span>
              <span className="text-right font-mono text-[10.5px] text-textFaint">
                {formatBytes(file.size)}
              </span>
            </button>
          )
        }}
      </VList>
    </div>
  )
}
