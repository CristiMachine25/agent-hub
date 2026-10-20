import { useEffect, useRef, useState } from 'react'
import clsx from 'clsx'
import { ArrowRightLeft, FolderOpen, History, Loader2, LogIn, Maximize2, Minimize2, Play, RotateCcw, RotateCw, X } from 'lucide-react'
import type { Account } from '@shared/types'
import { api } from '@/api'
import { useStore } from '@/store'
import { atLimit, continueCandidates, historyShared, isSignedOut, pickerGroups, type PaneConfig, type PaneTarget } from '@/lib/grid'
import { bindingWindow, leftOf } from '@/lib/quota'
import { formatCountdown, formatPct, windowName } from '@/lib/format'
import { PROVIDER_META } from '@/lib/providers'
import { Meter } from '@/components/Meter'
import { AccountDot, ProviderMark } from '@/components/ui'
import { fitTerm, getTerm } from './registry'
import { useGrid } from './store'

/** Adopts the registry's xterm host and keeps it fitted to the pane. */
function TermView({ id }: { id: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const mode = useStore((s) => s.resolvedTheme)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const entry = getTerm(id, mode)
    el.appendChild(entry.host)
    let timer: ReturnType<typeof setTimeout> | undefined
    const ro = new ResizeObserver(() => {
      clearTimeout(timer)
      timer = setTimeout(() => fitTerm(id), 50)
    })
    ro.observe(el)
    requestAnimationFrame(() => fitTerm(id))
    return () => {
      clearTimeout(timer)
      ro.disconnect()
      if (entry.host.parentElement === el) el.removeChild(entry.host)
    }
    // The theme is applied by restyleAll; re-adopting on theme change is not needed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id])
  return <div ref={ref} className="h-full w-full overflow-hidden px-2 pt-1.5" />
}

function useAccount(id: string | undefined): Account | undefined {
  return useStore((s) => (id ? s.accounts.find((a) => a.id === id) : undefined))
}

/** "Claude H2 (100% left)" style candidate text. */
function leftText(a: Account, usage: ReturnType<typeof useStore.getState>['usage']): string {
  const b = bindingWindow(usage[a.id])
  if (isSignedOut(a)) return 'signed out'
  if (!b) return 'no live limits'
  return atLimit(usage[a.id]) ? 'at its limit' : `${formatPct(leftOf(b))} left`
}

function ContinueMenu({ index, account, onDone }: { index: number; account: Account; onDone(): void }) {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const recs = useStore((s) => s.recs)
  const updateAccount = useStore((s) => s.updateAccount)
  const continueOn = useGrid((s) => s.continueOn)
  const list = continueCandidates(account, accounts, usage, recs)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onDone()
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onDone()
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [onDone])

  return (
    <div ref={ref} role="menu" className="toast-in absolute right-1 top-10 z-30 w-[min(20rem,calc(100%-0.5rem))] rounded-lg border border-rivet bg-deck p-1.5 shadow-2xl shadow-black/40">
      <p className="px-2 pb-1.5 pt-1 text-xs text-muted">
        Ends this session and resumes it on another {PROVIDER_META[account.providerId].short} account. The chat carries over only
        when chat history sharing is on for both accounts.
      </p>
      {list.length === 0 && <p className="px-2 py-2 text-sm text-muted">No other {PROVIDER_META[account.providerId].short} accounts yet.</p>}
      {list.map((a) => {
        const usable = !isSignedOut(a) && !atLimit(usage[a.id])
        const shared = historyShared(account, a)
        return (
          <div key={a.id} className="flex items-center gap-1 rounded hover:bg-raised">
            <button
              role="menuitem"
              disabled={!usable}
              onClick={() => {
                onDone()
                void continueOn(index, a.id)
              }}
              className="flex min-w-0 flex-1 items-center gap-2 px-2 py-1.5 text-left disabled:opacity-50"
            >
              <AccountDot account={a} />
              <span className="truncate text-sm">{a.label}</span>
              <span className="ml-auto shrink-0 text-xs tabular-nums text-muted">{leftText(a, usage)}</span>
            </button>
            {!shared && usable && (
              <button
                className="btn btn-ghost btn-sm mr-1 shrink-0 px-1.5"
                title="Turn on chat history sharing for both accounts so the chat continues"
                onClick={async () => {
                  for (const x of [account, a]) {
                    if (!x.isDefaultHome && !x.sharing.history)
                      await updateAccount(x.id, { sharing: { ...x.sharing, history: true } }, { applySharing: true, quiet: true })
                  }
                  useStore.getState().toast({ kind: 'success', title: 'Chat history is now shared', body: `${account.label} and ${a.label} see the same chats.` })
                }}
              >
                <History size={13} /> Share history
              </button>
            )}
          </div>
        )
      })}
    </div>
  )
}

function LimitBanner({ index, account }: { index: number; account: Account }) {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const recs = useStore((s) => s.recs)
  const continueOn = useGrid((s) => s.continueOn)
  const b = bindingWindow(usage[account.id])
  const next = continueCandidates(account, accounts, usage, recs).find((a) => !isSignedOut(a) && !atLimit(usage[a.id]))
  return (
    <div className="flex items-center gap-3 border-b border-rivet bg-raised px-3 py-2 text-sm" role="status">
      <span className="h-2 w-2 shrink-0 rounded-full bg-empty" aria-hidden />
      <span className="min-w-0 flex-1 truncate">
        {account.label} hit its {b ? windowName(b.label).toLowerCase() : 'usage'} limit.
        {next ? ` Continue on ${next.label} (${leftText(next, usage)}).` : ' No other account has quota left right now.'}
      </span>
      {next && (
        <button className="btn btn-primary btn-sm shrink-0" onClick={() => void continueOn(index, next.id)}>
          Continue on {next.label}
        </button>
      )}
    </div>
  )
}

function Header({ index, pane, account }: { index: number; pane: PaneConfig; account: Account }) {
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const info = useGrid((s) => (pane.terminalId ? s.infos[pane.terminalId] : undefined))
  const maximized = useGrid((s) => s.ws.maximized === index)
  const starting = useGrid((s) => !!s.starting[pane.uid])
  const { restart, close, toggleMax } = useGrid.getState()
  const [menu, setMenu] = useState(false)
  const b = bindingWindow(usage[account.id])
  const left = b ? leftOf(b) : undefined
  const countdown = b?.resetsAt ? formatCountdown(b.resetsAt, now) : null

  const status = starting
    ? 'Starting…'
    : !info
      ? 'Not running'
      : info.exited
        ? `Exited${info.exitCode !== null && info.exitCode !== undefined ? ` (code ${info.exitCode})` : ''}`
        : pane.target === 'login'
          ? 'Signing in'
          : 'Running'

  return (
    <div className="pane-head relative flex h-9 shrink-0 items-center gap-2 border-b border-rivet bg-deck pl-3 pr-1">
      <AccountDot account={account} size={9} />
      <span className="min-w-[4.5rem] truncate text-sm font-semibold" title={account.label}>
        {account.label}
      </span>
      <span className="pane-provider flex shrink-0 items-center gap-1 text-xs text-muted">
        <ProviderMark id={account.providerId} size={12} />
        {PROVIDER_META[account.providerId].short}
      </span>
      {left !== undefined && b && (
        <span className="pane-meter ml-1 flex w-28 shrink-0 items-center gap-1.5" title={`${b.label}: ${formatPct(left)} left`}>
          <Meter left={left} cells={10} size="xs" label={`${account.label} ${b.label} left`} />
          <span className="shrink-0 text-[11px] tabular-nums text-muted">{countdown ?? formatPct(left)}</span>
        </span>
      )}
      <span className={clsx('pane-status ml-auto shrink-0 truncate pl-2 text-xs', info?.exited ? 'text-muted' : 'text-faint')}>{status}</span>
      <div className="ml-auto flex shrink-0 items-center">
        <button className="btn btn-ghost btn-sm px-1.5" title="Restart" aria-label="Restart" onClick={() => void restart(index, false)}>
          <RotateCw size={14} />
        </button>
        <button className="btn btn-ghost btn-sm px-1.5" title="Restart and resume the last session" aria-label="Resume last session" onClick={() => void restart(index, true)}>
          <RotateCcw size={14} />
        </button>
        <button
          className={clsx('btn btn-ghost btn-sm px-1.5', menu && 'bg-raised text-ink')}
          title="Continue on another account"
          aria-label="Continue on another account"
          aria-haspopup="menu"
          aria-expanded={menu}
          onClick={() => setMenu((m) => !m)}
        >
          <ArrowRightLeft size={14} />
        </button>
        <button
          className="btn btn-ghost btn-sm px-1.5"
          title={maximized ? 'Restore (Ctrl+Shift+Enter)' : 'Maximize (Ctrl+Shift+Enter)'}
          aria-label={maximized ? 'Restore pane' : 'Maximize pane'}
          onClick={() => toggleMax(index)}
        >
          {maximized ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
        </button>
        <button className="btn btn-ghost btn-sm px-1.5" title="Close" aria-label="Close pane" onClick={() => void close(index)}>
          <X size={14} />
        </button>
      </div>
      {menu && <ContinueMenu index={index} account={account} onDone={() => setMenu(false)} />}
    </div>
  )
}

/** Empty pane: choose an account, what to run, and where. */
function Picker({ index, pane }: { index: number; pane: PaneConfig }) {
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const recs = useStore((s) => s.recs)
  const providers = useStore((s) => s.providers)
  const defaultCwd = useStore((s) => s.settings?.defaultCwd)
  const start = useGrid((s) => s.start)
  const error = useGrid((s) => s.errors[pane.uid])
  const starting = useGrid((s) => !!s.starting[pane.uid])
  const groups = pickerGroups(accounts, recs)
  // Until the user picks, follow the best account (recommendations can arrive after the pane mounts).
  const [picked, setAccountId] = useState<string | undefined>(pane.accountId)
  const accountId = picked ?? groups[0]?.accounts.find((a) => !isSignedOut(a))?.id
  const [target, setTarget] = useState<PaneTarget>(pane.target)
  const [cwd, setCwd] = useState<string | undefined>(pane.cwd)
  const [resume, setResume] = useState(!!pane.resume)
  const chosen = accounts.find((a) => a.id === accountId)
  const prov = chosen ? providers.find((p) => p.id === chosen.providerId) : undefined

  if (!accounts.length)
    return (
      <div className="flex h-full items-center justify-center p-6 text-center text-muted">
        <div>
          <p className="font-serif text-lg text-ink">No accounts yet</p>
          <p className="mt-1">Add an account first, then start it here.</p>
          <button className="btn btn-primary mt-3" onClick={() => useStore.getState().setAddOpen(true)}>
            Add an account
          </button>
        </div>
      </div>
    )

  return (
    <div className="flex h-full overflow-y-auto">
      <form
        className="picker m-auto flex w-full max-w-[420px] flex-col gap-3 p-5"
        onSubmit={(e) => {
          e.preventDefault()
          if (accountId) void start(index, { accountId, target, cwd, resume: target === 'cli' && resume })
        }}
      >
        <p className="picker-title font-serif text-lg">{pane.accountId ? 'Start this pane again' : 'Start an AI CLI here'}</p>
        <div className="picker-list flex max-h-[220px] flex-col gap-2 overflow-y-auto rounded-lg border border-rivet bg-deck p-1.5" role="radiogroup" aria-label="Account">
          {groups.map((g) => (
            <div key={g.providerId}>
              <p className="flex items-center gap-1.5 px-2 pb-1 pt-1 text-xs text-muted">
                <ProviderMark id={g.providerId} size={13} /> {PROVIDER_META[g.providerId].product}
              </p>
              {g.accounts.map((a) => {
                const on = a.id === accountId
                const b = bindingWindow(usage[a.id])
                return (
                  <button
                    type="button"
                    key={a.id}
                    role="radio"
                    aria-checked={on}
                    onClick={() => setAccountId(a.id)}
                    className={clsx('flex w-full items-center gap-2 rounded px-2 py-1.5 text-left', on ? 'bg-raised ring-1 ring-hop/60' : 'hover:bg-raised/60')}
                  >
                    <AccountDot account={a} />
                    <span className="min-w-0 flex-1 truncate text-sm">{a.label}</span>
                    {b && !isSignedOut(a) && (
                      <span className="w-16 shrink-0">
                        <Meter left={leftOf(b)} cells={8} size="xs" label={`${a.label} quota left`} />
                      </span>
                    )}
                    <span className="w-20 shrink-0 text-right text-xs tabular-nums text-muted">{leftText(a, usage)}</span>
                  </button>
                )
              })}
            </div>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="inline-flex rounded border border-rivet p-0.5" role="radiogroup" aria-label="What to run">
            {(['cli', 'login'] as const).map((t) => (
              <button
                key={t}
                type="button"
                role="radio"
                aria-checked={target === t}
                onClick={() => setTarget(t)}
                className={clsx('flex h-6 items-center gap-1 rounded-[4px] px-2 text-xs', target === t ? 'bg-raised font-semibold text-ink' : 'text-muted hover:text-ink')}
              >
                {t === 'cli' ? chosen ? <ProviderMark id={chosen.providerId} size={11} /> : <Play size={11} /> : <LogIn size={11} />}
                {t === 'cli' ? (chosen ? PROVIDER_META[chosen.providerId].product : 'CLI') : 'Sign in'}
              </button>
            ))}
          </div>
          {target === 'cli' && (
            <label className="flex items-center gap-1.5 text-xs text-muted">
              <input type="checkbox" checked={resume} onChange={(e) => setResume(e.target.checked)} /> Resume last session
            </label>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="btn btn-sm shrink-0"
            onClick={async () => {
              const dir = await api.system.pickDirectory()
              if (dir) setCwd(dir)
            }}
          >
            <FolderOpen size={13} /> Folder
          </button>
          <code className="code min-w-0 flex-1 truncate text-muted" title={cwd ?? defaultCwd}>
            {cwd ?? defaultCwd ?? 'Home folder'}
          </code>
          {cwd && (
            <button type="button" className="btn btn-ghost btn-sm px-1.5" aria-label="Use the default folder" onClick={() => setCwd(undefined)}>
              <X size={13} />
            </button>
          )}
        </div>
        {prov && !prov.cliInstalled && (
          <p className="text-xs text-muted">
            {prov.name} isn't installed. Install it with <code className="code text-ink">{prov.installHint}</code>
          </p>
        )}
        {error && <p className="text-xs text-empty">{error}</p>}
        <button type="submit" className="btn btn-primary" disabled={!accountId || starting || (!!prov && !prov.cliInstalled)}>
          {starting ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} fill="currentColor" />}
          {starting ? 'Starting…' : chosen ? `Start as ${chosen.label}` : 'Pick an account'}
        </button>
      </form>
    </div>
  )
}

export function Pane({ index, pane }: { index: number; pane: PaneConfig }) {
  const account = useAccount(pane.accountId)
  const focused = useGrid((s) => s.ws.focused === index)
  const info = useGrid((s) => (pane.terminalId ? s.infos[pane.terminalId] : undefined))
  const usage = useStore((s) => s.usage[pane.accountId ?? ''])
  const focus = useGrid((s) => s.focus)
  const live = !!pane.terminalId && !!account
  const limited = live && !!info && !info.exited && pane.target === 'cli' && atLimit(usage)

  return (
    <section
      aria-label={account ? `Terminal: ${account.label}` : `Empty pane ${index + 1}`}
      onMouseDown={() => focus(index)}
      onFocus={() => focus(index)}
      className={clsx(
        'flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-ground transition-colors',
        focused ? 'border-hop/70 ring-1 ring-hop/40' : 'border-rivet'
      )}
    >
      {live && <Header index={index} pane={pane} account={account} />}
      {limited && <LimitBanner index={index} account={account} />}
      <div className="pane-body min-h-0 flex-1" data-pane-body={pane.uid}>
        {live ? <TermView id={pane.terminalId!} /> : <Picker key={pane.uid + (pane.accountId ?? '')} index={index} pane={pane} />}
      </div>
    </section>
  )
}
