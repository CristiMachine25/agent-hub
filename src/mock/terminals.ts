// Fake embedded terminals for the browser preview: prints a banner, echoes what you type, and understands
// a few demo commands (/limit simulates this account hitting its limit, /exit ends the session).
import type { Account, AppEvents, AgentHubApi, ProviderInfo, TerminalInfo, UsageSnapshot } from '@shared/types'

interface Deps {
  account(id: string): Account | undefined
  provider(id: Account['providerId']): ProviderInfo | undefined
  defaultCwd(): string
  usage(): Record<string, UsageSnapshot>
  emit<K extends keyof AppEvents>(event: K, payload: AppEvents[K]): void
  notify(accountId: string, title: string, body: string): void
}

interface Fake {
  info: TerminalInfo
  line: string
  scrollback: string
  account: Account
}

const ESC = '\x1b['
const dim = (s: string) => `${ESC}2m${s}${ESC}0m`
const bold = (s: string) => `${ESC}1m${s}${ESC}0m`
const color = (hex: string, s: string) => {
  const n = parseInt(hex.slice(1), 16)
  return `${ESC}38;2;${(n >> 16) & 255};${(n >> 8) & 255};${n & 255}m${s}${ESC}0m`
}
const BRAND: Record<Account['providerId'], string> = { claude: '#D97757', codex: '#10A37F', gemini: '#4E86F7', copilot: '#8B6CEF' }

export function createMockTerminals(deps: Deps): AgentHubApi['terminals'] {
  const terms = new Map<string, Fake>()
  let seq = 0

  function out(t: Fake, data: string) {
    t.scrollback = (t.scrollback + data).slice(-256_000)
    deps.emit('terminal-output', { id: t.info.id, data })
  }

  function prompt(t: Fake) {
    out(t, `${color(BRAND[t.account.providerId], '>')} `)
  }

  function exit(t: Fake, code: number) {
    if (t.info.exited) return
    t.info.exited = true
    t.info.exitCode = code
    deps.emit('terminal-exit', { id: t.info.id, code })
  }

  function banner(t: Fake, resume: boolean, login: boolean) {
    const a = t.account
    const prov = deps.provider(a.providerId)
    const name = prov?.name ?? a.providerId
    const w = Math.min(64, Math.max(40, a.label.length + 30))
    const line = '─'.repeat(w)
    const c = (s: string) => color(BRAND[a.providerId], s)
    const pad = (s: string, raw: string) => s + ' '.repeat(Math.max(0, w - 2 - raw.length))
    const rows: [string, string][] = [
      [`${c('✻')} ${bold(name)}`, `✻ ${name}`],
      ['', ''],
      [`${dim('account')}  ${a.label}${a.plan ? dim(` · ${a.plan}`) : ''}`, `account  ${a.label}${a.plan ? ` · ${a.plan}` : ''}`],
      [`${dim('cwd')}      ${t.info.cwd}`, `cwd      ${t.info.cwd}`]
    ]
    let s = `${c('╭' + line + '╮')}\r\n`
    for (const [styled, raw] of rows) s += `${c('│')} ${pad(styled, raw.slice(0, w - 2))}${c('│')}\r\n`
    s += `${c('╰' + line + '╯')}\r\n\r\n`
    if (login) s += `Opening the browser to sign in as ${bold(a.label)}…\r\n${dim('(browser preview: nothing is really opened)')}\r\n\r\n`
    else if (resume) s += `${dim('Resuming the most recent conversation in this folder…')}\r\n\r\n`
    s += dim('Browser preview: this terminal only echoes what you type.') + '\r\n'
    s += dim('Try ') + '/limit' + dim(' to simulate a usage limit, or ') + '/exit' + dim('.') + '\r\n\r\n'
    out(t, s)
    prompt(t)
  }

  function runLine(t: Fake, line: string) {
    const cmd = line.trim()
    if (cmd === '/exit' || cmd === 'exit') {
      out(t, dim('Goodbye.') + '\r\n')
      setTimeout(() => exit(t, 0), 150)
      return
    }
    if (cmd === '/limit') {
      const snap = deps.usage()[t.account.id]
      const w = snap?.windows.find((x) => !/opus|sonnet|haiku/i.test(x.id)) ?? snap?.windows[0]
      if (w) {
        w.usedPercent = 100
        if (!w.resetsAt) w.resetsAt = new Date(Date.now() + 2 * 3600e3).toISOString()
        deps.emit('usage-updated', deps.usage())
        deps.notify(t.account.id, `${t.account.label} reached its ${w.label.toLowerCase()} limit`, 'Switch to another account to keep going.')
      }
      out(t, `${color('#F06868', '⎿')} You've hit your limit · resets in 2h\r\n\r\n`)
      prompt(t)
      return
    }
    if (cmd) out(t, `${dim('You said:')} ${cmd}\r\n\r\n`)
    prompt(t)
  }

  return {
    async spawn(req) {
      await new Promise((r) => setTimeout(r, 180))
      const a = deps.account(req.accountId)
      if (!a) throw new Error('That account no longer exists. Pick another one.')
      const prov = deps.provider(a.providerId)
      if (prov && !prov.cliInstalled) throw new Error(`${prov.name} isn't installed. Install it with "${prov.installHint}" and try again.`)
      const id = `term-${++seq}-${Date.now().toString(36)}`
      const cli = prov?.cliCommand ?? a.providerId
      const t: Fake = {
        account: a,
        line: '',
        scrollback: '',
        info: {
          id,
          accountId: a.id,
          title: a.label,
          commandLine: req.target === 'login' ? `${cli} /login` : req.resume ? `${cli} --continue` : cli,
          cwd: req.cwd ?? deps.defaultCwd(),
          pid: 4000 + seq,
          startedAt: new Date().toISOString(),
          exited: false,
          exitCode: null
        }
      }
      terms.set(id, t)
      setTimeout(() => banner(t, !!req.resume, req.target === 'login'), 60)
      return structuredClone(t.info)
    },
    async write(id, data) {
      const t = terms.get(id)
      if (!t || t.info.exited) return
      // Ignore escape sequences (arrow keys etc.); echo everything else like a cooked tty.
      const clean = data.replace(/\x1b\[[0-9;?]*[A-Za-z~]|\x1bO[A-Za-z]/g, '')
      let echo = ''
      for (const ch of clean) {
        if (ch === '\r' || ch === '\n') {
          out(t, echo + '\r\n')
          echo = ''
          const line = t.line
          t.line = ''
          runLine(t, line)
        } else if (ch === '\x7f' || ch === '\b') {
          if (t.line) {
            t.line = t.line.slice(0, -1)
            echo += '\b \b'
          }
        } else if (ch === '\x03') {
          t.line = ''
          out(t, echo + '^C\r\n')
          echo = ''
          prompt(t)
        } else if (ch >= ' ') {
          t.line += ch
          echo += ch
        }
      }
      if (echo) out(t, echo)
    },
    async resize() {},
    async kill(id) {
      const t = terms.get(id)
      if (t) exit(t, 1)
      terms.delete(id)
    },
    async list() {
      return [...terms.values()].map((t) => structuredClone(t.info))
    },
    async scrollback(id) {
      return terms.get(id)?.scrollback ?? ''
    },
    async session() {
      return null
    }
  }
}
