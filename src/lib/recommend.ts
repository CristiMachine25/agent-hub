import type { Account, ProviderId, Recommendation, TaskType, UsageSnapshot } from '@shared/types'
import { bindingWindow, leftOf, longWindow } from './quota'
import { formatCountdown, windowName } from './format'
import { PROVIDER_META } from './providers'

/** How well each provider suits a kind of task (0..1). Used only to break ties between similar quotas. */
const FIT: Record<TaskType, Record<ProviderId, number>> = {
  coding: { claude: 1, codex: 0.95, gemini: 0.7, copilot: 0.75 },
  'long-refactor': { claude: 1, codex: 0.9, gemini: 0.6, copilot: 0.5 },
  'quick-question': { claude: 0.8, codex: 0.8, gemini: 0.9, copilot: 0.9 },
  research: { claude: 0.85, codex: 0.75, gemini: 1, copilot: 0.5 },
  review: { claude: 0.9, codex: 1, gemini: 0.7, copilot: 0.85 },
  any: { claude: 1, codex: 1, gemini: 1, copilot: 1 }
}

/**
 * Score every account for a task. Pure, so the mock backend and tests share it; the Rust side has its own.
 * Unusable accounts (signed out, at a limit) score 0 and explain why.
 */
export function scoreAccounts(
  accounts: Account[],
  usage: Record<string, UsageSnapshot>,
  task: TaskType,
  now: number = Date.now(),
  providerId?: ProviderId,
  /** CLIs known to be missing are pushed to the bottom: launching them would fail. */
  installed?: Partial<Record<ProviderId, boolean>>
): Recommendation[] {
  const recs: Recommendation[] = []
  for (const a of accounts) {
    if (providerId && a.providerId !== providerId) continue
    if (a.status === 'needs-login' || a.status === 'expired') {
      recs.push({ accountId: a.id, score: 0, reason: 'Signed out. Sign in to use this account.' })
      continue
    }
    const snap = usage[a.id]
    const bind = bindingWindow(snap)
    const long = longWindow(snap)
    if (!snap || !bind || !long) {
      recs.push({ accountId: a.id, score: 20, reason: 'No usage data yet, so limits are unknown.' })
      continue
    }
    const bindLeft = leftOf(bind)
    const longLeft = leftOf(long)
    if (bindLeft <= 0.5) {
      const c = formatCountdown(bind.resetsAt, now)
      recs.push({
        accountId: a.id,
        score: 0,
        reason: `${bind.label} limit reached${c && c !== 'now' ? `, back in ${c}` : ''}.`
      })
      continue
    }

    // Long tasks need the long window; short tasks only need the session window to have room.
    const needLong = task === 'long-refactor' ? 0.75 : task === 'quick-question' ? 0.2 : 0.5
    let score = bindLeft * (1 - needLong) + longLeft * needLong
    if (task === 'quick-question' && bindLeft >= 10) score = Math.max(score, 55)
    score *= 0.75 + 0.25 * FIT[task][a.providerId]

    let reason = `${Math.round(longLeft)}% of ${windowName(long.label)} left`
    if (bind.id !== long.id) reason += `, ${Math.round(bindLeft)}% of ${windowName(bind.label)}`

    // Quota that is about to reset unused is free value: push it up.
    const hoursToReset = long.resetsAt ? (Date.parse(long.resetsAt) - now) / 3_600_000 : Infinity
    if (hoursToReset < 48 && longLeft >= 40 && (long.windowMinutes ?? 0) >= 3 * 24 * 60) {
      score += 18
      reason += `. Resets in ${formatCountdown(long.resetsAt, now)}, so use it before it's gone`
    } else if (bindLeft < 20) {
      reason += '. Close to the session limit'
    }
    if (a.tags.includes(task)) score += 6
    if (installed && installed[a.providerId] === false) {
      score = Math.min(score, 5)
      reason = `${PROVIDER_META[a.providerId].product} isn't installed on this computer. ` + reason
    }
    if (snap.stale) {
      score -= 8
      reason += '. Numbers may be out of date'
    }
    recs.push({ accountId: a.id, score: Math.max(1, Math.min(100, Math.round(score))), reason: reason + '.' })
  }
  return recs.sort((x, y) => y.score - x.score)
}
