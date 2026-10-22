// Shared contract between the Rust backend (src-tauri/src/model.rs mirrors it) and the UI.
// Every module codes against these types. Change them only with care: both sides depend on them.

export type ProviderId = 'claude' | 'codex' | 'gemini' | 'copilot'

/** What we know about a provider's tooling on this machine. */
export interface ProviderInfo {
  id: ProviderId
  name: string // "Claude Code"
  vendor: string // "Anthropic"
  /** Executable name of the CLI, e.g. "claude". */
  cliCommand: string
  cliInstalled: boolean
  cliPath?: string
  cliVersion?: string
  /** Desktop app (Claude Desktop, ChatGPT/Codex app, ...) if one exists for this provider. */
  desktopName?: string
  desktopInstalled: boolean
  desktopPath?: string
  /** Desktop app can run several accounts at once (separate user-data dirs). */
  desktopMultiAccount: boolean
  /** Env var that points the CLI at an isolated config home, e.g. CLAUDE_CONFIG_DIR. */
  homeEnvVar?: string
  /** Install hint shown in the UI when the CLI is missing. */
  installHint: string
}

/** Which parts of a profile's home are linked to a shared source instead of being private. */
export interface SharingOptions {
  skills: boolean
  plugins: boolean
  agents: boolean // also covers slash commands
  instructions: boolean // CLAUDE.md / AGENTS.md / GEMINI.md
  settings: boolean // settings.json / config.toml (never credentials)
  mcp: boolean // MCP server config
  history: boolean // chats / sessions / projects, so one account can resume another's session
}

export const DEFAULT_SHARING: SharingOptions = {
  skills: true,
  plugins: true,
  agents: true,
  instructions: true,
  settings: false,
  mcp: true,
  history: true
}

export type AccountStatus = 'ok' | 'needs-login' | 'expired' | 'error' | 'unknown'

/** One AI subscription/login, bound to an isolated config home. */
export interface Account {
  id: string
  providerId: ProviderId
  label: string // user-facing name, e.g. "Work Max"
  email?: string
  plan?: string // "Max 20x", "Pro", "Plus", "Business", "Copilot Pro+"...
  /** Absolute path of the isolated config home for this account (value of homeEnvVar). */
  homeDir: string
  /** true when homeDir is the provider's stock location (~/.claude, ~/.codex ...). */
  isDefaultHome: boolean
  /** Path of the home whose skills/plugins/history are shared into this one. */
  sharedFrom?: string
  sharing: SharingOptions
  color: string // hex, for charts and badges
  tags: string[] // free-form, e.g. "coding", "research"; feeds recommendations
  notes?: string
  status: AccountStatus
  statusMessage?: string
  createdAt: string // ISO
  lastLaunchedAt?: string
  launchCount: number
  /** Manual monthly price, for "value left" maths. */
  monthlyCost?: number
  pinned?: boolean
}

/** Fields to change on an account. JSON drops `undefined`, so a cleared field is sent as `null`. */
export type AccountPatch = { [K in keyof Account]?: Account[K] | null }

export interface NewAccountInput {
  providerId: ProviderId
  label: string
  /** Use an existing directory (import) instead of creating a new one under the app's profiles dir. */
  homeDir?: string
  sharing?: SharingOptions
  sharedFrom?: string
  color?: string
  tags?: string[]
  monthlyCost?: number
  /** Plan picked when creating the profile; the live value from the provider wins once signed in. */
  plan?: string
}

// ---------- moving the setup to another computer ----------

export interface ExportOptions {
  accountIds: string[]
  /** Skills, agents and instructions from each tool's main folder. */
  includeFiles: boolean
  includeSettings: boolean
  /** Include sign-ins, encrypted with this passphrase. */
  passphrase?: string
}

export interface ExportSummary {
  accounts: number
  signIns: number
  files: number
  skippedFiles: number
}

export interface ImportPreview {
  fromOs: string
  exportedAt: string
  accounts: {
    key: string
    providerId: ProviderId
    label: string
    email?: string
    plan?: string
    hasSignIn: boolean
    /** Label of the matching account already here; such accounts are left alone. */
    alreadyHere?: string
    targetHome: string
  }[]
  kinds: { providerId: ProviderId; kind: 'skills' | 'agents' | 'instructions'; files: number; newFiles: number }[]
  hasSettings: boolean
  hasSignIns: boolean
  signInsUnlocked: boolean
}

export interface ImportChoice {
  accountKeys: string[]
  signIns: boolean
  files: boolean
  settings: boolean
}

export interface ImportReport {
  added: string[]
  skipped: string[]
  signIns: number
  filesAdded: number
  filesKept: number
  settings: boolean
  problems: string[]
}

/** An account found on disk that the user may import. */
export interface DetectedAccount {
  providerId: ProviderId
  homeDir: string
  isDefaultHome: boolean
  email?: string
  plan?: string
  loggedIn: boolean
  alreadyManaged: boolean
  /** e.g. "found via ~/.local/bin/claude-h2.cmd" */
  hint?: string
}

/** One rate-limit window, e.g. Claude "5-hour session" or "Weekly". */
export interface UsageWindow {
  id: string // stable key: "five_hour", "seven_day", "seven_day_opus", "primary", "secondary", "premium", "gemini-2.5-pro"...
  label: string // "5-hour session"
  usedPercent: number // 0..100
  resetsAt?: string // ISO
  windowMinutes?: number
}

export interface UsageSnapshot {
  accountId: string
  providerId: ProviderId
  fetchedAt: string // ISO
  source: 'api' | 'cli' | 'local-logs' | 'manual' | 'none'
  windows: UsageWindow[]
  plan?: string
  email?: string
  /** Pay-as-you-go / extra usage credits, when the provider reports them. */
  credits?: { used: number; limit?: number; unit: string }
  /** Set when the fetch failed; windows may then hold the last good values. */
  error?: string
  stale?: boolean
}

/** Point stored every poll, for charts. */
export interface UsageHistoryPoint {
  accountId: string
  t: string // ISO
  windows: Record<string, number> // window id -> usedPercent
}

/** Token activity per day, parsed from local session logs. */
export interface DailyActivity {
  accountId: string
  date: string // YYYY-MM-DD local
  inputTokens: number
  outputTokens: number
  cacheTokens: number
  messages: number
  sessions: number
  models: Record<string, number> // model -> output tokens
}

export type LaunchTarget = 'cli' | 'desktop' | 'login'

export interface LaunchRequest {
  accountId: string
  target: LaunchTarget
  cwd?: string
  /** Continue the most recent session (claude --continue, codex resume --last ...). */
  resume?: boolean
  extraArgs?: string[]
}

export interface LaunchResult {
  ok: boolean
  error?: string
  /** The command that was run, shown to the user for transparency. */
  commandLine?: string
  /** Something to tell the user after a successful launch. */
  note?: string
}

export type NotificationKind =
  | 'limit-reached'
  | 'limit-reset'
  | 'nearly-exhausted'
  | 'use-it-or-lose-it'
  | 'switch-suggested'
  | 'login-required'
  | 'info'

export interface AppNotification {
  id: string
  kind: NotificationKind
  accountId?: string
  title: string
  body: string
  createdAt: string
  read: boolean
}

export type TaskType = 'coding' | 'long-refactor' | 'quick-question' | 'research' | 'review' | 'any'

export interface Recommendation {
  accountId: string
  score: number // 0..100, higher = better pick now
  reason: string // plain-language, e.g. "62% of weekly left, resets in 2 days - use it before it resets"
}

export interface AppSettings {
  pollIntervalMinutes: number // default 5
  notifications: {
    enabled: boolean
    nearlyExhaustedPercent: number // default 80
    useItOrLoseItHours: number // warn when a window resets within N hours with lots left; default 24
    useItOrLoseItMinLeftPercent: number // default 40
    onReset: boolean
  }
  /** "auto" picks Windows Terminal / Terminal.app / the default Linux terminal. */
  terminal: string
  defaultCwd?: string
  defaultSharing: SharingOptions
  /** Where new isolated homes are created. Default: <userData>/profiles */
  profilesDir?: string
  minimizeToTray: boolean
  launchAtLogin: boolean
  theme: 'system' | 'dark' | 'light'
  /** Opt-in: call provider usage APIs with the account's own token. Off = local logs only. */
  liveUsage: boolean
  /** Passed as CLI flags on every launch, for every account of that tool. */
  permissionModes: PermissionModes
}

/** How much each CLI may do without asking. "default" adds no flag. */
export interface PermissionModes {
  claude: 'default' | 'acceptEdits' | 'plan' | 'bypassPermissions'
  codex: 'default' | 'auto' | 'bypass'
  gemini: 'default' | 'autoEdit' | 'yolo'
  copilot: 'default' | 'allowAll'
}

export const DEFAULT_PERMISSION_MODES: PermissionModes = {
  claude: 'default',
  codex: 'default',
  gemini: 'default',
  copilot: 'default'
}

export type KnowledgeKind = 'skills' | 'plugins' | 'agents' | 'commands' | 'instructions' | 'settings'

/** One thing in a tool's shared profile: a skill folder, an agent file, CLAUDE.md, settings.json... */
export interface KnowledgeItem {
  kind: KnowledgeKind
  name: string
  path: string
}

/** An account of the tool and which parts of the shared profile it links in. */
export interface KnowledgeAccount {
  accountId: string
  label: string
  /** Runs from the tool's main folder (~/.claude ...) instead of the shared profile. */
  isDefaultHome: boolean
  /** 'skills' | 'plugins' | 'agents' | 'instructions' | 'settings' | 'mcp' */
  kinds: string[]
}

/** The shared profile Agent Hub owns for one tool: what is in it and who uses it. */
export interface ProviderKnowledge {
  providerId: ProviderId
  /** The shared folder every account links to. */
  folder: string
  /** The tool's own folder, e.g. ~/.claude, that the shared folder was seeded from. */
  mainFolder: string
  items: KnowledgeItem[]
  accounts: KnowledgeAccount[]
}

/** A skill in the public skills.sh directory. */
export interface DirectorySkill {
  /** "owner/repo/skill" */
  id: string
  name: string
  /** The GitHub repository, "owner/repo". */
  source: string
  installs: number
}

/** A skill's SKILL.md, read before installing. */
export interface SkillPreview {
  id: string
  name: string
  description: string
  /** SKILL.md without its front matter. */
  body: string
  /** Files the skill brings, SKILL.md included. */
  files: number
}

/** A skill folder in a tool's shared skills folder. */
export interface InstalledSkill {
  providerId: ProviderId
  /** The folder name. */
  name: string
  /** The directory id when Agent Hub installed it; null for skills copied from the main folder or added by hand. */
  id: string | null
}

export interface SkillInstallOutcome {
  providerId: ProviderId
  error: string | null
}

/** Start a CLI inside an embedded terminal pane (the in-app grid) instead of an external window. */
export interface TerminalSpawnRequest {
  accountId: string
  target: 'cli' | 'login'
  resume?: boolean
  /** With `resume`: reopen this conversation (from `terminals.session`) instead of the account's newest one. */
  resumeSession?: string
  cwd?: string
  extraArgs?: string[]
  cols: number
  rows: number
}

export interface TerminalInfo {
  id: string
  accountId: string
  title: string // e.g. "Claude H2"
  commandLine: string
  cwd: string
  pid?: number
  startedAt: string
  exited: boolean
  exitCode?: number | null
}

export interface AppEvents {
  'usage-updated': Record<string, UsageSnapshot>
  'accounts-changed': Account[]
  notification: AppNotification
  /** UTF-8 text from a terminal, already split on character boundaries. Batched (~16 ms). */
  'terminal-output': { id: string; data: string }
  'terminal-exit': { id: string; code: number | null }
  /** Ask the UI to show a screen, e.g. "terminals" from the tray menu. */
  navigate: string
}

/** API the UI uses (src/api.ts implements it over Tauri invoke; mock mode in a plain browser). */
export interface AgentHubApi {
  platform: 'win32' | 'darwin' | 'linux'
  providers: {
    list(): Promise<ProviderInfo[]>
  }
  accounts: {
    list(): Promise<Account[]>
    detect(): Promise<DetectedAccount[]>
    import(found: DetectedAccount[]): Promise<Account[]>
    create(input: NewAccountInput): Promise<Account>
    /** `null` clears a field; a missing (or undefined) key leaves it as it is. */
    update(id: string, patch: AccountPatch): Promise<Account>
    /** Resolves true when the account's folder was deleted too (only folders inside profiles are). */
    remove(id: string, opts: { deleteHome: boolean }): Promise<boolean>
  }
  usage: {
    latest(): Promise<Record<string, UsageSnapshot>>
    history(accountId: string | null, days: number): Promise<UsageHistoryPoint[]>
    activity(accountId: string | null, days: number): Promise<DailyActivity[]>
    refresh(accountId?: string): Promise<Record<string, UsageSnapshot>>
    setManual(accountId: string, windows: UsageWindow[]): Promise<UsageSnapshot>
  }
  launch(req: LaunchRequest): Promise<LaunchResult>
  terminals: {
    spawn(req: TerminalSpawnRequest): Promise<TerminalInfo>
    write(id: string, data: string): Promise<void>
    resize(id: string, cols: number, rows: number): Promise<void>
    kill(id: string): Promise<void>
    list(): Promise<TerminalInfo[]>
    /** Recent output kept by the backend (max ~256 KB per terminal), to restore a pane after a UI reload. */
    scrollback(id: string): Promise<string>
    /** The conversation the CLI in this terminal is in now, when the tool can say. */
    session(id: string): Promise<string | null>
  }
  recommend(task: TaskType, providerId?: ProviderId): Promise<Recommendation[]>
  notifications: {
    list(): Promise<AppNotification[]>
    markRead(id: string | 'all'): Promise<void>
  }
  settings: {
    get(): Promise<AppSettings>
    set(patch: Partial<AppSettings>): Promise<AppSettings>
  }
  knowledge: {
    get(): Promise<ProviderKnowledge[]>
    /** Copy anything new from the tool's main folder into its shared folder. Never overwrites. */
    reseed(providerId: ProviderId): Promise<{ added: number; kept: number }>
  }
  skills: {
    /** Search skills.sh. Fewer than two characters finds nothing. */
    search(query: string): Promise<DirectorySkill[]>
    preview(id: string): Promise<SkillPreview>
    /** Skill folders in the shared folder of every tool that has accounts. */
    installed(): Promise<InstalledSkill[]>
    /** Download once, then add to each tool's shared folder. Never overwrites. */
    install(id: string, providerIds: ProviderId[]): Promise<SkillInstallOutcome[]>
    /** Only removes a copy Agent Hub installed. */
    remove(id: string, providerId: ProviderId): Promise<void>
  }
  system: {
    openPath(path: string): Promise<void>
    /** Open a skills.sh or GitHub page in the browser. */
    openUrl(url: string): Promise<void>
    pickDirectory(): Promise<string | null>
    /** Ask where to save a file; null when cancelled. */
    pickSavePath(defaultName: string): Promise<string | null>
    /** Ask for a setup file to open; null when cancelled. */
    pickSetupFile(): Promise<string | null>
    /** The app data folder (Tauri `appDataDir`); profiles go in its "profiles" folder unless settings.profilesDir is set. */
    appDataDir(): Promise<string>
  }
  transfer: {
    export(path: string, opts: ExportOptions): Promise<ExportSummary>
    preview(path: string, passphrase?: string): Promise<ImportPreview>
    import(path: string, passphrase: string | undefined, choice: ImportChoice): Promise<ImportReport>
  }
  on<K extends keyof AppEvents>(event: K, cb: (payload: AppEvents[K]) => void): () => void
}

/**
 * Tauri command names (Rust #[tauri::command] fns, snake_case). Arguments are passed as an object
 * with camelCase keys matching the AgentHubApi parameter names, e.g. invoke('accounts_update', { id, patch }).
 * Events are emitted from Rust with app.emit(<AppEvents key>, payload).
 */
export const COMMANDS = {
  providersList: 'providers_list',
  accountsList: 'accounts_list',
  accountsDetect: 'accounts_detect',
  accountsImport: 'accounts_import', // { found }
  accountsCreate: 'accounts_create', // { input }
  accountsUpdate: 'accounts_update', // { id, patch }
  accountsRemove: 'accounts_remove', // { id, deleteHome }
  usageLatest: 'usage_latest',
  usageHistory: 'usage_history', // { accountId, days }
  usageActivity: 'usage_activity', // { accountId, days }
  usageRefresh: 'usage_refresh', // { accountId? }
  usageSetManual: 'usage_set_manual', // { accountId, windows }
  launch: 'launch', // { req }
  recommend: 'recommend', // { task, providerId? }
  notificationsList: 'notifications_list',
  notificationsMarkRead: 'notifications_mark_read', // { id }
  settingsGet: 'settings_get',
  settingsSet: 'settings_set', // { patch }
  knowledgeGet: 'knowledge_get',
  knowledgeReseed: 'knowledge_reseed', // { providerId }
  skillsSearch: 'skills_search', // { query }
  skillsPreview: 'skills_preview', // { id }
  skillsInstalled: 'skills_installed',
  skillsInstall: 'skills_install', // { id, providerIds }
  skillsRemove: 'skills_remove', // { id, providerId }
  systemOpenPath: 'system_open_path', // { path }
  systemOpenUrl: 'system_open_url', // { url }
  terminalSpawn: 'terminal_spawn', // { req }
  terminalWrite: 'terminal_write', // { id, data }
  terminalResize: 'terminal_resize', // { id, cols, rows }
  terminalKill: 'terminal_kill', // { id }
  terminalList: 'terminal_list',
  terminalScrollback: 'terminal_scrollback', // { id }
  terminalSession: 'terminal_session', // { id }
  transferExport: 'transfer_export', // { path, opts }
  transferPreview: 'transfer_preview', // { path, passphrase? }
  transferImport: 'transfer_import' // { path, passphrase?, choice }
  // pickDirectory is done in the UI with @tauri-apps/plugin-dialog
} as const
