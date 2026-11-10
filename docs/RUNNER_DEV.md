# Demeteo: Remote-Runner Dev Workflow

> **How the `demeteo-runner` binary is located, built, and pushed to remote
> machines — and how to diagnose it when it won't start.** See
> [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) for the feature design this
> supports, and [`ARCHITECTURE.md`](ARCHITECTURE.md) for the surrounding hexagon.
> Read this before editing runner binary location, arch detection, or install logic.

The `demeteo-runner` binary runs on **every** remote machine. CI ships it as a
Linux x86_64 musl static binary (`x86_64-unknown-linux-musl`). Anything else will
fail with `Exec format error` on the remote — see the `Mach-O on Linux` failure
mode below.

## The three-tier lookup (in priority order)

`crates/demeteo-core/src/infrastructure/runner/binary.rs::locate_local()` checks each source in this order and stops at the first one that resolves to an existing file:

| Tier | Source | Use case |
|------|--------|----------|
| 1    | Dev cache: `$TMPDIR/demeteo-runner-cache/dev/demeteo-runner-x86_64-unknown-linux-musl` | Written by `npm run build:runner`. **Preferred for dev.** |
| 2    | `$DEMETEO_RUNNER_BIN` env var | Explicit per-shell override. Useful for pointing at a CI-built artifact you downloaded manually. |
| 3    | `<app-dir>/demeteo-runner` — sibling of the running Tauri binary | Whatever plain `cargo build --release -p demeteo-runner` produced on this laptop. **Often the wrong arch on Mac devs** — see the arch guard below. |

If none of the three resolve, `remote_runner_local_check` returns `Missing` and the UI prompts the user to download the release asset (`adapters/tauri_ui/runner_download.rs::download_release`) — which works on any host with internet, no toolchain needed.

## Building locally — `npm run build:runner`

```bash
npm run build:runner
```

The script (`scripts/build-runner.sh`) detects `uname -s`/`uname -m` and:

| Host | Action |
|------|--------|
| Linux x86_64 | Native `cargo build --release -p demeteo-runner` (already ELF). No cross-compile. |
| macOS (arm64 or x86_64) | `rustup target add x86_64-unknown-linux-musl` (one-time), then cross-build. Requires a **musl C cross-compiler** for `ring`/libcrypto to link — see the next sub-section. |
| Anything else | Refuses with a clear error. |

The result is copied to the dev cache path above. After that, click *Enable remote runs* on any machine in `Settings → Machines` and the app pushes that binary.

### macOS musl-cross setup (one-time)

The `ring` crate (Rust crypto, used by reqwest) needs a real C cross-compiler when building for `x86_64-unknown-linux-musl`. The cross-target alone isn't enough on macOS — install one:

```bash
brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl
```

Then point Cargo at it (the build script does this automatically once it's installed):

```bash
export CC_x86_64-unknown-linux-musl=/opt/homebrew/bin/x86_64-unknown-linux-musl-gcc
```

Without this, `cargo build --release -p demeteo-runner --target x86_64-unknown-linux-musl` fails with `failed to find tool "x86_64-linux-musl-gcc"`.

## The arch guard (defense in depth)

Even if the dev cache is empty and `$DEMETEO_RUNNER_BIN` points at a stale Mach-O from a previous `cargo build`, the push-time guard catches it:

- `crates/demeteo-core/src/infrastructure/runner/binary.rs::RunnerBinary::arch()` — single source of truth for magic-byte classification (`RunnerArch::{LinuxX86_64, LinuxOther, MacOs, Windows, Unknown}`).
- `src-tauri/src/commands/remote_install.rs::reject_non_linux_x86_64` — constructs a `RunnerBinary` and refuses anything that isn't `LinuxX86_64`, with an error message that points the dev at `npm run build:runner`.

Tests cover the classification:

```bash
cargo test -p demeteo-core --lib infrastructure::runner::binary
```

7 tests, covering ELF x86_64, ELF 32-bit, ELF big-endian, Mach-O arm64 LE, Windows PE, missing file, short file.

## Mach-O on Linux

If the runner was installed on the remote but won't start, the systemd journal will show:

```
demeteo-runner.service: Failed to execute /home/<user>/.local/bin/demeteo-runner: Exec format error
demeteo-runner.service: Main process exited, code=exited, status=203/EXEC
```

Diagnose:

```bash
ssh <user>@<machine> 'systemctl --user status demeteo-runner.service -l --no-pager; \
  echo ---; \
  journalctl --user -xeu demeteo-runner.service --no-pager -n 50'
```

`203/EXEC` = kernel rejected `execve(2)` on the binary. Confirm the file's actual format:

```bash
ssh <user>@<machine> 'file ~/.local/bin/demeteo-runner'
```

Fix:

```bash
npm run build:runner                                # rebuild + cache
# OR — for the rest of this session, point the app at a Linux x86_64 build
DEMETEO_RUNNER_BIN=/path/to/linux-x86_64-binary npm run dev:tauri
# OR — quickest: delete the Mach-O sibling and let the app download the release
rm -f src-tauri/target/{debug,release}/demeteo-runner
```

## When the runner is running but still won't serve runs

| Symptom (UI) | Remote cause | Fix |
|--------------|--------------|-----|
| Green "Running · v0.1.0" + amber linger warning | `loginctl enable-linger` needs admin/polkit | Ask an admin to run `sudo loginctl enable-linger <user>` on the box |
| Slate "Installed, stopped" | systemd `--user` unit isn't active | `ssh <user>@<machine> 'systemctl --user start demeteo-runner'` or re-click *Upgrade runner* in the UI |
| Slate "Remote runner not installed" | No push has happened yet | Click *Enable remote runs* |

### A dev build is blocked against a nightly runner

`npm run dev:tauri` reports `tauri.conf.json`'s version, which is always stable
(e.g. `1.2.0`). A machine still running a nightly runner (`1.2.0-N`) therefore
reads as *runner ahead — upgrade Demeteo*, and detached runs are refused. That is
the locked policy, not a bug: the channel comes from the version string alone and
there is no debug-build override (see
`crates/demeteo-core/src/domain/runner_version.rs` and
[`DECISIONS.md`](DECISIONS.md)). Ignore the "upgrade Demeteo" advice — push your
local runner build (`npm run build:runner`, which stamps it with that same
`tauri.conf.json` version, then *Downgrade runner to 1.2.0* on that machine in
`Settings → Machines`) so the two versions match.

## Pairing a runner with the Hub

Design: [`HUB.md`](HUB.md) §4 *Pairing a runner by device code*, decision 64.
On the runner host, as the runner's user:

```bash
demeteo-runner hub seal                   # once; the installers already ran it
systemctl --user restart demeteo-runner   # so the unit loads the key
demeteo-runner hub pair https://hub.example.com
```

`hub pair` prints a code, the Hub's `#/device` URL and this runner's
fingerprint; approve it on the Hub with a passkey after checking the fingerprint
matches. Ctrl-C cancels. Then `hub status`, `hub policy` (no flags prints it;
`--spend on`, `--run-ceiling 500`, `--daily-ceiling off`, …), `hub log [<count>]`
(what the Hub asked here and how it was answered, newest first) and
`hub disconnect`. All five are clients of the running `serve` over its control
socket, so `serve` must be up; the same `hub_*` methods are what a desktop calls
over SSH. Only `hub seal` works without `serve`.

**Pairing alone does not let the runner run work.** Approving `read spend
gates` on the Hub says what the Hub may *ask*; the runner's own policy says
what it will *do*, and `spend` and `gates` start off, with both ceilings unset
(decision 64, [`HUB.md`](HUB.md) §7). Until you set them, the Hub's dispatch
page reads "Spend is off on this runner" and a start sent anyway comes back
`scope_off`. Turn them on once, after pairing:

```bash
demeteo-runner hub policy --spend on --gates on --run-ceiling 2500 --daily-ceiling 10000
```

The ceilings are cents and are yours to choose: a start whose max cost is above
the run ceiling, or that would take the last 24 hours past the daily one, is
refused, and an unset ceiling refuses every start. `--gates on` is only needed
to decide this runner's gates from the Hub. The runner's Hub panel in `Settings
→ Machines` sets the same fields over SSH. A change to `spend` or `gates`
reconnects the runner's socket, so the Hub sees it within seconds.

From a desktop, the runner's Hub panel on its machine card in `Settings →
Machines` does the same over SSH: pair (it shows the code, the `#/device` URL
and the fingerprint), status, policy, activity log and disconnect. A runner
too old for a method is named as such there. Once paired, `serve` answers the
Hub's cancels, assignments, gate decisions and starts; `hub log` shows each. A
Hub-started run (`hub-<id>`) gets its GitHub token from the Hub's
`POST /git-token`, not from a desktop ([`HUB.md`](HUB.md) §6.5), so the Hub
must be up whenever the run pushes.

| Symptom | Cause | Fix |
|---------|-------|-----|
| `cannot pair: … CREDENTIALS_DIRECTORY is not set` | The unit loads no key: never sealed, or not restarted since | `hub seal`, then restart the unit |
| `hub seal` fails in `systemd-creds encrypt --user` | systemd older than 256 has no user-scoped credentials | That host cannot pair (decision 64); it still runs work |
| `hub seal` says a key is sealed already | Re-running it keeps the key, so an upgrade keeps the pairing | `hub seal --force` replaces it and forgets the pairing; pair again |
| A `hub-…` run stays `needs-credentials` | The token keeper stopped: the journal says `run <id>: no more git tokens from the Hub: <reason>`. A `422` is a GitLab or unknown-host repository; a `503` that never clears is the Hub's App config, its Transit policy, or the App not installed on that repository | Fix the Hub side, then retry the run |
| After a restart, `no accepted Hub start is logged for it` | The run's accepted start never reached `hub_request_log`, so its keeper cannot be restarted | Cancel and start the run again from the Hub |
| The Hub says "Spend is off on this runner", or a start comes back `scope_off` | The runner's own `spend` toggle is off; pairing does not turn it on | `hub policy --spend on` with both ceilings, as above |
| A start comes back `over_run_ceiling` or `over_rolling_ceiling` | A ceiling is unset, or the dispatch's max cost is above it | `hub policy --run-ceiling … --daily-ceiling …`, or lower the dispatch's max cost |
| A gate decision from the Hub comes back `scope_off` | The runner's `gates` toggle is off | `hub policy --gates on` |
| `cannot reach demeteo-runner serve` | The service is stopped, or `XDG_DATA_HOME` differs from the unit's | `systemctl --user status demeteo-runner` |

The sealed key is `hub-store-key.cred` in the runner's data dir and the drop-in
is `~/.config/systemd/user/demeteo-runner.service.d/hub-credential.conf`.
`hub seal` never restarts the unit, because a restart interrupts in-flight runs.

## Cross-cutting change map

When editing the runner binary location/arch logic, these files move together:

| File | What it owns |
|------|--------------|
| `crates/demeteo-core/src/infrastructure/runner/binary.rs` | `locate_local`, `RunnerArch::arch()`, `dev_cache_path`, `release_cache_path`, `stale_version_warning`, `probe_version` + 7 unit tests |
| `crates/demeteo-core/src/infrastructure/runner/install.rs` | `unit_for(machines, machine_id)` — systemd unit template + webhook injection; `hub_seal_command` run by the installer before its restart |
| `crates/demeteo-core/src/infrastructure/runner/status.rs` | `probe(exec, machine_id)` — remote state probe (binary + service + linger) |
| `src-tauri/src/adapters/tauri_ui/runner_download.rs` | `download_release`, `cancel`, `reset_cancel` + the two `#[tauri::command]` entry points |
| `src-tauri/src/commands/remote_install.rs` | Thin Tauri command layer; `reject_non_linux_x86_64` is the only logic here, reuses `RunnerBinary::arch()` (no duplicated parsing) |
| `scripts/build-runner.sh` | Host-side build script; produces `LinuxX86_64` for all three OS branches |
