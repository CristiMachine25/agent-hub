import { Component, type ReactNode } from 'react'
import { TitleBar } from './TitleBar'

/** Last line of defence: a render error shows this screen instead of unmounting the whole app. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null }

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) }
  }

  render() {
    const { error } = this.state
    if (!error) return this.props.children
    return (
      <div className="flex h-full flex-col" role="alert">
        <TitleBar />
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
          <p className="text-md font-semibold">Something went wrong in Relay</p>
          <p className="max-w-md text-muted">Your accounts and settings are safe. Reload to carry on; running terminals keep running.</p>
          <code className="code max-w-md truncate text-xs text-faint">{error.message}</code>
          <button className="btn btn-primary" onClick={() => location.reload()}>
            Reload
          </button>
        </div>
      </div>
    )
  }
}
