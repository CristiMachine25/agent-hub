import clsx from 'clsx'
import { Bell, Gauge, RefreshCw, Rocket, Settings2, SquareTerminal, Users } from 'lucide-react'
import { useStore, type Screen } from '@/store'
import { IS_MOCK, api } from '@/api'
import { formatAgo } from '@/lib/format'
import { modKey } from '@/lib/platform'
import { Logo } from './Logo'

const NAV: { id: Screen; label: string; icon: typeof Gauge; key: string }[] = [
  { id: 'overview', label: 'Overview', icon: Gauge, key: '1' },
  { id: 'accounts', label: 'Accounts', icon: Users, key: '2' },
  { id: 'terminals', label: 'Terminals', icon: SquareTerminal, key: '3' },
  { id: 'notifications', label: 'Notifications', icon: Bell, key: '4' },
  { id: 'settings', label: 'Settings', icon: Settings2, key: '5' }
]

export function Sidebar() {
  const screen = useStore((s) => s.screen)
  const go = useStore((s) => s.go)
  const unread = useStore((s) => s.notifications.filter((n) => !n.read).length)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const refreshing = useStore((s) => s.refreshing)
  const refresh = useStore((s) => s.refresh)
  const setPalette = useStore((s) => s.setPalette)

  const newest = Object.values(usage)
    .filter((u) => !u.stale)
    .map((u) => Date.parse(u.fetchedAt))
    .sort((a, b) => b - a)[0]

  return (
    <nav
      aria-label="Main"
      className="flex w-[208px] shrink-0 flex-col border-r border-rivet bg-deck max-[999px]:w-[60px]"
    >
      <div className="flex h-14 items-center gap-2.5 px-4 max-[999px]:justify-center max-[999px]:px-0">
        <Logo size={26} />
        <span className="text-md font-bold tracking-tight max-[999px]:hidden">Baton</span>
      </div>
      <ul className="flex flex-col gap-0.5 px-2 pt-2">
        {NAV.map((n) => {
          const on = screen === n.id
          const Icon = n.icon
          return (
            <li key={n.id}>
              <button
                onClick={() => go(n.id)}
                aria-current={on ? 'page' : undefined}
                title={`${n.label} (${n.key})`}
                className={clsx(
                  'relative flex h-9 w-full items-center gap-2.5 rounded px-2.5 text-sm transition-colors max-[999px]:justify-center',
                  on ? 'bg-raised font-semibold text-ink' : 'text-muted hover:bg-raised/60 hover:text-ink'
                )}
              >
                {on && <span className="absolute left-0 top-2 bottom-2 w-[3px] rounded-r bg-hop" aria-hidden />}
                <Icon size={17} strokeWidth={on ? 2.2 : 1.8} />
                <span className="flex-1 text-left max-[999px]:hidden">{n.label}</span>
                {n.id === 'notifications' && unread > 0 && (
                  <span
                    className="min-w-5 rounded-full bg-hop px-1.5 text-center text-[11px] font-bold leading-5 text-hop-ink max-[999px]:absolute max-[999px]:right-1 max-[999px]:top-0.5 max-[999px]:min-w-4 max-[999px]:px-1 max-[999px]:leading-4"
                    aria-label={`${unread} unread`}
                  >
                    {unread}
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ul>

      <div className="mt-auto flex flex-col gap-2 p-3 max-[999px]:items-center max-[999px]:px-1">
        <button
          onClick={() => setPalette(true)}
          className="btn w-full justify-between max-[999px]:w-10 max-[999px]:px-0"
          title={`Quick launch (${modKey(api.platform)}+K)`}
        >
          <span className="flex items-center gap-2">
            <Rocket size={15} />
            <span className="max-[999px]:hidden">Quick launch</span>
          </span>
          <span className="kbd max-[999px]:hidden">{modKey(api.platform)} K</span>
        </button>
        <button
          onClick={() => refresh()}
          disabled={refreshing}
          className="flex items-center gap-2 rounded px-1 py-1 text-xs text-muted hover:text-ink max-[999px]:justify-center"
          title="Refresh usage for every account"
        >
          <RefreshCw size={13} className={clsx(refreshing && 'animate-spin')} />
          <span className="max-[999px]:hidden">
            {refreshing ? 'Refreshing…' : newest ? `Usage updated ${formatAgo(new Date(newest).toISOString(), now)}` : 'Refresh usage'}
          </span>
        </button>
        {IS_MOCK && (
          <p className="px-1 text-[11px] leading-4 text-faint max-[999px]:hidden">
            Browser preview with demo data. Nothing here touches your real accounts.
          </p>
        )}
      </div>
    </nav>
  )
}
