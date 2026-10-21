import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, Check, ExternalLink, Loader2, Search, Trash2, X } from 'lucide-react'
import type { DirectorySkill, InstalledSkill, ProviderId, SkillPreview } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META, PROVIDER_ORDER } from '@/lib/providers'
import { formatTokens } from '@/lib/format'
import { FEATURED, featuredFor, folderOf, installState, repoUrl, skillPageUrl } from '@/lib/skills'
import { Dialog, EmptyState, ProviderMark, SectionTitle } from '@/components/ui'
import { Page } from './Overview'

/** What a row needs: a directory result, or a featured entry before its counts are known. */
type Listing = { id: string; name: string; source: string; installs?: number; blurb?: string }

const toolList = new Intl.ListFormat('en', { type: 'conjunction' })

const fromId = (id: string): Listing => ({ id, name: folderOf(id), source: id.slice(0, id.lastIndexOf('/')), blurb: featuredFor(id)?.blurb })
const fromResult = (s: DirectorySkill): Listing => ({ ...s, blurb: featuredFor(s.id)?.blurb })

export function Skills() {
  const accounts = useStore((s) => s.accounts)
  const [installed, setInstalled] = useState<InstalledSkill[]>([])
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<DirectorySkill[] | null>(null)
  const [searching, setSearching] = useState(false)
  const [searchError, setSearchError] = useState<string>()
  const [open, setOpen] = useState<Listing | null>(null)
  const seq = useRef(0)

  // Tools that have accounts: only their shared folders can take a skill.
  const tools = useMemo(() => PROVIDER_ORDER.filter((p) => accounts.some((a) => a.providerId === p)), [accounts])

  const loadInstalled = useCallback(async () => {
    try {
      setInstalled(await api.skills.installed())
    } catch {
      /* the marks just stay unlit; installing still reports its own errors */
    }
  }, [])

  useEffect(() => {
    void loadInstalled()
  }, [loadInstalled, accounts])

  const q = query.trim()
  useEffect(() => {
    const n = ++seq.current
    if (q.length < 2) {
      setResults(null)
      setSearching(false)
      setSearchError(undefined)
      return
    }
    setSearching(true)
    const t = setTimeout(async () => {
      try {
        const found = await api.skills.search(q)
        if (n !== seq.current) return
        setResults(found)
        setSearchError(undefined)
      } catch (e) {
        if (n === seq.current) setSearchError(errorText(e))
      } finally {
        if (n === seq.current) setSearching(false)
      }
    }, 250)
    return () => clearTimeout(t)
  }, [q])

  return (
    <Page title="Skills">
      <p className="-mt-3 max-w-[720px] text-muted">
        Find skills in the public{' '}
        <button className="text-ink underline decoration-faint underline-offset-2 hover:decoration-ink" onClick={() => openUrl('https://skills.sh')}>
          skills.sh
        </button>{' '}
        directory. A skill installs into a tool's shared folder, so every account that links skills from it gets the skill at once.
      </p>

      <div className="relative max-w-[720px]">
        <Search size={16} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted" />
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Escape' && setQuery('')}
          placeholder="Search skills: react, pdf, testing, postgres…"
          aria-label="Search skills"
          className="h-10 w-full pl-9 pr-9"
        />
        {searching ? (
          <Loader2 size={15} className="absolute right-3 top-1/2 -translate-y-1/2 animate-spin text-muted" aria-label="Searching" />
        ) : (
          query && (
            <button className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1 text-muted hover:text-ink" onClick={() => setQuery('')} aria-label="Clear search">
              <X size={14} />
            </button>
          )
        )}
      </div>

      {tools.length === 0 && (
        <EmptyState title="Add an account first" body="Skills install into the shared folder of a tool you have accounts for. You can still browse and read them." />
      )}

      {q.length < 2 ? (
        FEATURED.map((g) => (
          <section key={g.title} aria-label={g.title}>
            <SectionTitle>{g.title}</SectionTitle>
            <SkillList items={g.skills.map((s) => fromId(s.id))} installed={installed} tools={tools} onOpen={setOpen} />
          </section>
        ))
      ) : searchError ? (
        <EmptyState title="Couldn't search skills.sh" body={searchError} />
      ) : results && results.length === 0 ? (
        <EmptyState title={`No skills match “${q}”`} body="Try a broader word, such as the framework or file type you work with." />
      ) : (
        results && (
          <section aria-label="Search results">
            <SectionTitle aside={<span className="text-xs text-muted">Most relevant first</span>}>
              {results.length === 50 ? 'Top 50 results' : `${results.length} ${results.length === 1 ? 'result' : 'results'}`}
            </SectionTitle>
            <SkillList items={results.map(fromResult)} installed={installed} tools={tools} onOpen={setOpen} />
          </section>
        )
      )}

      {open && <SkillDialog listing={open} installed={installed} tools={tools} onClose={() => setOpen(null)} onChanged={loadInstalled} />}
    </Page>
  )
}

function openUrl(url: string) {
  api.system.openUrl(url).catch((e) => useStore.getState().toast({ kind: 'error', title: "Couldn't open the link", body: errorText(e) }))
}

function SkillList({ items, installed, tools, onOpen }: { items: Listing[]; installed: InstalledSkill[]; tools: ProviderId[]; onOpen: (l: Listing) => void }) {
  return (
    <ul className="grid grid-cols-1 overflow-hidden rounded-lg border border-rivet min-[1100px]:grid-cols-2">
      {items.map((it) => (
        <li key={it.id} className="border-b border-rivet last:border-b-0 min-[1100px]:[&:nth-child(odd):not(:last-child)]:border-r min-[1100px]:[&:nth-last-child(2):nth-child(odd)]:border-b-0">
          <button className="flex h-full w-full items-start gap-3 bg-deck px-4 py-3 text-left hover:bg-raised" onClick={() => onOpen(it)} title={`Read ${it.name} before installing`}>
            <span className="min-w-0 flex-1">
              <span className="flex min-w-0 items-baseline gap-2">
                <span className="truncate font-semibold">{it.name}</span>
                <span className="truncate text-xs text-muted">{it.source}</span>
              </span>
              {it.blurb && <span className="mt-0.5 block text-muted">{it.blurb}</span>}
              {it.installs !== undefined && it.installs > 0 && <span className="mt-0.5 block text-xs tabular-nums text-faint">{formatTokens(it.installs)} installs</span>}
            </span>
            <ToolMarks id={it.id} installed={installed} tools={tools} />
          </button>
        </li>
      ))}
    </ul>
  )
}

/** One mark per tool with accounts: lit where this skill is installed. */
function ToolMarks({ id, installed, tools }: { id: string; installed: InstalledSkill[]; tools: ProviderId[] }) {
  if (tools.length === 0) return null
  return (
    <span className="flex shrink-0 items-center gap-1.5 pt-0.5">
      {tools.map((p) => {
        const state = installState(id, p, installed)
        const tool = PROVIDER_META[p].product
        const title = state === 'installed' ? `Installed for ${tool}` : state === 'taken' ? `${tool} already has a different skill named ${folderOf(id)}` : `Not installed for ${tool}`
        return (
          <span key={p} title={title} aria-label={title} className={clsx('flex', state === 'absent' && 'opacity-25 grayscale', state === 'taken' && 'opacity-60')}>
            <ProviderMark id={p} size={15} />
          </span>
        )
      })}
    </span>
  )
}

function SkillDialog({
  listing,
  installed,
  tools,
  onClose,
  onChanged
}: {
  listing: Listing
  installed: InstalledSkill[]
  tools: ProviderId[]
  onClose: () => void
  onChanged: () => Promise<void>
}) {
  const toast = useStore((s) => s.toast)
  const [preview, setPreview] = useState<SkillPreview | null>(null)
  const [previewError, setPreviewError] = useState<string>()
  const absent = tools.filter((p) => installState(listing.id, p, installed) === 'absent')
  const [picked, setPicked] = useState<Set<ProviderId>>(() => new Set(absent))
  const [busy, setBusy] = useState<'install' | ProviderId | null>(null)

  useEffect(() => {
    let live = true
    api.skills
      .preview(listing.id)
      .then((p) => live && setPreview(p))
      .catch((e) => live && setPreviewError(errorText(e)))
    return () => {
      live = false
    }
  }, [listing.id])

  const chosen = absent.filter((p) => picked.has(p))
  const install = async () => {
    setBusy('install')
    try {
      const outcomes = await api.skills.install(listing.id, chosen)
      const ok = outcomes.filter((o) => !o.error).map((o) => PROVIDER_META[o.providerId].short)
      const failed = outcomes.filter((o) => o.error)
      if (ok.length) toast({ kind: 'success', title: `Installed ${listing.name} for ${toolList.format(ok)}`, body: 'Accounts that link skills from the shared folder have it now.' })
      for (const f of failed) toast({ kind: 'error', title: `Couldn't install ${listing.name} for ${PROVIDER_META[f.providerId].short}`, body: f.error ?? undefined })
      await onChanged()
      if (!failed.length) onClose()
    } catch (e) {
      toast({ kind: 'error', title: `Couldn't install ${listing.name}`, body: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  const remove = async (p: ProviderId) => {
    setBusy(p)
    try {
      await api.skills.remove(listing.id, p)
      toast({ kind: 'success', title: `Removed ${listing.name} from ${PROVIDER_META[p].short}` })
      await onChanged()
    } catch (e) {
      toast({ kind: 'error', title: `Couldn't remove ${listing.name}`, body: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  return (
    <Dialog
      open
      onClose={onClose}
      title={listing.name}
      width={760}
      footer={
        <>
          <button className="btn mr-auto" onClick={() => openUrl(repoUrl(listing.source))} title={repoUrl(listing.source)}>
            <ExternalLink size={14} /> GitHub
          </button>
          <button className="btn" onClick={() => openUrl(skillPageUrl(listing.id))} title={skillPageUrl(listing.id)}>
            <ExternalLink size={14} /> skills.sh
          </button>
          <button className="btn btn-primary" onClick={install} disabled={!chosen.length || busy !== null}>
            {busy === 'install' && <Loader2 size={14} className="animate-spin" />}
            {chosen.length ? `Install for ${chosen.length === 1 ? PROVIDER_META[chosen[0]].short : `${chosen.length} tools`}` : 'Install'}
          </button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div>
          <div className="text-xs text-muted">
            {listing.source}
            {listing.installs ? ` · ${formatTokens(listing.installs)} installs` : ''}
            {preview ? ` · ${preview.files} ${preview.files === 1 ? 'file' : 'files'}` : ''}
          </div>
          {(preview?.description || listing.blurb) && <p className="mt-1 max-w-[640px]">{preview?.description || listing.blurb}</p>}
        </div>

        {tools.length > 0 && (
          <fieldset className="flex flex-col gap-1.5">
            <legend className="mb-1.5 font-medium">Install for</legend>
            {tools.map((p) => {
              const state = installState(listing.id, p, installed)
              const tool = PROVIDER_META[p].product
              return (
                <label key={p} className={clsx('flex items-center gap-2.5', state !== 'absent' && 'text-muted')}>
                  <input
                    type="checkbox"
                    checked={state === 'installed' || (state === 'absent' && picked.has(p))}
                    disabled={state !== 'absent' || busy !== null}
                    onChange={(e) => {
                      const next = new Set(picked)
                      if (e.target.checked) next.add(p)
                      else next.delete(p)
                      setPicked(next)
                    }}
                  />
                  <ProviderMark id={p} size={15} />
                  <span className="flex-1">
                    {tool}
                    {state === 'installed' && <span className="ml-2 inline-flex items-center gap-1 text-xs text-full"><Check size={12} /> Installed</span>}
                    {state === 'taken' && <span className="ml-2 text-xs">A different skill named {folderOf(listing.id)} is already there</span>}
                  </span>
                  {state === 'installed' && (
                    <button type="button" className="btn btn-ghost btn-sm btn-danger" onClick={() => remove(p)} disabled={busy !== null}>
                      {busy === p ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />} Remove
                    </button>
                  )}
                </label>
              )
            })}
          </fieldset>
        )}

        <p className="flex items-start gap-2 text-xs text-muted">
          <AlertTriangle size={13} className="mt-px shrink-0 text-low" />
          Skills are instructions and scripts your agent follows. Read SKILL.md before you install one from a source you don't know.
        </p>

        <div>
          <div className="mb-1.5 font-medium">SKILL.md</div>
          {previewError ? (
            <p className="text-muted">Couldn't load it: {previewError}</p>
          ) : !preview ? (
            <div className="flex items-center gap-2 py-6 text-muted" role="status">
              <Loader2 size={15} className="animate-spin" /> Reading SKILL.md…
            </div>
          ) : (
            <pre className="code max-h-[42vh] overflow-auto whitespace-pre-wrap break-words rounded border border-rivet bg-well px-3 py-2.5 text-ink">{preview.body}</pre>
          )}
        </div>
      </div>
    </Dialog>
  )
}
