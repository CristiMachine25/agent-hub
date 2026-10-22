import { describe, expect, it } from 'vitest'
import type { Account } from '@shared/types'
import { liveSnoozes, useSnoozes } from './snooze'

const acc = (id: string, status: Account['status']) => ({ id, status })

describe('sign-in reminders put off with "Not now"', () => {
  it('hold only while the account keeps the status it had', () => {
    const snoozed = { a: 'needs-login', b: 'needs-login', gone: 'needs-login' } as const
    expect(liveSnoozes(snoozed, [acc('a', 'needs-login'), acc('b', 'ok')])).toEqual({ a: 'needs-login' })
  })

  it('come back after the account signs in and out again', () => {
    const { snooze, sync } = useSnoozes.getState()
    snooze(acc('a', 'needs-login'))
    sync([acc('a', 'needs-login')])
    expect(useSnoozes.getState().snoozed).toEqual({ a: 'needs-login' })
    sync([acc('a', 'ok')])
    sync([acc('a', 'needs-login')])
    expect(useSnoozes.getState().snoozed).toEqual({})
  })
})
