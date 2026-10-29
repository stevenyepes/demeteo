// `npm run dev:tauri`: `tauri dev` against the dev config, with the WebKitGTK
// GPU renderer back on.
//
// On NVIDIA, `configure_linux_gpu_env` in src-tauri/src/lib.rs turns the
// DMA-BUF renderer off because WebKitGTK 2.54 ran away with the installed
// app's data; CPU rasterization is what makes the dev window lag. The dev
// database never reproduced the runaway, so dev opts back in. An explicit
// value in the caller's environment wins, and `npm run dev:tauri:sw` still
// forces the CPU path. Elsewhere the variable is WebKit's own default or unread.
//
// A wrapper rather than `VAR=0 tauri dev` in package.json: npm runs scripts
// through cmd.exe on Windows, which has no inline assignment.
import { spawn } from "node:child_process";

const env = { WEBKIT_DISABLE_DMABUF_RENDERER: "0", ...process.env };
const args = ["dev", "--config", "src-tauri/tauri.dev.conf.json", ...process.argv.slice(2)];

const child = spawn("tauri", args, { stdio: "inherit", env, shell: process.platform === "win32" });
child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 1)));
