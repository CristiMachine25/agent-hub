import clsx from 'clsx'
import type { InstalledSkill, ProviderId } from '@shared/types'
import { PROVIDER_META } from '@/lib/providers'
import { formatTokens } from '@/lib/format'
import { folderOf, installState, installedSkills } from '@/lib/skills'
import { ProviderMark } from '@/components/ui'
import { fromId, type Listing } from './shared'

type State = ReturnType<typeof installState>

/** A titled run of rows in the list column. */
export function ListSection({ title, aside, children }: { title: string; aside?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section aria-label={title} className="pt-3 first:pt-1">
      <div className="flex items-baseline justify-between gap-3 px-3 pb-1">
        <h2 className="text-xs font-semibold text-muted">{title}</h2>
        {aside && <span className="text-xs text-faint">{aside}</span>}
      </div>
      <ul className="flex flex-col gap-px">{children}</ul>
    </section>
  )
}

export function SkillRows({
  items,
  installed,
  tools,
  openId,
  onOpen
}: {
  items: Listing[]
  installed: InstalledSkill[]
  tools: ProviderId[]
  openId?: string
  onOpen: (l: Listing) => void
}) {
  return (
    <>
      {items.map((it) => (
        <li key={it.id}>
          <SkillRow listing={it} current={openId === it.id} onOpen={onOpen}>
            <ToolMarks tools={tools} name={it.name} stateOf={(p) => installState(it.id, p, installed)} />
          </SkillRow>
        </li>
      ))}
    </>
  )
}

function SkillRow({ listing: it, current, onOpen, children }: { listing: Listing; current: boolean; onOpen: (l: Listing) => void; children: React.ReactNode }) {
  return (
    <button
      data-skill-row
      aria-current={current || undefined}
      onClick={() => onOpen(it)}
      className={clsx(
        'group flex w-full flex-col gap-0.5 rounded px-3 py-2.5 text-left focus-visible:outline-offset-[-2px]',
        current ? 'bg-raised shadow-[inset_2px_0_0_rgb(var(--hop))]' : 'hover:bg-raised/60'
      )}
    >
      <span className="flex w-full min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-semibold">{it.name}</span>
        {children}
      </span>
      {it.blurb && <span className="line-clamp-2 text-muted">{it.blurb}</span>}
      <span className="flex w-full min-w-0 items-baseline gap-3 text-xs text-faint">
        <span className="min-w-0 flex-1 truncate">{it.source}</span>
        {it.installs !== undefined && it.installs > 0 && <span className="shrink-0 tabular-nums">{formatTokens(it.installs)} installs</span>}
      </span>
    </button>
  )
}

/** The Installed view: every skill folder in the tools' shared folders, ours first by name. */
export function InstalledRows({ installed, tools, openId, onOpen }: { installed: InstalledSkill[]; tools: ProviderId[]; openId?: string; onOpen: (l: Listing) => void }) {
  return (
    <>
      {installedSkills(installed).map((s) => {
        const marks = <ToolMarks tools={tools} name={s.name} stateOf={(p) => (s.providers.includes(p) ? 'installed' : 'absent')} />
        return (
          <li key={s.key}>
            {s.id ? (
              <SkillRow listing={fromId(s.id)} current={openId === s.id} onOpen={onOpen}>
                {marks}
              </SkillRow>
            ) : (
              // Not from the directory, so there is no SKILL.md to fetch: a plain row, out of the arrow-key order.
              <div className="flex flex-col gap-0.5 px-3 py-2.5">
                <span className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-semibold">{s.name}</span>
                  {marks}
                </span>
                <span className="text-xs text-faint">Copied from the main folder or added by hand</span>
              </div>
            )}
          </li>
        )
      })}
    </>
  )
}

/** One mark per tool with accounts: in full colour where the skill is installed, faded where it isn't. */
export function ToolMarks({ tools, name, stateOf, size = 14 }: { tools: ProviderId[]; name: string; stateOf: (p: ProviderId) => State; size?: number }) {
  if (tools.length === 0) return null
  return (
    <span className="flex shrink-0 items-center gap-1.5">
      {tools.map((p) => {
        const state = stateOf(p)
        const tool = PROVIDER_META[p].product
        const title = state === 'installed' ? `Installed for ${tool}` : state === 'taken' ? `${tool} already has a different skill named ${folderOf(name)}` : `Not installed for ${tool}`
        return (
          <span key={p} role="img" data-tip={title} aria-label={title} className={clsx('relative flex', state === 'absent' && 'opacity-25 grayscale', state === 'taken' && 'opacity-60')}>
            <ProviderMark id={p} size={size} />
            {/* "Taken" is not "installed": a small notch tells them apart without relying on how faded the mark is. */}
            {state === 'taken' && <span className="absolute -bottom-0.5 -right-0.5 h-[6px] w-[6px] rounded-full bg-ink ring-[1.5px] ring-well" />}
          </span>
        )
      })}
    </span>
  )
}
