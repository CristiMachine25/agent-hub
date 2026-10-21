import { useEffect, useRef, useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, Bell, Check, Copy, X } from 'lucide-react'
import { useStore, type Toast } from '@/store'

export function CopyButton({ text, label = 'Copy command' }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className="btn btn-ghost btn-sm shrink-0 px-1.5"
      aria-label={label}
      data-tip={label}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text)
          setDone(true)
          setTimeout(() => setDone(false), 1500)
        } catch {
          /* clipboard can be unavailable; the text is still selectable */
        }
      }}
    >
      {done ? <Check size={14} /> : <Copy size={14} />}
    </button>
  )
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts)
  const dismiss = useStore((s) => s.dismiss)
  // While the pointer or focus is on the stack, toasts whose time ran out stay up (e.g. to finish copying a command).
  const [held, setHeld] = useState<Toast[] | null>(null)
  const release = useRef<ReturnType<typeof setTimeout>>(undefined)
  useEffect(() => () => clearTimeout(release.current), [])
  const hold = () => {
    clearTimeout(release.current)
    setHeld((h) => h ?? toasts)
  }
  const letGo = () => {
    clearTimeout(release.current)
    release.current = setTimeout(() => setHeld(null), 1500)
  }
  const shown = held ? [...held.filter((h) => !toasts.some((t) => t.id === h.id)), ...toasts].sort((a, b) => a.id - b.id) : toasts
  const close = (id: number) => {
    setHeld((h) => h && h.filter((t) => t.id !== id))
    dismiss(id)
  }
  const card = (t: Toast) => {
    const Icon = t.kind === 'error' ? AlertTriangle : t.kind === 'info' ? Bell : Check
    return (
      <div
        key={t.id}
        className={clsx(
          'toast-in pointer-events-auto rounded-lg border bg-deck px-3.5 py-3 shadow-xl shadow-black/30',
          t.kind === 'error' ? 'border-empty/60' : 'border-rivet'
        )}
      >
        <div className="flex items-start gap-2.5">
          <Icon size={16} className={clsx('mt-0.5 shrink-0', t.kind === 'error' ? 'text-empty' : 'text-muted')} />
          <div className="min-w-0 flex-1">
            <p className="font-semibold">{t.title}</p>
            {t.body && <p className="mt-0.5 text-muted">{t.body}</p>}
          </div>
          <button className="-mr-1 -mt-0.5 rounded p-0.5 text-muted hover:text-ink" aria-label="Dismiss" onClick={() => close(t.id)}>
            <X size={14} />
          </button>
        </div>
        {t.command && (
          <div className="mt-2 flex items-start gap-1 rounded border border-rivet bg-ground py-1.5 pl-2.5 pr-1">
            <code className="code min-w-0 flex-1 select-text break-all text-muted">
              <span className="select-none text-faint">$ </span>
              {t.command}
            </code>
            <CopyButton text={t.command} />
          </div>
        )}
      </div>
    )
  }
  return (
    <div
      className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-[440px] max-w-[calc(100vw-2rem)] flex-col"
      onPointerEnter={hold}
      onPointerLeave={letGo}
      onFocus={hold}
      onBlur={(e) => !e.currentTarget.contains(e.relatedTarget) && letGo()}
    >
      {/* Two persistent live regions, so each toast is announced once: errors right away, the rest politely. */}
      <div aria-live="polite" className="flex flex-col gap-2">
        {shown.filter((t) => t.kind !== 'error').map(card)}
      </div>
      <div role="alert" className="flex flex-col gap-2 [&:not(:empty)]:pt-2">
        {shown.filter((t) => t.kind === 'error').map(card)}
      </div>
    </div>
  )
}
