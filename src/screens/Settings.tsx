import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { Download, FolderOpen, ShieldCheck, Upload } from 'lucide-react'
import type { AppSettings } from '@shared/types'
import { api } from '@/api'
import { useStore } from '@/store'
import { SHARING_ITEMS } from '@/lib/providers'
import { Segmented, Switch } from '@/components/ui'
import { ExportDialog, ImportDialog } from '@/components/Transfer'
import { Page } from '@/components/Page'
import { FONT_SIZE_KEY, FONT_SIZES, restyleAll, terminalFontSize } from '@/terminals/registry'

const TERMINALS: Record<typeof api.platform, { id: string; label: string }[]> = {
  win32: [
    { id: 'auto', label: 'Automatic (Windows Terminal if installed)' },
    { id: 'wt', label: 'Windows Terminal' },
    { id: 'powershell', label: 'PowerShell' },
    { id: 'cmd', label: 'Command Prompt' }
  ],
  darwin: [
    { id: 'auto', label: 'Automatic (Terminal)' },
    { id: 'terminal', label: 'Terminal' },
    { id: 'iterm', label: 'iTerm2' },
    { id: 'ghostty', label: 'Ghostty' },
    { id: 'kitty', label: 'kitty' },
    { id: 'alacritty', label: 'Alacritty' },
    { id: 'wezterm', label: 'WezTerm' }
  ],
  linux: [
    { id: 'auto', label: 'Automatic (your default terminal)' },
    { id: 'gnome-terminal', label: 'GNOME Terminal' },
    { id: 'konsole', label: 'Konsole' },
    { id: 'kitty', label: 'kitty' },
    { id: 'alacritty', label: 'Alacritty' },
    { id: 'wezterm', label: 'WezTerm' },
    { id: 'ghostty', label: 'Ghostty' },
    { id: 'xterm', label: 'xterm' }
  ]
}

export function Settings() {
  const s = useStore((st) => st.settings)
  const save = useStore((st) => st.saveSettings)
  if (!s) return null
  const n = s.notifications
  const setN = (patch: Partial<AppSettings['notifications']>) => save({ notifications: { ...n, ...patch } })

  return (
    <Page title="Settings">
      <div className="max-w-[760px] space-y-8">
        <Group title="Usage">
          <Row
            label="Check usage live"
            help={
              <>
                Asks each provider's own usage endpoint with that account's saved sign-in. Tokens never leave your computer except to the provider they
                belong to. Turn this off to use only local session logs, which is less accurate for limits.
              </>
            }
          >
            {(ids) => <Switch {...ids} checked={s.liveUsage} onChange={(v) => save({ liveUsage: v })} />}
          </Row>
          <Row label="Check every" help="How often limits are refreshed in the background.">
            {(ids) => (
              <select {...ids} value={s.pollIntervalMinutes} onChange={(e) => save({ pollIntervalMinutes: Number(e.target.value) })}>
                {[1, 2, 5, 10, 15, 30].map((m) => (
                  <option key={m} value={m}>
                    {m === 1 ? '1 minute' : `${m} minutes`}
                  </option>
                ))}
              </select>
            )}
          </Row>
          {s.liveUsage && (
            <p className="flex items-start gap-2 text-xs text-muted">
              <ShieldCheck size={14} className="mt-px shrink-0" /> No telemetry and no Relay server. Everything is stored in this computer's app data folder.
            </p>
          )}
        </Group>

        <Group title="Notifications">
          <Row label="Show notifications" help="System notifications, plus the list in Notifications.">
            {(ids) => <Switch {...ids} checked={n.enabled} onChange={(v) => setN({ enabled: v })} />}
          </Row>
          <Row label="Warn when a limit is nearly used" help={`When an account has used ${n.nearlyExhaustedPercent}% of any limit.`}>
            {(ids) => (
              <Slider {...ids} value={n.nearlyExhaustedPercent} min={50} max={95} step={5} unit="% used" onChange={(v) => setN({ nearlyExhaustedPercent: v })} disabled={!n.enabled} />
            )}
          </Row>
          <Row
            label="Use it or lose it"
            help={`When a weekly or monthly limit resets within ${n.useItOrLoseItHours} hours and at least ${n.useItOrLoseItMinLeftPercent}% is still unused. Claude, Codex and Copilot have such limits; Gemini's daily quotas get no reminder.`}
          >
            {(ids) => (
              <div role="group" {...ids} className="flex flex-col items-end gap-2">
                <Slider value={n.useItOrLoseItHours} min={6} max={72} step={6} unit=" hours before" onChange={(v) => setN({ useItOrLoseItHours: v })} disabled={!n.enabled} label="Hours before reset" />
                <Slider value={n.useItOrLoseItMinLeftPercent} min={10} max={80} step={10} unit="% unused" onChange={(v) => setN({ useItOrLoseItMinLeftPercent: v })} disabled={!n.enabled} label="Minimum unused" />
              </div>
            )}
          </Row>
          <Row label="Tell me when a limit resets" help="Only for limits that you had used up or nearly used up.">
            {(ids) => <Switch {...ids} checked={n.onReset} onChange={(v) => setN({ onReset: v })} disabled={!n.enabled} />}
          </Row>
        </Group>

        <Group title="Launching">
          <Row label="Terminal" help="Where CLIs open when you launch an account.">
            {(ids) => (
              <select {...ids} value={s.terminal} onChange={(e) => save({ terminal: e.target.value })} className="max-w-[300px]">
                {(TERMINALS[api.platform] ?? TERMINALS.linux).map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.label}
                  </option>
                ))}
              </select>
            )}
          </Row>
          <Row label="Start in folder" help="The working directory for launches, unless you pick one.">
            {(ids) => (
              <div role="group" {...ids} className="flex min-w-0 items-center gap-2">
                <code className="code max-w-[260px] truncate text-muted" data-tip={s.defaultCwd}>
                  {s.defaultCwd ?? 'Your home folder'}
                </code>
                <button
                  className="btn btn-sm"
                  onClick={async () => {
                    const d = await api.system.pickDirectory()
                    if (d) save({ defaultCwd: d })
                  }}
                >
                  <FolderOpen size={13} /> Choose
                </button>
              </div>
            )}
          </Row>
        </Group>

        <Group title="New accounts share by default">
          {SHARING_ITEMS.map((it) => (
            <Row key={it.key} label={it.label} help={it.explain}>
              {/* The name keeps "Share … by default"; the row label alone ("Skills") would not say what the switch does. */}
              {(ids) => (
                <Switch
                  checked={s.defaultSharing[it.key]}
                  onChange={(v) => save({ defaultSharing: { ...s.defaultSharing, [it.key]: v } })}
                  label={`Share ${it.label} by default`}
                  aria-describedby={ids['aria-describedby']}
                />
              )}
            </Row>
          ))}
        </Group>

        <Group title="App">
          <Row label="Theme">
            {(ids) => (
              <Segmented
                aria-labelledby={ids['aria-labelledby']}
                size="sm"
                value={s.theme}
                onChange={(v) => save({ theme: v })}
                options={[
                  { id: 'dark', label: 'Dark' },
                  { id: 'light', label: 'Light' },
                  { id: 'system', label: 'System' }
                ]}
              />
            )}
          </Row>
          <Row label="Keep running in the tray" help="Closing the window keeps Relay in the tray so it can warn you about limits.">
            {(ids) => <Switch {...ids} checked={s.minimizeToTray} onChange={(v) => save({ minimizeToTray: v })} />}
          </Row>
          <Row label="Start when you sign in to your computer" help="Relay starts hidden in the tray, ready to warn you about limits.">
            {(ids) => <Switch {...ids} checked={s.launchAtLogin} onChange={(v) => save({ launchAtLogin: v })} />}
          </Row>
          <Row label="Terminal font size" help="Text size in the terminal grid. Stored on this computer only.">
            {(ids) => <FontSize {...ids} />}
          </Row>
        </Group>

        <Group title="Move to another computer">
          <Transfer />
        </Group>
      </div>
    </Page>
  )
}

/** Export the setup to a file, or bring one in from another computer. */
function Transfer() {
  const [dialog, setDialog] = useState<'export' | 'import' | null>(null)
  return (
    <>
      <Row
        label="Export your setup"
        help="Accounts, plans and colors, and optionally skills, preferences and encrypted sign-ins, in one file for Relay on Windows, macOS or Linux."
      >
        {(ids) => (
          <button className="btn btn-sm" onClick={() => setDialog('export')} aria-describedby={ids['aria-describedby']}>
            <Upload size={14} /> Export…
          </button>
        )}
      </Row>
      <Row label="Import a setup" help="Choose what to add from a file. Accounts, files and sign-ins already on this computer are never replaced or removed.">
        {(ids) => (
          <button className="btn btn-sm" onClick={() => setDialog('import')} aria-describedby={ids['aria-describedby']}>
            <Download size={14} /> Import…
          </button>
        )}
      </Row>
      <ExportDialog open={dialog === 'export'} onClose={() => setDialog(null)} />
      <ImportDialog open={dialog === 'import'} onClose={() => setDialog(null)} />
    </>
  )
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2 className="mb-1 font-serif text-[17px] font-semibold">{title}</h2>
      <div className="divide-y divide-rivet border-y border-rivet">{children}</div>
    </section>
  )
}

/** What ties a row's control to the row: its label names the control and its help describes it. */
type RowIds = { 'aria-labelledby': string; 'aria-describedby'?: string }

function Row({ label, help, children }: { label: string; help?: ReactNode; children: (ids: RowIds) => ReactNode }) {
  const id = useId()
  const ids: RowIds = { 'aria-labelledby': `${id}label`, 'aria-describedby': help ? `${id}help` : undefined }
  return (
    <div className="flex items-center justify-between gap-6 py-3">
      <div className="min-w-0 max-w-[440px]">
        <div id={ids['aria-labelledby']} className="font-medium">
          {label}
        </div>
        {help && (
          <div id={ids['aria-describedby']} className="text-xs text-muted">
            {help}
          </div>
        )}
      </div>
      <div className="shrink-0">{children(ids)}</div>
    </div>
  )
}

function Slider({
  value,
  min,
  max,
  step,
  unit,
  onChange,
  disabled,
  label,
  ...ids
}: Partial<RowIds> & {
  value: number
  min: number
  max: number
  step: number
  unit: string
  onChange: (v: number) => void
  disabled?: boolean
  label?: string
}) {
  // Show the value while dragging; save once the slider has been still for 400 ms.
  const [local, setLocal] = useState(value)
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const pending = useRef<() => void>(undefined)
  useEffect(() => setLocal(value), [value])
  // Leaving the screen within the 400 ms saves the value now instead of dropping it.
  useEffect(
    () => () => {
      clearTimeout(timer.current)
      pending.current?.()
    },
    []
  )
  return (
    <label className="flex items-center gap-3">
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={local}
        disabled={disabled}
        aria-label={label}
        {...ids}
        onChange={(e) => {
          const v = Number(e.target.value)
          setLocal(v)
          clearTimeout(timer.current)
          pending.current = () => {
            pending.current = undefined
            onChange(v)
          }
          timer.current = setTimeout(pending.current, 400)
        }}
        className="w-36"
      />
      <span className="w-28 text-right text-xs tabular-nums text-muted">
        {local}
        {unit}
      </span>
    </label>
  )
}

function FontSize(ids: RowIds) {
  const [size, setSize] = useState(terminalFontSize)
  const theme = useStore((st) => st.resolvedTheme)
  return (
    <select
      {...ids}
      value={size}
      onChange={(e) => {
        const v = Number(e.target.value)
        setSize(v)
        try {
          localStorage.setItem(FONT_SIZE_KEY, String(v))
        } catch {
          /* not persisted; still applies for this session */
        }
        restyleAll(theme)
      }}
    >
      {FONT_SIZES.map((n) => (
        <option key={n} value={n}>
          {n} px
        </option>
      ))}
    </select>
  )
}
