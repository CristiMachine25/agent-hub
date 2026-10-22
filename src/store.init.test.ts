import { describe, expect, it, vi } from 'vitest'
import type { Account } from '@shared/types'

const handlers: Record<string, (payload: unknown) => void> = {}
let finishAccounts: (a: Account[]) => void = () => {}
vi.mock('./api', () => ({
  api: {
    providers: { list: async () => [] },
    accounts: { list: () => new Promise<Account[]>((r) => (finishAccounts = r)) },
    usage: { latest: async () => ({}) },
    notifications: { list: async () => [] },
    settings: { get: async () => ({}) },
    recommend: async () => [],
    on: (event: string, cb: (payload: unknown) => void) => {
      handlers[event] = cb
      return () => {}
    }
  },
  errorText: String
}))

const { useStore } = await import('./store')
const account = (status: Account['status']) => ({ id: 'a', label: 'Work', status }) as Account

describe('starting up', () => {
  it('keeps a change the backend sent while the first load was still in flight', async () => {
    const started = useStore.getState().init()
    await vi.waitFor(() => expect(handlers['accounts-changed']).toBeDefined())
    // The backend renewed the login after it answered the load, but before the answer arrived.
    handlers['accounts-changed']!([account('ok')])
    finishAccounts([account('expired')])
    await started
    expect(useStore.getState().accounts[0]!.status).toBe('ok')
  })
})
