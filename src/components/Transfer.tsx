import { useEffect, useState } from 'react'
import clsx from 'clsx'
import { KeyRound, Loader2, ShieldCheck } from 'lucide-react'
import type { ImportPreview, ImportReport } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META } from '@/lib/providers'
import { formatAgo, plural } from '@/lib/format'
import { AccountDot, Dialog, ProviderMark } from './ui'

const OS_NAMES: Record<string, string> = { windows: 'Windows', macos: 'macOS', linux: 'Linux' }
const KIND_NAMES: Record<string, string> = { skills: 'Skills', agents: 'Agents and commands', instructions: 'Instructions' }
/** The kind comes from the import file, so an unknown one is shown as it is instead of crashing the dialog. */
export const kindName = (kind: string) => (KIND_NAMES[kind] ?? kind).toLowerCase()

function Check({ checked, onChange, disabled, children }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean; children: React.ReactNode }) {
  return (
    <label className={clsx('flex items-start gap-2.5 py-1', disabled && 'opacity-60')}>
      <input type="checkbox" className="mt-1" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="min-w-0 flex-1">{children}</span>
    </label>
  )
}

/** Write the setup to one file to open on another computer. */
export function ExportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const accounts = useStore((s) => s.accounts)
  const toast = useStore((s) => s.toast)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [files, setFiles] = useState(true)
  const [settings, setSettings] = useState(true)
  const [signIns, setSignIns] = useState(false)
  const [pass, setPass] = useState('')
  const [pass2, setPass2] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    // Read the accounts once: depending on them would reset the choices on every accounts-changed.
    setPicked(new Set(useStore.getState().accounts.map((a) => a.id)))
    setSignIns(false)
    setPass('')
    setPass2('')
    setError(null)
  }, [open])

  const passProblem = !signIns ? null : pass.length < 8 ? 'At least 8 characters.' : pass !== pass2 ? "The two passphrases don't match." : null

  const run = async () => {
    setError(null)
    const date = new Date().toISOString().slice(0, 10)
    const path = await api.system.pickSavePath(`agent-hub-setup-${date}.agenthub`)
    if (!path) return
    setBusy(true)
    try {
      const r = await api.transfer.export(path, {
        accountIds: [...picked],
        includeFiles: files,
        includeSettings: settings,
        passphrase: signIns ? pass : undefined
      })
      toast({
        kind: 'success',
        title: `Exported ${plural(r.accounts, 'account')}`,
        body: [r.signIns ? `${plural(r.signIns, 'sign-in')}, encrypted` : 'No sign-ins', r.files ? plural(r.files, 'file') : ''].filter(Boolean).join(', ') + '.',
        command: path
      })
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Export your setup"
      width={560}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={busy || picked.size === 0 || !!passProblem} onClick={run}>
            {busy && <Loader2 size={14} className="animate-spin" />}
            Export…
          </button>
        </>
      }
    >
      <p className="mb-3 text-muted">
        Saves one file you can open in Agent Hub on another computer (Windows, macOS or Linux). Importing it only adds what's missing there.
      </p>
      <fieldset className="mb-3">
        <legend className="mb-1 font-medium">Accounts</legend>
        <ul className="max-h-48 overflow-y-auto rounded border border-rivet px-2.5 py-1">
          {accounts.map((a) => (
            <li key={a.id}>
              <Check
                checked={picked.has(a.id)}
                onChange={(v) => {
                  const next = new Set(picked)
                  if (v) next.add(a.id)
                  else next.delete(a.id)
                  setPicked(next)
                }}
              >
                <span className="flex items-center gap-2">
                  <AccountDot account={a} size={8} />
                  <ProviderMark id={a.providerId} size={13} />
                  <span className="truncate">{a.label}</span>
                  <span className="ml-auto shrink-0 text-xs text-faint">{a.plan ?? PROVIDER_META[a.providerId].short}</span>
                </span>
              </Check>
            </li>
          ))}
        </ul>
      </fieldset>
      <Check checked={files} onChange={setFiles}>
        <span className="block">Skills, agents and instructions</span>
        <span className="block text-xs text-muted">From each tool's shared knowledge (see Knowledge). Files the other computer already has stay as they are.</span>
      </Check>
      <Check checked={settings} onChange={setSettings}>
        <span className="block">Preferences</span>
        <span className="block text-xs text-muted">Theme, notifications, how often usage is checked. Not folders or the terminal program.</span>
      </Check>
      <Check checked={signIns} onChange={setSignIns}>
        <span className="block">Sign-ins, so the accounts are ready to use there</span>
        <span className="block text-xs text-muted">Encrypted with a passphrase you choose. You'll need it when importing.</span>
      </Check>
      {signIns && (
        <div className="mt-2 space-y-2 rounded border border-rivet bg-ground p-3">
          <div className="grid grid-cols-2 gap-2">
            <input type="password" placeholder="Passphrase" value={pass} onChange={(e) => setPass(e.target.value)} aria-label="Passphrase" autoComplete="new-password" />
            <input type="password" placeholder="Repeat it" value={pass2} onChange={(e) => setPass2(e.target.value)} aria-label="Repeat passphrase" autoComplete="new-password" />
          </div>
          {passProblem && pass && <p className="text-xs text-low">{passProblem}</p>}
          <p className="flex items-start gap-1.5 text-xs text-muted">
            <ShieldCheck size={13} className="mt-px shrink-0" />
            Both computers then share each login. When one renews it, the other may need to sign in again for that account.
          </p>
        </div>
      )}
      {error && (
        <p className="mt-3 text-empty" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  )
}

/** Open a setup file, choose what to bring in, and add it without changing anything already here. */
export function ImportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useStore((s) => s.toast)
  const [path, setPath] = useState<string | null>(null)
  const [preview, setPreview] = useState<ImportPreview | null>(null)
  const [pass, setPass] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [signIns, setSignIns] = useState(true)
  const [files, setFiles] = useState(true)
  const [settings, setSettings] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<ImportReport | null>(null)

  const load = async (p: string, passphrase?: string) => {
    setBusy(true)
    setError(null)
    try {
      const pv = await api.transfer.preview(p, passphrase)
      setPreview(pv)
      setPicked((prev) => (prev.size ? prev : new Set(pv.accounts.filter((a) => !a.alreadyHere).map((a) => a.key))))
      setFiles(pv.kinds.some((k) => k.newFiles > 0))
      if (passphrase && !pv.signInsUnlocked) setError("That passphrase doesn't open the sign-ins.")
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    if (!open) return
    setPath(null)
    setPreview(null)
    setReport(null)
    setPass('')
    setPicked(new Set())
    setError(null)
    void (async () => {
      let p: string | null
      try {
        p = await api.system.pickSetupFile()
      } catch (e) {
        return setError(errorText(e))
      }
      if (!p) return onClose()
      setPath(p)
      await load(p)
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const locked = !!preview?.hasSignIns && !preview.signInsUnlocked
  const run = async () => {
    if (!path || !preview) return
    setBusy(true)
    setError(null)
    try {
      const r = await api.transfer.import(path, preview.signInsUnlocked ? pass : undefined, {
        accountKeys: [...picked],
        signIns: signIns && preview.signInsUnlocked,
        files,
        settings
      })
      setReport(r)
      toast({ kind: 'success', title: r.added.length ? `Added ${plural(r.added.length, 'account')}` : 'Nothing new to add' })
      // New accounts arrive through the accounts-changed event; preferences need a fresh read.
      if (r.settings) {
        try {
          useStore.setState({ settings: await api.settings.get() })
        } catch {
          /* the import itself worked; don't report it as failed */
        }
      }
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const footer = report ? (
    <button className="btn btn-primary" onClick={onClose}>
      Done
    </button>
  ) : (
    <>
      <button className="btn" onClick={onClose}>
        Cancel
      </button>
      <button className="btn btn-primary" disabled={busy || !preview || (picked.size === 0 && !files && !settings)} onClick={run}>
        {busy && <Loader2 size={14} className="animate-spin" />}
        Import
      </button>
    </>
  )

  return (
    <Dialog open={open} onClose={onClose} title="Import a setup" width={600} footer={preview || report ? footer : undefined}>
      {report ? (
        <ImportResult report={report} />
      ) : !preview ? (
        <div className="flex items-center gap-2 py-6 text-muted" role="status">
          {busy ? <Loader2 size={16} className="animate-spin" /> : null}
          {error ?? (busy ? 'Reading the file…' : 'Choose an .agenthub file…')}
        </div>
      ) : (
        <div className="space-y-3">
          <p className="text-muted">
            Exported from {OS_NAMES[preview.fromOs] ?? preview.fromOs} {formatAgo(preview.exportedAt)}. Importing only adds: nothing
            on this computer is replaced or removed.
          </p>
          {locked && (
            <form
              className="flex items-center gap-2 rounded border border-rivet bg-ground p-2.5"
              onSubmit={(e) => {
                e.preventDefault()
                if (path) void load(path, pass)
              }}
            >
              <KeyRound size={15} className="shrink-0 text-muted" />
              <input
                type="password"
                className="min-w-0 flex-1"
                placeholder="Passphrase for the sign-ins"
                value={pass}
                onChange={(e) => setPass(e.target.value)}
                aria-label="Passphrase"
                autoFocus
              />
              <button className="btn btn-sm" type="submit" disabled={!pass || busy}>
                Unlock
              </button>
            </form>
          )}
          <fieldset>
            <legend className="mb-1 font-medium">Accounts</legend>
            <ul className="max-h-56 overflow-y-auto rounded border border-rivet px-2.5 py-1">
              {preview.accounts.map((a) => (
                <li key={a.key}>
                  <Check
                    checked={!a.alreadyHere && picked.has(a.key)}
                    disabled={!!a.alreadyHere}
                    onChange={(v) => {
                      const next = new Set(picked)
                      if (v) next.add(a.key)
                      else next.delete(a.key)
                      setPicked(next)
                    }}
                  >
                    <span className="flex items-center gap-2">
                      <ProviderMark id={a.providerId} size={13} />
                      <span className="truncate">{a.label}</span>
                      <span className="ml-auto shrink-0 text-xs text-faint">
                        {a.alreadyHere ? `Already here${a.alreadyHere !== a.label ? ` as ${a.alreadyHere}` : ''}` : [a.plan, a.hasSignIn ? 'with sign-in' : 'sign in after'].filter(Boolean).join(', ')}
                      </span>
                    </span>
                    {!a.alreadyHere && <span className="code block truncate text-[11px] text-faint">{a.targetHome}</span>}
                  </Check>
                </li>
              ))}
            </ul>
          </fieldset>
          {preview.hasSignIns && (
            <Check checked={signIns && !locked} disabled={locked} onChange={setSignIns}>
              <span className="block">Add the sign-ins</span>
              <span className="block text-xs text-muted">
                {locked ? 'Enter the passphrase above to use them.' : "Only where this computer has no sign-in for that folder yet."}
              </span>
            </Check>
          )}
          {preview.kinds.length > 0 && (
            <Check checked={files} onChange={setFiles}>
              <span className="block">Skills, agents and instructions</span>
              <span className="block text-xs text-muted">
                {preview.kinds
                  .map((k) => `${PROVIDER_META[k.providerId].short} ${kindName(k.kind)}: ${k.newFiles} new of ${k.files}`)
                  .join(' · ')}
                . Files already here are kept.
              </span>
            </Check>
          )}
          {preview.hasSettings && (
            <Check checked={settings} onChange={setSettings}>
              <span className="block">Use the exported preferences</span>
              <span className="block text-xs text-muted">Theme, notifications and how often usage is checked.</span>
            </Check>
          )}
          {error && (
            <p className="text-empty" role="alert">
              {error}
            </p>
          )}
        </div>
      )}
    </Dialog>
  )
}

function ImportResult({ report: r }: { report: ImportReport }) {
  return (
    <div className="space-y-2">
      {r.added.length > 0 && (
        <p>
          <span className="font-semibold">Added:</span> {r.added.join(', ')}.
        </p>
      )}
      {r.skipped.length > 0 && <p className="text-muted">Left alone: {r.skipped.join(', ')}.</p>}
      <p className="text-muted">
        {[
          r.signIns ? `${plural(r.signIns, 'sign-in')} added` : null,
          r.filesAdded || r.filesKept ? `${plural(r.filesAdded, 'file')} added, ${r.filesKept} already here kept` : null,
          r.settings ? 'preferences applied' : null
        ]
          .filter(Boolean)
          .join(', ') || 'No files or preferences changed.'}
      </p>
      {r.problems.map((p) => (
        <p key={p} className="text-low">
          {p}
        </p>
      ))}
    </div>
  )
}
