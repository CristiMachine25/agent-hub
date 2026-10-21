import { describe, expect, it } from 'vitest'
import type { InstalledSkill } from '@shared/types'
import { FEATURED, featuredFor, folderOf, installState, repoUrl, skillPageUrl } from './skills'

const installed: InstalledSkill[] = [
  { providerId: 'claude', name: 'pdf', id: 'anthropics/skills/pdf' },
  { providerId: 'claude', name: 'tdd', id: null },
  { providerId: 'codex', name: 'pdf', id: 'someone/fork/pdf' }
]

describe('installState', () => {
  it('knows our copy, a folder of the same name, and nothing', () => {
    expect(installState('anthropics/skills/pdf', 'claude', installed)).toBe('installed')
    expect(installState('mattpocock/skills/tdd', 'claude', installed)).toBe('taken')
    expect(installState('anthropics/skills/pdf', 'codex', installed)).toBe('taken')
    expect(installState('anthropics/skills/pdf', 'gemini', installed)).toBe('absent')
    expect(installState('anthropics/skills/docx', 'claude', installed)).toBe('absent')
  })
})

describe('the featured catalog', () => {
  it('lists installable GitHub skills once each', () => {
    const ids = FEATURED.flatMap((g) => g.skills.map((s) => s.id))
    expect(new Set(ids).size).toBe(ids.length)
    // Same rule as SkillRef::parse in skills.rs.
    for (const id of ids) expect(id).toMatch(/^[A-Za-z0-9-]{1,39}\/[A-Za-z0-9._-]{1,100}\/[a-z0-9]+(-[a-z0-9]+)*$/)
    for (const g of FEATURED) for (const s of g.skills) expect(s.blurb.length).toBeGreaterThan(10)
  })

  it('finds a featured entry by id', () => {
    expect(featuredFor('anthropics/skills/pdf')?.blurb).toMatch(/PDF/)
    expect(featuredFor('nobody/none/nothing')).toBeUndefined()
  })
})

describe('links', () => {
  it('points at skills.sh and GitHub', () => {
    expect(folderOf('vercel-labs/agent-skills/vercel-react-best-practices')).toBe('vercel-react-best-practices')
    expect(skillPageUrl('anthropics/skills/pdf')).toBe('https://skills.sh/anthropics/skills/pdf')
    expect(repoUrl('anthropics/skills')).toBe('https://github.com/anthropics/skills')
  })
})
