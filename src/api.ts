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
    recommend: (task, providerId) => call(COMMANDS.recommend, { task, providerId: providerId ?? null }),
    notifications: {
      list: () => call(COMMANDS.notificationsList),
      markRead: (id) => call(COMMANDS.notificationsMarkRead, { id })
    },
    settings: {
      get: () => call(COMMANDS.settingsGet),
      set: (patch) => call(COMMANDS.settingsSet, { patch })
    },
    system: {
      openPath: (path) => call(COMMANDS.systemOpenPath, { path }),
      async pickDirectory() {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const picked = await open({ directory: true, multiple: false })
        return typeof picked === 'string' ? picked : null
      }
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

/** Turn any thrown value (Tauri returns plain strings) into a readable message. */
export function errorText(e: unknown): string {
  if (typeof e === 'string') return e
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message)
  return 'Something went wrong. Try again, and check the app log if it keeps happening.'
}
