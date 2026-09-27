import { useEffect, useRef } from 'react'
import { footageFor } from '@/lib/showcase'
import { previewLine } from '@/lib/previewMpv'

/**
 * The picture, in the browser preview.
 *
 * In the app, mpv draws the film behind the page. The preview has no mpv, so
 * this plays the showcase's footage in the same place — under the controls,
 * filling the window — and draws a subtitle line the way mpv would when
 * subtitles are on. Nothing here runs inside the app.
 */
export function PreviewPicture({
  path,
  paused,
  position,
  subtitles,
}: {
  path: string
  paused: boolean
  position: number
  subtitles: boolean
}): React.JSX.Element | null {
  const video = useRef<HTMLVideoElement | null>(null)
  const src = footageFor(path)

  useEffect(() => {
    const v = video.current
    if (!v) return
    if (paused) v.pause()
    else void v.play().catch(() => {})
  }, [paused, src])

  if (!src) return null
  const line = subtitles ? previewLine(position) : null

  return (
    <div className="pointer-events-none absolute inset-0 bg-black">
      <video
        ref={video}
        src={src}
        autoPlay
        loop
        muted
        playsInline
        className="h-full w-full object-cover"
      />
      {line && (
        <div className="absolute inset-x-0 bottom-[14%] flex justify-center px-8">
          <span
            className="text-center text-[clamp(16px,2.6vw,34px)] font-semibold leading-snug text-white"
            style={{
              textShadow:
                '0 0 3px #000, 0 0 3px #000, 0 2px 4px rgba(0,0,0,0.8), 1px 1px 0 #000, -1px -1px 0 #000',
            }}
          >
            {line}
          </span>
        </div>
      )}
    </div>
  )
}
