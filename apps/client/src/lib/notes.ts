/**
 * Turning release notes into something worth reading in the app.
 *
 * The notes come from GitHub, where they are Markdown, and GitHub renders
 * them. Shown raw in a panel they arrive as `### Basalt v1.0.0` and
 * `**There is nothing to configure.**` — which reads worse than no notes at
 * all, because it looks like the app is broken rather than terse.
 *
 * Deliberately not a Markdown library. What release notes actually use is
 * headings, bullets, bold and the odd `code` span; a parser for that is
 * forty lines, and a dependency for it is a parser for tables, footnotes and
 * HTML embedding — none of which should render inside a settings panel
 * anyway.
 */

/** One rendered line, already split into its runs of plain and bold text. */
export interface Span {
  text: string
  bold?: boolean
  code?: boolean
}

export type Block =
  | { kind: 'heading'; spans: Span[] }
  | { kind: 'bullet'; spans: Span[] }
  | { kind: 'text'; spans: Span[] }

/**
 * Reads notes into blocks, dropping the syntax that marked them up.
 *
 * Blank lines separate paragraphs and are not themselves blocks: a run of
 * them collapses, so notes written with generous spacing do not open a hole
 * in the middle of the panel.
 */
export function parseNotes(notes: string): Block[] {
  const blocks: Block[] = []
  let paragraph: string[] = []

  const flush = (): void => {
    if (paragraph.length > 0) {
      blocks.push({ kind: 'text', spans: parseSpans(paragraph.join(' ')) })
      paragraph = []
    }
  }

  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim()

    if (line === '') {
      flush()
      continue
    }

    const heading = /^#{1,6}\s+(.*)$/.exec(line)
    if (heading) {
      flush()
      blocks.push({ kind: 'heading', spans: parseSpans(heading[1] ?? '') })
      continue
    }

    const bullet = /^[-*+]\s+(.*)$/.exec(line)
    if (bullet) {
      flush()
      blocks.push({ kind: 'bullet', spans: parseSpans(bullet[1] ?? '') })
      continue
    }

    // A horizontal rule is a separator in a document and clutter in a panel
    // this small.
    if (/^([-*_])\1{2,}$/.test(line)) {
      flush()
      continue
    }

    paragraph.push(line)
  }

  flush()
  return blocks
}

/**
 * Splits one line into bold, code and plain runs.
 *
 * Bold is read first and then read again inside itself, because the shape
 * release notes actually use is a bold filename — ``**`Basalt-Host.exe`**``.
 * Treating the bold run as finished text would print the backticks.
 *
 * Unmatched markers are left as written rather than swallowed: a stray `**`
 * in someone's notes should look like a stray `**`, not silently eat the
 * rest of the line.
 */
export function parseSpans(line: string): Span[] {
  const spans: Span[] = []
  let plain = ''

  const push = (...added: Span[]): void => {
    if (plain) {
      spans.push({ text: plain })
      plain = ''
    }
    spans.push(...added)
  }

  let at = 0
  while (at < line.length) {
    if (line.startsWith('**', at)) {
      const close = line.indexOf('**', at + 2)
      if (close > at + 2) {
        const inner = parseSpans(line.slice(at + 2, close))
        push(...inner.map((span) => ({ ...span, bold: true })))
        at = close + 2
        continue
      }
    }
    if (line[at] === '`') {
      const close = line.indexOf('`', at + 1)
      if (close > at + 1) {
        push({ text: line.slice(at + 1, close), code: true })
        at = close + 1
        continue
      }
    }
    plain += line[at]
    at += 1
  }

  if (plain) spans.push({ text: plain })
  return spans
}
