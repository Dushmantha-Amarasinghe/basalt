import { describe, expect, it } from 'vitest'
import { parseNotes, parseSpans, type Block } from './notes'

/** The plain text of a block, for assertions that do not care about runs. */
const text = (block: Block | undefined): string =>
  (block?.spans ?? []).map((s) => s.text).join('')

describe('parseNotes', () => {
  it('reads the shape real release notes are written in', () => {
    const blocks = parseNotes(
      [
        '## New',
        '',
        '* **Battery tile** — charge left, amber below 20 percent.',
        '* Network speeds in megabits or megabytes.',
        '',
        '## Fixed',
        '',
        'A paragraph explaining the rest.',
      ].join('\n'),
    )

    expect(blocks.map((b) => b.kind)).toEqual([
      'heading',
      'bullet',
      'bullet',
      'heading',
      'text',
    ])
    expect(text(blocks[0])).toBe('New')
    expect(text(blocks[1])).toBe('Battery tile — charge left, amber below 20 percent.')
    expect(text(blocks[4])).toBe('A paragraph explaining the rest.')
  })

  it('keeps the heading text and drops the hashes, at any level', () => {
    for (const hashes of ['#', '##', '###', '######']) {
      const [block] = parseNotes(`${hashes} Basalt v1.0.0`)
      expect(block).toEqual({ kind: 'heading', spans: [{ text: 'Basalt v1.0.0' }] })
    }
  })

  it('accepts any of the three bullet markers', () => {
    for (const marker of ['*', '-', '+']) {
      const [block] = parseNotes(`${marker} one thing`)
      expect(block?.kind).toBe('bullet')
      expect(text(block)).toBe('one thing')
    }
  })

  it('joins a wrapped paragraph into one block', () => {
    // Notes are wrapped at whatever width their author's editor used, and a
    // line break there is not a line break in the reading.
    const blocks = parseNotes('The first release of Basalt,\na personal NAS\nfor Windows.')
    expect(blocks).toHaveLength(1)
    expect(text(blocks[0])).toBe('The first release of Basalt, a personal NAS for Windows.')
  })

  it('collapses blank lines instead of drawing holes', () => {
    const blocks = parseNotes('one\n\n\n\n\ntwo')
    expect(blocks.map(text)).toEqual(['one', 'two'])
  })

  it('drops a horizontal rule, which is clutter in a panel this small', () => {
    expect(parseNotes('one\n\n---\n\ntwo').map((b) => b.kind)).toEqual(['text', 'text'])
    expect(parseNotes('***').length).toBe(0)
  })

  it('reads nothing out of nothing', () => {
    expect(parseNotes('')).toEqual([])
    expect(parseNotes('\n\n  \n')).toEqual([])
  })

  it('leaves a hash that is not a heading alone', () => {
    // `#7` is an issue number, not an empty heading.
    expect(text(parseNotes('Fixes #7 and #9.')[0])).toBe('Fixes #7 and #9.')
  })
})

describe('parseSpans', () => {
  it('splits bold out of the line around it', () => {
    expect(parseSpans('a **bold** word')).toEqual([
      { text: 'a ' },
      { text: 'bold', bold: true },
      { text: ' word' },
    ])
  })

  it('handles a line that is bold from end to end', () => {
    expect(parseSpans('**all of it**')).toEqual([{ text: 'all of it', bold: true }])
  })

  it('handles several bold runs', () => {
    expect(parseSpans('**one** and **two**').filter((s) => s.bold)).toEqual([
      { text: 'one', bold: true },
      { text: 'two', bold: true },
    ])
  })

  it('reads a code span', () => {
    expect(parseSpans('press `Ctrl+K`')).toEqual([
      { text: 'press ' },
      { text: 'Ctrl+K', code: true },
    ])
  })

  it('reads a code span inside bold, which is how a filename is written', () => {
    // `**`Basalt-Host-1.0.0-setup.exe`** — on the machine with the drive.`
    // Reading the bold run as finished text printed the backticks.
    expect(parseSpans('**`Basalt-Host.exe`** — on the host.')).toEqual([
      { text: 'Basalt-Host.exe', code: true, bold: true },
      { text: ' — on the host.' },
    ])
  })

  it('leaves an unmatched marker as written rather than eating the line', () => {
    // Swallowing it would lose everything after the stray marker, which is
    // the worst way to handle notes somebody typed in a hurry.
    expect(parseSpans('a ** stray marker')).toEqual([{ text: 'a ** stray marker' }])
    expect(parseSpans('an ` unclosed tick')).toEqual([{ text: 'an ` unclosed tick' }])
  })

  it('does not read an empty pair as a run', () => {
    expect(parseSpans('****')).toEqual([{ text: '****' }])
  })

  it('leaves a line with no markup as one plain run', () => {
    expect(parseSpans('nothing special here')).toEqual([{ text: 'nothing special here' }])
  })
})
