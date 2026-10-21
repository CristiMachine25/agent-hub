import { describe, expect, it } from 'vitest'
import { canDeleteHome, profilesDir } from './profiles'

describe('profilesDir', () => {
  it('uses settings.profilesDir when set, else "profiles" in the app data folder', () => {
    expect(profilesDir({ profilesDir: ' /data/p ' }, '/app')).toBe('/data/p')
    expect(profilesDir({ profilesDir: '  ' }, '/app/')).toBe('/app/profiles')
    expect(profilesDir(null, 'C:\\Users\\sam\\AppData\\Roaming\\dev.switchyard.app')).toBe('C:\\Users\\sam\\AppData\\Roaming\\dev.switchyard.app/profiles')
  })
})

describe('canDeleteHome matches the backend rule', () => {
  const profiles = '/Users/sam/Library/Application Support/dev.switchyard.app/profiles'
  const acc = (homeDir: string, isDefaultHome = false) => ({ homeDir, isDefaultHome })

  it('allows only a non-default home inside the profiles folder', () => {
    expect(canDeleteHome(acc(`${profiles}/claude/side`), profiles, 'darwin')).toBe(true)
    expect(canDeleteHome(acc(`${profiles}/claude/side/`), `${profiles}/`, 'darwin')).toBe(true)
    expect(canDeleteHome(acc(`${profiles}/claude/side`, true), profiles, 'darwin')).toBe(false)
    expect(canDeleteHome(acc('/Users/sam/.claude-personal'), profiles, 'darwin')).toBe(false)
    expect(canDeleteHome(acc(profiles), profiles, 'darwin')).toBe(false)
    expect(canDeleteHome(acc(`${profiles}-old/claude/x`), profiles, 'darwin')).toBe(false)
  })

  it('compares case-insensitively with either slash on Windows only', () => {
    const win = 'C:\\Users\\sam\\AppData\\Roaming\\dev.switchyard.app/profiles'
    expect(canDeleteHome(acc('c:\\users\\sam\\appdata\\roaming\\dev.switchyard.app\\Profiles\\gemini\\x'), win, 'win32')).toBe(true)
    expect(canDeleteHome(acc(`${profiles.toLowerCase()}/claude/x`), profiles, 'darwin')).toBe(false)
  })
})
