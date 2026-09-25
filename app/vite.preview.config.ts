import { defineConfig, mergeConfig } from "vite";
import base from "./vite.config";

// Preview-only config for dogfooding the views in a plain browser, where no
// Tauri backend exists. It aliases the IPC boundary to the fixture mock in
// `src/lib/test-support/` and serves on port 8090:
//
//   pnpm vite dev --config vite.preview.config.ts
//
// Scenario is chosen with `?scenario=populated|empty|error`. Nothing here
// affects `pnpm build` or the Tauri bundle — they use the default config.
// Note: no `plugins` override here on purpose — mergeConfig concatenates
// arrays, so re-adding `svelte()` would run the Svelte plugin twice and
// feed its own compiled output through the compiler a second time
// (observed as spurious tag_invalid_name / expected_token build errors).
export default mergeConfig(
  base,
  defineConfig({
    server: {
      port: 8090,
      strictPort: true,
    },
    resolve: {
      alias: [
        {
          find: "@tauri-apps/api/core",
          replacement: "/src/lib/test-support/tauri-mock.ts",
        },
      ],
    },
  }),
);
