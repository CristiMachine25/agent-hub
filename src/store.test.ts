import { describe, expect, it, vi } from 'vitest'
import type { AppSettings } from '@shared/types'

const sets: { resolve: (s: AppSettings) => void; reject: (e: Error) => void }[] = []
vi.mock('./api', () => ({
  api: { settings: { set: () => new Promise((resolve, reject) => sets.push({ resolve, reject })) } },
  errorText: (e: Error) => e.message
}))

const { useStore } = await import('./store')

describe('saveSettings', () => {
  it("doesn't roll back a newer save when an older one fails", async () => {
    useStore.setState({ settings: { pollIntervalMinutes: 5 } as AppSettings })
    const first = useStore.getState().saveSettings({ pollIntervalMinutes: 10 })
    const second = useStore.getState().saveSettings({ pollIntervalMinutes: 15 })
    sets[1].resolve({ pollIntervalMinutes: 15 } as AppSettings)
    await second
    sets[0].reject(new Error('disk full'))
    await first
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(15)
    expect(useStore.getState().toasts.at(-1)?.title).toBe("Couldn't save settings")
  })

  it('rolls back when the latest save fails', async () => {
    sets.length = 0
    useStore.setState({ settings: { pollIntervalMinutes: 5 } as AppSettings })
    const save = useStore.getState().saveSettings({ pollIntervalMinutes: 30 })
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(30)
    sets[0].reject(new Error('nope'))
    await save
    expect(useStore.getState().settings?.pollIntervalMinutes).toBe(5)
  })
})
