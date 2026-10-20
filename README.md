# Baton

**Pass your work to the account that still has quota.** All your AI subscriptions in one place. See how much of every limit you have left, get told before you hit one (or before unused quota resets), and launch Claude Code, Codex, Gemini CLI or Copilot CLI with the account you pick, in one click.

Runs on **Windows, macOS and Linux**. Everything stays on your computer.

> Status: early development (v0.2). Website: https://cristimachine25.github.io/baton/. See [docs/PLAN.md](docs/PLAN.md) for the roadmap.

## Why

If you pay for more than one AI plan (two Claude Max accounts, ChatGPT Pro, Gemini, Copilot...), you know the routine: hit a limit, log out, log in to another account, forget which one still has quota, and watch a weekly limit reset while half of it was never used. Baton keeps every account signed in side by side and tells you which one to use next.

## Features

- **Finds your accounts.** Detects installed CLIs and existing config folders, including side-by-side ones like `~/.claude-work`.
- **Every account stays signed in.** Each account gets its own config home (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, ...), so switching never means logging out.
- **Live limits.** 5-hour session, weekly, weekly Opus, Codex primary/secondary windows, Gemini model quotas, Copilot premium requests, with reset countdowns and history charts.
- **Notifications.** Nearly exhausted, limit reached, limit reset, and "use it or lose it" when lots of quota is about to reset unused.
- **Recommendations.** Pick a task type (coding, long refactor, quick question, research, review) and get the best account to use right now.
- **One-click launch.** Opens your terminal with the right account, or resumes your last session. Desktop apps too, where they support it.
- **Sharing you control.** Per account, choose what to share with your main setup: skills, plugins, agents and commands, instructions, MCP servers, settings, chat history. Credentials are never shared.
- **Terminal grid.** Run up to nine AI CLIs side by side in one window, each pane on its own account with its own meter. When one hits its limit, continue the same chat on another account in one click.
- **Tray menu** for quick launches without opening the window. Baton can start at login, hidden in the tray.

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
- With **live usage** on, Baton reads each account's own login token from your disk and sends it only to that provider's own usage endpoint. Turn it off in Settings to use local session logs only.
- Baton never copies or links credential files between accounts.

## Install

Download the installer for your system from [Releases](../../releases) (Windows `.msi`/`.exe`, macOS `.dmg`, Linux `.AppImage`/`.deb`).

## Build from source

Prerequisites: Node.js 20+, Rust (stable), and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run app:dev     # run the desktop app with hot reload
npm run app:build   # build installers into src-tauri/target/release/bundle
npm run dev         # UI only, in a browser, with mock data
```

## Contributing

Issues and pull requests are welcome. Adding a provider means implementing one Rust trait: see `src-tauri/src/providers/mod.rs`.

## Disclaimer

Baton is an independent project, not affiliated with Anthropic, OpenAI, Google or GitHub. It uses each tool's own login and its documented or observed usage endpoints, which can change without notice. Follow each provider's terms of service for how you use multiple accounts.

## License

[MIT](LICENSE)
