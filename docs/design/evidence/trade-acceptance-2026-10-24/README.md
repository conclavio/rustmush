# Trade acceptance evidence

The genuine rate-24 passage and second acceptance visit passed on 2026-10-25.
The second visit includes one documented harness reconnect after a read-only
assertion mismatch. See [the workstream record](../../25-trade-acceptance.md).

- `story-tests.log`: eleven production-composition stories, including public boards.
- `watch-log-test.log`: the additional linkdead-grace/revival regression.
- `notice-suites.log`: 101 order, plan, port-tick and voyage regressions.
- `repair-clippy.log`: strict TacSim library Clippy after the notice repair.
- `release-final.log`, `source-manifest.json`, `release-deploy.json`: build, source
  identity and the release running on dev1. `post-build-format.patch` records
  the final formatting-only relocation of one module declaration.
- `before-day1-audit.json`: fresh pre-enrolment ledger and goods baseline.
- `day1-departure-audit.json`: fully loaded hold, one burn ticket and conservation.
- `passage.json`: actual departure, arrival and durable notices during grace.
- `restart.json`, `after-restart-audit.json`: real child restart and unchanged
  journey, ticket, ledger, inventory, watch log and clock.
- `boards-underway.json`: live public departure row and origin gallery behavior.

Only inspected test output and public audit summaries are copied here. Full
Redis snapshots, raw live sessions and login credentials remain on dev1. The
first diagnostic attempt is separately preserved there and is not accepted.

- `before-day2-audit.json`, `after-day2-audit.json`: conservation, capacity and
  seeded lineage before/after the second visit, with settled commerce.
- `session-two.json`: eleven watch notices delivered once, no replay on harness
  recovery, Yard/boards walk, the new return departure and final statement.
- `merged-source-manifest.json`: source delta integrated with `f171b36c`; this
  describes the final regression build, not the earlier overnight live release.
- `merged-story-tests.log`, `merged-clippy.log`: all twelve production-composition
  stories and strict TacSim Clippy after integration with current main.
