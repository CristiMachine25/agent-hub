import { useMemo } from 'react'
import type { Account, DailyActivity } from '@shared/types'
import { formatTokens } from '@/lib/format'

export type ActivityMetric = 'tokens' | 'messages'

/** Ranked bars of activity per account. Plain HTML bars: easier to label and read than an SVG chart here. */
export function MostUsed({ accounts, activity, metric }: { accounts: Account[]; activity: DailyActivity[]; metric: ActivityMetric }) {
  const rows = useMemo(() => {
    const sum = new Map<string, number>()
    for (const d of activity) {
      const v = metric === 'tokens' ? d.inputTokens + d.outputTokens : d.messages
      sum.set(d.accountId, (sum.get(d.accountId) ?? 0) + v)
    }
    return accounts
      .map((a) => ({ a, v: sum.get(a.id) ?? 0 }))
      .filter((r) => r.v > 0)
      .sort((x, y) => y.v - x.v)
  }, [accounts, activity, metric])

  if (!rows.length) return <p className="py-10 text-center text-muted">No activity found in local session logs yet.</p>
  const max = rows[0].v
  const total = rows.reduce((s, r) => s + r.v, 0)
  return (
    <ol className="space-y-2.5">
      {rows.map(({ a, v }) => (
        <li key={a.id} className="grid grid-cols-[minmax(0,9.5rem)_1fr_3.5rem] items-center gap-3" data-tip={`${Math.round((v / total) * 100)}% of all ${metric}`}>
          <span className="truncate text-muted">{a.label}</span>
          <span className="block h-3 rounded-r-[3px] bg-[rgb(var(--cell-off))]">
            <span className="block h-full rounded-r-[3px]" style={{ width: `${Math.max(2, (v / max) * 100)}%`, background: a.color }} />
          </span>
          <span className="text-right tabular-nums">
            {metric === 'tokens' ? formatTokens(v) : v.toLocaleString()}
            <span className="sr-only">, {Math.round((v / total) * 100)}% of all {metric}</span>
          </span>
        </li>
      ))}
    </ol>
  )
}
