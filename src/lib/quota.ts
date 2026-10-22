import type { Account, AppSettings, UsageSnapshot, UsageWindow, UsageHistoryPoint } from '@shared/types'

export type Level = 'full' | 'half' | 'low' | 'empty'

function clampPct(n: number): number {
  if (!Number.isFinite(n)) return 0
  return Math.min(100, Math.max(0, n))
}

/** Percent of a window still available. */
export function leftOf(w: Pick<UsageWindow, 'usedPercent'>): number {
  return clampPct(100 - w.usedPercent)
}

/** Color level for a remaining percentage. Full >= 50, half 20-49, low 5-19, empty < 5. */
export function levelFor(left: number): Level {
  if (left >= 50) return 'full'
  if (left >= 20) return 'half'
  if (left >= 5) return 'low'
  return 'empty'
}

/** Plain words for a level, so color is never the only signal. */
export function levelWord(left: number): string {
  if (left <= 0.5) return 'Limit reached'
  switch (levelFor(left)) {
    case 'full':
      return 'Plenty left'
    case 'half':
      return 'Some left'
    case 'low':
      return 'Running low'
    default:
      return 'Nearly out'
  }
}

/** Lit cells out of `cells` for a remaining percent; a non-zero value never rounds down to zero cells. */
export function litCells(left: number, cells = 20): number {
  const l = clampPct(left)
  if (l <= 0.5) return 0
  return Math.min(cells, Math.max(1, Math.round((l / 100) * cells)))
}

/** Windows that only cap one model (e.g. "Weekly Opus"): running out still leaves the account usable. */
export function isModelWindow(w: Pick<UsageWindow, 'id'>): boolean {
  return /opus|sonnet|haiku/i.test(w.id)
}

/**
 * The window with the least left: the one that actually limits the account right now.
 * Model-specific windows are ignored when a general window exists.
 */
export function bindingWindow(snap: UsageSnapshot | undefined): UsageWindow | undefined {
  if (!snap || snap.windows.length === 0) return undefined
  const general = snap.windows.filter((w) => !isModelWindow(w))
  const pool = general.length ? general : snap.windows
  return pool.reduce((a, b) => (leftOf(b) < leftOf(a) ? b : a))
}

/** The longest window (weekly or monthly): the one worth charting over time. */
export function longWindow(snap: UsageSnapshot | undefined): UsageWindow | undefined {
  if (!snap || snap.windows.length === 0) return undefined
  const general = snap.windows.filter((w) => !isModelWindow(w))
  const pool = general.length ? general : snap.windows
  return pool.reduce((a, b) => ((b.windowMinutes ?? 0) > (a.windowMinutes ?? 0) ? b : a))
}

export type AlertKind = 'limit-reached' | 'nearly-exhausted' | 'use-it-or-lose-it' | 'needs-login' | 'blocked' | 'stale'

export interface Alert {
  kind: AlertKind
  accountId: string
  windowId?: string
  left?: number
  resetsAt?: string
}

/** "Use it or lose it" only watches windows at least this long; same as notify.rs. Unknown length doesn't count. */
export const USE_IT_MIN_WINDOW_MINUTES = 3 * 24 * 60

const ALERT_RANK: Record<AlertKind, number> = {
  'use-it-or-lose-it': 0,
  'limit-reached': 1,
  'nearly-exhausted': 2,
  'needs-login': 3,
  blocked: 4,
  stale: 5
}

/** Things worth surfacing on the dashboard right now, most actionable first. */
export function computeAlerts(
  accounts: Account[],
  usage: Record<string, UsageSnapshot>,
  settings: Pick<AppSettings, 'notifications'>,
  now: number = Date.now()
): Alert[] {
  const out: Alert[] = []
  const n = settings.notifications
  for (const a of accounts) {
    // An expired login renews by itself: it shows as stale (below) until it does.
    if (a.status === 'needs-login') {
      out.push({ kind: 'needs-login', accountId: a.id })
      continue
    }
    // Blocked is permanent: its reason replaces any stale numbers.
    if (a.status === 'blocked') {
      out.push({ kind: 'blocked', accountId: a.id })
      continue
    }
    const snap = usage[a.id]
    if (!snap) continue
    if (snap.stale) out.push({ kind: 'stale', accountId: a.id })
    for (const w of snap.windows) {
      const left = leftOf(w)
      // Old numbers for a window that has reset since say nothing about its limit now.
      const current = !snap.stale || !w.resetsAt || Date.parse(w.resetsAt) > now
      if (current && left <= 0.5) {
        out.push({ kind: 'limit-reached', accountId: a.id, windowId: w.id, left, resetsAt: w.resetsAt })
      } else if (current && w.usedPercent >= n.nearlyExhaustedPercent && left < 20) {
        out.push({ kind: 'nearly-exhausted', accountId: a.id, windowId: w.id, left, resetsAt: w.resetsAt })
      }
      if (w.resetsAt && (w.windowMinutes ?? 0) >= USE_IT_MIN_WINDOW_MINUTES && !isModelWindow(w)) {
        const hours = (Date.parse(w.resetsAt) - now) / 3_600_000
        if (hours > 0 && hours <= n.useItOrLoseItHours && left >= n.useItOrLoseItMinLeftPercent) {
          out.push({ kind: 'use-it-or-lose-it', accountId: a.id, windowId: w.id, left, resetsAt: w.resetsAt })
        }
      }
    }
  }
  // One use-it-or-lose-it per account: keep the window with the most left.
  const best = new Map<string, Alert>()
  for (const al of out) {
    if (al.kind !== 'use-it-or-lose-it') continue
    const prev = best.get(al.accountId)
    if (!prev || (al.left ?? 0) > (prev.left ?? 0)) best.set(al.accountId, al)
  }
  return out
    .filter((al) => al.kind !== 'use-it-or-lose-it' || best.get(al.accountId) === al)
    .sort((x, y) => ALERT_RANK[x.kind] - ALERT_RANK[y.kind])
}

/**
 * Percent left unused each time a window reset, found as drops in the history series.
 * A drop of more than 15 points between two samples counts as a reset; the value just before it is the peak.
 */
export function unusedAtResets(points: UsageHistoryPoint[], windowId: string): number[] {
  const series = points
    .filter((p) => windowId in p.windows)
    .sort((a, b) => Date.parse(a.t) - Date.parse(b.t))
    .map((p) => p.windows[windowId]!) // present: filtered with `in` above
  const out: number[] = []
  for (let i = 1; i < series.length; i++) {
    if (series[i - 1]! - series[i]! > 15) out.push(clampPct(100 - series[i - 1]!))
  }
  return out
}

/** Rough dollar value of quota that reset unused, given what one window period is worth. */
export function wastedValue(unused: number[], valuePerPeriod: number): number {
  return unused.reduce((s, u) => s + (u / 100) * valuePerPeriod, 0)
}
