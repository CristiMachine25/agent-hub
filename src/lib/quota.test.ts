import { describe, expect, it } from 'vitest'
import type { Account, AppSettings, UsageHistoryPoint, UsageSnapshot, UsageWindow } from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import { USE_IT_MIN_WINDOW_MINUTES, bindingWindow, computeAlerts, leftOf, levelFor, litCells, unusedAtResets, wastedValue } from './quota'
import { recommend, tier } from './recommend'

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
  // Same vectors as recommend::tests::model_only_windows_do_not_bind on the Rust side.
  it('keeps an account usable when only a model window is used up', () => {
    const s = snap('a', [
      { id: 'five_hour', label: '5-hour session', usedPercent: 20 },
      { id: 'seven_day', label: 'Weekly', usedPercent: 30 },
      { id: 'seven_day_sonnet', label: 'Weekly Sonnet', usedPercent: 100 }
    ])
    const w = bindingWindow(s)
    expect(w?.id).toBe('seven_day')
    expect(w && leftOf(w)).toBe(70)
  })
  it('counts model windows when nothing else is known', () => {
    const s = snap('a', [{ id: 'seven_day_sonnet', label: 'Weekly Sonnet', usedPercent: 100 }])
    expect(bindingWindow(s)?.id).toBe('seven_day_sonnet')
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

  it('shows a blocked account once, without stale or limit alerts', () => {
    const usage = { x: snap('x', [{ id: 'primary', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsAt: iso(1) }], { stale: true }) }
    const kinds = computeAlerts([account('x', { status: 'blocked' })], usage, settings, NOW).map((x) => `${x.kind}:${x.accountId}`)
    expect(kinds).toEqual(['blocked:x'])
  })

  it('raises no limit alert for a stale window that has already reset', () => {
    const usage = {
      a: snap(
        'a',
        [
          { id: 'five_hour', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsAt: iso(-2) },
          { id: 'seven_day', label: 'Weekly limit', usedPercent: 90, windowMinutes: 10080, resetsAt: iso(-1) }
        ],
        { stale: true }
      ),
      b: snap('b', [{ id: 'five_hour', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsAt: iso(2) }], { stale: true })
    }
    const kinds = computeAlerts([account('a'), account('b')], usage, settings, NOW).map((x) => `${x.kind}:${x.accountId}`)
    expect(kinds).toEqual(['limit-reached:b', 'stale:a', 'stale:b'])
  })

  // Same vector as notify::tests::use_it_or_lose_it_needs_a_window_of_three_days on the Rust side.
  it('watches only windows of 3 days or more for use-it-or-lose-it', () => {
    const cases: [UsageWindow, boolean][] = [
      [{ id: 'daily', label: 'Weekly', usedPercent: 20, windowMinutes: 1440, resetsAt: iso(10) }, false],
      [{ id: 'premium', label: 'Weekly', usedPercent: 20, resetsAt: iso(10) }, false],
      [{ id: 'three_day', label: 'Weekly', usedPercent: 20, windowMinutes: 4320, resetsAt: iso(10) }, true]
    ]
    for (const [w, fires] of cases) {
      const kinds = computeAlerts([account('a')], { a: snap('a', [w]) }, settings, NOW).map((x) => x.kind)
      expect(kinds.includes('use-it-or-lose-it'), w.id).toBe(fires)
    }
    expect(USE_IT_MIN_WINDOW_MINUTES).toBe(3 * 24 * 60)
  })

  it('shows an expired login as stale numbers, not as signed out', () => {
    const usage = { e: snap('e', [], { stale: true, error: 'The login expired; renewing it now.' }) }
    const kinds = computeAlerts([account('e', { status: 'expired' })], usage, settings, NOW).map((x) => `${x.kind}:${x.accountId}`)
    expect(kinds).toEqual(['stale:e'])
  })
})

describe('unused quota', () => {
  it('reads the peak before each reset', () => {
    const pts: UsageHistoryPoint[] = [10, 50, 70, 0, 20, 90, 5].map((v, i) => ({ accountId: 'a', t: iso(i), windows: { w: v } }))
    expect(unusedAtResets(pts, 'w')).toEqual([30, 10])
    expect(wastedValue([30, 10], 50)).toBe(20)
  })
})

// The same vectors as the tests in src-tauri/src/recommend.rs, which this port mirrors.
describe('recommend', () => {
  const win = (id: string, used: number, resetsInH: number | undefined, minutes: number): UsageWindow => ({
    id,
    label: minutes < 1440 ? '5-hour session' : 'Weekly',
    usedPercent: used,
    resetsAt: resetsInH === undefined ? undefined : iso(resetsInH),
    windowMinutes: minutes
  })
  const plan = (id: string, p: string, extra: Partial<Account> = {}) => account(id, { plan: p, ...extra })

  it('ranks exhausted and signed-out accounts last', () => {
    const accounts = [plan('a', 'Pro'), plan('b', 'Pro'), plan('c', 'Max', { status: 'needs-login' })]
    const usage = { a: snap('a', [win('five_hour', 100, 2, 300)]), b: snap('b', [win('five_hour', 30, 2, 300)]) }
    const r = recommend('coding', undefined, accounts, usage, [], NOW)
    expect(r[0]!.accountId).toBe('b')
    expect(r[0]!.reason.startsWith('70% of 5-hour session left')).toBe(true)
    expect(r.find((x) => x.accountId === 'a')?.reason).toContain('limit reached')
    expect(r.every((x) => x.score >= 0 && x.score <= 100)).toBe(true)
    // Overview's "Use now" only offers accounts scoring above 0.
    expect(r.filter((x) => x.score === 0).map((x) => x.accountId)).toEqual(['a', 'c'])
  })

  // Same vectors as recommend::tests::blocked_accounts_are_never_recommended on the Rust side.
  it('never recommends a blocked account', () => {
    const a = plan('a', 'Max', { status: 'blocked', statusMessage: "This account's subscription was canceled." })
    const cases: Record<string, UsageSnapshot>[] = [{}, { a: snap('a', [win('five_hour', 10, 2, 300)]) }]
    for (const usage of cases) {
      const r = recommend('any', undefined, [a], usage, [], NOW)[0]!
      expect(r.score).toBe(0)
      expect(r.reason).toBe("Can't be used: This account's subscription was canceled.")
    }
  })

  it('prefers Max for a long refactor and the small account for a quick question', () => {
    const accounts = [plan('max', 'Max 20x'), plan('pro', 'Pro')]
    const usage = { max: snap('max', [win('five_hour', 20, 3, 300)]), pro: snap('pro', [win('five_hour', 60, 3, 300)]) }
    expect(recommend('long-refactor', undefined, accounts, usage, [], NOW)[0]!.accountId).toBe('max')
    expect(recommend('quick-question', undefined, accounts, usage, [], NOW)[0]!.accountId).toBe('pro')
  })

  it('pushes up a weekly window that resets soon', () => {
    const accounts = [plan('a', 'Pro'), plan('b', 'Pro')]
    const usage = { a: snap('a', [win('seven_day', 40, 10, 10080)]), b: snap('b', [win('seven_day', 40, 100, 10080)]) }
    const r = recommend('any', undefined, accounts, usage, [], NOW)
    expect(r[0]!.accountId).toBe('a')
    expect(r[0]!.reason).toContain('Use it before it resets')
  })

  it('never scores a usable account 0', () => {
    const accounts = [plan('a', 'Max 20x'), plan('b', 'Max 20x')]
    const usage = { a: snap('a', [win('five_hour', 97, 2, 300)]), b: snap('b', [win('five_hour', 100, 2, 300)]) }
    const r = recommend('quick-question', undefined, accounts, usage, [], NOW)
    expect(r.find((x) => x.accountId === 'a')?.score).toBeGreaterThanOrEqual(1)
    expect(r.find((x) => x.accountId === 'b')?.score).toBe(0)
  })

  it('sorts accounts without their CLI last but keeps them usable', () => {
    const accounts = [
      plan('a', 'Pro'),
      plan('b', 'Pro', { providerId: 'codex' }),
      plan('c', 'Pro', { providerId: 'codex' }),
      plan('d', 'Pro', { providerId: 'codex', status: 'needs-login' })
    ]
    const usage = {
      a: snap('a', [win('five_hour', 30, undefined, 300)]),
      b: snap('b', [win('five_hour', 90, undefined, 300)]),
      c: snap('c', [win('five_hour', 10, undefined, 300)])
    }
    const r = recommend('any', undefined, accounts, usage, ['codex'], NOW)
    // Still usable (its desktop app may work), so 1 and not 0; the one with more left comes first.
    expect(r.map((x) => [x.accountId, x.score])).toEqual([['a', 45.5], ['c', 1], ['b', 1], ['d', 0]])
    expect(r[1]!.reason).toBe("90% of 5-hour session left. The Codex CLI isn't installed on this computer.")
    expect(r[3]!.reason).toBe('Needs login before it can be used.')
    // Without the CLI check the same accounts rank on quota alone.
    expect(recommend('any', undefined, accounts, usage, [], NOW)[0]!.accountId).toBe('c')
  })

  it('filters by provider and rewards matching tags', () => {
    const accounts = [plan('u', 'Pro'), plan('t', 'Pro', { tags: ['Research'] }), plan('o', 'Pro', { providerId: 'codex' })]
    const r = recommend('research', 'claude', accounts, {}, [], NOW)
    expect(r.map((x) => x.accountId)).toEqual(['t', 'u'])
    expect(tier({ plan: 'Copilot Pro+', tags: [] })).toBe(3)
    expect(tier({ plan: 'Plus', tags: [] })).toBe(1)
    expect(tier({ plan: 'Pro 200', tags: [] })).toBe(3)
    expect(tier({ plan: 'Team Premium', tags: [] })).toBe(3)
    expect(tier({ plan: 'Google AI Pro', tags: [] })).toBe(2)
    expect(tier({ plan: 'Google AI Plus', tags: [] })).toBe(1)
  })

  it('ignores a used-up model-only window', () => {
    const sonnet = { ...win('seven_day_sonnet', 100, 30, 10080), label: 'Weekly Sonnet' }
    const s = snap('a', [win('five_hour', 20, 2, 300), win('seven_day', 30, 100, 10080), sonnet])
    const r = recommend('any', undefined, [plan('a', 'Pro')], { a: s }, [], NOW)[0]!
    expect(r.score).toBeGreaterThan(10)
    expect(r.reason.startsWith('70% of Weekly left')).toBe(true)
  })
})
