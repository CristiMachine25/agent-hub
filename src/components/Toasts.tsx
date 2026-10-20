import { useState } from 'react'
import clsx from 'clsx'
import { AlertTriangle, Bell, Check, Copy, X } from 'lucide-react'
import { useStore } from '@/store'

export function CopyButton({ text, label = 'Copy command' }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      className="btn btn-ghost btn-sm shrink-0 px-1.5"
      aria-label={label}
      title={label}
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
  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-[440px] max-w-[calc(100vw-2rem)] flex-col gap-2"
    >
      {toasts.map((t) => {
        const Icon = t.kind === 'error' ? AlertTriangle : t.kind === 'info' ? Bell : Check
        return (
          <div
            key={t.id}
            role={t.kind === 'error' ? 'alert' : 'status'}
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
              <button className="-mr-1 -mt-0.5 rounded p-0.5 text-muted hover:text-ink" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
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
      })}
    </div>
  )
}
