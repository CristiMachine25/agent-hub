// HopperApi: Tauri invoke/listen inside the app, an in-memory mock in a plain browser.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { AppEvents, HopperApi } from '@shared/types'
import { COMMANDS } from '@shared/types'
import { createMockApi } from './mock/backend'
import { detectPlatform, isTauri } from './lib/platform'

function createTauriApi(): HopperApi {
  const call = <T,>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args)
  return {
    platform: detectPlatform(),
    providers: {
      list: () => call(COMMANDS.providersList)
    },
    accounts: {
      list: () => call(COMMANDS.accountsList),
      detect: () => call(COMMANDS.accountsDetect),
      import: (found) => call(COMMANDS.accountsImport, { found }),
      create: (input) => call(COMMANDS.accountsCreate, { input }),
      update: (id, patch) => call(COMMANDS.accountsUpdate, { id, patch }),
      remove: (id, opts) => call(COMMANDS.accountsRemove, { id, deleteHome: opts.deleteHome }),
      applySharing: (id) => call(COMMANDS.accountsApplySharing, { id })
    },
    usage: {
      latest: () => call(COMMANDS.usageLatest),
      history: (accountId, days) => call(COMMANDS.usageHistory, { accountId, days }),
      activity: (accountId, days) => call(COMMANDS.usageActivity, { accountId, days }),
      refresh: (accountId) => call(COMMANDS.usageRefresh, { accountId: accountId ?? null }),
      setManual: (accountId, windows) => call(COMMANDS.usageSetManual, { accountId, windows })
    },
    launch: (req) => call(COMMANDS.launch, { req }),
    terminals: {
      spawn: (req) => call(COMMANDS.terminalSpawn, { req }),
      write: (id, data) => call(COMMANDS.terminalWrite, { id, data }),
      resize: (id, cols, rows) => call(COMMANDS.terminalResize, { id, cols, rows }),
      kill: (id) => call(COMMANDS.terminalKill, { id }),
      list: () => call(COMMANDS.terminalList),
      scrollback: (id) => call(COMMANDS.terminalScrollback, { id })
    },
    recommend: (task, providerId) => call(COMMANDS.recommend, { task, providerId: providerId ?? null }),
    notifications: {
      list: () => call(COMMANDS.notificationsList),
      markRead: (id) => call(COMMANDS.notificationsMarkRead, { id })
    },
    settings: {
      get: () => call(COMMANDS.settingsGet),
      set: (patch) => call(COMMANDS.settingsSet, { patch })
    },
    knowledge: {
      get: () => call(COMMANDS.knowledgeGet),
      reseed: (providerId) => call(COMMANDS.knowledgeReseed, { providerId })
    },
    skills: {
      search: (query) => call(COMMANDS.skillsSearch, { query }),
      preview: (id) => call(COMMANDS.skillsPreview, { id }),
      installed: () => call(COMMANDS.skillsInstalled),
      install: (id, providerIds) => call(COMMANDS.skillsInstall, { id, providerIds }),
      remove: (id, providerId) => call(COMMANDS.skillsRemove, { id, providerId })
    },
    system: {
      openPath: (path) => call(COMMANDS.systemOpenPath, { path }),
      openUrl: (url) => call(COMMANDS.systemOpenUrl, { url }),
      async pickDirectory() {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const picked = await open({ directory: true, multiple: false })
        return typeof picked === 'string' ? picked : null
      },
      async pickSavePath(defaultName) {
        const { save } = await import('@tauri-apps/plugin-dialog')
        return (await save({ defaultPath: defaultName, filters: [{ name: 'Agent Hub setup', extensions: ['agenthub'] }] })) ?? null
      },
      async pickSetupFile() {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const picked = await open({ multiple: false, filters: [{ name: 'Agent Hub setup', extensions: ['agenthub', 'switchyard', 'json'] }] })
        return typeof picked === 'string' ? picked : null
      }
    },
    transfer: {
      export: (path, opts) => call(COMMANDS.transferExport, { path, opts }),
      preview: (path, passphrase) => call(COMMANDS.transferPreview, { path, passphrase: passphrase ?? null }),
      import: (path, passphrase, choice) => call(COMMANDS.transferImport, { path, passphrase: passphrase ?? null, choice })
    },
    on<K extends keyof AppEvents>(event: K, cb: (payload: AppEvents[K]) => void) {
      let unlisten: (() => void) | undefined
      let cancelled = false
      listen<AppEvents[K]>(event, (e) => cb(e.payload)).then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      return () => {
        cancelled = true
        unlisten?.()
      }
    }
  }
}

export const IS_MOCK = !isTauri()

export const api: HopperApi = IS_MOCK ? createMockApi(detectPlatform()) : createTauriApi()

/** Where the backend writes its log file (tauri-plugin-log's LogDir target). */
const LOG_DIR = {
  darwin: '~/Library/Logs/dev.switchyard.app',
  win32: '%LOCALAPPDATA%\\dev.switchyard.app\\logs',
  linux: '~/.local/share/dev.switchyard.app/logs'
} as const

/** Turn any thrown value (Tauri returns plain strings) into a readable message. */
export function errorText(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message)
  return `Something went wrong. Try again; if it keeps happening, check Agent Hub.log in ${LOG_DIR[detectPlatform()]}.`
}
