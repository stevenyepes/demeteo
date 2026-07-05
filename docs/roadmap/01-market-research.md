# Market Research — July 2026

> **Purpose:** Snapshot of the CLI coding agent, agent orchestration, and agent
> memory markets as of early July 2026, gathered to ground the
> [H2 2026 roadmap](03-roadmap-6-months.md). Each claim links to its source at
> the bottom; re-verify before acting on anything older than a quarter — this
> market moves monthly.

## 1. The CLI coding agent landscape

The market went from "a few options" in 2025 to **30+ terminal agents** in mid-2026.
Cost and reliability now matter as much as raw capability. The relevant fact for
Demeteo: **every serious agent now ships a scriptable, non-interactive mode with
structured output** — exactly the integration surface Demeteo's
`UnifiedCliRuntime` consumes.

### Agents Demeteo already supports

| Agent | Status | Notes |
|-------|--------|-------|
| opencode | ✅ shipped | `opencode run --format json` |
| claude-code | ✅ shipped | `claude --print --output-format stream-json` |
| hermes | ✅ shipped | Notable: hermes ships a native [Honcho memory integration](https://hermes-agent.nousresearch.com/docs/user-guide/features/honcho) — a reference point for our own Memory v2 work |
| antigravity | ⚠️ compiled but broken | See `docs/OPEN_QUESTIONS.md` §17 — npm CLI doesn't match the bundled parser. **Verified July 2026: headless surface is inadequate for integration — demoted to watch list** (see below) |

### Integration targets — verified non-interactive surfaces

**OpenAI Codex CLI** — `npm i -g @openai/codex`, runs GPT-5.5 by default, top of
several 2026 agent benchmarks. Headless surface is mature and CI-oriented:

- `codex exec "<prompt>" --json` → JSONL event stream on stdout (commands, file
  changes, agent messages as structured objects)
- `--output-schema ./schema.json -o out.json` → final response conforming to a
  caller-supplied JSON Schema (useful for our "Demeteo Brain" use cases: titles,
  PR descriptions as typed output)
- `--ephemeral` skips session persistence; `--sandbox` modes are the safety
  control; `CODEX_API_KEY` env for automation auth

**pi coding agent** (Mario Zechner / badlogic, `@mariozechner/pi-coding-agent`) —
minimal agent (<1k-token system prompt, ~54k GitHub stars). Four modes:
interactive, print/JSON, **RPC, and SDK**. RPC mode (`pi --rpc`) is a headless
JSON-over-stdio service — LF-delimited JSONL commands in on stdin, events out on
stdout. This is the *cleanest* integration surface of any agent we'd support:
a long-lived session protocol rather than one-shot spawn. Caveat from their
docs: split records on `\n` only (generic line readers break on Unicode
separators inside payloads).

**MiniMax** — two distinct things, don't conflate them:
- **MMX-CLI** (`mmx`) is a *multimodal generation* CLI (text/image/video/speech/
  music/vision/search), agent-friendly (`--non-interactive`, `--output json`,
  clean stdout/stderr separation, JSON tool-definition export) but **not a coding
  agent**.
- **Mini-Agent** is their open single-agent demo running MiniMax-M2.x, a model
  explicitly built for coding/agentic workflows with strong long-chain tool
  calling.
- Integration call: target Mini-Agent (or M2 via an OpenAI-compatible runner)
  for the coding-agent slot; optionally expose MMX-CLI later as a *tool*, not an
  agent.

**Google Antigravity CLI** — Google **retired Gemini CLI at I/O (May 19, 2026)**
and replaced it with Antigravity CLI (`agy`, Go binary, same harness as the
Antigravity desktop app). Gemini CLI stopped serving free/Pro/Ultra requests on
**June 18, 2026**.

*Headless-mode verification (July 2026):* the initial assumption that fixing
our adapter was urgent **did not survive verification** of `agy`'s
non-interactive surface:

- `agy -p "<prompt>"` exists, but by default non-read tools block on approval
  prompts that never render headless — effectively **query/answer only**.
- Tool execution (file writes, commands) headless requires
  `--dangerously-skip-permissions` (all-or-nothing YOLO mode; Google's own
  codelab demonstrates file creation this way). No granular approval policy.
- **No documented structured output**: the official codelab documents no JSON
  or streaming format for `-p` mode; reports indicate plain text with no result
  envelope, plus a known bug where `--print` output is **dropped under non-TTY
  stdout** (pipes/subprocesses) — exactly how Demeteo invokes agents. Some
  third-party posts mention `--output json` with a `.status` envelope; accounts
  conflict, likely version drift.
- One-shot only; no session continuation.

**Conclusion:** even "fixed," an Antigravity adapter today would be a degraded
text-scraping integration with unreliable stdout — below our adapter quality
bar. Demoted to the watch list with a per-release re-probe (the CLI iterates
fast; structured output may land). Google-model coverage is meanwhile served
through **pi**, whose unified LLM API drives Gemini-class models directly —
which is why pi moves up to the "Now" horizon in the roadmap.

### Second-tier candidates (watch list, not committed)

Amp (Sourcegraph), Cursor Agent CLI, Crush (Charm), Qwen Code, Aider, Droid
(Factory), Goose (Block). Decide per quarter based on user requests; the adapter
platform work (Theme A in the roadmap) is what makes each additional agent cheap.

### Strategic lesson from the Gemini → Antigravity churn

An agent CLI we integrate can be renamed, re-flagged, or killed **inside one
release cycle**. Bespoke per-agent parsers are a liability. The mitigation is an
**adapter conformance harness** (recorded golden transcripts per agent version +
CI probe that detects drift) and preferring session protocols (pi RPC) or
schema'd output (Codex `--output-schema`) over scraping human-oriented output.

## 2. Agent orchestration / "kanban for agents"

Gartner: multi-agent inquiries up **1,445%** Q1'24→Q2'25; 40% of enterprise apps
projected to embed agents by end of 2026. The emerging usage pattern is
three-tier: interactive pairing → parallel sprints → **overnight backlog
draining**. Demeteo currently serves tier 2; the kanban + CLI backlog items are
precisely tiers 2→3.

Direct competitors:

| Product | What it is | Strengths | Gaps Demeteo can exploit |
|---------|-----------|-----------|--------------------------|
| **Vibe Kanban** (BloopAI, OSS) | Kanban board (CLI + web UI) that plans tasks and runs agents in parallel, one git worktree per task; MCP client support | Category leader mindshare ("kanban for agents" = Vibe Kanban); broad agent support (Claude Code, Codex, Amp, Cursor, Gemini) | No human-approval **gates**, no versioned multi-step **workflows**, no memory system, no remote/SSH execution; review UX is thin |
| **Conductor** (macOS app) | Parallel Claude Code/Codex agents in worktrees, central dashboard, diff-first review | Polished native UX | Mac-only, two agents only, no workflow/pipeline concept |
| **Composio AO** | Full automation: agents in isolated worktrees, one PR each, single dashboard | Automation depth | Cloud-centric; no local-first story |
| Terragon, Omnara, Claude Squad, Sculptor | Assorted cloud/TUI agent fleet managers | — | Fragmented; none combine desktop control plane + gates + memory |

**Positioning takeaway:** nobody else combines (a) versioned workflows with
human gates, (b) local-first desktop + remote SSH execution, and (c) a learning
memory layer. The kanban feature should be built *on top of* those
differentiators — "a board where every card can carry a workflow and a gate
policy" — not as a Vibe Kanban clone.

## 3. Agent memory: standards and engines

### Google Open Knowledge Format (OKF)

Google released **OKF v0.1** (Apache 2.0, vendor-neutral): knowledge represented
as **a directory of Markdown files with YAML frontmatter**, readable by humans
and agents, explicitly designed so agent memory is *portable, versioned, and
tool-independent* — a standardization of the "LLM wiki" pattern. No proprietary
account or SDK required to read/write/serve.

Fit for Demeteo: excellent. Our memories (conventions, lessons, decisions,
preferences, facts) map 1:1 to OKF documents; a per-project OKF directory can
live *in the repo or beside it*, be git-versioned, be hand-edited, and be read
natively by the very coding agents we orchestrate (they all read Markdown). It
converts our memory from an opaque SQLite+embeddings store into a user-ownable
asset. OKF is v0.1 — early. Adopting early is cheap (it's just Markdown) and
positions Demeteo as one of the first orchestrators with OKF-native project
memory.

### Honcho (Plastic Labs)

- Memory *service*: API server + background "Deriver/Summarizer/Dreamer" worker,
  Postgres + Redis, self-hostable, **AGPL-3.0**, managed cloud at api.honcho.dev
  (usage-priced ~$2/M tokens ingested).
- Model: **peers** (humans *and* agents are first-class), sessions, messages, and
  a **Dialectic API** — a natural-language oracle endpoint ("what does this peer
  know/prefer?") used to hydrate prompts.
- Benchmarks: strongest reasoning depth of the field (90.4% on LongMem-class
  evals vs ~65% for first-wave tools).
- Constraints for us: AGPL is incompatible with linking into an MIT app —
  integration must stay **out-of-process over its HTTP API** (adapter behind a
  port, like our provider adapters). Postgres+Redis is heavy for a desktop
  default → Honcho must be *opt-in* (self-host or cloud), never required.

### The rest of the field (for the pluggable-backend decision)

Mem0 (fast setup, extraction+vector, $19/mo, HIPAA/SOC2), Letta (memory-as-OS
runtime), Zep/Graphiti (temporal knowledge graph), Cognee (corpus graph).
Rule of thumb from third-party testing: preferences → Mem0-style extraction;
time-scoped facts → Zep; entity graphs → Cognee; durable agent identity → Letta;
deep multi-peer reasoning → Honcho.

**Takeaway:** the field has *not* converged on one engine, which argues for a
`MemoryBackendPort` with our current local embeddings as the default adapter and
Honcho as the first alternative — rather than betting the schema on any single
vendor. OKF is the *storage/interchange format* bet (safe, it's Markdown);
Honcho is an *engine* bet (hedge via port abstraction).

## Sources

- [Every AI Coding CLI in 2026: The Complete Map (30+ Tools)](https://dev.to/soulentheo/every-ai-coding-cli-in-2026-the-complete-map-30-tools-compared-4gob)
- [CLI Coding Agents: 2026 Q2 Comparison](https://wal.sh/research/2026-q2-cli-coding-agents/)
- [The 2026 Guide to Coding CLI Tools — Tembo](https://www.tembo.io/blog/coding-cli-tools-comparison)
- [Codex non-interactive mode — OpenAI Developers](https://developers.openai.com/codex/noninteractive)
- [Codex Exec in CI: headless guide](https://www.developersdigest.tech/blog/codex-exec-ci-headless-guide)
- [pi-mono repository (badlogic)](https://github.com/badlogic/pi-mono) · [pi RPC protocol docs](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/rpc.md) · [pi.dev](https://pi.dev/) · [Author's design post](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)
- [MiniMax MMX-CLI announcement — MarkTechPost](https://www.marktechpost.com/2026/04/12/minimax-releases-mmx-cli-a-command-line-interface-that-gives-ai-agents-native-access-to-image-video-speech-music-vision-and-search/) · [MiniMax Mini-Agent](https://github.com/MiniMax-AI/Mini-Agent) · [MiniMax-M2](https://github.com/MiniMax-AI/MiniMax-M2)
- Antigravity headless verification: [Hands-on with Antigravity CLI — Google Codelabs](https://codelabs.developers.google.com/antigravity-cli-hands-on) (tool use requires `--dangerously-skip-permissions`; no structured output documented) · [agy headless non-TTY stdout problem — Antigravity Lab](https://antigravitylab.net/en/articles/integrations/antigravity-cli-agy-headless-non-tty-stdout-ci) · [Headless design before CI/cron — Antigravity Lab](https://antigravitylab.net/en/articles/integrations/antigravity-cli-headless-non-interactive-ci-design) · [agy hands-on guide — DEV](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7)
- [Vibe Kanban](https://www.vibekanban.com/) · [GitHub — BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban) · [VirtusLab review](https://virtuslab.com/blog/ai/vibe-kanban) · [Show HN thread](https://news.ycombinator.com/item?id=44533004)
- [Vibe Kanban vs Paperclip vs Dispatch — MindStudio](https://www.mindstudio.ai/blog/vibe-kanban-vs-paperclip-vs-claude-code-dispatch) · [Vibe Kanban alternatives 2026](https://nimbalyst.com/blog/best-vibe-kanban-alternatives-2026/)
- [The Code Agent Orchestra — Addy Osmani](https://addyosmani.com/blog/code-agent-orchestra/) · [Top AI Coding Trends 2026](https://beyond.addy.ie/2026-trends/)
- [Open-source agent orchestrators — Augment Code](https://www.augmentcode.com/tools/open-source-agent-orchestrators)
- [What Is the Open Knowledge Format (OKF)? — REVERB](https://reverbico.com/blog/what-is-the-open-knowledge-format/) · [Google Cloud blog on OKF](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/) · [OKF: When Docs Become Agent Memory — Document360](https://document360.com/blog/open-knowledge-format/) · [OKF standardizes the LLM-wiki pattern](https://themenonlab.blog/blog/google-okf-open-knowledge-format-karpathy-llm-wiki-standard)
- [Honcho — plastic-labs/honcho (AGPL-3.0)](https://github.com/plastic-labs/honcho) · [Honcho self-hosting](https://honcho.dev/docs/v3/contributing/self-hosting) · [Honcho review 2026](https://andrew.ooo/posts/honcho-plastic-labs-agent-memory-review/)
- [Mem0 vs Letta vs Zep vs Cognee 2026](https://mcp.directory/blog/mem0-vs-letta-vs-zep-vs-cognee-2026) · [Agent memory frameworks tested — Particula](https://particula.tech/blog/agent-memory-frameworks-tested-mem0-zep-letta-cognee-2026) · [Mem0 vs Honcho](https://mem0.ai/compare/mem0-vs-honcho)
- [Best AI agent frameworks 2026 — LangChain](https://www.langchain.com/resources/ai-agent-frameworks) (Gartner figures)
