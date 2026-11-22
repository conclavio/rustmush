import { fileURLToPath, URL } from "node:url";

import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vitest/config";

/** The mushd web listener `vite dev` proxies to (see mushd.toml). */
const backend = "http://127.0.0.1:8080";

export default defineConfig({
  // `svelteTesting()` (16b cut B.2, `T-44`) is the documented Svelte 5 route
  // for mounting a component under vitest: it resolves Svelte's browser build
  // and cleans up after each test, IN TEST RUNS ONLY. The conditions are not
  // overridden here by hand, because on Vite 6+ a `resolve.conditions` of our
  // own drops the defaults for the client build too.
  plugins: [svelte(), svelteTesting()],
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  build: {
    // Embedded into mush-net-web via rust-embed at cargo build time.
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules/three") || id.includes("node_modules\\three")) {
            return "vendor-three";
          }
          if (
            id.includes("node_modules/@codemirror") ||
            id.includes("node_modules\\@codemirror") ||
            id.includes("node_modules/@lezer") ||
            id.includes("node_modules\\@lezer") ||
            id.includes("node_modules/codemirror") ||
            id.includes("node_modules\\codemirror")
          ) {
            return "vendor-codemirror";
          }
          return undefined;
        },
      },
    },
  },
  server: {
    fs: {
      strict: true,
      // The playtest guide is bundled from the repo's docs (`?raw` import in
      // GuideView.svelte); allow that one file beside the web root.
      allow: [".", "../docs/playthrough.md"],
    },
    proxy: {
      "/api": { target: backend },
      "/ws": { target: backend, ws: true },
    },
  },
  test: {
    // The suite stays in node: the stores and the controller need no DOM. A
    // file that mounts a component opts in with `// @vitest-environment jsdom`
    // on its first line (`src/components/CockpitScreen.test.ts`).
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
