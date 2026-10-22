import { Search } from 'lucide-react'
import { Meter } from '@/components/Meter'

const STEPS = [
  { title: 'Add an account', body: 'Pick the Claude, Codex, Gemini and Copilot profiles already on this computer, or create a fresh profile for another login.' },
  { title: 'See what is left', body: 'Each account gets a meter for every limit window, with its reset countdown.' },
  { title: 'Launch the best one', body: 'The account with the most room for your task sits at the top, one click away.' }
]

/** Overview before any account exists: what the board will show, and the one way to start it. */
export function FirstRun({ onAdd }: { onAdd: () => void }) {
  return (
    <section aria-labelledby="firstrun-title" className="rounded-lg border border-rivet bg-deck">
      <div className="grid grid-cols-1 min-[1100px]:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <div className="px-6 py-8">
          <h2 id="firstrun-title" className="font-serif text-[26px] font-semibold leading-8">
            No accounts yet
          </h2>
          <p className="mt-2 max-w-[60ch] text-muted">
            Add the AI accounts you use and this screen answers one question: which one to use right now.
          </p>
          <ol className="mt-6 max-w-[60ch] space-y-4">
            {STEPS.map((s, i) => (
              <li key={s.title} className="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-x-3">
                <span aria-hidden className="flex h-6 w-6 items-center justify-center rounded-full border border-rivet text-xs tabular-nums text-muted">
                  {i + 1}
                </span>
                <div>
                  <p className="font-semibold leading-6">{s.title}</p>
                  <p className="text-muted">{s.body}</p>
                </div>
              </li>
            ))}
          </ol>
          <button className="btn btn-primary btn-lg mt-8" onClick={onAdd}>
            <Search size={16} /> Find accounts on this computer
          </button>
        </div>
        {/* A placeholder of the board, so the first account has an obvious place to land. */}
        <div aria-hidden className="hidden flex-col justify-center gap-4 border-l border-rivet px-6 py-8 min-[1100px]:flex">
          <p className="text-xs text-faint">Your accounts will appear here</p>
          {['5-hour session', 'Weekly limit', 'Monthly requests'].map((label) => (
            <div key={label}>
              <div className="mb-2 text-xs text-faint">{label}</div>
              <Meter left={0} cells={20} size="sm" label={label} />
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}
