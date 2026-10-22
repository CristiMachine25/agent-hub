import { useMemo } from 'react'
import { Area, AreaChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import type { Account, UsageHistoryPoint, UsageSnapshot, UsageWindow } from '@shared/types'
import { USE_IT_MIN_WINDOW_MINUTES, longWindow } from '@/lib/quota'
import { formatDay, windowName } from '@/lib/format'
import { tipTime, useChartColors } from './theme'

/**
 * Weekly/monthly used % per account over time, as small multiples: one strip per account on a shared time axis.
 * Overlaid sawtooth lines for 6+ accounts are unreadable; aligned strips make "who resets full vs empty" obvious.
 */
export function UsageOverTime({
  accounts,
  usage,
  history,
  days
}: {
  accounts: Account[]
  usage: Record<string, UsageSnapshot>
  history: UsageHistoryPoint[]
  days: number
}) {
  const c = useChartColors()
  const end = useMemo(() => Date.now(), [history, days]) // eslint-disable-line react-hooks/exhaustive-deps
  const start = end - days * 86_400_000

  const rows = useMemo(() => {
    const step = (days > 10 ? 2 : 1) * 3_600_000
    return accounts
      .map((a) => ({ a, w: longWindow(usage[a.id]) }))
      .filter((x): x is { a: Account; w: UsageWindow } => !!x.w && (x.w.windowMinutes ?? 0) >= USE_IT_MIN_WINDOW_MINUTES)
      .map(({ a, w }) => {
        const seen = new Set<number>()
        const data: { t: number; v: number }[] = []
        for (const p of history) {
          if (p.accountId !== a.id || !(w.id in p.windows)) continue
          const t = Date.parse(p.t)
          if (t < start) continue
          const b = Math.floor(t / step)
          if (seen.has(b)) continue
          seen.add(b)
          data.push({ t, v: p.windows[w.id]! }) // present: checked with `in` above
        }
        data.sort((x, y) => x.t - y.t)
        return { a, w, data }
      })
      .filter((r) => r.data.length > 1)
  }, [accounts, usage, history, days, start])

  if (!rows.length) return <p className="py-10 text-center text-muted">No history yet. It fills in as Agent Hub checks usage.</p>

  const ticks = [0, 1, 2, 3].map((i) => start + ((end - start) * i) / 3)
  return (
    <div role="group" aria-label="Weekly and monthly limits used over time, one strip per account">
      <ul className="space-y-1">
        {rows.map(({ a, w, data }) => {
          const last = data[data.length - 1]!.v // rows keep two or more points
          return (
            <li key={a.id} className="grid grid-cols-[minmax(0,8.5rem)_1fr_3.25rem] items-center gap-3">
              <span className="min-w-0">
                <span className="block truncate">{a.label}</span>
                <span className="block truncate text-[11px] leading-4 text-muted">{windowName(w.label)}</span>
              </span>
              <div className="h-9 border-b border-rivet">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={data} margin={{ top: 2, right: 0, bottom: 0, left: 0 }} accessibilityLayer={false}>
                    <XAxis dataKey="t" type="number" domain={[start, end]} hide />
                    <YAxis domain={[0, 100]} hide />
                    <Tooltip
                      cursor={{ stroke: c.axis, strokeWidth: 1 }}
                      allowEscapeViewBox={{ y: true }}
                      content={({ active, payload }) =>
                        active && payload?.length ? (
                          <div className="rounded border border-rivet bg-deck px-2.5 py-1.5 text-xs shadow-lg">
                            <span className="text-muted">{tipTime.format(Number(payload[0]!.payload.t))}: </span>
                            <span className="tabular-nums">{Math.round(Number(payload[0]!.value))}% used</span>
                          </div>
                        ) : null
                      }
                    />
                    <Area
                      type="linear"
                      dataKey="v"
                      stroke={a.color}
                      strokeWidth={1.5}
                      fill={a.color}
                      fillOpacity={c.theme === 'dark' ? 0.22 : 0.16}
                      isAnimationActive={false}
                      activeDot={{ r: 3.5, strokeWidth: 2, stroke: c.surface }}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
              <span className="text-right text-xs tabular-nums text-muted" data-tip="Used now">
                {Math.round(last)}%<span className="sr-only"> used now</span>
              </span>
            </li>
          )
        })}
      </ul>
      <div className="mt-1 grid grid-cols-[minmax(0,8.5rem)_1fr_3.25rem] gap-3 text-[11px] text-muted">
        <span>Strip height = % used</span>
        <span className="flex justify-between tabular-nums">
          {ticks.map((t) => (
            <span key={t}>{formatDay(t)}</span>
          ))}
        </span>
        <span className="text-right">Now</span>
      </div>
    </div>
  )
}
