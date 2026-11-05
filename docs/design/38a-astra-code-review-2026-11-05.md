# 38a Astra code review 2026-11-05

Review window: 2026-11-02 07:18:26 PST through 2026-11-05 07:18:26 PST. Reviewed head: `57f4c7e66c43f9f67060d276f8f567575432cbd2`. Comparison base: `0f08e3d2dfbe8eed7e03f86977c319c6304783c9`, the last first-parent commit before the window. The window contains 761 timestamp-selected commits and 160 mainline merges. The cumulative change touches 870 files, with 144,343 insertions and 16,735 deletions.

This report preserves the findings at the reviewed head. Later merges into main have not been re-reviewed here.

The review found 12 actionable defects in the implemented portions of design docs 23, 39, 40, 41 and 42, using doc 38 as the programme map. Five are P1 because they permit payment or enforcement bypasses, destroy purchased output, strand escrowed property, or remove a unit from combat after failed re-entry. Deferred cuts and accepted design limitations are excluded.

**1. P1 — Unpaid debts stop enforcing refusal when reinstated.**

[crates/tacsim/src/surface/retrieval.rs:1158](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/surface/retrieval.rs#L1158). The first desk attempt changes a delinquent obligation to Active without collecting payment. Both `delinquent_debts` and port security's debt check recognize only Delinquent, so a second `+tv/lease` or `+coop/enrol` before the next sweep bypasses the refusal; a corresponding fine also ceases to hold the carrier. Preserve enforcement until settlement, or reinstate and attempt payment atomically. Violates [doc 41](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/41-restoration-and-the-salvage-loop.md) Q-41-13 and Q-41-25. Introduced by `7435853153`; release fees joined this path in `3837c935cd`. Probe: `review_72h_unpaid_debt_must_still_refuse_a_second_desk_attempt`.

**2. P1 — Reposting a delivered retrieval removes the old payment fact.**

[crates/tacsim/src/surface/retrieval.rs:528](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/surface/retrieval.rs#L528). The replacement guard tests `active()`, which excludes Delivered. A poster can repost the hull after workshop delivery and before the settlement sweep; the new serial overwrites the record. The old contract's fact provider now returns None, and settlement re-resolves that fact, preventing the completed retriever's payment. Keep each delivery fact by serial or reject reposting until the existing contract has settled. Violates [doc 41](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/41-restoration-and-the-salvage-loop.md) Q-41-8 and §4.3. Introduced by `7435853153`. Probe: `review_72h_reposting_must_preserve_unpaid_retrieval_delivery_fact`.

**3. P1 — A multiple-hull yard order consumes all kits for one hull.**

[crates/tacsim/src/port/works.rs:1268](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/port/works.rs#L1268). `+yard/order tv_escort 2` accepts and scales the bill, but commissioning selects one kit, creates one commission record, and consumes every escrow lot. Additional kits are treated as parts, or their quantity is ignored. The player loses the additional paid hulls. Either enforce quantity one before accepting payment or commission every purchased kit. Violates [doc 39](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/39-industry-production-chain.md) §4.12–4.13. Introduced by IN.9, `9a2c835626` / `572c7996e0`. Probe: `review_72h_yard_quantity_creates_each_paid_hull`.

**4. P1 — Cancelling a supplied yard order strands its parts in escrow.**

[crates/tacsim/src/port/works.rs:1442](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/port/works.rs#L1442). `escrow_contents` reports identified parts both as bulk goods and as lots because `pooled()` uses a reader that includes part-lot tallies. The return action first tries to transfer a nonexistent bulk pool and refuses before relocating the actual part. Cancellation retries and collection cannot return the supplied property. Exclude identified parts from the bulk return list. Violates [doc 39](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/39-industry-production-chain.md) §4.12's whole-bill return and §4.11's exact-lot lifecycle. Exposed by `63a57e777` over the escrow reader introduced in `f0d2340aea`. Probe: `review_72h_yard_cancel_returns_identified_parts`.

**5. P1 — A rejected boarding rejoin has no retry.**

[crates/tacsim/src/field/boarding.rs:249](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/field/boarding.rs#L249). When a carrier is lost during boarding, `rejoin()` succeeds as soon as the Join message is queued, and `go_back()` deletes the boarding record. Kernel refusal then removes the unit from the live field without restoring that record. A licensed hull reaches refusal because this join supplies no retained source; occupied placement can also refuse. The hull stays outside the running battle with no retry. Keep the boarding state until acknowledgement and supply the required retained source. Violates [doc 40](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/40-field-second-round.md)'s ramp boarding ruling that the hull returns to the field. Introduced by `b34208d82`. Probe: `review_72h_a_refused_boarding_rejoin_remains_retryable`.

**6. P2 — Commissioning erases input part condition and provenance restrictions.**

[crates/tacsim/src/port/commission.rs:563](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/port/commission.rs#L563). The yard constructs a pristine registry specification without applying the supplied parts' condition or recording their fitted destination. A damaged stripped sensor becomes intact. The same path contains no rejection of hot provenance. Validate every supplied part and carry its condition and ancestry into the commissioned hull. Violates [doc 39](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/39-industry-production-chain.md) §4.11 and Q-39-27, which explicitly preserve stripped parts' claim and condition and refuse hot parts at a build. Introduced by `9a2c835626`. Probe: `review_72h_yard_keeps_a_used_parts_condition`.

**7. P2 — Capturing multiple crew leaves some station assignments behind.**

[crates/tacsim/src/field/crew.rs:1073](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/field/crew.rs#L1073). Capture processes crew by character ID, but `vacate_seat` examines only the first occupied station by station ID and returns when that station belongs to someone else. Those orders can differ on a multi-crew space carrier. Both bodies move and both records become Captured, yet one keeps its original assignment and simulation channel grants. Iterate every station held by each captive and release its control lease. Violates [doc 42](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/42-crew-custody-and-capture.md) Q-42-1's requirement to vacate every station before moving the body. Introduced by `ff906fe81`, exposed by CR.6's multi-crew space path. Probe: `review_72h_capture_vacates_every_captive_station`.

**8. P2 — Mid-field relay damage does not affect orders.**

[crates/tacsim/src/field/orders.rs:207](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/field/orders.rs#L207). The relay gate reads the world unit's check-out preimage. An intact Halcyon relay jammed or damaged inside the arena therefore continues passing orders immediately until check-in; no kernel relay gate supplies the missing behavior. Read the live arena condition or mirror its updates before offering orders. Violates [doc 40](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/40-field-second-round.md) Q-40-3 and Q-40-5's same-field hold or ten-second delay. Introduced by `5a9107b66`. Validation: source trace through the world read, arena custody, and order gate; no runtime probe.

The relay limitation is already disclosed in doc 40's cut notes at lines 705–707. It remains a mismatch with the binding Q-40-5 behavior, rather than a newly discovered implementation behavior.

**9. P2 — Tycho skips accepted retrievals whose destination is not Ceres.**

[crates/tacsim/src/port/house_arm.rs:239](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/port/house_arm.rs#L239). Retrieval requests accept a selected workshop port, but the only house arm selects jobs whose destination equals its Ceres home. A request to Stickney can pass the public window and be handed to Tycho, yet the arm never works it and the job expires. Route the house to the requested port or refuse unsupported destinations before accepting the request. Violates [doc 41](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/41-restoration-and-the-salvage-loop.md) Q-41-9 and Q-41-11. Introduced by `9d82910d8a`. Validation: source trace of request acceptance, fallback and house selection.

**10. P2 — Yard registry marks repeat after the 65th commission.**

[crates/tacsim/src/port/commission.rs:252](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/port/commission.rs#L252). To skip house seeds, `next_mark` excludes every commission whose index is at least 64. The 65th ordinary commission itself receives index 64 and is then excluded, so subsequent builds repeat index 64, mark SYB-65, and the ship mooring coordinate. Distinguish house seeds by their actual identity and allocate fresh ordinary indices. Violates [doc 39](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/39-industry-production-chain.md) §4.13's fresh registry identity. Introduced by `1b80f2adc4`. Validation: deterministic source trace of the counter and filter.

**11. P2 — Every subsequent field journals another carried ABI drop.**

[crates/tacsim/src/arena_bridge.rs:1815](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/arena_bridge.rs#L1815). Retained installation runs before the carried mind is seeded. On a later field of the same epoch, installation sees fresh `dropped: None`, journals another ABI drop and emits another notification, then restores the already-dropped carried state. Restore the carried drop before deciding whether to journal. Violates [doc 23](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/23-ic-sorties.md) §17.6 / D-23-17's once-per-epoch rule. Introduced by `3ec9a7005`. Probe: `review_72h_carried_abi_drop_is_not_journalled_again_in_the_next_field`.

**12. P2 — Ejection after a field mistakes a cab room for its hull.**

[crates/tacsim/src/field/eject.rs:129](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/crates/tacsim/src/field/eject.rs#L129). The post-field branch uses the pilot's immediate object location as the unit. A seated pilot in an authored cab occupies a room, so the station check fails. A mobile hull with a damaged control section remains a Unit crew record at check-in and is specifically eligible to eject, but receives the not-seated refusal. Resolve the containing unit through the station/interior model. Violates [doc 42](https://github.com/conclavio/rustmush/blob/57f4c7e66c43f9f67060d276f8f567575432cbd2/docs/design/42-crew-custody-and-capture.md) Q-42-10 and Q-42-11. Introduced in CR.4, `0e8d03812` / `e9919752c`. Validation: source trace through cab placement, check-in and the post-field branch.

Eight added regression probes fail at the intended assertions and reproduce findings 1–7 and 11. The 11 existing crew unit tests passed. Findings 8, 9, 10 and 12 are established by source tracing, without a runtime reproduction. The relay mismatch in finding 8 was already disclosed in the cut notes.

This review follows the six requested design programmes and their changed code paths; it is not an exhaustive line-by-line audit of all 870 changed files. The full workspace suite was not run. This documentation commit changes no production code or active test files. The added regression tests are supplied as the bundled patch and were run in an isolated review worktree.

Evidence: [timestamp-selected commits](evidence/38a-astra-code-review-2026-11-05/commits-72h.tsv), [mainline merges](evidence/38a-astra-code-review-2026-11-05/mainline-72h.tsv), [changed files](evidence/38a-astra-code-review-2026-11-05/changed-files.tsv), [crew baseline](evidence/38a-astra-code-review-2026-11-05/crew-baseline.log), [library probes](evidence/38a-astra-code-review-2026-11-05/lib-probes.log), [economy probes](evidence/38a-astra-code-review-2026-11-05/domain-economy.log), [retrieval command reproduction](evidence/38a-astra-code-review-2026-11-05/retrieval-repost.log), [test-only patch](evidence/38a-astra-code-review-2026-11-05/regression-probes.patch).

To reproduce, run these commands from the repository root containing this report. They create a separate checkout of the reviewed commit, apply the bundled test-only patch, and run the eight probes:

```sh
git worktree add --detach .worktrees/reproduce-38a 57f4c7e66c43f9f67060d276f8f567575432cbd2
git -C .worktrees/reproduce-38a apply ../../docs/design/evidence/38a-astra-code-review-2026-11-05/regression-probes.patch
cargo test --manifest-path .worktrees/reproduce-38a/Cargo.toml -p tacsim --lib review_72h_
cargo test --manifest-path .worktrees/reproduce-38a/Cargo.toml -p tacsim --test domain_economy review_72h_ -- --test-threads=1
```

The probe commands are expected to report three library assertion failures and five economy assertion failures until the defects are corrected. The original runs used cached dependencies with --offline; add that flag only when the dependencies are already available. The bundled logs preserve the original run output, including local build paths.

Excluded from findings: explicitly deferred features; the same-second void behavior prescribed by Q-40-17; and the orphaned detachment snapshot issue that predates this window. Doc 38's historical survey statements were checked against the newer cuts rather than treated as an assertion that each proposed feature had shipped.
