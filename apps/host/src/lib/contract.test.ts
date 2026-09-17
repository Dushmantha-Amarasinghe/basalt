/**
 * The contract between the Rust host and this interface.
 *
 * Neither side can check the other at compile time: the Tauri shell is outside
 * the Cargo workspace, and TypeScript has never heard of `serde`. The gap
 * between them is where this project's most expensive bug lived — Tauri
 * converts command *arguments* from camelCase to snake_case but leaves
 * *responses* exactly as serde wrote them, so a Rust field named `host_id`
 * arrives as `host_id` while the interface reads `hostId` and silently gets
 * `undefined`. The app runs, connects, and shows nothing.
 *
 * So this test reads the actual Rust source and checks three things:
 *
 * 1. Every command `api.ts` calls exists in the shell.
 * 2. Every argument it passes is a parameter of that command.
 * 3. Every response interface matches its `#[serde(rename_all = "camelCase")]`
 *    struct in `basalt-host::ui`, field for field.
 *
 * Reading source with regular expressions is crude, and it is the right amount
 * of machinery here: the alternative is no check at all, and the shapes it
 * parses are ones this project writes by hand in a consistent style.
 */

import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { describe, expect, it } from 'vitest'

const here = path.dirname(fileURLToPath(import.meta.url))
const read = (relative: string): string =>
  readFileSync(path.resolve(here, relative), 'utf8')

const shell = read('../../src-tauri/src/lib.rs')
const uiRust = read('../../../../crates/basalt-host/src/ui.rs')
const apiTs = read('./api.ts')

function camel(snake: string): string {
  return snake.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())
}

function snake(camelCase: string): string {
  return camelCase.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`)
}

/** Strips `//` comments so they cannot be mistaken for declarations. */
function uncomment(source: string): string {
  return source.replace(/^\s*\/\/.*$/gm, '')
}

// ---------------------------------------------------------------------------
// Reading the Rust
// ---------------------------------------------------------------------------

interface Command {
  name: string
  /** Parameters the interface has to supply, snake_case as Rust spells them. */
  params: string[]
}

function rustCommands(source: string): Map<string, Command> {
  const commands = new Map<string, Command>()
  const pattern = /#\[tauri::command\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(([^)]*)\)/g

  for (const match of source.matchAll(pattern)) {
    const [, name, rawParams] = match
    const params = (rawParams ?? '')
      .split(',')
      .map((p) => p.trim())
      .filter(Boolean)
      .map((p) => {
        const [ident, ...rest] = p.split(':')
        return { ident: ident!.trim(), type: rest.join(':') }
      })
      // Tauri injects these; the interface never passes them.
      .filter((p) => !/State\s*<|AppHandle|Window|Webview/.test(p.type))
      .map((p) => p.ident)

    commands.set(name!, { name: name!, params })
  }
  return commands
}

/** Structs carrying `#[serde(rename_all = "camelCase")]`, with their fields. */
function rustStructs(source: string): Map<string, string[]> {
  const structs = new Map<string, string[]>()
  const pattern =
    /#\[serde\(rename_all = "camelCase"\)\]\s*pub struct (\w+) \{([\s\S]*?)\n\}/g

  for (const match of source.matchAll(pattern)) {
    const [, name, body] = match
    const fields = uncomment(body!)
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.startsWith('pub '))
      .map((line) => line.slice(4).split(':')[0]!.trim())
      .map(camel)
    structs.set(name!, fields.sort())
  }
  return structs
}

// ---------------------------------------------------------------------------
// Reading the TypeScript
// ---------------------------------------------------------------------------

interface Invocation {
  command: string
  args: string[]
}

function tsInvocations(source: string): Invocation[] {
  const found: Invocation[] = []
  // `call('name')` or `call('name', { a, b })`.
  const pattern = /call(?:<[^>]*>)?\(\s*'(\w+)'\s*(?:,\s*\{([^}]*)\})?\s*\)/g

  for (const match of source.matchAll(pattern)) {
    const [, command, rawArgs] = match
    const args = (rawArgs ?? '')
      .split(',')
      .map((a) => a.split(':')[0]!.trim())
      .filter(Boolean)
    found.push({ command: command!, args })
  }
  return found
}

function tsInterfaces(source: string): Map<string, string[]> {
  const interfaces = new Map<string, string[]>()
  const pattern = /export interface (\w+) \{([\s\S]*?)\n\}/g

  for (const match of source.matchAll(pattern)) {
    const [, name, body] = match
    const fields = body!
      // Block comments are used for field documentation here.
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => /^\w+\??\s*:/.test(line))
      .map((line) => line.split(/[?:]/)[0]!.trim())
    interfaces.set(name!, fields.sort())
  }
  return interfaces
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

const commands = rustCommands(shell)
const invocations = tsInvocations(apiTs)
const structs = rustStructs(uiRust)
const interfaces = tsInterfaces(apiTs)

describe('the shell and the interface agree', () => {
  it('finds commands on both sides, so an empty parse cannot pass', () => {
    expect(commands.size).toBeGreaterThan(8)
    expect(invocations.length).toBeGreaterThan(8)
    expect(structs.size).toBeGreaterThan(3)
  })

  it.each(invocations)('$command exists in the shell', ({ command }) => {
    expect([...commands.keys()]).toContain(command)
  })

  it.each(invocations.filter((i) => i.args.length > 0))(
    '$command takes the arguments the interface sends',
    ({ command, args }) => {
      const declared = commands.get(command)?.params ?? []
      // Tauri lowercases the camelCase the interface sends into the snake_case
      // the function declares, so compare in Rust's spelling.
      for (const arg of args) {
        expect(declared).toContain(snake(arg))
      }
    },
  )

  it('every command is reachable from the interface', () => {
    const called = new Set(invocations.map((i) => i.command))
    const unreachable = [...commands.keys()].filter((name) => !called.has(name))
    expect(unreachable).toEqual([])
  })

  it('every command is registered in the invoke handler', () => {
    const handler = shell.match(/generate_handler!\[([\s\S]*?)\]/)?.[1] ?? ''
    const registered = handler
      .split(',')
      .map((name) => name.trim())
      .filter(Boolean)

    for (const name of commands.keys()) {
      expect(registered).toContain(name)
    }
  })
})

/**
 * Every Tauri plugin call the interface makes must be granted in the
 * capability file.
 *
 * This is a second, separate way for the two halves to disagree, and it fails
 * even more quietly than a renamed field: an ungranted call rejects at the ACL,
 * and if nothing is catching, the button simply does nothing. That is exactly
 * how `dialog:allow-confirm` went missing — Forget this vault, Pair with a
 * different vault and Delete all looked like dead controls, with no error in
 * the console and nothing in the logs.
 */
function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...sourceFiles(full))
    else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith('.test.ts')) {
      out.push(full)
    }
  }
  return out
}

/** `const { confirm, save } = await import('@tauri-apps/plugin-dialog')` */
function pluginCalls(sources: string[]): Set<string> {
  const needed = new Set<string>()
  const pattern =
    /(?:const|let)\s*\{([^}]*)\}\s*=\s*await import\(\s*'@tauri-apps\/plugin-(\w+)'\s*\)/g

  for (const source of sources) {
    const text = readFileSync(source, 'utf8')
    for (const match of text.matchAll(pattern)) {
      const [, names, plugin] = match
      for (const raw of (names ?? '').split(',')) {
        // `{ open: openPath }` — the permission follows the imported name.
        const name = raw.split(':')[0]!.trim()
        if (name) needed.add(`${plugin}:allow-${name}`)
      }
    }
  }
  return needed
}

describe('plugin permissions', () => {
  const capability = JSON.parse(
    read('../../src-tauri/capabilities/default.json'),
  ) as { permissions: string[] }
  const granted = new Set(capability.permissions)
  const needed = [...pluginCalls(sourceFiles(path.resolve(here, '..')))]

  it('finds plugin calls at all, so an empty scan cannot pass', () => {
    expect(needed.length).toBeGreaterThan(0)
  })

  it.each(needed)('%s is granted in the capability file', (permission) => {
    expect(granted).toContain(permission)
  })
})

describe('response shapes', () => {
  // The exact bug: `host_id` on the wire, `hostId` in the interface, every
  // field undefined and no error anywhere.
  it.each([
    ['HostStatus', 'HostStatus'],
    ['VaultView', 'VaultView'],
    ['DriveView', 'DriveView'],
    ['DeviceView', 'DeviceView'],
    ['PairingView', 'PairingView'],
  ])('%s has the same fields in Rust and TypeScript', (rustName, tsName) => {
    const rustFields = structs.get(rustName)
    const tsFields = interfaces.get(tsName)

    expect(rustFields, `${rustName} not found in basalt-host::ui`).toBeDefined()
    expect(tsFields, `${tsName} not found in api.ts`).toBeDefined()
    expect(tsFields).toEqual(rustFields)
  })
})
