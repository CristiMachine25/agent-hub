import { useEffect, useMemo, useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, Clock, Hourglass, LogIn, Play, Plus, RefreshCw, RotateCcw, Search, WifiOff } from 'lucide-react'
import type { Account, DailyActivity, ProviderId, Recommendation, UsageHistoryPoint, UsageSnapshot } from '@shared/types'
import { api } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META, PROVIDER_ORDER, TASKS, launchVerb } from '@/lib/providers'
import { bindingWindow, computeAlerts, leftOf, levelFor, longWindow, unusedAtResets, wastedValue, type Alert } from '@/lib/quota'
import { creditsText, formatAgo, formatCountdown, formatMoney, formatPct, plural, resetText, windowName } from '@/lib/format'
import { Meter, WindowGauge, LEVEL_TEXT } from '@/components/Meter'
import { AccountDot, EmptyState, ProviderMark, Segmented, SectionTitle } from '@/components/ui'
import { UsageOverTime } from '@/charts/UsageOverTime'
import { MostUsed, type ActivityMetric } from '@/charts/MostUsed'

export function Overview() {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const settings = useStore((s) => s.settings)
  const now = useStore((s) => s.now)
  const setAddOpen = useStore((s) => s.setAddOpen)

  const [history, setHistory] = useState<UsageHistoryPoint[]>([])
  const [activity, setActivity] = useState<DailyActivity[]>([])
  const [days, setDays] = useState<'7' | '30'>('30')
  const [metric, setMetric] = useState<ActivityMetric>('tokens')

  const accountKey = accounts.map((a) => a.id).join(',')
  useEffect(() => {
    let live = true
    Promise.all([api.usage.history(null, 30), api.usage.activity(null, 30)])
      .then(([h, a]) => {
        if (live) {
          setHistory(h)
          setActivity(a)
        }
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [accountKey])

  const alerts = useMemo(() => (settings ? computeAlerts(accounts, usage, settings, now) : []), [accounts, usage, settings, now])

  if (accounts.length === 0) {
    return (
      <Page title="Overview">
        <EmptyState
          title="No accounts yet"
          body="AI Hopper can find the Claude, Codex, Gemini and Copilot profiles already on this computer, or create a fresh profile for another login."
          action={
            <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
              <Search size={15} /> Find accounts on this computer
            </button>
          }
        />
      </Page>
    )
  }

  const activitySince = Date.now() - Number(days) * 86_400_000
  const act = activity.filter((d) => Date.parse(d.date + 'T23:59:59') >= activitySince)

  return (
    <Page title="Overview">
      <UseNow />
      {alerts.length > 0 && <Alerts alerts={alerts} />}
      <Board />
      <section className="grid grid-cols-1 gap-x-8 gap-y-6 border-t border-rivet pt-6 min-[1180px]:grid-cols-[1.5fr_1fr]">
        <div className="min-w-0">
          <SectionTitle
            aside={
              <Segmented
                size="sm"
                label="Time range"
                value={days}
                onChange={setDays}
                options={[
                  { id: '7', label: '7 days' },
                  { id: '30', label: '30 days' }
                ]}
              />
            }
          >
            Weekly and monthly limits used
          </SectionTitle>
          <UsageOverTime accounts={accounts} usage={usage} history={history} days={Number(days)} />
        </div>
        <div className="min-w-0">
          <SectionTitle
            aside={
              <Segmented
                size="sm"
                label="Measure"
                value={metric}
                onChange={setMetric}
                options={[
                  { id: 'tokens', label: 'Tokens' },
                  { id: 'messages', label: 'Messages' }
                ]}
              />
            }
          >
            Most used, last {days} days
          </SectionTitle>
          <MostUsed accounts={accounts} activity={act} metric={metric} />
          <Unused accounts={accounts} usage={usage} history={history} />
        </div>
      </section>
    </Page>
  )
}

export function Page({ title, actions, children }: { title: string; actions?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="mx-auto flex w-full max-w-[1280px] flex-col gap-6 px-7 pb-10 pt-5 max-[999px]:px-5">
      <header className="flex min-h-9 items-center justify-between gap-4">
        <h1 className="text-lg font-bold tracking-tight">{title}</h1>
        <div className="flex items-center gap-2">{actions}</div>
      </header>
      {children}
    </div>
  )
}

/** The loud part: the account to use right now, with its launch button. */
function UseNow() {
  const task = useStore((s) => s.task)
  const setTask = useStore((s) => s.setTask)
  const recs = useStore((s) => s.recs)
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const launching = useStore((s) => s.launching)
  const { launch, go } = useStore.getState()

  // Signed-out accounts can't be launched usefully, so they never appear as a pick here.
  const usable = recs.filter((r) => {
    const a = accounts.find((x) => x.id === r.accountId)
    return r.score > 0 && !!a && a.status !== 'needs-login' && a.status !== 'expired'
  })
  const top = usable[0]
  const acc = top && accounts.find((a) => a.id === top.accountId)
  const snap = acc && usage[acc.id]
  const bind = bindingWindow(snap)
  const left = bind ? leftOf(bind) : undefined
  const others = usable.slice(1, 4)

  return (
    <section aria-labelledby="usenow-title" className="rounded-lg border border-rivet bg-deck">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-rivet px-5 py-3">
        <h2 id="usenow-title" className="text-md font-semibold">
          Best account right now
        </h2>
        <Segmented
          label="Kind of task"
          value={task}
          onChange={setTask}
          options={TASKS.map((t) => ({ id: t.id, label: t.label, title: t.hint }))}
        />
      </div>

      {!acc ? (
        <NothingUsable accounts={accounts} usage={usage} now={now} />
      ) : (
        <div className="grid grid-cols-1 min-[1100px]:grid-cols-[1fr_320px]">
          <div className="min-w-0 px-5 pb-5 pt-4">
            <div className="flex items-center gap-2.5">
              <AccountDot account={acc} size={12} />
              <button className="truncate text-[22px] font-bold leading-7 tracking-tight hover:underline" onClick={() => go('accounts', acc.id)}>
                {acc.label}
              </button>
              <span className="flex items-center gap-1.5 text-muted">
                <ProviderMark id={acc.providerId} size={16} />
                {PROVIDER_META[acc.providerId].product}
                {acc.plan ? `, ${acc.plan}` : ''}
              </span>
            </div>

            {bind && left !== undefined && (
              <div className="mt-4 flex items-end gap-5">
                <div className="shrink-0">
                  <div className={clsx('text-hero font-bold tabular-nums tracking-tight', LEVEL_TEXT[levelFor(left)])}>
                    {formatPct(left)}
                  </div>
                  <div className="mt-1 text-xs text-muted">left of {windowName(bind.label)}</div>
                </div>
                <div className="min-w-0 flex-1 pb-1">
                  <Meter left={left} cells={20} size="lg" label={`${acc.label}, ${bind.label}`} />
                  <div className="mt-1.5 text-xs text-muted">{resetText(bind.resetsAt, now) ?? 'No reset scheduled'}</div>
                </div>
              </div>
            )}
            {snap && snap.windows.length > 1 && (
              <div className="mt-4 grid grid-cols-2 gap-x-6 gap-y-3 min-[1240px]:grid-cols-3">
                {snap.windows
                  .filter((w) => w.id !== bind?.id)
                  .map((w) => (
                    <WindowGauge key={w.id} w={w} now={now} compact />
                  ))}
              </div>
            )}
            <p className="mt-4 text-muted">{top.reason}</p>
            <div className="mt-4 flex flex-wrap gap-2">
              <button
                className="btn btn-primary btn-lg"
                onClick={() => launch(acc.id, 'cli')}
                disabled={launching === acc.id + 'cli'}
              >
                <Play size={16} fill="currentColor" /> {launchVerb(acc.providerId)}
              </button>
              <button className="btn btn-lg" onClick={() => launch(acc.id, 'cli', { resume: true })}>
                <RotateCcw size={15} /> Resume last session
              </button>
            </div>
          </div>
          <div className="border-t border-rivet px-5 py-4 min-[1100px]:border-l min-[1100px]:border-t-0">
            <h3 className="mb-2 text-xs font-medium text-muted">If not that one</h3>
            {others.length === 0 && <p className="text-muted">No other account has room right now.</p>}
            <ul className="space-y-1">
              {others.map((r) => (
                <AltRow key={r.accountId} rec={r} account={accounts.find((a) => a.id === r.accountId)!} snap={usage[r.accountId]} />
              ))}
            </ul>
          </div>
        </div>
      )}
    </section>
  )
}

function AltRow({ rec, account, snap }: { rec: Recommendation; account: Account; snap?: UsageSnapshot }) {
  const launch = useStore((s) => s.launch)
  const bind = bindingWindow(snap)
  const left = bind ? leftOf(bind) : 0
  return (
    <li className="group flex items-center gap-3 rounded px-2 py-2 -mx-2 hover:bg-raised">
      <AccountDot account={account} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <span className="truncate font-medium">{account.label}</span>
          <span className="shrink-0 text-xs tabular-nums text-muted">{formatPct(left)} left</span>
        </div>
        <Meter left={left} cells={20} size="xs" label={`${account.label} quota left`} className="mt-1" />
        <p className="mt-1 line-clamp-2 text-xs text-muted" title={rec.reason}>
          {rec.reason}
        </p>
      </div>
      <button className="btn btn-sm" onClick={() => launch(account.id, 'cli')} aria-label={`${launchVerb(account.providerId)} as ${account.label}`}>
        <Play size={12} fill="currentColor" /> Launch
      </button>
    </li>
  )
}

function NothingUsable({ accounts, usage, now }: { accounts: Account[]; usage: Record<string, UsageSnapshot>; now: number }) {
  // Find the soonest reset among accounts that are at a limit.
  let soonest: { a: Account; at: string } | undefined
  for (const a of accounts) {
    const b = bindingWindow(usage[a.id])
    if (b?.resetsAt && (!soonest || Date.parse(b.resetsAt) < Date.parse(soonest.at))) soonest = { a, at: b.resetsAt }
  }
  return (
    <div className="flex items-center gap-3 px-5 py-6">
      <Hourglass className="text-low" size={20} />
      <div>
        <p className="font-semibold">Every account is at a limit or signed out</p>
        <p className="text-muted">
          {soonest ? `${soonest.a.label} is back first, in ${formatCountdown(soonest.at, now)}.` : 'Sign in to an account to see its limits.'}
        </p>
      </div>
    </div>
  )
}

function alertCopy(al: Alert, a: Account, snap: UsageSnapshot | undefined, now: number): string {
  const w = snap?.windows.find((x) => x.id === al.windowId)
  const name = w ? windowName(w.label) : 'limit'
  const c = formatCountdown(al.resetsAt, now)
  switch (al.kind) {
    case 'use-it-or-lose-it':
      return `${a.label} has ${formatPct(al.left ?? 0)} of its ${name} left, and it resets in ${c}. Use it or lose it.`
    case 'limit-reached':
      return `${a.label} hit its ${name}${/limit|requests/.test(name) ? '' : ' limit'}${c && c !== 'now' ? `. Back in ${c}` : ''}.`
    case 'nearly-exhausted':
      return `${a.label} has only ${formatPct(al.left ?? 0)} of its ${name} left. ${resetText(al.resetsAt, now) ?? ''}.`
    case 'needs-login':
      return `${a.label} is signed out, so its limits aren't tracked.`
    case 'stale':
      return `${a.label}: usage last checked ${formatAgo(snap?.fetchedAt, now)}. ${snap?.error ?? ''}`
  }
}

const ALERT_ICON = {
  'use-it-or-lose-it': Clock,
  'limit-reached': AlertTriangle,
  'nearly-exhausted': AlertTriangle,
  'needs-login': LogIn,
  stale: WifiOff
}

function Alerts({ alerts }: { alerts: Alert[] }) {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const { launch, refresh } = useStore.getState()
  const [all, setAll] = useState(false)
  const shown = all ? alerts : alerts.slice(0, 3)
  return (
    <section aria-label="Alerts" className="-mt-1">
      <ul className="divide-y divide-rivet rounded-lg border border-rivet">
        {shown.map((al, i) => {
          const a = accounts.find((x) => x.id === al.accountId)
          if (!a) return null
          const Icon = ALERT_ICON[al.kind]
          const tone =
            al.kind === 'use-it-or-lose-it' ? 'text-hop' : al.kind === 'limit-reached' ? 'text-empty' : al.kind === 'nearly-exhausted' ? 'text-low' : 'text-muted'
          return (
            <li key={i} className="flex items-center gap-3 px-4 py-2">
              <Icon size={15} className={clsx('shrink-0', tone)} aria-hidden />
              <p className="min-w-0 flex-1">{alertCopy(al, a, usage[a.id], now)}</p>
              {al.kind === 'use-it-or-lose-it' && (
                <button className="btn btn-sm" onClick={() => launch(a.id, 'cli')}>
                  <Play size={12} fill="currentColor" /> {launchVerb(a.providerId)}
                </button>
              )}
              {al.kind === 'needs-login' && (
                <button className="btn btn-sm" onClick={() => launch(a.id, 'login')}>
                  Sign in
                </button>
              )}
              {al.kind === 'stale' && (
                <button className="btn btn-sm" onClick={() => refresh(a.id)}>
                  <RefreshCw size={12} /> Check again
                </button>
              )}
            </li>
          )
        })}
      </ul>
      {alerts.length > 3 && (
        <button className="mt-1.5 text-xs text-muted hover:text-ink" onClick={() => setAll(!all)}>
          {all ? 'Show fewer' : `Show ${plural(alerts.length - 3, 'more alert')}`}
        </button>
      )}
    </section>
  )
}

/** Every account at a glance, grouped by tool so the same windows line up in columns. */
function Board() {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const setAddOpen = useStore((s) => s.setAddOpen)
  const groups = PROVIDER_ORDER.map((p) => ({ p, list: accounts.filter((a) => a.providerId === p) })).filter((g) => g.list.length)

  return (
    <section aria-labelledby="board-title">
      <SectionTitle
        aside={
          <button className="btn btn-ghost btn-sm" onClick={() => setAddOpen(true)}>
            <Plus size={14} /> Add account
          </button>
        }
      >
        <span id="board-title">All accounts</span>
      </SectionTitle>
      <div className="overflow-hidden rounded-lg border border-rivet">
        {groups.map(({ p, list }) => (
          <div key={p}>
            <div className="flex items-center gap-2 border-b border-rivet bg-deck/60 px-4 py-1.5 text-xs text-muted">
              <ProviderMark id={p} size={14} />
              {PROVIDER_META[p].product}
            </div>
            <ul className="divide-y divide-rivet border-b border-rivet last:border-b-0">
              {list.map((a) => (
                <BoardRow key={a.id} a={a} snap={usage[a.id]} now={now} provider={p} />
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  )
}

function BoardRow({ a, snap, now, provider }: { a: Account; snap?: UsageSnapshot; now: number; provider: ProviderId }) {
  const { launch, go } = useStore.getState()
  const signedOut = a.status === 'needs-login' || a.status === 'expired'
  const bind = bindingWindow(snap)
  const atLimit = bind ? leftOf(bind) <= 0.5 : false
  return (
    <li className="grid grid-cols-[minmax(150px,1fr)_minmax(0,2.4fr)_auto] items-center gap-5 px-4 py-3 hover:bg-deck">
      <button className="flex min-w-0 items-center gap-2.5 text-left" onClick={() => go('accounts', a.id)} title="Show details">
        <AccountDot account={a} />
        <span className="min-w-0">
          <span className="block truncate font-semibold">{a.label}</span>
          <span className="block truncate text-xs text-muted">
            {signedOut ? 'Signed out' : snap?.stale ? `Checked ${formatAgo(snap.fetchedAt, now)}` : a.plan && !a.label.includes(a.plan) ? a.plan : PROVIDER_META[provider].short}
          </span>
        </span>
      </button>
      <div className="min-w-0">
        {signedOut ? (
          <p className="text-muted">{a.statusMessage ?? 'Sign in to see this account’s limits.'}</p>
        ) : snap && snap.windows.length ? (
          <div className="grid grid-cols-3 gap-5">
            {snap.windows.slice(0, 3).map((w) => (
              <WindowGauge key={w.id} w={w} now={now} compact stale={snap.stale} />
            ))}
            {snap.credits && snap.windows.length < 3 && <div className="self-center text-xs text-muted">{creditsText(snap.credits)}</div>}
          </div>
        ) : (
          <p className="text-muted">
            {snap?.error ?? 'No usage data yet.'}
            {snap?.credits && ` ${creditsText(snap.credits)}.`}
          </p>
        )}
      </div>
      <div className="flex justify-end">
        {signedOut ? (
          <button className="btn btn-sm w-[92px]" onClick={() => launch(a.id, 'login')}>
            <LogIn size={13} /> Sign in
          </button>
        ) : (
          <button
            className="btn btn-sm w-[92px]"
            onClick={() => launch(a.id, 'cli')}
            title={atLimit ? 'At its limit: the CLI opens but will refuse new messages until the reset' : undefined}
            aria-label={`${launchVerb(a.providerId)} as ${a.label}`}
          >
            <Play size={12} fill="currentColor" /> Launch
          </button>
        )}
      </div>
    </li>
  )
}

/** Bonus: paid quota that reset without being used, from history. */
function Unused({ accounts, usage, history }: { accounts: Account[]; usage: Record<string, UsageSnapshot>; history: UsageHistoryPoint[] }) {
  const rows = useMemo(() => {
    return accounts
      .map((a) => {
        const w = longWindow(usage[a.id])
        if (!w || !a.monthlyCost) return null
        const unused = unusedAtResets(
          history.filter((p) => p.accountId === a.id),
          w.id
        )
        const perPeriod = a.monthlyCost * ((w.windowMinutes ?? 10080) / 43_200)
        return { a, value: wastedValue(unused, perPeriod), resets: unused.length }
      })
      .filter((r): r is { a: Account; value: number; resets: number } => !!r && r.value >= 1)
      .sort((x, y) => y.value - x.value)
  }, [accounts, usage, history])
  if (!rows.length) return null
  const total = rows.reduce((s, r) => s + r.value, 0)
  return (
    <div className="mt-6 border-t border-rivet pt-4">
      <p>
        <span className="text-md font-semibold tabular-nums">About {formatMoney(Math.round(total))}</span>{' '}
        <span className="text-muted">of paid quota reset unused in the last 30 days.</span>
      </p>
      <p className="mt-1 text-xs text-muted">
        Most from {rows
          .slice(0, 2)
          .map((r) => `${r.a.label} (${formatMoney(Math.round(r.value))})`)
          .join(' and ')}
        . Estimated from your monthly cost and how full each weekly limit was when it reset.
      </p>
    </div>
  )
}
