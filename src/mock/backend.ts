// In-memory AgentHubApi used when the UI runs in a plain browser. Mutations persist for the page's lifetime.
import type {
  Account,
  AppEvents,
  AppSettings,
  DetectedAccount,
  DirectorySkill,
  AgentHubApi,
  InstalledSkill,
  KnowledgeItem,
  LaunchRequest,
  LaunchResult,
  NewAccountInput,
  ProviderId,
  ProviderKnowledge,
  UsageSnapshot,
  UsageWindow
} from '@shared/types'
import { DEFAULT_SHARING } from '@shared/types'
import { recommend as rankAccounts } from '@/lib/recommend'
import { ACCOUNT_COLORS, PROVIDER_META, PROVIDER_ORDER } from '@/lib/providers'
import { FEATURED, featuredFor, folderOf } from '@/lib/skills'
import type { Platform } from '@/lib/platform'
import { buildMockDb, homeRoot } from './data'
import { createMockTerminals } from './terminals'

type Listener = (payload: never) => void

const PROTECTED = ['id', 'providerId', 'createdAt', 'isDefaultHome', 'launchCount', 'lastLaunchedAt']

/** Same rule as import_label in commands.rs: "Claude main" for the stock home, "Claude H2" for ~/.claude-h2. */
function importLabel(f: DetectedAccount): string {
  const short = PROVIDER_META[f.providerId].short
  if (f.isDefaultHome) return `${short} main`
  const dir = f.homeDir.split(/[\\/]/).pop() ?? ''
  const suffix = dir.replace(/^\.+/, '').replace(new RegExp(`^${f.providerId}`), '').replace(/^[-_.]+|[-_.]+$/g, '')
  if (!suffix) return f.email?.trim() || short
  return `${short} ${suffix.split(/[-_.]/).filter(Boolean).map((w) => w[0]!.toUpperCase() + w.slice(1)).join(' ')}`
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
const clone = <T,>(x: T): T => structuredClone(x)

export function createMockApi(platform: Platform): AgentHubApi {
  const db = buildMockDb(platform)
  const listeners = new Map<keyof AppEvents, Set<Listener>>()
  const { home, sep } = homeRoot(platform)

  function emit<K extends keyof AppEvents>(event: K, payload: AppEvents[K]) {
    listeners.get(event)?.forEach((cb) => (cb as (p: AppEvents[K]) => void)(clone(payload)))
  }

  function accountsChanged() {
    emit('accounts-changed', db.accounts)
  }

  function find(id: string): Account {
    const a = db.accounts.find((x) => x.id === id)
    if (!a) throw new Error(`No account with id ${id}. It may have been removed; reload the list.`)
    return a
  }

  function slug(s: string) {
    return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'account'
  }

  function nudgeUsage() {
    const now = Date.now()
    for (const snap of Object.values(db.usage)) {
      if (snap.stale) continue
      snap.fetchedAt = new Date(now).toISOString()
      for (const w of snap.windows) {
        if (w.usedPercent >= 100) continue
        if (w.resetsAt && Date.parse(w.resetsAt) <= now) {
          w.usedPercent = 0
          w.resetsAt = new Date(now + (w.windowMinutes ?? 300) * 60_000).toISOString()
          continue
        }
        const bump = Math.random() < 0.5 ? Math.random() * 3 : 0
        w.usedPercent = Math.min(100, Math.round((w.usedPercent + bump) * 10) / 10)
      }
    }
  }

  let pollTimer: ReturnType<typeof setInterval> | undefined
  function schedulePoll() {
    if (pollTimer) clearInterval(pollTimer)
    pollTimer = setInterval(() => {
      nudgeUsage()
      emit('usage-updated', db.usage)
    }, Math.max(1, db.settings.pollIntervalMinutes) * 60_000)
  }
  schedulePoll()

  function quote(s: string) {
    return /\s/.test(s) ? `"${s}"` : s
  }

  function buildCommand(a: Account, req: LaunchRequest): string {
    const prov = db.providers.find((p) => p.id === a.providerId)!
    const cwd = req.cwd ?? db.settings.defaultCwd ?? home
    let cli = prov.cliCommand
    if (req.target === 'login') {
      cli = a.providerId === 'codex' ? 'codex login' : a.providerId === 'claude' ? 'claude /login' : a.providerId === 'copilot' ? 'copilot /login' : 'gemini'
    } else if (req.resume) {
      cli += a.providerId === 'codex' ? ' resume --last' : a.providerId === 'gemini' ? ' --resume latest' : ' --continue'
    }
    if (req.extraArgs?.length) cli += ' ' + req.extraArgs.map(quote).join(' ')
    const envVar = prov.homeEnvVar
    const term = db.settings.terminal

    if (req.target === 'desktop') {
      const app = platform === 'win32'
        ? `${home}\\AppData\\Local\\AnthropicClaude\\claude.exe`
        : platform === 'darwin' ? '/Applications/Claude.app' : '/usr/bin/claude-desktop'
      const dataDir = `${a.homeDir}${sep}desktop`
      return platform === 'darwin'
        ? `open -n -a ${quote(app)} --args --user-data-dir=${quote(dataDir)}`
        : `${quote(app)} --user-data-dir=${quote(dataDir)}`
    }

    if (platform === 'win32') {
      const env = a.isDefaultHome || !envVar ? '' : `set ${envVar}=${a.homeDir}&& `
      if (term === 'cmd') return `cmd.exe /c start "${a.label}" /d ${quote(cwd)} cmd /k "${env}${cli}"`
      if (term === 'powershell') return `powershell.exe -NoExit -Command "${a.isDefaultHome || !envVar ? '' : `$env:${envVar}='${a.homeDir}'; `}${cli}"`
      return `wt.exe -w 0 new-tab --title "${a.label}" -d ${quote(cwd)} cmd /k "${env}${cli}"`
    }
    const env = a.isDefaultHome || !envVar ? '' : `${envVar}=${quote(a.homeDir)} `
    const script = `cd ${quote(cwd)} && ${env}${cli}`
    if (platform === 'darwin') {
      if (term === 'iterm') return `osascript -e 'tell application "iTerm" to create window with default profile command "${script}"'`
      return `osascript -e 'tell application "Terminal" to do script "${script}"'`
    }
    const t = term === 'auto' ? 'gnome-terminal' : term
    if (t === 'kitty' || t === 'alacritty') return `${t} -e bash -lc '${script}; exec bash'`
    if (t === 'konsole') return `konsole -e bash -lc '${script}; exec bash'`
    return `${t} -- bash -lc '${script}; exec bash'`
  }

  // Shared profiles live under the app data folder; the main folders are the tools' stock homes.
  const appData =
    platform === 'win32'
      ? `${home}\\AppData\\Roaming\\dev.switchyard.app`
      : platform === 'darwin'
        ? `${home}/Library/Application Support/dev.switchyard.app`
        : `${home}/.local/share/dev.switchyard.app`
  const knowledge = mockKnowledge()
  const sharedKinds = ['skills', 'plugins', 'agents', 'instructions', 'settings', 'mcp'] as const

  function knowledgeFor(providerId: ProviderId): ProviderKnowledge {
    const folder = `${appData}${sep}shared${sep}${providerId}`
    const item = ([kind, name, sub]: MockItem): KnowledgeItem => ({ kind, name, path: sub ? `${folder}${sep}${sub}${sep}${name}` : `${folder}${sep}${name}` })
    return {
      providerId,
      folder,
      mainFolder: `${home}${sep}.${providerId}`,
      items: knowledge[providerId].items.map(item),
      accounts: db.accounts
        .filter((a) => a.providerId === providerId)
        .map((a) => ({
          accountId: a.id,
          label: a.label,
          isDefaultHome: a.isDefaultHome,
          kinds: a.isDefaultHome ? [] : sharedKinds.filter((k) => a.sharing[k])
        }))
    }
  }

  // The skills directory: the featured skills and a few more. Installs land in the shared folders above.
  const directory = mockDirectory()
  const installedFrom = new Map<string, string>() // "<provider>/<folder>" -> directory id

  const api: AgentHubApi = {
    platform,
    providers: {
      async list() {
        await wait(80)
        return clone(db.providers)
      }
    },
    accounts: {
      async list() {
        await wait(60)
        return clone(db.accounts)
      },
      async detect() {
        await wait(900)
        const managed = new Set(db.accounts.map((a) => a.homeDir.toLowerCase()))
        return clone(db.detected.map((d) => ({ ...d, alreadyManaged: managed.has(d.homeDir.toLowerCase()) })))
      },
      async import(found) {
        await wait(400)
        const created: Account[] = []
        for (const f of found) {
          const n = db.accounts.length
          const a: Account = {
            id: `acc-${slug(f.homeDir)}-${n}`,
            providerId: f.providerId,
            label: importLabel(f),
            email: f.email,
            plan: f.plan,
            homeDir: f.homeDir,
            isDefaultHome: f.isDefaultHome,
            // Like accounts_import: nothing in an existing home is linked until the user turns sharing on.
            sharedFrom: f.isDefaultHome ? undefined : `${home}${sep}.${f.providerId}`,
            sharing: { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false },
            color: ACCOUNT_COLORS[n % ACCOUNT_COLORS.length]!,
            tags: [],
            notes: f.hint,
            status: f.loggedIn ? 'ok' : 'needs-login',
            statusMessage: f.loggedIn ? undefined : 'Not signed in yet. Choose Sign in to finish setting up this account.',
            createdAt: new Date().toISOString(),
            launchCount: 0
          }
          db.accounts.push(a)
          created.push(a)
          if (f.loggedIn) {
            db.usage[a.id] = {
              accountId: a.id,
              providerId: a.providerId,
              fetchedAt: new Date().toISOString(),
              source: 'api',
              windows: defaultWindows(a.providerId, 10, 25)
            }
          }
        }
        accountsChanged()
        emit('usage-updated', db.usage)
        return clone(created)
      },
      async create(input: NewAccountInput) {
        await wait(500)
        const n = db.accounts.length
        const homeDir = input.homeDir ?? `${appData}${sep}profiles${sep}${input.providerId}${sep}${slug(input.label)}`
        if (db.accounts.some((a) => a.homeDir.toLowerCase() === homeDir.toLowerCase())) {
          throw new Error(`Another account already uses ${homeDir}. Pick a different folder or label.`)
        }
        const a: Account = {
          id: `acc-${slug(input.label)}-${n}`,
          providerId: input.providerId,
          label: input.label,
          homeDir,
          isDefaultHome: false,
          sharedFrom: input.sharedFrom,
          sharing: input.sharing ?? { ...DEFAULT_SHARING },
          color: input.color ?? ACCOUNT_COLORS[n % ACCOUNT_COLORS.length]!,
          tags: input.tags ?? [],
          status: 'needs-login',
          statusMessage: 'Not signed in yet. Choose Sign in to finish setting up this account.',
          createdAt: new Date().toISOString(),
          launchCount: 0,
          monthlyCost: input.monthlyCost,
          plan: input.plan
        }
        db.accounts.push(a)
        accountsChanged()
        return clone(a)
      },
      async update(id, patch) {
        await wait(120)
        const a = find(id)
        const fields = a as unknown as Record<string, unknown>
        // Like accounts_update: protected fields are ignored, undefined is dropped (as JSON does), null clears.
        for (const [k, v] of Object.entries(patch)) {
          if (v === undefined || PROTECTED.includes(k)) continue
          if (v === null) delete fields[k]
          else fields[k] = v
        }
        accountsChanged()
        return clone(a)
      },
      async remove(id, opts) {
        await wait(300)
        find(id)
        db.accounts = db.accounts.filter((a) => a.id !== id)
        delete db.usage[id]
        accountsChanged()
        emit('usage-updated', db.usage)
        return opts.deleteHome
      }
    },
    usage: {
      async latest() {
        await wait(80)
        return clone(db.usage)
      },
      async history(accountId, days) {
        await wait(120)
        const since = Date.now() - days * 86_400_000
        return clone(db.history.filter((p) => (accountId === null || p.accountId === accountId) && Date.parse(p.t) >= since))
      },
      async activity(accountId, days) {
        await wait(120)
        const since = Date.now() - days * 86_400_000
        return clone(
          db.activity.filter((d) => (accountId === null || d.accountId === accountId) && Date.parse(d.date + 'T23:59:59') >= since)
        )
      },
      async refresh(accountId) {
        await wait(700)
        nudgeUsage()
        if (accountId && db.usage[accountId]?.stale) {
          // Simulate the provider coming back.
          db.usage[accountId].stale = false
          db.usage[accountId].error = undefined
          db.usage[accountId].fetchedAt = new Date().toISOString()
        }
        emit('usage-updated', db.usage)
        return clone(db.usage)
      },
      async setManual(accountId, windows: UsageWindow[]) {
        await wait(150)
        const a = find(accountId)
        const snap: UsageSnapshot = {
          accountId,
          providerId: a.providerId,
          fetchedAt: new Date().toISOString(),
          source: 'manual',
          windows
        }
        db.usage[accountId] = snap
        emit('usage-updated', db.usage)
        return clone(snap)
      }
    },
    async launch(req: LaunchRequest): Promise<LaunchResult> {
      await wait(350)
      const a = db.accounts.find((x) => x.id === req.accountId)
      if (!a) return { ok: false, error: 'That account no longer exists. Reload the account list.' }
      const prov = db.providers.find((p) => p.id === a.providerId)!
      if (req.target === 'desktop' && !prov.desktopInstalled) {
        return { ok: false, error: `${prov.name} has no desktop app installed on this computer. Launch the CLI instead.` }
      }
      if (req.target !== 'desktop' && !prov.cliInstalled) {
        return {
          ok: false,
          error: `${prov.name} isn't installed, so there is nothing to launch. Install it with "${prov.installHint}" and try again.`
        }
      }
      const commandLine = buildCommand(a, req)
      a.launchCount += 1
      a.lastLaunchedAt = new Date().toISOString()
      accountsChanged()
      return { ok: true, commandLine }
    },
    terminals: createMockTerminals({
      account: (id) => db.accounts.find((a) => a.id === id),
      provider: (id) => db.providers.find((p) => p.id === id),
      defaultCwd: () => db.settings.defaultCwd ?? home,
      usage: () => db.usage,
      emit,
      notify(accountId, title, body) {
        const n = { id: `n-term-${Date.now()}`, kind: 'limit-reached' as const, accountId, title, body, createdAt: new Date().toISOString(), read: false }
        db.notifications.unshift(n)
        emit('notification', n)
      }
    }),
    async recommend(task, providerId) {
      await wait(90)
      return rankAccounts(task, providerId, db.accounts, db.usage)
    },
    notifications: {
      async list() {
        await wait(60)
        return clone(db.notifications)
      },
      async markRead(id) {
        await wait(60)
        for (const n of db.notifications) if (id === 'all' || n.id === id) n.read = true
      }
    },
    settings: {
      async get() {
        await wait(40)
        return clone(db.settings)
      },
      async set(patch: Partial<AppSettings>) {
        await wait(120)
        db.settings = {
          ...db.settings,
          ...patch,
          notifications: { ...db.settings.notifications, ...(patch.notifications ?? {}) },
          defaultSharing: { ...db.settings.defaultSharing, ...(patch.defaultSharing ?? {}) }
        }
        if (patch.pollIntervalMinutes) schedulePoll()
        return clone(db.settings)
      }
    },
    knowledge: {
      async get() {
        await wait(220)
        return PROVIDER_ORDER.map(knowledgeFor)
      },
      async reseed(providerId) {
        await wait(600)
        const k = knowledge[providerId]
        const kept = k.items.length
        const added = k.pending.length
        k.items.push(...k.pending)
        k.pending = []
        return { added, kept }
      }
    },
    skills: {
      async search(query) {
        const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
        if (query.trim().length < 2) return []
        await wait(250)
        return clone(directory.filter((s) => terms.every((t) => s.id.includes(t))).sort((a, b) => b.installs - a.installs))
      },
      async preview(id) {
        await wait(350)
        const name = folderOf(id)
        const description = featuredFor(id)?.blurb ?? 'A skill from the skills.sh directory.'
        const body = `# ${name}\n\n${description}\n\n## When to use it\n\nWhen the task matches the description above, the agent reads the rest of this file and follows it.\n\n## Steps\n\n1. Read the request.\n2. Follow the checklist.\n3. Report what changed.`
        return { id, name, description, body, files: 3 }
      },
      async installed() {
        await wait(120)
        const used = new Set(db.accounts.map((a) => a.providerId))
        return PROVIDER_ORDER.filter((p) => used.has(p)).flatMap((providerId) =>
          knowledge[providerId].items
            .filter(([kind]) => kind === 'skills')
            .map(([, name]): InstalledSkill => ({ providerId, name, id: installedFrom.get(`${providerId}/${name}`) ?? null }))
        )
      },
      async install(id, providerIds) {
        await wait(1100)
        const folder = folderOf(id)
        return providerIds.map((providerId) => {
          const items = knowledge[providerId].items
          if (items.some(([kind, name]) => kind === 'skills' && name === folder)) {
            return { providerId, error: `A skill named ${folder} is already in the shared folder` }
          }
          items.push(['skills', folder, 'skills'])
          installedFrom.set(`${providerId}/${folder}`, id)
          return { providerId, error: null }
        })
      },
      async remove(id, providerId) {
        await wait(300)
        const folder = folderOf(id)
        if (installedFrom.get(`${providerId}/${folder}`) !== id) throw new Error("Agent Hub didn't install this copy, so it was left alone")
        const k = knowledge[providerId]
        k.items = k.items.filter(([kind, name]) => !(kind === 'skills' && name === folder))
        installedFrom.delete(`${providerId}/${folder}`)
      }
    },
    system: {
      async openPath(path) {
        console.info('[mock] open folder', path)
      },
      async openUrl(url) {
        console.info('[mock] open link', url)
      },
      async pickDirectory() {
        await wait(200)
        return `${home}${sep}code${sep}northwind-api`
      },
      async pickSavePath(name) {
        await wait(150)
        return `${home}${sep}Desktop${sep}${name}`
      },
      async pickSetupFile() {
        await wait(150)
        return `${home}${sep}Downloads${sep}agent-hub-setup.agenthub`
      }
    },
    transfer: {
      async export(_path, opts) {
        await wait(400)
        return { accounts: opts.accountIds.length, signIns: opts.passphrase ? opts.accountIds.length : 0, files: opts.includeFiles ? 23 : 0, skippedFiles: 0 }
      },
      async preview(_path, passphrase) {
        await wait(300)
        const here = db.accounts.slice(0, 2).map((a) => a.label)
        return {
          fromOs: 'windows',
          exportedAt: new Date(Date.now() - 3600e3).toISOString(),
          accounts: [
            { key: 'x1', providerId: 'claude', label: here[0] ?? 'Work Max 20x', email: 'sam@northwind.dev', plan: 'Max 20x', hasSignIn: true, alreadyHere: here[0], targetHome: `${home}${sep}.claude` },
            { key: 'x2', providerId: 'claude', label: 'Laptop Pro', email: 'sam.side@northwind.dev', plan: 'Pro', hasSignIn: true, targetHome: `${home}${sep}.claude-laptop` },
            { key: 'x3', providerId: 'codex', label: 'Codex Plus', email: 'sam@northwind.dev', plan: 'Plus', hasSignIn: true, targetHome: `${home}${sep}.codex-plus` },
            { key: 'x4', providerId: 'gemini', label: 'Gemini spare', plan: 'Google AI Pro', hasSignIn: false, targetHome: `${home}${sep}gemini-spare` }
          ],
          kinds: [
            { providerId: 'claude', kind: 'skills', files: 18, newFiles: 6 },
            { providerId: 'claude', kind: 'agents', files: 4, newFiles: 0 },
            { providerId: 'claude', kind: 'instructions', files: 1, newFiles: 0 }
          ],
          hasSettings: true,
          hasSignIns: true,
          signInsUnlocked: passphrase === 'correct horse'
        }
      },
      async import(_path, passphrase, choice) {
        await wait(500)
        if (choice.signIns && passphrase !== 'correct horse') throw new Error("That passphrase doesn't open the sign-ins in this file")
        return { added: choice.accountKeys.map((k) => `Account ${k}`), skipped: [], signIns: choice.signIns ? choice.accountKeys.length : 0, filesAdded: choice.files ? 6 : 0, filesKept: choice.files ? 17 : 0, settings: choice.settings, problems: [] }
      }
    },
    on(event, cb) {
      let set = listeners.get(event)
      if (!set) listeners.set(event, (set = new Set()))
      set.add(cb as Listener)
      return () => set!.delete(cb as Listener)
    }
  }

  // Demo: a new notification arrives shortly after load so the live path is visible.
  setTimeout(() => {
    const n = {
      id: `n-live-${Date.now()}`,
      kind: 'switch-suggested' as const,
      accountId: 'acc-gpt-pro',
      title: 'Work Max 20x is at 36% of its session: ChatGPT Pro has 78% left',
      body: 'Switch for the next long task to keep Work Max for Opus work.',
      createdAt: new Date().toISOString(),
      read: false
    }
    db.notifications.unshift(n)
    emit('notification', n)
  }, 45_000)

  return api
}

/** The featured skills plus a few others, with made-up install counts. */
function mockDirectory(): DirectorySkill[] {
  const ids = [...FEATURED.flatMap((g) => g.skills.map((s) => s.id)), 'designed-by-ai/skills/design-mobile-apps', 'mattpocock/skills/codebase-design', 'anthropics/skills/web-artifacts-builder']
  return ids.map((id, i) => ({ id, name: folderOf(id), source: id.slice(0, id.lastIndexOf('/')), installs: 1_040_000 - i * 37_311 }))
}

/** [kind, name, subfolder]: items at the top of the shared folder have no subfolder. */
type MockItem = [KnowledgeItem['kind'], string, string?]

/** What each shared profile holds, plus what "Copy new" finds in the main folder the first time. */
function mockKnowledge(): Record<ProviderId, { items: MockItem[]; pending: MockItem[] }> {
  const skills = (...names: string[]): MockItem[] => names.map((n) => ['skills', n, 'skills'])
  return {
    claude: {
      items: [
        ['instructions', 'CLAUDE.md'],
        ['settings', 'settings.json'],
        ...skills('review-pr', 'write-tests', 'release-notes', 'sql-explain', 'tailwind-audit', 'pdf', 'changelog', 'deploy-preview', 'migrate-db'),
        ['agents', 'code-reviewer.md', 'agents'],
        ['agents', 'test-runner.md', 'agents'],
        ['agents', 'docs-writer.md', 'agents'],
        ['commands', 'ship.md', 'commands'],
        ['commands', 'standup.md', 'commands'],
        ['plugins', 'typescript-lsp', 'plugins'],
        ['plugins', 'github', 'plugins']
      ],
      pending: [...skills('incident-review', 'figma-to-react'), ['commands', 'triage.md', 'commands']]
    },
    codex: {
      items: [
        ['instructions', 'AGENTS.md'],
        ['settings', 'config.toml'],
        ...skills('review-pr', 'write-tests'),
        ['commands', 'fix-ci.md', 'prompts']
      ],
      pending: [['commands', 'bump-deps.md', 'prompts']]
    },
    gemini: {
      items: [
        ['instructions', 'GEMINI.md'],
        ['settings', 'settings.json'],
        ['commands', 'plan.toml', 'commands']
      ],
      pending: []
    },
    copilot: {
      items: [['settings', 'config.json']],
      pending: [['instructions', 'copilot-instructions.md']]
    }
  }
}

function defaultWindows(p: Account['providerId'], session: number, week: number): UsageWindow[] {
  const now = Date.now()
  const iso = (ms: number) => new Date(now + ms).toISOString()
  switch (p) {
    case 'claude':
      return [
        { id: 'five_hour', label: '5-hour session', usedPercent: session, windowMinutes: 300, resetsAt: iso(3 * 3600e3) },
        { id: 'seven_day', label: 'Weekly limit', usedPercent: week, windowMinutes: 10080, resetsAt: iso(5 * 86400e3) }
      ]
    case 'codex':
      return [
        { id: 'primary', label: '5-hour session', usedPercent: session, windowMinutes: 300, resetsAt: iso(4 * 3600e3) },
        { id: 'secondary', label: 'Weekly limit', usedPercent: week, windowMinutes: 10080, resetsAt: iso(6 * 86400e3) }
      ]
    case 'gemini':
      return [{ id: 'gemini-2.5-pro', label: 'Gemini Pro daily', usedPercent: session, windowMinutes: 1440, resetsAt: iso(14 * 3600e3) }]
    case 'copilot':
      return [{ id: 'premium', label: 'Premium requests', usedPercent: week, windowMinutes: 43200, resetsAt: iso(12 * 86400e3) }]
  }
}
