import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AppSettings } from '@shared/types'

const remove = vi.fn<(id: string, opts: { deleteHome: boolean }) => Promise<boolean>>()
const sets: { resolve: (s: AppSettings) => void; reject: (e: Error) => void }[] = []
vi.mock('./api', () => ({
  api: {
    accounts: { remove },
    recommend: async () => [],
    settings: { set: () => new Promise((resolve, reject) => sets.push({ resolve, reject })) }
  },
  errorText: (e: unknown) => (e instanceof Error ? e.message : String(e))
}))

const { useStore } = await import('./store')

describe('removing an account', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    useStore.setState({ toasts: [], accounts: [{ id: 'a', label: 'Work' } as never] })
  })

  it('says the folder was deleted only when the backend deleted it', async () => {
    remove.mockResolvedValueOnce(true)
    await useStore.getState().removeAccount('a', true)
    expect(useStore.getState().toasts.at(-1)?.body).toBe('Its profile folder was deleted too.')
  })

  it('says the folder was kept when the backend declined to delete it', async () => {
    remove.mockResolvedValueOnce(false)
    await useStore.getState().removeAccount('a', true)
    expect(useStore.getState().toasts.at(-1)?.body).toMatch(/^Its folder was left on disk/)
  })
})

describe('saveSettings', () => {
  beforeEach(() => {
    sets.length = 0
  })

  it("doesn't roll back a newer save when an older one fails", async () => {
    useStore.setState({ settings: { pollIntervalMinutes: 5 } as AppSettings })
    const first = useStore.getState().saveSettings({ pollIntervalMinutes: 10 })
    const second = useStore.getState().saveSettings({ pollIntervalMinutes: 15 })
    sets[1]!.resolve({ pollIntervalMinutes: 15 } as AppSettings)
    await second
    sets[0]!.reject(new Error('disk full'))
    await first
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(15)
    expect(useStore.getState().toasts.at(-1)?.title).toBe("Couldn't save settings")
  })

  it('rolls back when the latest save fails', async () => {
    useStore.setState({ settings: { pollIntervalMinutes: 5 } as AppSettings })
    const save = useStore.getState().saveSettings({ pollIntervalMinutes: 30 })
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(30)
    sets[0]!.reject(new Error('nope'))
    await save
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(5)
  })
})
