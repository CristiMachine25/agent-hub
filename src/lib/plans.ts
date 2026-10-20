import type { ProviderId } from '@shared/types'

export interface PlanOption {
  /** Stored on the account and shown next to its name. */
  name: string
  /** US list price per month (per seat for team plans), billed monthly. */
  price?: number
  note: string
}

/**
 * Subscription plans that include each CLI, as listed on the providers' pricing pages in October 2026.
 * Prices change; the user can always type their own monthly cost.
 */
export const PLANS: Record<ProviderId, PlanOption[]> = {
  claude: [
    { name: 'Pro', price: 20, note: 'Claude Code included' },
    { name: 'Max 5x', price: 100, note: '5x the usage of Pro' },
    { name: 'Max 20x', price: 200, note: '20x the usage of Pro' },
    { name: 'Team Standard', price: 25, note: 'Per seat' },
    { name: 'Team Premium', price: 125, note: 'Per seat, 5x Standard' },
    { name: 'Enterprise', note: 'Seat plus usage at API rates' },
    { name: 'Free', price: 0, note: 'No Claude Code' }
  ],
  codex: [
    { name: 'Plus', price: 20, note: 'Codex included' },
    { name: 'Pro 100', price: 100, note: 'Higher Codex limits' },
    { name: 'Pro 200', price: 200, note: 'More than Pro 100' },
    { name: 'Pro 500', price: 500, note: 'Highest limits, fastest models' },
    { name: 'Go', price: 8, note: 'Light Codex use' },
    { name: 'Business', price: 25, note: 'Per seat' },
    { name: 'Enterprise', note: 'Contact sales' },
    { name: 'Free', price: 0, note: 'Limited Codex use' }
  ],
  gemini: [
    { name: 'Google AI Pro', price: 19.99, note: 'Higher Gemini CLI limits' },
    { name: 'Google AI Ultra', price: 99.99, note: '5x the limits of Pro' },
    { name: 'Google AI Ultra 20x', price: 199.99, note: '20x the limits of Pro' },
    { name: 'Google AI Plus', price: 4.99, note: 'Entry plan' },
    { name: 'Gemini Enterprise', price: 21, note: 'Per seat, from' },
    { name: 'Free', price: 0, note: 'Sign in with a Google account' }
  ],
  copilot: [
    { name: 'Copilot Pro', price: 10, note: 'Includes AI credits' },
    { name: 'Copilot Pro+', price: 39, note: 'About 5x the credits of Pro' },
    { name: 'Copilot Max', price: 100, note: 'Most credits' },
    { name: 'Copilot Business', price: 19, note: 'Per seat' },
    { name: 'Copilot Enterprise', price: 39, note: 'Per seat' },
    { name: 'Copilot Free', price: 0, note: 'Limited chat and agent use' }
  ]
}

/** The plan most people pick for each tool, preselected when creating a profile. */
export const DEFAULT_PLAN: Record<ProviderId, string> = {
  claude: 'Max 5x',
  codex: 'Plus',
  gemini: 'Google AI Pro',
  copilot: 'Copilot Pro'
}

export function findPlan(providerId: ProviderId, name: string | undefined): PlanOption | undefined {
  return name ? PLANS[providerId].find((p) => p.name.toLowerCase() === name.toLowerCase()) : undefined
}

export function planLabel(p: PlanOption): string {
  const price = p.price === undefined ? '' : p.price === 0 ? ', free' : `, $${p.price}/mo`
  return `${p.name}${price}`
}
