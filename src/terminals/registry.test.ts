import { describe, expect, it, vi } from 'vitest'

const handlers: Record<string, (p: never) => void> = {}
const writes = vi.hoisted(() => vi.fn(() => Promise.resolve()))
vi.mock('@/api', () => ({
  api: { platform: 'darwin', on: (event: string, cb: (p: never) => void) => (handlers[event] = cb), terminals: { write: writes } }
}))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    written: string[] = []
    parsed: (() => void)[] = []
    input: (d: string) => void = () => {}
    cols = 80
    rows = 24
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    onData(cb: (d: string) => void) {
      this.input = cb
    }
    open() {}
    write(d: string, cb?: () => void) {
      this.written.push(d)
      if (cb) this.parsed.push(cb)
    }
  }
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class {} }))
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))
vi.stubGlobal('document', { createElement: () => ({}), documentElement: {} })
vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '' }))

const { disposeTerm, getTerm, prepare } = await import('./registry')
const output = (id: string, data: string) => handlers['terminal-output']!({ id, data } as never)
const written = (id: string) => (getTerm(id, 'dark').term as unknown as { written: string[] }).written

describe('terminal output buffering', () => {
  it('replays output that arrived before the terminal was created', () => {
    prepare()
    output('early', 'hello')
    expect(written('early')).toEqual(['hello'])
  })

  it('drops output that arrives after the terminal was disposed', () => {
    prepare()
    disposeTerm('closed')
    output('closed', 'trailing chunk')
    expect(written('closed')).toEqual([])
  })
})

describe('restored scrollback', () => {
  it('does not send replies to queries in the replayed scrollback, but sends typing after it', async () => {
    const term = getTerm('restored', 'dark', 'old\x1b[6n').term as unknown as { input: (d: string) => void; parsed: (() => void)[] }
    term.input('\x1b[1;1R') // xterm answering the stale query while it parses the seed
    expect(writes).not.toHaveBeenCalled()
    for (const done of term.parsed) done()
    term.input('x')
    await Promise.resolve()
    expect(writes).toHaveBeenCalledWith('restored', 'x')
    expect(writes).toHaveBeenCalledTimes(1)
  })
})
