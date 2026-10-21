import { useEffect } from 'react'
import { Loader2 } from 'lucide-react'
import { useStore, type Screen } from './store'
import { Sidebar } from './components/Sidebar'
import { Toasts } from './components/Toasts'
import { TitleBar } from './components/TitleBar'
import { CommandPalette } from './components/CommandPalette'
import { AddAccountDialog } from './components/AddAccountDialog'
import { Overview } from './screens/Overview'
import { Accounts } from './screens/Accounts'
import { Knowledge } from './screens/Knowledge'
import { Skills } from './screens/Skills'
import { Notifications } from './screens/Notifications'
import { Settings } from './screens/Settings'
import { Terminals } from './terminals/Terminals'

const SCREENS: Screen[] = ['overview', 'accounts', 'knowledge', 'skills', 'terminals', 'notifications', 'settings']

function useTheme() {
  const pref = useStore((s) => s.settings?.theme ?? 'dark')
  const setResolved = useStore((s) => s.setResolvedTheme)
  useEffect(() => {
    const forced = new URLSearchParams(location.search).get('theme')
    const mq = window.matchMedia('(prefers-color-scheme: light)')
    const apply = () => {
      const want = forced === 'light' || forced === 'dark' ? forced : pref
      const t = want === 'system' ? (mq.matches ? 'light' : 'dark') : want
      document.documentElement.dataset.theme = t
      setResolved(t)
    }
    apply()
    mq.addEventListener('change', apply)
    return () => mq.removeEventListener('change', apply)
  }, [pref, setResolved])
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState()
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        s.setPalette(!s.paletteOpen)
        return
      }
      const t = e.target as HTMLElement
      const typing = t.closest('input, textarea, select, [contenteditable="true"]')
      if (typing || e.ctrlKey || e.metaKey || e.altKey || s.paletteOpen || s.addOpen || document.querySelector('[role="dialog"]')) return
      const n = Number(e.key)
      if (n >= 1 && n <= SCREENS.length) s.go(SCREENS[n - 1])
      if (e.key === 'r' || e.key === 'R') void s.refresh()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

/** Dev convenience: #accounts/acc-h2 opens a screen (used for screenshots and links from notifications). */
function useHashRoute() {
  const ready = useStore((s) => s.phase === 'ready')
  useEffect(() => {
    if (!ready) return
    const apply = () => {
      const [screen, id] = location.hash.replace(/^#\/?/, '').split('/')
      const s = useStore.getState()
      if (SCREENS.includes(screen as Screen)) s.go(screen as Screen, id ?? null)
      s.setPalette(screen === 'palette')
      s.setAddOpen(screen === 'add')
    }
    apply()
    window.addEventListener('hashchange', apply)
    return () => window.removeEventListener('hashchange', apply)
  }, [ready])
}

export function App() {
  const phase = useStore((s) => s.phase)
  const loadError = useStore((s) => s.loadError)
  const screen = useStore((s) => s.screen)
  const init = useStore((s) => s.init)
  const tick = useStore((s) => s.tick)

  useTheme()
  useShortcuts()
  useHashRoute()

  useEffect(() => {
    void init()
    const id = setInterval(tick, 30_000)
    return () => clearInterval(id)
  }, [init, tick])

  if (phase === 'loading')
    return (
      <div className="flex h-full flex-col">
        <TitleBar />
        <div className="flex flex-1 items-center justify-center gap-2 text-muted" role="status">
          <Loader2 size={16} className="animate-spin" /> Loading your accounts…
        </div>
      </div>
    )
  if (phase === 'error')
    return (
      <div className="flex h-full flex-col">
        <TitleBar />
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
          <p className="text-md font-semibold">Switchyard couldn't load its data</p>
          <p className="max-w-md text-muted">{loadError}</p>
          <button className="btn btn-primary" onClick={() => init()}>
            Try again
          </button>
        </div>
      </div>
    )

  return (
    <div className="flex h-full">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TitleBar />
        <main className={screen === 'terminals' ? 'min-h-0 flex-1 overflow-hidden' : 'min-h-0 flex-1 overflow-y-auto'} id="main">
          {screen === 'overview' && <Overview />}
          {screen === 'accounts' && <Accounts />}
          {screen === 'knowledge' && <Knowledge />}
          {screen === 'skills' && <Skills />}
          {screen === 'terminals' && <Terminals />}
          {screen === 'notifications' && <Notifications />}
          {screen === 'settings' && <Settings />}
        </main>
      </div>
      <CommandPalette />
      <AddAccountDialog />
      <Toasts />
    </div>
  )
}
