import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev port and no browser auto-open.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2021",
    // minify omitted: Vite 8 defaults to oxc; esbuild minify is deprecated.
    sourcemap: false,
  },
  // Under Vitest, resolve Svelte's browser build so component tests mount the
  // real runtime; outside tests Vite's default resolution stands.
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  test: {
    // globals: true lets @testing-library/svelte register its automatic
    // afterEach cleanup — without it, renders accumulate across tests.
    globals: true,
    environment: "jsdom",
  },
});
