export interface Box {
  left: number
  top: number
  width: number
  height: number
}

export type TipSide = 'bottom' | 'top' | 'right' | 'left'

const GAP = 6
const MARGIN = 8

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)))

/**
 * Where a tooltip of `tip` size goes next to `anchor`, kept `MARGIN` px inside the viewport.
 * 'bottom' flips above when there is no room below; 'right' flips left when there is no room on the right.
 */
export function placeTip(
  anchor: Box,
  tip: { width: number; height: number },
  viewport: { width: number; height: number },
  want: 'bottom' | 'right' = 'bottom'
): { left: number; top: number; side: TipSide } {
  if (want === 'right') {
    let side: TipSide = 'right'
    let left = anchor.left + anchor.width + GAP
    if (left + tip.width > viewport.width - MARGIN && anchor.left - GAP - tip.width >= MARGIN) {
      side = 'left'
      left = anchor.left - GAP - tip.width
    }
    const top = clamp(anchor.top + (anchor.height - tip.height) / 2, MARGIN, viewport.height - MARGIN - tip.height)
    return { left: clamp(left, MARGIN, viewport.width - MARGIN - tip.width), top, side }
  }
  let side: TipSide = 'bottom'
  let top = anchor.top + anchor.height + GAP
  if (top + tip.height > viewport.height - MARGIN && anchor.top - GAP - tip.height >= MARGIN) {
    side = 'top'
    top = anchor.top - GAP - tip.height
  }
  const left = clamp(anchor.left + (anchor.width - tip.width) / 2, MARGIN, viewport.width - MARGIN - tip.width)
  return { left, top: clamp(top, MARGIN, viewport.height - MARGIN - tip.height), side }
}
