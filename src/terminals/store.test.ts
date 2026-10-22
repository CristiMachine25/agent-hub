import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { TerminalInfo, TerminalSpawnRequest } from '@shared/types'

const spawns: ((info: TerminalInfo) => void)[] = []
const requests: TerminalSpawnRequest[] = []
const kill = vi.fn(async () => {})
const session = vi.fn(async (_id: string): Promise<string | null> => null)
vi.mock('@/api', () => ({
  api: {
    terminals: {
      spawn: (req: TerminalSpawnRequest) => {
        requests.push(req)
        return new Promise((r) => spawns.push(r))
      },
      kill,
      list: async () => [],
      scrollback: async () => '',
      session
    }
  },
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
    requests.length = 0
    vi.clearAllMocks()
  })

  it('kills the terminal when the pane was closed during the spawn', async () => {
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli' })
    await useGrid.getState().close(0)
    spawns[0]!(info('t-closed'))
    await started
    expect(kill).toHaveBeenCalledWith('t-closed')
    expect(registry.disposeTerm).toHaveBeenCalledWith('t-closed')
    expect(registry.getTerm).not.toHaveBeenCalled()
    expect(useGrid.getState().ws.panes[0]!.terminalId).toBeUndefined()
    expect(useGrid.getState().starting[useGrid.getState().ws.panes[0]!.uid]).toBe(false)
  })

  it('attaches the terminal to its pane in the session the user switched away from', async () => {
    const uid = useGrid.getState().ws.panes[0]!.uid
    const from = useGrid.getState().activeId
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli' })
    useGrid.getState().newSession()
    spawns[0]!(info('t-elsewhere'))
    await started
    expect(kill).not.toHaveBeenCalled()
    expect(useGrid.getState().ws.panes.some((p) => p.terminalId)).toBe(false)
    const pane = useGrid.getState().sessions.find((x) => x.id === from)!.ws.panes.find((p) => p.uid === uid)!
    expect(pane.terminalId).toBe('t-elsewhere')
    useGrid.getState().switchSession(from)
    expect(useGrid.getState().ws.panes[0]!.terminalId).toBe('t-elsewhere')
  })
})

describe('moving a pane to another account', () => {
  beforeEach(() => {
    spawns.length = 0
    requests.length = 0
    vi.clearAllMocks()
  })

  it("reopens that pane's own conversation, asked for before the pane stops", async () => {
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli', cwd: '/work' })
    spawns[0]!(info('t-1'))
    await started
    session.mockImplementation(async (id) => {
      expect(kill).not.toHaveBeenCalled()
      return id === 't-1' ? 'conv-1' : null
    })

    const moved = useGrid.getState().continueOn(0, 'acc-2')
    await vi.waitFor(() => expect(spawns).toHaveLength(2))
    spawns[1]!({ id: 't-2', accountId: 'acc-2' } as TerminalInfo)
    await moved
    expect(kill).toHaveBeenCalledWith('t-1')
    expect(requests[1]).toMatchObject({ accountId: 'acc-2', cwd: '/work', resume: true, resumeSession: 'conv-1' })
  })

  it('falls back to the most recent conversation when the tool cannot say', async () => {
    const started = useGrid.getState().start(0, { accountId: 'acc-1', target: 'cli' })
    spawns[0]!(info('t-3'))
    await started
    session.mockResolvedValue(null)
    const moved = useGrid.getState().continueOn(0, 'acc-2')
    await vi.waitFor(() => expect(spawns).toHaveLength(2))
    spawns[1]!({ id: 't-4', accountId: 'acc-2' } as TerminalInfo)
    await moved
    expect(requests[1]).toMatchObject({ resume: true })
    expect(requests[1]!.resumeSession).toBeUndefined()
  })
})
