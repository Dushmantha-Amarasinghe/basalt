import { describe, expect, it } from 'vitest'
import type { LibraryItem } from './api'
import { itemQuality, qualityOf } from './quality'

const size = (width: number, height: number) => ({ width, height })

describe('qualityOf', () => {
  it('names the standard sizes', () => {
    expect(qualityOf(size(720, 480))).toBe('SD')
    expect(qualityOf(size(1280, 720))).toBe('HD')
    expect(qualityOf(size(1920, 1080))).toBe('FHD')
    expect(qualityOf(size(2560, 1440))).toBe('2K')
    expect(qualityOf(size(3840, 2160))).toBe('4K')
    expect(qualityOf(size(7680, 4320))).toBe('8K')
  })

  it('does not mark a film down for being cropped to cinema shape', () => {
    expect(qualityOf(size(1920, 800))).toBe('FHD')
    expect(qualityOf(size(3840, 1600))).toBe('4K')
    expect(qualityOf(size(1280, 536))).toBe('HD')
  })

  it('says nothing about a size it was not given', () => {
    expect(qualityOf(null)).toBeNull()
    expect(qualityOf(undefined)).toBeNull()
    expect(qualityOf(size(0, 0))).toBeNull()
  })
})

describe('itemQuality', () => {
  const series = (heights: number[]): LibraryItem =>
    ({
      id: 's',
      kind: 'series',
      title: 'Show',
      size: 0,
      added: 0,
      confidence: 99,
      hasArt: false,
      seasons: [
        {
          number: 1,
          episodes: heights.map((h, i) => ({
            number: i + 1,
            path: `e${i}.mkv`,
            size: 1,
            added: 0,
            resolution: h ? size(Math.round((h * 16) / 9), h) : undefined,
          })),
        },
      ],
    }) as LibraryItem

  it('goes by what most episodes are', () => {
    expect(itemQuality(series([2160, 2160, 2160, 720]))).toBe('4K')
    expect(itemQuality(series([1080, 1080, 2160]))).toBe('FHD')
  })

  it('takes the better picture in a tie, and ignores episodes it cannot tell', () => {
    expect(itemQuality(series([1080, 2160]))).toBe('4K')
    expect(itemQuality(series([0, 0, 720]))).toBe('HD')
    expect(itemQuality(series([0, 0]))).toBeNull()
  })

  it('uses a film its own size', () => {
    const film = { kind: 'film', seasons: [], resolution: size(1920, 1080) } as unknown as LibraryItem
    expect(itemQuality(film)).toBe('FHD')
  })
})
