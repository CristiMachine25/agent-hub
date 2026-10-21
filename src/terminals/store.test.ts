import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { TerminalInfo } from '@shared/types'

const spawns: ((info: TerminalInfo) => void)[] = []
const kill = vi.fn(async () => {})
vi.mock('@/api', () => ({
  api: { terminals: { spawn: () => new Promise((r) => spawns.push(r)), kill, list: async () => [], scrollback: async () => '' } },
  errorText: String
}))
vi.mock('@/store', () => ({ useStore: { getState: () => ({ resolvedTheme: 'dark', settings: null, go() {} }) } }))
const registry = vi.hoisted(() => ({
  disposeTerm: vi.fn(),
  estimateSize: () => ({ cols: 80, rows: 24 }),
  focusTerm: vi.fn(),
  getTerm: vi.fn(),
  onTerminalExit: vi.fn(),
  prepare: vi.fn()
}))
vi.mock('./registry', () => registry)
vi.stubGlobal('document', { querySelector: () => null })
vi.stubGlobal('requestAnimationFrame', (f: () => void) => f())

const { useGrid } = await import('./store')
const info = (id: string) => ({ id, accountId: 'acc-1' }) as TerminalInfo

describe('a spawn that finishes after its pane moved on', () => {
  beforeEach(() => {
    spawns.length = 0
    vi.clearAllMocks()
  })

  it('kills the terminal when the pane was closed during the spawn', async () => {
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli' })
    await useGrid.getState().close(0)
    spawns[0](info('t-closed'))
    await started
    expect(kill).toHaveBeenCalledWith('t-closed')
    expect(registry.disposeTerm).toHaveBeenCalledWith('t-closed')
    expect(registry.getTerm).not.toHaveBeenCalled()
    expect(useGrid.getState().ws.panes[0].terminalId).toBeUndefined()
    expect(useGrid.getState().starting[useGrid.getState().ws.panes[0].uid]).toBe(false)
  })

  it('attaches the terminal to its pane in the session the user switched away from', async () => {
    const uid = useGrid.getState().ws.panes[0].uid
    const from = useGrid.getState().activeId
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli' })
    useGrid.getState().newSession()
    spawns[0](info('t-elsewhere'))
    await started
    expect(kill).not.toHaveBeenCalled()
    expect(useGrid.getState().ws.panes.some((p) => p.terminalId)).toBe(false)
    const pane = useGrid.getState().sessions.find((x) => x.id === from)!.ws.panes.find((p) => p.uid === uid)!
    expect(pane.terminalId).toBe('t-elsewhere')
    useGrid.getState().switchSession(from)
    expect(useGrid.getState().ws.panes[0].terminalId).toBe('t-elsewhere')
  })
})
