// Realistic demo data for running the UI in a plain browser (npm run dev without Tauri).
import type {
  Account,
  AppNotification,
  AppSettings,
  DailyActivity,
  DetectedAccount,
  ProviderInfo,
  UsageHistoryPoint,
  UsageSnapshot,
  UsageWindow
} from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import type { Platform } from '@/lib/platform'

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

/** Deterministic PRNG so the demo looks the same on every reload. */
export function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export function homeRoot(p: Platform): { home: string; sep: string } {
  if (p === 'win32') return { home: 'C:\\Users\\sam', sep: '\\' }
  if (p === 'darwin') return { home: '/Users/sam', sep: '/' }
  return { home: '/home/sam', sep: '/' }
}

export function mockProviders(p: Platform): ProviderInfo[] {
  const { home } = homeRoot(p)
  const bin = p === 'win32' ? `${home}\\AppData\\Roaming\\npm` : p === 'darwin' ? '/opt/homebrew/bin' : `${home}/.local/bin`
  const ext = p === 'win32' ? '.cmd' : ''
  const s = p === 'win32' ? '\\' : '/'
  return [
    {
      id: 'claude',
      name: 'Claude Code',
      vendor: 'Anthropic',
      cliCommand: 'claude',
      cliInstalled: true,
      cliPath: `${bin}${s}claude${ext}`,
      cliVersion: '2.4.1',
      desktopName: 'Claude Desktop',
      desktopInstalled: true,
      desktopMultiAccount: true,
      homeEnvVar: 'CLAUDE_CONFIG_DIR',
      installHint: 'npm install -g @anthropic-ai/claude-code'
    },
    {
      id: 'codex',
      name: 'Codex',
      vendor: 'OpenAI',
      cliCommand: 'codex',
      cliInstalled: true,
      cliPath: `${bin}${s}codex${ext}`,
      cliVersion: '0.98.0',
      desktopName: 'ChatGPT',
      desktopInstalled: true,
      desktopMultiAccount: false,
      homeEnvVar: 'CODEX_HOME',
      installHint: 'npm install -g @openai/codex'
    },
    {
      id: 'gemini',
      name: 'Gemini CLI',
      vendor: 'Google',
      cliCommand: 'gemini',
      cliInstalled: true,
      cliPath: `${bin}${s}gemini${ext}`,
      cliVersion: '0.21.0',
      desktopInstalled: false,
      desktopMultiAccount: false,
      homeEnvVar: 'GEMINI_CLI_HOME',
      installHint: 'npm install -g @google/gemini-cli'
    },
    {
      id: 'copilot',
      name: 'GitHub Copilot CLI',
      vendor: 'GitHub',
      cliCommand: 'copilot',
      cliInstalled: false,
      desktopInstalled: false,
      desktopMultiAccount: false,
      homeEnvVar: 'COPILOT_HOME',
      installHint: 'npm install -g @github/copilot'
    }
  ]
}

interface Seed {
  account: Omit<Account, 'homeDir' | 'createdAt' | 'launchCount' | 'sharing' | 'isDefaultHome'> & {
    dir: string
    isDefaultHome?: boolean
    launchCount: number
    lastLaunchedAgo?: number
  }
  windows: (Omit<UsageWindow, 'resetsAt'> & { resetsIn?: number; resetsAtIso?: string })[]
  snap?: Partial<UsageSnapshot>
  /** Typical peak usage of the long window at reset time, for history. */
  typicalPeak: number
  /** Relative token volume. */
  intensity: number
  models: string[]
}

function nextFirstOfMonth(now: number): string {
  const d = new Date(now)
  return new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1)).toISOString()
}

function seeds(now: number): Seed[] {
  return [
    {
      account: {
        id: 'acc-work',
        providerId: 'claude',
        label: 'Work Max 20x',
        email: 'sam@northwind.dev',
        plan: 'Max 20x',
        dir: '.claude',
        isDefaultHome: true,
        color: '#3987E5',
        tags: ['coding', 'long-refactor'],
        status: 'ok',
        launchCount: 214,
        lastLaunchedAgo: 40 * MIN,
        monthlyCost: 200,
        pinned: true
      },
      windows: [
        { id: 'five_hour', label: '5-hour session', usedPercent: 64, windowMinutes: 300, resetsIn: 2 * HOUR + 14 * MIN },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 41, windowMinutes: 10080, resetsIn: 3 * DAY + 5 * HOUR },
        { id: 'seven_day_opus', label: 'Weekly Opus', usedPercent: 77, windowMinutes: 10080, resetsIn: 3 * DAY + 5 * HOUR }
      ],
      typicalPeak: 82,
      intensity: 1,
      models: ['claude-opus-4-6', 'claude-sonnet-4-6']
    },
    {
      account: {
        id: 'acc-personal',
        providerId: 'claude',
        label: 'Personal Pro',
        email: 'sam.k@fastmail.com',
        plan: 'Pro',
        dir: '.claude-personal',
        color: '#D95926',
        tags: ['quick-question'],
        status: 'ok',
        launchCount: 61,
        lastLaunchedAgo: 5 * HOUR,
        monthlyCost: 20
      },
      windows: [
        { id: 'five_hour', label: '5-hour session', usedPercent: 12, windowMinutes: 300, resetsIn: 4 * HOUR + 40 * MIN },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 96, windowMinutes: 10080, resetsIn: 1 * DAY + 7 * HOUR }
      ],
      typicalPeak: 97,
      intensity: 0.3,
      models: ['claude-sonnet-4-6']
    },
    {
      account: {
        id: 'acc-h2',
        providerId: 'claude',
        label: 'H2 Max',
        email: 'h2@northwind.dev',
        plan: 'Max 5x',
        dir: '.claude-h2',
        color: '#199E70',
        tags: ['research', 'review'],
        status: 'ok',
        launchCount: 38,
        lastLaunchedAgo: 3 * DAY,
        monthlyCost: 100
      },
      windows: [
        { id: 'five_hour', label: '5-hour session', usedPercent: 0, windowMinutes: 300 },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: 30, windowMinutes: 10080, resetsIn: 9 * HOUR + 20 * MIN },
        { id: 'seven_day_opus', label: 'Weekly Opus', usedPercent: 18, windowMinutes: 10080, resetsIn: 9 * HOUR + 20 * MIN }
      ],
      typicalPeak: 45,
      intensity: 0.45,
      models: ['claude-opus-4-6', 'claude-sonnet-4-6']
    },
    {
      account: {
        id: 'acc-gpt-pro',
        providerId: 'codex',
        label: 'ChatGPT Pro',
        email: 'sam@northwind.dev',
        plan: 'Pro',
        dir: '.codex',
        isDefaultHome: true,
        color: '#C98500',
        tags: ['review', 'coding'],
        status: 'ok',
        launchCount: 120,
        lastLaunchedAgo: 2 * HOUR,
        monthlyCost: 200
      },
      windows: [
        { id: 'primary', label: '5-hour session', usedPercent: 22, windowMinutes: 300, resetsIn: 3 * HOUR + 2 * MIN },
        { id: 'secondary', label: 'Weekly limit', usedPercent: 58, windowMinutes: 10080, resetsIn: 4 * DAY + 2 * HOUR }
      ],
      typicalPeak: 70,
      intensity: 0.75,
      models: ['gpt-5.2-codex', 'gpt-5.2']
    },
    {
      account: {
        id: 'acc-plus',
        providerId: 'codex',
        label: 'Plus side account',
        email: 'sam.side@gmail.com',
        plan: 'Plus',
        dir: '.codex-plus',
        color: '#D55181',
        tags: ['quick-question'],
        status: 'ok',
        launchCount: 44,
        lastLaunchedAgo: 70 * MIN,
        monthlyCost: 20
      },
      windows: [
        { id: 'primary', label: '5-hour session', usedPercent: 100, windowMinutes: 300, resetsIn: 1 * HOUR + 20 * MIN },
        { id: 'secondary', label: 'Weekly limit', usedPercent: 74, windowMinutes: 10080, resetsIn: 2 * DAY + 1 * HOUR }
      ],
      typicalPeak: 92,
      intensity: 0.25,
      models: ['gpt-5.2-codex']
    },
    {
      account: {
        id: 'acc-gemini',
        providerId: 'gemini',
        label: 'Gemini AI Pro',
        email: 'sam.k@gmail.com',
        plan: 'AI Pro',
        dir: '.gemini',
        isDefaultHome: true,
        color: '#9085E9',
        tags: ['research'],
        status: 'ok',
        launchCount: 17,
        lastLaunchedAgo: 2 * DAY,
        monthlyCost: 20
      },
      windows: [
        { id: 'gemini-2.5-pro', label: 'Gemini Pro daily', usedPercent: 35, windowMinutes: 1440, resetsIn: 11 * HOUR + 5 * MIN },
        { id: 'gemini-2.5-flash', label: 'Gemini Flash daily', usedPercent: 8, windowMinutes: 1440, resetsIn: 11 * HOUR + 5 * MIN }
      ],
      snap: {
        stale: true,
        error: "Couldn't reach Google's quota service (timed out). Showing values from 3 hours ago."
      },
      typicalPeak: 40,
      intensity: 0.2,
      models: ['gemini-2.5-pro', 'gemini-2.5-flash']
    },
    {
      account: {
        id: 'acc-copilot',
        providerId: 'copilot',
        label: 'Copilot Pro+',
        email: 'samk-gh',
        plan: 'Copilot Pro+',
        dir: '.copilot',
        isDefaultHome: true,
        color: '#E66767',
        tags: ['review'],
        status: 'ok',
        launchCount: 9,
        lastLaunchedAgo: 6 * DAY,
        monthlyCost: 39
      },
      windows: [
        { id: 'premium', label: 'Premium requests', usedPercent: (312 / 1500) * 100, windowMinutes: 43200, resetsAtIso: nextFirstOfMonth(now) }
      ],
      snap: { credits: { used: 312, limit: 1500, unit: 'premium requests' } },
      typicalPeak: 35,
      intensity: 0.15,
      models: ['claude-sonnet-4-6', 'gpt-5.2']
    },
    {
      account: {
        id: 'acc-client',
        providerId: 'claude',
        label: 'Client team seat',
        email: 'sam@acme-client.com',
        plan: 'Team',
        dir: '.claude-acme',
        color: '#008300',
        tags: [],
        status: 'needs-login',
        statusMessage: 'The sign-in for this account expired. Sign in again to keep tracking it.',
        launchCount: 3,
        lastLaunchedAgo: 12 * DAY,
        monthlyCost: 30
      },
      windows: [],
      typicalPeak: 50,
      intensity: 0.1,
      models: ['claude-sonnet-4-6']
    }
  ]
}

export interface MockDb {
  providers: ProviderInfo[]
  accounts: Account[]
  usage: Record<string, UsageSnapshot>
  history: UsageHistoryPoint[]
  activity: DailyActivity[]
  notifications: AppNotification[]
  settings: AppSettings
  detected: DetectedAccount[]
}

export function buildMockDb(p: Platform, now: number = Date.now()): MockDb {
  const { home, sep } = homeRoot(p)
  const list = seeds(now)
  const accounts: Account[] = []
  const usage: Record<string, UsageSnapshot> = {}
  const history: UsageHistoryPoint[] = []
  const activity: DailyActivity[] = []

  list.forEach((s, idx) => {
    const { dir, lastLaunchedAgo, isDefaultHome, ...rest } = s.account
    const a: Account = {
      ...rest,
      homeDir: `${home}${sep}${dir}`,
      isDefaultHome: !!isDefaultHome,
      sharedFrom: isDefaultHome ? undefined : `${home}${sep}.${s.account.providerId === 'codex' ? 'codex' : 'claude'}`,
      sharing: { ...DEFAULT_SHARING, history: s.account.providerId === 'claude' },
      createdAt: new Date(now - (60 - idx * 5) * DAY).toISOString(),
      lastLaunchedAt: lastLaunchedAgo ? new Date(now - lastLaunchedAgo).toISOString() : undefined
    }
    accounts.push(a)

    const windows: UsageWindow[] = s.windows.map(({ resetsIn, resetsAtIso, ...w }) => ({
      ...w,
      resetsAt: resetsAtIso ?? (resetsIn !== undefined ? new Date(now + resetsIn).toISOString() : undefined)
    }))
    if (a.status === 'ok') {
      usage[a.id] = {
        accountId: a.id,
        providerId: a.providerId,
        fetchedAt: new Date(now - (s.snap?.stale ? 3 * HOUR : 2 * MIN)).toISOString(),
        source: 'api',
        windows,
        plan: a.plan,
        email: a.email,
        ...s.snap
      }
    }

    const r = rng(1000 + idx * 77)
    if (windows.length) history.push(...historyFor(a.id, windows, s.typicalPeak, now, r))
    activity.push(...activityFor(a.id, s.intensity, s.models, now, r))
  })

  const ago = (ms: number) => new Date(now - ms).toISOString()
  const notifications: AppNotification[] = [
    {
      id: 'n1',
      kind: 'use-it-or-lose-it',
      accountId: 'acc-h2',
      title: 'H2 Max: 70% of the weekly limit resets unused in 9 hours',
      body: 'Start a long task on H2 Max now, or that quota is gone at the reset.',
      createdAt: ago(25 * MIN),
      read: false
    },
    {
      id: 'n2',
      kind: 'limit-reached',
      accountId: 'acc-plus',
      title: 'Plus side account hit its 5-hour limit',
      body: 'It comes back in about 1h 20m. ChatGPT Pro has 78% of its session left.',
      createdAt: ago(70 * MIN),
      read: false
    },
    {
      id: 'n3',
      kind: 'nearly-exhausted',
      accountId: 'acc-personal',
      title: 'Personal Pro has 4% of its weekly limit left',
      body: 'Switch to Work Max 20x for anything longer than a quick question.',
      createdAt: ago(3 * HOUR),
      read: false
    },
    {
      id: 'n4',
      kind: 'login-required',
      accountId: 'acc-client',
      title: 'Client team seat needs you to sign in again',
      body: 'The saved sign-in expired. Open the account and choose Sign in.',
      createdAt: ago(20 * HOUR),
      read: true
    },
    {
      id: 'n5',
      kind: 'limit-reset',
      accountId: 'acc-work',
      title: 'Work Max 20x: 5-hour session reset',
      body: 'The full session limit is available again.',
      createdAt: ago(26 * HOUR),
      read: true
    },
    {
      id: 'n6',
      kind: 'info',
      title: 'Found 2 more accounts on this computer',
      body: 'Open Accounts and choose Find accounts to import them.',
      createdAt: ago(3 * DAY),
      read: true
    }
  ]

  const settings: AppSettings = {
    pollIntervalMinutes: 5,
    notifications: {
      enabled: true,
      nearlyExhaustedPercent: 80,
      useItOrLoseItHours: 24,
      useItOrLoseItMinLeftPercent: 40,
      onReset: true
    },
    terminal: 'auto',
    defaultCwd: p === 'win32' ? `${home}\\code` : `${home}/code`,
    defaultSharing: { ...DEFAULT_SHARING },
    minimizeToTray: true,
    launchAtLogin: false,
    theme: 'dark',
    liveUsage: true
  }

  const detected: DetectedAccount[] = [
    { providerId: 'claude', homeDir: `${home}${sep}.claude`, isDefaultHome: true, email: 'sam@northwind.dev', plan: 'Max 20x', loggedIn: true, alreadyManaged: true },
    { providerId: 'claude', homeDir: `${home}${sep}.claude-h3`, isDefaultHome: false, email: 'h3@northwind.dev', plan: 'Max 5x', loggedIn: true, alreadyManaged: false, hint: `found via ${p === 'win32' ? '~\\.local\\bin\\claude-h3.cmd' : '~/.local/bin/claude-h3'}` },
    { providerId: 'codex', homeDir: `${home}${sep}.codex`, isDefaultHome: true, email: 'sam@northwind.dev', plan: 'Pro', loggedIn: true, alreadyManaged: true },
    { providerId: 'codex', homeDir: `${home}${sep}.codex-research`, isDefaultHome: false, loggedIn: false, alreadyManaged: false, hint: 'found via CODEX_HOME in your shell profile' },
    { providerId: 'gemini', homeDir: `${home}${sep}.gemini`, isDefaultHome: true, email: 'sam.k@gmail.com', loggedIn: true, alreadyManaged: true }
  ]

  return { providers: mockProviders(p), accounts, usage, history, activity, notifications, settings, detected }
}

/** Sawtooth history: usage climbs through each window period, more on weekdays and daytime, then resets. */
function historyFor(accountId: string, windows: UsageWindow[], typicalPeak: number, now: number, r: () => number): UsageHistoryPoint[] {
  const start = now - 30 * DAY
  const step = HOUR
  const pts: UsageHistoryPoint[] = []
  // Precompute a per-period peak for each window.
  const peaks: Record<string, Map<number, number>> = {}
  for (const w of windows) peaks[w.id] = new Map()
  for (let t = start - (start % step); t <= now; t += step) {
    const rec: Record<string, number> = {}
    for (const w of windows) {
      const P = (w.windowMinutes ?? 300) * MIN
      const R = w.resetsAt ? Date.parse(w.resetsAt) : now + P
      const k = Math.floor((R - t) / P) // 0 = current period
      const end = R - k * P
      const s = end - P
      let peak = peaks[w.id].get(k)
      if (peak === undefined) {
        const base = P <= 5 * HOUR ? 55 : typicalPeak
        peak = Math.max(4, Math.min(100, base + (r() - 0.5) * 40))
        if (P <= 5 * HOUR) {
          const hour = new Date(t).getHours()
          const dow = new Date(t).getDay()
          if (hour < 9 || hour > 22 || dow === 0 || dow === 6) peak *= 0.2
        }
        peaks[w.id].set(k, peak)
      }
      let v: number
      if (k <= 0) {
        const frac = Math.max(0, Math.min(1, (t - s) / Math.max(1, now - s)))
        v = w.usedPercent * Math.pow(frac, 1.1)
      } else {
        v = peak * Math.pow((t - s) / P, 0.9)
      }
      rec[w.id] = Math.round(Math.max(0, Math.min(100, v)) * 10) / 10
    }
    pts.push({ accountId, t: new Date(t).toISOString(), windows: rec })
  }
  return pts
}

function activityFor(accountId: string, intensity: number, models: string[], now: number, r: () => number): DailyActivity[] {
  const out: DailyActivity[] = []
  for (let d = 29; d >= 0; d--) {
    const day = new Date(now - d * DAY)
    const dow = day.getDay()
    const weekend = dow === 0 || dow === 6
    const busy = (weekend ? 0.25 : 1) * intensity * (0.4 + r() * 1.1)
    if (busy < 0.05) continue
    const output = Math.round(busy * 900_000)
    const m: Record<string, number> = {}
    models.forEach((name, i) => {
      m[name] = Math.round(output * (i === 0 ? 0.65 : 0.35 / Math.max(1, models.length - 1)))
    })
    if (models.length === 1) m[models[0]] = output
    const y = day.getFullYear()
    const mo = String(day.getMonth() + 1).padStart(2, '0')
    const da = String(day.getDate()).padStart(2, '0')
    out.push({
      accountId,
      date: `${y}-${mo}-${da}`,
      inputTokens: Math.round(output * 3.2),
      outputTokens: output,
      cacheTokens: Math.round(output * 28),
      messages: Math.round(busy * 260),
      sessions: Math.max(1, Math.round(busy * 9)),
      models: m
    })
  }
  return out
}
