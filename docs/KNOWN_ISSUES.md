# Demeteo: Known Issues

> **Platform quirks and their workarounds.** Entries here describe
> real-world breakage we've observed, the fix we shipped, and the
> escape hatch for users on hosts where the fix doesn't apply.
> When an entry is fully resolved upstream, move it to a CHANGELOG
> entry and remove it from this doc.

## GPU rendering on Linux + NVIDIA Wayland

**Symptom:** Launching `npm run tauri dev` on a host with an NVIDIA
proprietary GPU and a Wayland compositor (Hyprland, Sway, recent
GNOME, recent KDE Plasma) crashes the process at webview creation
with:

```
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

**Cause:** WebKitGTK's DMA-BUF renderer mismatches NVIDIA's
`linux-drm-syncobj-v1` explicit-sync implementation, producing a
Wayland protocol error that the host process can't recover from.
This is upstream-blocked at the WebKitGTK / NVIDIA driver layer —
tracked as
[tauri-apps/tauri#10702](https://github.com/tauri-apps/tauri/issues/10702)
and
[tauri-apps/tauri#14924](https://github.com/tauri-apps/tauri/issues/14924).

**Auto-detected fix:** `configure_linux_gpu_env` in `src-tauri/src/lib.rs`
detects the NVIDIA proprietary driver via
`/proc/driver/nvidia/version` and sets:

| Env var | Value | Reason |
|---|---|---|
| `GBM_BACKEND` | `nvidia-drm` | Force the GBM buffer API (NVIDIA 495+ supports both GBM and EGLStreams; GBM is correct here) |
| `__GLX_VENDOR_LIBRARY_NAME` | `nvidia` | Pin GLX to NVIDIA's ICD |
| `__NV_DISABLE_EXPLICIT_SYNC` | `1` | Skip the `linux-drm-syncobj-v1` path that triggers Error 71 |
| `WEBKIT_DISABLE_DMABUF_RENDERER` | `1` | Avoid the WebKitGTK 2.54 runaway described below |

This is applied only when NVIDIA is detected and only if the
user hasn't already set those variables. macOS, Windows, and
non-NVIDIA Linux hosts are unaffected — WebKitGTK's defaults are
correct on Mesa/AMD/Intel.

**Escape hatch:** Set `DEMETEO_DISABLE_GPU=1` to force CPU
rendering. This restores the prior behavior of disabling DMA-BUF
and accelerated compositing. Use it on hosts where:

- The auto-detected fix doesn't apply (non-proprietary NVIDIA
  drivers, hybrid GPU setups, exotic Wayland compositors).
- The app crashes at startup regardless of the auto-detected
  env vars.

```bash
DEMETEO_DISABLE_GPU=1 npm run tauri dev
# or, equivalently:
npm run dev:tauri:sw
```

**Why we don't force GPU on every host:** The Error 71 is a
process-killing crash, not a visual artifact. Restoring GPU
rendering for users on broken hosts would brick the app at
launch. The current design is "auto-fix when we recognize the
host; opt-out when we don't" — strictly safer than "GPU by
default, opt-in when broken."

**Verifying which path you're on:** The startup banner includes
one of these lines on Linux:

```
[demeteo] NVIDIA detected: explicit sync off, DMA-BUF renderer off, CPU rasterization
[demeteo] NVIDIA detected: explicit sync off, GPU rendering via DMA-BUF
[demeteo] GPU rendering disabled via DEMETEO_DISABLE_GPU
```

No banner means non-NVIDIA Linux and WebKitGTK defaults are in
effect.

### WebKitGTK 2.54 runs away on NVIDIA's GPU path

**Symptom:** after a distro upgrade from `webkit2gtk-4.1` 2.52.6 to
2.54.1 (NVIDIA 615.71.09, Wayland), the app paints its skeletons and
then freezes. `WebKitWebProcess` sits at 100% CPU and grows by about
430 MB/s, past 25 GB within a minute. The Rust side is idle, so the
log shows nothing, and the stall looks like a hung `Promise.allSettled`
in the frontend.

**What was measured** on that host, same binary and same database:

| Environment | Web process after ~20 s |
|---|---|
| NVIDIA defaults above, DMA-BUF renderer on | 100% CPU, 8.5 GB and climbing |
| `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` (GPU render, SHM hand-off) | 100% CPU, 12.7 GB at 30 s |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 0% at rest, ~375 MB |
| `DEMETEO_DISABLE_GPU=1` | 0% at rest, ~400 MB |

Forcing shared-memory hand-off does not help, so the fault is in the
web process's GPU rendering, not in how frames reach the window.

**Why 2.54:** no upstream bug matches this yet. The 2.53/2.54 cycle
replaced the web-process compositor with Skia's GPU backend, added a
GPU atlas written through DMA-BUF mappings, and switched to
damage-only composition ([2.54 highlights][wk254], [2.54.1][wk2541]).
All of that runs only on the DMA-BUF path, which is consistent with the
table but is an inference, not a bisected cause. A
[report against another WebKitGTK app][sink78] shows the same driver
and the same 2.52 → 2.54 regression, with a different symptom.

**The fix and its cost:** `configure_linux_gpu_env` now also sets
`WEBKIT_DISABLE_DMABUF_RENDERER=1` on NVIDIA. In 2.54 this does more
than give up zero-copy: with no DMA-BUF transport WebKit turns
hardware acceleration off, and the web process rasterizes on the CPU
into shared memory. At rest that costs nothing measurable. Anything
that repaints — a running CSS animation, scrolling, a streaming
terminal — rasterizes on the CPU, so expect more CPU during motion
than the GPU path used before 2.54. Visuals are unchanged.
`backdrop-filter` blur on the glass cards is the most expensive thing
this repaints.

**Opting back in:** `WEBKIT_DISABLE_DMABUF_RENDERER=0` restores the
GPU path. WebKit treats `0` as enabled (`AcceleratedBackingStore.cpp`
compares against `"0"`), and Demeteo leaves an already-set variable
alone. Try it after a WebKitGTK upgrade; the banner says which path is
active.

**What would close it:** an upstream fix, or keeping DMA-BUF while
turning off the `UseSkiaForComposition` feature through
`webkit_settings_set_feature_enabled` on the webview. That second
route needs the `webkit2gtk` crate as a direct dependency, and that
WebKit's Skia compositor is the cause has not been confirmed.

[wk254]: https://webkitgtk.org/2026/09/16/webkitgtk-2.54-highlights.html
[wk2541]: https://webkitgtk.org/2026/10/02/webkitgtk2.54.1-released.html
[sink78]: https://github.com/NC1107/sink/issues/78

## Agents behave as though Windows were Linux

**Symptom:** On a Windows desktop, an agent step acts like it is on
Linux. It hunts for bash or the Unix utilities as though they had to
be found, or it rewrites your configured test/build command into
PowerShell before running it. The turn is spent on the detour, and
work can end up judged against a command you never configured.

**Cause:** Nearly everything an agent uses to work out what machine
it is on pointed at POSIX. Demeteo forwarded its own `SHELL` and
`TMPDIR` to every agent it spawned — and Demeteo started from a Git
Bash terminal exports `SHELL=/usr/bin/bash` — while the prompt named
no operating system at all and quoted your project's POSIX gate
commands verbatim. The agent drew the obvious conclusion.

**Shipped fix:** a Windows agent inherits neither variable, and its
prompt now opens by naming the OS, naming the Git Bash that runs
those commands for it, and forbidding it to translate them. Where the
harness runs the agent's own commands through something other than
that bash — codex uses PowerShell — the block also gives it the
resolved interpreter to wrap a quoted command in unchanged, so the
prohibition leaves it a way to run the command at all.

**What this does not settle:** the prompt is an instruction, so an
agent can still ignore it; the correction makes that less likely
rather than impossible. Separately, `codex` on Windows is sent the
same sandbox setting it is sent on Linux, and whether Windows backs
that with anything is unknown — see
[WINDOWS_PARITY.md](WINDOWS_PARITY.md). Do not count that setting
as containment on Windows: what actually bounds a step there is the
worktree fence and the out-of-scope write check, the same two layers
you would have with no sandbox at all.

**Escape hatch — run the work on WSL2 instead.** A WSL2 distribution
with an sshd is registerable as an ordinary machine under
*Settings → Machines*, exactly like any other Linux box; it is not a
special mode and needs nothing Windows-specific. The worktree, the
agent, and your gate commands then all live on Linux, where every
signal above is simply true.

Two costs, both worth knowing before you choose it: keep the clone
inside the distribution's own filesystem — a repo under `/mnt/c` is
much slower and has different file semantics — and Windows-native
toolchains (MSBuild, signtool, the .NET SDK) are not reachable from
there, so a project that needs them wants the native path despite
the caveats above.

## A GitLab fix run never stacks on the branch it reviewed

**Symptom:** A fix run launched from a GitHub pull request whose head
branch lives in the upstream repository opens its pull request against
that head branch, so the fix is reviewed against the work it fixes. The
same run launched from a GitLab merge request always opens against the
branch the merge request targets — `main` — even for a same-project
merge request whose source branch demonstrably exists upstream.

**Cause:** `crates/demeteo-core/src/domain/fix_destination.rs` will only
stack on the head branch when the provider has said we may add commits
to it. Merging that stack lands its commits on the contributor's branch
and changes what the reviewed request contains, so an unstated
permission is refused rather than assumed — the rule
`crates/demeteo-core/src/domain/mr_summary.rs` applies to every
permission field. GitHub answers it with `head.repo.permissions.push`.
A GitLab merge request payload carries no equivalent field at all, so
`MrSummary::head_repo_push` is hardcoded `false` for every merge
request, and the fork test in the destination's suite is deliberately
split so the fork case cannot pass for this reason.

**Consequence:** none to correctness. The fallback is the merge
request's own target branch, which is always upstream, so the fix run
still produces a mergeable merge request; it just carries the reviewed
commits along with the fix rather than stacking on top of them.

**What would close it:** a real GitLab signal for push permission on the
source branch, parsed in the provider mapping (`MrSummary::from_gitlab`)
— not a special case in `fix_destination`, which has no provider to ask.

## A clone's embedded token is only scrubbed when it matches the keyring

**Symptom:** A clone made by an older Demeteo keeps a token inside its
`origin` URL (`https://user:token@host/…`), visible in `.git/config`,
after the project has been opened and fetched from with a current
build. No error is shown; for one case below, nothing is logged either.

**Cause:** Moving the token out of `origin` is a `git remote set-url`
that cannot be undone — the old token is gone once it returns — so
`prepare_origin` in
`crates/demeteo-core/src/adapters/worktree/git_ops/clone.rs` rewrites
the URL only when the keyring PAT for that host is *exactly* the
password the URL embeds (raw or percent-decoded). A different token for
the same host — a stale keyring entry, or another account's PAT — would
replace a working credential with one that fails. On a mismatch it logs
a redacted warning and leaves the URL alone, so the fetch authenticates
as it always did.

**Consequence:** The token stays in `.git/config` until the user
re-saves the provider token (so the keyring matches) or fixes the
remote by hand. Any PAT that has already sat in a `.git/config` — or in
a log, a backup or a synced folder that copied it — has leaked as far
as that copy goes and must be rotated; scrubbing the URL does not
un-expose it.

A password containing an unencoded `/`, `?` or `#` is never migrated and
**no log line says so**. `HttpRemote` in
`crates/demeteo-core/src/domain/git_push.rs` ends the authority at the
first of those delimiters, so `https://u:p/ss@host/r` has no userinfo
and reads as a token-free URL. This is accepted: re-parsing at the last
`@` would mis-rewrite URLs whose *path* contains one. A password
percent-encoded in the URL is handled.

**What would close it:** for the mismatch, a probe that proves which
credential works before choosing (an authenticated `ls-remote` with
each), or a prompt that lets the user pick; for the delimiter case, a
parse that can tell a path `@` from a password one — neither is built.

## References

- [tauri-apps/tauri#10702](https://github.com/tauri-apps/tauri/issues/10702) — Error 71 dispatching to Wayland display
- [tauri-apps/tauri#14924](https://github.com/tauri-apps/tauri/issues/14924) — Linux/Nvidia: Crash (GBM/Error 71) or visual artifacts with transparent windows
- [tauri-apps/tauri#10566](https://github.com/tauri-apps/tauri/issues/10566) — Poor performance on Arch Linux until Web Inspector is opened
- [Arch Wiki: NVIDIA § Wayland configuration](https://wiki.archlinux.org/title/NVIDIA#Wayland_configuration)
- [Arch Wiki: Wayland § NVIDIA driver](https://wiki.archlinux.org/title/Wayland#NVIDIA_driver)