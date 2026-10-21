/** A screen: a fixed toolbar like the terminal grid's, with the content scrolling below it. */
export function Page({ title, actions, children }: { title: string; actions?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex h-full flex-col">
      <Toolbar title={title}>{actions}</Toolbar>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="flex flex-col gap-6 px-5 pb-10 pt-5">{children}</div>
      </div>
    </div>
  )
}

/** The 56px bar at the top of a screen. It doubles as the window's title bar, so empty parts of it drag the window. */
export function Toolbar({ title, level = 1, children }: { title: React.ReactNode; level?: 1 | 2; children?: React.ReactNode }) {
  // A screen with its own titled side panel (Accounts) uses level 2 here, so it keeps one h1.
  const Heading = level === 1 ? 'h1' : 'h2'
  return (
    <header data-tauri-drag-region="deep" className="flex h-14 shrink-0 items-center gap-4 border-b border-rivet px-5">
      <Heading className="min-w-0 truncate font-serif text-lg font-semibold">{title}</Heading>
      <div className="ml-auto flex shrink-0 items-center gap-2">{children}</div>
    </header>
  )
}
