import { beforeEach, describe, expect, it, vi } from 'vitest'

const remove = vi.fn<(id: string, opts: { deleteHome: boolean }) => Promise<boolean>>()
vi.mock('./api', () => ({
  api: { accounts: { remove }, recommend: async () => [] },
  errorText: (e: unknown) => String(e)
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
