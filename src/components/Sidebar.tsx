import clsx from 'clsx'
import { useEffect } from 'react'
import { Bell, Blocks, Gauge, Library, PanelLeftClose, PanelLeftOpen, RefreshCw, Settings, SquareTerminal, Users } from 'lucide-react'
import { useStore, type Screen } from '@/store'
import { IS_MOCK, api } from '@/api'
import { formatAgo } from '@/lib/format'
import { detectPlatform, isTauri, modKey } from '@/lib/platform'
import { useCollapsed } from '@/lib/collapse'
import { Logo } from './Logo'

interface NavItem {
  id: Screen
  label: string
  icon: typeof Gauge
  key: string
}

const NAV: NavItem[] = [
  { id: 'overview', label: 'Overview', icon: Gauge, key: '1' },
  { id: 'accounts', label: 'Accounts', icon: Users, key: '2' },
  { id: 'knowledge', label: 'Knowledge', icon: Library, key: '3' },
  { id: 'skills', label: 'Skills', icon: Blocks, key: '4' },
  { id: 'terminals', label: 'Terminals', icon: SquareTerminal, key: '5' },
  { id: 'notifications', label: 'Notifications', icon: Bell, key: '6' }
]

/** Pinned to the bottom of the sidebar, away from the screens you switch between all day. */
const SETTINGS: NavItem = { id: 'settings', label: 'Settings', icon: Settings, key: '7' }

/** On macOS the native window buttons float over the top of the sidebar. */
const MAC_TRAFFIC_LIGHTS = isTauri() && detectPlatform() === 'darwin'

export function Sidebar() {
  const screen = useStore((s) => s.screen)
  const go = useStore((s) => s.go)
  const unread = useStore((s) => s.notifications.filter((n) => !n.read).length)
  const usage = useStore((s) => s.usage)
  const now = useStore((s) => s.now)
  const refreshing = useStore((s) => s.refreshing)
  const refresh = useStore((s) => s.refresh)
  const [collapsed, setCollapsed] = useCollapsed('switchyard.sidebarCollapsed', () => window.innerWidth < 1000)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Terminals keep Ctrl+B for the program running in them.
      if ((e.target as HTMLElement).closest?.('.xterm')) return
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === 'b') {
        e.preventDefault()
        setCollapsed(!collapsed)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [collapsed, setCollapsed])
  const toggleLabel = `${collapsed ? 'Expand' : 'Collapse'} sidebar (${modKey(api.platform)}+B)`

  const newest = Object.values(usage)
    .filter((u) => !u.stale)
    .map((u) => Date.parse(u.fetchedAt))
    .sort((a, b) => b - a)[0]

  return (
    <nav
      aria-label="Main"
      className={clsx(
        'flex shrink-0 flex-col border-r border-rivet bg-deck transition-[width] duration-150',
        collapsed ? 'sidebar-compact w-[60px]' : 'w-[208px]'
      )}
    >
      {MAC_TRAFFIC_LIGHTS && <div data-tauri-drag-region className="h-7 shrink-0" />}
      <div data-tauri-drag-region="deep" className="flex h-14 items-center gap-2.5 px-4 compact:justify-center compact:px-0">
        <Logo size={26} />
        <span className="flex-1 font-serif text-[19px] font-semibold compact:hidden">Agent Hub</span>
      </div>
      <ul className="flex flex-col gap-0.5 px-2 pt-2">
        {NAV.map((n) => (
          <li key={n.id}>
            <NavButton item={n} on={screen === n.id} onClick={() => go(n.id)}>
              {n.id === 'notifications' && unread > 0 && (
                <span
                  className="min-w-5 rounded-full bg-hop px-1.5 text-center text-[11px] font-bold leading-5 text-hop-ink compact:absolute compact:right-1 compact:top-0.5 compact:min-w-4 compact:px-1 compact:leading-4"
                  aria-label={`${unread} unread`}
                >
                  {unread}
                </span>
              )}
            </NavButton>
          </li>
        ))}
      </ul>

      <div className="mt-auto flex flex-col gap-2 p-3 compact:items-center compact:px-1">
        <button
          onClick={() => refresh()}
          disabled={refreshing}
          className="flex items-center gap-2 rounded px-1 py-1 text-xs text-muted hover:text-ink compact:justify-center"
          data-tip="Refresh usage for every account"
          data-tip-side="right"
          aria-label={collapsed ? 'Refresh usage for every account' : undefined}
        >
          <RefreshCw size={13} className={clsx(refreshing && 'animate-spin')} />
          <span className="compact:hidden">
            {refreshing ? 'Refreshing…' : newest ? `Usage updated ${formatAgo(new Date(newest).toISOString(), now)}` : 'Refresh usage'}
          </span>
        </button>
        {IS_MOCK && (
          <p className="px-1 text-[11px] leading-4 text-faint compact:hidden">
            Browser preview with demo data. Nothing here touches your real accounts.
          </p>
        )}
        <div className="flex w-full items-center gap-1 compact:flex-col">
          <NavButton item={SETTINGS} on={screen === SETTINGS.id} onClick={() => go(SETTINGS.id)} />
          <button
            onClick={() => setCollapsed(!collapsed)}
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded text-muted hover:bg-raised hover:text-ink"
            data-tip={toggleLabel}
            data-tip-side="right"
            aria-label={toggleLabel}
            aria-expanded={!collapsed}
          >
            {collapsed ? <PanelLeftOpen size={17} /> : <PanelLeftClose size={17} />}
          </button>
        </div>
      </div>
    </nav>
  )
}

function NavButton({ item, on, onClick, children }: { item: NavItem; on: boolean; onClick: () => void; children?: React.ReactNode }) {
  const Icon = item.icon
  return (
    <button
      onClick={onClick}
      aria-current={on ? 'page' : undefined}
      data-tip={`${item.label} (${item.key})`}
      data-tip-side="right"
      className={clsx(
        'relative flex h-9 w-full items-center gap-2.5 rounded px-2.5 text-sm transition-colors compact:justify-center',
        on ? 'bg-raised font-semibold text-ink' : 'text-muted hover:bg-raised/60 hover:text-ink'
      )}
    >
      {on && <span className="absolute left-0 top-2 bottom-2 w-[3px] rounded-r bg-hop" aria-hidden />}
      <Icon size={17} strokeWidth={on ? 2.2 : 1.8} />
      <span className="flex-1 text-left compact:sr-only">{item.label}</span>
      {children}
    </button>
  )
}
