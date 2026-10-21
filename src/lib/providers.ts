import type { ProviderId, SharingOptions, TaskType } from '@shared/types'

export const PROVIDER_ORDER: ProviderId[] = ['claude', 'codex', 'gemini', 'copilot']

/** Short user-facing names. The logos live in ProviderMark. */
export const PROVIDER_META: Record<ProviderId, { short: string; product: string }> = {
  claude: { short: 'Claude', product: 'Claude Code' },
  codex: { short: 'Codex', product: 'Codex (ChatGPT)' },
  gemini: { short: 'Gemini', product: 'Gemini CLI' },
  copilot: { short: 'Copilot', product: 'GitHub Copilot CLI' }
}

export const TASKS: { id: TaskType; label: string; hint: string }[] = [
  { id: 'coding', label: 'Coding', hint: 'Everyday coding in a repo' },
  { id: 'long-refactor', label: 'Long refactor', hint: 'Hours of agent work: needs a big weekly budget' },
  { id: 'quick-question', label: 'Quick question', hint: 'A few messages: any account with a little left' },
  { id: 'research', label: 'Research', hint: 'Reading docs and the web, long context' },
  { id: 'review', label: 'Review', hint: 'Reviewing a diff or a pull request' }
]

export const SHARING_ITEMS: { key: keyof SharingOptions; label: string; explain: string }[] = [
  { key: 'skills', label: 'Skills', explain: 'Use the same skills as your main setup.' },
  { key: 'plugins', label: 'Plugins', explain: 'Installed plugins and marketplaces carry over.' },
  { key: 'agents', label: 'Agents and commands', explain: 'Custom subagents and slash commands are shared.' },
  {
    key: 'instructions',
    label: 'Instructions',
    explain: 'Your global CLAUDE.md, AGENTS.md or GEMINI.md applies here too.'
  },
  { key: 'mcp', label: 'MCP servers', explain: 'The same MCP server list. Server logins may still be per account.' },
  {
    key: 'settings',
    label: 'Settings',
    explain: 'Model, permissions and other preferences. Sign-in tokens are never shared.'
  },
  {
    key: 'history',
    label: 'Chat history',
    explain: 'Sessions are visible from both, so this account can resume a chat another account started.'
  }
]

/** Account identity colors: a validated categorical set (see docs/DESIGN.md). */
export const ACCOUNT_COLORS = ['#3987E5', '#D95926', '#199E70', '#C98500', '#D55181', '#9085E9', '#E66767', '#008300']
/** What screen readers call each of ACCOUNT_COLORS, in the same order. */
export const ACCOUNT_COLOR_NAMES = ['Blue', 'Orange', 'Green', 'Amber', 'Pink', 'Violet', 'Red', 'Dark green']

export function launchVerb(p: ProviderId): string {
  return `Launch ${PROVIDER_META[p].short}`
}
