import type { InstalledSkill, ProviderId } from '@shared/types'

export interface FeaturedSkill {
  /** "owner/repo/skill" on skills.sh */
  id: string
  /** One line, from the skill's own SKILL.md description. */
  blurb: string
}

/** A hand-picked start: widely installed skills that work in any of the tools. */
export const FEATURED: { title: string; skills: FeaturedSkill[] }[] = [
  {
    title: 'Start here',
    skills: [
      { id: 'vercel-labs/skills/find-skills', blurb: 'Finds and installs the right skill when you ask how to do something.' },
      { id: 'anthropics/skills/skill-creator', blurb: 'Create, improve and benchmark your own skills.' }
    ]
  },
  {
    title: 'Documents',
    skills: [
      { id: 'anthropics/skills/pdf', blurb: 'Read, merge, split, fill and OCR PDF files.' },
      { id: 'anthropics/skills/docx', blurb: 'Create and edit Word documents with real formatting.' },
      { id: 'anthropics/skills/xlsx', blurb: 'Open, clean, chart and build spreadsheets.' },
      { id: 'anthropics/skills/pptx', blurb: 'Build, read and edit slide decks.' },
      { id: 'anthropics/skills/doc-coauthoring', blurb: 'A structured way to write specs, proposals and docs together.' }
    ]
  },
  {
    title: 'Frontend and design',
    skills: [
      { id: 'anthropics/skills/frontend-design', blurb: 'Distinctive, intentional visual design for new or reshaped UI.' },
      { id: 'vercel-labs/agent-skills/vercel-react-best-practices', blurb: 'React and Next.js performance rules from Vercel engineering.' },
      { id: 'vercel-labs/agent-skills/web-design-guidelines', blurb: 'Reviews UI code for accessibility and interface guidelines.' },
      { id: 'vercel-labs/agent-skills/vercel-composition-patterns', blurb: 'React composition patterns that scale, React 19 included.' },
      { id: 'anthropics/skills/canvas-design', blurb: 'Posters and static art, saved as PNG or PDF.' }
    ]
  },
  {
    title: 'Planning and review',
    skills: [
      { id: 'obra/superpowers/brainstorming', blurb: 'Turns an idea into an agreed design before any code is written.' },
      { id: 'obra/superpowers/writing-plans', blurb: 'Writes a step-by-step plan from a spec before touching code.' },
      { id: 'mattpocock/skills/grill-me', blurb: 'A relentless interview that sharpens a plan or design.' },
      { id: 'mattpocock/skills/code-review', blurb: "Reviews a branch against the repo's standards and the spec." },
      { id: 'mattpocock/skills/handoff', blurb: 'Compacts a session into a handoff document for the next agent.' }
    ]
  },
  {
    title: 'Testing and debugging',
    skills: [
      { id: 'mattpocock/skills/tdd', blurb: 'Builds features and fixes bugs test-first, red-green-refactor.' },
      { id: 'obra/superpowers/systematic-debugging', blurb: 'Finds the root cause of a bug before proposing a fix.' },
      { id: 'anthropics/skills/webapp-testing', blurb: 'Tests local web apps with Playwright, screenshots and logs.' },
      { id: 'vercel-labs/agent-browser/agent-browser', blurb: 'Drives a browser: navigate, click, fill forms, take screenshots.' }
    ]
  },
  {
    title: 'Building with',
    skills: [
      { id: 'anthropics/skills/mcp-builder', blurb: 'Builds MCP servers in Python or TypeScript.' },
      { id: 'supabase/agent-skills/supabase-postgres-best-practices', blurb: 'Postgres schema, RLS, index and migration rules from Supabase.' },
      { id: 'remotion-dev/skills/remotion-best-practices', blurb: 'Makes videos in React with Remotion.' }
    ]
  }
]

export function featuredFor(id: string): FeaturedSkill | undefined {
  for (const g of FEATURED) {
    const hit = g.skills.find((s) => s.id === id)
    if (hit) return hit
  }
  return undefined
}

/** The folder a skill installs into: the last part of its id. */
export function folderOf(id: string): string {
  return id.slice(id.lastIndexOf('/') + 1)
}

/**
 * Where a skill stands in one tool's shared folder: installed by Relay from this id, a
 * different skill already using its folder name (copied from the main folder, added by hand or
 * from another repo), or not there.
 */
export function installState(id: string, providerId: ProviderId, installed: InstalledSkill[]): 'installed' | 'taken' | 'absent' {
  const folder = folderOf(id)
  const there = installed.find((s) => s.providerId === providerId && s.name === folder)
  if (!there) return 'absent'
  return there.id === id ? 'installed' : 'taken'
}

export const skillPageUrl = (id: string) => `https://skills.sh/${id}`
export const repoUrl = (source: string) => `https://github.com/${source}`
