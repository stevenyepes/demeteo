# Settings

Demeteo has two settings surfaces:

- **Preferences** — global, app-wide configuration. Open via the gear icon in the top bar.
- **Project Settings** — per-workspace. Open via the settings icon on any workspace's home.

## Preferences

The Preferences screen has five tabs along the top.

### Machines

The list of **hosts where agents can run**. Each entry shows name, kind (`local` or `remote`), and connection health.

- For **local** machines, the runner is auto-detected via a three-tier lookup (dev cache → `$DEMETEO_RUNNER_BIN` → sibling of the Tauri binary).
- For **remote** machines, click *Enable remote runs* — Demeteo pushes the `demeteo-runner` binary to the box, installs a systemd `--user` service, and probes state on every load. The probe returns one of **Running**, **Installed (stopped)**, or **Not installed**.
- If you see an amber *linger* warning, an admin must run `sudo loginctl enable-linger <user>` on that box (systemd requires it for the user service to outlive the SSH session).

### Providers

Git hosting providers (GitHub, GitLab, etc.) — Demeteo uses them to clone repos, manage branches, and open merge requests. The Preferences tab is a redirect; the full management UI lives on its own **Providers** page.

Each provider stores its **Personal Access Token in the OS keyring** — never in SQLite or on disk.

### Defaults

| Setting | What it controls |
|---------|------------------|
| **Workspace Storage** | Directory where Demeteo clones project repositories. Defaults to the app data directory; set a custom path to use, e.g., a faster SSD or a synced folder. Restart the app after changing. |
| **Default Agent & Model** | The default agent kind and model for new workspaces. Currently set per-workspace in **Project Settings → Agent Strategy & Policies**; a global default will follow in a future release. |
| **Agent Timeouts** | Three global thresholds (in seconds) applied to every agent turn: **Fast** (the "agent blocked" timer — fires when both stdout and stderr are silent), **Normal** (no event ever received), and **Wall cap** (absolute upper bound). Raise *Fast* if long-running tasks are being killed too eagerly. |

### Memory

The **Memory Agent** — an opt-in background process that learns from each run.

Enable it, set a chat model (e.g. `llama3.1`) and an embeddings model (e.g. `nomic-embed-text`) via any **OpenAI-compatible local or remote LLM** ([Ollama](https://ollama.com) is the common choice), then click *Test connection*. The Memory Agent is the one place Demeteo calls a model provider directly, and it stores its API key in the OS keyring.

Per-project memories are viewable and editable under **Project Settings → Project Memory**.

### About

Version, build channel (`stable` / `nightly`), and links to the docs, releases, and issue tracker.

## Project Settings

Per-workspace settings override the global defaults. Open from any workspace home via the gear icon.

| Section | What it controls |
|---------|------------------|
| **Agent Strategy & Policies** | The default agent and model for this workspace; per-step agent overrides; budget caps; commit-artifacts toggle. |
| **Conflict Policy** | What to do when a subtask merge back to `feature/<slug>` fails, or `feature_sync` against `origin/<default>` leaves conflicts. Choices: `auto_agent` (default — Demeteo spawns a resolution agent in a temp worktree, commits the fix, replays the validation step), `auto_human` (open a Gate immediately with the file list), `always_gate` (open a Gate on every conflict). |
| **Project Memory** | View, edit, and prune the memories the Memory Agent has learned for this project. |
| **Permissions** | The compiled `PermissionProfile` applied to every agent turn in this workspace — a complete list of `allow`/`deny` capabilities with no real-time human-in-the-loop prompts at the tool level. |