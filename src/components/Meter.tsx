import clsx from 'clsx'
import type { UsageWindow } from '@shared/types'
import { leftOf, levelFor, levelWord, litCells, type Level } from '@/lib/quota'
import { formatPct, resetText } from '@/lib/format'

const LEVEL_BG: Record<Level, string> = {
  full: 'bg-full',
  half: 'bg-half',
  low: 'bg-low',
  empty: 'bg-empty'
}
export const LEVEL_TEXT: Record<Level, string> = {
  full: 'text-full',
  half: 'text-half',
  low: 'text-low',
  empty: 'text-empty'
}

/** Segmented fuel strip: lit cells = quota left, color = how much is left. */
export function Meter({
  left,
  cells = 20,
  size = 'sm',
  label,
  className
}: {
  left: number
  cells?: number
  size?: 'xs' | 'sm' | 'lg'
  label: string
  className?: string
}) {
  const lit = litCells(left, cells)
  const level = levelFor(left)
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(left)}
      aria-valuetext={`${formatPct(left)} left, ${levelWord(left).toLowerCase()}`}
      className={clsx(
        'flex w-full',
        size === 'lg' ? 'h-7 gap-[3px]' : size === 'sm' ? 'h-2.5 gap-[2px]' : 'h-1.5 gap-[2px]',
        className
      )}
    >
      {Array.from({ length: cells }, (_, i) => (
        <span
          key={i}
          className={clsx(
            'cell flex-1',
            size === 'lg' ? 'rounded-[3px]' : 'rounded-[1.5px]',
            i < lit ? LEVEL_BG[level] : 'bg-[rgb(var(--cell-off))]'
          )}
          style={i < lit ? { transitionDelay: `${i * 12}ms` } : undefined}
        />
      ))}
    </div>
  )
}

/** One window: name, meter, "36% left", reset countdown. */
export function WindowGauge({
  w,
  now,
  compact = false,
  stale = false
}: {
  w: UsageWindow
  now: number
  compact?: boolean
  stale?: boolean
}) {
  const left = leftOf(w)
  const level = levelFor(left)
  const reset = resetText(w.resetsAt, now)
  const reached = left <= 0.5
  return (
    <div className={clsx('min-w-0', stale && '[&_[role=meter]]:opacity-60')}>
      <div className="flex items-baseline justify-between gap-2 mb-1">
        <span className="text-xs text-muted truncate" data-tip={w.label}>
          {w.label}
        </span>
        <span className={clsx('shrink-0 whitespace-nowrap text-xs font-semibold tabular-nums', reached ? 'text-empty' : 'text-ink')}>
          {reached ? 'Limit reached' : `${formatPct(left)} left`}
        </span>
      </div>
      <Meter left={left} label={`${w.label}`} cells={compact ? 20 : 25} size={compact ? 'xs' : 'sm'} />
      {!compact && (
        <div className="mt-1 flex justify-between text-xs text-muted">
          <span className={clsx(level === 'full' ? 'text-muted' : LEVEL_TEXT[level])}>{levelWord(left)}</span>
          <span className="tabular-nums">{reset ?? 'No reset scheduled'}</span>
        </div>
      )}
      {compact && <div className="mt-0.5 text-[11px] leading-4 text-muted tabular-nums truncate">{reset ?? 'Not started'}</div>}
    </div>
  )
}
