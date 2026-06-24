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

**Auto-detected fix:** `src-tauri/src/lib.rs` detects the NVIDIA
proprietary driver via `/proc/driver/nvidia/version` and sets:

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

## References

- [tauri-apps/tauri#10702](https://github.com/tauri-apps/tauri/issues/10702) — Error 71 dispatching to Wayland display
- [tauri-apps/tauri#14924](https://github.com/tauri-apps/tauri/issues/14924) — Linux/Nvidia: Crash (GBM/Error 71) or visual artifacts with transparent windows
- [tauri-apps/tauri#10566](https://github.com/tauri-apps/tauri/issues/10566) — Poor performance on Arch Linux until Web Inspector is opened
- [Arch Wiki: NVIDIA § Wayland configuration](https://wiki.archlinux.org/title/NVIDIA#Wayland_configuration)
- [Arch Wiki: Wayland § NVIDIA driver](https://wiki.archlinux.org/title/Wayland#NVIDIA_driver)