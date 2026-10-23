import { describe, expect, it, vi } from 'vitest'

vi.mock('@/api', () => ({ api: {}, errorText: String }))
vi.mock('@/terminals/store', () => ({ useGrid: {} }))
const { filterItems } = await import('./CommandPalette')

const rows = [
  { title: 'Launch Claude as Work', keywords: 'launch claude work', group: 'Launch' },
  { title: 'Show details for Work', keywords: 'show details account settings sharing Work', group: 'More actions' },
  { title: 'Go to Settings', keywords: 'settings preferences', group: 'App' }
]

describe('filterItems', () => {
  it('puts rows whose title matches above rows that only match a keyword', () => {
    expect(filterItems(rows, 'settings').map((r) => r.title)).toEqual(['Go to Settings', 'Show details for Work'])
  })

  it('hides the extra actions until something is typed', () => {
    expect(filterItems(rows, '').map((r) => r.title)).toEqual(['Launch Claude as Work', 'Go to Settings'])
  })
})
