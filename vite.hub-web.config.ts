import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const projectRoot = fileURLToPath(new URL(".", import.meta.url));
const hubWebRoot = fileURLToPath(new URL("./hub-web", import.meta.url));

// Not the root `dist/`: `frontendDist` in `src-tauri/tauri.conf.json` owns that
// directory and `vite build` empties its outDir, so sharing it would have each
// build delete the other's bundle. `hub-web/dist` is the directory the
// not-yet-built `crates/demeteo-hub` will be configured to serve.
const hubWebDist = fileURLToPath(new URL("./hub-web/dist", import.meta.url));

// Same realpath as `resolvedModules` in `vite.config.ts`; the worktree comment
// there is the rationale.
const resolvedModules = (() => {
  try {
    return [realpathSync(fileURLToPath(new URL("./node_modules", import.meta.url)))];
  } catch {
    return [];
  }
})();

// No fixed listen address on purpose: the desktop dev server pins its own in
// `vite.config.ts`, and the two have to run side by side.
export default defineConfig({
  root: hubWebRoot,
  // Relative asset URLs, so the bundle assumes nothing about the path the Hub
  // mounts it at.
  base: "./",
  plugins: [react(), tailwindcss()],
  build: {
    outDir: hubWebDist,
    emptyOutDir: true,
  },
  server: {
    // The repo root rather than `hub-web/`: hub-web imports from `../src`,
    // starting with the shared stylesheet. Vite's default lands on the same
    // directory only by falling back to the nearest `package.json`, so it
    // would shrink to `hub-web/` and 403 those imports if one appeared there.
    fs: { allow: [projectRoot, ...resolvedModules] },
  },
});
