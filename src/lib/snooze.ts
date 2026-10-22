import { create } from 'zustand'
import type { Account } from '@shared/types'

/** Sign-in reminders put off with "Not now": account id -> the status the account had then. */
export type Snoozed = Record<string, Account['status']>

const KEY = 'switchyard.snoozedLogins'

/** The snoozes that still hold: the account is there and its status hasn't changed since. */
export function liveSnoozes(snoozed: Snoozed, accounts: Pick<Account, 'id' | 'status'>[]): Snoozed {
  const out: Snoozed = {}
  for (const a of accounts) if (snoozed[a.id] === a.status) out[a.id] = a.status
  return out
}

function load(): Snoozed {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return v && typeof v === 'object' && !Array.isArray(v) ? v : {}
  } catch {
    return {}
  }
}

function persist(snoozed: Snoozed) {
  try {
    localStorage.setItem(KEY, JSON.stringify(snoozed))
  } catch {
    /* kept for this run only */
  }
}

interface SnoozeState {
  snoozed: Snoozed
  snooze(a: Pick<Account, 'id' | 'status'>): void
  /** Forget snoozes whose account changed status, so the reminder comes back next time. */
  sync(accounts: Pick<Account, 'id' | 'status'>[]): void
}

export const useSnoozes = create<SnoozeState>((set, get) => ({
  snoozed: load(),
  snooze: (a) => {
    const snoozed = { ...get().snoozed, [a.id]: a.status }
    set({ snoozed })
    persist(snoozed)
  },
  sync: (accounts) => {
    const snoozed = liveSnoozes(get().snoozed, accounts)
    if (Object.keys(snoozed).length === Object.keys(get().snoozed).length) return
    set({ snoozed })
    persist(snoozed)
  }
}))
