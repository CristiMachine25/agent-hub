import { describe, expect, it, vi } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import type { Account, ProviderInfo, ProviderKnowledge } from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'

vi.mock('@/api', () => ({ api: { platform: 'darwin', on: () => () => {} }, errorText: String }))
// The terminal grid store touches the DOM when it loads; these screens only call it on click.
vi.mock('@/terminals/store', () => ({ useGrid: { getState: () => ({}) } }))
// The charts read CSS variables from the DOM.
vi.mock('@/charts/AccountCharts', () => ({ DailyTokens: () => null, WindowHistory: () => null }))
const { useStore } = await import('@/store')
const { Accounts } = await import('./Accounts')
const { UsedBy } = await import('./Knowledge')

// Saved by an older build: at the tool's default folder, but without the flag.
const old: Account = {
  id: 'old',
  providerId: 'claude',
  label: 'Old',
  homeDir: '/Users/sam/.claude/',
  isDefaultHome: false,
  sharing: { ...DEFAULT_SHARING, history: false },
  color: '#3987E5',
  tags: [],
  status: 'ok',
  createdAt: '2026-01-01T00:00:00Z',
  launchCount: 0
}
// renderToStaticMarkup reads the store's initial state (zustand's server snapshot), so that is what gets set.
const seed = (s: Partial<ReturnType<typeof useStore.getState>>) => Object.assign(useStore.getInitialState(), s)
const claude = { id: 'claude', name: 'Claude Code', homeEnvVar: 'CLAUDE_CONFIG_DIR', defaultHome: '/Users/sam/.claude' } as ProviderInfo

describe('an unflagged account at the default folder', () => {
  it('reads as the main folder on the Accounts screen', () => {
    seed({ accounts: [old], providers: [claude], selectedId: 'old' })
    const html = renderToStaticMarkup(<Accounts />)
    expect(html).toContain('main folder, so it launches without any override')
    expect(html).toContain('This is your main Claude setup')
    expect(html).not.toContain('CLAUDE_CONFIG_DIR')
  })

  it('reads as the main folder on the Knowledge screen once the backend flags it', () => {
    seed({ accounts: [old], providers: [claude] })
    const k: ProviderKnowledge = {
      providerId: 'claude',
      folder: '/x/shared/claude',
      mainFolder: '/Users/sam/.claude',
      items: [],
      accounts: [{ accountId: 'old', label: 'Old', isDefaultHome: true, kinds: [] }]
    }
    expect(renderToStaticMarkup(<UsedBy k={k} />)).toContain('Uses its own folder (main)')
  })
})
