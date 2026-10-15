# Doc 18 cut notes

The per-cut record for [18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) §5: what
each cut landed, what it deferred and why, every pin it re-blessed, and how it was verified. One
heading per cut, in the order they land.

## IC0

**Engine blockers first** (doc 18 §5, the IC0 row and paragraph), built 2026-10-15 on main at
8a32734a, minus the one item that rests on an owner question. One commit, pushed to origin/main.

### What landed

1. **Lot deliveries short-circuit** (`mush-market/src/module.rs`). `deliver()` and `cannot_deliver`
   now share one predicate, `settles_inside_the_contract`, true for `Representation::Asset` **and**
   `Representation::Lot`. A lot's matter leg is the market's own `market_lot` action staged at
   activation, so a settled lot sale no longer walks an empty delivery table and pages
   `SettlementRefused` about a buyer who holds exactly what they paid for.
2. **Listing TTL ≤ hold TTL, and the hold is read where money moves**
   (`mush-market/src/{api,commands,trade,lot}.rs`). `@market/vendor stock` caps `days=` at
   `RESERVATION_TTL_IC_DAYS` for a lot listing, and `api::list_goods` cuts the listing's real-time
   window to the hold's seven IC days converted at the world's current rate
   (`lot::hold_ttl_real_secs`; a paused clock caps nothing). `trade::quote` and `trade::is_current`
   read the hold itself through the typed-state seam the composition delegates to the market
   (`lot::hold_standing`, the sweep's own reconcile read): at accept anything but a live hold —
   lapsed, consumed, released, never placed, on another clock generation, or unreadable from the
   context — refuses with `STALE_QUOTE` and nothing charged; at quote a lapsed or gone hold refuses
   and an unreadable one is let through, because a quote reserves nothing and the accept re-reads.
3. **The non-staff manual-route arm is closed** (`mush-sim/src/commands.rs` `build_route`). A
   non-admin line that lists places is refused with the new `MANUAL_ROUTE_STAFF_ONLY` **before**
   the catalog is consulted, so an authored id, an id from elsewhere and a nonsense id read
   identically with a place list behind them (fold O12 kept). The authored route by its bare id is
   still the crew's to fly; staff keep the manual fixture arm exactly as cut 6 built it.
4. **Major-unit money lines market-wide** (`mush-market/src/commands.rs`). One render, `money`,
   which is `mush_economy::render_amount` on the instrument the record settles in (a listing's
   vendor account; a quote's listing or, once a lot listing is gone, its seller's vendor record; a
   venue's first vendor for `+market/depth`; the first vendor for a catalogue baseline), and the
   bare minor value only when nothing knows the instrument. `+market/list`, `/show`, `/depth`,
   `/quote`, `/orders`, `@market/catalog show` and the economist's `market.quote` all go through
   it; `price=<n>` still parses minor units like every economy verb. A 700-centicredit price
   reads `7.00cr` (the credit's `InstrumentDisplay` is the suffix `cr`, and a display symbol may not
   carry whitespace, so the `7.00 cr` doc 18 writes is not what the economy's one render yields —
   flagged below).
5. **The economist answers the asker** (`mush-market/src/interface.rs`). `market.list` and
   `market.quote` default to `ReplyAudience::Asker`: the list was filtered by `may_view_listing`
   as the asker and the quote is their record.
6. **The lot tour's driver** (`mush-atlas/content/region.limbo.json`). `npc.aaron_samuels` →
   `npc.aaron`, the OOC area's actual entry; the bus now announces itself by name instead of "The
   driver". A new pin, `ic0_every_shipped_service_driver_names_an_npc_entry_that_exists`, checks
   every shipped service driver against the pack it points at (the validator checks the vehicle,
   not the driver).
7. **The proving loop's pay arrows are a gate** (`mush-market/tests/ic0.rs`,
   `ic0_a_lot_sale_settles_and_pages_nothing`): stock → quote → accept → settle through the verbs
   a player types, on the production composition in miniature (org + economy with the vendor's
   party cap and the market observer + inventory delegating to `market` + the market with one lot
   catalogue entry), landing each queued authoritative submission the way the world task does;
   asserts the trade-tape row, the money moved once, the shop's pool lost the hold, the buyer's
   hold gained one live lot, and the ladder holds **no** `SettlementRefused` page. Toggling item 1
   off makes it fail with the exact page it was written against.
   `mush-server/tests/e7r2_proving_loop.rs` keeps its staff-act audit and its header now points
   at the gate.

**Found by the gate and fixed in the same cut:** `MarketStore::alloc_listing` allocates listing
ids from **zero** on a fresh world, and the `market_lot` action's offer schema floored `listing`
at **one** — so the very first lot listing any world published queued its reserve, had it refused
at resolution with `InvalidParameters`, and advertised a hold nobody placed until the sweep
withdrew it and paged. The floor is now zero (`mush-market/src/lot.rs`).

### Tests added

- `mush-market/tests/ic0.rs` (new): the settle gate; `ic0_a_lot_listing_stands_no_longer_than_its_hold`
  (rate 1 and rate 4); `ic0_a_quote_against_a_lapsed_hold_is_refused_before_money_moves` (the IC
  calendar re-phased seven days forward while real time stands still, so only the hold lapsed:
  accept refused, nothing charged, no contract, no matter leg staged, the hold untouched, a fresh
  quote refused the same way); `ic0_every_price_line_reads_in_major_units` (list, quote, depth,
  catalogue baseline, orders); `ic0_the_economist_answers_the_asker_and_not_the_room` (both
  abilities `Asker`, the quote in major units, through a detached typed-state context).
- `mush-market/src/interface.rs`: the existing unit test asserts the list's audience.
- `mush-sim/tests/e7c6.rs`: `ic0_a_non_staff_place_list_is_refused_before_the_catalog_is_consulted`
  (the doc's `@journey/depart x=r #12:1` line refused, no journey, identical for an authored and a
  nonsense id, `/plan` closed the same way, staff keep the arm).
- `mush-atlas/tests/starter.rs`: the shipped-driver pin above.

### Pins re-blessed

- `region.limbo` content digest `f127f7d7cc79b6d1f8f2e0b11f49f57497e101af0fe19cdd8cfaad0631501ef0`
  → `46beb771027b8fa0e4056d9374259f4a32321c8b308aeb0e721c737b7b1cc94e`, in the pack's own
  `digest` field and in `mush-atlas/tests/starter.rs` `PINNED`. No other pin moved; the panic
  budgets are unchanged.

### Deferred, and why

- **`spacecraft` out of `SELF_MODES` and `options_from_corridor_room`** — excluded from this cut
  by the brief; waits on Q-18-6.
- **The `region.limbo` version bump.** The brief asked for one beside the driver fix; the pack
  stays at version 2 with a re-derived digest, for three reasons. The only kept world, dev1,
  still stands at region **1** and owes the hand apply 1 → 2 (deploy memory), so the corrected v2
  rides that apply and no world is left holding the stale key. A bump to 3 cascades: both
  neighborhoods `require` `region.limbo@2` exactly (`check_requires` matches the version, not
  "at least"), so their manifests and digests move too, and `mush-atlas/tests/interior.rs`
  rebuilds historical fixtures (`ooc_v3`, `ooc_v6`, `staff_lobby_v4`) from the shipped bytes with
  pinned digests that assume `region.limbo@2` — a four-pack, multi-fixture change for a one-token
  fix. And a world that *had* applied v2 at the old digest would need v3 to re-apply the
  connection (the latch answers "applied at this version; nothing to do" for a same-version
  digest change) — no such world exists pre-launch; if one appears, bump then.
- **The settle gate on the real roster.** `tacsim`'s composition passes `with_party_caps(Vec::new())`,
  so a vendor organization cannot accept programmatically there and the money leg stops at "that
  vendor cannot complete the sale" (honest, nothing charged). The gate therefore runs on the
  composition in miniature with the vendor's cap wired; doc 18 cut IC5's `with_party_cap_source`
  is what brings it to `server_modules_with`, and IC5's own verify lens names that run.
- **The sweep and a lapsed hold.** A lot listing whose hold lapsed is now unsellable (quote and
  accept refuse) and, under the cap, lapses from the page at the same real moment under the rate
  it was listed at; but a rate change after listing can leave it on the page until its real-time
  deadline, and the sweep does not withdraw it — a withdrawal must also release the hold, which is
  the custodian's authoritative submission and not a GOD tick's to stage (the tick-cadence rule).
  The seller withdraws it by hand; a sweep-side release is a design question for a later cut.
- **`7.00cr` vs `7.00 cr`.** The economy's one render puts a suffix symbol flush against the
  digits and `InstrumentDisplay` refuses whitespace, so the credit reads `7.00cr`. Doc 18's
  `7.00 cr` would need either a render rule change in `mush-economy` or a display change on
  `tacsim.credit` — the owner's call, not this cut's.
- The `docs/design/README.md` row for this file is left to the doc 18 revision in flight.

### Pins amended, with the reason

- `mush-sim/tests/e7r1.rs` (E7r1 R6, four tests): the crew-driven departure tests typed the
  manual place list (`corridor #field:100 #phobos:200`) as the pilot and the owner, which is
  the arm item 3 closes. They now fly an authored `corridor` (field → phobos, the same two
  legs, no propellant) through the `RouteCatalog` seam by its bare id, and the staff diverts
  type `detour` with a place list. What R6 proves — a billeted pilot, or the hull's owner, puts
  the hull under way with no staff step; a bystander and a channel-less gunner are refused
  byte-identically; `/divert` stays staff — is unchanged.

### Verification

On Windows at the commit: `cargo test -p mush-market -p mush-sim -p mush-atlas -p mush-inventory
-p mush-contract-tests` green (the four e7r1 pins above amended, everything else untouched),
`cargo clippy --workspace --all-targets` clean, `cargo fmt --check` clean on the touched crates,
the panic budgets unchanged (no production panic surface added: the new paths are
`Option`/`Result` throughout).
