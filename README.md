# Demeteo

Demeteo is a desktop control plane (Tauri + React) for orchestrating local and remote AI coding agents. It bridges raw SSH terminal access and structured agent protocols into a single supervisor-style workspace. See [`AGENTS.md`](file:///home/jsteven/Projects/demeteo/AGENTS.md) for the project constitution and [`AGENT_INTEGRATION.md`](file:///home/jsteven/Projects/demeteo/AGENT_INTEGRATION.md) for the agent-integration spec.

## Third-party projects

Demeteo integrates with third-party coding agents that the user installs and configures locally. Trademarks and project names belong to their respective owners:

- **opencode** — Demeteo's `opencode` integration targets the [`anomalyco/opencode`](https://github.com/anomalyco/opencode) project. **Demeteo is not built by, maintained by, or affiliated with the opencode project.** "opencode" is a trademark of its respective owner and is used here only to describe compatibility.
- **Hermes** — Demeteo's `hermes` integration targets [Nous Research's hermes-agent](https://github.com/NousResearch/hermes-agent). Demeteo is not affiliated with Nous Research.

## Development setup

Tauri + React + TypeScript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
