// Terminal grid state: named sessions, each a workspace (layout + panes), terminal status, and the actions
// panes call. `ws` is always the active session's workspace; the others wait in `sessions` with their
// terminals still running. xterm instances live in ./registry; this store only holds plain data.
import { create } from 'zustand'
import type { TerminalInfo } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import {
  fitPanes,
  layoutFor,
  neighbor,
  LAYOUTS,
  parseWorkspace,
  reattach,
  serializeWorkspace,
  emptyPane,
  defaultWorkspace,
  type Dir,
  type LayoutId,
  type PaneConfig,
  type PaneTarget,
  type Workspace
} from '@/lib/grid'
import { disposeTerm, estimateSize, focusTerm, getTerm, onTerminalExit, prepare } from './registry'

const STORAGE_KEY = 'switchyard.workspace'
const SESSIONS_KEY = 'switchyard.sessions'

export interface Session {
  id: string
  name: string
  /** Workspace of an inactive session. The active one's lives in `ws` (this copy may be stale). */
  ws: Workspace
}

export interface StartOptions {
  accountId: string
  target: PaneTarget
  cwd?: string
  resume?: boolean
}

interface GridState {
  ws: Workspace
  sessions: Session[]
  activeId: string
  infos: Record<string, TerminalInfo>
  /** Pane uid → spawn in progress. */
  starting: Record<string, boolean>
  /** Pane uid → last spawn error. */
  errors: Record<string, string>
  restored: boolean

  restore(): Promise<void>
  setLayout(id: LayoutId): void
  focus(index: number): void
  move(dir: Dir): void
  toggleMax(index?: number): void
  start(index: number, opts: StartOptions): Promise<void>
  restart(index: number, resume: boolean): Promise<void>
  continueOn(index: number, accountId: string): Promise<void>
  close(index: number): Promise<void>
  openInGrid(accountId: string, opts?: { resume?: boolean; cwd?: string }): Promise<void>
  newSession(): void
  switchSession(id: string): void
  renameSession(id: string, name: string): void
  closeSession(id: string): Promise<void>
}

const newSessionId = () => `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

/** Saved sessions, or one session holding the single workspace older versions saved. */
function load(): { sessions: Session[]; activeId: string } {
  try {
    const d = JSON.parse(read(SESSIONS_KEY) ?? 'null') as { active?: string; list?: { id?: string; name?: string; ws?: unknown }[] } | null
    const list = (d?.list ?? [])
      .filter((x) => typeof x?.id === 'string')
      .map((x) => ({ id: x.id as string, name: typeof x.name === 'string' && x.name.trim() ? x.name : 'Session', ws: parseWorkspace(JSON.stringify(x.ws ?? null)) }))
    if (list.length) return { sessions: list, activeId: list.some((x) => x.id === d?.active) ? (d?.active as string) : list[0].id }
  } catch {
    /* unreadable: start fresh below */
  }
  const id = newSessionId()
  return { sessions: [{ id, name: 'Session 1', ws: parseWorkspace(read(STORAGE_KEY)) }], activeId: id }
}

function save(sessions: Session[], activeId: string, ws: Workspace) {
  try {
    const list = sessions.map((x) => ({ id: x.id, name: x.name, ws: JSON.parse(serializeWorkspace(x.id === activeId ? ws : x.ws)) }))
    localStorage.setItem(SESSIONS_KEY, JSON.stringify({ active: activeId, list }))
  } catch {
    /* private mode or full storage: the grid still works, it just won't be restored */
  }
}

/** "Session N" with the lowest N not in use. */
function nextName(sessions: Session[]): string {
  const used = new Set(sessions.map((x) => x.name))
  let n = 1
  while (used.has(`Session ${n}`)) n++
  return `Session ${n}`
}

const initial = load()

const theme = () => useStore.getState().resolvedTheme

/** The DOM node of a pane body, used to size a terminal before it exists. */
const paneBody = (uid: string) => document.querySelector<HTMLElement>(`[data-pane-body="${uid}"]`)

async function killQuietly(id: string | undefined) {
  if (!id) return
  disposeTerm(id)
  try {
    await api.terminals.kill(id)
  } catch {
    /* already gone */
  }
}

export const useGrid = create<GridState>((set, get) => {
  const patchPane = (uid: string, patch: Partial<PaneConfig> | null) => {
    set((s) => {
      const panes = s.ws.panes.map((p) => {
        if (p.uid !== uid) return p
        if (patch === null) return emptyPane(p.uid)
        const next = { ...p, ...patch }
        for (const k of Object.keys(patch) as (keyof PaneConfig)[]) if (patch[k] === undefined) delete next[k]
        return next
      })
      return { ws: { ...s.ws, panes } }
    })
  }

  onTerminalExit((id, code) => {
    set((s) => {
      const info = s.infos[id]
      if (!info) return {}
      return { infos: { ...s.infos, [id]: { ...info, exited: true, exitCode: code } } }
    })
  })

  /** Put the live workspace back into its session entry. */
  const stash = (): Session[] => {
    const { sessions, activeId, ws } = get()
    return sessions.map((x) => (x.id === activeId ? { ...x, ws } : x))
  }

  return {
    ws: initial.sessions.find((x) => x.id === initial.activeId)!.ws,
    sessions: initial.sessions,
    activeId: initial.activeId,
    infos: {},
    starting: {},
    errors: {},
    restored: false,

    async restore() {
      if (get().restored) return
      prepare()
      let running: TerminalInfo[] = []
      try {
        running = await api.terminals.list()
      } catch {
        /* backend without terminals: start with no live panes */
      }
      // Other sessions keep their own running terminals; only truly orphaned ones join the active grid.
      const alive = new Set(running.map((t) => t.id))
      const sessions = get().sessions.map((x) =>
        x.id === get().activeId ? x : { ...x, ws: { ...x.ws, panes: x.ws.panes.map((p) => (p.terminalId && !alive.has(p.terminalId) ? { ...p, terminalId: undefined } : p)) } }
      )
      const elsewhere = new Set(sessions.filter((x) => x.id !== get().activeId).flatMap((x) => x.ws.panes.map((p) => p.terminalId)))
      const ws = reattach(get().ws, running.filter((t) => !elsewhere.has(t.id)))
      const infos = Object.fromEntries(running.map((t) => [t.id, t]))
      // Seed each re-attached terminal with what it printed while the UI was away.
      const panes = [...ws.panes, ...sessions.flatMap((x) => (x.id === get().activeId ? [] : x.ws.panes))]
      await Promise.all(
        panes.map(async (p) => {
          if (!p.terminalId) return
          const sb = await api.terminals.scrollback(p.terminalId).catch(() => '')
          getTerm(p.terminalId, theme(), sb)
        })
      )
      set({ ws, sessions, infos, restored: true })
    },

    setLayout(id) {
      const { panes, removed } = fitPanes(get().ws.panes, LAYOUTS[id].rects.length)
      for (const p of removed) void killQuietly(p.terminalId)
      set((s) => ({
        ws: {
          ...s.ws,
          layout: id,
          panes,
          focused: Math.min(s.ws.focused, panes.length - 1),
          maximized: s.ws.maximized !== null && s.ws.maximized < panes.length ? s.ws.maximized : null
        }
      }))
    },

    focus(index) {
      if (index === get().ws.focused) return
      set((s) => ({ ws: { ...s.ws, focused: index } }))
      focusTerm(get().ws.panes[index]?.terminalId)
    },

    move(dir) {
      const { ws } = get()
      if (ws.maximized !== null) return
      const next = neighbor(LAYOUTS[ws.layout].rects, ws.focused, dir)
      get().focus(next)
    },

    toggleMax(index) {
      set((s) => {
        const i = index ?? s.ws.focused
        return { ws: { ...s.ws, focused: i, maximized: s.ws.maximized === i ? null : i } }
      })
      const ws = get().ws
      requestAnimationFrame(() => focusTerm(ws.panes[ws.focused]?.terminalId))
    },

    async start(index, opts) {
      const pane = get().ws.panes[index]
      if (!pane || get().starting[pane.uid]) return
      const uid = pane.uid
      set((s) => ({ starting: { ...s.starting, [uid]: true }, errors: { ...s.errors, [uid]: '' } }))
      patchPane(uid, { accountId: opts.accountId, target: opts.target, cwd: opts.cwd, resume: opts.resume || undefined, terminalId: undefined })
      try {
        const { cols, rows } = estimateSize(paneBody(uid))
        const cwd = opts.cwd ?? useStore.getState().settings?.defaultCwd
        const info = await api.terminals.spawn({ accountId: opts.accountId, target: opts.target, resume: opts.resume, cwd, cols, rows })
        // While the CLI started, the pane may have been closed or its session switched away.
        const owns = (ws: Workspace) => ws.panes.some((p) => p.uid === uid && p.accountId === opts.accountId)
        const here = owns(get().ws)
        const away = here ? undefined : get().sessions.find((x) => x.id !== get().activeId && owns(x.ws))
        if (!here && !away) {
          void killQuietly(info.id)
          return
        }
        getTerm(info.id, theme())
        set((s) => ({ infos: { ...s.infos, [info.id]: info } }))
        if (away) {
          const panes = away.ws.panes.map((p) => (p.uid === uid ? { ...p, terminalId: info.id } : p))
          set((s) => ({ sessions: s.sessions.map((x) => (x.id === away.id ? { ...x, ws: { ...x.ws, panes } } : x)) }))
          return
        }
        patchPane(uid, { terminalId: info.id })
        if (get().ws.panes[get().ws.focused]?.uid === uid) requestAnimationFrame(() => focusTerm(info.id))
      } catch (e) {
        set((s) => ({ errors: { ...s.errors, [uid]: errorText(e) } }))
      } finally {
        set((s) => ({ starting: { ...s.starting, [uid]: false } }))
      }
    },

    async restart(index, resume) {
      const pane = get().ws.panes[index]
      if (!pane?.accountId) return
      await killQuietly(pane.terminalId)
      await get().start(index, { accountId: pane.accountId, target: 'cli', cwd: pane.cwd, resume })
    },

    async continueOn(index, accountId) {
      const pane = get().ws.panes[index]
      if (!pane) return
      await killQuietly(pane.terminalId)
      await get().start(index, { accountId, target: 'cli', cwd: pane.cwd, resume: true })
    },

    async close(index) {
      const pane = get().ws.panes[index]
      if (!pane) return
      await killQuietly(pane.terminalId)
      patchPane(pane.uid, null)
      set((s) => ({ ws: { ...s.ws, maximized: s.ws.maximized === index ? null : s.ws.maximized } }))
    },

    async openInGrid(accountId, opts = {}) {
      await get().restore()
      useStore.getState().go('terminals')
      let { ws } = get()
      let index = ws.panes.findIndex((p) => !p.accountId)
      if (index < 0) {
        if (ws.panes.length >= 9) index = ws.focused
        else {
          get().setLayout(layoutFor(ws.panes.length + 1))
          ws = get().ws
          index = ws.panes.findIndex((p) => !p.accountId)
        }
      }
      set((s) => ({ ws: { ...s.ws, focused: index, maximized: null } }))
      // Let the grid render so the new pane has a size to fit the terminal to.
      await new Promise((r) => requestAnimationFrame(() => r(null)))
      if (ws.panes[index]?.terminalId) await killQuietly(ws.panes[index].terminalId)
      await get().start(index, { accountId, target: 'cli', cwd: opts.cwd, resume: opts.resume })
    },

    newSession() {
      const sessions = stash()
      const id = newSessionId()
      const ws = defaultWorkspace()
      set({ sessions: [...sessions, { id, name: nextName(sessions), ws }], activeId: id, ws })
    },

    switchSession(id) {
      if (id === get().activeId) return
      const sessions = stash()
      const target = sessions.find((x) => x.id === id)
      if (!target) return
      set({ sessions, activeId: id, ws: target.ws })
      requestAnimationFrame(() => focusTerm(target.ws.panes[target.ws.focused]?.terminalId))
    },

    renameSession(id, name) {
      const clean = name.trim().slice(0, 40)
      if (!clean) return
      set((s) => ({ sessions: s.sessions.map((x) => (x.id === id ? { ...x, name: clean } : x)) }))
    },

    async closeSession(id) {
      const sessions = stash()
      const gone = sessions.find((x) => x.id === id)
      if (!gone) return
      await Promise.all(gone.ws.panes.map((p) => killQuietly(p.terminalId)))
      let rest = sessions.filter((x) => x.id !== id)
      if (!rest.length) rest = [{ id: newSessionId(), name: 'Session 1', ws: defaultWorkspace() }]
      const activeId = id === get().activeId ? rest[Math.max(0, sessions.indexOf(gone) - 1)]?.id ?? rest[0].id : get().activeId
      set({ sessions: rest, activeId, ws: rest.find((x) => x.id === activeId)!.ws })
    }
  }
})

useGrid.subscribe((s, prev) => {
  if (s.ws !== prev.ws || s.sessions !== prev.sessions || s.activeId !== prev.activeId) save(s.sessions, s.activeId, s.ws)
})
