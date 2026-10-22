// xterm.js instances for the terminal grid, kept outside React so output never re-renders components.
// Each terminal draws into its own host <div>; panes adopt that div when they mount and release it when
// they unmount, so switching screens or layouts keeps the terminal and its scrollback intact.
import { Terminal, type ITheme } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { api } from '@/api'

export interface TermEntry {
  term: Terminal
  fit: FitAddon
  host: HTMLDivElement
  cols: number
  rows: number
  /** Set while the terminal draws with WebGL; otherwise xterm's DOM renderer is in use. */
  webgl?: WebglAddon
  /** The canvas holding the addon's WebGL context, freed when the addon is released. */
  webglCanvas?: HTMLCanvasElement
  /** The context was lost once; the next show tries WebGL again. */
  lostWebgl?: boolean
  /** WebGL failed to load or lost its context twice; this terminal stays on the DOM renderer. */
  noWebgl?: boolean
}

const entries = new Map<string, TermEntry>()
/** Output that arrived before the pane created its terminal (spawn returns after the process starts). */
const pending = new Map<string, string[]>()
const PENDING_LIMIT = 512
/** Closed terminals whose last output may still be on its way; it is dropped until their exit event. */
const disposed = new Set<string>()

/**
 * WebGL keeps busy grids smooth (the DOM renderer drops to ~15 fps with 9 busy panes in WKWebView), but each
 * WebGL terminal holds a live context and WebKit drops the oldest past a limit (16). Terminals get WebGL
 * when a pane shows them, at most MAX_WEBGL at once (the largest grid), taking it from one that is off
 * screen if needed; terminals in hidden grid sessions keep theirs until then. Everything else uses DOM.
 */
export const MAX_WEBGL = 9

let listening = false
let exitHandler: (id: string, code: number | null) => void = () => {}

export const FONT_SIZE_KEY = 'switchyard.terminalFontSize'
export const FONT_SIZES = [11, 12, 13, 14, 15, 16, 18]

export function terminalFontSize(): number {
  try {
    const n = Number(localStorage.getItem(FONT_SIZE_KEY))
    return FONT_SIZES.includes(n) ? n : 13
  } catch {
    return 13
  }
}

const rgb = (name: string) => {
  const v = getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim()
  return v ? `rgb(${v.split(/\s+/).join(', ')})` : undefined
}

/** ANSI palettes tuned for the app's warm charcoal (dark) and paper (light) grounds. */
const ANSI: Record<'dark' | 'light', ITheme> = {
  dark: {
    black: '#2A2826',
    red: '#F07A7A',
    green: '#7CC99A',
    yellow: '#E3C26E',
    blue: '#8FA8F5',
    magenta: '#C29CF0',
    cyan: '#79C4C9',
    white: '#D8D5CD',
    brightBlack: '#75726B',
    brightRed: '#FF9A9A',
    brightGreen: '#9EDDB6',
    brightYellow: '#F2D88F',
    brightBlue: '#AFC1FF',
    brightMagenta: '#D7B9FA',
    brightCyan: '#9AD9DD',
    brightWhite: '#F5F3EE'
  },
  light: {
    black: '#2A2826',
    red: '#B4232F',
    green: '#1E7A4A',
    yellow: '#8A6300',
    blue: '#3352C2',
    magenta: '#8236B8',
    cyan: '#16737A',
    white: '#E4E1D9',
    brightBlack: '#6E6A62',
    brightRed: '#D03440',
    brightGreen: '#25915A',
    brightYellow: '#A27700',
    brightBlue: '#4566DA',
    brightMagenta: '#9A4BD1',
    brightCyan: '#1B8A92',
    brightWhite: '#FFFFFF'
  }
}

function terminalTheme(mode: 'dark' | 'light'): ITheme {
  return {
    ...ANSI[mode],
    background: rgb('ground'),
    foreground: rgb('ink'),
    cursor: rgb('hop'),
    cursorAccent: rgb('ground'),
    selectionBackground: mode === 'dark' ? 'rgba(154, 165, 255, 0.32)' : 'rgba(69, 86, 214, 0.22)'
  }
}

function ensureListeners() {
  if (listening) return
  listening = true
  api.on('terminal-output', ({ id, data }) => {
    const e = entries.get(id)
    if (e) {
      e.term.write(data)
      return
    }
    if (disposed.has(id)) return
    const buf = pending.get(id) ?? []
    if (buf.length < PENDING_LIMIT) buf.push(data)
    pending.set(id, buf)
  })
  api.on('terminal-exit', ({ id, code }) => {
    disposed.delete(id)
    exitHandler(id, code)
  })
  // WebGL caches glyph bitmaps, so glyphs drawn before the terminal font finished loading would stay in the
  // fallback font.
  document.fonts?.addEventListener?.('loadingdone', () => {
    for (const e of entries.values()) e.webgl?.clearTextureAtlas()
  })
}

/** Called by the grid store so exits update pane status. */
export function onTerminalExit(fn: (id: string, code: number | null) => void) {
  exitHandler = fn
  ensureListeners()
}

/** Start listening before the first spawn so early output is buffered, not lost. */
export function prepare() {
  ensureListeners()
}

function isMac() {
  return api.platform === 'darwin'
}

/**
 * Keys xterm should leave alone: grid shortcuts (Ctrl/Cmd+Shift+arrows and Enter), the quick launcher,
 * and copy/paste on Windows and Linux (Ctrl+Shift+C/V, plus Ctrl+C while text is selected).
 */
function keyHandler(term: Terminal) {
  return (ev: KeyboardEvent): boolean => {
    if (ev.type !== 'keydown') return true
    const mod = isMac() ? ev.metaKey : ev.ctrlKey
    const key = ev.key.toLowerCase()
    if (mod && ev.shiftKey && (key.startsWith('arrow') || key === 'enter')) return false
    if (mod && !ev.shiftKey && key === 'k') return false
    if (!isMac() && ev.ctrlKey && (ev.shiftKey || term.hasSelection()) && key === 'c') {
      const text = term.getSelection()
      if (text) void navigator.clipboard?.writeText(text).catch(() => {})
      ev.preventDefault()
      return false
    }
    if (!isMac() && ev.ctrlKey && ev.shiftKey && key === 'v') {
      ev.preventDefault()
      void navigator.clipboard
        ?.readText()
        .then((t) => t && term.paste(t))
        .catch(() => {})
      return false
    }
    return true
  }
}

/** Sends keystrokes in order: one write in flight per terminal; input typed meanwhile goes in the next one. */
function inputWriter(id: string): (data: string) => void {
  let queued = ''
  let sending = false
  const send = () => {
    if (sending || !queued) return
    const data = queued
    queued = ''
    sending = true
    void api.terminals
      .write(id, data)
      .catch(() => {})
      .finally(() => {
        sending = false
        send()
      })
  }
  return (data) => {
    queued += data
    send()
  }
}

/** The xterm for a backend terminal, created on first use. `scrollback` seeds a re-attached terminal. */
export function getTerm(id: string, mode: 'dark' | 'light', scrollback?: string): TermEntry {
  ensureListeners()
  const existing = entries.get(id)
  if (existing) return existing
  disposed.delete(id)
  const host = document.createElement('div')
  host.className = 'h-full w-full'
  const term = new Terminal({
    fontFamily: '"IBM Plex Mono", ui-monospace, Consolas, monospace',
    fontSize: terminalFontSize(),
    lineHeight: 1.15,
    cursorBlink: true,
    scrollback: 5000,
    allowProposedApi: true,
    macOptionIsMeta: true,
    theme: terminalTheme(mode)
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
  term.loadAddon(
    new WebLinksAddon((_e, uri) => {
      void import('@tauri-apps/plugin-opener').then((m) => m.openUrl(uri)).catch(() => window.open(uri, '_blank'))
    })
  )
  term.attachCustomKeyEventHandler(keyHandler(term))
  const send = inputWriter(id)
  let replaying = false
  term.onData((data) => {
    if (!replaying) send(data)
  })
  term.open(host)
  const entry: TermEntry = { term, fit, host, cols: term.cols, rows: term.rows }
  entries.set(id, entry)
  // A re-attached terminal starts from the backend's scrollback, which already includes anything buffered.
  // xterm answers queries (cursor position, device attributes) as it parses; in old output those replies are
  // stale and would reach the CLI as keystrokes, so input is dropped until the replay is parsed.
  if (scrollback !== undefined) {
    pending.delete(id)
    if (scrollback) {
      replaying = true
      term.write(scrollback, () => (replaying = false))
    }
  } else {
    for (const chunk of pending.get(id) ?? []) term.write(chunk)
    pending.delete(id)
  }
  return entry
}

const canvases = (e: TermEntry) => [...(e.term.element?.querySelectorAll('canvas') ?? [])]

function releaseWebgl(e: TermEntry) {
  const { webgl: addon, webglCanvas: canvas } = e
  e.webgl = e.webglCanvas = undefined
  addon?.dispose()
  // Disposing only removes the canvas; its context would count against the browser's limit until garbage
  // collection and could push out a live terminal's. Lose it now, after dispose so the addon no longer listens.
  canvas?.getContext('webgl2')?.getExtension('WEBGL_lose_context')?.loseContext()
}

/** Draw a terminal that a pane just put on screen with WebGL when a context is available. */
export function acquireWebgl(id: string) {
  const e = entries.get(id)
  if (!e || e.webgl || e.noWebgl) return
  const live = [...entries.values()].filter((x) => x.webgl)
  // At the cap, an off-screen terminal hands over its context, but only once the new one loaded (briefly one
  // over the cap, still far below the browser's limit).
  const offscreen = live.length >= MAX_WEBGL ? live.find((x) => !x.host.isConnected) : undefined
  if (live.length >= MAX_WEBGL && !offscreen) return
  let addon: WebglAddon | undefined
  const before = new Set(canvases(e))
  try {
    addon = new WebglAddon()
    addon.onContextLoss(() => {
      if (e.lostWebgl) e.noWebgl = true
      e.lostWebgl = true
      if (e.webgl !== addon) return
      releaseWebgl(e)
      // WebGL rounds cells to whole device pixels, so the DOM renderer fits fewer columns in the same pane.
      fitTerm(id)
    })
    e.term.loadAddon(addon)
    e.webgl = addon
    e.webglCanvas = canvases(e).find((c) => !before.has(c) && c.getContext('webgl2'))
  } catch {
    e.noWebgl = true
    try {
      addon?.dispose()
    } catch {
      /* activation failed partway; disposing only unregisters it from the terminal */
    }
    // A renderer that failed after adding its canvas cannot remove it; it would cover the DOM renderer.
    for (const c of canvases(e)) if (!before.has(c)) c.remove()
  }
  if (offscreen && e.webgl) releaseWebgl(offscreen)
}

/** Fit to the host's size and tell the backend when the character grid changed. */
export function fitTerm(id: string) {
  const e = entries.get(id)
  if (!e || !e.host.isConnected || e.host.clientWidth < 20 || e.host.clientHeight < 20) return
  try {
    e.fit.fit()
  } catch {
    return
  }
  if (e.term.cols !== e.cols || e.term.rows !== e.rows) {
    e.cols = e.term.cols
    e.rows = e.term.rows
    void api.terminals.resize(id, e.cols, e.rows).catch(() => {})
  }
}

/** Character grid that fits an element, for sizing a terminal before it is spawned. */
export function estimateSize(el: HTMLElement | null): { cols: number; rows: number } {
  const size = terminalFontSize()
  const w = el?.clientWidth ?? 0
  const h = el?.clientHeight ?? 0
  if (w < 100 || h < 60) return { cols: 100, rows: 30 }
  return { cols: Math.max(20, Math.floor((w - 16) / (size * 0.6))), rows: Math.max(5, Math.floor((h - 8) / (size * 1.15 * 1.2))) }
}

export function disposeTerm(id: string) {
  const e = entries.get(id)
  pending.delete(id)
  disposed.add(id)
  if (!e) return
  entries.delete(id)
  releaseWebgl(e)
  e.term.dispose()
  e.host.remove()
}

/** Re-theme every terminal (theme switch) and apply the font size setting. */
export function restyleAll(mode: 'dark' | 'light') {
  const size = terminalFontSize()
  for (const [id, e] of entries) {
    e.term.options.theme = terminalTheme(mode)
    if (e.term.options.fontSize !== size) {
      e.term.options.fontSize = size
      fitTerm(id)
    }
  }
}

export function focusTerm(id: string | undefined) {
  if (id) entries.get(id)?.term.focus()
}
