import { describe, expect, it } from 'vitest'
import type { Account, TerminalInfo, UsageSnapshot } from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import {
  LAYOUTS,
  LAYOUT_IDS,
  atLimit,
  continueCandidates,
  fitPanes,
  historyShared,
  historyToTurnOn,
  isSignedOut,
  isUsable,
  layoutFor,
  statusLabel,
  neighbor,
  parseWorkspace,
  pickerGroups,
  reattach,
  serializeWorkspace,
  type Workspace
} from './grid'

const seq = () => {
  let n = 0
  return () => `u${++n}`
}

describe('layouts', () => {
  it('has one rect per pane and every rect inside the grid', () => {
    for (const id of LAYOUT_IDS) {
      const l = LAYOUTS[id]
      expect(l.rects).toHaveLength(id)
      for (const r of l.rects) {
        expect(r.col + r.colSpan).toBeLessThanOrEqual(l.cols)
        expect(r.row + r.rowSpan).toBeLessThanOrEqual(l.rows)
      }
    }
  })

  it('covers every grid cell exactly once', () => {
    for (const id of LAYOUT_IDS) {
      const l = LAYOUTS[id]
      const cells = new Map<string, number>()
      for (const r of l.rects)
        for (let c = r.col; c < r.col + r.colSpan; c++)
          for (let w = r.row; w < r.row + r.rowSpan; w++) cells.set(`${c},${w}`, (cells.get(`${c},${w}`) ?? 0) + 1)
      expect(cells.size).toBe(l.cols * l.rows)
      expect([...cells.values()].every((v) => v === 1)).toBe(true)
    }
  })

  it('picks the smallest layout that fits', () => {
    expect(layoutFor(0)).toBe(1)
    expect(layoutFor(1)).toBe(1)
    expect(layoutFor(3)).toBe(3)
    expect(layoutFor(5)).toBe(6)
    expect(layoutFor(7)).toBe(9)
    expect(layoutFor(20)).toBe(9)
  })
})

describe('neighbor', () => {
  const r4 = LAYOUTS[4].rects // 0 1 / 2 3
  const r9 = LAYOUTS[9].rects
  const r3 = LAYOUTS[3].rects // tall 0 | 1 over 2

  it('moves in a 2x2 grid and stops at edges', () => {
    expect(neighbor(r4, 0, 'right')).toBe(1)
    expect(neighbor(r4, 0, 'down')).toBe(2)
    expect(neighbor(r4, 3, 'left')).toBe(2)
    expect(neighbor(r4, 3, 'up')).toBe(1)
    expect(neighbor(r4, 0, 'left')).toBe(0)
    expect(neighbor(r4, 0, 'up')).toBe(0)
  })

  it('takes the nearest pane, not one further away', () => {
    expect(neighbor(r9, 3, 'right')).toBe(4)
    expect(neighbor(r9, 1, 'down')).toBe(4)
    expect(neighbor(r9, 8, 'up')).toBe(5)
  })

  it('handles spanning panes', () => {
    expect(neighbor(r3, 0, 'right')).toBe(1) // tie between 1 and 2: same center distance, first wins
    expect(neighbor(r3, 2, 'left')).toBe(0)
    expect(neighbor(r3, 1, 'down')).toBe(2)
    expect(neighbor(r3, 0, 'down')).toBe(0)
  })

  it('returns the same index for unknown panes', () => {
    expect(neighbor(r4, 7, 'right')).toBe(7)
  })
})

describe('fitPanes', () => {
  it('grows with empty panes', () => {
    const { panes, removed } = fitPanes([{ uid: 'a', target: 'cli', accountId: 'x' }], 3, seq())
    expect(panes.map((p) => p.uid)).toEqual(['a', 'u1', 'u2'])
    expect(removed).toEqual([])
  })

  it('drops empty panes first, then idle ones, keeping running ones', () => {
    const panes = [
      { uid: 'run', target: 'cli' as const, accountId: 'x', terminalId: 't1' },
      { uid: 'empty', target: 'cli' as const },
      { uid: 'idle', target: 'cli' as const, accountId: 'y' },
      { uid: 'run2', target: 'cli' as const, accountId: 'z', terminalId: 't2' }
    ]
    const r = fitPanes(panes, 2)
    expect(r.panes.map((p) => p.uid)).toEqual(['run', 'run2'])
    expect(r.removed.map((p) => p.uid).sort()).toEqual(['empty', 'idle'])
  })
})

describe('workspace persistence', () => {
  const ws: Workspace = {
    version: 1,
    layout: 4,
    focused: 2,
    maximized: null,
    panes: [
      { uid: 'a', target: 'cli', accountId: 'acc-work', cwd: 'C:\\code\\api', terminalId: 't1', resume: true },
      { uid: 'b', target: 'login', accountId: 'acc-h2' },
      { uid: 'c', target: 'cli' },
      { uid: 'd', target: 'cli' }
    ]
  }

  it('round-trips', () => {
    expect(parseWorkspace(serializeWorkspace(ws))).toEqual(ws)
  })

  it('does not store transient fields', () => {
    const withExtra = { ...ws, panes: ws.panes.map((p) => ({ ...p, wantStart: true })) }
    expect(serializeWorkspace(withExtra)).not.toContain('wantStart')
  })

  it('falls back on garbage', () => {
    for (const raw of [null, '', 'not json', '42', '{"layout":5,"panes":"x"}']) {
      const w = parseWorkspace(raw, seq())
      expect(w.layout).toBe(1)
      expect(w.panes).toHaveLength(1)
      expect(w.focused).toBe(0)
    }
  })

  it('normalizes pane count, duplicate uids and out-of-range indexes', () => {
    const raw = JSON.stringify({
      layout: 2,
      focused: 9,
      maximized: 1,
      panes: [{ uid: 'a', target: 'weird' }, { uid: 'a', accountId: 5 }, { uid: 'z' }]
    })
    const w = parseWorkspace(raw, seq())
    expect(w.panes).toHaveLength(2)
    expect(new Set(w.panes.map((p) => p.uid)).size).toBe(2)
    expect(w.panes[0]!.target).toBe('cli')
    expect(w.panes[1]!.accountId).toBeUndefined()
    expect(w.focused).toBe(0)
    expect(w.maximized).toBe(1)
  })
})

const term = (id: string, accountId: string, exited = false): TerminalInfo => ({
  id,
  accountId,
  title: id,
  commandLine: 'claude',
  cwd: '/code',
  startedAt: '2026-10-20T10:00:00Z',
  exited
})

describe('reattach', () => {
  const ws: Workspace = {
    version: 1,
    layout: 2,
    focused: 1,
    maximized: null,
    panes: [
      { uid: 'a', target: 'cli', accountId: 'x', terminalId: 't1' },
      { uid: 'b', target: 'cli', accountId: 'y', terminalId: 'gone' }
    ]
  }

  it('keeps live terminals and forgets missing ones without respawning', () => {
    const w = reattach(ws, [term('t1', 'x')])
    expect(w.panes[0]!.terminalId).toBe('t1')
    expect(w.panes[1]!.terminalId).toBeUndefined()
    expect(w.panes[1]!.accountId).toBe('y')
  })

  it('keeps exited terminals so their output and code still show', () => {
    expect(reattach(ws, [term('t1', 'x', true)]).panes[0]!.terminalId).toBe('t1')
  })

  it('places unknown running terminals into empty panes and grows the layout', () => {
    const w = reattach(ws, [term('t1', 'x'), term('t9', 'q'), term('t8', 'r'), term('dead', 's', true)], seq())
    expect(w.layout).toBe(4)
    expect(w.panes.map((p) => p.terminalId)).toEqual(['t1', undefined, 't9', 't8'])
    expect(w.panes[2]!.accountId).toBe('q')
  })
})

const acc = (id: string, providerId: Account['providerId'], extra: Partial<Account> = {}): Account => ({
  id,
  providerId,
  label: id,
  homeDir: `/h/${id}`,
  isDefaultHome: false,
  sharing: { ...DEFAULT_SHARING },
  color: '#3987E5',
  tags: [],
  status: 'ok',
  createdAt: '2026-01-01T00:00:00Z',
  launchCount: 0,
  ...extra
})
const snap = (accountId: string, used: number): UsageSnapshot => ({
  accountId,
  providerId: 'claude',
  fetchedAt: '2026-10-20T10:00:00Z',
  source: 'api',
  windows: [{ id: 'five_hour', label: '5-hour session', usedPercent: used }]
})

describe('accounts in panes', () => {
  const accounts = [
    acc('h1', 'claude'),
    acc('h2', 'claude'),
    acc('h3', 'claude'),
    acc('out', 'claude', { status: 'needs-login' }),
    acc('gpt', 'codex')
  ]
  const usage = { h1: snap('h1', 100), h2: snap('h2', 0), h3: snap('h3', 40) }

  it('detects a used-up binding window', () => {
    expect(atLimit(usage.h1)).toBe(true)
    expect(atLimit(snap('x', 99.4))).toBe(false)
    expect(atLimit(undefined)).toBe(false)
  })

  it('lists same-provider accounts, usable and best first', () => {
    const recs = [
      { accountId: 'h3', score: 80, reason: '' },
      { accountId: 'h2', score: 60, reason: '' }
    ]
    expect(continueCandidates(accounts[0]!, accounts, usage, recs).map((a) => a.id)).toEqual(['h3', 'h2', 'out'])
    expect(continueCandidates(accounts[1]!, accounts, usage, []).map((a) => a.id)).toEqual(['h3', 'h1', 'out'])
  })

  it('keeps an expired login usable: starting the CLI renews it', () => {
    const expired = acc('exp', 'claude', { status: 'expired' })
    expect(isSignedOut(expired)).toBe(false)
    expect(statusLabel(expired)).toBe('Login expired')
    expect(statusLabel(accounts[3]!)).toBe('Signed out')
    expect(statusLabel(accounts[0]!)).toBeNull()
    expect(continueCandidates(accounts[2]!, [...accounts, expired], usage, []).map((a) => a.id)).toEqual(['h2', 'exp', 'h1', 'out'])
  })

  it('never offers a blocked account: signing in again won\'t fix it', () => {
    const blocked = acc('blk', 'claude', { status: 'blocked' })
    expect(isSignedOut(blocked)).toBe(false)
    expect(isUsable(blocked, usage)).toBe(false)
    expect(isUsable(accounts[1]!, usage)).toBe(true)
    expect(statusLabel(blocked)).toBe('No access')
    const recs = [
      { accountId: 'h1', score: 0, reason: '' },
      { accountId: 'blk', score: 0, reason: '' }
    ]
    expect(continueCandidates(accounts[2]!, [blocked, ...accounts], usage, recs).map((a) => a.id)).toEqual(['h2', 'h1', 'blk', 'out'])
    expect(pickerGroups([blocked, ...accounts.slice(0, 4)], recs)[0]!.accounts.map((a) => a.id)).toEqual(['h1', 'h2', 'h3', 'out', 'blk'])
  })

  it('knows when history is shared', () => {
    const on = acc('a', 'claude')
    const off = acc('b', 'claude', { sharing: { ...DEFAULT_SHARING, history: false } })
    const main = acc('m', 'claude', { isDefaultHome: true, sharing: { ...DEFAULT_SHARING, history: false } })
    // Not flagged, but at the path the tool uses by default: still the main folder.
    const atMainPath = acc('p', 'claude', { homeDir: '/Users/sam/.claude/', sharing: { ...DEFAULT_SHARING, history: false } })
    expect(historyShared(on, on, undefined, 'darwin')).toBe(true)
    expect(historyShared(on, off, undefined, 'darwin')).toBe(false)
    expect(historyShared(main, on, undefined, 'darwin')).toBe(true)
    expect(historyShared(atMainPath, on, '/Users/sam/.claude', 'darwin')).toBe(true)
    expect(historyShared(atMainPath, on, undefined, 'darwin')).toBe(false)
  })

  it('turns history on only for the accounts that need it, never the main folder', () => {
    const off = { ...DEFAULT_SHARING, history: false }
    const work = acc('w', 'claude', { sharing: off })
    const atMainPath = acc('p', 'claude', { homeDir: '/Users/sam/.claude', sharing: off })
    const main = acc('m', 'claude', { isDefaultHome: true, sharing: off })
    const on = acc('o', 'claude')
    expect(historyToTurnOn([atMainPath, work], '/Users/sam/.claude', 'darwin').map((a) => a.id)).toEqual(['w'])
    expect(historyToTurnOn([main, on], '/Users/sam/.claude', 'darwin')).toEqual([])
  })

  it('orders picker groups by their best account', () => {
    const recs = [
      { accountId: 'gpt', score: 90, reason: '' },
      { accountId: 'h2', score: 70, reason: '' },
      { accountId: 'h1', score: 0, reason: '' }
    ]
    const g = pickerGroups(accounts, recs)
    expect(g.map((x) => x.providerId)).toEqual(['codex', 'claude'])
    expect(g[1]!.accounts.map((a) => a.id)).toEqual(['h2', 'h1', 'h3', 'out'])
  })
})
