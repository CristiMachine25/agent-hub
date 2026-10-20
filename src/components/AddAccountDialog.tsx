import { useEffect, useState } from 'react'
import clsx from 'clsx'
import { CheckCircle2, FolderOpen, Loader2 } from 'lucide-react'
import type { Account, DetectedAccount, ProviderId, SharingOptions } from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META, PROVIDER_ORDER, ACCOUNT_COLORS, SHARING_ITEMS } from '@/lib/providers'
import { plural } from '@/lib/format'
import { DEFAULT_PLAN, PLANS, findPlan, planLabel } from '@/lib/plans'
import { Dialog, ProviderMark, Segmented } from './ui'
import { CopyButton } from './Toasts'

type Mode = 'find' | 'create'
type Preset = 'recommended' | 'separate' | 'everything'

const PRESETS: Record<Preset, SharingOptions> = {
  recommended: DEFAULT_SHARING,
  separate: { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false },
  everything: { skills: true, plugins: true, agents: true, instructions: true, settings: true, mcp: true, history: true }
}

export function AddAccountDialog() {
  const open = useStore((s) => s.addOpen)
  const setAddOpen = useStore((s) => s.setAddOpen)
  const accounts = useStore((s) => s.accounts)
  const [mode, setMode] = useState<Mode>('find')
  useEffect(() => {
    if (open) setMode('find')
  }, [open])
  const close = () => setAddOpen(false)
  return (
    <Dialog open={open} onClose={close} title="Add an account" width={620}>
      <div className="mb-4">
        <Segmented
          label="How to add"
          value={mode}
          onChange={setMode}
          options={[
            { id: 'find', label: 'Find on this computer' },
            { id: 'create', label: 'Create a new profile' }
          ]}
        />
      </div>
      {mode === 'find' ? <FindStep onDone={close} onCreate={() => setMode('create')} /> : <CreateStep onDone={close} count={accounts.length} />}
    </Dialog>
  )
}

function FindStep({ onDone, onCreate }: { onDone: () => void; onCreate: () => void }) {
  const [found, setFound] = useState<DetectedAccount[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const { toast, go } = useStore.getState()

  const scan = async () => {
    setFound(null)
    setError(null)
    try {
      const f = await api.accounts.detect()
      setFound(f)
      setPicked(new Set(f.filter((d) => !d.alreadyManaged).map((d) => d.homeDir)))
    } catch (e) {
      setError(errorText(e))
    }
  }
  useEffect(() => {
    void scan()
  }, [])

  if (error)
    return (
      <div className="space-y-3">
        <p className="text-empty">Couldn't search this computer: {error}</p>
        <button className="btn" onClick={scan}>
          Search again
        </button>
      </div>
    )
  if (!found)
    return (
      <div className="flex items-center gap-2.5 py-8 text-muted" role="status">
        <Loader2 size={16} className="animate-spin" />
        Looking for Claude, Codex, Gemini and Copilot profiles in your home folder…
      </div>
    )

  const fresh = found.filter((d) => !d.alreadyManaged)
  const chosen = fresh.filter((d) => picked.has(d.homeDir))
  return (
    <div>
      {fresh.length === 0 ? (
        <p className="mb-3 text-muted">
          Every profile on this computer is already in Switchyard. To add another login for the same tool, create a new profile.
        </p>
      ) : (
        <p className="mb-3 text-muted">
          Found {plural(fresh.length, 'profile')} you haven't added yet. Importing keeps each one where it is and doesn't change its sign-in.
        </p>
      )}
      <ul className="divide-y divide-rivet rounded border border-rivet">
        {found.map((d) => {
          const id = `det-${d.homeDir}`
          return (
            <li key={d.homeDir} className={clsx('flex items-start gap-3 px-3 py-2.5', d.alreadyManaged && 'opacity-60')}>
              <input
                id={id}
                type="checkbox"
                className="mt-1"
                disabled={d.alreadyManaged}
                checked={d.alreadyManaged || picked.has(d.homeDir)}
                onChange={(e) => {
                  const next = new Set(picked)
                  if (e.target.checked) next.add(d.homeDir)
                  else next.delete(d.homeDir)
                  setPicked(next)
                }}
              />
              <label htmlFor={id} className="min-w-0 flex-1">
                <span className="flex items-center gap-2">
                  <ProviderMark id={d.providerId} size={16} />
                  <span className="font-medium">{PROVIDER_META[d.providerId].product}</span>
                  {d.plan && <span className="text-muted">{d.plan}</span>}
                  <span className="ml-auto text-xs text-muted">
                    {d.alreadyManaged ? 'Already added' : d.loggedIn ? (d.email ?? 'Signed in') : 'Not signed in'}
                  </span>
                </span>
                <span className="code mt-0.5 block truncate text-muted">{d.homeDir}</span>
                {d.hint && <span className="block text-xs text-faint">{d.hint}</span>}
              </label>
            </li>
          )
        })}
      </ul>
      <div className="mt-4 flex items-center justify-between gap-2">
        <button className="btn btn-ghost" onClick={onCreate}>
          Create a new profile instead
        </button>
        <div className="flex gap-2">
          <button className="btn" onClick={scan}>
            Search again
          </button>
          <button
            className="btn btn-primary"
            disabled={chosen.length === 0 || busy}
            onClick={async () => {
              setBusy(true)
              try {
                const created = await api.accounts.import(chosen)
                toast({ kind: 'success', title: `Imported ${plural(created.length, 'account')}` })
                go('accounts', created[0]?.id ?? null)
                onDone()
              } catch (e) {
                toast({ kind: 'error', title: "Couldn't import", body: errorText(e) })
              } finally {
                setBusy(false)
              }
            }}
          >
            {busy && <Loader2 size={14} className="animate-spin" />}
            {chosen.length ? `Import ${plural(chosen.length, 'account')}` : 'Import'}
          </button>
        </div>
      </div>
    </div>
  )
}

function CreateStep({ onDone, count }: { onDone: () => void; count: number }) {
  const providers = useStore((s) => s.providers)
  const defaultSharing = useStore((s) => s.settings?.defaultSharing ?? DEFAULT_SHARING)
  const { toast, launch, go } = useStore.getState()
  const [providerId, setProviderId] = useState<ProviderId>('claude')
  const [label, setLabel] = useState('')
  const [preset, setPreset] = useState<Preset>('recommended')
  const [folder, setFolder] = useState<string | null>(null)
  const [plan, setPlan] = useState(DEFAULT_PLAN.claude)
  const [cost, setCost] = useState(String(findPlan('claude', DEFAULT_PLAN.claude)?.price ?? ''))
  const [costEdited, setCostEdited] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [created, setCreated] = useState<Account | null>(null)

  const prov = providers.find((p) => p.id === providerId)
  const sharing = preset === 'recommended' ? defaultSharing : PRESETS[preset]

  const pickPlan = (id: ProviderId, name: string) => {
    setPlan(name)
    if (!costEdited) setCost(String(findPlan(id, name)?.price ?? ''))
  }
  const pickProvider = (id: ProviderId) => {
    setProviderId(id)
    pickPlan(id, DEFAULT_PLAN[id])
  }

  if (created)
    return (
      <div className="space-y-4">
        <div className="flex items-start gap-3">
          <CheckCircle2 className="mt-0.5 shrink-0 text-hop" size={20} />
          <div>
            <p className="font-semibold">{created.label} is ready</p>
            <p className="text-muted">
              Sign in once so Switchyard can launch it and track its limits. A terminal opens with the {PROVIDER_META[created.providerId].short} sign-in.
            </p>
            <p className="code mt-2 text-muted">{created.homeDir}</p>
          </div>
        </div>
        <div className="flex justify-end gap-2">
          <button
            className="btn"
            onClick={() => {
              go('accounts', created.id)
              onDone()
            }}
          >
            Sign in later
          </button>
          <button
            className="btn btn-primary"
            data-autofocus
            onClick={() => {
              void launch(created.id, 'login')
              go('accounts', created.id)
              onDone()
            }}
          >
            Sign in now
          </button>
        </div>
      </div>
    )

  return (
    <form
      className="space-y-5"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!label.trim()) {
          setError('Give the account a name you will recognise, like "Work Max".')
          return
        }
        setBusy(true)
        setError(null)
        try {
          const a = await api.accounts.create({
            providerId,
            label: label.trim(),
            homeDir: folder ?? undefined,
            sharing,
            color: ACCOUNT_COLORS[count % ACCOUNT_COLORS.length],
            monthlyCost: cost ? Number(cost) : undefined,
            plan: plan || undefined
          })
          setCreated(a)
        } catch (err) {
          setError(errorText(err))
        } finally {
          setBusy(false)
        }
      }}
    >
      <fieldset>
        <legend className="mb-2 font-medium">Tool</legend>
        <div className="grid grid-cols-2 gap-2">
          {PROVIDER_ORDER.map((id) => {
            const p = providers.find((x) => x.id === id)
            const on = id === providerId
            return (
              <label
                key={id}
                className={clsx(
                  'flex cursor-default items-center gap-2.5 rounded border px-3 py-2',
                  on ? 'border-hop bg-hop/10' : 'border-rivet hover:bg-raised'
                )}
              >
                <input type="radio" name="provider" className="sr-only" checked={on} onChange={() => pickProvider(id)} />
                <ProviderMark id={id} />
                <span className="min-w-0 flex-1">
                  <span className="block font-medium">{p?.name ?? PROVIDER_META[id].product}</span>
                  <span className={clsx('block text-xs', p?.cliInstalled ? 'text-muted' : 'text-low')}>
                    {p?.cliInstalled ? `Installed${p.cliVersion ? `, version ${p.cliVersion}` : ''}` : 'Not installed'}
                  </span>
                </span>
              </label>
            )
          })}
        </div>
        {prov && !prov.cliInstalled && (
          <div className="mt-2 flex items-center gap-2 rounded border border-rivet bg-ground py-1 pl-2.5 pr-1">
            <span className="text-xs text-muted">Install it first:</span>
            <code className="code flex-1 truncate">{prov.installHint}</code>
            <CopyButton text={prov.installHint} label="Copy install command" />
          </div>
        )}
      </fieldset>

      <div className="grid grid-cols-[1fr_190px_100px] gap-3">
        <label className="block">
          <span className="mb-1 block font-medium">Name</span>
          <input className="w-full" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="e.g. Work Max, Side project Plus" />
        </label>
        <label className="block">
          <span className="mb-1 block font-medium">Plan</span>
          <select className="w-full" value={plan} onChange={(e) => pickPlan(providerId, e.target.value)}>
            {PLANS[providerId].map((p) => (
              <option key={p.name} value={p.name}>
                {planLabel(p)}
              </option>
            ))}
            <option value="">Not sure</option>
          </select>
          <span className="mt-0.5 block truncate text-xs text-faint">{findPlan(providerId, plan)?.note ?? 'Switchyard fills it in after you sign in'}</span>
        </label>
        <label className="block">
          <span className="mb-1 block font-medium">Monthly cost</span>
          <input
            className="w-full"
            inputMode="decimal"
            value={cost}
            onChange={(e) => {
              setCost(e.target.value.replace(/[^\d.]/g, ''))
              setCostEdited(true)
            }}
            placeholder="$ optional"
          />
        </label>
      </div>

      <fieldset>
        <legend className="mb-1 font-medium">What to share with your main setup</legend>
        <div className="space-y-1.5">
          {(
            [
              ['recommended', 'Your usual setup', 'Skills, plugins, agents, instructions, MCP and chat history are shared. Settings stay separate.'],
              ['separate', 'Fully separate', 'A clean profile with nothing shared. Good for client work.'],
              ['everything', 'Everything, including settings', 'Behaves exactly like your main setup, just signed in as a different account.']
            ] as const
          ).map(([id, title, body]) => (
            <label key={id} className="flex items-start gap-2.5 rounded px-1 py-1">
              <input type="radio" name="preset" className="mt-1" checked={preset === id} onChange={() => setPreset(id)} style={{ accentColor: 'rgb(var(--hop))' }} />
              <span>
                <span className="block">{title}</span>
                <span className="block text-xs text-muted">{body}</span>
              </span>
            </label>
          ))}
        </div>
        <p className="mt-1 text-xs text-faint">
          Sign-in tokens are never shared. Change any of these later: {SHARING_ITEMS.map((s) => s.label.toLowerCase()).join(', ')}.
        </p>
      </fieldset>

      <div>
        <span className="mb-1 block font-medium">Profile folder</span>
        <div className="flex items-center gap-2">
          <span className={clsx('min-w-0 flex-1 truncate', folder ? 'code' : 'text-muted')}>
            {folder ?? 'Switchyard creates a new folder for this account'}
          </span>
          {folder && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setFolder(null)}>
              Use a new folder
            </button>
          )}
          <button
            type="button"
            className="btn btn-sm"
            onClick={async () => {
              const p = await api.system.pickDirectory()
              if (p) setFolder(p)
            }}
          >
            <FolderOpen size={14} /> Choose existing folder
          </button>
        </div>
      </div>

      {error && <p className="text-empty" role="alert">{error}</p>}

      <div className="flex justify-end gap-2 border-t border-rivet pt-4">
        <button type="button" className="btn" onClick={onDone}>
          Cancel
        </button>
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy && <Loader2 size={14} className="animate-spin" />}
          Create profile
        </button>
      </div>
    </form>
  )
}
