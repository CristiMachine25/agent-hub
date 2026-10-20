import { useEffect, useRef, type ReactNode } from 'react'
import clsx from 'clsx'
import { X } from 'lucide-react'
import type { Account, ProviderId } from '@shared/types'
import { PROVIDER_META } from '@/lib/providers'

export function ProviderMark({ id, size = 18 }: { id: ProviderId; size?: number }) {
  const m = PROVIDER_META[id]
  return (
    <span
      aria-hidden
      className="inline-flex shrink-0 items-center justify-center rounded-[4px] font-bold text-white"
      style={{ width: size, height: size, background: m.brand, fontSize: Math.round(size * 0.56) }}
    >
      {m.mark}
    </span>
  )
}

export function AccountDot({ account, size = 10 }: { account: Pick<Account, 'color'>; size?: number }) {
  return (
    <span
      aria-hidden
      className="inline-block shrink-0 rounded-full ring-2 ring-ground"
      style={{ width: size, height: size, background: account.color }}
    />
  )
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
  id
}: {
  checked: boolean
  onChange: (v: boolean) => void
  label: string
  disabled?: boolean
  id?: string
}) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={clsx(
        'relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors disabled:opacity-50',
        checked ? 'bg-hop border-hop' : 'bg-ground border-rivet'
      )}
    >
      <span
        className={clsx(
          'inline-block h-3.5 w-3.5 rounded-full transition-transform',
          checked ? 'translate-x-[18px] bg-hop-ink' : 'translate-x-[2px] bg-faint'
        )}
      />
    </button>
  )
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  size = 'md'
}: {
  value: T
  options: { id: T; label: string; title?: string }[]
  onChange: (v: T) => void
  label: string
  size?: 'sm' | 'md'
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex flex-wrap gap-1 rounded-md bg-ground p-0.5 border border-rivet">
      {options.map((o, i) => {
        const on = o.id === value
        return (
          <button
            key={o.id}
            ref={(el) => {
              refs.current[i] = el
            }}
            type="button"
            role="radio"
            aria-checked={on}
            title={o.title}
            tabIndex={on ? 0 : -1}
            onClick={() => onChange(o.id)}
            onKeyDown={(e) => {
              const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0
              if (!d) return
              e.preventDefault()
              const n = (i + d + options.length) % options.length
              onChange(options[n].id)
              refs.current[n]?.focus()
            }}
            className={clsx(
              'rounded-[5px] px-2.5 transition-colors whitespace-nowrap',
              size === 'sm' ? 'h-6 text-xs' : 'h-7 text-sm',
              on ? 'bg-raised text-ink font-semibold shadow-[inset_0_0_0_1px_rgb(var(--rivet))]' : 'text-muted hover:text-ink'
            )}
          >
            {o.label}
          </button>
        )
      })}
    </div>
  )
}

export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  width = 520
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  footer?: ReactNode
  width?: number
}) {
  const panel = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const prev = document.activeElement as HTMLElement | null
    const el = panel.current
    const first = el?.querySelector<HTMLElement>('[data-autofocus], input, button:not([data-close]), select, textarea')
    first?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        onClose()
      }
      if (e.key === 'Tab' && el) {
        const items = [...el.querySelectorAll<HTMLElement>('button, input, select, textarea, [tabindex]:not([tabindex="-1"])')].filter(
          (x) => !x.hasAttribute('disabled')
        )
        if (!items.length) return
        const a = items[0]
        const z = items[items.length - 1]
        if (e.shiftKey && document.activeElement === a) {
          e.preventDefault()
          z.focus()
        } else if (!e.shiftKey && document.activeElement === z) {
          e.preventDefault()
          a.focus()
        }
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      prev?.focus?.()
    }
  }, [open, onClose])
  if (!open) return null
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 px-4 pt-[10vh]" onMouseDown={onClose}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
        className="toast-in flex max-h-[80vh] w-full flex-col rounded-lg border border-rivet bg-deck shadow-2xl shadow-black/40"
        style={{ maxWidth: width }}
      >
        <div className="flex items-center justify-between border-b border-rivet px-5 py-3">
          <h2 className="text-md font-semibold">{title}</h2>
          <button data-close className="btn btn-ghost btn-sm -mr-2 px-1.5" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="overflow-y-auto px-5 py-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-rivet px-5 py-3">{footer}</div>}
      </div>
    </div>
  )
}

export function EmptyState({ title, body, action }: { title: string; body: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-2 rounded-lg border border-dashed border-rivet px-6 py-8">
      <h3 className="text-md font-semibold">{title}</h3>
      <p className="max-w-md text-muted">{body}</p>
      {action && <div className="mt-2">{action}</div>}
    </div>
  )
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="mb-3 flex items-center justify-between gap-3">
      <h2 className="text-md font-semibold">{children}</h2>
      {aside}
    </div>
  )
}
