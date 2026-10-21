import { useEffect } from 'react'
import clsx from 'clsx'
import { Loader2 } from 'lucide-react'
import { api } from '@/api'
import { useStore } from '@/store'
import { LAYOUTS, LAYOUT_IDS, type Dir, type LayoutId } from '@/lib/grid'
import { modKey } from '@/lib/platform'
import { restyleAll } from './registry'
import { useGrid } from './store'
import { Pane } from './Pane'
import { SessionsPanel } from './Sessions'

/** Tiny picture of a layout preset, drawn from the same rects the grid uses. */
function LayoutIcon({ id }: { id: LayoutId }) {
  const l = LAYOUTS[id]
  const W = 18
  const H = 13
  const cw = W / l.cols
  const rh = H / l.rows
  return (
    <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} aria-hidden>
      {l.rects.map((r, i) => (
        <rect key={i} x={r.col * cw + 0.75} y={r.row * rh + 0.75} width={r.colSpan * cw - 1.5} height={r.rowSpan * rh - 1.5} rx={1.2} fill="currentColor" />
      ))}
    </svg>
  )
}

const ARROWS: Record<string, Dir> = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' }

function useGridKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = api.platform === 'darwin' ? e.metaKey : e.ctrlKey
      if (!mod || !e.shiftKey) return
      const g = useGrid.getState()
      if (ARROWS[e.key]) {
        e.preventDefault()
        g.move(ARROWS[e.key])
      } else if (e.key === 'Enter') {
        e.preventDefault()
        g.toggleMax()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

export function Terminals() {
  const ws = useGrid((s) => s.ws)
  const restored = useGrid((s) => s.restored)
  const restore = useGrid((s) => s.restore)
  const setLayout = useGrid((s) => s.setLayout)
  const mode = useStore((s) => s.resolvedTheme)
  const layout = LAYOUTS[ws.layout]
  const mod = modKey(api.platform)

  useGridKeys()
  useEffect(() => {
    void restore()
  }, [restore])
  useEffect(() => {
    // Wait a frame so the new theme's CSS variables are in place before reading them.
    requestAnimationFrame(() => restyleAll(mode))
  }, [mode])

  return (
    <div className="flex h-full">
      <SessionsPanel />
      <div className="flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region="deep" className="flex h-14 shrink-0 items-center gap-4 border-b border-rivet px-5">
          <h1 className="font-serif text-lg font-semibold">Terminals</h1>
          <p className="hidden text-xs text-muted min-[1100px]:block">
            Each pane runs on its own account. {mod}+Shift+arrows move between panes, {mod}+Shift+Enter maximizes.
          </p>
          <div className="ml-auto inline-flex rounded border border-rivet bg-deck p-0.5" role="radiogroup" aria-label="Layout">
            {LAYOUT_IDS.map((id) => (
              <button
                key={id}
                role="radio"
                aria-checked={ws.layout === id}
                data-tip={LAYOUTS[id].label}
                aria-label={LAYOUTS[id].label}
                onClick={() => setLayout(id)}
                className={clsx(
                  'flex h-7 w-9 items-center justify-center rounded-[4px] transition-colors',
                  ws.layout === id ? 'bg-raised text-hop' : 'text-faint hover:text-ink'
                )}
              >
                <LayoutIcon id={id} />
              </button>
            ))}
          </div>
        </header>
        {!restored ? (
          <div className="flex flex-1 items-center justify-center gap-2 text-muted" role="status">
            <Loader2 size={16} className="animate-spin" /> Reconnecting to running terminals…
          </div>
        ) : (
          <div
            className="grid min-h-0 flex-1 gap-2 p-2"
            style={
              ws.maximized !== null
                ? { gridTemplateColumns: '1fr', gridTemplateRows: '1fr' }
                : { gridTemplateColumns: `repeat(${layout.cols}, minmax(0, 1fr))`, gridTemplateRows: `repeat(${layout.rows}, minmax(0, 1fr))` }
            }
          >
            {ws.panes.map((p, i) => {
              const r = layout.rects[i]
              const hidden = ws.maximized !== null && ws.maximized !== i
              return (
                <div
                  key={p.uid}
                  className={clsx('flex min-h-0 min-w-0 [&>section]:flex-1', hidden && 'hidden')}
                  style={
                    ws.maximized === i || !r
                      ? undefined
                      : { gridColumn: `${r.col + 1} / span ${r.colSpan}`, gridRow: `${r.row + 1} / span ${r.rowSpan}` }
                  }
                >
                  <Pane index={i} pane={p} />
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}
