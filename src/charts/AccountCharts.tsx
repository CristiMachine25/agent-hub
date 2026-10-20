import { useMemo } from 'react'
import { Bar, BarChart, CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import type { DailyActivity, UsageHistoryPoint, UsageWindow } from '@shared/types'
import { formatDay, formatTokens } from '@/lib/format'
import { axisTick, useChartColors } from './theme'

/** Fixed categorical order (validated all-pairs for the first three) for window lines within one account. */
const WINDOW_COLORS = { dark: ['#3987E5', '#D95926', '#199E70', '#C98500'], light: ['#2A78D6', '#EB6834', '#1BAF7A', '#EDA100'] }
const fmtTip = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', hour: 'numeric' })

export function WindowHistory({ windows, history, height = 200 }: { windows: UsageWindow[]; history: UsageHistoryPoint[]; height?: number }) {
  const c = useChartColors()
  const colors = WINDOW_COLORS[c.theme]
  const data = useMemo(
    () => history.map((p) => ({ t: Date.parse(p.t), ...p.windows })).sort((a, b) => a.t - b.t),
    [history]
  )
  const ticks = useMemo(() => {
    if (!data.length) return []
    const first = data[0].t
    const last = data[data.length - 1].t
    const out: number[] = []
    const d = new Date(first)
    d.setHours(0, 0, 0, 0)
    d.setDate(d.getDate() + 1)
    for (let t = d.getTime(); t <= last; t += 86_400_000) out.push(t)
    const every = Math.ceil(out.length / 8)
    return out.filter((_, i) => i % every === 0)
  }, [data])
  if (!data.length || !windows.length) return <p className="py-8 text-center text-muted">No history for this account yet.</p>
  return (
    <div>
      <div style={{ height }}>
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={data} margin={{ top: 6, right: 8, bottom: 0, left: -18 }}>
            <CartesianGrid stroke={c.grid} vertical={false} />
            <XAxis dataKey="t" type="number" scale="time" domain={['dataMin', 'dataMax']} ticks={ticks} tickFormatter={(t: number) => formatDay(t)} tick={axisTick(c.axis)} stroke={c.grid} tickLine={false} />
            <YAxis domain={[0, 100]} ticks={[0, 50, 100]} tickFormatter={(v: number) => `${v}%`} tick={axisTick(c.axis)} axisLine={false} tickLine={false} />
            <Tooltip
              cursor={{ stroke: c.axis, strokeWidth: 1 }}
              content={({ active, payload, label }) =>
                active && payload?.length ? (
                  <div className="rounded border border-rivet bg-deck px-3 py-2 text-xs shadow-lg">
                    <p className="mb-1 font-semibold">{fmtTip.format(Number(label))}</p>
                    {payload.map((r, i) => (
                      <p key={i} className="flex items-center gap-2">
                        <span className="h-2 w-2 rounded-full" style={{ background: r.color }} />
                        <span className="flex-1 text-muted">{r.name}</span>
                        <span className="tabular-nums">{Math.round(Number(r.value))}% used</span>
                      </p>
                    ))}
                  </div>
                ) : null
              }
            />
            {windows.map((w, i) => (
              <Line key={w.id} dataKey={w.id} name={w.label} stroke={colors[i % colors.length]} strokeWidth={1.75} dot={false} isAnimationActive={false} activeDot={{ r: 4, strokeWidth: 2, stroke: c.surface }} />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>
      {windows.length > 1 && (
        <ul className="mt-2 flex flex-wrap gap-x-4 text-xs text-muted" aria-label="Legend">
          {windows.map((w, i) => (
            <li key={w.id} className="flex items-center gap-1.5">
              <span className="h-[3px] w-3 rounded-full" style={{ background: colors[i % colors.length] }} />
              {w.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

export function DailyTokens({ activity, color, span, height = 160 }: { activity: DailyActivity[]; color: string; span: number; height?: number }) {
  const c = useChartColors()
  const data = useMemo(() => {
    // One bar per calendar day, including quiet days, so gaps read as gaps.
    const by = new Map(activity.map((d) => [d.date, d]))
    const days = Math.max(1, Math.round(span))
    const out: { date: string; tokens: number; messages: number }[] = []
    for (let i = days - 1; i >= 0; i--) {
      const d = new Date(Date.now() - i * 86_400_000)
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
      const a = by.get(key)
      out.push({ date: key, tokens: a ? a.inputTokens + a.outputTokens : 0, messages: a?.messages ?? 0 })
    }
    return activity.length ? out : []
  }, [activity, span])
  if (!data.length) return <p className="py-8 text-center text-muted">No sessions in the local logs for this period.</p>
  return (
    <div style={{ height }}>
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 6, right: 8, bottom: 0, left: -12 }} barCategoryGap={2}>
          <CartesianGrid stroke={c.grid} vertical={false} />
          <XAxis dataKey="date" tickFormatter={(d: string) => formatDay(d)} tick={axisTick(c.axis)} stroke={c.grid} tickLine={false} minTickGap={30} />
          <YAxis tickFormatter={(v: number) => formatTokens(v)} tick={axisTick(c.axis)} axisLine={false} tickLine={false} width={50} />
          <Tooltip
            cursor={{ fill: c.grid, opacity: 0.4 }}
            content={({ active, payload }) =>
              active && payload?.length ? (
                <div className="rounded border border-rivet bg-deck px-3 py-2 text-xs shadow-lg">
                  <p className="mb-1 font-semibold">{formatDay(String(payload[0].payload.date))}</p>
                  <p className="tabular-nums">{formatTokens(Number(payload[0].payload.tokens))} tokens</p>
                  <p className="tabular-nums text-muted">{Number(payload[0].payload.messages).toLocaleString()} messages</p>
                </div>
              ) : null
            }
          />
          <Bar dataKey="tokens" fill={color} radius={[3, 3, 0, 0]} isAnimationActive={false} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}
