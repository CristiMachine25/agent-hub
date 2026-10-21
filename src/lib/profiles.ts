import type { Account, AppSettings } from '@shared/types'
import type { Platform } from './platform'

/** Where new profiles live: settings.profilesDir, else "profiles" in the app data folder (store.rs `profiles_dir`). */
export function profilesDir(settings: Pick<AppSettings, 'profilesDir'> | null, appDataDir: string): string {
  return settings?.profilesDir?.trim() || `${appDataDir.replace(/[\\/]+$/, '')}/profiles`
}

/** util.rs `path_key`: forward slashes, no trailing slash, case-insensitive on Windows. */
function pathKey(p: string, platform: Platform): string {
  const s = p.replace(/[\\/]+$/, '').replace(/\\/g, '/')
  return platform === 'win32' ? s.toLowerCase() : s
}

/** Whether "Also delete the profile folder" really deletes it: remove_account only deletes a non-default home inside the profiles folder. */
export function canDeleteHome(a: Pick<Account, 'homeDir' | 'isDefaultHome'>, profiles: string, platform: Platform): boolean {
  return !a.isDefaultHome && pathKey(a.homeDir, platform).startsWith(`${pathKey(profiles, platform)}/`)
}
