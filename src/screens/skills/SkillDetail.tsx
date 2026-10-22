import { useEffect, useId, useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, Check, ExternalLink, FileText, Loader2, ShieldAlert, Trash2 } from 'lucide-react'
import type { InstalledSkill, ProviderId, SkillPreview } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { PROVIDER_META } from '@/lib/providers'
import { formatTokens } from '@/lib/format'
import { folderOf, installState, repoUrl, skillPageUrl } from '@/lib/skills'
import { ProviderMark, Segmented } from '@/components/ui'
import { Markdown } from './Markdown'
import { openUrl, toolList, type Listing } from './shared'

/** What the last install did, shown under the picker until the next one. */
type Outcome = { ok: ProviderId[]; failed: { providerId: ProviderId; error: string }[] } | { error: string }

/** The reading pane: a header that stays put with the install action, and SKILL.md below it. Key it by skill id. */
export function SkillDetail({ listing, installed, tools, onChanged }: { listing: Listing; installed: InstalledSkill[]; tools: ProviderId[]; onChanged: () => Promise<void> }) {
  const toast = useStore((s) => s.toast)
  const uid = useId()
  const [preview, setPreview] = useState<SkillPreview | null>(null)
  const [previewError, setPreviewError] = useState<string>()
  const [attempt, setAttempt] = useState(0)
  const absent = tools.filter((p) => installState(listing.id, p, installed) === 'absent')
  const [picked, setPicked] = useState<Set<ProviderId>>(() => new Set(absent))
  const [busy, setBusy] = useState<'install' | ProviderId | null>(null)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const [view, setView] = useState<'formatted' | 'source'>('formatted')

  useEffect(() => {
    let live = true
    setPreviewError(undefined)
    api.skills
      .preview(listing.id)
      .then((p) => live && setPreview(p))
      .catch((e) => live && setPreviewError(errorText(e)))
    return () => {
      live = false
    }
  }, [listing.id, attempt])

  const chosen = absent.filter((p) => picked.has(p))
  const install = async () => {
    setBusy('install')
    setOutcome(null)
    try {
      const outcomes = await api.skills.install(listing.id, chosen)
      const ok = outcomes.filter((o) => !o.error).map((o) => o.providerId)
      const failed = outcomes.filter((o) => o.error).map((o) => ({ providerId: o.providerId, error: o.error! }))
      if (ok.length) toast({ kind: 'success', title: `Installed ${listing.name} for ${toolList.format(ok.map((p) => PROVIDER_META[p].short))}`, body: 'Accounts that link skills from the shared folder have it now.' })
      for (const f of failed) toast({ kind: 'error', title: `Couldn't install ${listing.name} for ${PROVIDER_META[f.providerId].short}`, body: f.error })
      setOutcome({ ok, failed })
      await onChanged()
    } catch (e) {
      toast({ kind: 'error', title: `Couldn't install ${listing.name}`, body: errorText(e) })
      setOutcome({ error: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  const remove = async (p: ProviderId) => {
    setBusy(p)
    setOutcome(null)
    try {
      await api.skills.remove(listing.id, p)
      toast({ kind: 'success', title: `Removed ${listing.name} from ${PROVIDER_META[p].short}` })
      await onChanged()
    } catch (e) {
      toast({ kind: 'error', title: `Couldn't remove ${listing.name}`, body: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  const everywhere = tools.length > 0 && absent.length === 0 && tools.some((p) => installState(listing.id, p, installed) === 'installed')
  const label =
    busy === 'install'
      ? `Installing for ${chosen.length === 1 ? PROVIDER_META[chosen[0]!].short : `${chosen.length} tools`}…`
      : chosen.length
        ? `Install for ${chosen.length === 1 ? PROVIDER_META[chosen[0]!].short : `${chosen.length} tools`}`
        : everywhere
          ? 'Installed'
          : 'Install'
  const description = preview?.description || listing.blurb

  return (
    <article aria-labelledby={`${uid}-name`} className="flex min-h-0 flex-1 flex-col">
      <header className="shrink-0 border-b border-rivet bg-ground px-6 pb-4 pt-5">
        <div className="flex flex-wrap items-start gap-x-4 gap-y-3">
          <div className="min-w-0 flex-1">
            <h2 id={`${uid}-name`} className="break-words font-serif text-lg font-semibold">
              {listing.name}
            </h2>
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
              <button className="inline-flex items-center gap-1 hover:text-ink" onClick={() => openUrl(repoUrl(listing.source))} data-tip={repoUrl(listing.source)}>
                {listing.source} <ExternalLink size={11} aria-hidden />
              </button>
              <button className="inline-flex items-center gap-1 hover:text-ink" onClick={() => openUrl(skillPageUrl(listing.id))} data-tip={skillPageUrl(listing.id)}>
                skills.sh <ExternalLink size={11} aria-hidden />
              </button>
              {listing.installs ? <span className="tabular-nums text-faint">{formatTokens(listing.installs)} installs</span> : null}
              {preview && (
                <span className="text-faint">
                  {preview.files} {preview.files === 1 ? 'file' : 'files'}
                </span>
              )}
            </div>
          </div>
          {tools.length > 0 && (
            <button className={clsx('btn shrink-0', !everywhere && 'btn-primary')} onClick={install} disabled={!chosen.length || busy !== null}>
              {busy === 'install' ? <Loader2 size={14} className="animate-spin" /> : everywhere && <Check size={14} />}
              {label}
            </button>
          )}
        </div>

        {tools.length > 0 ? (
          <fieldset className="mt-4">
            <legend className="mb-2 text-xs font-medium text-muted">Install for</legend>
            <div className="flex flex-wrap gap-2">
              {tools.map((p) => (
                <ToolChip
                  key={p}
                  id={`${uid}-${p}`}
                  p={p}
                  name={listing.name}
                  state={installState(listing.id, p, installed)}
                  picked={picked.has(p)}
                  installing={busy === 'install' && chosen.includes(p)}
                  removing={busy === p}
                  disabled={busy !== null}
                  onPick={(on) => {
                    const next = new Set(picked)
                    if (on) next.add(p)
                    else next.delete(p)
                    setPicked(next)
                  }}
                  onRemove={() => remove(p)}
                />
              ))}
            </div>
          </fieldset>
        ) : (
          <p className="mt-3 text-muted">Add an account to install skills. Each tool has a shared folder that its accounts link skills from.</p>
        )}

        <div role="status" className="empty:hidden mt-3 text-sm">
          {outcome && 'error' in outcome && (
            <span className="flex items-start gap-1.5">
              <AlertTriangle size={14} className="mt-0.5 shrink-0 text-muted" aria-hidden /> Couldn't install {listing.name}: {outcome.error}
            </span>
          )}
          {outcome && 'ok' in outcome && outcome.ok.length > 0 && (
            <span className="flex items-start gap-1.5">
              <Check size={14} className="mt-0.5 shrink-0" aria-hidden /> Installed for {toolList.format(outcome.ok.map((p) => PROVIDER_META[p].short))}. Accounts that link skills from the shared folder have it now.
            </span>
          )}
          {outcome &&
            'failed' in outcome &&
            outcome.failed.map((f) => (
              <span key={f.providerId} className="mt-1 flex items-start gap-1.5">
                <AlertTriangle size={14} className="mt-0.5 shrink-0 text-muted" aria-hidden /> Couldn't install for {PROVIDER_META[f.providerId].short}: {f.error}
              </span>
            ))}
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="px-6 pb-12 pt-5">
          {description && <p className="max-w-[68ch] font-serif text-md">{description}</p>}
          <p className={clsx('flex max-w-[68ch] items-start gap-2 text-xs text-muted', description && 'mt-3')}>
            <ShieldAlert size={14} className="mt-px shrink-0" aria-hidden />
            Skills are instructions and scripts your agent follows. Read SKILL.md before you install one from a source you don't know.
          </p>

          <div className="mb-4 mt-6 flex max-w-[68ch] items-center justify-between gap-3 border-b border-rivet pb-2">
            <h3 className="flex items-center gap-1.5 text-xs font-semibold text-muted">
              <FileText size={13} aria-hidden /> SKILL.md
            </h3>
            {preview && (
              <Segmented
                size="sm"
                label="Show SKILL.md as"
                value={view}
                onChange={setView}
                options={[
                  { id: 'formatted', label: 'Formatted' },
                  { id: 'source', label: 'Source', title: 'Exactly what the agent reads' }
                ]}
              />
            )}
          </div>

          {previewError ? (
            <div className="flex max-w-[68ch] flex-col items-start gap-3 rounded border border-dashed border-rivet px-5 py-6">
              <p className="font-semibold">Couldn't read SKILL.md</p>
              <p className="text-muted">{previewError}</p>
              <div className="flex gap-2">
                <button className="btn btn-sm" onClick={() => setAttempt((n) => n + 1)}>
                  Try again
                </button>
                <button className="btn btn-sm btn-ghost" onClick={() => openUrl(skillPageUrl(listing.id))}>
                  <ExternalLink size={13} aria-hidden /> Read it on skills.sh
                </button>
              </div>
            </div>
          ) : !preview ? (
            <div role="status" aria-label="Reading SKILL.md" className="flex max-w-[68ch] flex-col gap-3">
              <span className="flex items-center gap-2 text-muted">
                <Loader2 size={14} className="animate-spin" aria-hidden /> Reading SKILL.md…
              </span>
              {[92, 100, 76, 0, 60, 96, 84].map((w, i) => (w ? <span key={i} className="h-3 rounded bg-raised" style={{ width: `${w}%` }} /> : <span key={i} className="h-2" />))}
            </div>
          ) : view === 'formatted' ? (
            <Markdown source={preview.body} />
          ) : (
            <pre className="code max-w-[80ch] whitespace-pre-wrap break-words rounded border border-rivet bg-well px-4 py-3 text-ink">{preview.body}</pre>
          )}
        </div>
      </div>
    </article>
  )
}

function ToolChip({
  id,
  p,
  name,
  state,
  picked,
  installing,
  removing,
  disabled,
  onPick,
  onRemove
}: {
  id: string
  p: ProviderId
  name: string
  state: 'installed' | 'taken' | 'absent'
  picked: boolean
  installing: boolean
  removing: boolean
  disabled: boolean
  onPick: (on: boolean) => void
  onRemove: () => void
}) {
  const meta = PROVIDER_META[p]
  if (state === 'installed')
    return (
      <span className="inline-flex h-8 items-center gap-2 rounded border border-rivet bg-raised pl-2.5 pr-1">
        <ProviderMark id={p} size={14} />
        <span className="font-medium">{meta.product}</span>
        <span id={id} className="inline-flex items-center gap-1 text-xs text-muted">
          <Check size={12} aria-hidden /> Installed
        </span>
        <button
          type="button"
          className="ml-0.5 flex h-6 w-6 items-center justify-center rounded text-muted hover:bg-deck hover:text-empty disabled:opacity-50"
          onClick={onRemove}
          disabled={disabled}
          aria-label={`Remove ${name} from ${meta.product}`}
          data-tip={`Remove from ${meta.product}`}
        >
          {removing ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
        </button>
      </span>
    )
  if (state === 'taken')
    return (
      <span className="inline-flex h-8 items-center gap-2 rounded border border-dashed border-rivet px-2.5 text-muted" data-tip={`A different skill named ${folderOf(name)} is already in ${meta.product}'s shared folder, so it won't be overwritten`}>
        <span className="flex opacity-60">
          <ProviderMark id={p} size={14} />
        </span>
        <span>{meta.product}</span>
        <span id={id} className="text-xs">
          Name in use
        </span>
      </span>
    )
  return (
    <label
      className={clsx(
        'inline-flex h-8 items-center gap-2 rounded border px-2.5 transition-colors has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-hop',
        picked ? 'border-hop/70 bg-hop/10' : 'border-rivet hover:bg-raised',
        disabled && 'opacity-60'
      )}
    >
      <input type="checkbox" className="focus-visible:outline-none" checked={picked} disabled={disabled} onChange={(e) => onPick(e.target.checked)} />
      <ProviderMark id={p} size={14} />
      <span className="font-medium">{meta.product}</span>
      {installing && <Loader2 size={13} className="animate-spin text-muted" aria-label="Installing" />}
    </label>
  )
}
