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

/** The tool's own folder (~/.claude): flagged as such, or at the path the tool uses by default (ProviderInfo.defaultHome). */
export function isMainFolder(a: Pick<Account, 'homeDir' | 'isDefaultHome'>, defaultHome: string | undefined, platform: Platform): boolean {
  return a.isDefaultHome || (!!defaultHome && pathKey(a.homeDir, platform) === pathKey(defaultHome, platform))
}

/** Whether "Also delete the profile folder" really deletes it: remove_account only deletes a home inside the profiles folder that isn't the tool's main folder. */
export function canDeleteHome(a: Pick<Account, 'homeDir' | 'isDefaultHome'>, profiles: string, defaultHome: string | undefined, platform: Platform): boolean {
  return !isMainFolder(a, defaultHome, platform) && pathKey(a.homeDir, platform).startsWith(`${pathKey(profiles, platform)}/`)
}
