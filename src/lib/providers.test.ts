import { describe, expect, it } from 'vitest'
import { primaryTarget, primaryVerb } from './providers'

describe('primaryTarget', () => {
  it('opens the desktop app only when the CLI is missing and the app is installed', () => {
    expect(primaryTarget({ cliInstalled: false, desktopInstalled: true })).toBe('desktop')
    expect(primaryTarget({ cliInstalled: true, desktopInstalled: true })).toBe('cli')
    expect(primaryTarget({ cliInstalled: false, desktopInstalled: false })).toBe('cli')
    expect(primaryTarget(undefined)).toBe('cli')
    expect(primaryVerb('claude', { cliInstalled: false, desktopInstalled: true, desktopName: 'Claude Desktop' })).toBe('Open Claude Desktop')
    expect(primaryVerb('claude', { cliInstalled: true, desktopInstalled: true, desktopName: 'Claude Desktop' })).toBe('Launch Claude')
  })
})
