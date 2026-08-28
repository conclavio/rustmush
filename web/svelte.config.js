import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  // Handles <script lang="ts"> blocks via Vite's esbuild pipeline.
  preprocess: vitePreprocess(),
};
