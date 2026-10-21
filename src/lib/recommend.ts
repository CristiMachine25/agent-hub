import type { Account, ProviderId, Recommendation, TaskType, UsageSnapshot, UsageWindow } from '@shared/types'
import { PROVIDER_META } from './providers'
import { isModelWindow, leftOf } from './quota'

// A line-for-line port of src-tauri/src/recommend.rs, which is what the app ships. Only the browser
// preview (src/mock/backend.ts) runs this copy; the tests in quota.test.ts use the Rust tests' vectors.
// Change both together.

/** A window that resets within this many hours with plenty left earns a "use it or lose it" bonus. */
const RESET_SOON_HOURS = 24

const clamp = (n: number) => Math.min(100, Math.max(0, n))

/** recommend::tightest: the most-used window, ignoring model-only windows when a general one exists. */
function tightest(snap: UsageSnapshot): UsageWindow | undefined {
  const onlyModel = snap.windows.every(isModelWindow)
  return snap.windows
    .filter((w) => onlyModel || !isModelWindow(w))
    .reduce<UsageWindow | undefined>((a, b) => (!a || b.usedPercent >= a.usedPercent ? b : a), undefined)
}

/** recommend::is_short_window: 5-hour sessions reset constantly, so they never count as "about to be lost". */
function isShortWindow(w: UsageWindow): boolean {
  if (w.windowMinutes != null) return w.windowMinutes < 24 * 60
  const id = w.id.toLowerCase()
  const label = w.label.toLowerCase()
  return id.includes('five_hour') || id.includes('5h') || label.includes('5-hour') || label.includes('hour')
}

/** util::human_until */
function humanUntil(t: number, now: number): string {
  const m = Math.trunc((t - now) / 60_000)
  if (m <= 0) return 'now'
  if (m < 60) return `in ${m} min`
  if (m < 48 * 60) {
    const h = Math.floor(m / 60)
    const rem = m % 60
    return h < 6 && rem >= 5 ? `in ${h} h ${rem} min` : `in ${Math.floor((m + 30) / 60)} h`
  }
  return `in ${Math.floor((m + 720) / 1440)} days`
}

function parseIso(s: string | undefined): number | null {
  const t = s ? Date.parse(s) : NaN
  return Number.isFinite(t) ? t : null
}

/** 0 = unknown/free, 1 = Plus, 2 = Pro/Team/Business, 3 = Max/Pro+/Ultra/Enterprise and ChatGPT Pro 100/200/500. */
export function tier(account: Pick<Account, 'plan' | 'tags'>): number {
  const text = [account.plan ?? '', ...account.tags].join(' ').toLowerCase()
  if (['max', 'pro+', 'pro plus', 'ultra', 'enterprise', 'pro 100', 'pro 200', 'pro 500', 'premium'].some((k) => text.includes(k))) return 3
  if (['pro', 'team', 'business'].some((k) => text.includes(k))) return 2
  if (text.includes('plus')) return 1
  return 0
}

const TASK_KEYWORDS: Record<TaskType, string[]> = {
  coding: ['coding', 'code', 'dev', 'development'],
  'long-refactor': ['long-refactor', 'refactor', 'long', 'heavy'],
  'quick-question': ['quick-question', 'quick', 'chat', 'questions'],
  research: ['research'],
  review: ['review', 'reviews'],
  any: []
}

const rec = (account: Account, score: number, reason: string): Recommendation => ({
  accountId: account.id,
  score: Math.round(clamp(score) * 10) / 10,
  reason
})

/** A long window (weekly, monthly) resetting within RESET_SOON_HOURS with at least 30% left. */
function useItOrLoseIt(snap: UsageSnapshot, now: number): [UsageWindow, number] | undefined {
  let best: [UsageWindow, number] | undefined
  for (const w of snap.windows) {
    if (isShortWindow(w)) continue
    const t = parseIso(w.resetsAt)
    if (t === null) continue
    const left = leftOf(w)
    if (t > now && Math.trunc((t - now) / 3_600_000) < RESET_SOON_HOURS && left >= 30 && (!best || left >= best[1])) best = [w, left]
  }
  return best
}

export function scoreAccount(account: Account, task: TaskType, snap: UsageSnapshot | undefined, now: number): Recommendation {
  if (account.status === 'needs-login' || account.status === 'expired') return rec(account, 0, 'Needs login before it can be used.')
  const t = tier(account)
  const fit =
    task === 'long-refactor' ? t * 8 : task === 'coding' ? t * 5 : task === 'research' || task === 'review' ? t * 2 : task === 'quick-question' ? -t * 4 : 0
  const kws = TASK_KEYWORDS[task]
  const tagBonus = account.tags.some((tag) => kws.includes(tag.trim().toLowerCase())) ? 10 : 0
  const pinBonus = account.pinned === true ? 2 : 0

  const tight = snap && tightest(snap)
  if (!snap || !tight) return rec(account, 40 + fit + tagBonus + pinBonus, 'No usage data yet.')
  const left = leftOf(tight)
  const r = parseIso(tight.resetsAt)
  const resets = r !== null && r > now ? r : null

  if (left < 1) {
    const when = resets !== null ? `, resets ${humanUntil(resets, now)}` : ''
    return rec(account, 0, `${tight.label} limit reached${when}.`)
  }

  // Save big accounts: prefer ones with little headroom left, as long as there is enough.
  const base = task === 'quick-question' ? (left >= 10 ? 50 + (100 - left) * 0.25 : left * 4) : left * 0.65
  const lose = useItOrLoseIt(snap, now)
  const resetBonus = lose ? (12 * lose[1]) / 100 : 0
  const stalePenalty = snap.stale === true ? 5 : 0
  const score = base + fit + tagBonus + pinBonus + resetBonus - stalePenalty

  let reason = `${left.toFixed(0)}% of ${tight.label} left`
  if (resets !== null) reason += `, resets ${humanUntil(resets, now)}`
  reason += '.'
  if (lose) {
    reason += lose[0].id === tight.id ? ' Use it before it resets.' : ` ${lose[1].toFixed(0)}% of ${lose[0].label} resets soon, use it before then.`
  }
  if ((task === 'long-refactor' || task === 'coding') && t >= 3) reason += ' Top-tier plan suits long sessions.'
  else if (task === 'quick-question' && left < 50) reason += ' Good for a quick task, saves your bigger accounts.'
  if (tagBonus > 0) reason += ' Tagged for this kind of task.'
  if (stalePenalty > 0) reason += ' (Last known values.)'
  // 0 means "can't use it" (the Use now card skips those), so a usable account always scores at least 1.
  return rec(account, Math.max(1, score), reason)
}

/**
 * recommend::recommend: best first, ties by label. 0 = can't be used now (signed out or at a limit).
 * Accounts of a provider in `cliMissing` can still be used in its desktop app, so they score 1 (not 0)
 * and sort after every other usable account, best quota first.
 */
export function recommend(
  task: TaskType,
  providerId: ProviderId | undefined,
  accounts: Account[],
  usage: Record<string, UsageSnapshot>,
  cliMissing: ProviderId[],
  now: number = Date.now()
): Recommendation[] {
  return accounts
    .filter((a) => !providerId || a.providerId === providerId)
    .map((a) => {
      const r = scoreAccount(a, task, usage[a.id], now)
      const missing = cliMissing.includes(a.providerId)
      const raw = r.score
      if (missing && raw > 0) {
        // Keep what is left first; the task-fit advice doesn't apply without the CLI.
        const left = r.reason.split('. ')[0]!.replace(/\.?$/, '.')
        r.score = 1
        r.reason = `${left} The ${PROVIDER_META[a.providerId].short} CLI isn't installed on this computer.`
      }
      return { r, missing, raw, label: a.label }
    })
    .sort(
      (x, y) =>
        y.r.score - x.r.score || Number(x.missing) - Number(y.missing) || y.raw - x.raw || (x.label < y.label ? -1 : x.label > y.label ? 1 : 0)
    )
    .map((x) => x.r)
}
