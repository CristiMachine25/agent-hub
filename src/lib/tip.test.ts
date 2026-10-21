import { describe, expect, it } from 'vitest'
import { placeTip } from './tip'

const VIEW = { width: 1000, height: 700 }
const TIP = { width: 120, height: 24 }

describe('placeTip', () => {
  it('centers below the anchor', () => {
    expect(placeTip({ left: 400, top: 100, width: 40, height: 28 }, TIP, VIEW)).toEqual({ left: 360, top: 134, side: 'bottom' })
  })

  it('flips above when there is no room below', () => {
    expect(placeTip({ left: 400, top: 660, width: 40, height: 28 }, TIP, VIEW)).toEqual({ left: 360, top: 630, side: 'top' })
  })

  it('stays 8px inside the left and right edges', () => {
    expect(placeTip({ left: 0, top: 100, width: 28, height: 28 }, TIP, VIEW).left).toBe(8)
    expect(placeTip({ left: 975, top: 100, width: 20, height: 28 }, TIP, VIEW).left).toBe(1000 - 8 - 120)
  })

  it('pins to the margin when the tip is wider than the viewport', () => {
    expect(placeTip({ left: 10, top: 10, width: 20, height: 20 }, { width: 400, height: 24 }, { width: 300, height: 700 }).left).toBe(8)
  })

  it('goes to the right, vertically centered', () => {
    expect(placeTip({ left: 10, top: 200, width: 40, height: 36 }, TIP, VIEW, 'right')).toEqual({ left: 56, top: 206, side: 'right' })
  })

  it('flips left when there is no room on the right', () => {
    expect(placeTip({ left: 940, top: 200, width: 40, height: 36 }, TIP, VIEW, 'right')).toEqual({ left: 814, top: 206, side: 'left' })
  })

  it('keeps a right-side tip inside the bottom edge', () => {
    expect(placeTip({ left: 10, top: 680, width: 40, height: 20 }, TIP, VIEW, 'right').top).toBe(700 - 8 - 24)
  })
})
