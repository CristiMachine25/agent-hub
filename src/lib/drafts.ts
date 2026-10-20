import { create } from 'zustand'
import type { ProviderId } from '@shared/types'
import { DEFAULT_PLAN } from './plans'

/** A half-filled "Create a new profile" form, kept when the dialog was dismissed by accident. */
export interface ProfileDraft {
  id: string
  savedAt: string
  providerId: ProviderId
  label: string
  plan: string
  cost: string
  costEdited: boolean
  preset: 'recommended' | 'separate' | 'everything'
  folder: string | null
}

export type DraftFields = Omit<ProfileDraft, 'id' | 'savedAt'>

const KEY = 'switchyard.profileDrafts'

function load(): ProfileDraft[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((d) => d && typeof d.id === 'string' && typeof d.providerId === 'string') : []
  } catch {
    return []
  }
}

function persist(drafts: ProfileDraft[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(drafts))
  } catch {
    /* kept for this run only */
  }
}

interface DraftState {
  drafts: ProfileDraft[]
  /** The draft the dialog should open with, if any. */
  resumeId: string | null
  save(fields: DraftFields, id?: string | null): void
  remove(id: string): void
  resume(id: string | null): void
}

export const useDrafts = create<DraftState>((set, get) => ({
  drafts: load(),
  resumeId: null,
  save(fields, id) {
    const draft: ProfileDraft = { ...fields, id: id ?? `d${Date.now().toString(36)}`, savedAt: new Date().toISOString() }
    const drafts = [draft, ...get().drafts.filter((d) => d.id !== draft.id)]
    persist(drafts)
    set({ drafts })
  },
  remove(id) {
    const drafts = get().drafts.filter((d) => d.id !== id)
    persist(drafts)
    set({ drafts, resumeId: get().resumeId === id ? null : get().resumeId })
  },
  resume(id) {
    set({ resumeId: id })
  }
}))

/** True when the form differs from a fresh one, so dismissing it would lose something. */
export function isWorthKeeping(f: DraftFields): boolean {
  return !!f.label.trim() || f.costEdited || f.preset !== 'recommended' || !!f.folder || f.providerId !== 'claude' || f.plan !== DEFAULT_PLAN[f.providerId]
}
