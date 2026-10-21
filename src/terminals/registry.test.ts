import { describe, expect, it, vi } from 'vitest'

const handlers: Record<string, (p: never) => void> = {}
vi.mock('@/api', () => ({ api: { platform: 'darwin', on: (event: string, cb: (p: never) => void) => (handlers[event] = cb) } }))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    written: string[] = []
    cols = 80
    rows = 24
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    onData() {}
    open() {}
    write(d: string) {
      this.written.push(d)
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
