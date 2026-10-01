// @vitest-environment jsdom
//
// Opening Movies on a large library asked the host for every poster at once.
// Now no more than a few are out together, and one film shown twice is
// fetched once.

import { act, render, waitFor } from '@testing-library/react'
import { createElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const asked = vi.hoisted(() => ({ ids: [] as string[], answers: [] as Array<() => void> }))

vi.mock('@/lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/api')>()
  return {
    ...actual,
    api: {
      ...actual.api,
      art: (id: string) =>
        new Promise<string>((resolve) => {
          asked.ids.push(id)
          asked.answers.push(() => resolve(`data:image/jpeg;base64,${id}`))
        }),
    },
  }
})

import { Poster } from '@/components/Poster'

describe('posters', () => {
  it('are fetched a few at a time, and once each', async () => {
    const films = Array.from({ length: 60 }, (_, i) => `film-${i}`)
    render(
      createElement(
        'div',
        null,
        ...films.map((id) => createElement(Poster, { key: id, title: id, id, hasArt: true })),
        // The same film again, as in a series sheet over the grid.
        createElement(Poster, { key: 'again', title: 'film-0', id: 'film-0', hasArt: true }),
      ),
    )
    await waitFor(() => expect(asked.ids.length).toBe(4))

    // Answering frees a place, and the next is asked for.
    await act(async () => {
      asked.answers.shift()!()
    })
    await waitFor(() => expect(asked.ids.length).toBe(5))

    while (asked.answers.length > 0) {
      await act(async () => {
        asked.answers.shift()!()
      })
    }
    expect(new Set(asked.ids).size).toBe(asked.ids.length)
    expect(asked.ids.length).toBe(films.length)
  })
})
