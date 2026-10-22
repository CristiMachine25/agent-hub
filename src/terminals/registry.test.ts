import { afterEach, describe, expect, it, vi } from 'vitest'

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
    // Like xterm: an addon is registered, then activated; activate may throw.
    loadAddon(addon: { activate?: (t: unknown) => void }) {
      addon.activate?.(this)
    }
    attachCustomKeyEventHandler() {}
    onData(cb: (d: string) => void) {
      this.input = cb
    }
    open() {}
    dispose() {}
    write(d: string, cb?: () => void) {
      this.written.push(d)
      if (cb) this.parsed.push(cb)
    }
  }
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class {} }))
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }))
const webgl = vi.hoisted(() => ({ throws: false, last: undefined as unknown }))
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    lost = () => {}
    disposed = false
    constructor() {
      webgl.last = this
    }
    // The real addon reports a missing WebGL2 context from activate, which loadAddon calls.
    activate() {
      if (webgl.throws) throw new Error('WebGL2 not supported')
    }
    onContextLoss(cb: () => void) {
      this.lost = cb
      return { dispose() {} }
    }
    dispose() {
      this.disposed = true
    }
    clearTextureAtlas() {}
  }
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))
vi.stubGlobal('document', { createElement: () => ({ isConnected: false, remove() {} }), documentElement: {} })
vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '' }))

const { MAX_WEBGL, acquireWebgl, disposeTerm, getTerm, prepare } = await import('./registry')
const output = (id: string, data: string) => handlers['terminal-output']!({ id, data } as never)
const written = (id: string) => (getTerm(id, 'dark').term as unknown as { written: string[] }).written

interface FakeAddon {
  lost(): void
  disposed: boolean
}
const addonOf = (id: string) => getTerm(id, 'dark').webgl as unknown as FakeAddon | undefined
/** Creates a terminal, puts its host on screen as a pane would, and asks for WebGL. */
function show(id: string) {
  const e = getTerm(id, 'dark')
  ;(e.host as { isConnected: boolean }).isConnected = true
  acquireWebgl(id)
  return e
}
const hide = (id: string) => ((getTerm(id, 'dark').host as { isConnected: boolean }).isConnected = false)

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

describe('WebGL renderer', () => {
  const ids: string[] = []
  const shown = (id: string) => {
    ids.push(id)
    return show(id)
  }
  afterEach(() => {
    webgl.throws = false
    for (const id of ids.splice(0)) disposeTerm(id)
  })

  it('keeps the DOM renderer when the addon fails to activate, and unregisters it', () => {
    webgl.throws = true
    expect(() => shown('no-gl')).not.toThrow()
    expect(addonOf('no-gl')).toBeUndefined()
    expect((webgl.last as FakeAddon).disposed).toBe(true)
  })

  it('renders at most MAX_WEBGL terminals with WebGL', () => {
    for (let i = 0; i <= MAX_WEBGL; i++) shown(`cap-${i}`)
    expect(ids.filter((id) => addonOf(id)).length).toBe(MAX_WEBGL)
    expect(addonOf(`cap-${MAX_WEBGL}`)).toBeUndefined()
  })

  it('moves WebGL from a terminal that is off screen to one being shown', () => {
    for (let i = 0; i < MAX_WEBGL; i++) shown(`move-${i}`)
    const hidden = addonOf('move-0')!
    hide('move-0')
    shown('move-new')
    expect(hidden.disposed).toBe(true)
    expect(addonOf('move-0')).toBeUndefined()
    expect(addonOf('move-new')).toBeDefined()
  })

  it('falls back to the DOM renderer when the context is lost', () => {
    shown('lost')
    const addon = addonOf('lost')!
    addon.lost()
    expect(addon.disposed).toBe(true)
    expect(addonOf('lost')).toBeUndefined()
    // A terminal that lost its context does not retry on the next show.
    acquireWebgl('lost')
    expect(addonOf('lost')).toBeUndefined()
  })

  it('releases the addon when the terminal is disposed', () => {
    shown('gone')
    const addon = addonOf('gone')!
    disposeTerm('gone')
    expect(addon.disposed).toBe(true)
  })
})
