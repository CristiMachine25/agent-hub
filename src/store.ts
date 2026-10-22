import { create } from 'zustand'
import type {
  Account,
  AccountPatch,
  AppNotification,
  AppSettings,
  LaunchTarget,
  ProviderInfo,
  Recommendation,
  TaskType,
  UsageSnapshot
} from '@shared/types'
import { api, errorText } from './api'
import { PROVIDER_META } from './lib/providers'

export type Screen = 'overview' | 'accounts' | 'knowledge' | 'skills' | 'terminals' | 'notifications' | 'settings'

export interface Toast {
  id: number
  kind: 'success' | 'error' | 'info'
  title: string
  body?: string
  command?: string
}

interface State {
  phase: 'loading' | 'ready' | 'error'
  loadError?: string
  providers: ProviderInfo[]
  accounts: Account[]
  usage: Record<string, UsageSnapshot>
  notifications: AppNotification[]
  settings: AppSettings | null
  recs: Recommendation[]
  task: TaskType
  screen: Screen
  selectedId: string | null
  now: number
  refreshing: boolean
  toasts: Toast[]
  paletteOpen: boolean
  addOpen: boolean
  launching: string | null
  resolvedTheme: 'dark' | 'light'

  setResolvedTheme(t: 'dark' | 'light'): void
  init(): Promise<void>
  tick(): void
  go(screen: Screen, selectedId?: string | null): void
  setTask(task: TaskType): void
  loadRecs(): Promise<void>
  refresh(accountId?: string): Promise<void>
  launch(accountId: string, target?: LaunchTarget, opts?: { resume?: boolean; cwd?: string }): Promise<void>
  updateAccount(id: string, patch: AccountPatch, opts?: { quiet?: boolean }): Promise<boolean>
  removeAccount(id: string, deleteHome: boolean): Promise<boolean>
  markRead(id: string | 'all'): Promise<void>
  saveSettings(patch: Partial<AppSettings>): Promise<void>
  toast(t: Omit<Toast, 'id'>): void
  dismiss(id: number): void
  setPalette(open: boolean): void
  setAddOpen(open: boolean): void
}

let toastSeq = 1
let settingsSeq = 0
let subscribed = false
/** Set while init() loads: which of accounts/usage the backend pushed meanwhile (newer than the load). */
let pushedDuringLoad: Set<'accounts' | 'usage'> | null = null

export const useStore = create<State>((set, get) => ({
  phase: 'loading',
  providers: [],
  accounts: [],
  usage: {},
  notifications: [],
  settings: null,
  recs: [],
  task: 'coding',
  screen: 'overview',
  selectedId: null,
  now: Date.now(),
  refreshing: false,
  toasts: [],
  paletteOpen: false,
  addOpen: false,
  launching: null,
  resolvedTheme: 'dark',

  setResolvedTheme(t) {
    set({ resolvedTheme: t })
  },

  async init() {
    set({ phase: 'loading', loadError: undefined })
    // Subscribe before loading: the backend may change accounts (a login renewed at startup)
    // between answering the load and the subscription, and that change would be lost.
    if (!subscribed) {
      subscribed = true
      api.on('usage-updated', (u) => {
        pushedDuringLoad?.add('usage')
        set({ usage: u })
        void get().loadRecs()
      })
      api.on('accounts-changed', (a) => {
        pushedDuringLoad?.add('accounts')
        set({ accounts: a })
        void get().loadRecs()
      })
      api.on('navigate', (screen) => {
        if (['overview', 'accounts', 'knowledge', 'skills', 'terminals', 'notifications', 'settings'].includes(screen)) get().go(screen as Screen)
      })
      api.on('notification', (n) => {
        set((s) => ({ notifications: [n, ...s.notifications.filter((x) => x.id !== n.id)] }))
        get().toast({ kind: 'info', title: n.title, body: n.body })
      })
    }
    const pushed = new Set<'accounts' | 'usage'>()
    pushedDuringLoad = pushed
    try {
      const [providers, accounts, usage, notifications, settings] = await Promise.all([
        api.providers.list(),
        api.accounts.list(),
        api.usage.latest(),
        api.notifications.list(),
        api.settings.get()
      ])
      set({
        providers,
        notifications,
        settings,
        phase: 'ready',
        now: Date.now(),
        ...(pushed.has('accounts') ? {} : { accounts }),
        ...(pushed.has('usage') ? {} : { usage })
      })
      void get().loadRecs()
    } catch (e) {
      set({ phase: 'error', loadError: errorText(e) })
    } finally {
      if (pushedDuringLoad === pushed) pushedDuringLoad = null
    }
  },

  tick() {
    set({ now: Date.now() })
  },

  go(screen, selectedId) {
    set((s) => ({ screen, selectedId: selectedId === undefined ? s.selectedId : selectedId }))
  },

  setTask(task) {
    set({ task })
    void get().loadRecs()
  },

  async loadRecs() {
    const task = get().task
    try {
      const recs = await api.recommend(task)
      if (get().task === task) set({ recs })
    } catch {
      /* recommendations are optional; the board still shows every account */
    }
  },

  async refresh(accountId) {
    set({ refreshing: true })
    try {
      const usage = await api.usage.refresh(accountId)
      set({ usage, now: Date.now() })
      void get().loadRecs()
    } catch (e) {
      get().toast({ kind: 'error', title: "Couldn't refresh usage", body: errorText(e) })
    } finally {
      set({ refreshing: false })
    }
  },

  async launch(accountId, target = 'cli', opts = {}) {
    const acc = get().accounts.find((a) => a.id === accountId)
    const name = acc ? PROVIDER_META[acc.providerId].short : 'the CLI'
    const cwd = opts.cwd ?? get().settings?.defaultCwd
    set({ launching: accountId + target })
    try {
      const res = await api.launch({ accountId, target, cwd, resume: opts.resume })
      if (res.ok) {
        const what =
          target === 'login'
            ? `Opened sign-in for ${acc?.label ?? 'the account'}`
            : target === 'desktop'
              ? `Opened the desktop app as ${acc?.label}`
              : `${opts.resume ? 'Resumed' : 'Launched'} ${name} as ${acc?.label}`
        get().toast({ kind: 'success', title: what, body: res.note, command: res.commandLine })
      } else {
        get().toast({
          kind: 'error',
          title: `Couldn't launch ${name}`,
          body: res.error ?? 'The launcher gave no reason. Check the terminal setting and try again.',
          command: res.commandLine
        })
      }
    } catch (e) {
      get().toast({ kind: 'error', title: `Couldn't launch ${name}`, body: errorText(e) })
    } finally {
      set({ launching: null })
    }
  },

  async updateAccount(id, patch, opts = {}) {
    try {
      const updated = await api.accounts.update(id, patch)
      // accounts_update re-applies sharing itself when the sharing options changed.
      set((s) => ({ accounts: s.accounts.map((a) => (a.id === id ? updated : a)) }))
      if (!opts.quiet) get().toast({ kind: 'success', title: 'Saved' })
      return true
    } catch (e) {
      get().toast({ kind: 'error', title: "Couldn't save the account", body: errorText(e) })
      return false
    }
  },

  async removeAccount(id, deleteHome) {
    const acc = get().accounts.find((a) => a.id === id)
    try {
      const deleted = await api.accounts.remove(id, { deleteHome })
      set((s) => ({
        accounts: s.accounts.filter((a) => a.id !== id),
        selectedId: s.selectedId === id ? null : s.selectedId
      }))
      get().toast({
        kind: 'success',
        title: `Removed ${acc?.label ?? 'the account'}`,
        body: deleted
          ? 'Its profile folder was deleted too.'
          : deleteHome
            ? 'Its folder was left on disk: Agent Hub only deletes folders it made in its profiles folder.'
            : 'Its profile folder was left on disk.'
      })
      void get().loadRecs()
      return true
    } catch (e) {
      get().toast({ kind: 'error', title: "Couldn't remove the account", body: errorText(e) })
      return false
    }
  },

  async markRead(id) {
    set((s) => ({ notifications: s.notifications.map((n) => (id === 'all' || n.id === id ? { ...n, read: true } : n)) }))
    try {
      await api.notifications.markRead(id)
    } catch {
      /* the local state is already updated; the next list() reconciles */
    }
  },

  async saveSettings(patch) {
    // Saves can overlap (sliders, toggles). Only the newest one may apply its result or roll back.
    const seq = ++settingsSeq
    const prev = get().settings
    if (prev) {
      set({
        settings: {
          ...prev,
          ...patch,
          notifications: { ...prev.notifications, ...(patch.notifications ?? {}) },
          defaultSharing: { ...prev.defaultSharing, ...(patch.defaultSharing ?? {}) }
        }
      })
    }
    try {
      const settings = await api.settings.set(patch)
      if (seq === settingsSeq) set({ settings })
    } catch (e) {
      if (seq === settingsSeq) set({ settings: prev })
      get().toast({ kind: 'error', title: "Couldn't save settings", body: errorText(e) })
    }
  },

  toast(t) {
    const id = toastSeq++
    set((s) => ({ toasts: [...s.toasts.slice(-3), { ...t, id }] }))
    const ttl = t.kind === 'error' ? 12_000 : t.command ? 9_000 : 5_000
    setTimeout(() => get().dismiss(id), ttl)
  },

  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }))
  },

  setPalette(open) {
    set({ paletteOpen: open })
  },

  setAddOpen(open) {
    set({ addOpen: open })
  }
}))
