import { useEffect, useState } from 'react'
import { Copy, Minus, Square, X } from 'lucide-react'
import { detectPlatform, isTauri } from '@/lib/platform'

/**
 * The window has no native title bar on Windows and Linux, so this strip is the drag area and
 * holds the window buttons. On macOS the native traffic lights float over the sidebar instead, and the
 * strip only appears on the loading and error screens so the window can still be dragged.
 * The browser preview has no window chrome, so it gets no strip at all.
 */
export function TitleBar() {
  const custom = isTauri() && detectPlatform() !== 'darwin'
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    if (!custom) return
    let off: (() => void) | undefined
    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const w = getCurrentWindow()
      setMaximized(await w.isMaximized())
      off = await w.onResized(async () => setMaximized(await w.isMaximized()))
    })
    return () => off?.()
  }, [custom])

  const act = (what: 'minimize' | 'toggleMaximize' | 'close') =>
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow()[what]())

  if (!isTauri()) return null
  return (
    <div data-tauri-drag-region className="flex h-8 shrink-0 select-none items-stretch justify-end">
      {custom && (
        <div className="flex">
          <button className="titlebar-btn" aria-label="Minimize" onClick={() => act('minimize')}>
            <Minus size={15} strokeWidth={1.6} />
          </button>
          <button className="titlebar-btn" aria-label={maximized ? 'Restore' : 'Maximize'} onClick={() => act('toggleMaximize')}>
            {maximized ? <Copy size={12} strokeWidth={1.6} className="-scale-x-100" /> : <Square size={12} strokeWidth={1.6} />}
          </button>
          <button className="titlebar-btn titlebar-close" aria-label="Close" onClick={() => act('close')}>
            <X size={16} strokeWidth={1.6} />
          </button>
        </div>
      )}
    </div>
  )
}
