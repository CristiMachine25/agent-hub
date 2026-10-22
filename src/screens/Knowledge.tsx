import { useCallback, useEffect, useRef, useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, CopyPlus, FolderOpen, Loader2, RefreshCw } from 'lucide-react'
import type { KnowledgeItem, KnowledgeKind, PermissionModes, ProviderId, ProviderKnowledge } from '@shared/types'
import { DEFAULT_PERMISSION_MODES } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META, PROVIDER_ORDER } from '@/lib/providers'
import { AccountDot, EmptyState, ProviderMark, Segmented } from '@/components/ui'
import { Page } from '@/components/Page'

type ModeOption<P extends ProviderId> = { id: PermissionModes[P]; label: string; title: string }

const MODES: { [P in ProviderId]: ModeOption<P>[] } = {
  claude: [
    { id: 'default', label: 'Ask (default)', title: 'Asks before edits and commands' },
    { id: 'acceptEdits', label: 'Accept edits', title: 'Edits files without asking, still asks before commands' },
    { id: 'plan', label: 'Plan', title: 'Reads and plans only, changes nothing' },
    { id: 'bypassPermissions', label: 'Bypass permissions', title: 'Never asks' }
  ],
  codex: [
    { id: 'default', label: 'Ask (default)', title: 'Asks before edits and commands' },
    { id: 'auto', label: 'Full auto', title: 'Works without asking inside the sandbox' },
    { id: 'bypass', label: 'Bypass approvals and sandbox', title: 'Never asks and runs outside the sandbox' }
  ],
  gemini: [
    { id: 'default', label: 'Ask (default)', title: 'Asks before edits and commands' },
    { id: 'autoEdit', label: 'Auto-edit', title: 'Edits files without asking' },
    { id: 'yolo', label: 'YOLO', title: 'Never asks' }
  ],
  copilot: [
    { id: 'default', label: 'Ask (default)', title: 'Asks before using tools' },
    { id: 'allowAll', label: 'Allow all tools', title: 'Uses every tool without asking' }
  ]
}

/** Modes that let the CLI run commands without asking get a quiet warning. */
const UNGUARDED = new Set<string>(['bypassPermissions', 'bypass', 'yolo', 'allowAll'])

const SECTIONS: { label: string; kinds: KnowledgeKind[]; empty: string }[] = [
  { label: 'Skills', kinds: ['skills'], empty: 'No skills yet.' },
  { label: 'Agents and commands', kinds: ['agents', 'commands'], empty: 'No custom agents or slash commands.' },
  { label: 'Instructions', kinds: ['instructions'], empty: 'No global instructions file.' },
  { label: 'Plugins', kinds: ['plugins'], empty: 'No plugins installed.' },
  { label: 'Settings', kinds: ['settings'], empty: 'No shared settings file.' }
]

const KIND_LABEL: Record<string, string> = {
  skills: 'Skills',
  plugins: 'Plugins',
  agents: 'Agents',
  instructions: 'Instructions',
  settings: 'Settings',
  mcp: 'MCP'
}

/** Files are shown in mono, like anything you could paste into a terminal. */
const FILE_KINDS = new Set<KnowledgeKind>(['agents', 'commands', 'instructions', 'settings'])

/** Skills lists can get long; the rest is shown behind "Show all". */
const CHIP_LIMIT = 14

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`
const baseName = (p: string) => p.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || p

export function Knowledge() {
  const [data, setData] = useState<ProviderKnowledge[] | null>(null)
  const [error, setError] = useState<string>()
  const [loading, setLoading] = useState(false)
  const accounts = useStore((s) => s.accounts)
  const seq = useRef(0)

  const load = useCallback(async () => {
    const n = ++seq.current
    setLoading(true)
    try {
      const list = await api.knowledge.get()
      if (n !== seq.current) return
      list.sort((a, b) => PROVIDER_ORDER.indexOf(a.providerId) - PROVIDER_ORDER.indexOf(b.providerId))
      setData(list)
      setError(undefined)
    } catch (e) {
      if (n === seq.current) setError(errorText(e))
    } finally {
      if (n === seq.current) setLoading(false)
    }
  }, [])

  // Load when the screen opens, and again when accounts or their sharing change.
  useEffect(() => {
    void load()
  }, [load, accounts])

  return (
    <Page
      title="Knowledge"
      actions={
        <button className="btn btn-sm" onClick={() => load()} disabled={loading} data-tip="Read the shared folders again">
          <RefreshCw size={13} className={clsx(loading && 'animate-spin')} /> Refresh
        </button>
      }
    >
      <p className="-mt-3 max-w-[720px] text-muted">
        Each tool keeps one shared profile for skills, agents, instructions, plugins and settings. Accounts link to it instead of keeping their own
        copies, so every account works the same way. Sign-in tokens are never shared.
      </p>

      {!data && !error && (
        <div className="flex items-center gap-2 py-10 text-muted" role="status">
          <Loader2 size={16} className="animate-spin" /> Reading the shared folders…
        </div>
      )}

      {error && !data && (
        <EmptyState
          title="Couldn't read the shared folders"
          body={error}
          action={
            <button className="btn btn-primary" onClick={() => load()}>
              Try again
            </button>
          }
        />
      )}

      {error && data && (
        <p className="flex items-start gap-2 rounded border border-rivet bg-deck px-4 py-2.5 text-sm" role="alert">
          <AlertTriangle size={15} className="mt-px shrink-0 text-low" />
          <span className="min-w-0">
            Couldn't refresh, so this may be out of date. <span className="text-muted">{error}</span>
          </span>
        </p>
      )}

      {data && data.length === 0 && (
        <EmptyState title="Nothing shared yet" body="Add an account and Agent Hub sets up a shared profile for its tool." />
      )}

      {data?.map((k) => <ProviderCard key={k.providerId} k={k} onChanged={load} />)}
    </Page>
  )
}

function ProviderCard({ k, onChanged }: { k: ProviderKnowledge; onChanged: () => Promise<void> }) {
  const meta = PROVIDER_META[k.providerId]
  const toast = useStore((s) => s.toast)
  const [copying, setCopying] = useState(false)
  const main = baseName(k.mainFolder)

  const reseed = async () => {
    setCopying(true)
    try {
      const { added, kept } = await api.knowledge.reseed(k.providerId)
      toast(
        added
          ? {
              kind: 'success',
              title: `Copied ${plural(added, 'new item')} from ${main}`,
              body: kept ? `${plural(kept, 'item')} already in the shared folder ${kept === 1 ? 'was' : 'were'} left as they were.` : undefined
            }
          : { kind: 'info', title: `Nothing new in ${main}`, body: 'Everything in it is already in the shared folder.' }
      )
      await onChanged()
    } catch (e) {
      toast({ kind: 'error', title: `Couldn't copy from ${main}`, body: errorText(e) })
    } finally {
      setCopying(false)
    }
  }

  const open = (path: string) =>
    api.system.openPath(path).catch((e) => toast({ kind: 'error', title: "Couldn't open the folder", body: errorText(e) }))

  return (
    <section aria-labelledby={`kn-${k.providerId}`} className="overflow-hidden rounded-lg border border-rivet">
      <header className="flex flex-wrap items-center gap-x-4 gap-y-3 border-b border-rivet bg-deck px-5 py-3.5">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <ProviderMark id={k.providerId} size={22} />
          <div className="min-w-0">
            <h2 id={`kn-${k.providerId}`} className="font-serif text-[17px] font-semibold leading-6">
              {meta.product}
            </h2>
            <code className="code block truncate text-muted" data-tip={k.folder}>
              {k.folder}
            </code>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <button className="btn btn-sm" onClick={() => open(k.folder)}>
            <FolderOpen size={13} /> Open folder
          </button>
          <button className="btn btn-sm" onClick={reseed} disabled={copying} data-tip={`Copy anything new from ${k.mainFolder}. Nothing in the shared folder is overwritten.`}>
            {copying ? <Loader2 size={13} className="animate-spin" /> : <CopyPlus size={13} />} Copy new from {main}
          </button>
        </div>
      </header>

      <PermissionRow providerId={k.providerId} />

      <div className="grid grid-cols-1 min-[1100px]:grid-cols-[minmax(0,1fr)_minmax(280px,340px)]">
        <div className="divide-y divide-rivet border-rivet min-[1100px]:border-r">
          {SECTIONS.map((s) => (
            <ItemSection key={s.label} label={s.label} empty={s.empty} items={k.items.filter((i) => s.kinds.includes(i.kind))} />
          ))}
        </div>
        <UsedBy k={k} />
      </div>
    </section>
  )
}

function PermissionRow({ providerId }: { providerId: ProviderId }) {
  const settings = useStore((s) => s.settings)
  const save = useStore((s) => s.saveSettings)
  const modes = settings?.permissionModes ?? DEFAULT_PERMISSION_MODES
  const value = modes[providerId]
  const options = MODES[providerId] as { id: string; label: string; title: string }[]
  const tool = PROVIDER_META[providerId].product
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 border-b border-rivet px-5 py-3">
      <div className="min-w-0 max-w-[440px]">
        <div className="font-medium">Permission mode</div>
        <div className="text-xs text-muted">Used every time Agent Hub starts {tool}, for every account.</div>
        {UNGUARDED.has(value) && (
          <div className="mt-1 flex items-center gap-1.5 text-xs text-low">
            <AlertTriangle size={12} className="shrink-0" /> Runs commands without asking. Use it only in folders you trust.
          </div>
        )}
      </div>
      <Segmented
        label={`${tool} permission mode`}
        size="sm"
        value={value}
        options={options}
        onChange={(v) => save({ permissionModes: { ...modes, [providerId]: v } })}
      />
    </div>
  )
}

function ItemSection({ label, items, empty }: { label: string; items: KnowledgeItem[]; empty: string }) {
  const [all, setAll] = useState(false)
  const shown = all ? items : items.slice(0, CHIP_LIMIT)
  return (
    <div className="flex gap-4 px-5 py-3 max-[640px]:flex-col max-[640px]:gap-2">
      <div className="w-[150px] shrink-0 pt-0.5">
        <span className="font-medium">{label}</span> <span className="tabular-nums text-faint">{items.length}</span>
      </div>
      {items.length === 0 ? (
        <p className="pt-0.5 text-faint">{empty}</p>
      ) : (
        <ul className="flex min-w-0 flex-1 flex-wrap gap-1.5">
          {shown.map((i) => (
            <li
              key={i.path}
              data-tip={i.path}
              className={clsx(
                'flex h-6 max-w-full items-center truncate rounded border border-rivet bg-ground px-2 text-ink',
                FILE_KINDS.has(i.kind) ? 'code' : 'text-xs'
              )}
            >
              <span className="truncate">{i.name}</span>
            </li>
          ))}
          {items.length > CHIP_LIMIT && (
            <li>
              <button className="btn btn-ghost btn-sm h-6 px-2" onClick={() => setAll(!all)} aria-expanded={all}>
                {all ? 'Show fewer' : `Show all ${items.length}`}
              </button>
            </li>
          )}
        </ul>
      )}
    </div>
  )
}

export function UsedBy({ k }: { k: ProviderKnowledge }) {
  const accounts = useStore((s) => s.accounts)
  const go = useStore((s) => s.go)
  const meta = PROVIDER_META[k.providerId]
  return (
    <div className="border-t border-rivet px-5 py-3 min-[1100px]:border-t-0">
      <h3 className="mb-2 font-medium">
        Used by <span className="tabular-nums text-faint">{k.accounts.length}</span>
      </h3>
      {k.accounts.length === 0 ? (
        <p className="text-faint">No {meta.short} accounts yet.</p>
      ) : (
        <ul className="-mx-2 grid grid-cols-1 sm:grid-cols-2 min-[1100px]:grid-cols-1">
          {k.accounts.map((ka) => {
            const acc = accounts.find((a) => a.id === ka.accountId)
            return (
              <li key={ka.accountId}>
                <button
                  className="flex w-full flex-col items-start gap-1 rounded px-2 py-1.5 text-left hover:bg-raised"
                  onClick={() => go('accounts', ka.accountId)}
                  data-tip={`Open ${ka.label}`}
                >
                  <span className="flex min-w-0 max-w-full items-center gap-2">
                    {acc && <AccountDot account={acc} size={8} />}
                    <span className="truncate font-medium">{ka.label}</span>
                  </span>
                  {ka.isDefaultHome ? (
                    <span className="text-xs text-muted" data-tip={k.mainFolder}>
                      Uses its own folder (main)
                    </span>
                  ) : ka.kinds.length === 0 ? (
                    <span className="text-xs text-faint">Shares nothing</span>
                  ) : (
                    <span className="flex flex-wrap gap-1">
                      {ka.kinds.map((kind) => (
                        <span key={kind} className="rounded border border-rivet px-1.5 text-[11px] leading-[16px] text-muted">
                          {KIND_LABEL[kind] ?? kind}
                        </span>
                      ))}
                    </span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
