# UI and Discord review fixes

2026-09-07. Implementation base: `b8ec25c`, including the unrelated pulse fixes that followed the review at `a18b345`. Scope: the seven findings in `2026-09-07-ui-discord-review.md`. Requests remain advisory and RAM-only; consent remains advisory. No live Discord sends, deployment, or unrelated tactical work is required.

## Implementation plan

1. **F2: request identity.** Add a random 128-bit engine epoch to Request/Respond, carried unchanged through web, SSH/TUI, Discord and IPC. Reject missing or mismatched epochs at the engine before consuming the pending request. Preserve the small sequential id and oldest-first ordering within an epoch. Generate the epoch from OS entropy and fail at construction if entropy is unavailable. Clear obsolete web and Discord controls on Hello; bind Discord request answers to the message that displayed the current request. Regenerate TypeScript wire types. Tests: two engine instances; missing/mismatched epoch; valid response; round-trip and client restart behavior.
2. **F7: turn lifetime.** Add a defaulted turn-visit generation to scene records. Change it on advance, rewind, resume, participant-driven turn changes, and nudge policy resets; carry it in nudge continuations and refuse a response for another generation. Turning nudges off invalidates outstanding nudges. Preserve round/index as position. Tests: advance then rewind, off/on, pause/resume, ordinary Pass and Hold.
3. **F3: DM draining.** Keep receiving while one REST flush is in flight. Bound queued narrative and priority work, preserve priority ordering and retry backoff, and put retried work ahead of newer queued work without exceeding caps. An exhausted priority lane explicitly detaches rather than silently dropping an ask. Cancellation drops pending output; graceful close has a bounded flush. Tests use a stalled fake sender, overflow/drop marker, priority during a send, retry ordering, and cancellation.
4. **F1: revocation.** Add an explicit gateway revocation control and emit it for every character on the account after a successful unlink or ban, before reporting success. Invalidate the follow synchronously when control arrives, cancel its writer, and detach through the shared connection path. Check the current account/link/ban on input and interaction, fail closed on lookup errors, and revalidate an attach after asynchronous work. Avoid an attach writing a stale cached link back over an unlink. Tests cover both commands, an alt character, incoming buttons, an attach racing revocation, and outbound cancellation.
5. **F4–F6: web interaction.** Keep native keys on focused interactive controls and honor already-handled events. Suspend shell keys during dialogs. Use a native modal dialog for requests, with initial focus, browser focus containment/background inertness, request-change focus, Escape/cancel, and focus/draft restoration. Retain an unsent request and its answer on failed transport admission, show an inline retry message, and expire it normally. Tests cover failed/successful send and request replacement; browser checks cover Enter, Tab/Shift+Tab, background typing, Escape, queued asks, and composer restoration.
6. **Integration.** Promote the six review probes into the relevant maintained suites, add focused tests for the newly exposed races, update help/operational contract notes, then run the affected crate suites, web tests/build, format and clippy. Broaden to workspace checks once targeted tests pass. Record exact results and remaining limits here.

## Review of the plan before implementation

- A randomized numeric starting id was rejected: JavaScript's integer precision and a counter range shared by multiple restarts make a separate opaque epoch clearer. Legacy replies deserialize with an empty epoch and fail closed; generated clients echo the epoch. Keep a rolling-upgrade note because old gateway/client code cannot safely answer the new requests.
- Client clearing is cleanup, not authorization. The engine epoch is mandatory even when a restart notification is delayed. Discord checks must validate the originating message before forwarding or marking an ask answered; custom-id limits must still hold.
- Revocation must cover an attachment still awaiting REST/world admission, not only entries already visible in the follow map. A cached DM-channel update must not recreate a revoked link. Writer cancellation must stop queued work; it cannot recall a message Discord already accepted.
- Splitting the writer without bounding the priority lane would move the memory problem. Preserve one send in flight, a bounded final flush, and a fixed retry deadline that new arrivals cannot shorten.
- Position is not turn identity. Rewinds and policy resets must invalidate the previous offer even when actor, round, and index compare equal. Generation exhaustion must fail closed rather than wrap to a previous offer.
- ARIA alone does not enforce a modal. Use native dialog behavior and actual browser checks; source probes and pure key-map tests do not establish focus behavior. A failed response keeps the dialog, choice text, and server deadline.

Review outcome: proceed with this revised plan. The existing authorization covers the fixes and their tests. No owner policy decision or external publication is needed.

## Execution record

All seven findings are implemented and reviewed on `codex/ui-discord-review-fixes`, in a separate worktree so the original design edits remain intact. Verification is complete, with one unrelated baseline test failure documented below. Validation did not exercise a live Discord account or deployment.

### Review of the implementation

- Revocation cancels a writer before asynchronous session cleanup. Attachments check both current storage and a revocation generation before publication and after world admission. The generation deliberately invalidates any concurrent attachment when a revocation arrives; an unrelated attachment may need one retry. Existing follows on other accounts keep running. Both self/staff unlink and ban issue controls for every character on the affected account.
- Ordinary detach still flushes its goodbye. Revocation drops pending work. A writer that exits because of a closed DM or priority overflow also removes its shared connection, avoiding an orphan session. At most 127 waiting priority items and one in-flight batch are retained; narrative is capped by `queue_lines`, plus one bounded batch. A closing writer has a five-second limit; new arrivals cannot shorten a retry deadline.
- Replies carry `(epoch, id)` through every transport. Discord also compares the originating message and retires only that message, so an answer or expiry timer cannot remove a replacement request. Missing epochs fail closed in the engine. The engine's line-mode `respond` command deliberately answers its current oldest ask.
- Turn visits advance on movement through turn order, rewinds, participant removal, resume, and nudge-policy resets. The maximum generation is a disabled-nudge sentinel. Old scene records default the new field to zero.
- Native request dialogs make the background inert, explicitly wrap Tab/Shift+Tab, focus each queued ask, and restore prior focus and the composer draft when finished. Failed transport admission preserves the request and typed answer, shows retry text, and retains normal expiry.

### Upgrade and operational notes

The gateway/server IPC version is now **5**. Upgrade both processes together; restart the gateway as part of deployment rather than hot-restarting only the server against an older gateway. Rebuild the web bundle with the generated bindings. Old clients can deserialize the additional request field but cannot answer safely until refreshed; their missing-epoch replies are ignored and the server's deadline/default still applies.

The DM channel is resolved idempotently for each follow and is no longer written back from an attachment's cached account snapshot. The legacy storage field remains readable for compatibility. Revocation cannot recall a message already accepted by Discord, and transport admission does not constitute a server acknowledgement.

### Reproducible browser checks

Run `npm run dev` in `web`, then open `/tests/ui-review.html`. The fixture mounts the real Game and RequestModal components, uses a fake local transport, and records submitted frames in scrollback. It never connects to the game or Discord.

1. Focus the notifications button: Enter activates it; Tab advances to the theme button.
2. Click **Ask after 3s**, focus the composer, and type a draft. The request focuses Yes, hides the background from the accessibility tree, and typing cannot change that draft. Tab and Shift+Tab wrap through the request controls. Escape sends Cancel and restores the draft and focus.
3. **Queue asks** opens a confirm followed by text. Enter on Yes advances to the text field; submitting text restores the original control's focus.
4. **Offline ask** refuses sends for 15 seconds. Submit text and verify the inline error and retained value. Retry after recovery and verify the original epoch/id/value in scrollback.
5. At a 375 × 812 viewport, verify the bottom sheet, visible actions, and bounded width. Restore the viewport afterward.

These browser checks passed during implementation, including the phone viewport and reconnect retry. The temporary browser tab and development server were closed afterward.

### Final verification

| Check | Result |
| --- | --- |
| Rust workspace unit/integration tests (`cargo test --workspace --lib --tests --no-fail-fast -j 2`) | 4,722 passed, 1 baseline failure, 64 ignored across 224 test targets |
| Rust workspace doctests (`cargo test --workspace --doc -j 2`) | 15 passed, 2 ignored across 44 targets |
| Workspace clippy (`cargo clippy --workspace --all-targets -j 2 -- -D warnings`) | Passed |
| Rust formatting (`cargo fmt --all --check`) and `git diff --check` | Passed |
| Generated TypeScript bindings (`cargo gen-ts`) | Passed, including both export tests |
| Web tests (`npm test`) | 410 passed across 40 files |
| Web production build (`npm run build`) | Passed; Svelte reported zero errors and zero warnings |
| Browser interaction and phone-layout checks above | Passed |

The sole workspace failure is `mush-campaign::e7c11::vf_b8_every_station_lane_verb_has_a_recorded_proxy_disposition` at `crates/mush-campaign/tests/e7c11.rs:1404`. Its source census expects zero `seat_restriction` calls in `mush-sim/src/tick.rs`; the pulse changes already present in base `b8ec25c` contain two. The census test and every source file it scans are byte-for-byte unchanged from that base according to `git diff --exit-code b8ec25c --` for the test, `mush-sim/src/{commands,combat,station,tick,seat}.rs`, and `mush-propagation/src/commands.rs`. This unrelated failure remains open; the full workspace suite is therefore not green.

Focused regression coverage includes cross-engine replies, stale Discord messages and expiry races, unlink/ban during both DM creation and world admission, synchronous output cancellation, bounded queue draining under a stalled sender, fixed retry backoff, and stale scene nudges after rewinds or policy changes. The workspace run covers these tests with the final implementation.

Validation used fake Discord APIs and a local browser fixture. It did not exercise a live Discord deployment; existing ignored and externally gated tests retain their normal behavior. Local command logs are retained under the ignored `logs/ui-discord-review/` directory. Rust commands reused the earlier review's target cache and limited broad builds to two jobs.

### Integration before publication

Rebased onto `553058b` after the E7 economy merge reached `origin/main`. The rebase had no conflicts, and `git range-diff` confirmed the fix patch was unchanged. Re-ran the gateway, Discord transport, and server library suites together with `--locked` against this updated base; all passed. Formatting and diff whitespace checks also passed. The broader workspace and web results above were collected on the original implementation base; no web files changed upstream during this integration.
