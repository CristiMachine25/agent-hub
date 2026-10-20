// Terminal grid state: the workspace (layout + panes), terminal status, and the actions panes call.
// xterm instances live in ./registry; this store only holds plain data.
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
  type Dir,
  type LayoutId,
  type PaneConfig,
  type PaneTarget,
  type Workspace
} from '@/lib/grid'
import { disposeTerm, estimateSize, focusTerm, getTerm, onTerminalExit, prepare } from './registry'

const STORAGE_KEY = 'baton.workspace'

export interface StartOptions {
  accountId: string
  target: PaneTarget
  cwd?: string
  resume?: boolean
}

interface GridState {
  ws: Workspace
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
}

function load(): Workspace {
  try {
    return parseWorkspace(localStorage.getItem(STORAGE_KEY))
  } catch {
    return parseWorkspace(null)
  }
}

function save(ws: Workspace) {
  try {
    localStorage.setItem(STORAGE_KEY, serializeWorkspace(ws))
  } catch {
    /* private mode or full storage: the grid still works, it just won't be restored */
  }
}

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

  return {
    ws: load(),
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
      const ws = reattach(get().ws, running)
      const infos = Object.fromEntries(running.map((t) => [t.id, t]))
      // Seed each re-attached terminal with what it printed while the UI was away.
      await Promise.all(
        ws.panes.map(async (p) => {
          if (!p.terminalId) return
          const sb = await api.terminals.scrollback(p.terminalId).catch(() => '')
          getTerm(p.terminalId, theme(), sb)
        })
      )
      set({ ws, infos, restored: true })
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
        getTerm(info.id, theme())
        set((s) => ({ infos: { ...s.infos, [info.id]: info } }))
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
    }
  }
})

useGrid.subscribe((s, prev) => {
  if (s.ws !== prev.ws) save(s.ws)
})
