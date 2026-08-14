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

**Auto-detected fix:** `src-tauri/src/lib.rs:90-112` (`configure_linux_gpu_env`)
detects the NVIDIA proprietary driver via
`/proc/driver/nvidia/version` and sets:

| Env var | Value | Reason |
|---|---|---|
| `GBM_BACKEND` | `nvidia-drm` | Force the GBM buffer API (NVIDIA 495+ supports both GBM and EGLStreams; GBM is correct here) |
| `__GLX_VENDOR_LIBRARY_NAME` | `nvidia` | Pin GLX to NVIDIA's ICD |
| `__NV_DISABLE_EXPLICIT_SYNC` | `1` | Skip the `linux-drm-syncobj-v1` path that triggers Error 71 |

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
[demeteo] NVIDIA detected: GPU rendering enabled (explicit sync off)
[demeteo] GPU rendering disabled via DEMETEO_DISABLE_GPU
```

No banner means non-NVIDIA Linux and WebKitGTK defaults are in
effect.

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

## References

- [tauri-apps/tauri#10702](https://github.com/tauri-apps/tauri/issues/10702) — Error 71 dispatching to Wayland display
- [tauri-apps/tauri#14924](https://github.com/tauri-apps/tauri/issues/14924) — Linux/Nvidia: Crash (GBM/Error 71) or visual artifacts with transparent windows
- [tauri-apps/tauri#10566](https://github.com/tauri-apps/tauri/issues/10566) — Poor performance on Arch Linux until Web Inspector is opened
- [Arch Wiki: NVIDIA § Wayland configuration](https://wiki.archlinux.org/title/NVIDIA#Wayland_configuration)
- [Arch Wiki: Wayland § NVIDIA driver](https://wiki.archlinux.org/title/Wayland#NVIDIA_driver)