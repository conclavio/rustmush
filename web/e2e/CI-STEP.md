# TS.9's CI steps, for the `rest-mushd` cell

Doc 43 cut TS.9 (D-43-8, §10.1) puts the browser smoke in the cell that already
builds `mushd`, not in the npm-only `web` job. This branch does not edit
`.github/workflows/ci.yml` (another session owns it); these are the steps to
add to the `test` job, after the `rest-mushd`-gated "Panic hook and per-half log
files" step, so they run after that cell's `cargo nextest run -P ci -p mushd`.

## The binary

`cargo nextest run -p mushd` leaves `target/debug/mushd`: the integration tests
read `env!("CARGO_BIN_EXE_mushd")` (`tests/durable_process/cli_args.rs` and its
siblings), so cargo builds the bin target into `target/debug/` beside the test
binaries. The smoke uses that binary as it is. A separate
`cargo build -p mushd` would not be free: under resolver 2 the test build
unifies mushd's dev-dependency features (`mush-server/fault-injection`) into
that bin, and a plain build without them is a different fingerprint, which
relinks mushd and recompiles `mush-server` and everything above it. The
fault-injection seam is inert unless a test arms it, so the smoke gets the
same server the cell's own process tests boot. The fallback build below runs
only if the binary is somehow missing.

## The steps

```yaml
      # Doc 43 cut TS.9 (D-43-8): the one browser smoke. It runs in this cell
      # because the cell already built `target/debug/mushd` (the integration
      # tests' CARGO_BIN_EXE_mushd); `npm run build` builds the SPA it serves,
      # which needs the wasm32 target for the picture module
      # (web/scripts/build-wasm.mjs), as the `web` job does.
      - name: Browser smoke (TS.9) - the wasm target
        if: matrix.shard == 'rest-mushd'
        run: rustup target add wasm32-unknown-unknown
      - name: Browser smoke (TS.9) - node
        if: matrix.shard == 'rest-mushd'
        uses: actions/setup-node@v4
        with:
          node-version: "22"
          cache: npm
          cache-dependency-path: web/package-lock.json
      - name: Browser smoke (TS.9) - the SPA and Chromium
        if: matrix.shard == 'rest-mushd'
        working-directory: web
        run: |
          npm ci
          npm run build
          npx playwright install --with-deps chromium
      - name: Browser smoke (TS.9) - the mushd binary
        if: matrix.shard == 'rest-mushd'
        run: |
          # Built by the nextest sweep above; built here only if it is not.
          test -x target/debug/mushd || cargo build -p mushd
      - name: Browser smoke (TS.9)
        if: matrix.shard == 'rest-mushd'
        working-directory: web
        env:
          MUSHD_BIN: ${{ github.workspace }}/target/debug/mushd
        run: npm run test:e2e
      - name: Browser smoke (TS.9) - report
        if: failure() && matrix.shard == 'rest-mushd'
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: |
            web/playwright-report/
            web/test-results/
          if-no-files-found: ignore
```

Notes for whoever lands it:

- `CI=true` (set on every hosted runner) turns on the HTML reporter, the list
  reporter and `forbidOnly` (`web/playwright.config.ts`). On failure
  `web/test-results/` holds the trace, the screenshot and `mushd.log` (the
  server's log, attached by the spec); `web/playwright-report/` the HTML report.
- `npm run build` also runs `svelte-check`; the `web` job already gates it, so a
  red here on that step is the same red as there.
- Costs measured on the owner's Windows box (debug `mushd`, warm caches):
  `npm run build` 51 s; the smoke itself 6.5–10 s of test time and 16–52 s of
  wall per run including the boot of `mushd` (the boot dominates and varies).
  The Chromium download (`playwright install --with-deps chromium`) is roughly
  150 MB plus the apt packages; caching `~/.cache/ms-playwright` keyed on the
  `@playwright/test` version in `web/package-lock.json` is an optional saving.
- The binary boots on the in-memory store with `[tacsim].enabled = true` (the
  simulator is what gives a fresh character a cockpit to paint), both
  listeners on loopback ports `web/e2e/boot-mushd.mjs` picks, and a scratch
  `data_dir` it removes after the run. Nothing else on the runner is needed:
  no Redis, no network.
