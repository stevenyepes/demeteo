# Product

<!-- impeccable:product-schema 1 -->

> Written from repository evidence; audience, commercial intent, the Hub, UI
> language, voice, the accessibility target and the Project noun were
> confirmed by the owner. Facts marked *(inferred)* are
> unconfirmed; questions only the owner can answer are under
> [Open Decisions](#open-decisions). Terms follow AGENTS.md §1 exactly.

## Platform

web

A Tauri v2 desktop app: the UI is a React 19 webview, not a native design
language. It ships on Linux x86_64, macOS aarch64 and Windows x86_64; Intel
Macs are unsupported. A second surface is in progress: Demeteo Hub, a browser
app where Gates and runs can be handled away from the desktop. Its server and
JSON API exist; its web UI does not yet. This supersedes the "no web
companion" line in `docs/OPEN_QUESTIONS.md` §17. A mobile app stays out of
scope.

## Users

A developer who already uses CLI coding agents (Claude Code, Codex, OpenCode,
Hermes, pi) and wants to hand them work bigger than one prompt can finish. In
Demeteo they work as a product manager over a fleet of agents: they describe
the work, settle the decisions an interviewer agent raises, and step in only at
Gates. They don't chat with a model or watch tool calls happen.

They know Git, worktrees, branches, PRs, SSH and their own repo's checks, and
they read diffs and spec artifacts as part of the job. They run several
Features at once, sometimes on a remote machine or a headless runner while the
laptop is closed, and come back to whatever is waiting on them.

Multi-user, team sync and RBAC are out of scope (`docs/roadmap/03-roadmap-6-months.md`),
so the user is one person with their own Projects. The primary audience is a
solo developer. Company engineers (GitHub Enterprise, self-hosted GitLab) are
supported but are not who the product is designed around first.

## Product Purpose

Demeteo is a desktop control plane for coding agents. The user describes a
Feature in plain language. Demeteo interviews them (Discovery), breaks the work
into dependency-gated tickets, and runs each ticket as a Feature through a
versioned Workflow. Each Step runs a coding-agent CLI in its own Git worktree,
locally, over SSH or on `demeteo-runner`. Work stops at human-approval Gates
before anything merges.

The bet, stated in the README: one-shotting an app gets you a demo, and then
the second feature breaks the first. The alternative is to settle the
decisions up front, split the work into pieces an agent can finish, send every
piece down the same research → spec → review → implement → validate → critic
path, and keep a human at the points that matter.

Success means the user can delegate a large body of work (the reference
walkthrough goes from one idea to 52 tickets), trust what lands, and spend
their own attention only at Gates. Every Step leaves an artifact they can read,
and every run can be reproduced from the Workflow version it pinned.

## Positioning

No other product combines all four of these (`docs/roadmap/02-opportunities.md`,
July 2026 competitive table):

- versioned multi-step Workflows with human Gates
- an isolated worktree for every Subtask
- local-first desktop execution plus remote SSH and headless runner execution
  that behave identically
- neutrality across agent vendors

The named competitors are Vibe Kanban (no gates, no versioned workflows, no
remote execution), Conductor (Mac-only, two agents, no pipelines) and Composio
AO (cloud-centric). The stated defence against agent vendors absorbing
orchestration is to stay a neutral multi-agent control plane: the more agents
Demeteo runs well, the less any one vendor's tool can replace it.

## Operating Context

- **Long-running, mostly unattended work.** A Feature runs for minutes to
  hours. The user keeps several in flight across Projects and comes back when
  a Gate, a failure or a finished run needs them. "What needs me" is the
  question the app answers most often (the rail badges, the "N gates need you"
  banner and the notification bell all serve it).
- **Review is the core interaction.** At a Gate the user reads the artifacts
  the pipeline produced (spec, task list, validation report, critic review,
  diff). Then they Approve, Redirect back into a loop with feedback, or Abort.
  Per-project gate autonomy (`attended` default, `review`, `clean`, `full`)
  sets how many Gates wait for them.
- **Dense technical material.** The app shows step graphs, live agent output,
  terminals (xterm, local or SSH), Monaco code and diff views, cost, tokens,
  durations and failure diagnostics.
- **Three transports, one experience.** A run on a local subprocess, over SSH,
  or detached on `demeteo-runner` must look and behave the same
  (`docs/EXECUTION_PARITY.md`). A remote run can be detached from the desktop
  and mirrored back.
- **Keyboard and power-user navigation.** A command palette (`Cmd/Ctrl+K`), a
  standard desktop shortcut set, and Escape/back handled through a single
  overlay ladder.
- **Other agents as clients.** An opt-in, OAuth-protected MCP endpoint lets
  another agent read run state and start Features or tickets. Approving Gates
  and merging are permanently excluded from it (decision 50).

## Capabilities and Constraints

**Ubiquitous language.** Use these terms in UI copy as AGENTS.md §1 defines
them: Project, Feature, Workflow, Step (`agent`, `sequence`, `gate`, `sync`,
`finalize`, `command`), Subtask, Gate, ProviderInstance. Discovery and ticket
are user-facing terms from the Discovery surface (`docs/PRD_DISCOVERY.md`).
A ProviderInstance is a GitHub or GitLab connection (`kind`, `host`, user),
not an AI model provider. See [Vocabulary](#vocabulary) for what the UI calls
it and for Project vs "Workspace".

**Shipped capability (v1.3.0).**
- Discovery interview, then a decompose pass. Tickets have stable ids. Nothing
  applies until the user clicks Apply.
- Ten starter Workflows, plus a visual DAG canvas for editing Workflows.
- Per-step agent, model and effort overrides, which can be changed mid-run.
- Gates with gate autonomy, and rework loops when validation fails.
- Remote SSH execution and the headless runner.
- Embedded terminals that show agent activity.
- GitHub and GitLab PR publishing.
- An MCP endpoint and an opt-in Memory Agent.
- Cost, token and duration telemetry per Step.

**Constraints the interface must respect.**
- **No pre-launch cost estimates.** Show only cost accrued in real time
  (decision 15).
- **No product telemetry** (decision 31).
- **Secrets live only in the OS keyring.** UI never echoes them back.
- **No real-time approval at the tool level.** A Step's permissions are
  compiled `allow`/`deny` before it runs. The Gate is the only human-in-the-loop
  surface, so the UI must never imply that a running agent can be steered
  mid-tool-call.
- **No surface may promise more than the harness carries.** Capabilities a
  harness lacks (e.g. Hermes reasoning effort) are shown as unsupported, not
  faked (AGENTS.md §2).
- **Works offline for local pipelines.** Review surfaces must not depend on a
  network fetch (audit F34).
- **Linux WebKitGTK cost.** Any running infinite CSS animation, even an
  opacity-only one, pins UI-thread CPU and GPU at the display refresh rate. On
  NVIDIA with WebKitGTK 2.54 the app renders on the CPU (`docs/KNOWN_ISSUES.md`).
- **Live preview is unavailable.** Components call Tauri IPC through typed
  wrappers in `src/lib/` and do not render in a plain browser. Visual
  iteration has to happen in `npm run dev:tauri` or on screenshots, not on a
  standalone browser preview.
- **Every surface on every OS.** Host-side surfaces must work on all three
  desktop OSes. On Windows, local terminals are `cmd.exe` and agent activity
  indicators come from the output scanner, not hooks.

**Known as-built debt.** `docs/ux-audit/findings.md` (F1–F49 plus 13
opportunities) records UI that drops input, labels that contradict behaviour,
status colours that drift between views (F27), and parallel implementations.
These are scheduled as Theme F (UX1–UX3) in the roadmap.

## Brand Commitments

- **Name:** Demeteo. **Tagline in use:** "A desktop control plane for coding
  agents." **Line in use:** "You make the calls — the agents do the execution."
- **License:** MIT. Open source at `github.com/stevenyepes/demeteo`, with a
  single primary maintainer. Open source only: no paid tier is planned, so no
  surface sells, upsells or gates features.
- **App icon:** `src-tauri/icons/`, `public/icon.png`.
- **Voice** (binding): direct, no hype. Observed alongside it in the README,
  docs-site and AGENTS.md: second person and technically exact. It states
  limits plainly (e.g. "Expected: the app isn't notarized"; "Antigravity is
  honestly de-scoped"). Labels say what the system actually did. The UX2 epic
  is literally titled "truthful UI".
- **Language:** UI copy is English today. Other languages are planned, so copy
  must be translatable: no strings built by concatenation, no meaning carried
  by letter case, and layouts that survive text roughly 30% longer.

## Evidence on Hand

- Real product screenshots (15): `docs-site/assets/screenshots/` covers
  Discovery, ticket graph, ticket editor, pipeline with a Gate waiting,
  implement tickets, critic review, diff, Gate, Workflow library and canvas,
  Project pipelines, start-feature overrides, create-project, and MCP settings.
- An 18-minute walkthrough video, in Spanish, from one idea to 52 tickets:
  `youtu.be/PDLcUuBeC6A`.
- Usage docs: `docs-site/` (published at `stevenyepes.github.io/demeteo`).
- A competitive and market snapshot: `docs/roadmap/01-market-research.md`
  (July 2026).
- An as-built UX audit: `docs/ux-audit/`.
- Demeteo Hub mockups (`docs/hub-design/`), whose UI is not built yet. They
  reuse the desktop's glass look, so the direction chosen for the desktop
  decides the Hub's too.
- Unbuilt terminal-session mocks (`ui-mocks/terminal-sessions/`).
- **None on hand:** testimonials, named users or customers, usage numbers,
  benchmarks, pricing, press. Do not fabricate any of them.

## Product Principles

1. **The human decides; agents execute.** Attention goes to Gates and
   decisions, not to watching output. Anything that needs the user must be
   findable from anywhere in the app.
2. **Truthful over reassuring.** A status, label or count shows what actually
   happened, including failure, absence and "unsupported". A failed run never
   looks alive, and a missing capability is never papered over.
3. **Every Step leaves something readable.** Artifacts, diffs and reasons are
   the product's evidence. Review surfaces are first-class, not a drawer.
4. **One experience regardless of where it ran.** Local, SSH and runner
   executions render identically, and calling code never branches on
   transport.
5. **Neutral across agents.** No surface privileges one vendor's CLI. Each
   agent's capabilities are shown as it actually declares them.

## Accessibility & Inclusion

Target: **WCAG 2.2 AA**. Observed today: keyboard-first navigation (command
palette, shortcuts, Escape ladder), ARIA attributes throughout `src/`, and
`prefers-reduced-motion` handling in `src/App.css`. Colour carries status
meaning (AGENTS.md §4), so status must never rely on hue alone: every status
colour ships with a label or icon.

## Vocabulary

The UI says **Project**, matching AGENTS.md §1 and the code. "Workspace" in
current UI copy is drift to be renamed. A Project can hold more than one
Repository (the `repositories` table keys on `project_id`).

The UI calls a ProviderInstance a **Git host** ("Connect GitHub or GitLab").
"Provider" in current UI copy is drift: next to coding agents, people read it as
an AI model provider.

## Open Decisions

None. Record new ones here as they come up.
