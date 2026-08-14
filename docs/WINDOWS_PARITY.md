# Windows Parity — The Plan, and the Decision Behind It

> **Phases 0–4 are implemented on `feat/native-windows-local-execution`; Phase 5
> is not.** The tree compiles, links and unit-tests on `windows-latest`.
>
> A Windows desktop has now been driven by hand as far as **creating a project
> and starting a feature**, both of which work. The first agent step did not:
> `codex` resolves to the `.cmd` shim npm installs, and a prompt cannot be
> passed as an argument to one — see [Phase 2](#phase-2--spawning-is-correct-and-children-die-when-told).
> That is fixed, and everything past it is still unobserved. A *completed* run,
> the DACL fence denying a real agent, and an authenticated push remain
> reasoned rather than executed; treat every claim about them as a prediction.
>
> Separately, three POSIX signals Demeteo emitted *regardless of platform* have
> been removed or corrected, and a fourth question — what codex's sandbox flag
> is backed by on Windows — is now instrumented but open. That work is under
> [What Demeteo itself was telling the agent](#what-demeteo-itself-was-telling-the-agent),
> and no Windows run has confirmed any of it either.
>
> Since then the `cfg(windows)` bodies are at least *executable* from Linux —
> `scripts/check-windows.sh --run`, whose blind spots are set out under
> [Gates](#gates). It is not a substitute for any of the above. Read
> [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) first — the transport contract
> this has to extend rather than contradict. The rejected design this document
> argues against is fc8d65c's, kept below because the reasoning is the point.

## The decision: one body, one shell

**A user-authored command is one POSIX script body on every platform.** Demeteo
never asks for a second body, never translates between shells, and never stores
two versions of the same command. On Windows that body is executed by the bash
that Git for Windows already installs at `<gitroot>\bin\bash.exe`.

Everything **Demeteo itself** invokes — git, filesystem mutation, probes, agent
spawn — runs as structured argv with no shell on any platform.

That is two planes split by **authorship**, not by operating system. The plane
split is the whole design; the rest of this document is consequences.

### The agent has to be told, or it decides for itself

An agent reading a Windows worktree while the prompt quotes the project's POSIX
gate commands will conclude the prompt is stale and helpfully rewrite them, which
turns one body into two at the only layer no gate inspects. So the agent prompt
carries a Windows-only block naming the OS, the bash those commands run under,
and the prohibition on translating them —
`crates/demeteo-core/src/domain/platform_context.rs`, which also records why
keying on platform is not the transport branch §2 forbids. A change to which
interpreter runs a user's script is a change to what that block claims.

The prohibition alone is not enough, because the agent's *own* command tool is a
third author this two-plane split does not cover. Which interpreter sits behind
that tool is the harness's choice — codex composes for PowerShell where Claude
Code runs the same bash Demeteo does — and an agent whose shell cannot parse a
POSIX body has been forbidden the rewrite without being given an alternative,
while the shipped workflow templates ask it to run `{{test_command}}` by hand.
So the block also hands it the resolved interpreter to wrap the body in,
unchanged. **Wrapping is not translating**: the bytes between the quotes are
what the conformance suites compare, and only a rewrite changes them.

Each harness declares its shell as an `AgentCapabilities` field, never a `match`
on the agent kind (AGENTS.md §3). The declaration is a claim about a
third-party binary that no gate here can check, so `WindowsAgentShell::Unknown`
is the honest value where upstream gives no stable answer — and it renders a
block that promises nothing about syntax.

Four of the five are settled from upstream source or documentation:

| harness | shell | what decides it upstream |
|---|---|---|
| `codex` | PowerShell | `default_user_shell_from_path` returns PowerShell under `cfg!(windows)` before reading the user's shell — unconditional |
| `opencode` | PowerShell | `Shell.preferred` takes `$SHELL`, else the head of `[pwsh, powershell, gitbash, COMSPEC]`; Demeteo sends no `SHELL`, so the bash candidate is third and unreachable |
| `pi` | Git Bash | resolves the two Program Files roots then `bash.exe` on PATH, and raises rather than falling back to a native shell |
| `claude-code` | Git Bash | Git for Windows is optional upstream (without it the tool is PowerShell), but Demeteo refuses to run without that same bash, so the condition holds |
| `hermes` | `Unknown` | upstream calls native Windows experimental and directs users to WSL2 |

`claude-code` is the one Demeteo *makes* true rather than predicts. Upstream is
rolling the PowerShell tool out progressively alongside Bash on installs that
have Git for Windows, so the declaration would otherwise rest on guessing a
rollout cohort. `CLAUDE_CODE_USE_POWERSHELL_TOOL=0` is set per invocation, never
written to the user's own `settings.json`, which §2 forbids.

That pin looks Windows-shaped and is not, which is why it does not ride
`static_env` with the hygiene switches: the same tool is **opt-in on Linux and
macOS**, so an unconditional `0` would strip it from Demeteo's agents for a user
who deliberately enabled it there — on a platform with no declaration to defend.
`domain::agent_env::pinned_shell_env` owns that narrowing, so it is a
synchronous decision with a test rather than a condition buried in a spawn path.
The general rule: a harness switch is only Demeteo's to override where Demeteo
makes a claim that depends on it.

One thing this does **not** establish is precedence. Upstream documents the
variable as settable "in your environment or in `settings.json`" without saying
which wins when they disagree, so a user who set `1` in `settings.json` may or
may not be overridden by the process env Demeteo sets. Worth an observation on a
Windows box before anyone treats the pin as absolute.

`opencode` remains a default its own `shell` key can move, and no equivalent
per-invocation switch exists: `$SHELL` is the only lever, and Demeteo
deliberately sends none to a Windows agent (`domain/agent_env.rs`) because that
variable is the loudest false POSIX claim available. So a user who sets that key
gets a block that is wrong about their syntax. It stays followable regardless —
the wrapper and the prohibition do not depend on which shell the agent turned
out to have.

### Why not two script bodies

fc8d65c's `ScriptVariants { posix, powershell }` does not remove the transport
branch — it relocates it from adapter code, where a conformance suite can see
it, into **user data**, where nothing can. The runner is always Linux. So the
same Step in the same Feature executes a *different program* depending on which
machine the scheduler picked, and the only proposed guard catches a *missing*
twin, never a **stale** one. Edit the POSIX body, leave the PowerShell body
alone, and Windows and the runner silently measure different things forever.
§2's parity invariant is not satisfiable under that shape.

It also does not work today, for a reason already encoded in the branch:
`git_ops/strategy.rs:240`'s `posix_script()` is the *only* constructor
autodetection uses (`strategy.rs:101,102,110,115`), so every autodetected
`test`/`build`/`prepare` command and every named harness has `powershell: None`
— and `run_script`'s Windows arm returns *"configuration error: this Windows
project has no PowerShell script variant"*. **No Windows project runs out of
the box**, and nothing validates it at save time.

The closest analogue in the wild carries the same complaint: `just`'s
`set windows-shell` (issues #1050, #3202).

### Why Git Bash is not a new dependency

Demeteo already hard-requires Git. Git for Windows' `compat/mingw.c::
setup_windows_environment` prepends `<root>\usr\bin` to `PATH` for every child
git spawns, and `mingw_spawnvpe`/`parse_interpreter` resolves `#!` shebangs
itself — which is why a project's own `commit-msg` hook runs on Windows today,
with no `ScriptVariants` involved. A design where `default_test_command` needs a
PowerShell twin while the hook it triggers runs under bundled `sh` is
internally inconsistent.

This is also what GitHub Actions concluded: `shell: bash` on Windows *is* Git
for Windows' bash, invoked as
`C:\Program Files\Git\bin\bash.EXE --noprofile --norc -e -o pipefail`.

The interpreter's **path** differs per platform; the **program text** does not.
That is the same category of difference as glibc-vs-musl, which the parity
contract already tolerates — and it is the only shape under which
`run-topology-conformance.sh` can assert byte equality of the composed body
across transports.

### Why not PowerShell, on correctness grounds alone

Two facts are independently disqualifying for a verdict-bearing path:

- **Windows does not ship `pwsh`.** It ships Windows PowerShell 5.1
  (`powershell.exe`). PowerShell 7 is a separate install. fc8d65c's
  `Command::new("pwsh")` is therefore a *new hard prerequisite strictly weaker
  than the Git dependency Demeteo already has*.
- **`pwsh -Command` destroys exit codes.** Per `about_Pwsh`: an external
  program's exit code other than 0 or 1 "is converted to 1 for process exit
  code". Demeteo's entire D3 verdict taxonomy and the C6 environment-vs-
  regression triage read `status.code()`. A false green in a human-approval-
  gated orchestrator is the worst failure mode available.

PowerShell returns in Phase 5 as an **explicit opt-in** `ShellSpec` for
genuinely Windows-native work (MSBuild, signtool, .NET) — one body, chosen by
the author, never a required twin. Its invocation is then the Actions runner's:
`-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <temp>.ps1`
with an `exit $LASTEXITCODE` epilogue. Even then its failure taxonomy is
degraded and must be documented as such: a `CommandNotFoundException` exits 1
where `sh -c` exits 127, and `environment.rs`'s triage keys on that distinction.

### Why not WSL

A WSL2 distro is registerable **today** as an ordinary SSH machine. It costs no
new code and adds no fourth transport. It stays the documented escape hatch for
Linux-only toolchains, not the strategy — requiring it would make the Windows
desktop a client of a Linux box rather than a supported host.

---

## What Demeteo itself was telling the agent

The one-body decision governs what *executes*. It says nothing about what an
agent **believes** it is standing on, and that is the failure the first Windows
desktop actually hit: codex reached for bash tooling on a machine where Demeteo
had already resolved a bash and was invoking it. The agent was not guessing.
Three things Demeteo emitted unconditionally said POSIX, and none of them was
sensitive to the platform in any way:

| The signal | Why it read as POSIX | Where the rule lives now |
|---|---|---|
| `SHELL` and `TMPDIR` forwarded from the GUI process into every agent spawn | A Demeteo launched from a Git Bash terminal exports `SHELL=/usr/bin/bash`, so the orchestrator handed a Windows agent a POSIX identity and was then surprised by POSIX behaviour | `domain/agent_env.rs` — forwarded to a POSIX target and to nothing else, plus a strip pass over every non-PTY child |
| The prompt named no operating system, while `{{harness_baseline}}` quoted the project's POSIX gate commands verbatim on every platform | The only OS evidence in the prompt pointed one way | `domain/platform_context.rs` — a Windows-only block, placed above the command text it reframes |
| `-c sandbox_mode=…` chosen for codex with no platform in the decision | Not a claim the agent reads, but a posture Demeteo asserts without knowing it holds | `domain/models/sandbox.rs` — the platform reaches the arg builder and the table records what is known per platform |

The first two are corrections. The third is **not**, and reading it as one is
the mistake this section exists to prevent.

### The codex sandbox question is instrumented, not answered

Codex's two published sandbox backends are POSIX kernel facilities — Seatbelt on
macOS, Landlock/seccomp on Linux. Whether its Windows build carries a third has
not been observed here, so the Windows entry is `Unknown` and Demeteo still
sends the identical bytes it sends on Linux. An open question must not change
what ships; the seam exists so that answering it later is an edit to one table
arm rather than a redesign.

Settling it needs no code change, only a capture. `DEMETEO_AGENT_TRACE` writes
an agent's raw stdout verbatim, including the `command_execution` items the
event parser drops — [`AGENT_INTEGRATION.md`](../AGENT_INTEGRATION.md) §5.2.1
covers turning it on. One codex turn on a Windows desktop either runs with the
mode accepted or names the unsupported sandbox on its own stream.

### What is still a prediction

This work was written, tested and reviewed on macOS. Nothing added sits behind a
`#[cfg]` — the platform travels as data through the port, so `Platform::Windows`
is exercised by the Linux and macOS suites as an ordinary value. That is why the
compile gates say nothing here: there is no Windows-only body to miscompile, and
equally no Windows-only body whose *behaviour* anything has checked.

- **That the corrections change codex's behaviour at all.** The chain from
  "`SHELL` said bash" to "the harness reached for bash utilities" is inferred
  from the environment block and the prompt, not read off a capture. The first
  Windows capture is the evidence and it does not exist yet.
- **That no agent depends on the two variables.** The adapters drive third-party
  CLIs whose behaviour on Windows without `TMPDIR` is untested; the reasoning
  that they lose nothing is recorded in `domain/agent_env.rs` and is reasoning,
  not observation.
- **That the prompt block is sufficient.** It is instruction — see the gap
  below.

---

## What fc8d65c got right, and what has to go

The branch is roughly half salvage. Its argv migration is the proof this
direction works: `git_ops/{squash,sync,merge,clone,health,strategy}.rs` went
from 56 shell strings to zero.

**Keep:** `ProgramRequest`/`run_program` · the filesystem port methods
(`create_dir_all`, `remove_dir_all`, `remove_file`) · the `git_ops` argv
migration · the Job Object *concept* · the `TrustedWorktreePort` shape in
`ports/worktree_ops.rs` · the SSH-side additions · the `windows-latest` CI job.

**Revert or rebuild:**

| Thing | Why |
|---|---|
| `ScriptVariants` and every consumer | Relocates the transport branch into user data; autodetection can only emit `powershell: None` |
| The four `project_settings` command columns' JSON encoding | `parse_script` runs `serde_json::from_str::<ScriptVariants>` against columns holding a bare `npm test`. **`get_settings()` returns `Err` on every existing install** — settings, feature launch, and the settings panel all dead. There is no migration; V38 is still the highest |
| `HarnessesColumn`'s untagged→struct rewrite | A legacy bare-map row now parses as all-`None`, **silently dropping every configured harness** |
| The `pwsh` requirement and `-Command` invocation | See above |
| The icacls fence + `.demeteo/scope-acl.txt` | `/save`+`/restore` matches by relative filename, so it skips every file the agent created and leaves stale entries for every one it deleted — it is not an inverse. The mask `(WD,AD,DC)` **omits `DELETE`, so an agent can `rm src/main.rs` straight through the fence today**. The snapshot lives inside the tree the fence bounds, which is what forced the `#[cfg(windows)]` git-status filter in `scope.rs` — itself a transport branch in calling code |
| `WindowsJob::attach` after `spawn()` | Job membership is forward-only, so grandchildren spawned before assignment escape permanently; `AssignProcessToJobObject` returns `ERROR_ACCESS_DENIED` on an already-exited process, so a fast command fails *for succeeding*; and `attach` returns `Err`, making a best-effort teardown guarantee a precondition for running anything |
| `set_file_mode` / `is_executable` on the port | `set_file_mode` is a documented Windows no-op whose only caller protects a file containing a provider PAT; `is_executable` returns `!is_dir()` on Windows, i.e. every regular file is a runnable git hook. A port method a transport can only no-op or lie about is a parity break *inside* the contract |
| The two-field script editors | The UI cost of asking every user to author and forever synchronise two bodies, with nothing detecting drift |

### Three regressions fc8d65c ships to Linux and macOS

These are not Windows problems and must be fixed first, in Phase 0:

- **`ScriptRequest` has no `login_shell` and no `interactive`**
  (`ports/execution.rs:37`), and `steps/command.rs:260` now runs on it. So a
  `command` node no longer resolves mise/asdf/nvm-managed tools — the exact
  two-call-sites-disagree-about-the-shell failure `harness_shell.rs`'s module
  header says it exists to prevent, and which its rustdoc records as having
  already caused one production failure.
- **`merge_stderr_into_stdout` was dropped** from the command node. A green
  `cargo test` or `npm run build` reports on stderr, so `command-output` is
  filed empty.
- **Failure feedback prints `step_exec.step_id.0` instead of the command**, in
  three places. That is the text the rework agent reads.

Plus `remove_terminal_worktree` (`git_ops/worktree.rs:189`) now returns `Err`
unconditionally, killing the feature #109 landed two commits earlier.

---

## The phases

Each is independently shippable. The ordering answers one question: **what is
the shortest path to a real Windows user completing a feature run, without
shipping a degradation to the platforms that work today?**

The ~110 remaining Demeteo-owned shell strings migrate to `run_program`
**alongside the phase that already touches their subsystem** — not as a Phase 0
monolith. Shell-string-to-argv conversion is mechanical but not risk-free:
quoting a POSIX shell strips becomes literal under structured argv, and
`artifacts/declared.rs:250` builds `git add -A{add_paths}` from pre-quoted
`':!name'` pathspecs that only work *because* a shell strips them. Migrating
those while already exercising that subsystem is what makes the risk visible.

### Phase 0 — One body, compiling, three-OS CI

Revert `ScriptVariants` to a single `body: String` across `ports/execution.rs`,
`domain/models/script.rs`, `domain/verifier`, `adapters/database/repos/project.rs`,
`git_ops/strategy.rs`, and the settings UI. This restores the pre-branch on-disk
encoding, so **no V39 migration is owed and no existing install breaks** — the
single largest risk removed by construction rather than managed.

Fold the full former `ShellOptions` contract into one `ScriptRequest`
(`body`, `shell`, `login_shell`, `interactive`, `cwd`, `env`, `timeout`) and
converge the ~15 user-authored-script callers on it: `harness_shell.rs`,
`steps/command.rs`, `baseline/mod.rs`, `driver/verifier/mod.rs`,
`Machine.setup_commands`. Two method families with different contracts is how
the regression above happened; one method is the structural fix.

Fix the three Linux regressions and the remaining 99 compile errors. Fix
`domain/command_step/spec.rs:78,83` — `validate_relative_cwd` checks
`starts_with('/')` and `split('/')`, so it accepts `C:\Windows`,
`\\server\share`, and `..\..\..`; this bounds a command node's blast radius and
must be correct before any Windows build exists. Route
`ports/agent_runtime.rs:215,219`, `application/agents.rs:36`, and
`application/agent_probe.rs:48` through `exec.resolve_home`/`resolve_user`
instead of reading `std::env::var("HOME")` directly.

Add `windows-latest` and `macos-latest` jobs to `pr-checks.yml` running clippy
`-D warnings`, `cargo test -p demeteo-core`, and `tsc` on the pinned toolchain.
**Compile coverage alone would have caught all 99 errors**, and everything in
this plan lives behind `#[cfg(windows)]` and is therefore invisible to the
ubuntu-only gate — the branch's own `scope_windows.rs` is never executed.

*Shippable:* `npm run checks` green; the tree compiles and unit-tests on all
three targets for the first time; an existing installation's project settings
open exactly as before fc8d65c; a `command` node resolves mise-managed tools and
files a non-empty artifact for a green `cargo test` again.

### Phase 1 — The shell resolves, and user scripts run on Windows

`shell_invocation`'s return type becomes `(PathBuf, Vec<String>)`. The Unix arms
return the same `bash`/`sh` and the *same argv*; the Windows arms return the
resolved Git Bash. The `Vec<String>` is produced by identical code on every
platform, so `command_body`/`export_prefix`/`job_control_prefix` stay literally
shared.

Resolution order, cached and persisted as a probed machine capability:
`DEMETEO_BASH_PATH` override → registry `HKLM\SOFTWARE\GitForWindows\InstallPath`
then `HKCU\...` (the per-user install lands in `%LOCALAPPDATA%\Programs\Git`),
opened under **both** `KEY_WOW64_64KEY` and `KEY_WOW64_32KEY` → derived from git
(`git --exec-path` returns *forward slashes*, so pop three components with
`Path::parent`, never a split on `\`) → well-known directories. Validate the
candidate once with `-c 'echo ${BASH_VERSION:-none}'` and reject `none`, which
rejects the BusyBox MinGit variant that ships `ash`.

**Never PATH-search for a bare `bash`.** `C:\Windows\System32\bash.exe` is the
WSL launcher, System32 precedes Git in `PATH`, and WSL bash cannot see the
Windows paths we pass. This is actions/runner #786 and #216 and GitPython's
Windows hook bug — the same mistake three times.

Prefer `<root>\bin\bash.exe` over `<root>\usr\bin\bash.exe` (the former arranges
the MSYS PATH view) and never `<root>\git-bash.exe` (a mintty launcher: it
detaches and returns no exit code).

Add a `MissingPosixShell` preflight verdict naming MinGit explicitly — a machine
can have a working `git.exe` and no `bash.exe`, and that must fail loudly with a
named remediation, never fall back silently.

Rebuild the child `PATH` from `HKCU\Environment` + `HKLM\SYSTEM\CurrentControlSet\
Control\Session Manager\Environment` rather than trusting `std::env::var("PATH")`:
a Tauri GUI launched from Explorer holds the environment block captured at logon
and never sees `WM_SETTINGCHANGE`, so a tool installed while Demeteo is running
is otherwise invisible until restart.

*Shippable:* a project autodetected on Linux runs its unmodified `npm run checks`
on Windows with no second body and no configuration change; a shell-parity
conformance case (both streams, non-ASCII UTF-8, `exit 42`) produces identical
stdout, stderr, and exit code on local-Linux and local-Windows; a MinGit-only
machine gets a named, actionable failure.

### Phase 2 — Spawning is correct and children die when told

PATHEXT-aware executable resolution over the reconstructed PATH. `CreateProcessW`
does not apply PATHEXT and Rust's `Command` resolves only `.exe`, so
`run_program("npm", …)` fails on Windows while succeeding on Linux — a parity
break with a near-100% hit rate for Demeteo's workload. One resolver serves
`local_run_program` and `adapters/agent/mod.rs`'s availability probe, so
"available" and "runnable" can no longer disagree.

One spawn-hygiene helper beside `sanitize_child_env`, applied at every non-PTY
spawn: `CREATE_NO_WINDOW` (nothing in the tree sets it, and
`src-tauri/src/main.rs` uses `windows_subsystem="windows"` in release — so a
packaged build **flashes a console per spawned process**, and this does *not*
reproduce under `npm run dev:tauri`) · `NoDefaultCurrentDirectoryInExePath=1`
(`CreateProcess` searches CWD before PATH and Demeteo's CWD is an agent-written
worktree — GHSA-2mqj-m65w-jghx, and it reinforces the §2 fence) · stripping
`MSYSTEM`/`MSYS`/`MSYS2_*` from the inherited env, because git guards its own
`<root>\usr\bin` PATH augmentation on `MSYSTEM` being unset and forwarding an
inherited value silently disables shebang resolution for every spawned git.

Rebuild the Job Object: create with `KILL_ON_JOB_CLOSE | BREAKAWAY_OK` **before**
spawn, spawn `CREATE_SUSPENDED`, assign, `ResumeThread`. `BREAKAWAY_OK` but
deliberately **not** `SILENT_BREAKAWAY_OK`: Hermes launches its gateway with
`CREATE_BREAKAWAY_FROM_JOB`, which fails `ERROR_ACCESS_DENIED` inside a job
lacking the bit, while silent breakaway would let ordinary agent children escape
the tree kill entirely. Assignment failure must **degrade** to `kill_on_drop`
and a debug log, never `Err`.

Map `io::ErrorKind::InvalidInput` at spawn to a distinct **non-retryable
configuration error**: post-CVE-2024-24576, std refuses to spawn `.bat`/`.cmd`
with unescapable arguments, and Demeteo passes agent prompts and ticket text as
arguments — so this *will* fire. If it reads as a harness failure it feeds the
C6 rework loop something no ticket can close.

It fired on the first real Windows run, at the first agent step, exactly as
described, and the classification held: the run stopped with a configuration
error instead of asking an agent to repair source that had never executed.
Classifying it is not enough to make an agent *run*, though, and quoting
harder cannot help — `cmd.exe` truncates its command line at a literal
newline, so a multi-line prompt is unrepresentable to a batch target no matter
how it is escaped. `shared/win/npm_shim.rs` therefore recognises the fixed
shape npm emits and launches the `node.exe` and package entrypoint behind the
shim directly, where arguments reach `CreateProcessW` with no `cmd.exe` in the
path. It reads the shim only as a path shape, never as batch code, and refuses
an entrypoint that climbs out of its `node_modules` directory.

Harden the one-shot agent path: closed stdin, a mandatory wall-clock deadline,
and treat `exit 0` with empty stdout as a **failure**, not an empty successful
turn. Documented Windows bugs in the agent CLIs produce exactly that signature,
and the verifier would otherwise fabricate a green verdict.

*Shippable:* an agent step runs end to end on Windows; a release build spawns
dozens of children with no console flash; a test that spawns a grandchild proves
it is reaped on timeout — **watched failing against the Phase 1 code first**
(§7).

### Phase 3 — Worktrees provision, fence, and tear down

Shorten on-disk path segments to 8-hex id prefixes, full ids in SQLite, and
default the Windows workspace root short. This is the *real* fix for MAX_PATH:
std's `maybe_verbatim` makes `std::fs` long-path-safe, but
`sys/process/windows.rs::make_dirp` deliberately strips the `\\?\` prefix before
passing `lpCurrentDirectory` to `CreateProcessW`, which hard-fails past MAX_PATH
**regardless of `core.longpaths` or the registry key**. So the *agent spawn*
fails before `node_modules` does. `core.longpaths=true` and
`<longPathAware>true</longPathAware>` in the Tauri manifest still help the child
toolchains.

Rewrite `remove_dir_all`'s local impl as a readonly-clearing, backoff-retrying
depth-first walk (`symlink_metadata`, never `metadata`; clear
`FILE_ATTRIBUTE_READONLY`; retry codes 5/32/145 to ~2s). Today's bare
`std::fs::remove_dir_all` **cannot delete a git repo on Windows at all** — git
writes loose objects `0444` — while the SFTP impl succeeds. That is a *contract*
violation, not a local quirk. Teardown ordering: job → ACE → `git worktree
remove --force --force` (single `-f` refuses a *locked* worktree, which is
exactly what a killed step leaves behind) → `prune` → residue → a persistent
cleanup queue retried at startup **with a visible notice**.

Write `core.autocrlf=false` **once, persistently, into the Demeteo-owned clone's
own `.git/config`** — never the user's config, and never on a command line. Git
for Windows defaults to `autocrlf=true`, so a repo whose test command runs
`./scripts/checks.sh` gets a file bash rejects with `bad interpreter`: green on
the Linux runner, red on Windows. But forcing it per-command is how opencode
#27276 happened — index LF vs worktree CRLF makes every file read as modified,
which here would make `verify_and_revert_out_of_scope_writes` classify the whole
tree as out-of-scope and **`git checkout` away the step's real work**. Record
that constraint in `git_ops/mod.rs`, where the wrong edit would be made.

Move dependency-cache exclusion from pathspec-shape to `.git/info/exclude` **on
all platforms** — a mechanism change, not a Windows special case, so captured
artifacts do not differ by platform for the same feature.

Migrate this subsystem's shell strings: the eight `rm -rf`, six `chmod -R u+w`,
four `mkdir -p`, and the `test -d` probes in `setup.rs`, `repo_probe.rs`, and
`snapshot.rs` that **fail closed** today — their shell failure makes every step
report "worktree missing" and every bootstrap report "not cloned".

*Shippable:* clone → provision → agent step → harness → artifact capture → merge
back → teardown runs on a real repository on `windows-latest`, twenty times, with
no leaked worktree.

### Phase 4 — The fence, natively

Replace icacls with one **inheritable** DENY ACE per non-writable top-level entry
via `SetEntriesInAclW` + `SetNamedSecurityInfoW`, `CONTAINER_INHERIT_ACE |
OBJECT_INHERIT_ACE` so files created *afterwards* are fenced too. Trustee from
`GetTokenInformation(TokenUser)`, not `whoami` — which returns
`AzureAD\Display Name` on Entra-joined machines and is locale- and rename-proof
only as a SID. Mask must include `DELETE` and `FILE_DELETE_CHILD`. Skip
`writable_paths` per entry, mirroring the Unix `chmod a-w` shape exactly; a
root-level inheritable deny would propagate into `artifacts/` and break the
agent's legitimate writes. Teardown is `REVOKE_ACCESS` for the same trustee — a
true inverse, no snapshot, no `/t` walk, O(top-level entries) instead of
O(files).

Deleting the `#[cfg(windows)]` git-status filter in `scope.rs` is the
**acceptance criterion**, not a side effect: that filter exists only because the
snapshot lived inside the worktree, and it is itself a transport branch in
calling code.

**State the strength plainly, in `scope.rs`'s module doc, in the same paragraph
as the Unix note.** A deny ACE naming the token's user SID *is* enforced against
a process running as that user — deny ACEs are canonically first. But the user
owns a worktree they created, and an owner is implicitly granted `WRITE_DAC`, so
the agent can `icacls . /reset` and the fence evaporates. That is the same class
as the `chmod u+w .` escape already conceded for Unix — parity, not degradation,
and the claim must not be inflated. Adding `WRITE_DAC` to the deny mask would
block Demeteo's own teardown under the same token, so it is deliberately not
done. `verify_and_revert_out_of_scope_writes` remains the real gate.

Rewrite `mr_publisher/push.rs`. Today it writes `#!/bin/sh\nprintf '%s' '<PAT>'`
and protects it with a `set_file_mode(0o700)` that is a **no-op on Windows**,
leaving the token on disk under the inherited ACL. Worse, git consults credential
*helpers* before `GIT_ASKPASS`, and the Git for Windows installer writes
`credential.helper = manager` into the system gitconfig — so the askpass **never
runs**, and GCM either pushes with a stale identity or opens a **GUI prompt that
`GIT_TERMINAL_PROMPT=0` does not suppress**, blocking an unattended run forever
(the push sets no timeout today). Replace with `-c credential.helper=` (empty
resets the list) plus an env-reading helper, `GCM_INTERACTIVE=false`,
`GCM_GUI_PROMPT=0`, and a real timeout. One path for all three OSes, no disk
write, no argv exposure — which lets `set_file_mode` be deleted from the port.

*Shippable:* an `ArtifactsOnly` step's agent is denied **writes and deletes**
outside its declared paths, asserted by one conformance test running the same
assertions against Unix chmod and Windows DACL; fence apply/restore over a
`node_modules` tree completes in under a second; pushing an MR works identically
on all three desktop OSes with no secret written to disk.

### Phase 5 — Escape hatches and the last Unix-only feature

`ShellSpec::PowerShell` / `Cmd` as an explicit per-step override, gated at
**save time** by a synchronous `domain/` check against the platforms the
project's configured machines span — a step that can only run on Windows must
not be schedulable to the always-Linux runner, and blocking at save is the only
place that is catchable. Then the Win32 arm of `TrustedWorktreePort`:
handle-relative traversal via `CreateFileW` with `FILE_FLAG_OPEN_REPARSE_POINT |
FILE_FLAG_BACKUP_SEMANTICS` and file-id verification against the parent handle.
Win32 has no `fchdir`-before-exec, so the guarantee is genuinely weaker than the
Unix `openat` path — the UI must mark Windows terminal worktrees as
reduced-guarantee rather than presenting them as equivalent.

---

## Gates

`pr-checks.yml` gains, in order of value:

1. ~~`windows-latest` + `macos-latest` compile/clippy/test (Phase 0)~~ — **landed**
   as the `cross-os` job. Would have caught fc8d65c entirely.
2. ~~A **Windows exec-contract** conformance case~~ — **landed by construction**:
   `exec_contract`'s local leg is an ordinary test, so `cargo test -p demeteo-core`
   runs it on `windows-latest` against the identical assertions Linux uses.
3. **Windows-host → Linux-sshd** `run-ssh-conformance.sh` — **outstanding**, and
   still the single most valuable test here, because it is exactly the topology
   a real user has. `ssh-conformance` runs on ubuntu only.

`run-topology-conformance.sh` stays Linux-only with a **written** exemption
rather than an implied one.

### The Linux-side tier, and what it cannot see

`scripts/check-windows.sh --run` links the `gnu` target's whole test binary and
executes it under Wine, so a `cfg(windows)` body can be *run* here in about two
minutes rather than a CI round trip. It is local-only by construction — it
refuses when `CI` is set, `checks.sh` never calls it, and `windows-latest`
remains the authority.

Its blind spots are structural, and none of them is a matter of configuring the
prefix better:

- **The MSYS2 runtime does not work under Wine at all** (`Couldn't compute
  FAST_CWD pointer`, `cygpath: Bad address`), so a login shell yields nothing.
  Every probe spelled `bash -l -i -c` is therefore unreachable. Plain `bash -c`
  works, so Phase 1's shell-parity path *is* covered.
- **Deny ACEs are not enforced.** The fence applies without error and then
  permits the write it denied. Phase 4 is verifiable on real Windows only.
- **`pwsh` reports success for everything** — empty stdout, empty stderr, exit 0
  for `exit 42` — so it is deliberately absent from the prefix and the tests
  that spawn it fail honestly instead. Worth noting that those fixtures depend
  on a program Windows does not ship at all; they pass in CI because the GitHub
  runner preinstalls PowerShell 7, not because a user's machine would have it.

`scripts/wine-known-failures.txt` enumerates every excused test with its cause,
and each entry is confirmed green on `windows-latest` — so the list records
gaps in the emulator, never an unproven claim about Windows. Nothing is
skipped: a failure outside the list fails the run.

Per §7, the fence and tree-kill tests count only once watched failing against the
prior phase's code. And per §7's warning: the e2e `FakeExec` returns `Ok("")` for
every command, so it passes while masking exactly this class of drift. New
doubles here must error on anything they were not explicitly told to say.

## Stated capability gaps

Honest degradation, not silent:

- **Tool managers that hook `$PROFILE`** (fnm, `mise activate pwsh`) cannot be
  recovered from a GUI-spawned non-interactive child under *any* shell. Shim-based
  managers (Volta, pyenv-win, nvm-windows, `mise activate --shims`) work. The
  preflight names the remediation. Windows PATH resolution is architecturally less
  reliable than Linux's and the plan does not pretend otherwise.
- **MSVC/MSBuild-centric projects** are not served by one POSIX body —
  `vcvarsall.bat` exists to mutate its caller's environment and cannot work from
  bash. Phase 5's `ShellSpec` is the answer; a shell-free `command` node is the
  interim one.
- **MSYS argument path-translation** rewrites arguments that look like POSIX
  paths when bash invokes a native `.exe`. Left **on** deliberately — it is what
  makes portable scripts work — and everything Demeteo constructs goes through
  `run_program`, which never touches bash. `MSYS_NO_PATHCONV` is not used: its
  mere definedness disables conversion and `=0` does not re-enable it.
- **The Windows platform block is instruction, not enforcement.** Nothing
  rejects an agent that translates a gate command into PowerShell anyway, and
  nothing could today: detection would have to compare what the agent ran
  against what the project configured, and no layer sees both — the agent's
  commands run inside its own turn. The verdict-bearing run is Demeteo's own, so
  a translated command costs a wasted turn rather than a false green. The block
  raises the odds; it does not close the hole.
- **Codex's sandbox is unverified on Windows.** Demeteo sends a mode it cannot
  confirm is backed by anything, so on Windows an agent's containment rests on
  the Phase 4 fence — whose own strength is stated above, and which has not yet
  denied a real agent — and on `verify_and_revert_out_of_scope_writes`, which
  is the layer that actually holds. On Linux and macOS the harness sandbox is a
  genuine second layer; on Windows it is unknown rather than absent, and no
  user-visible claim may count it.
- **Defender** makes `node_modules` installs and post-build deletes markedly
  slower than Linux on identical hardware. Advisory mitigations only (Dev Drive,
  exclusions, lower default Windows parallelism).

## Related

- [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) — the contract this extends
- [`TRUSTED_WORKTREE.md`](TRUSTED_WORKTREE.md) — Phase 5's Win32 arm
- [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) — user-facing Windows caveats
- [`DECISIONS.md`](DECISIONS.md) — the one-body decision and its rejected
  alternative belong here once Phase 0 lands
