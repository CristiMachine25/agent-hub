import { describe, expect, it } from 'vitest'
import { primaryTarget, primaryVerb } from './providers'

describe('primaryTarget', () => {
  const app = { desktopInstalled: true, desktopMultiAccount: true, desktopName: 'Claude Desktop' }
  it('opens the desktop app only when the CLI is missing and the app can run the chosen account', () => {
    expect(primaryTarget({ ...app, cliInstalled: false })).toBe('desktop')
    expect(primaryTarget({ ...app, cliInstalled: true })).toBe('cli')
    expect(primaryTarget({ ...app, cliInstalled: false, desktopInstalled: false })).toBe('cli')
    // A one-account app (the Codex app) opens whichever account it used last, not the recommended one.
    expect(primaryTarget({ ...app, cliInstalled: false, desktopMultiAccount: false })).toBe('cli')
    expect(primaryTarget(undefined)).toBe('cli')
    expect(primaryVerb('claude', { ...app, cliInstalled: false })).toBe('Open Claude Desktop')
    expect(primaryVerb('claude', { ...app, cliInstalled: true })).toBe('Launch Claude')
  })
})
