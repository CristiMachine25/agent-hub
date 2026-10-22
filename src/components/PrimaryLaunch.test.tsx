import { describe, expect, it, vi } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import type { Account, ProviderInfo } from '@shared/types'

vi.mock('@/api', () => ({ api: {}, errorText: String }))
const { PrimaryLaunch } = await import('./PrimaryLaunch')

const claude = (cliInstalled: boolean) =>
  ({ id: 'claude', cliInstalled, desktopInstalled: true, desktopMultiAccount: true, desktopName: 'Claude Desktop' }) as ProviderInfo
const work = { id: 'w', label: 'Work', providerId: 'claude' } as Account

describe('PrimaryLaunch', () => {
  it('launches the CLI, or the desktop app when the CLI is missing, on every Overview row alike', () => {
    let prov = claude(true)
    expect(renderToStaticMarkup(<PrimaryLaunch account={work} prov={prov} className="btn" />)).toContain('Launch Claude</button>')
    expect(renderToStaticMarkup(<PrimaryLaunch account={work} prov={prov} className="btn" short />)).toMatch(/aria-label="Launch Claude as Work".*Launch<\/button>/)

    prov = claude(false)
    expect(renderToStaticMarkup(<PrimaryLaunch account={work} prov={prov} className="btn" />)).toContain('Open Claude Desktop</button>')
    const short = renderToStaticMarkup(<PrimaryLaunch account={work} prov={prov} className="btn" short atLimit />)
    expect(short).toMatch(/aria-label="Open Claude Desktop as Work".*Open<\/button>/)
    expect(short).toContain('lucide-app-window')
    expect(short).toContain('data-tip="Limit reached: the app opens but will refuse new messages until the reset"')
  })
})
