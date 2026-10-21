import { describe, expect, it } from 'vitest'
import { createMockApi } from './backend'

describe('mock accounts behave like the real backend', () => {
  it('clears fields sent as null, keeps undefined ones and ignores protected fields', async () => {
    const api = createMockApi('darwin')
    const before = (await api.accounts.list()).find((a) => a.id === 'acc-work')!
    expect(before.monthlyCost).toBe(200)
    const after = await api.accounts.update('acc-work', {
      monthlyCost: null,
      plan: null,
      notes: undefined,
      label: 'Renamed',
      id: 'other',
      providerId: 'codex',
      launchCount: 0
    })
    expect(after.id).toBe('acc-work')
    expect(after.providerId).toBe('claude')
    expect(after.launchCount).toBe(before.launchCount)
    expect(after.label).toBe('Renamed')
    expect(after.monthlyCost).toBeUndefined()
    expect(after.plan).toBeUndefined()
    expect('monthlyCost' in after).toBe(false)
  })

  it('imports with sharing off and labels from the folder, like accounts_import', async () => {
    const api = createMockApi('darwin')
    const found = (await api.accounts.detect()).filter((d) => !d.alreadyManaged)
    const created = await api.accounts.import(found)
    expect(created.map((a) => a.label)).toEqual(['Claude H3', 'Codex Research'])
    for (const a of created) expect(Object.values(a.sharing).every((v) => v === false)).toBe(true)
    expect(created[0]!.sharedFrom).toBe('/Users/sam/.claude')
    expect(created[1]!.notes).toBe('found via CODEX_HOME in your shell profile')
  })

  it('puts new profiles under the real app data folder', async () => {
    const api = createMockApi('darwin')
    const a = await api.accounts.create({ providerId: 'claude', label: 'Side Project' })
    expect(a.homeDir).toBe('/Users/sam/Library/Application Support/dev.switchyard.app/profiles/claude/side-project')
    const linux = await createMockApi('linux').accounts.create({ providerId: 'codex', label: 'x' })
    expect(linux.homeDir).toBe('/home/sam/.local/share/dev.switchyard.app/profiles/codex/x')
    const win = await createMockApi('win32').accounts.create({ providerId: 'gemini', label: 'x' })
    expect(win.homeDir).toBe('C:\\Users\\sam\\AppData\\Roaming\\dev.switchyard.app\\profiles\\gemini\\x')
  })
})
