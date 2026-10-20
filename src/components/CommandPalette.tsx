import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import clsx from 'clsx'
import { Search } from 'lucide-react'
import { useStore } from '@/store'
import { useGrid } from '@/terminals/store'
import { PROVIDER_META } from '@/lib/providers'
import { bindingWindow, leftOf } from '@/lib/quota'
import { formatPct } from '@/lib/format'
import { AccountDot } from './ui'
import { Meter } from './Meter'

interface Item {
  id: string
  group: string
  title: string
  sub?: string
  keywords: string
  lead?: ReactNode
  trail?: ReactNode
  run(): void
}

export function CommandPalette() {
  const open = useStore((s) => s.paletteOpen)
  const setPalette = useStore((s) => s.setPalette)
  const accounts = useStore((s) => s.accounts)
  const usage = useStore((s) => s.usage)
  const recs = useStore((s) => s.recs)
  const providers = useStore((s) => s.providers)
  const { launch, go, refresh, setAddOpen } = useStore.getState()
  const [q, setQ] = useState('')
  const [active, setActive] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLUListElement>(null)

  useEffect(() => {
    if (open) {
      setQ('')
      setActive(0)
      setTimeout(() => input.current?.focus(), 0)
    }
  }, [open])

  const items = useMemo<Item[]>(() => {
    const close = () => setPalette(false)
    const rank = new Map(recs.map((r, i) => [r.accountId, i]))
    const sorted = [...accounts].sort((a, b) => (rank.get(a.id) ?? 99) - (rank.get(b.id) ?? 99))
    const out: Item[] = []
    for (const a of sorted) {
      const p = PROVIDER_META[a.providerId]
      const prov = providers.find((x) => x.id === a.providerId)
      const bind = bindingWindow(usage[a.id])
      const left = bind ? leftOf(bind) : undefined
      const kw = `${a.label} ${p.short} ${p.product} ${a.plan ?? ''} ${a.email ?? ''} ${a.tags.join(' ')}`
      const lead = <AccountDot account={a} />
      if (a.status === 'needs-login' || a.status === 'expired') {
        out.push({
          id: `login-${a.id}`,
          group: 'Launch',
          title: `Sign in to ${a.label}`,
          sub: `${p.product}, signed out`,
          keywords: `sign in login ${kw}`,
          lead,
          run: () => {
            close()
            void launch(a.id, 'login')
          }
        })
        continue
      }
      out.push({
        id: `cli-${a.id}`,
        group: 'Launch',
        title: `Launch ${p.short} as ${a.label}`,
        sub: a.plan ? `${p.product}, ${a.plan}` : p.product,
        keywords: `launch open start cli ${kw}`,
        lead,
        trail:
          left !== undefined ? (
            <span className="flex w-28 flex-col items-end gap-1">
              <span className="text-xs tabular-nums text-muted">{left <= 0.5 ? 'Limit reached' : `${formatPct(left)} left`}</span>
              <Meter left={left} cells={14} size="xs" label={`${a.label} quota left`} />
            </span>
          ) : undefined,
        run: () => {
          close()
          void launch(a.id, 'cli')
        }
      })
      out.push({
        id: `grid-${a.id}`,
        group: 'More actions',
        title: `Open ${p.short} in the grid as ${a.label}`,
        keywords: `grid terminal pane embedded split ${kw}`,
        lead,
        run: () => {
          close()
          void useGrid.getState().openInGrid(a.id)
        }
      })
      out.push({
        id: `resume-${a.id}`,
        group: 'More actions',
        title: `Resume last session as ${a.label}`,
        keywords: `resume continue last session ${kw}`,
        lead,
        run: () => {
          close()
          void launch(a.id, 'cli', { resume: true })
        }
      })
      if (prov?.desktopInstalled) {
        out.push({
          id: `desk-${a.id}`,
          group: 'More actions',
          title: `Open ${prov.desktopName ?? 'desktop app'} as ${a.label}`,
          keywords: `desktop app ${kw}`,
          lead,
          run: () => {
            close()
            void launch(a.id, 'desktop')
          }
        })
      }
    }
    for (const a of sorted) {
      out.push({
        id: `show-${a.id}`,
        group: 'More actions',
        title: `Show details for ${a.label}`,
        keywords: `show details account settings sharing ${a.label}`,
        lead: <AccountDot account={a} />,
        run: () => {
          close()
          go('accounts', a.id)
        }
      })
    }
    const nav: [string, string, () => void][] = [
      ['Go to Overview', 'overview dashboard home', () => go('overview')],
      ['Go to Accounts', 'accounts list', () => go('accounts')],
      ['Go to Notifications', 'notifications alerts', () => go('notifications')],
      ['Go to Settings', 'settings preferences', () => go('settings')],
      ['Add an account', 'add new create import find detect account', () => setAddOpen(true)],
      ['Refresh usage for every account', 'refresh reload update usage', () => void refresh()]
    ]
    for (const [title, kw, fn] of nav) {
      out.push({
        id: title,
        group: 'App',
        title,
        keywords: kw,
        run: () => {
          close()
          fn()
        }
      })
    }
    return out
  }, [accounts, usage, recs, providers, launch, go, refresh, setAddOpen, setPalette])

  const filtered = useMemo(() => {
    const terms = q.toLowerCase().split(/\s+/).filter(Boolean)
    if (!terms.length) return items.filter((i) => i.group !== 'More actions')
    return items.filter((i) => {
      const hay = `${i.title} ${i.keywords}`.toLowerCase()
      return terms.every((t) => hay.includes(t))
    })
  }, [items, q])

  useEffect(() => {
    setActive(0)
  }, [q])

  useEffect(() => {
    list.current?.querySelector(`[data-idx="${active}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [active])

  if (!open) return null

  let lastGroup = ''
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 px-4 pt-[12vh]" onMouseDown={() => setPalette(false)}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Quick launch"
        onMouseDown={(e) => e.stopPropagation()}
        className="toast-in w-full max-w-[600px] overflow-hidden rounded-lg border border-rivet bg-deck shadow-2xl shadow-black/50"
      >
        <div className="flex items-center gap-2.5 border-b border-rivet px-4">
          <Search size={16} className="text-muted" />
          <input
            ref={input}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Launch an account, or type a command"
            aria-label="Search accounts and commands"
            aria-controls="palette-list"
            aria-activedescendant={filtered[active] ? `pi-${active}` : undefined}
            className="h-12 flex-1 border-0 bg-transparent px-0 text-base focus-visible:outline-none"
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') {
                e.preventDefault()
                setActive((a) => Math.min(filtered.length - 1, a + 1))
              } else if (e.key === 'ArrowUp') {
                e.preventDefault()
                setActive((a) => Math.max(0, a - 1))
              } else if (e.key === 'Enter') {
                e.preventDefault()
                filtered[active]?.run()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                setPalette(false)
              }
            }}
          />
          <span className="kbd">Esc</span>
        </div>
        <ul id="palette-list" ref={list} role="listbox" className="max-h-[52vh] overflow-y-auto py-1.5">
          {filtered.length === 0 && <li className="px-4 py-6 text-center text-muted">Nothing matches “{q}”.</li>}
          {filtered.map((it, i) => {
            const header = it.group !== lastGroup ? it.group : null
            lastGroup = it.group
            return (
              <li key={it.id} role="presentation">
                {header && <div className="px-4 pb-1 pt-2.5 text-xs font-medium text-muted">{header}</div>}
                <div
                  id={`pi-${i}`}
                  data-idx={i}
                  role="option"
                  aria-selected={i === active}
                  onMouseMove={() => setActive(i)}
                  onClick={() => it.run()}
                  className={clsx(
                    'mx-1.5 flex items-center gap-3 rounded px-2.5 py-2',
                    i === active ? 'bg-raised shadow-[inset_2px_0_0_rgb(var(--hop))]' : ''
                  )}
                >
                  <span className="flex w-3 justify-center">{it.lead}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate">{it.title}</span>
                    {it.sub && <span className="block truncate text-xs text-muted">{it.sub}</span>}
                  </span>
                  {it.trail}
                </div>
              </li>
            )
          })}
        </ul>
        <div className="flex items-center gap-4 border-t border-rivet px-4 py-2 text-xs text-muted">
          <span>
            <span className="kbd">↑</span> <span className="kbd">↓</span> to move
          </span>
          <span>
            <span className="kbd">Enter</span> to run
          </span>
          <span className="ml-auto">Launches open in your terminal</span>
        </div>
      </div>
    </div>
  )
}
