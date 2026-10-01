// @vitest-environment jsdom
//
// Everything shown about a drive belongs to one host, and changing drive goes
// from one host to the next without ever disconnecting. These are the tests
// that were missing when the last drive's films stayed on screen after a
// change: each hook is switched from one host to another, and nothing of the
// first may survive — not its items, not its revision, not a slow answer that
// arrives after the switch.

import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const calls = vi.hoisted(() => ({
  library: [] as number[],
  collections: [] as number[],
}))

/** Answers each call with whatever the test has queued for it. */
const pending = vi.hoisted(() => ({
  library: [] as Array<(value: unknown) => void>,
  collections: [] as Array<(value: unknown) => void>,
  watched: [] as Array<(value: unknown) => void>,
  identity: [] as Array<(value: unknown) => void>,
  profiles: [] as Array<(value: unknown) => void>,
}))

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api')>()
  const wait = (queue: Array<(value: unknown) => void>) =>
    new Promise((resolve) => {
      queue.push(resolve)
    })
  return {
    ...actual,
    api: {
      ...actual.api,
      library: (known: number) => {
        calls.library.push(known)
        return wait(pending.library)
      },
      collections: (known: number) => {
        calls.collections.push(known)
        return wait(pending.collections)
      },
      watchProgress: () => wait(pending.watched),
      identity: () => wait(pending.identity),
      profiles: () => wait(pending.profiles),
      mediaBase: () => Promise.resolve('http://127.0.0.1:9/tok/'),
    },
  }
})

import { useCollections } from './useCollections'
import { useIdentity } from './useIdentity'
import { useMediaLibrary } from './useMediaLibrary'
import { thumbUrl, useMediaBase } from './thumbs'
import { useWatched } from './useWatched'

const film = (title: string) => ({ id: title, kind: 'film', title, path: `${title}.mkv` })

/** Resolves the oldest waiting call of a kind. */
async function answer(kind: keyof typeof pending, value: unknown): Promise<void> {
  await waitFor(() => expect(pending[kind].length).toBeGreaterThan(0))
  await act(async () => {
    pending[kind].shift()!(value)
  })
}

beforeEach(() => {
  calls.library = []
  calls.collections = []
  for (const queue of Object.values(pending)) queue.length = 0
})

describe('the film and series library', () => {
  it('starts again from nothing on another host, revision and all', async () => {
    const { result, rerender } = renderHook(({ host }) => useMediaLibrary(host), {
      initialProps: { host: 'host-a' as string | null },
    })
    await answer('library', { revision: 3, enabled: true, scanning: false, items: [film('Old Film')] })
    expect(result.current.films.map((f) => f.title)).toEqual(['Old Film'])

    rerender({ host: 'host-b' })
    // Gone at once, not when the new host answers.
    expect(result.current.films).toEqual([])
    // The new host is asked as a stranger: the last host's revision would let
    // it answer "nothing changed".
    await waitFor(() => expect(calls.library).toEqual([0, 0]))
    await answer('library', { revision: 3, enabled: true, scanning: false, items: [film('New Film')] })
    expect(result.current.films.map((f) => f.title)).toEqual(['New Film'])
  })

  it('throws away an answer from the last host that arrives after the change', async () => {
    const { result, rerender } = renderHook(({ host }) => useMediaLibrary(host), {
      initialProps: { host: 'host-a' as string | null },
    })
    await waitFor(() => expect(pending.library.length).toBe(1))
    const slowFromA = pending.library.shift()!

    rerender({ host: 'host-b' })
    await answer('library', { revision: 1, enabled: true, scanning: false, items: [film('From B')] })
    await act(async () => {
      slowFromA({ revision: 9, enabled: true, scanning: false, items: [film('From A')] })
    })
    expect(result.current.films.map((f) => f.title)).toEqual(['From B'])
  })

  it('still clears when the connection is lost', async () => {
    const { result, rerender } = renderHook(({ host }) => useMediaLibrary(host), {
      initialProps: { host: 'host-a' as string | null },
    })
    await answer('library', { revision: 2, enabled: true, scanning: false, items: [film('A')] })
    rerender({ host: null })
    expect(result.current.films).toEqual([])
  })
})

describe('videos, music, photos and recent', () => {
  it('starts again from nothing on another host', async () => {
    const lists = (name: string) => ({
      revision: 4,
      scanning: false,
      collections: { videos: [{ path: name, size: 1, mtime: 1 }], music: [], photos: [], recent: [], truncated: false },
    })
    const { result, rerender } = renderHook(({ host }) => useCollections(host), {
      initialProps: { host: 'host-a' as string | null },
    })
    await answer('collections', lists('a.mp4'))
    expect(result.current.collections.videos.map((v) => v.path)).toEqual(['a.mp4'])

    rerender({ host: 'host-b' })
    expect(result.current.collections.videos).toEqual([])
    await waitFor(() => expect(calls.collections).toEqual([0, 0]))
    await answer('collections', lists('b.mp4'))
    expect(result.current.collections.videos.map((v) => v.path)).toEqual(['b.mp4'])
  })
})

describe('continue watching', () => {
  const at = (path: string) => [{ path, fraction: 0.5, position: 60, duration: 120, updatedAt: 1 }]

  it('shows only the new host’s list, even when the old one answers late', async () => {
    const { result, rerender } = renderHook(({ host }) => useWatched(host, ''), {
      initialProps: { host: 'host-a' as string | null },
    })
    await waitFor(() => expect(pending.watched.length).toBe(1))
    const slowFromA = pending.watched.shift()!

    rerender({ host: 'host-b' })
    expect(result.current.all).toEqual([])
    await answer('watched', at('b.mkv'))
    await act(async () => {
      slowFromA(at('a.mkv'))
    })
    expect(result.current.all.map((w) => w.path)).toEqual(['b.mkv'])
  })
})

describe('who is using the device', () => {
  it('never offers the last host’s profiles on the next', async () => {
    const { result, rerender } = renderHook(({ host }) => useIdentity(host), {
      initialProps: { host: 'host-a' as string | null },
    })
    await answer('identity', { choose: false, profile: null, lastProfile: null, ended: false })
    await answer('profiles', [{ id: 'p1', name: 'Maya', color: 0, hasPin: true }])
    await waitFor(() => expect(result.current.profiles.map((p) => p.name)).toEqual(['Maya']))

    rerender({ host: 'host-b' })
    expect(result.current.profiles).toEqual([])
    expect(result.current.state).toBeNull()
  })
})

describe('thumbnails', () => {
  it('name the host, so one drive’s cached picture is never another’s', async () => {
    const { result, rerender } = renderHook(({ host }) => useMediaBase(host), {
      initialProps: { host: 'aaaaaaaaaaaaaaaa' as string | null },
    })
    await waitFor(() => expect(result.current).not.toBe(''))
    const fromA = thumbUrl(result.current, 'Photos/a.jpg', 100)

    rerender({ host: 'bbbbbbbbbbbbbbbb' })
    await waitFor(() => expect(result.current).not.toBe(''))
    const fromB = thumbUrl(result.current, 'Photos/a.jpg', 100)

    expect(fromA).not.toBe(fromB)
  })
})
