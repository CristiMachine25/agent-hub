const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

const monthDay = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric' })
const timeOfDay = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })

function parse(iso: string | null | undefined): number | null {
  if (!iso) return null
  const t = Date.parse(iso)
  return Number.isNaN(t) ? null : t
}

/** Compact duration until `iso`: "14m", "2h 14m", "3d 4h". Null when unknown, "now" when due. */
export function formatCountdown(iso: string | null | undefined, now: number = Date.now()): string | null {
  const t = parse(iso)
  if (t === null) return null
  const ms = t - now
  if (ms <= 30_000) return 'now'
  if (ms < HOUR) return `${Math.max(1, Math.round(ms / MIN))}m`
  if (ms < DAY) {
    const h = Math.floor(ms / HOUR)
    const m = Math.floor((ms % HOUR) / MIN)
    return m ? `${h}h ${m}m` : `${h}h`
  }
  const d = Math.floor(ms / DAY)
  const h = Math.floor((ms % DAY) / HOUR)
  return h ? `${d}d ${h}h` : `${d}d`
}

/** "Resets in 2h 14m", "Resets Nov 1" (more than a week out), "Reset due", or null when unknown. */
export function resetText(iso: string | null | undefined, now: number = Date.now()): string | null {
  const t = parse(iso)
  if (t === null) return null
  if (t - now > 7 * DAY) return `Resets ${monthDay.format(t)}`
  const c = formatCountdown(iso, now)
  if (c === 'now') return 'Reset due'
  return `Resets in ${c}`
}

/** "just now", "5m ago", "3h ago", "yesterday", "4 days ago", "Oct 12". */
export function formatAgo(iso: string | null | undefined, now: number = Date.now()): string {
  const t = parse(iso)
  if (t === null) return 'never'
  const ms = now - t
  if (ms < MIN) return 'just now'
  if (ms < HOUR) return `${Math.floor(ms / MIN)}m ago`
  if (ms < DAY) return `${Math.floor(ms / HOUR)}h ago`
  if (ms < 2 * DAY) return 'yesterday'
  if (ms < 7 * DAY) return `${Math.floor(ms / DAY)} days ago`
  return monthDay.format(t)
}

export function formatClock(iso: string): string {
  return timeOfDay.format(Date.parse(iso))
}

/** "Oct 12" from an ISO timestamp, a YYYY-MM-DD day, or epoch ms. */
export function formatDay(value: string | number): string {
  const t = typeof value === 'number' ? value : Date.parse(value.length === 10 ? value + 'T12:00:00' : value)
  return monthDay.format(t)
}

/** 950 -> "950", 1234 -> "1.2k", 3_400_000 -> "3.4M". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n)) return '0'
  const a = Math.abs(n)
  if (a >= 1e9) return `${trim(n / 1e9)}B`
  if (a >= 1e6) return `${trim(n / 1e6)}M`
  if (a >= 1e3) return `${trim(n / 1e3)}k`
  return String(Math.round(n))
}

function trim(x: number): string {
  return (Math.abs(x) >= 100 ? x.toFixed(0) : x.toFixed(1)).replace(/\.0$/, '')
}

export function formatMoney(n: number): string {
  return Number.isInteger(n) || n >= 100 ? `$${Math.round(n)}` : `$${n.toFixed(2)}`
}

export function plural(n: number, one: string, many = one + 's'): string {
  return `${n} ${n === 1 ? one : many}`
}

/** Percent, rounded, never shows "0%" for a tiny non-zero value. */
export function formatPct(n: number): string {
  if (n > 0 && n < 1) return '<1%'
  return `${Math.round(n)}%`
}

const PROPER = new Set(['Opus', 'Sonnet', 'Haiku', 'Gemini', 'Pro', 'Flash', 'Claude', 'Codex', 'Copilot', 'GPT'])

/** A window label for use mid-sentence: "Weekly Opus" -> "weekly Opus", "Gemini Pro daily" stays. */
export function windowName(label: string): string {
  return label
    .split(' ')
    .map((w) => (PROPER.has(w) || /[A-Z].*[A-Z]/.test(w) ? w : w.toLowerCase()))
    .join(' ')
}

/**
 * Plain words for UsageSnapshot.credits, whose meaning depends on the unit:
 * "credits left" (a balance), a currency code like "USD" (extra usage spent), or a countable unit with a limit.
 */
export function creditsText(c: { used: number; limit?: number | null; unit: string }): string {
  const n = (x: number) => x.toLocaleString('en-US', { maximumFractionDigits: 2 })
  if (/\bleft\b|remaining|balance/i.test(c.unit)) return `${n(c.used)} ${c.unit}`
  if (/^[A-Z]{3}$/.test(c.unit)) {
    const money = (x: number) => {
      try {
        return new Intl.NumberFormat('en-US', { style: 'currency', currency: c.unit }).format(x)
      } catch {
        return `${n(x)} ${c.unit}`
      }
    }
    return c.limit != null ? `${money(c.used)} of ${money(c.limit)} extra usage spent` : `${money(c.used)} of extra usage spent`
  }
  if (c.limit != null) return `${n(c.used)} of ${n(c.limit)} ${c.unit} used`
  return `${n(c.used)} ${c.unit} used`
}
