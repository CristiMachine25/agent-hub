import { AppWindow, Play } from 'lucide-react'
import type { Account, ProviderInfo } from '@shared/types'
import { useStore } from '@/store'
import { primaryTarget, primaryVerb } from '@/lib/providers'

/**
 * Launch an account as Use now does: its CLI, or its desktop app when primaryTarget picks it.
 * `short` shows just "Launch"/"Open" (the account is named for screen readers).
 */
export function PrimaryLaunch(props: { account: Account; prov: ProviderInfo | undefined; className: string; short?: boolean; atLimit?: boolean }) {
  const { account, prov, className, short, atLimit } = props
  const target = primaryTarget(prov)
  const verb = primaryVerb(account.providerId, prov)
  const what = target === 'desktop' ? 'the app' : 'the CLI'
  return (
    <button
      className={className}
      onClick={() => useStore.getState().launch(account.id, target)}
      data-tip={atLimit ? `Limit reached: ${what} opens but will refuse new messages until the reset` : undefined}
      aria-label={short ? `${verb} as ${account.label}` : undefined}
    >
      {target === 'desktop' ? <AppWindow size={12} /> : <Play size={12} fill="currentColor" />} {short ? (target === 'desktop' ? 'Open' : 'Launch') : verb}
    </button>
  )
}
