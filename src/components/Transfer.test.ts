import { describe, expect, it } from 'vitest'
import { kindName } from './Transfer'

describe('kindName', () => {
  it('names known kinds and shows an unknown kind from an import file as is', () => {
    expect(kindName('agents')).toBe('agents and commands')
    expect(kindName('plugins')).toBe('plugins')
  })
})
