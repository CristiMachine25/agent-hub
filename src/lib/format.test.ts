import { describe, expect, it } from 'vitest'
import { creditsText, formatAgo, formatCountdown, formatPct, formatTokens, resetText, windowName } from './format'

const NOW = Date.parse('2026-10-20T12:00:00Z')
const at = (ms: number) => new Date(NOW + ms).toISOString()
const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

describe('formatCountdown', () => {
  it('handles unknown and due', () => {
    expect(formatCountdown(undefined, NOW)).toBeNull()
    expect(formatCountdown('nope', NOW)).toBeNull()
    expect(formatCountdown(at(-5 * MIN), NOW)).toBe('now')
  })
  it('uses minutes, hours+minutes, days+hours', () => {
    expect(formatCountdown(at(14 * MIN), NOW)).toBe('14m')
    expect(formatCountdown(at(2 * HOUR + 14 * MIN), NOW)).toBe('2h 14m')
    expect(formatCountdown(at(3 * HOUR), NOW)).toBe('3h')
    expect(formatCountdown(at(3 * DAY + 4 * HOUR + 10 * MIN), NOW)).toBe('3d 4h')
  })
})

describe('resetText', () => {
  it('reads naturally', () => {
    expect(resetText(at(2 * HOUR + 14 * MIN), NOW)).toBe('Resets in 2h 14m')
    expect(resetText(at(0), NOW)).toBe('Reset due')
    expect(resetText(undefined, NOW)).toBeNull()
  })
  it('switches to a date beyond a week', () => {
    expect(resetText('2026-11-01T12:00:00Z', NOW)).toBe('Resets Nov 1')
  })
})

describe('small formatters', () => {
  it('formatAgo', () => {
    expect(formatAgo(at(-20_000), NOW)).toBe('just now')
    expect(formatAgo(at(-5 * MIN), NOW)).toBe('5m ago')
    expect(formatAgo(at(-3 * HOUR), NOW)).toBe('3h ago')
    expect(formatAgo(at(-30 * HOUR), NOW)).toBe('yesterday')
  })
  it('formatTokens', () => {
    expect(formatTokens(950)).toBe('950')
    expect(formatTokens(1234)).toBe('1.2k')
    expect(formatTokens(3_400_000)).toBe('3.4M')
    expect(formatTokens(120_000_000)).toBe('120M')
  })
  it('formatPct never shows 0% for a sliver', () => {
    expect(formatPct(0.4)).toBe('<1%')
    expect(formatPct(36.4)).toBe('36%')
  })
  it('windowName keeps model names capitalised', () => {
    expect(windowName('Weekly Opus')).toBe('weekly Opus')
    expect(windowName('5-hour session')).toBe('5-hour session')
    expect(windowName('Gemini Pro daily')).toBe('Gemini Pro daily')
  })
})

describe('creditsText', () => {
  it('treats "left" units as a balance', () => {
    expect(creditsText({ used: 1250, limit: null, unit: 'credits left' })).toBe('1,250 credits left')
  })
  it('shows counts against a limit', () => {
    expect(creditsText({ used: 312, limit: 1500, unit: 'premium requests' })).toBe('312 of 1,500 premium requests used')
  })
  it('formats currency codes as money', () => {
    expect(creditsText({ used: 12.5, unit: 'USD' })).toBe('$12.50 of extra usage spent')
  })
})
