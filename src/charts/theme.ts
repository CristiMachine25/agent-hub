import { useMemo } from 'react'
import { useStore } from '@/store'

/** Resolved chart colors for the current theme (SVG attributes can't use CSS variables reliably). */
export function useChartColors() {
  const theme = useStore((s) => s.resolvedTheme)
  return useMemo(() => {
    const cs = getComputedStyle(document.documentElement)
    const c = (name: string) => `rgb(${cs.getPropertyValue(`--${name}`).trim().split(/\s+/).join(',')})`
    return { theme, grid: c('rivet'), axis: c('muted'), ink: c('ink'), surface: c('deck'), ground: c('ground'), hop: c('hop') }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [theme])
}

export const axisTick = (fill: string) => ({ fill, fontSize: 11, fontFamily: 'inherit' })

/** Date and hour in chart tooltips: "Oct 21, 3 PM". */
export const tipTime = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', hour: 'numeric' })
