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

## Cross-cutting change map

When editing the runner binary location/arch logic, these files move together:

| File | What it owns |
|------|--------------|
| `crates/demeteo-core/src/infrastructure/runner/binary.rs` | `locate_local`, `RunnerArch::arch()`, `dev_cache_path`, `release_cache_path`, `stale_version_warning`, `probe_version` + 7 unit tests |
| `crates/demeteo-core/src/infrastructure/runner/install.rs` | `unit_for(machines, machine_id)` — systemd unit template + webhook injection |
| `crates/demeteo-core/src/infrastructure/runner/status.rs` | `probe(exec, machine_id)` — remote state probe (binary + service + linger) |
| `src-tauri/src/adapters/tauri_ui/runner_download.rs` | `download_release`, `cancel`, `reset_cancel` + the two `#[tauri::command]` entry points |
| `src-tauri/src/commands/remote_install.rs` | Thin Tauri command layer; `reject_non_linux_x86_64` is the only logic here, reuses `RunnerBinary::arch()` (no duplicated parsing) |
| `scripts/build-runner.sh` | Host-side build script; produces `LinuxX86_64` for all three OS branches |
