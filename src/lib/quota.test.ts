import { describe, expect, it } from 'vitest'
import type { Account, AppSettings, UsageHistoryPoint, UsageSnapshot } from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import { bindingWindow, computeAlerts, leftOf, levelFor, litCells, unusedAtResets, wastedValue } from './quota'
import { scoreAccounts } from './recommend'

const NOW = Date.parse('2026-10-20T12:00:00Z')
const iso = (h: number) => new Date(NOW + h * 3_600_000).toISOString()

function account(id: string, extra: Partial<Account> = {}): Account {
  return {
    id,
    providerId: 'claude',
    label: id,
    homeDir: `/home/u/.${id}`,
    isDefaultHome: false,
    sharing: DEFAULT_SHARING,
    color: '#3987E5',
    tags: [],
    status: 'ok',
    createdAt: iso(-1000),
    launchCount: 0,
    ...extra
  }
}

function snap(id: string, windows: UsageSnapshot['windows'], extra: Partial<UsageSnapshot> = {}): UsageSnapshot {
  return { accountId: id, providerId: 'claude', fetchedAt: iso(0), source: 'api', windows, ...extra }
}

const settings: Pick<AppSettings, 'notifications'> = {
  notifications: { enabled: true, nearlyExhaustedPercent: 80, useItOrLoseItHours: 24, useItOrLoseItMinLeftPercent: 40, onReset: true }
}

describe('levels and cells', () => {
  it('maps remaining percent to a level', () => {
    expect(levelFor(80)).toBe('full')
    expect(levelFor(50)).toBe('full')
    expect(levelFor(49)).toBe('half')
    expect(levelFor(19)).toBe('low')
    expect(levelFor(4)).toBe('empty')
  })
  it('never rounds a non-zero remainder to zero cells', () => {
    expect(litCells(1, 20)).toBe(1)
    expect(litCells(0, 20)).toBe(0)
    expect(litCells(100, 20)).toBe(20)
    expect(litCells(36, 20)).toBe(7)
  })
  it('clamps left', () => {
    expect(leftOf({ usedPercent: 120 })).toBe(0)
    expect(leftOf({ usedPercent: -5 })).toBe(100)
  })
})

describe('bindingWindow', () => {
  it('picks the tightest general window and ignores model-only caps', () => {
    const s = snap('a', [
      { id: 'five_hour', label: '5-hour session', usedPercent: 64 },
      { id: 'seven_day', label: 'Weekly limit', usedPercent: 41 },
      { id: 'seven_day_opus', label: 'Weekly Opus', usedPercent: 90 }
    ])
    expect(bindingWindow(s)?.id).toBe('five_hour')
  })
})

describe('computeAlerts', () => {
  it('flags limits, nearly-out, use-it-or-lose-it and sign-in', () => {
    const accounts = [account('a'), account('b'), account('c'), account('d', { status: 'needs-login' })]
    const usage = {
      a: snap('a', [{ id: 'primary', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsAt: iso(1) }]),
      b: snap('b', [{ id: 'seven_day', label: 'Weekly limit', usedPercent: 96, windowMinutes: 10080, resetsAt: iso(30) }]),
      c: snap('c', [
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 30, windowMinutes: 10080, resetsAt: iso(9) },
        { id: 'seven_day_opus', label: 'Weekly Opus', usedPercent: 10, windowMinutes: 10080, resetsAt: iso(9) }
      ])
    }
    const kinds = computeAlerts(accounts, usage, settings, NOW).map((x) => `${x.kind}:${x.accountId}`)
    expect(kinds).toEqual(['use-it-or-lose-it:c', 'limit-reached:a', 'nearly-exhausted:b', 'needs-login:d'])
  })
})

describe('unused quota', () => {
  it('reads the peak before each reset', () => {
    const pts: UsageHistoryPoint[] = [10, 50, 70, 0, 20, 90, 5].map((v, i) => ({ accountId: 'a', t: iso(i), windows: { w: v } }))
    expect(unusedAtResets(pts, 'w')).toEqual([30, 10])
    expect(wastedValue([30, 10], 50)).toBe(20)
  })
})

describe('scoreAccounts', () => {
  it('puts quota about to reset ahead, and limited or signed-out accounts last', () => {
    const accounts = [account('busy'), account('expiring'), account('full'), account('out', { status: 'needs-login' })]
    const usage = {
      busy: snap('busy', [
        { id: 'five_hour', label: '5-hour session', usedPercent: 60, windowMinutes: 300 },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 50, windowMinutes: 10080, resetsAt: iso(80) }
      ]),
      expiring: snap('expiring', [
        { id: 'five_hour', label: '5-hour session', usedPercent: 0, windowMinutes: 300 },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 30, windowMinutes: 10080, resetsAt: iso(9) }
      ]),
      full: snap('full', [{ id: 'five_hour', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsAt: iso(2) }])
    }
    const recs = scoreAccounts(accounts, usage, 'coding', NOW)
    expect(recs[0].accountId).toBe('expiring')
    expect(recs[0].reason).toMatch(/use it before/)
    const zero = recs.filter((r) => r.score === 0).map((r) => r.accountId).sort()
    expect(zero).toEqual(['full', 'out'])
  })
  it('pushes accounts whose CLI is missing to the bottom', () => {
    const accounts = [account('a'), account('b', { providerId: 'copilot' })]
    const usage = {
      a: snap('a', [{ id: 'seven_day', label: 'Weekly limit', usedPercent: 70, windowMinutes: 10080 }]),
      b: snap('b', [{ id: 'premium', label: 'Premium requests', usedPercent: 0, windowMinutes: 43200 }], { providerId: 'copilot' })
    }
    const recs = scoreAccounts(accounts, usage, 'coding', NOW, undefined, { claude: true, copilot: false })
    expect(recs[0].accountId).toBe('a')
    expect(recs[1].reason).toMatch(/isn't installed/)
  })
})
