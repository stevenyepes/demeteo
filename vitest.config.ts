import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Standalone from `vite.config.ts` on purpose. That config is an async factory
// wrapping Tauri dev-server settings (fixed port 1420, strictPort, HMR host)
// and the Monaco/xterm manualChunks split — none of which apply under jsdom,
// and `strictPort` actively breaks parallel test runs. The only piece the tests
// need is the React plugin for JSX/Fast-Refresh-free transforms.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    // Monaco and xterm both reach for canvas/WebGL APIs jsdom doesn't implement.
    // Components under test stub them (see src/test/setup.ts); this keeps a
    // stray real import from taking the whole suite down.
    server: {
      deps: {
        inline: ["@monaco-editor/react", "monaco-editor"],
      },
    },
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**"],
    },
  },
});
