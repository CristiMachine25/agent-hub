import clsx from 'clsx'
import { AlertTriangle, BellOff, Clock, Info, LogIn, RefreshCcw, Shuffle } from 'lucide-react'
import type { NotificationKind } from '@shared/types'
import { useStore } from '@/store'
import { formatAgo, formatClock } from '@/lib/format'
import { AccountDot, EmptyState } from '@/components/ui'
import { Page } from './Overview'

const ICON: Record<NotificationKind, typeof Info> = {
  'limit-reached': AlertTriangle,
  'nearly-exhausted': AlertTriangle,
  'limit-reset': RefreshCcw,
  'use-it-or-lose-it': Clock,
  'switch-suggested': Shuffle,
  'login-required': LogIn,
  info: Info
}
const TONE: Partial<Record<NotificationKind, string>> = {
  'limit-reached': 'text-empty',
  'nearly-exhausted': 'text-low',
  'use-it-or-lose-it': 'text-hop',
  'switch-suggested': 'text-hop'
}

export function Notifications() {
  const list = useStore((s) => s.notifications)
  const accounts = useStore((s) => s.accounts)
  const now = useStore((s) => s.now)
  const markRead = useStore((s) => s.markRead)
  const go = useStore((s) => s.go)
  const unread = list.filter((n) => !n.read).length

  const today = new Date(now).toDateString()
  const groups = [
    { title: 'Today', items: list.filter((n) => new Date(n.createdAt).toDateString() === today) },
    { title: 'Earlier', items: list.filter((n) => new Date(n.createdAt).toDateString() !== today) }
  ].filter((g) => g.items.length)

  return (
    <Page
      title="Notifications"
      actions={
        <button className="btn btn-sm" disabled={!unread} onClick={() => markRead('all')}>
          Mark all as read
        </button>
      }
    >
      {list.length === 0 ? (
        <EmptyState
          title="Nothing yet"
          body="You'll hear here when an account is nearly out, hits a limit, resets, or has quota about to go unused."
          action={
            <button className="btn" onClick={() => go('settings')}>
              <BellOff size={14} /> Choose what to be told about
            </button>
          }
        />
      ) : (
        groups.map((g) => (
          <section key={g.title}>
            <h2 className="mb-2 text-xs font-medium text-muted">{g.title}</h2>
            <ul className="divide-y divide-rivet rounded-lg border border-rivet">
              {g.items.map((n) => {
                const Icon = ICON[n.kind]
                const a = accounts.find((x) => x.id === n.accountId)
                return (
                  <li key={n.id}>
                    <button
                      className={clsx('flex w-full items-start gap-3 px-4 py-3 text-left hover:bg-deck', !n.read && 'bg-deck')}
                      onClick={() => {
                        if (!n.read) void markRead(n.id)
                        if (a) go('accounts', a.id)
                      }}
                    >
                      <Icon size={16} className={clsx('mt-0.5 shrink-0', TONE[n.kind] ?? 'text-muted')} />
                      <span className="min-w-0 flex-1">
                        <span className={clsx('block', !n.read && 'font-semibold')}>{n.title}</span>
                        <span className="block text-muted">{n.body}</span>
                        {a && (
                          <span className="mt-1 flex items-center gap-1.5 text-xs text-muted">
                            <AccountDot account={a} size={7} /> {a.label}
                          </span>
                        )}
                      </span>
                      <span className="flex shrink-0 items-center gap-2 text-xs text-muted" data-tip={new Date(n.createdAt).toLocaleString()}>
                        {g.title === 'Today' ? formatClock(n.createdAt) : formatAgo(n.createdAt, now)}
                        {!n.read && <span className="h-2 w-2 rounded-full bg-hop" aria-label="Unread" />}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </section>
        ))
      )}
    </Page>
  )
}
