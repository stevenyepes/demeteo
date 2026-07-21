import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @xterm/headless@6.0.0 mis-publishes its `module` field (points at a
// non-existent `lib/xterm.mjs`), so Vite's browser resolution fails with
// "Failed to resolve entry". The real ESM build is `lib-headless/
// xterm-headless.mjs`; alias the bare specifier straight to it. tsc/vitest are
// unaffected — they resolve via the package's valid `main`/`types` fields.
const xtermHeadlessEntry = fileURLToPath(
  new URL(
    "./node_modules/@xterm/headless/lib-headless/xterm-headless.mjs",
    import.meta.url,
  ),
);

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    // Exact-match the bare specifier (regex `$` end-anchor) so subpath imports
    // are untouched and the alias can't recurse on its own replacement.
    alias: [{ find: /^@xterm\/headless$/, replacement: xtermHeadlessEntry }],
  },

  // Split the Monaco editor (and its ~90 bundled languages) into its own
  // chunk instead of merging it into the single app `index.js`. Without this,
  // Rollup holds one ~5 MB module graph in memory through render + minify,
  // pushing peak RSS past 3 GB and forcing the `--max-old-space-size` band-aid
  // in the `build` script. Isolating monaco lets Rollup emit and minify it as a
  // separate unit, cutting peak memory (and shrinking the app chunk the webview
  // parses on boot).
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (
            id.includes("node_modules/monaco-editor") ||
            id.includes("node_modules/@monaco-editor")
          ) {
            return "monaco";
          }
          if (id.includes("node_modules/@xterm")) {
            return "xterm";
          }
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
      // 4. avoid HMR storms that can crash the Tauri webview's GDK/Wayland
      //    connection on some Linux desktops.
      usePolling: false,
    },
    // 5. ensure keep-alive timeouts don't kill long-lived esbuild sockets
    //    when the webview is slow to attach.
    cors: true,
  },
  // 6. Pre-bundle heavy deps so esbuild finishes transforming them BEFORE
  //    the Tauri webview connects. This avoids the "service was stopped"
  //    error caused by esbuild's subprocess being torn down mid-request.
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-dom/client",
      "lucide-react",
      "@tauri-apps/api/core",
      "@tauri-apps/plugin-dialog",
      "@xterm/xterm",
      "@xterm/headless",
      "@xterm/addon-fit",
      "@monaco-editor/react",
      "monaco-editor",
    ],
  },
}));
