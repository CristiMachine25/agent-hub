import { Fragment, useEffect, useId, useState } from 'react'
import clsx from 'clsx'
import { AppWindow, Ban, Info, FolderOpen, FolderSearch, LogIn, Pencil, Play, Plus, RefreshCw, RotateCcw, Search, SquareTerminal, Trash2 } from 'lucide-react'
import type { Account, DailyActivity, UsageWindow, SharingOptions, TaskType, UsageHistoryPoint } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { useGrid } from '@/terminals/store'
import { ACCOUNT_COLORS, ACCOUNT_COLOR_NAMES, PROVIDER_META, PROVIDER_ORDER, SHARING_ITEMS, TASKS, launchVerb } from '@/lib/providers'
import { isBlocked, isSignedOut, loginLabel } from '@/lib/grid'
import { bindingWindow, leftOf } from '@/lib/quota'
import { PLANS, findPlan, planLabel } from '@/lib/plans'
import { useDrafts } from '@/lib/drafts'
import { canDeleteHome, profilesDir } from '@/lib/profiles'
import { creditsText, formatAgo, formatPct, formatTokens, plural } from '@/lib/format'
import { Meter, WindowGauge } from '@/components/Meter'
import { AccountDot, Dialog, EmptyState, ProviderMark, Segmented, SectionTitle, Switch } from '@/components/ui'
import { CopyButton } from '@/components/Toasts'
import { DailyTokens, WindowHistory } from '@/charts/AccountCharts'
import { Page, Toolbar } from '@/components/Page'

export function Accounts() {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const selectedId = useStore((s) => s.selectedId)
  const go = useStore((s) => s.go)
  const setAddOpen = useStore((s) => s.setAddOpen)

  const selected = accounts.find((a) => a.id === selectedId) ?? accounts[0]

  if (!accounts.length)
    return (
      <Page title="Accounts">
        <EmptyState
          title="No accounts yet"
          body="Start by letting Agent Hub look for the CLI profiles already on this computer. Nothing is changed until you import."
          action={
            <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
              <Search size={15} /> Find accounts on this computer
            </button>
          }
        />
        <div className="mt-4 max-w-sm">
          <Drafts />
        </div>
      </Page>
    )

  return (
    <div className="flex h-full min-h-0">
      <aside aria-label="Accounts" className="flex w-[272px] shrink-0 flex-col border-r border-rivet bg-well max-[999px]:w-[230px]">
        <div data-tauri-drag-region="deep" className="flex h-14 shrink-0 items-center justify-between border-b border-rivet pl-4 pr-3">
          <h1 className="font-serif text-lg font-semibold">Accounts</h1>
          <button className="btn btn-sm" onClick={() => setAddOpen(true)}>
            <Plus size={14} /> Add
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-4 pt-1">
          <Drafts />
          {PROVIDER_ORDER.map((p) => {
            const list = accounts.filter((a) => a.providerId === p)
            if (!list.length) return null
            return (
              <div key={p} className="mb-3">
                <div className="flex items-center gap-2 px-2 pb-1 pt-2 text-xs text-muted">
                  <ProviderMark id={p} size={14} /> {PROVIDER_META[p].product}
                </div>
                <ul>
                  {list.map((a) => {
                    const b = bindingWindow(usage[a.id])
                    const left = b ? leftOf(b) : null
                    const on = a.id === selected?.id
                    const login = loginLabel(a)
                    return (
                      <li key={a.id}>
                        <button
                          onClick={() => go('accounts', a.id)}
                          aria-current={on ? 'true' : undefined}
                          className={clsx('w-full rounded px-2 py-2 text-left', on ? 'bg-raised shadow-[inset_2px_0_0_rgb(var(--hop))]' : 'hover:bg-raised/60')}
                        >
                          <span className="flex items-center gap-2">
                            <AccountDot account={a} size={8} />
                            <span className={clsx('flex-1 truncate', on && 'font-semibold')}>{a.label}</span>
                            <span className="text-xs tabular-nums text-muted">{login ?? (left === null ? '' : left <= 0.5 ? 'Limit reached' : formatPct(left))}</span>
                          </span>
                          {left !== null && !login && <Meter left={left} cells={20} size="xs" label={`${a.label} quota left`} className="mt-1.5 pl-4" />}
                        </button>
                      </li>
                    )
                  })}
                </ul>
              </div>
            )
          })}
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">{selected && <AccountDetail key={selected.id} account={selected} />}</div>
    </div>
  )
}

function AccountDetail({ account: a }: { account: Account }) {
  const snap = useStore((s) => s.usage[a.id])
  const providers = useStore((s) => s.providers)
  const now = useStore((s) => s.now)
  const launching = useStore((s) => s.launching)
  const { launch, refresh, updateAccount } = useStore.getState()
  const prov = providers.find((p) => p.id === a.providerId)
  const meta = PROVIDER_META[a.providerId]
  const signedOut = isSignedOut(a)
  const blocked = isBlocked(a)
  // A blocked account's CLI refuses to start, so its launch buttons are hidden.
  const canLaunch = !signedOut && !blocked

  const [range, setRange] = useState<'7' | '30'>('7')
  const [history, setHistory] = useState<UsageHistoryPoint[]>([])
  const [activity, setActivity] = useState<DailyActivity[]>([])
  const [editing, setEditing] = useState(false)
  const [removing, setRemoving] = useState(false)

  useEffect(() => {
    let live = true
    Promise.all([api.usage.history(a.id, Number(range)), api.usage.activity(a.id, Number(range))])
      .then(([h, act]) => {
        if (live) {
          setHistory(h)
          setActivity(act)
        }
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [a.id, range])

  const totals = activity.reduce(
    (t, d) => ({ tokens: t.tokens + d.inputTokens + d.outputTokens, messages: t.messages + d.messages, sessions: t.sessions + d.sessions }),
    { tokens: 0, messages: 0, sessions: 0 }
  )
  const models = new Map<string, number>()
  activity.forEach((d) => Object.entries(d.models).forEach(([m, v]) => models.set(m, (models.get(m) ?? 0) + v)))
  const topModel = [...models.entries()].sort((x, y) => y[1] - x[1])[0]?.[0]

  const setSharing = (key: keyof SharingOptions, v: boolean) =>
    updateAccount(a.id, { sharing: { ...a.sharing, [key]: v } }, { quiet: true }).then((ok) => {
      if (ok) useStore.getState().toast({ kind: 'success', title: `${SHARING_ITEMS.find((s) => s.key === key)!.label} ${v ? 'now shared' : 'now private'} for ${a.label}` })
    })

  const openFolder = () =>
    api.system.openPath(a.homeDir).catch((e) => useStore.getState().toast({ kind: 'error', title: "Couldn't open the folder", body: errorText(e) }))

  return (
    <>
      <Toolbar
        level={2}
        title={
          <span className="flex items-center gap-2.5">
            <AccountDot account={a} size={10} />
            <span className="truncate">{a.label}</span>
          </span>
        }
      >
        <button className="btn btn-ghost btn-sm" onClick={() => setEditing(true)}>
          <Pencil size={13} /> Edit
        </button>
      </Toolbar>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="flex flex-col gap-7 px-5 pb-10 pt-4">
          <header>
            <p className="flex flex-wrap items-center gap-x-1.5 text-muted">
              <ProviderMark id={a.providerId} size={14} />
              {meta.product}
              {a.plan ? `, ${a.plan} plan` : ''}
              {a.email ? (signedOut ? `, ${a.email}` : `, signed in as ${a.email}`) : ''}
            </p>

            {signedOut && (
              <div className="mt-4 flex items-center gap-3 rounded border border-rivet bg-deck px-4 py-2.5">
                <LogIn size={16} className="shrink-0 text-hop" />
                <p className="flex-1">{a.statusMessage ?? 'This account is signed out.'}</p>
                <button className="btn btn-primary btn-sm" onClick={() => launch(a.id, 'login')}>
                  Sign in
                </button>
              </div>
            )}
            {blocked && (
              <div className="mt-4 flex items-center gap-3 rounded border border-rivet bg-deck px-4 py-2.5">
                <Ban size={16} className="shrink-0 text-empty" aria-hidden />
                <p className="flex-1">{a.statusMessage ?? `This account can't use ${meta.product}.`}</p>
              </div>
            )}
            {canLaunch && (snap?.stale || snap?.error) && (
              <div className="mt-4 flex items-center gap-3 rounded border border-rivet px-4 py-2.5">
                <Info size={16} className="shrink-0 text-muted" />
                <p className="flex-1 text-muted">{snap.error ?? `These numbers are from ${formatAgo(snap.fetchedAt, now)}.`}</p>
                {snap.stale && (
                  <button className="btn btn-sm" onClick={() => refresh(a.id)}>
                    <RefreshCw size={13} /> Check again
                  </button>
                )}
              </div>
            )}

            <div className="mt-4 flex flex-wrap gap-2">
              {canLaunch && (
                <button className="btn btn-primary" onClick={() => launch(a.id, 'cli')} disabled={launching === a.id + 'cli'}>
                  <Play size={14} fill="currentColor" /> {launchVerb(a.providerId)}
                </button>
              )}
              {canLaunch && (
                <button className="btn" onClick={() => void useGrid.getState().openInGrid(a.id)} data-tip="Run it in a pane of the terminal grid">
                  <SquareTerminal size={14} /> Open in grid
                </button>
              )}
              {canLaunch && (
                <button className="btn" onClick={() => launch(a.id, 'cli', { resume: true })}>
                  <RotateCcw size={14} /> Resume last session
                </button>
              )}
              {canLaunch && (
                <button
                  className="btn"
                  onClick={async () => {
                    const dir = await api.system.pickDirectory()
                    if (dir) void launch(a.id, 'cli', { cwd: dir })
                  }}
                >
                  <FolderSearch size={14} /> Launch in a folder…
                </button>
              )}
              {prov?.desktopInstalled && !signedOut && (
                <button
                  className="btn"
                  onClick={() => launch(a.id, 'desktop')}
                  data-tip={prov.desktopMultiAccount ? undefined : `${prov.desktopName} runs one account at a time; it opens with whichever account it last used`}
                >
                  <AppWindow size={14} /> Open {prov.desktopName ?? 'desktop app'}
                </button>
              )}
              {!signedOut && (
                <button className="btn btn-ghost" onClick={() => launch(a.id, 'login')}>
                  <LogIn size={14} /> Sign in again
                </button>
              )}
            </div>
            {prov && !prov.cliInstalled && (
              <div className="mt-3 flex items-center gap-2 rounded border border-rivet bg-ground py-1 pl-3 pr-1 text-muted">
                <span className="text-xs">{prov.name} isn't installed. Install it with</span>
                <code className="code flex-1 truncate text-ink">{prov.installHint}</code>
                <CopyButton text={prov.installHint} label="Copy install command" />
              </div>
            )}
          </header>

          <section>
            <SectionTitle aside={snap && <span className="text-xs text-muted">Checked {formatAgo(snap.fetchedAt, now)}</span>}>Limits</SectionTitle>
            {snap && snap.windows.length ? (
              <div className="grid grid-cols-1 gap-x-8 gap-y-5 min-[1100px]:grid-cols-2">
                {snap.windows.map((w) => (
                  <WindowGauge key={w.id} w={w} now={now} stale={snap.stale} />
                ))}
                {snap.credits && <p className="self-end text-muted">{creditsText(snap.credits)}.</p>}
              </div>
            ) : (
              <p className="text-muted">
                {signedOut
                  ? 'Sign in to see this account’s limits.'
                  : blocked
                    ? `No limits to show: this account can't use ${meta.product}.`
                    : snap?.credits
                    ? `${creditsText(snap.credits)}.`
                    : snap?.error
                      ? 'No limits to show for this account.'
                      : 'No usage data yet. Agent Hub checks every few minutes.'}
              </p>
            )}
            {canLaunch && (!snap || snap.source === 'none' || snap.source === 'manual' || !snap.windows.length) && <ManualLimits account={a} />}
          </section>

          <section>
            <SectionTitle
              aside={
                <Segmented
                  size="sm"
                  label="Time range"
                  value={range}
                  onChange={setRange}
                  options={[
                    { id: '7', label: '7 days' },
                    { id: '30', label: '30 days' }
                  ]}
                />
              }
            >
              Usage over time
            </SectionTitle>
            <WindowHistory windows={snap?.windows ?? []} history={history} />
          </section>

          <section>
            <SectionTitle>Activity from local sessions</SectionTitle>
            <dl className="mb-3 grid grid-cols-4 gap-4">
              <Fact label="Tokens" value={formatTokens(totals.tokens)} />
              <Fact label="Messages" value={totals.messages.toLocaleString()} />
              <Fact label="Sessions" value={totals.sessions.toLocaleString()} />
              <Fact label="Main model" value={topModel ?? 'None yet'} />
            </dl>
            <DailyTokens activity={activity} color={a.color} span={Number(range)} />
          </section>

          <section>
            <SectionTitle>Profile folder</SectionTitle>
            <div className="flex items-center gap-2 rounded border border-rivet bg-deck py-1 pl-3 pr-1">
              <code className="code min-w-0 flex-1 truncate" data-tip={a.homeDir}>
                {a.homeDir}
              </code>
              <CopyButton text={a.homeDir} label="Copy path" />
              <button className="btn btn-sm" onClick={openFolder}>
                <FolderOpen size={13} /> Open folder
              </button>
            </div>
            <p className="mt-2 text-xs text-muted">
              {a.isDefaultHome ? (
                <>This is {meta.short}'s main folder, so it launches without any override. </>
              ) : prov?.homeEnvVar ? (
                <>
                  Launched with <code className="code">{prov.homeEnvVar}</code> pointing here.{' '}
                </>
              ) : null}
              {a.launchCount ? `Launched ${plural(a.launchCount, 'time')}, last ${formatAgo(a.lastLaunchedAt, now)}.` : 'Not launched from Agent Hub yet.'}
            </p>
          </section>

          <section>
            <SectionTitle>Shared with your main setup</SectionTitle>
            {a.isDefaultHome ? (
              <p className="text-muted">This is your main {meta.short} setup. Other {meta.short} accounts can share its skills, plugins and history.</p>
            ) : (
              <>
                <p className="mb-3 text-muted">
                  Shared items are linked, not copied: change them in one place and every account sees it. Sign-in tokens are never shared.
                  {a.sharedFrom && (
                    <>
                      {' '}
                      Source: <code className="code">{a.sharedFrom}</code>
                    </>
                  )}
                </p>
                <ul className="divide-y divide-rivet rounded-lg border border-rivet">
                  {SHARING_ITEMS.map((s) => (
                    <li key={s.key} className="flex items-center gap-4 px-4 py-2.5">
                      <label htmlFor={`share-${s.key}`} className="min-w-0 flex-1">
                        <span className="block font-medium">{s.label}</span>
                        <span className="block text-xs text-muted">{s.explain}</span>
                      </label>
                      <Switch id={`share-${s.key}`} checked={a.sharing[s.key]} onChange={(v) => setSharing(s.key, v)} label={`Share ${s.label}`} />
                    </li>
                  ))}
                </ul>
              </>
            )}
          </section>

          <section className="border-t border-rivet pt-5">
            <button className="btn btn-danger" onClick={() => setRemoving(true)}>
              <Trash2 size={14} /> Remove account
            </button>
          </section>
        </div>
      </div>
      <EditDialog open={editing} onClose={() => setEditing(false)} account={a} />
      <RemoveDialog open={removing} onClose={() => setRemoving(false)} account={a} />
    </>
  )
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted">{label}</dt>
      <dd className="truncate text-md font-semibold tabular-nums">{value}</dd>
    </div>
  )
}

/** Profiles the user started creating and saved for later. Clicking one reopens the form as it was. */
function Drafts() {
  const drafts = useDrafts((s) => s.drafts)
  const now = useStore((s) => s.now)
  const setAddOpen = useStore((s) => s.setAddOpen)
  if (!drafts.length) return null
  return (
    <div className="mb-3">
      <div className="px-2 pb-1 pt-2 text-xs text-muted">Drafts</div>
      <ul>
        {drafts.map((d) => (
          <li key={d.id} className="group flex items-center rounded hover:bg-raised/60">
            <button
              className="flex min-w-0 flex-1 items-center gap-2 px-2 py-2 text-left"
              onClick={() => {
                useDrafts.getState().resume(d.id)
                setAddOpen(true)
              }}
              data-tip="Finish creating this profile"
            >
              <ProviderMark id={d.providerId} size={14} />
              <span className="min-w-0 flex-1">
                <span className="block truncate">{d.label.trim() || `New ${PROVIDER_META[d.providerId].short} profile`}</span>
                <span className="block truncate text-xs text-faint">
                  Draft{d.plan ? `, ${d.plan}` : ''}, saved {formatAgo(d.savedAt, now)}
                </span>
              </span>
            </button>
            <button
              className="mr-1 flex h-7 w-7 shrink-0 items-center justify-center rounded text-faint opacity-0 hover:bg-ground hover:text-ink focus:opacity-100 group-hover:opacity-100"
              onClick={() => useDrafts.getState().remove(d.id)}
              data-tip="Delete draft"
              aria-label={`Delete draft ${d.label || ''}`.trim()}
            >
              <Trash2 size={13} />
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}

function EditDialog({ open, onClose, account: a }: { open: boolean; onClose: () => void; account: Account }) {
  const updateAccount = useStore((s) => s.updateAccount)
  const [label, setLabel] = useState(a.label)
  const [color, setColor] = useState(a.color)
  const [tags, setTags] = useState<string[]>(a.tags)
  const [cost, setCost] = useState(a.monthlyCost?.toString() ?? '')
  const [plan, setPlan] = useState(a.plan ?? '')
  const [notes, setNotes] = useState(a.notes ?? '')
  useEffect(() => {
    if (open) {
      setLabel(a.label)
      setColor(a.color)
      setTags(a.tags)
      setCost(a.monthlyCost?.toString() ?? '')
      setPlan(a.plan ?? '')
      setNotes(a.notes ?? '')
    }
    // Only when the dialog opens: `a` is a new object on every accounts-changed, which would wipe the edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, a.id])
  const toggleTag = (t: TaskType) => setTags(tags.includes(t) ? tags.filter((x) => x !== t) : [...tags, t])
  const save = async () => {
    const ok = await updateAccount(a.id, {
      label: label.trim() || a.label,
      color,
      tags,
      monthlyCost: cost ? Number(cost) : null,
      plan: plan || null,
      notes: notes.trim() || null
    })
    if (ok) onClose()
  }
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Edit ${a.label}`}
      width={600}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={save}>
            Save changes
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="grid grid-cols-[1fr_180px_120px] gap-3">
          <label>
            <span className="mb-1 block font-medium">Name</span>
            <input className="w-full" value={label} onChange={(e) => setLabel(e.target.value)} />
          </label>
          <label>
            <span className="mb-1 block font-medium">Plan</span>
            <select
              className="w-full"
              value={plan}
              onChange={(e) => {
                setPlan(e.target.value)
                const price = findPlan(a.providerId, e.target.value)?.price
                if (price !== undefined) setCost(String(price))
              }}
            >
              {plan && !findPlan(a.providerId, plan) && <option value={plan}>{plan}</option>}
              {PLANS[a.providerId].map((p) => (
                <option key={p.name} value={p.name}>
                  {planLabel(p)}
                </option>
              ))}
              <option value="">Not set</option>
            </select>
          </label>
          <label>
            <span className="mb-1 block font-medium">Monthly cost ($)</span>
            <input className="w-full" inputMode="decimal" value={cost} onChange={(e) => setCost(e.target.value.replace(/[^\d.]/g, ''))} placeholder="Optional" />
          </label>
        </div>
        <fieldset>
          <legend className="mb-1.5 font-medium">Color</legend>
          <div className="flex gap-2">
            {ACCOUNT_COLORS.map((c, i) => (
              <button
                key={c}
                type="button"
                aria-label={ACCOUNT_COLOR_NAMES[i] ?? c}
                aria-pressed={c.toLowerCase() === color.toLowerCase()}
                onClick={() => setColor(c)}
                className={clsx('h-7 w-7 rounded-full border-2', c.toLowerCase() === color.toLowerCase() ? 'border-ink' : 'border-transparent')}
                style={{ background: c }}
              />
            ))}
          </div>
        </fieldset>
        <fieldset>
          <legend className="mb-0.5 font-medium">Good for</legend>
          <p className="mb-2 text-xs text-muted">Recommendations favor this account for these kinds of task.</p>
          <div className="flex flex-wrap gap-1.5">
            {TASKS.map((t) => {
              const on = tags.includes(t.id)
              return (
                <button
                  key={t.id}
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggleTag(t.id)}
                  className={clsx('h-7 rounded-full border px-3 text-xs', on ? 'border-hop bg-hop/15 font-semibold text-ink' : 'border-rivet text-muted hover:text-ink')}
                >
                  {t.label}
                </button>
              )
            })}
          </div>
        </fieldset>
        <label className="block">
          <span className="mb-1 block font-medium">Notes</span>
          <textarea className="w-full" rows={2} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="e.g. Company card, cancel in March" />
        </label>
      </div>
    </Dialog>
  )
}

function RemoveDialog({ open, onClose, account: a }: { open: boolean; onClose: () => void; account: Account }) {
  const removeAccount = useStore((s) => s.removeAccount)
  const settings = useStore((s) => s.settings)
  const [deleteHome, setDeleteHome] = useState(false)
  const [busy, setBusy] = useState(false)
  const [appData, setAppData] = useState<string>()
  useEffect(() => {
    if (open) setDeleteHome(false)
  }, [open])
  // Without the path the box stays available; the backend still refuses folders outside profiles.
  useEffect(() => {
    api.system.appDataDir().then(setAppData, () => {})
  }, [])
  // The backend only deletes folders inside the profiles folder (an imported ~/.claude-x is left alone).
  const outside = !a.isDefaultHome && appData !== undefined && !canDeleteHome(a, profilesDir(settings, appData), api.platform)
  const locked = a.isDefaultHome || outside
  // A box ticked before the folder turned out to be off limits must not still delete it.
  const wipe = deleteHome && !locked
  const noteId = useId()
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Remove ${a.label}?`}
      width={480}
      footer={
        <>
          <button className="btn" onClick={onClose} data-autofocus>
            Keep account
          </button>
          <button
            className="btn btn-destructive"
            disabled={busy}
            onClick={async () => {
              setBusy(true)
              const ok = await removeAccount(a.id, wipe)
              setBusy(false)
              if (ok) onClose()
            }}
          >
            {wipe ? 'Remove and delete folder' : 'Remove account'}
          </button>
        </>
      }
    >
      <p className="text-muted">
        Agent Hub stops tracking and launching this account. Its sign-in and chats stay in its folder{locked ? '' : ' unless you delete it'}.
      </p>
      <label className={clsx('mt-4 flex items-start gap-2.5', locked && 'opacity-60')}>
        <input type="checkbox" className="mt-0.5" checked={wipe} disabled={locked} aria-describedby={noteId} onChange={(e) => setDeleteHome(e.target.checked)} />
        <span>Also delete the profile folder</span>
      </label>
      <div id={noteId} className={clsx('pl-6', locked && 'opacity-60')}>
        <span className="code block break-all text-muted">{a.homeDir}</span>
        {a.isDefaultHome && <span className="block text-xs text-muted">This is {PROVIDER_META[a.providerId].short}'s main folder, so Agent Hub never deletes it.</span>}
        {outside && <span className="block text-xs text-muted">Agent Hub didn't create this folder, so it never deletes it.</span>}
        {wipe && <span className="block text-xs text-empty">This can't be undone. Sessions not shared with another account are lost.</span>}
      </div>
    </Dialog>
  )
}

interface ManualRow {
  label: string
  used: string
  resetHours: string
}

/** For accounts without live limits (API keys, unsupported plans): type in what the provider's site shows. */
function ManualLimits({ account }: { account: Account }) {
  const snap = useStore((s) => s.usage[account.id])
  const [open, setOpen] = useState(false)
  const [rows, setRows] = useState<ManualRow[]>([])
  const [saving, setSaving] = useState(false)
  const manual = snap?.source === 'manual' && snap.windows.length > 0

  const begin = () => {
    const now = Date.now()
    setRows(
      manual
        ? snap.windows.map((w) => ({
            label: w.label,
            used: String(Math.round(w.usedPercent)),
            resetHours: w.resetsAt ? String(Math.max(0, Math.round((Date.parse(w.resetsAt) - now) / 3_600_000))) : ''
          }))
        : [{ label: 'Weekly', used: '0', resetHours: '' }]
    )
    setOpen(true)
  }
  const set = (i: number, patch: Partial<ManualRow>) => setRows((r) => r.map((x, j) => (j === i ? { ...x, ...patch } : x)))
  const valid = rows.length > 0 && rows.every((r) => r.label.trim() && r.used !== '' && Number(r.used) >= 0 && Number(r.used) <= 100)

  const save = async () => {
    const now = Date.now()
    const windows: UsageWindow[] = rows.map((r, i) => {
      const hours = Number(r.resetHours)
      return {
        id: `manual_${i}`,
        label: r.label.trim(),
        usedPercent: Math.min(100, Math.max(0, Number(r.used))),
        resetsAt: r.resetHours !== '' && hours > 0 ? new Date(now + hours * 3_600_000).toISOString() : undefined
      }
    })
    setSaving(true)
    try {
      const updated = await api.usage.setManual(account.id, windows)
      useStore.setState((s) => ({ usage: { ...s.usage, [account.id]: updated } }))
      void useStore.getState().loadRecs()
      setOpen(false)
    } catch (e) {
      useStore.getState().toast({ kind: 'error', title: "Couldn't save the limits", body: errorText(e) })
    } finally {
      setSaving(false)
    }
  }

  return (
    <>
      <button className="btn btn-sm mt-3" onClick={begin}>
        <Pencil size={13} /> {manual ? 'Edit limits' : 'Enter limits by hand'}
      </button>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title={`Limits for ${account.label}`}
        footer={
          <>
            <button className="btn" onClick={() => setOpen(false)}>
              Cancel
            </button>
            <button className="btn btn-primary" disabled={!valid || saving} onClick={save}>
              Save limits
            </button>
          </>
        }
      >
        <p className="mb-3 text-muted">
          Copy what the provider's usage page shows. Agent Hub uses these for recommendations and warnings until live limits are available.
        </p>
        <div className="grid grid-cols-[1fr_88px_104px_28px] items-center gap-2 text-xs text-muted">
          <span>Window</span>
          <span>% used</span>
          <span>Resets in (hours)</span>
          <span />
          {rows.map((r, i) => (
            <Fragment key={i}>
              <input aria-label="Window name" value={r.label} onChange={(e) => set(i, { label: e.target.value })} placeholder="Weekly" />
              <input aria-label="Percent used" type="number" min={0} max={100} value={r.used} onChange={(e) => set(i, { used: e.target.value })} />
              <input aria-label="Hours until reset" type="number" min={0} value={r.resetHours} onChange={(e) => set(i, { resetHours: e.target.value })} placeholder="Unknown" />
              <button className="btn btn-ghost btn-sm px-1" aria-label="Remove window" onClick={() => setRows((x) => x.filter((_, j) => j !== i))}>
                <Trash2 size={13} />
              </button>
            </Fragment>
          ))}
        </div>
        <button className="btn btn-ghost btn-sm mt-2" onClick={() => setRows((x) => [...x, { label: '', used: '0', resetHours: '' }])}>
          <Plus size={13} /> Add a window
        </button>
      </Dialog>
    </>
  )
}
