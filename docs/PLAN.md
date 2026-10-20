# Baton: plan

## The problem

Power users pay for several AI subscriptions (Claude Pro/Max, ChatGPT Plus/Pro for Codex, Gemini, GitHub Copilot) and often several accounts of the same one. Each has rolling usage limits (5-hour sessions, weekly caps, monthly premium requests). Today you switch by logging out and back in, lose track of which account has quota left, and let limits reset unused, which wastes money.

**Baton** is a small desktop app (Windows, macOS, Linux) that:

1. Finds the AI accounts already on the machine.
2. Keeps each account in its own isolated config home, so all of them stay logged in at once.
3. Shows live usage limits, reset times and history for every account in one dashboard.
4. Launches the CLI (or desktop app where possible) with the account you pick, in one click, from the window or the tray.
5. Warns you before you hit a limit, tells you when one resets, flags quota that is about to reset unused, and recommends which account to use next.
6. Lets you choose, per account, what is shared with your main setup: skills, plugins, agents/commands, instructions, MCP config, settings, chat history.

## What already exists (October 2026)

| Project | What it does | Gap |
|---|---|---|
| [CodexBar](https://github.com/steipete/CodexBar) | Menu-bar usage meter for ~50 providers | macOS only, no launching/profiles |
| [claude-swap](https://github.com/realiti4/claude-swap) | Claude account rotation + usage dashboard | Claude only, Python TUI, Mac-only GUI |
| [caam](https://github.com/Dicklesworthstone/coding_agent_account_manager) | Fast auth swapping for Claude/Codex/Gemini | CLI only, no Windows docs |
| [cli-account-switcher](https://github.com/HoonStyle/cli-account-switcher) | Electron profiles for Claude/Codex | Preview, no live limits, Windows partial |
| [Symbioose/claude-account-switcher](https://github.com/Symbioose/claude-account-switcher), [Codex-Multi-Switcher](https://github.com/Nadav-Fux/Codex-Multi-Switcher) | Menu-bar switchers | macOS only |
| [aisw](https://burakdede.com/blog/switch-accounts-claude-code-codex-gemini-cli/), ccs, claude-profiles | Shell profile switchers | CLI only |
| BridgeSpace (BridgeMind) | Grid of many agent terminals in one window | Not account-aware |

Nobody ships a **cross-platform GUI** that combines profiles, live limits, launching and recommendations across providers. That is Baton.

## Tech choice

**Tauri 2**: Rust backend + React/TypeScript UI. A ~10 MB installer, low idle RAM (it lives in the tray all day), and native webviews on each OS (WebView2 / WKWebView / WebKitGTK). Rust handles credential files, process spawning and HTTP polling safely.

## How accounts are isolated

Every CLI supports pointing at an alternate home. Baton creates one home per account and launches the CLI with the matching env var:

| Provider | CLI | Isolation | Credentials | Usage source |
|---|---|---|---|---|
| Claude | `claude` | `CLAUDE_CONFIG_DIR` | `<home>/.credentials.json` (Win/Linux), Keychain `Claude Code-credentials[-<hash>]` (macOS) | `GET api.anthropic.com/api/oauth/usage` (5-hour, weekly, weekly-Opus/Sonnet) + `projects/**/*.jsonl` |
| Codex (ChatGPT) | `codex` | `CODEX_HOME` | `<home>/auth.json` (force `cli_auth_credentials_store="file"`) | `GET chatgpt.com/backend-api/wham/usage` (primary/secondary windows) + `sessions/**/*.jsonl` rate_limits |
| Gemini | `gemini` | `GEMINI_CLI_HOME` or HOME override | `~/.gemini/oauth_creds.json` | `POST cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota` |
| Copilot | `copilot` | `COPILOT_HOME` / XDG override | GitHub token | `GET api.github.com/copilot_internal/user` (premium requests) |

Sharing uses links: a junction on Windows (no admin needed), a symlink on macOS/Linux. Credential files are never linked or copied.

Desktop apps: Electron-based desktop apps (Claude Desktop) can run per-account with `--user-data-dir=<dir>`. Others are launched as-is and flagged as single-account in the UI.

## Architecture

```
src/                      React UI (Vite, Tailwind, Recharts, Zustand)
  shared/types.ts         Contract shared with Rust (mirrored in src-tauri/src/model.rs)
  api.ts                  HopperApi over Tauri invoke; mock data when run in a plain browser
src-tauri/src/
  lib.rs                  App setup, plugins, tray, background poller
  commands.rs             #[tauri::command]s = HopperApi
  store.rs                JSON persistence in the app data dir (accounts, settings, notifications, history)
  sharing.rs              Junction/symlink management
  launcher.rs             Open the user's terminal per OS with the account env
  usage.rs                Poll loop, history, activity cache
  notify.rs               Rules: nearly exhausted, limit hit, reset, use-it-or-lose-it
  recommend.rs            Score accounts for "what should I use now"
  providers/{claude,codex,gemini,copilot}.rs
```

## Milestones

**M1: working app (now)**
- Detect installed CLIs and existing homes (including side-by-side homes like `~/.claude-h1`).
- Add, import, edit and remove accounts; per-account sharing toggles.
- Launch CLI (new terminal), login flow, resume last session; desktop launch where supported.
- Live usage for Claude and Codex, best-effort for Gemini and Copilot; manual entry fallback.
- Dashboard: per-account limit bars, reset countdowns, usage-over-time and most-used charts.
- Notifications and recommendations. Tray menu with quick launch.
- CI builds for Windows, macOS and Linux (GitHub Actions + tauri-action) on tagged releases.

**M2: power features**
- Account-aware terminal grid inside the app (BridgeSpace-style, xterm.js + portable-pty), each pane on a different account.
- Auto-rotate: when the active account hits its limit, offer to reopen the session on the next account with `--continue`/resume and shared history.
- More providers: Cursor, Antigravity, Kiro, Amp, z.ai, OpenRouter credit balance.
- Per-task routing rules ("long refactors go to Max accounts").
- Encrypted backups of account metadata; export/import between machines.

**M3: landing page and distribution**
- Landing page (static site on GitHub Pages): live animated demo of the dashboard, download buttons per OS with auto-detect, provider list, privacy statement (everything stays local).
- Signed builds (Windows code signing, macOS notarization), auto-update via Tauri updater, Homebrew cask, winget, Flathub/AUR.

## Privacy and safety rules

- Everything is local. No telemetry, no server.
- Tokens are read only to call that provider's own usage endpoint, only when "live usage" is on (default on, can be turned off for local-logs-only mode).
- Never write to another tool's credential file except an atomic token refresh that keeps the file valid for the CLI.
- Uninstall leaves every CLI home intact.
