// Doc 43 cut TS.9 (D-43-8): the one browser smoke, Chromium only. It is the
// only automatic run of the real canvas and socket path, so it is kept to one
// journey, one worker and no retries: a flake here is a finding, not noise to
// retry past.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.ts",
  // `e2e/boot-mushd.mjs` boots one `mushd` for the run and hands back its
  // teardown; the spec reads the server's address from `MUSHD_BASE_URL`.
  globalSetup: "./e2e/boot-mushd.mjs",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  // The journey's longest wait is the arena: a match boarding, the seat, the
  // first cockpit frame. Each step carries its own bound well under this.
  timeout: 180_000,
  expect: { timeout: 20_000 },
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: {
    ...devices["Desktop Chrome"],
    // A desktop: the cockpit lays the picture beside the map at this width.
    viewport: { width: 1600, height: 1000 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
