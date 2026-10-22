// Pure helpers for the terminal grid: layout geometry, focus navigation, workspace persistence.
import type { Account, Recommendation, TerminalInfo, UsageSnapshot } from '@shared/types'
import type { Platform } from './platform'
import { isMainFolder } from './profiles'
import { bindingWindow, leftOf } from './quota'

export type LayoutId = 1 | 2 | 3 | 4 | 6 | 9
export const LAYOUT_IDS: LayoutId[] = [1, 2, 3, 4, 6, 9]

/** A pane's place in the CSS grid, 0-based. */
export interface Rect {
  col: number
  row: number
  colSpan: number
  rowSpan: number
}

export interface Layout {
  id: LayoutId
  cols: number
  rows: number
  rects: Rect[]
  label: string
}

function uniform(cols: number, rows: number): Rect[] {
  const out: Rect[] = []
  for (let row = 0; row < rows; row++) for (let col = 0; col < cols; col++) out.push({ col, row, colSpan: 1, rowSpan: 1 })
  return out
}

export const LAYOUTS: Record<LayoutId, Layout> = {
  1: { id: 1, cols: 1, rows: 1, rects: uniform(1, 1), label: 'One pane' },
  2: { id: 2, cols: 2, rows: 1, rects: uniform(2, 1), label: 'Two side by side' },
  // One tall pane on the left, two stacked on the right: three columns of ~40 characters would be too narrow.
  3: {
    id: 3,
    cols: 2,
    rows: 2,
    rects: [
      { col: 0, row: 0, colSpan: 1, rowSpan: 2 },
      { col: 1, row: 0, colSpan: 1, rowSpan: 1 },
      { col: 1, row: 1, colSpan: 1, rowSpan: 1 }
    ],
    label: 'One large, two stacked'
  },
  4: { id: 4, cols: 2, rows: 2, rects: uniform(2, 2), label: 'Two by two' },
  6: { id: 6, cols: 3, rows: 2, rects: uniform(3, 2), label: 'Three by two' },
  9: { id: 9, cols: 3, rows: 3, rects: uniform(3, 3), label: 'Three by three' }
}

function isLayoutId(n: unknown): n is LayoutId {
  return typeof n === 'number' && (LAYOUT_IDS as number[]).includes(n)
}

/** Smallest layout with room for `count` panes (9 at most). */
export function layoutFor(count: number): LayoutId {
  return LAYOUT_IDS.find((id) => id >= count) ?? 9
}

export type Dir = 'left' | 'right' | 'up' | 'down'

/**
 * The pane you land on when moving focus from `from` in a direction. Picks the nearest pane on that side
 * that overlaps the current one on the other axis; ties go to the one whose center lines up best.
 * Returns `from` when there is nothing in that direction (no wrap-around).
 */
export function neighbor(rects: Rect[], from: number, dir: Dir): number {
  const cur = rects[from]
  if (!cur) return from
  const horizontal = dir === 'left' || dir === 'right'
  const span = (r: Rect): [number, number] => (horizontal ? [r.row, r.row + r.rowSpan] : [r.col, r.col + r.colSpan])
  const center = (r: Rect) => (horizontal ? r.row + r.rowSpan / 2 : r.col + r.colSpan / 2)
  const [a0, a1] = span(cur)
  let best = from
  let bestKey: [number, number] | null = null
  rects.forEach((r, i) => {
    if (i === from) return
    const [b0, b1] = span(r)
    if (b1 <= a0 || b0 >= a1) return // no overlap on the other axis
    let gap: number
    if (dir === 'right') gap = r.col - (cur.col + cur.colSpan)
    else if (dir === 'left') gap = cur.col - (r.col + r.colSpan)
    else if (dir === 'down') gap = r.row - (cur.row + cur.rowSpan)
    else gap = cur.row - (r.row + r.rowSpan)
    if (gap < 0) return
    const key: [number, number] = [gap, Math.abs(center(r) - center(cur))]
    if (!bestKey || key[0] < bestKey[0] || (key[0] === bestKey[0] && key[1] < bestKey[1])) {
      best = i
      bestKey = key
    }
  })
  return best
}

// ---------- Workspace ----------

export type PaneTarget = 'cli' | 'login'

export interface PaneConfig {
  uid: string
  accountId?: string
  target: PaneTarget
  cwd?: string
  resume?: boolean
  /** The conversation the last start asked to reopen, so trying again reopens the same one. */
  resumeSession?: string
  /** Backend terminal this pane shows; kept so a UI reload re-attaches instead of spawning again. */
  terminalId?: string
}

export interface Workspace {
  version: 1
  layout: LayoutId
  panes: PaneConfig[]
  focused: number
  maximized: number | null
}

let uidSeq = 0
function newUid(): string {
  uidSeq += 1
  return `p${Date.now().toString(36)}${uidSeq.toString(36)}`
}

export function emptyPane(uid: string = newUid()): PaneConfig {
  return { uid, target: 'cli' }
}

export function defaultWorkspace(makeUid: () => string = newUid): Workspace {
  return { version: 1, layout: 1, panes: [emptyPane(makeUid())], focused: 0, maximized: null }
}

/**
 * Resize the pane list to `count`. When shrinking, empty panes go first, then panes with nothing running,
 * then the last ones. Returns the panes that were dropped so the caller can close their terminals.
 */
export function fitPanes(
  panes: PaneConfig[],
  count: number,
  makeUid: () => string = newUid
): { panes: PaneConfig[]; removed: PaneConfig[] } {
  const out = [...panes]
  const removed: PaneConfig[] = []
  const lastIndex = (pred: (p: PaneConfig) => boolean) => {
    for (let i = out.length - 1; i >= 0; i--) if (pred(out[i]!)) return i
    return -1
  }
  while (out.length > count) {
    let i = lastIndex((p) => !p.accountId)
    if (i < 0) i = lastIndex((p) => !p.terminalId)
    if (i < 0) i = out.length - 1
    removed.push(...out.splice(i, 1))
  }
  while (out.length < count) out.push(emptyPane(makeUid()))
  return { panes: out, removed }
}

/** What goes to localStorage: only the stable parts of each pane. */
export function serializeWorkspace(ws: Workspace): string {
  return JSON.stringify({
    version: 1,
    layout: ws.layout,
    focused: ws.focused,
    maximized: ws.maximized,
    panes: ws.panes.map((p) => {
      const o: PaneConfig = { uid: p.uid, target: p.target }
      if (p.accountId) o.accountId = p.accountId
      if (p.cwd) o.cwd = p.cwd
      if (p.resume) o.resume = true
      if (p.terminalId) o.terminalId = p.terminalId
      return o
    })
  })
}

const str = (v: unknown) => (typeof v === 'string' && v.length > 0 ? v : undefined)

/** Read a stored workspace defensively: anything malformed falls back to sensible defaults. */
export function parseWorkspace(raw: string | null | undefined, makeUid: () => string = newUid): Workspace {
  let data: unknown
  try {
    data = raw ? JSON.parse(raw) : null
  } catch {
    data = null
  }
  if (!data || typeof data !== 'object') return defaultWorkspace(makeUid)
  const d = data as Record<string, unknown>
  const layout: LayoutId = isLayoutId(d.layout) ? d.layout : 1
  const seen = new Set<string>()
  const list = Array.isArray(d.panes) ? d.panes : []
  const parsed: PaneConfig[] = list.map((x) => {
    const p = (x && typeof x === 'object' ? x : {}) as Record<string, unknown>
    let uid = str(p.uid) ?? makeUid()
    if (seen.has(uid)) uid = makeUid()
    seen.add(uid)
    const pane: PaneConfig = { uid, target: p.target === 'login' ? 'login' : 'cli' }
    if (str(p.accountId)) pane.accountId = str(p.accountId)
    if (str(p.cwd)) pane.cwd = str(p.cwd)
    if (p.resume === true) pane.resume = true
    if (str(p.terminalId)) pane.terminalId = str(p.terminalId)
    return pane
  })
  const { panes } = fitPanes(parsed, layout, makeUid)
  const clampIdx = (v: unknown) => (typeof v === 'number' && Number.isInteger(v) && v >= 0 && v < panes.length ? v : null)
  return { version: 1, layout, panes, focused: clampIdx(d.focused) ?? 0, maximized: clampIdx(d.maximized) }
}

/**
 * Match a restored workspace with the terminals the backend still has. Panes keep their terminal when it
 * still exists (running or exited, so its output and exit code show); otherwise the pane keeps its account
 * and folder but no terminal, so nothing is spawned twice. Running terminals no pane knows about are placed
 * into empty panes, growing the layout when needed.
 */
export function reattach(ws: Workspace, running: TerminalInfo[], makeUid: () => string = newUid): Workspace {
  const byId = new Map(running.map((t) => [t.id, t]))
  const used = new Set<string>()
  let panes = ws.panes.map((p): PaneConfig => {
    if (p.terminalId && byId.has(p.terminalId) && !used.has(p.terminalId)) {
      used.add(p.terminalId)
      return p
    }
    if (!p.terminalId) return p
    const { terminalId: _gone, ...rest } = p
    return rest
  })
  const orphans = running.filter((t) => !t.exited && !used.has(t.id))
  let layout = ws.layout
  if (orphans.length) {
    const empties = panes.filter((p) => !p.accountId).length
    const need = panes.length + Math.max(0, orphans.length - empties)
    layout = layoutFor(Math.max(need, ws.layout))
    panes = fitPanes(panes, layout, makeUid).panes
    for (const t of orphans) {
      const i = panes.findIndex((p) => !p.accountId)
      if (i < 0) break
      panes[i] = { uid: panes[i]!.uid, target: 'cli', accountId: t.accountId, cwd: t.cwd, terminalId: t.id }
    }
  }
  const clamp = (n: number | null) => (n !== null && n < panes.length ? n : null)
  return { ...ws, layout, panes, focused: clamp(ws.focused) ?? 0, maximized: clamp(ws.maximized) }
}

// ---------- Accounts in panes ----------

/** The account has used up its binding window (Claude's 5-hour session, Codex primary...). */
export function atLimit(snap: UsageSnapshot | undefined): boolean {
  const b = bindingWindow(snap)
  return !!b && b.usedPercent >= 99.5
}

/** Nothing works for this account until the user signs in again. An expired login isn't signed
 *  out: it renews by itself, and starting the CLI renews it too. */
export function isSignedOut(a: Pick<Account, 'status'>): boolean {
  return a.status === 'needs-login'
}

/** Signed in, but the plan doesn't include the CLI (e.g. a canceled subscription). Signing in again won't fix it. */
export function isBlocked(a: Pick<Account, 'status'>): boolean {
  return a.status === 'blocked'
}

/** Can start or continue a session now: signed in, not blocked, not at its limit. */
export function isUsable(a: Account, usage: Record<string, UsageSnapshot>): boolean {
  return !isSignedOut(a) && !isBlocked(a) && !atLimit(usage[a.id])
}

/** Short status for an account that is signed out, renewing its login or blocked; null when it is fine. */
export function statusLabel(a: Pick<Account, 'status'>): string | null {
  if (isBlocked(a)) return 'No access'
  return a.status === 'needs-login' ? 'Signed out' : a.status === 'expired' ? 'Login expired' : null
}

/**
 * Other accounts of the same provider to continue on, best first: recommendation order, then quota left.
 * Accounts at their limit, signed out or blocked sort last (the UI shows them disabled).
 */
export function continueCandidates(
  from: Pick<Account, 'id' | 'providerId'>,
  accounts: Account[],
  usage: Record<string, UsageSnapshot>,
  recs: Recommendation[]
): Account[] {
  const score = new Map(recs.map((r) => [r.accountId, r.score]))
  const left = (a: Account) => {
    const b = bindingWindow(usage[a.id])
    return b ? leftOf(b) : -1
  }
  const usable = (a: Account) => isUsable(a, usage)
  return accounts
    .filter((a) => a.providerId === from.providerId && a.id !== from.id)
    .sort((x, y) => {
      const ux = usable(x) ? 1 : 0
      const uy = usable(y) ? 1 : 0
      if (ux !== uy) return uy - ux
      const sx = score.get(x.id) ?? -1
      const sy = score.get(y.id) ?? -1
      if (sx !== sy) return sy - sx
      return left(y) - left(x)
    })
}

type Shares = Pick<Account, 'homeDir' | 'isDefaultHome' | 'sharing'>

/** True when chats started on one account can be resumed from the other (history shared on both sides).
 *  Both are accounts of the provider whose default home is `defaultHome`. */
export function historyShared(a: Shares, b: Shares, defaultHome: string | undefined, platform: Platform): boolean {
  // The provider's main folder is the source that others link to, so it always counts as sharing.
  const shares = (x: Shares) => isMainFolder(x, defaultHome, platform) || x.sharing.history
  return shares(a) && shares(b)
}

/** The accounts "Share history" has to turn history on for: the main folder already is the source. */
export function historyToTurnOn<T extends Shares>(accounts: T[], defaultHome: string | undefined, platform: Platform): T[] {
  return accounts.filter((x) => !isMainFolder(x, defaultHome, platform) && !x.sharing.history)
}

/** Account picker order: provider groups by their best account, accounts by recommendation within a group. */
export function pickerGroups(accounts: Account[], recs: Recommendation[]): { providerId: Account['providerId']; accounts: Account[] }[] {
  const score = new Map(recs.map((r) => [r.accountId, r.score]))
  const s = (a: Account) => (isBlocked(a) ? -3 : isSignedOut(a) ? -2 : (score.get(a.id) ?? -1))
  const groups = new Map<Account['providerId'], Account[]>()
  for (const a of accounts) {
    const g = groups.get(a.providerId) ?? []
    g.push(a)
    groups.set(a.providerId, g)
  }
  return [...groups.entries()]
    .map(([providerId, list]) => ({ providerId, accounts: [...list].sort((x, y) => s(y) - s(x)) }))
    .sort((x, y) => s(y.accounts[0]!) - s(x.accounts[0]!)) // every group has an account
}
