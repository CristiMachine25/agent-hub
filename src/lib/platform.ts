export type Platform = 'win32' | 'darwin' | 'linux'

export function detectPlatform(): Platform {
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent
  if (/Windows/i.test(ua)) return 'win32'
  if (/Mac OS X|Macintosh/i.test(ua)) return 'darwin'
  return 'linux'
}

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/** "Ctrl" or "⌘" for shortcut hints. */
export function modKey(p: Platform = detectPlatform()): string {
  return p === 'darwin' ? '⌘' : 'Ctrl'
}
