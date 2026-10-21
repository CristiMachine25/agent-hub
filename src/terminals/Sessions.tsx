import { useState } from "react";
import clsx from "clsx";
import { PanelLeftClose, PanelLeftOpen, Plus, X } from "lucide-react";
import { useStore } from "@/store";
import { useCollapsed } from "@/lib/collapse";
import { AccountDot, ProviderMark } from "@/components/ui";
import { useGrid, type Session } from "./store";

/**
 * The darker list beside the grid: every session (its own layout and terminals) and the terminals in it.
 * Hidden sessions keep running, so switching back shows them exactly as they were.
 */
export function SessionsPanel() {
  const sessions = useGrid((s) => s.sessions);
  const activeId = useGrid((s) => s.activeId);
  const ws = useGrid((s) => s.ws);
  const { newSession, switchSession } = useGrid.getState();
  const [collapsed, setCollapsed] = useCollapsed(
    "switchyard.sessionsCollapsed",
    () => false,
  );
  const toggle = `${collapsed ? "Show" : "Hide"} sessions`;

  if (collapsed)
    return (
      <aside
        aria-label="Sessions"
        className="flex w-11 shrink-0 flex-col items-center gap-1 border-r border-rivet bg-well py-2"
      >
        <button
          className="side-icon-btn"
          onClick={() => setCollapsed(false)}
          data-tip={toggle}
          data-tip-side="right"
          aria-label={toggle}
          aria-expanded={false}
        >
          <PanelLeftOpen size={16} />
        </button>
        {sessions.map((x, i) => (
          <button
            key={x.id}
            onClick={() => switchSession(x.id)}
            data-tip={x.name}
            data-tip-side="right"
            aria-label={x.name}
            aria-current={x.id === activeId ? "true" : undefined}
            className={clsx(
              "flex h-7 w-7 items-center justify-center rounded text-xs font-semibold tabular-nums",
              x.id === activeId
                ? "bg-raised text-ink"
                : "text-muted hover:bg-raised/60 hover:text-ink",
            )}
          >
            {i + 1}
          </button>
        ))}
        <button
          className="side-icon-btn"
          onClick={newSession}
          data-tip="New session"
          data-tip-side="right"
          aria-label="New session"
        >
          <Plus size={16} />
        </button>
      </aside>
    );

  return (
    <aside
      aria-label="Sessions"
      className="flex w-[216px] shrink-0 flex-col border-r border-rivet bg-well"
    >
      <div data-tauri-drag-region="deep" className="flex h-14 shrink-0 items-center gap-1 border-b border-rivet pl-3 pr-1.5">
        <span className="flex-1 text-xs font-semibold text-muted">
          Sessions
        </span>
        <button
          className="side-icon-btn"
          onClick={newSession}
          data-tip="New session"
          aria-label="New session"
        >
          <Plus size={16} />
        </button>
        <button
          className="side-icon-btn"
          onClick={() => setCollapsed(true)}
          data-tip={toggle}
          aria-label={toggle}
          aria-expanded
        >
          <PanelLeftClose size={16} />
        </button>
      </div>
      <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto px-1.5 py-2">
        {sessions.map((x) => (
          <SessionItem
            key={x.id}
            session={x}
            active={x.id === activeId}
            panes={x.id === activeId ? ws.panes : x.ws.panes}
            focused={x.id === activeId ? ws.focused : -1}
          />
        ))}
      </ul>
    </aside>
  );
}

function SessionItem({
  session,
  active,
  panes,
  focused,
}: {
  session: Session;
  active: boolean;
  panes: Session["ws"]["panes"];
  focused: number;
}) {
  const accounts = useStore((s) => s.accounts);
  const infos = useGrid((s) => s.infos);
  const { switchSession, renameSession, closeSession, focus } =
    useGrid.getState();
  const [editing, setEditing] = useState(false);
  // Closing a session with live terminals takes a second click.
  const [confirming, setConfirming] = useState(false);
  const used = panes
    .map((p, i) => ({
      p,
      i,
      account: accounts.find((a) => a.id === p.accountId),
    }))
    .filter((x) => x.account);
  const running = used.filter(
    (x) =>
      x.p.terminalId && infos[x.p.terminalId] && !infos[x.p.terminalId].exited,
  ).length;

  return (
    <li>
      <div
        className={clsx(
          "group flex h-8 items-center gap-2 rounded pl-2 pr-1 text-sm",
          active
            ? "bg-raised font-semibold text-ink"
            : "text-muted hover:bg-raised/60 hover:text-ink",
        )}
      >
        {editing ? (
          <input
            autoFocus
            defaultValue={session.name}
            aria-label="Session name"
            className="h-6 min-w-0 flex-1 px-1.5 text-sm"
            onBlur={(e) => {
              renameSession(session.id, e.target.value);
              setEditing(false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") setEditing(false);
            }}
          />
        ) : (
          <button
            className="min-w-0 flex-1 truncate text-left"
            onClick={() => switchSession(session.id)}
            onDoubleClick={() => setEditing(true)}
            data-tip="Double-click to rename"
            aria-current={active ? "true" : undefined}
          >
            {session.name}
          </button>
        )}
        <span
          className="shrink-0 text-xs font-normal tabular-nums text-faint"
          data-tip={`${running} running`}
        >
          {running || ""}
        </span>
        {confirming ? (
          <button
            autoFocus
            className="h-6 shrink-0 rounded bg-empty/15 px-1.5 text-xs font-semibold text-empty hover:bg-empty/25"
            onClick={() => void closeSession(session.id)}
            onBlur={() => setConfirming(false)}
            data-tip={`Close ${session.name} and stop its terminals`}
          >
            Stop {running}
          </button>
        ) : (
          <button
            className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-faint opacity-0 hover:bg-ground hover:text-ink focus:opacity-100 group-hover:opacity-100"
            onClick={() =>
              running ? setConfirming(true) : void closeSession(session.id)
            }
            data-tip="Close session"
            aria-label={`Close ${session.name}`}
          >
            <X size={13} />
          </button>
        )}
      </div>
      {used.length > 0 && (
        <ul className="ml-3 mt-0.5 border-l border-rivet pl-1.5">
          {used.map(({ p, i, account }) => {
            const info = p.terminalId ? infos[p.terminalId] : undefined;
            const live = !!info && !info.exited;
            return (
              <li key={p.uid}>
                <button
                  onClick={() => {
                    switchSession(session.id);
                    requestAnimationFrame(() => focus(i));
                  }}
                  className={clsx(
                    "flex h-7 w-full items-center gap-2 rounded px-1.5 text-left text-xs",
                    active && focused === i
                      ? "bg-raised/70 text-ink"
                      : "text-muted hover:bg-raised/50 hover:text-ink",
                  )}
                  data-tip={`${account!.label}: ${live ? "running" : info ? "exited" : "not running"}`}
                >
                  <AccountDot account={account!} size={7} />
                  <ProviderMark id={account!.providerId} size={12} />
                  <span className="min-w-0 flex-1 truncate">
                    {account!.label}
                  </span>
                  <span
                    className={clsx(
                      "h-1.5 w-1.5 shrink-0 rounded-full",
                      live ? "bg-full" : "bg-faint/50",
                    )}
                    aria-hidden
                  />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </li>
  );
}
