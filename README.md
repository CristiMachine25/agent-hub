# Agent Hub

**Switch your work to the account that still has quota.** All your AI subscriptions in one place. See how much of every limit you have left, get told before you hit one (or before unused quota resets), and launch Claude Code, Codex, Gemini CLI or Copilot CLI with the account you pick, in one click.

Runs on **Windows, macOS and Linux**. Everything stays on your computer.

> Status: early development (v0.2). Website: https://cristimachine25.github.io/agent-hub/. See [docs/PLAN.md](docs/PLAN.md) for what has shipped and what is next.

## Why

If you pay for more than one AI plan (two Claude Max accounts, ChatGPT Pro, Gemini, Copilot...), you know the routine: hit a limit, log out, log in to another account, forget which one still has quota, and watch a weekly limit reset while half of it was never used. Agent Hub keeps every account signed in side by side and tells you which one to use next.

## Features

- **Finds your accounts.** Detects installed CLIs and existing config folders, including side-by-side ones like `~/.claude-work`.
- **Every account stays signed in.** Each account gets its own config home (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, ...), so switching never means logging out.
- **Live limits.** 5-hour session, weekly, weekly Opus, Codex primary/secondary windows, Gemini model quotas, Copilot premium requests, with reset countdowns and history charts.
- **Notifications.** Nearly exhausted, limit reached, limit reset, and "use it or lose it" when lots of quota is about to reset unused.
- **Recommendations.** Pick a task type (coding, long refactor, quick question, research, review) and get the best account to use right now.
- **One-click launch.** Opens your terminal with the right account, or resumes your last session. Desktop apps too, where they support it.
- **Launcher.** Press Ctrl/Cmd+K, type an account or screen name, press Enter. Keys `1` to `7` jump between screens.
- **Permission modes.** Pick per tool how every CLI starts, on every account: ask as usual, accept edits, plan, or bypass permissions (Claude Code), auto or bypass (Codex), auto edit or YOLO (Gemini), allow all tools (Copilot).
- **Sharing you control.** Per account, choose what to share with your main setup: skills, plugins, agents and commands, instructions, MCP servers, settings, chat history. Credentials are never shared.
- **Shared knowledge.** Each tool gets one shared profile for skills, agents, instructions, plugins and settings, and every account links to it, so all your accounts work the same way. The Knowledge screen shows what is in it.
- **Skills from skills.sh.** Browse featured skills or search the public skills.sh directory, read a skill's SKILL.md, and install it for Claude Code, Codex, Gemini CLI or Copilot in one step. It goes into each tool's shared folder, downloaded from the skill's GitHub repository at a single commit, and nothing already there is overwritten.
- **Terminal grid.** Run up to nine AI CLIs side by side in one window, each pane on its own account with its own meter. When one hits its limit, continue the same chat on another account in one click. Keep several sessions, each with its own layout, in a collapsible list beside the grid; hidden sessions keep running.
- **Move to a new computer.** Export your setup (accounts, skills, preferences) to an `.agenthub` file and import it on Windows, macOS or Linux. Sign-ins are optional and encrypted with a passphrase you choose. Import only adds: nothing already on the computer is replaced.
- **Tray menu** for quick launches without opening the window. Agent Hub can start at login, hidden in the tray.

## Supported tools

| Tool | Accounts | Live limits | Launch |
|---|---|---|---|
| Claude Code (+ Claude Desktop) | yes | yes | CLI, desktop |
| Codex CLI (ChatGPT plans) | yes | yes | CLI |
| Gemini CLI | yes | yes | CLI |
| GitHub Copilot CLI | yes | premium requests | CLI |

More are planned: Cursor, Antigravity, Kiro, Amp, OpenRouter.

## Privacy

- No servers, no telemetry, no account with us.
- With **live usage** on, Agent Hub reads each account's own login token from your disk and sends it only to that provider's own servers: its usage endpoint, its account profile when usage is refused, and its sign-in renewal. Turn it off in Settings to use local session logs only.
- Agent Hub never copies or links credential files between accounts.

## Install

Download the installer for your system from [Releases](https://github.com/CristiMachine25/agent-hub/releases/latest) (Windows `.msi`/`.exe`, macOS `.dmg`, Linux `.AppImage`/`.deb`).

macOS builds are not notarized yet. The first time you open Agent Hub, macOS says it can't verify the developer: open **System Settings > Privacy & Security** and click **Open Anyway**.

## Build from source

Prerequisites: Node.js 22.12+, Rust (stable), and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run app:dev     # run the desktop app with hot reload
npm run app:build   # build installers into src-tauri/target/release/bundle
npm run dev         # UI only, in a browser, with mock data
```

## Contributing

Issues and pull requests are welcome. Adding a provider takes an adapter plus a few tables on both sides; the compiler points at most of them once the new id exists:

- **Rust:** implement the `ProviderAdapter` trait in a new file in `src-tauri/src/providers/` and register it in `providers::all()` (`providers/mod.rs`, which also maps permission modes to CLI flags). Add the id to `ProviderId` and `PermissionModes` in `src-tauri/src/model.rs`, its short name in `ProviderId::name` (`model.rs`), and, if its plans use tier words the recommender doesn't know yet, those words to `tier` in `recommend.rs` (mirrored in `src/lib/recommend.ts`). Return failures as a `FetchError` (`providers/mod.rs`) of the right kind, so being offline never reads as signed out.
- **Contract check:** `cargo test` checks `src/shared/__fixtures__/serde.ts` against the Rust types. After changing a shape, a command or an enum (a new `ProviderId` also goes in the `variants!` lists in `model.rs`), regenerate it with `cd src-tauri && UPDATE_SERDE_FIXTURE=1 cargo test serde_fixture`, then run `npm run typecheck` to see what `types.ts` is missing.
- **UI:** add the id to `ProviderId` and `PermissionModes` in `src/shared/types.ts`, then to `PROVIDER_ORDER`/`PROVIDER_META` (`src/lib/providers.ts`), `PLANS`/`DEFAULT_PLAN` (`src/lib/plans.ts`), the logo (`LOGO_PATHS` in `src/components/ui.tsx`), the permission mode options (`src/screens/Knowledge.tsx`) and the browser-preview data (`src/mock/`).

## Disclaimer

Agent Hub is an independent project, not affiliated with Anthropic, OpenAI, Google or GitHub. It uses each tool's own login and its documented or observed usage endpoints, which can change without notice. Follow each provider's terms of service for how you use multiple accounts.

## License

[MIT](LICENSE)
