import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { ExternalLink, Loader2, Search, X } from 'lucide-react'
import type { DirectorySkill, InstalledSkill } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META, PROVIDER_ORDER } from '@/lib/providers'
import { FEATURED, installedSkills } from '@/lib/skills'
import { ProviderMark, Segmented } from '@/components/ui'
import { Toolbar } from '@/components/Page'
import { InstalledRows, ListSection, SkillRows } from './skills/SkillList'
import { SkillDetail } from './skills/SkillDetail'
import { fromId, fromResult, openUrl, type Listing } from './skills/shared'

export function Skills() {
  const accounts = useStore((s) => s.accounts)
  const [installed, setInstalled] = useState<InstalledSkill[]>([])
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<DirectorySkill[] | null>(null)
  const [searching, setSearching] = useState(false)
  const [searchError, setSearchError] = useState<string>()
  const [attempt, setAttempt] = useState(0)
  const [open, setOpen] = useState<Listing | null>(null)
  const [browse, setBrowse] = useState<'featured' | 'installed'>('featured')
  const seq = useRef(0)
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

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
  }, [q, attempt])

  // "/" jumps to the search field from anywhere on this screen.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return
      const t = e.target as HTMLElement
      if (t.closest('input, textarea, select, [contenteditable="true"]') || document.querySelector('[role="dialog"]')) return
      e.preventDefault()
      searchRef.current?.focus()
      searchRef.current?.select()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const rows = () => [...(listRef.current?.querySelectorAll<HTMLElement>('[data-skill-row]') ?? [])]
  const focusRow = (el: HTMLElement | undefined) => {
    el?.focus()
    el?.scrollIntoView({ block: 'nearest' })
  }
  // Arrows move through the rows of every section as one list; Enter (a button's own click) opens one.
  const onListKey = (e: KeyboardEvent) => {
    const all = rows()
    const i = all.indexOf(document.activeElement as HTMLElement)
    if (i < 0) return
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (e.key === 'ArrowUp' && i === 0) searchRef.current?.focus()
      else focusRow(all[Math.min(all.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1))])
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault()
      focusRow(e.key === 'Home' ? all[0] : all[all.length - 1])
    }
  }
  const onSearchKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') setQuery('')
    else if (e.key === 'ArrowDown') {
      e.preventDefault()
      focusRow(rows()[0])
    } else if (e.key === 'Enter') {
      // Enter opens the first match, then the arrows carry on from there.
      const first = rows()[0]
      if (first && q.length >= 2 && results?.length) {
        focusRow(first)
        first.click()
      }
    }
  }

  const count = results && (results.length === 50 ? 'Top 50 results' : `${results.length} ${results.length === 1 ? 'result' : 'results'}`)
  const status =
    q.length < 2 ? '' : searching ? 'Searching skills.sh…' : searchError ? "Couldn't search skills.sh" : results?.length === 0 ? `No skills match “${q}”` : count ? `${count} for “${q}”` : ''

  // The featured lists stay up until the first results arrive, so the list doesn't go blank while it searches.
  const showBrowse = q.length < 2 || (!results && !searchError)
  const mine = useMemo(() => installedSkills(installed), [installed])

  return (
    <div className="flex h-full flex-col">
      <Toolbar title="Skills">
        <button className="btn btn-ghost btn-sm" onClick={() => openUrl('https://skills.sh')} data-tip="https://skills.sh">
          <ExternalLink size={13} aria-hidden /> skills.sh
        </button>
      </Toolbar>

      <div className="flex min-h-0 flex-1">
        <div className="flex w-[300px] shrink-0 flex-col border-r border-rivet bg-well min-[1100px]:w-[350px] min-[1400px]:w-[400px]">
          <div className="shrink-0 border-b border-rivet p-3">
            <div className="relative">
              <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted" aria-hidden />
              <input
                ref={searchRef}
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={onSearchKey}
                placeholder="Search skills.sh: react, pdf, testing"
                aria-label="Search skills"
                className="h-9 w-full pl-8 pr-14"
              />
              {searching ? (
                <Loader2 size={15} className="absolute right-2.5 top-1/2 -translate-y-1/2 animate-spin text-muted" aria-hidden />
              ) : query ? (
                <button className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-1 text-muted hover:text-ink" onClick={() => setQuery('')} aria-label="Clear search">
                  <X size={14} />
                </button>
              ) : (
                <kbd className="kbd pointer-events-none absolute right-2 top-1/2 -translate-y-1/2" aria-hidden>
                  /
                </kbd>
              )}
              <p className="sr-only" role="status">
                {status}
              </p>
            </div>
            {showBrowse && (
              <div className="mt-2.5">
                <Segmented
                  size="sm"
                  label="Show"
                  value={browse}
                  onChange={setBrowse}
                  options={[
                    { id: 'featured', label: 'Featured' },
                    { id: 'installed', label: `Installed ${mine.length}`, title: 'Skills in the shared folders of your tools' }
                  ]}
                />
              </div>
            )}
          </div>

          <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-2 pb-4 pt-1" onKeyDown={onListKey}>
            {showBrowse ? (
              browse === 'featured' ? (
                FEATURED.map((g) => (
                  <ListSection key={g.title} title={g.title}>
                    <SkillRows items={g.skills.map((s) => fromId(s.id))} installed={installed} tools={tools} openId={open?.id} onOpen={setOpen} />
                  </ListSection>
                ))
              ) : mine.length ? (
                <ListSection title="In your shared folders" aside={`${mine.length} ${mine.length === 1 ? 'skill' : 'skills'}`}>
                  <InstalledRows installed={installed} tools={tools} openId={open?.id} onOpen={setOpen} />
                </ListSection>
              ) : (
                <ListMessage title="No skills installed yet" body={tools.length ? 'Pick one from Featured or search skills.sh, read it, then install it for your tools.' : 'Add an account first: skills install into the shared folder of a tool you have accounts for.'}>
                  <button className="btn btn-sm" onClick={() => setBrowse('featured')}>
                    Show featured
                  </button>
                </ListMessage>
              )
            ) : searchError ? (
              <ListMessage title="Couldn't search skills.sh" body={searchError}>
                <button className="btn btn-sm btn-primary" onClick={() => setAttempt((n) => n + 1)}>
                  Try again
                </button>
              </ListMessage>
            ) : results && results.length === 0 ? (
              <ListMessage title={`No skills match “${q}”`} body="Try a broader word, such as the framework or file type you work with.">
                <button className="btn btn-sm" onClick={() => setQuery('')}>
                  Clear search
                </button>
              </ListMessage>
            ) : (
              results && (
                <ListSection title={count || 'Results'} aside="Most relevant first">
                  <SkillRows items={results.map(fromResult)} installed={installed} tools={tools} openId={open?.id} onOpen={setOpen} />
                </ListSection>
              )
            )}
          </div>
        </div>

        <div className="flex min-w-0 flex-1 flex-col">
          {open ? <SkillDetail key={open.id} listing={open} installed={installed} tools={tools} onChanged={loadInstalled} /> : <Intro installed={installed} tools={tools} />}
        </div>
      </div>
    </div>
  )
}

function ListMessage({ title, body, children }: { title: string; body: string; children?: React.ReactNode }) {
  return (
    <div className="mx-1 mt-3 flex flex-col items-start gap-2 rounded border border-dashed border-rivet px-4 py-5">
      <p className="font-semibold">{title}</p>
      <p className="break-words text-muted">{body}</p>
      {children && <div className="mt-1">{children}</div>}
    </div>
  )
}

/** The reading pane before a skill is picked: what skills are, where they go, and how to move around. */
function Intro({ installed, tools }: { installed: InstalledSkill[]; tools: typeof PROVIDER_ORDER }) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="max-w-[60ch] px-8 pb-12 pt-10">
        <h2 className="font-serif text-lg font-semibold">Read a skill, then install it</h2>
        <p className="mt-3 text-[14px] leading-[22px] text-muted">
          Skills come from the public skills.sh directory. Each one is a SKILL.md your agent follows, sometimes with scripts beside it. Installing puts it in a tool's shared folder, so every
          account that links skills from that folder gets it at once.
        </p>

        <h3 className="mt-8 text-xs font-semibold text-muted">Your shared folders</h3>
        {tools.length ? (
          <ul className="mt-2 divide-y divide-rivet border-y border-rivet">
            {tools.map((p) => {
              const n = installed.filter((s) => s.providerId === p).length
              return (
                <li key={p} className="flex items-center gap-2.5 py-2.5">
                  <ProviderMark id={p} size={15} />
                  <span className="flex-1 font-medium">{PROVIDER_META[p].product}</span>
                  <span className="tabular-nums text-muted">
                    {n} {n === 1 ? 'skill' : 'skills'}
                  </span>
                </li>
              )
            })}
          </ul>
        ) : (
          <p className="mt-2 text-muted">None yet. Add an account first: skills install into the shared folder of a tool you have accounts for. You can still browse and read them.</p>
        )}

        <h3 className="mt-8 text-xs font-semibold text-muted">Keys</h3>
        <dl className="mt-2 grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-2 text-muted">
          <dt>
            <kbd className="kbd">/</kbd>
          </dt>
          <dd>Search skills.sh</dd>
          <dt className="flex gap-1">
            <kbd className="kbd">↑</kbd>
            <kbd className="kbd">↓</kbd>
          </dt>
          <dd>Move through the list</dd>
          <dt>
            <kbd className="kbd">Enter</kbd>
          </dt>
          <dd>Open the skill</dd>
        </dl>
      </div>
    </div>
  )
}
