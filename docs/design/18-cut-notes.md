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

## IC1

**Two stations, the IC door and the IC-return rule** (doc 18 §5, the IC1 row and paragraph), built
2026-10-15 on main at 0ce0d0fa. One commit, pushed to origin/main.

### What landed

1. **The four station packs** (`crates/tacsim/content/atlas/region.ceres_freeport.json`,
   `neighborhood.ceres_freeport.json`, `region.stickney_yards.json`,
   `neighborhood.stickney_yards.json`), version 1, compiled into tacsim and digest-pinned. Ceres
   Freeport: 13 rooms on the Freeport Hub's three decks (Freight 0 at z 0, Public 1 at z 6, Office 2
   at z 10.5); Stickney Yards: 12 rooms on the Yard Rim's two (Freight 0 at z 0, Public 1 at z 7).
   18a's names, *Short.* lines, descs and details verbatim with the markdown (backticks, bold,
   italics) stripped and line wraps joined; the 24 exits exactly as the brief's table, including
   the **unlocked** Limbo door `exit.limbo_arrivals` (`Ceres Freeport <C>;c;ceres` /
   `Limbo <X>;x;limbo;ooc`) into C03 (D-18-3), the two public stairs and the Registry ladder with
   `portal.at` + `dz_m`, and the three hidden `lock: staff` ways in (C14→C15, C05→C16, P07→P15); six
   Ceres and seven Stickney `wall` features on every exitless public seam of 18a §0.4's boxes,
   `z_range [0, 4.5]`, blocking movement and sight; twelve non-blocking `counter` features; six
   storefront items in the two Maintenance Spines, no flags, no interior; the thirteen cast NPCs
   with interfaces ⊆ {atlas, travel, help, market},
   persona/speech/background/knows/does-not-know/fallback from 18a, and the nine principals
   `profile.enabled: false` with no interfaces or tags (C16: Wen, Varga, Ndiaye, Osei-Ferrante,
   Holm; C05: Kalu; P15: Castellanos, Tanaka, Bask); lore pools public and guarded only, each tag
   owned by one pack (Ceres: `port_ceres`, `corridor_trade`, `coop_lay`, `tether`, `tycho`;
   Stickney: `port_stickney`); 18a's three *Secret* facts as atlas `secret` entries, `reveal:
   staff`, with `about` (`secret.slowdown` → kalu, petrossian; `secret.hawk` → bask; `secret.tap`),
   so `secrets.leak` gates every station prompt field; the 2350 base theme on each region and the
   flavour theme (style guide, vocabulary, banned) on each neighborhood; twelve ambience lines each;
   hydration off; `port.dock` on C01 and P01 and no other room. Digests: `region.ceres_freeport`
   `80b18bc0…4f83f0`, `neighborhood.ceres_freeport` `383e0b13…1dc1cf`, `region.stickney_yards`
   `07e25b41…7047eeb2b1`, `neighborhood.stickney_yards` `6a2cb600…30def90b` (in full under "Pins
   re-blessed" and in `crates/tacsim/tests/stations.rs` `PINNED`).
2. **`RoomDef.markers` and the closed registry** (`crates/mush-atlas/src/def.rs`: `ROOM_MARKERS =
   ["port.dock"]`, `PORT_DOCK_MARKER`, `pub fn check_room_markers`, shared by the applier and the
   validator so plan and apply give one answer). `create_room` (`apply.rs`) refuses an unknown or
   duplicate marker before any mutation (`{id}: unknown room marker "port.dok" (a room may carry
   ["port.dock"])`) and, after `dress`, sets exactly the pack's markers as **object markers** on the
   room, clearing every registered marker the def leaves out — no `RoomGeo` field, no wire change.
   The validator (`validate.rs`) reports the new `schema.marker` error and seeds `topology()` with
   every room of the pack marked `port.dock` (D-18-19), so Stickney validates with no foot route.
3. **`RoomDef.short_desc` → the existing `short_desc` room attribute** (`def.rs` `SHORT_DESC_ATTR`,
   `MAX_SHORT_DESC_BYTES` 256; `apply.rs` `create_room`, rooms only, written as `dress` writes
   `desc`, blank removes it). The tree's one-line lane: worldgen writes it, the NPC module's turn
   and scene views read it before `desc`. `view.rs` is untouched and `+atlas` is byte-identical with
   and without the field.
4. **`NeighborhoodDef.ambience` / `NeighborhoodRec.ambience`** (`def.rs`, `record.rs`, `apply.rs`):
   `MAX_AMBIENCE_LINES` 24, `MAX_AMBIENCE_LINE_BYTES` 512, non-blank, refused before any write;
   `#[serde(default, skip_serializing_if = "Vec::is_empty")]` so a record without ambience
   serializes byte-identically and the starter digests cannot move; `NeighborhoodSummary` unchanged;
   read by no player surface (the emitter is a later cut).
5. **`spacecraft` out of `SELF_MODES`** (`crates/mush-atlas/src/travel.rs`: `["foot", "car",
   "transit"]`, doc comment citing D-18-49, Q-18-6, D-18-16), and **`options_from_corridor_room`
   deleted**: `options_from` returns `Vec::new()` for a room in no neighborhood and loses its
   now-unused `w: &World` parameter (`travel_cmd.rs`'s two callers updated). `CORRIDOR_MODE` keeps
   `"spacecraft"` with its doc comment replaced (`crates/tacsim/src/content.rs`): the atlas's
   projection of a carrier route, which no player verb travels.
6. **The boot list and the shelf** (`crates/tacsim/src/definitions/content.rs`: `CERES_REGION_PACK`,
   `CERES_PACK`, `STICKNEY_REGION_PACK`, `STICKNEY_PACK`, the four `*_JSON` includes,
   `STATION_SOURCES` in requires order, `CERES_ROOMS = 13`, `STICKNEY_ROOMS = 12`,
   `station_packs()`; `crates/tacsim/src/composition/mod.rs`: `pub fn boot_pack_list(starter_area,
   game_content)` extracted and used by `content_module_with`, "the game's six boot packs";
   `atlas_module` shelves `station_packs()` after `boot_packs()` on the same degrade-and-log rule).
   `boot_packs()` stays exactly the proving ground's two, because the arena fixtures `pop()` it.
7. **The IC-return rule** (`crates/tacsim/src/ic_return.rs`, new; `pub mod ic_return` and
   `ic_return::register(reg)?` after the tutorial in `lib.rs`, inheriting `LegacyPostImage`).
   `IcPort { id, name, area, arrival, foot_route }`, `PORTS` (ceres → `room.c03_arrivals`, foot
   route; stickney → `room.p01_berth_gallery`, none), `DOOR_PACK`/`DOOR_ENTRY`/`LAST_PORT_MARKER`,
   `port_of_room` (by `RoomGeo.neighborhood.pack`), `last_port` (two markers read as the first port
   with no foot route — fails closed), `stamp` (one marker set, the others cleared), `door` and
   `arrival_room` through `latched_object`. `IcReturnStamp`, a `Moved` observer: a Player moving
   from a port room to a room outside every port is stamped — C03's `x`, Limbo's global `o`/`s`/`c`
   typed in a station, `home`, a staff `@teleport`; a failed write warns once. `IcDoorHook`, a
   `HookPoint::MoveAttempt` veto on the door alone: standing in a port with no foot route → `There
   is no door from Stickney Yards to Ceres Freeport; the way there is by ship.`; stamped at one →
   doc 18 §4.5's `You left the corridor at Stickney Yards; +ic returns you there.`; no stamp or
   Ceres passes; no staff exemption. **`+ic`** (`ArgShape::None`, the brief's help verbatim) refuses
   `TacSim is not enabled on this server.` / `Only a character can do that.` / `You are already in
   the corridor.` / `There is no corridor on this world to return to.` in that order, then follows
   `go_home`'s shape — `check_move_attempt` (a veto prints as `Sys`; this is what lets IC4's
   `PlacementGate` refuse a hull occupant), `move_to`, `You return to the corridor at <port>.`,
   `<name> leaves.` / `<name> has arrived.`, `show_room`, `Moved`. It writes no document.
8. **The testkit's kept-store primitives** (`crates/mush-testkit/src/lib.rs`):
   `fold_module_ops_into` / `fold_module_ops` fold a drained batch the way the store would —
   `JsonSet` last write wins (and replaces a `log:` chunk already moved into `ledger`), `Del` is
   type-agnostic and removes a stream too (a stream appended to afterwards restarts at `1-0`),
   `StreamAppend` gets synthesized monotonic `"{n}-0"` ids trimmed to `maxlen`, then `normalize()` —
   and `TestWorld::reboot_into(modules, boot)` restarts a lived-in world into a **different**
   composition (every connection dropped, `Engine::with_modules` over the world with a fresh
   registry, `try_modules_loaded`, God reconnected).
9. **Contract gates and help**: `crates/mush-contract-tests/tests/exit_command_words.rs`
   `shipped_exit_segments()` walks `station_packs()` after the starter lot (no new module-verb
   overlap, no kernel or host word among the station aliases, so no `DECIDED` row and no alias
   dropped); the `+ic` stanza in `legacy-handlers.toml` (`legacy-post-image`, family
   `tacsim.ic_return`, `PINNED_BASELINE` 97); one paragraph in `docs/help/getting-started.md` naming
   the door `c` and `` `+ic` `` (T-45 green).
10. **Tests**: `crates/tacsim/tests/stations.rs` (new, 13: the pins with the
    `TACSIM_WRITE_CONTENT=1` re-sign mode, the boot list and shelf in order, validate clean with
    zero warnings, the adjacency check over 18a §0.4's boxes with the six and seven walls named and
    no `geometry.portal_gap`, the seams walled on the rasterized Public Deck with both cells
    straddling each door threshold open, Stickney unreachable without its marker (all 11 public
    rooms), a fresh world, a kept world rebooted from the exact old list into the production
    composition over a cumulative store fold with a third boot creating nothing, every public room
    walked both ways by its authored aliases with the three locked ways refusing a player and
    admitting GOD, the Stickney pilot refused at the door and returned by `+ic`, the Ceres and fresh
    pilot, and the principals never waking while Oduya answers with a verbatim fallback line through
    the real off-loop runner with the LLM disabled); `crates/mush-atlas/tests/markers.rs` (new, 4);
    `crates/mush-atlas/tests/validate.rs` (2, the seed and the refusal in validator **and** applier
    with nothing written); `crates/mush-atlas/tests/travel.rs`
    `s10_a_journey_under_way_ends_where_it_stands_when_its_road_is_reapplied` (the revision lens the
    replaced s9 test carried, now on the Valley Road, with `modules_shelved` and `region_v2`
    fixtures); `crates/tacsim/tests/s9_corridor.rs`
    `no_player_verb_traverses_a_spacecraft_connection` (written against `tacsim::CORRIDOR_MODE`);
    `crates/mush-server/tests/boot_parity.rs`
    `ic1_a_kept_world_takes_the_stations_when_the_game_is_switched_on` (the real split boot over one
    `InMemoryStore`: three latches → nine with the starter digests unmoved and 13 + 12 `RoomGeo` → a
    third boot creating nothing); three unit tests each in `ic_return.rs` and the testkit.

**Found by review and fixed in the same cut:** the fold's `Del` left a stream in place (the store's
DEL is type-agnostic); test 12's "never spoke" half could never fail because `TestWorld::fire_due`
drops its `Effects`; test 3 dropped its model's zero-warnings assertion, the only gate on the
stations' `banned` theme words; the ambience-bounds test checked only `is_err()`; and the replaced
s9 test was the only coverage of `travel.rs`'s "the way changed under it" guard (a mutation forcing
it true made the ported test fail).

### Deferred, and why

**Lead rulings taken by the planner** (recorded here so the owner sees them):

- **No new `enabled` field on the npc entry.** `ProfileDef.enabled: Option<bool>` already exists
  (`mush-npc/src/content.rs:97`, default true), so the principals use `profile.enabled: false` and
  mush-npc is unchanged.
- **The door hook is a tacsim `HookPoint::MoveAttempt` hook**, doc 18 §4.5's words; the lead note's
  "the atlas's MoveAttempt seam the onboarding gate already uses" names a `CommandGate` in the tree.
  The `OnboardingGate` is untouched and already refuses the door and `+ic` to an unapproved
  character.
- **The stamp is one unregistered object marker per port** (`tacsim.last_port.ceres` / `.stickney`,
  at most one present), because markers are booleans and an unregistered one cannot be forged by
  `@set` (doc 18 §3-A correction 2); written by a `Moved` observer on **any** move from a port room
  to a room outside every port, read broadly because `home` from Stickney then `c` is exactly the
  skip the rule exists to close. `+ship/leave`'s stamp is IC6's.
- **18a's *Short.* line is `RoomDef.short_desc` on the existing `short_desc` attribute**, not a new
  attribute and not a `+atlas` legend change.
- **The ambience lines ship on `NeighborhoodDef.ambience` / `NeighborhoodRec.ambience`**, read by no
  player surface: a `hook` would print "Going on: Ambience" to every player. The emitter cut reads
  the field, or moves the lines with a pack version bump.
- **18a's *Secret* facts are atlas `secret` entries with `about`**, not tier-secret lore, because
  only `secret` entries are checked by `secrets.leak`.
- **The kept-world boot is proven twice**: the testkit reboot from the exact old list into
  `content_module_with(true, true)` over a store-faithful cumulative fold, and the real split boot
  over an `InMemoryStore` with tacsim switched on over a kept starter world.
- **`options_from_corridor_room` is deleted, not filtered**, and s9_corridor's atlas-journey test is
  replaced (Q-18-6 binding; D-18-49 makes the drawing a projection). Its re-applied-way lens was
  ported to the Valley Road.
- Also taken: the neighborhood `setting` word is `station` (nothing branches on it); 18a's markdown
  is stripped and the words kept verbatim; counters are non-blocking `counter` features 2 m inside
  the wall named; `+ic`'s stanza is `legacy-post-image`, family `tacsim.ic_return`.

**Owner-critical default taken, carried to the owner:**

- **`+ic` lands a Ceres pilot in C03 Port Watch Arrivals** (where the Limbo door lands; Stickney
  pilots land in P01, a character with no stamp in C03). Doc 18 §4.5 says "`+ic` moves the character
  to that port's berth gallery" (C01). The lead ruled arrivals; it is one constant in
  `ic_return::PORTS` and C01 is two doors from C03. **Owner's call.**

**Stopped and reported, not improvised** (an owner or lead decision; a station pack v2 either way):

- **Five 18a details did not ship**: C09 `hull-registry board`, C14 `sailing-plan forms`, P05
  `sailing-plan pad`, P11 `yard-time board`, P11 `insurer's mark`. `apply.rs` `check_details` admits
  only ASCII letters, digits, `_` and spaces in a detail name. C09's desc and Mbatha's background
  still name the board, so `look hull-registry board` finds nothing. P11's insurer's-mark text still
  ships as the second `lore.tycho` fact. Remedies: fold the names (hyphen to space, drop the
  apostrophe) or widen `check_details` and `look`'s keyword fold to accept `-` and `'`.
- **"(LookRendered)" ships inside P11's player-facing `fittings card` detail**, verbatim from 18a
  line 1208 (the owner's text). Amend 18a and the pack together.
- **Fallback lines of NPCs enabled now name verbs of later cuts**: Oduya's
  `+coop`/`+coop/enrol`/`+coop/statement` (IC6), Halvorsen's `+credit` (IC3), Akintola's
  `+ship/order` and Ilunga-Brandt's and Adeyinka-Rusk's `+ship` (IC6). Verbatim and enabled are both
  bound by the brief; the gap stands until IC3/IC6.

**Deferred to named cuts:**

- **IC3**: `ic_return::PORTS` is the stand-in for the `port` entries (`area`, `rooms.arrivals`,
  `gallery`); IC3 replaces it and checks the gallery with `has_marker(PORT_DOCK_MARKER)`.
- **IC4**: boarding a hull from a berth gallery is a move out of a port room and stamps the boarder;
  `+ic` and Limbo's `c` typed by a hull occupant rely on the sim's `PlacementGate` veto (`+ic` keeps
  `check_move_attempt` for that reason); IC4's boundary test should add both to its refused list.
- **IC6**: the `+ship/leave` stamp; and the stale-stamp edge — arriving in a port never restamps, so
  a Stickney-stamped character standing in Ceres reads the `+ic` hint at Limbo's global `c` and
  "already in the corridor" from `+ic` (today only a staff `@teleport` reaches that state; `o`
  repairs it). Either the door passes when standing in a foot-route port or arrival restamps.
- **The emitter cut**: the ambience lines sit on `NeighborhoodRec.ambience`, twelve per station,
  displayed nowhere.

**Known edges left as they are:**

- The validator's `budget.text` pass does not cover `short_desc` or the ambience bounds; only the
  applier refuses them (a shelved pack over them plans clean, then refuses at apply). The shipped
  packs are within bounds and pinned.
- The `topology.unreachable` message and hint do not name `port.dock`.
- `IcDoorHook` and `IcReturnStamp` are registered unconditionally like their neighbours, while `+ic`
  refuses when `[tacsim].enabled` is off.
- Relocations that emit no `Moved` skip the stamp: mush-script `move()` (a builder with a room of
  their own) and `World::destroy`'s dispersal of an occupied room; both need builder or staff
  rights.
- `NeighborhoodRec` is `deny_unknown_fields` and both station records serialize `ambience`: **a
  binary rollback past IC1 over a store that booted it must restore the snapshot or retire the
  station packs first** (deploy runbook; no pre-launch code change).
- Limbo's exits are global, so `c` typed in any room without a local `c` exit now goes to Ceres (the
  door hook and the sim's `PlacementGate` still apply); Limbo's exit list and completion gain `Ceres
  Freeport <C>`. `c` typed inside Ceres is a station-wide shortcut to C03.
- The fold does not model the loader's boot-manifest strip or `skip_key_prefixes`.
- Cosmetic: a real newline inside a string literal at `s9_corridor.rs:242-245` and
  `mush-atlas/tests/travel.rs:973-974` (the Bash backslash collapse).
- Not tested: `+ic`'s three non-port refusals and its veto branch, the stamp on a staff `@teleport`,
  the door's no-staff-exemption, the applier's duplicate-marker refusal, `MAX_SHORT_DESC_BYTES`.

### Pins re-blessed

- **New digest pins**, version 1, in `crates/tacsim/tests/stations.rs` `PINNED` and in each pack's
  own `digest` field: `region.ceres_freeport`
  `80b18bc047d703b5c6f2899155d4f914b68a190ab73a0e5695ffbaab6a4f83f0`, `neighborhood.ceres_freeport`
  `383e0b135c1723100853df6b2cbe9bd3ee98eb6d752c0d00a825a365301dc1cf`, `region.stickney_yards`
  `07e25b41e8e66f29e6d26773472f3783216f3c8d8967f5f2c091075047eeb2b1`, `neighborhood.stickney_yards`
  `6a2cb6004dc2c7af9aeba3f3f27b6c1619fc03ed1d83feb02270a30a30def90b`. Reason: the four packs are new
  content; `ic1_the_declared_digest_is_the_content_digest` pins that each field is derived, never
  hand-edited.
- **`legacy-handlers.toml`**: one new non-baseline stanza, `+ic` (`legacy-post-image`, family
  `tacsim.ic_return`), between `+home` and `+intel`. `PINNED_BASELINE` stays 97. Reason: every new
  player verb gets a stanza; the verb writes no document.
- **`tacsim/tests/s9_corridor.rs`**:
  `a_journey_under_way_ends_where_it_stands_when_its_corridor_is_reapplied` replaced by
  `no_player_verb_traverses_a_spacecraft_connection`. Reason: the old test pinned atlas `+travel`
  flying the corridor, the path Q-18-6 (binding) removes; its revision lens is re-pinned on an
  ordinary road in `mush-atlas/tests/travel.rs` `s10_…`.
- **`mush-contract-tests/tests/exit_command_words.rs`** widened to walk `station_packs()`; the
  `DECIDED` table is unchanged (two `board` rows) because no station alias overlaps a module verb
  and none is a kernel or host word.
- **Unmoved, as the brief requires**: `mush-atlas/tests/starter.rs` `PINNED` (the three starter
  packs; the new fields have serde defaults and the record field skips when empty),
  `tacsim/tests/content.rs` `DIGESTS`, the panic budgets (the new paths are `Option`/`Result`
  throughout), `state-slot-owners.toml`, `module-ns-owners.toml`, the composition-edge pin, the
  restriction-lints census, `help-commands-allowlist.toml`, the CI gated-test inventory. No
  production-world census, Limbo-exit or completion assertion in tacsim, mushd, mush-bot or
  mush-server needed the stations' additions.

### How it was verified

On Windows at the working tree over 0ce0d0fa, the gate runner's run (logs under the run's
`scratchpad/cuts/IC1/gate1/`), every command first time, no reruns:

- `cargo fmt --all -- --check`: exit 0, no output.
- `cargo clippy --workspace --all-targets -- -D warnings`: ``Finished `dev` profile [unoptimized +
  debuginfo] target(s) in 0.72s``, exit 0, no warnings (fingerprint cache; the fixer's own run
  before it finished in 40.37s clean).
- `cargo test -p mush-atlas`: 16 binaries, 163 passed, 0 failed (`travel`: `test result: ok. 21
  passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s`; `markers`: `ok. 4
  passed`; `validate`: `ok. 13 passed`).
- `cargo test -p mush-testkit`: lib `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 0.00s`; every other target ok; 1 doc-test ignored as before.
- `cargo test -p tacsim --test stations`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 1.41s`.
- `cargo test -p tacsim --test s9_corridor`: `test result: ok. 6 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 0.19s`.
- `cargo test -p tacsim --test content`: `test result: ok. 23 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 0.69s` (`DIGESTS` unmoved).
- `cargo test -p tacsim --no-fail-fast`: 34 binaries, every one ok, 0 failed (lib 849, cockpit 180,
  match_lifecycle 135, minds 89, extensions 88, reflex 75, perception 67, arena 51, deliberation 43,
  motor 42, resolution 42, e7c1 38, arena_task 33, …; 12 ignored as before).
- `cargo test -p mush-contract-tests` (twice): 29 binaries, all ok, 0 failed — `exit_command_words`
  `ok. 2 passed`, `help_commands` `ok. 4 passed`, `legacy_handler_allowlist` `ok. 4 passed`,
  `panic_budget` `ok. 7 passed`, `state_slot_owners` `ok. 4 passed`, `restriction_lints` `ok. 11
  passed`, `module_ns_owners` `ok. 3 passed`.
- `cargo test -p mush-server --test boot_parity`: `test result: ok. 18 passed; 0 failed; 0 ignored;
  0 measured; 0 filtered out; finished in 2.04s`.
- `cargo test -p mushd --test front_door --test typeahead --test arena_story --test arena_custody`:
  12 passed, 0 failed (`arena_story` 56.11s).
- `cargo test -p mushd --test observer_rule --test sim_cockpit --test sim_lobby --test
  sim_net --test sim_range --test sim_reflex --test sim_rematch --test sim_solo --test
  sim_units --test sim_weapons`: 81 passed, 0 failed (the load-sensitive paint-timing suites held
  first time).
- `cargo test --workspace --exclude mush-bot --no-fail-fast`: exit 0; 380 `test result:` lines (335
  binaries + 44 doc-test runs), aggregate 7150 passed, 0 failed, 92 ignored; no `FAILED` anywhere;
  62 minutes, about 45 of them a full recompile under unified features. The one panic in the log is
  `mush-server/src/process_fault.rs:197`'s fault-injection test, whose binary reported ok.
- `cargo test -p mush-bot` (on its own line, twice): 21 binaries, all ok, 0 failed (lib 469,
  `arena_lane` 87, `e2e` 49, `arena_contract` 16, `arena_e2e` 9 in ~200s, `playtesting` 9 in ~125s;
  5 ignored as before).
- Signing: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test stations
  ic1_the_station_packs_load_and_their_digests_are_pinned`: `ok. 1 passed`, then the pin run green.
- `git status` after the sweep matched the starting set exactly (18 modified, 7 untracked): no test
  rewrote content or digests.

Flakes: none; no LNK1104, no hung binary, no rerun. A log-growth watcher flagged a 20-minute "stall"
in the workspace sweep that was the compile phase writing no lines (58 rustc/link processes live);
not a test hang.

Lenses: a correctness review (one should-fix, the fold's `Del`; seven notes), a conformance review
(the four packs compared to 18a word for word by script — all 25 rooms, every exit, box, wall,
counter, item, NPC field, lore fact, secret and ambience line match apart from the five stopped
details and the declared markdown stripping; a 47-item checklist, no must-fix), and a tests review
(four should-fixes on vacuity and lost coverage). The fixer took all five, re-ran fmt, clippy,
mush-atlas, mush-testkit and `stations`, and confirmed the ported revision lens with a mutation (the
guard forced true → `FAILED. 0 passed; 1 failed`, then restored). The judge re-read the whole diff
and every new file against the brief and doc 18 §4.5–4.8/§5/D-18-3/19/35/49 and verified each
disposition in the code rather than from the reports.

### Surface for the next cuts

**mush-atlas (`mush_atlas::def`)**

- `RoomDef.markers: Vec<String>` (serde default) → object markers on the room, exactly the pack's;
  `pub const ROOM_MARKERS: [&str; 1] = ["port.dock"]`; `pub const PORT_DOCK_MARKER: &str =
  "port.dock"`; `pub fn check_room_markers(markers: &[String]) -> Result<(), String>` (shared by
  applier and validator). IC3's `PortApplier` checks a gallery with `w.has_marker(room,
  PORT_DOCK_MARKER)`.
- `RoomDef.short_desc: String` (serde default) → room attribute `short_desc` (`pub const
  SHORT_DESC_ATTR: &str = "short_desc"`, `pub const MAX_SHORT_DESC_BYTES: usize = 256`), removed
  when blank; read by the NPC module's turn and scene views.
- `NeighborhoodDef.ambience: Vec<String>` / `NeighborhoodRec.ambience: Vec<String>`
  (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`), `pub const MAX_AMBIENCE_LINES: usize
  = 24`, `pub const MAX_AMBIENCE_LINE_BYTES: usize = 512`; `NeighborhoodRec::validate()` checks
  them; `NeighborhoodSummary` unchanged. The emitter cut reads
  `AtlasState.records.neighborhoods[key].ambience`.
- Validator error code `schema.marker`; `port.dock` rooms of the pack are `Node::Own` topology
  seeds.
- `mush_atlas::travel`: `pub const SELF_MODES: [&str; 3] = ["foot", "car", "transit"]`; `pub fn
  options_from(st: &AtlasState, who: ObjId, room: ObjId, staff: bool) -> Vec<TravelOption>` (the `w:
  &World` parameter is gone; a room in no neighborhood yields no option).
  `options_from_corridor_room` no longer exists.

**tacsim (`tacsim::definitions::content`)**

- `CERES_REGION_PACK = "region.ceres_freeport"`, `CERES_PACK = "neighborhood.ceres_freeport"`,
  `STICKNEY_REGION_PACK = "region.stickney_yards"`, `STICKNEY_PACK = "neighborhood.stickney_yards"`;
  `CERES_REGION_JSON`, `CERES_JSON`, `STICKNEY_REGION_JSON`, `STICKNEY_JSON`; `pub const
  STATION_SOURCES: [(&str, &str); 4]` in that order; `pub const CERES_ROOMS: usize = 13`; `pub const
  STICKNEY_ROOMS: usize = 12`; `pub fn station_packs() -> Result<Vec<mush_content::Pack>, String>`.
  `boot_packs()` is still exactly the proving ground's two.
- `tacsim::composition::boot_pack_list(starter_area: bool, game_content: bool) ->
  Result<Vec<mush_content::Pack>, String>`: starter three, then (with `game_content`) `boot_packs()`
  then `station_packs()`; `content_module_with` uses it; `atlas_module` shelves the stations after
  the proving ground.
- Pack entry ids (all version 1): rooms `room.c01_berth_gallery … room.c16_authority`,
  `room.p01_berth_gallery … room.p15_maintenance` as doc 18 §4.7's `port` entry names them; the
  neighborhood entry is `neighborhood`; `site.freeport_hub`/`structure.freeport_hub`,
  `site.yard_rim`/`structure.yard_rim`, `locality.ceres_freeport`/`locality.stickney_yards`; exits
  `exit.limbo_arrivals`, `exit.c01_c02` … `exit.p07_p15`; items `item.occator_manifold`,
  `item.exchange_manifold`, `item.chandlery_cage`, `item.kasei_manifold`, `item.commissary_locker`,
  `item.yard_spares` (IC7/IC8's vaults and vendors point at these); npc entries `npc.oduya`,
  `npc.halvorsen`, `npc.qureshi`, `npc.sondergaard`, `npc.akintola`, `npc.mbatha`, `npc.petrossian`,
  `npc.wen`, `npc.varga`, `npc.ndiaye`, `npc.osei_ferrante`, `npc.holm`, `npc.kalu` (Ceres) and
  `npc.ilunga_brandt`, `npc.achterberg`, `npc.castillo_haugen`, `npc.adeyinka_rusk`, `npc.okonkwo`,
  `npc.abubakar_lange`, `npc.castellanos`, `npc.tanaka`, `npc.bask` (Stickney); lore
  `lore.port_ceres`, `lore.corridor_trade`, `lore.coop_lay`, `lore.tether`, `lore.tycho` (Ceres),
  `lore.port_stickney`; secrets `secret.slowdown`, `secret.hawk`, `secret.tap`.

**tacsim (`tacsim::ic_return`)**

- `pub struct IcPort { pub id: &'static str, pub name: &'static str, pub area: &'static str, pub
  arrival: &'static str, pub foot_route: bool }` (`Debug, PartialEq, Eq`); `pub const PORTS:
  [IcPort; 2]` (ceres/"Ceres Freeport"/CERES_PACK/"room.c03_arrivals"/true; stickney/"Stickney
  Yards"/STICKNEY_PACK/"room.p01_berth_gallery"/false) — IC3's `port` entries replace this table.
- `pub const DOOR_PACK: &str = CERES_PACK`, `pub const DOOR_ENTRY: &str = "exit.limbo_arrivals"`,
  `pub const LAST_PORT_MARKER: &str = "tacsim.last_port."` (markers `tacsim.last_port.ceres` /
  `tacsim.last_port.stickney`, unregistered, at most one present), `pub const IC_VERB: &str =
  "+ic"`, `IC_HELP`, `ALREADY_IN`, `NOT_A_CHARACTER`, `NO_CORRIDOR`.
- `pub fn port_of_room(w: &World, room: ObjId) -> Option<&'static IcPort>`; `pub fn last_port(w:
  &World, who: ObjId) -> Option<&'static IcPort>`; `pub fn stamp(w: &mut World, who: ObjId, port:
  &IcPort) -> Result<(), WorldError>` (IC6's `+ship/leave` calls this); `pub fn door(w: &World) ->
  Option<ObjId>`; `pub fn arrival_room(w: &World, port: &IcPort) -> Option<ObjId>`; `pub fn
  left_at_line(&IcPort) -> String`, `pub fn no_door_line(&IcPort, &IcPort) -> String`, `pub fn
  return_line(&IcPort) -> String`.
- `pub struct IcReturnStamp` (`WorldObserver` on `Moved`), `pub struct IcDoorHook` (`EventHook` on
  `HookPoint::MoveAttempt`), `pub fn ic_command(ctx: &mut Ctx<'_>) -> Result<(), CmdError>`, `pub fn
  register(reg: &mut Registrar<'_>) -> Result<(), ModError>`.
- `+ic` goes through `ctx.check_move_attempt(actor, None, dest)` before moving: a `MoveAttempt` veto
  (IC4's `PlacementGate`) refuses it with the veto's reason as `Sys`.

**mush-testkit**

- `pub fn fold_module_ops_into(boot: &mut HashMap<SmolStr, ModuleBootData>, ops: Vec<ModuleOp>)`;
  `pub fn fold_module_ops(ops: Vec<ModuleOp>) -> HashMap<SmolStr, ModuleBootData>`;
  `TestWorld::reboot_into(self, modules: Vec<Box<dyn GameModule>>, boot: HashMap<SmolStr,
  ModuleBootData>) -> Result<TestWorld, mush_core::ModError>`.

**Test fixtures later cuts copy**: `crates/tacsim/tests/stations.rs`
`production_roster_with_starter(cfg)` (the production roster ending in `content_module_with(true,
true)`, sheet left out), `obj(tw, pack, entry)`, `latches(tw)`; `crates/mush-atlas/tests/travel.rs`
`modules_shelved(packs, shelf)`.

## IC2

**The Dray, the goods table, the lane and the sky seam** (doc 18 §5, the IC2 row and paragraph),
built 2026-10-15 on main at 07232d09. One commit, pushed to origin/main.

### What landed

1. **The logistics and interior schema** (`crates/tacsim/src/definitions/logistics.rs`, new; `pub
   mod logistics` and the re-exports in `definitions/mod.rs`). `LogisticsDef { dry_mass_t, stowage:
   BTreeMap<CanonicalId, StowageDef>, berths, occupant_allowance_kg, payload_ceiling_kg, drive:
   DriveDef }`, `StowageDef { accepts, limits }` (doc 06 D3's measure map, `MASS_KG = "si.mass_kg"`,
   `VOLUME_L = "si.volume_l"`), `DriveDef { class, law }`, the externally tagged `DriveLaw::Coast {
   kms, burn_permille }` (`Display`: `coast {kms 205, burn_permille 105}`), `InteriorDef { space,
   store, berths_slot, rooms, exits, gauges }`, `InteriorRoomDef` with optional
   `short_desc`/`desc`/`details` (lead ruling R6), `InteriorRoomKind`, `InteriorExitDef`,
   `BerthsSlotDef`; all integer, `deny_unknown_fields`. Every bound is a named constant
   (`MAX_DRY_MASS_T` 1,000,000, `MAX_COMPARTMENTS` 8, `MAX_COMPARTMENT_KEYS` 8, `MAX_BERTHS` 16,
   `MAX_OCCUPANT_ALLOWANCE_KG` 1,000, `MAX_COAST_KMS` 299,792, `MAX_BURN_PERMILLE` 1,000,
   `MIN_INTERIOR_ROOMS` 2, `MAX_INTERIOR_ROOMS` 16, `MAX_INTERIOR_EXITS` 32,
   `MAX_INTERIOR_SHORT_DESC_BYTES` 256, `MAX_INTERIOR_DESC_BYTES` 4,096, `MAX_INTERIOR_DETAILS` 32,
   `MAX_DETAIL_NAME_BYTES` 64, `MAX_DETAIL_TEXT_BYTES` 2,048 — the atlas's `check_details` rules by
   value, no atlas edge). `LogisticsDef::check` refuses by sentence: the bounds, a compartment with
   no tags or no limits, a zero limit, a compartment that does not limit `si.mass_kg` ("every
   compartment says what it weighs"), a coast outside `1..=299792` km/s or a burn outside
   `1..=1000`‰, and a ceiling below `payload_floor_kg()` = Σ mass limits + berths × allowance
   (180,680 for the Dray; checked arithmetic). `InteriorDef::check` refuses duplicate rooms, a
   non-walkable `space`, a non-store `store`, two stores, an exit touching the store ("a store has
   no exits"), an exit naming a non-walkable room or joining a room to itself, not exactly two names
   or an empty display name, two exits on one unordered pair, an alias leaving one room twice
   (`names[0]` leaves `from`, `names[1]` leaves `to`), a walkable room unreachable from `space`,
   `berths_slot.maximum_items != logistics.berths`, a gauge naming no compartment or a non-walkable
   room.
2. **`UnitSpec.logistics` / `UnitSpec.interior`** (`definitions/unit.rs`), optional, `default` +
   `skip_serializing_if`, after `class` and before `note`; `UnitSpec::check` calls `check_logistics`
   (an interior requires logistics; `class: hauler` requires logistics). `bind()` never reads
   either, so no shipped `DefinitionRef` moved (proven by
   `ic2_the_dray_is_a_hauler_and_binds_class_zero`). **`UnitClass::Hauler`** (word `hauler`, nouns
   hauler/haulers) appended to `ALL` (now 4) so `parse` and `read` know it, and `pub const fn
   fieldable(self) -> bool`, false for `Hauler` alone.
3. **One class, one law** (`definitions/mod.rs`): `Catalog.drives: BTreeMap<CanonicalId, DriveLaw>`,
   `Catalog::drive_for_class`, and `drive_table` walked in id order between the cross-reference step
   and minting, refusing "`<second>`: drive class `<c>` is already flown under `<law>` by `<first>`;
   one class, one law" and accepting the same law twice (Q-18-5, D-18-45).
4. **The Dray** (`crates/tacsim/content/units/dray.json`, new): `tacsim.pack.dray` v1
   `rules_definition`, `requires: []`, entry `tacsim.unit.dray` exactly doc 18 §4.2's JSON (class
   `hauler`; 240 t dry; cargo `{si.mass_kg 100000, si.volume_l 120000}` accepting `stow.bulk`;
   propellant `{si.mass_kg 80000}`; provisions `{si.mass_kg 480, si.volume_l 1200}`; 2 berths at 100
   kg; ceiling 181,000; `coast_hauler` / `coast {kms 205, burn_permille 105}`; the seven rooms, five
   exit pairs and three gauges name for name; the 991-byte note) plus 18a §3.1's *Short.*, *Desc.*
   and every *Details.* item transcribed verbatim onto `interior.rooms[]` with the markdown stripped
   and wraps joined, only two detail keys folded under the IC1 carry-forward (`builder's plate` →
   `builders plate`, `tie-points` → `tie points`; `ROM pane` keeps its capitals). Wired in
   `definitions/content.rs` as `DRAY_JSON`, `DRAY_PACK`, `DRAY`, `EXTENSIONS_HAULER` (the tenth
   table, fielded last, in the catalog and never in an arena), appended in `fielded_sources`; the
   module doc's tables extended. Digest
   `dc5cae81692853bca737049009e4fb9d768ea045b22789f6e2bf5c2786078583`, pinned in `tests/content.rs`
   `DRAY_DIGEST` and decoded there through `UnitSpec::check`, so a note over `MAX_NOTE_BYTES` fails
   the cut, never a boot.
5. **The hauler kept out of every arena path.** `match_/lobby.rs` `fieldable_ids` filters on
   `spec.class_of().fieldable()` (`SIM UNIT` never lists `dray`; `sim unit dray` answers "No such
   unit type: dray. SIM UNIT lists them."); `arena/state.rs` `ArenaError::UnfieldableClass { team,
   class }` ("the {team} side fields a {class}, which no arena fields"), returned in `Arena::new`
   for the first unfieldable unit in id order, after the duplicate check and before the class-cap
   loop; `mush-server/src/config_ext.rs` `class_caps` filters `UnitClass::parse(word)` on
   `fieldable()` with the AE.3 sentence unchanged; `tacsim/src/config.rs` `ArenaConfig::validate`
   refuses a non-fieldable key with the same sentence before `between` (reviewer F10).
   `arena/mod.rs` `test_support::hauler`.
6. **The goods table** (`crates/tacsim/content/market/goods.json`,
   `crates/tacsim/src/definitions/goods.rs`, new; re-exported from `definitions` as `Consumable,
   GoodClass, GoodDef, GoodsTable, MeasureDef`). A strict typed document, not a pack (lead ruling
   R12; the `resources.json` precedent), version 1: the `measures` registry (`si.mass_kg` kg "mass",
   `si.volume_l` L "packed volume") and the six rows in doc 18 §4.3's order — metals 1,000 kg/400 L
   ref 200; volatiles 1,000/1,400 ref 100; structural_alloys 1,000/300 ref 400; industrial_parts
   1,000/2,300 ref 400; propellant 1,000/1,000 `stow.propellant` consumable propellant ref 40;
   provisions (no family, class `ship_supply`, unit `crew_day`) 4 kg/10 L `stow.provisions`
   consumable provisions ref 10 — each note stating the role and per-unit figures and never a mix
   (D-18-39). `GoodsTable::load` refuses version 0, the measure and goods bounds (`MAX_MEASURES` 8,
   `MAX_GOODS` 32), duplicates, a registry without `si.mass_kg`, a family-less trade row ("a trade
   good names its family"), a family outside `FAMILIES`, an unregistered row measure, a per-unit of
   0 or over `MAX_PER_UNIT = u64::MAX / MAX_POOL_QUANTITY`, a unit over `MAX_UNIT_BYTES` 16, a
   reference over `MAX_REFERENCE_MAJOR` 1,000,000,000; row errors name `goods.json: <id>`.
   `check_stowage(entry, &LogisticsDef)`: every limit key registered, every accepted tag carried by
   some good, every stowed good carrying every limited measure ("`<good>` is stowed in `<c>`, which
   limits `<m>`, and declares no `<m>`"), and the locker rule (one tag, one good, two or more limits
   agree on the unit count: 480/4 = 1,200/10). `catalog_entries()` derives one lot-represented,
   stackable, legal `mush_market::CatalogEntry` per row through its own wire (ruleset `tacsim`, pack
   `GOODS_PACK = "tacsim.goods"`, the table's version, the label, `baseline` = reference × 100 at
   `CREDIT_SCALE` — display only, never charged, as `GoodDef.reference_major`'s doc says; tags =
   class word plus family). `goods_digest` under `GOODS_DIGEST_DOMAIN = "tacsim.goods.v1"`,
   `goods_table()`; digest `bc28e435bcef2717a4122c4a2b2c9d648a6292d1445bcdfa8f9bbb9271a6d4d9` pinned
   in `tests/content.rs` `GOODS_DIGEST`.
7. **The capacity function** (`crates/tacsim/src/port/mod.rs`, `crates/tacsim/src/port/capacity.rs`,
   new; `pub mod port` in `lib.rs`). Pure, checked `u64` throughout: `Compartment { id, accepts,
   limits }` with `from_stowage`; `Load`; `CapacityError::{UnknownGood, NotAccepted, MissingMeasure,
   Overflow}` (thiserror sentences); `CapacityRefusal { compartment, bound, fits, room }`;
   `Admission::{Admitted, Refused, Error}` (one enum, so no caller can `?` past a refusal); `load`,
   `admits` (qty 0 admitted with the current load; `bound` = every limited measure the after-load
   exceeds, in map order, mass before volume; `room` = limit − load saturating), `max_more` (the
   minimum over limits of `room / per_unit`, naming every measure that attains it; a compartment
   limiting nothing answers `u64::MAX`), `format_m3` (tenths, truncated) and `format_tonnes` (whole,
   truncated). No refusal sentence is rendered here (IC8's).
8. **The sky seam.** `pack.rs`: `BodyDef.epoch_ic: Option<i64>`, `MAX_EPOCH_IC = 1_000_000_000_000`,
   the wire field `default` + `skip_serializing_if`, `Serialize` writing it only when present,
   `from_authored(orbit_au, period_days, phase_deg, epoch_ic)` refusing "a body's epoch_ic is out of
   range". `ephemeris.rs`: `Body.epoch_ic`, carried by `From<&BodyDef>`; `phase_mdeg_at` measures
   elapsed time from `body.epoch_ic.unwrap_or(self.epoch)`; the module and method docs reworded to
   T17's constant sky as amended. `crates/tacsim/src/sky.rs` (new; `pub mod sky`, root re-exports
   `PosKm, Sky, SkyError, epoch_date`): `SKY_DIGEST_DOMAIN = "tacsim.sky.v1"`, `PosKm`,
   `Sky::Circular(Ephemeris)` with `from_pack`, `model`, `config_epoch`, `epoch_of` (a body's
   `epoch_ic`, else the config epoch), `position_km` (z = 0; no production caller, D-18-43),
   `distance_km(a, ta, b, tb)` = the same `separation_km` call `distance_km_at` makes (no second
   rounding path), `revision() -> Result<DefinitionDigest, SkyError>` (lead ruling R9) over model,
   epoch, rate and every body's four numbers; `epoch_date(ic) -> "YYYY-MM-DD"` on
   `mush_core::CivilDate` (lead ruling R3). `store.rs`: `materialize_route` takes `&Sky` and pins
   `sky.distance_km(&a, e, &b, e)` with `e = epoch_of(a)` falling back to the config epoch (every
   fixture bit-identical to the old `pinned_km`; a lane body at its own epoch, the instant printed
   beside the figure, under any rate); `route_in_world` and `reindex_routes` build `Sky::from_pack`;
   the content-latch half of `route_in_world` is now the private `latched_route` (one candidate
   rule: applied, placed, materializes) and the new `pub fn route_source(&World, &RouteDef) ->
   Option<(&RulesPack, &RouteEntry)>` answers the legacy routes-index entry holding exactly `def`
   (the version `resolve` picks, then any other), else the latch candidate only when what it
   materializes equals `def`, else `None` — so `@corridor/show` can never annotate one pack's legs
   with another's sky (correctness F1, fixed and tested with a forged route and with corridor@1
   beside the lane).
9. **The Horizons script** (`crates/tacsim/content/sky/fetch_horizons.py`, new): Python stdlib only,
   prints one JSON object, writes nothing, no test runs it. It fetches daily heliocentric
   ecliptic-J2000 vectors for Ceres (`DES=2000001;`) and Mars (`499`) over 2350-01-01..2352-12-31,
   takes the dates whose true longitudes are within 11°, picks the Ceres–Mars chord minimum under
   the circular model's own formula (ties to the earlier date; lead ruling R4), fetches Pallas
   (`DES=2000002;`) and the osculating periods at that date, places Kirkwood on the straight
   Ceres–Mars segment at exactly 2.06 AU by bisection (placeholder period 1,080 d, Q-18-36),
   computes `epoch_ic` on the game calendar, derives both legs exactly as the Rust does (uau/mdeg
   rounding, `separation_km`, `ceil(km/205)`), prints the laden hours at rate 24 for 142,184 kg and
   180,680 kg and the crew-days, plus the owner's two alternatives (`next_season`,
   `kms_matching_d18_30`), and falls back offline to Kepler propagation from full osculating element
   sets at JD 2579876.5. **Its result** (source `horizons`; the offline run picks the same date and
   fields): 2351-05-13, `epoch_ic` 74,152,540,800, Δλ 5.413°, qualifying season
   2351-04-29..2351-06-22; Ceres r 2.866568 AU λ 281.654° P 1681.861 d; Mars 1.445443 / 276.24 /
   687.034; Pallas 2.482328 / 190.512 / 1686.144; Kirkwood 2.06 / 279.506 / 1080 (placeholder);
   chord 214,534,234 km (1.4341 AU); legs 121,428,001 km (0.8117 AU) → 592,332 s and 93,106,232 km
   (0.6224 AU) → 454,177 s. At world rate 24: 142,184 kg → 1,057,637 + 810,955 s, 21.63 h, 23
   crew-days; 180,680 kg → 1,183,617 + 907,552 s, 24.2 h, 25 crew-days.
10. **The lane pack** (`crates/tacsim/content/lane/ceres_phobos.json`, new):
    `tacsim.lane.ceres_phobos` v1 `campaign_seed`, `requires: []`, ten entries and no `port` (IC3
    adds those in place): `sol` (the fixture's region def verbatim), bodies `ceres`, `mars`,
    `pallas`, `kirkwood` at the figures above with one `epoch_ic`, locations `ceres_freeport`
    ("Ceres Freeport", port, ceres), `kirkwood_depot` ("Kirkwood Depot", depot, kirkwood),
    `stickney_yards` ("Stickney Yards", port, mars), routes `ceres_stickney` (legs → kirkwood_depot
    592,332 s, → stickney_yards 454,177 s) and `stickney_ceres` (454,177 then 592,332), both
    `coast_hauler`, `propellant 0`, no hazard, interdiction or controller. Digest
    `d8074b883e92746e054e45070e62477d661a0535773b77b5ef2b245bdacea678`, pinned in `tests/lane.rs`
    `LANE_DIGEST`. Wired in `definitions/content.rs` as `LANE_PACK` (its doc carries the in-place
    and wipe rule), `LANE_JSON`, `LANE_SOURCES`, `LANE_ROUTE_OUT`/`LANE_ROUTE_BACK`, `lane_packs()`,
    `game_seed_sources()` (named `<built-in …>`).
11. **The game-packs list and the one switch** (lead ruling R2). `lib.rs`: `verify_sources_with(cfg,
    game)` — the game's sources read first so a colliding operator file is the offender the existing
    message names, `MAX_CONFIGURED_PACKS` counting the operator's only; `verify_sources(cfg)` =
    `verify_sources_with(cfg, &[])`; `TacsimRuleset::with_game_content(cfg, bool)` (`with_config` =
    false). `composition/mod.rs`: `tacsim_module_with(cfg, game_content)`; `boot_pack_list` appends
    `lane_packs()` after `station_packs()` (the game's seven boot packs; docs updated);
    `atlas_module` shelves the lane after the stations on the degrade-and-log rule.
    `mush-server/src/boot.rs`: `modules::tacsim_module_with(tacsim, tacsim_enabled)`, so
    `TacsimStore.loaded` and the latch's boot list move together. `mush-server/src/config_ext.rs`:
    the host preflight calls `verify_sources_with(&typed, &game)` with `game_seed_sources()` when
    `typed.enabled` (reviewer F3), so `--check-config`, the host check and the module cannot
    disagree.
12. **`@corridor/show` on the constant sky** (`commands.rs`): the chord line is now `  direct chord
    at the epoch: {km} km ({au} AU) between {a} and {b}; constant sky at epoch {date} (T17).`
    (`au_text`: AU to three decimals, half up in integer arithmetic; the date =
    `epoch_date(sky.epoch_of(a))` from the route's own pack via `route_source`, else the config
    epoch) and a leg between two bodies reads `  leg {n}: -> #{obj} ({secs}s unladen); {from} to
    {to}, {au} AU at the epoch.` with the distance from `Sky::distance_km` at the origin body's
    epoch; any other leg, the header and the refusal are unchanged; "frozen" appears nowhere. On the
    booted game world: `Corridor ceres_stickney: #202 -> #204, 2 legs, drive class coast_hauler,
    propellant 0. Held by unclaimed.` / `  direct chord at the epoch: 214534234 km (1.434 AU)
    between ceres and mars; constant sky at epoch 2351-05-13 (T17).` / `  leg 1: -> #203 (592332s
    unladen); ceres to kirkwood, 0.812 AU at the epoch.` / `  leg 2: -> #204 (454177s unladen);
    kirkwood to mars, 0.622 AU at the epoch.`
13. **Wording, comments only, no value moved**: `mush-sim/src/route.rs` (`RouteDef` "a constant sky
    has no departure windows; the planner computes per departure", `RouteDefWire.distance_km` "the
    direct chord between the endpoint bodies at the epoch, never the flown path … read by nothing in
    the slice", `pinned`, `DistanceLaw` and `route_duration_at` docs), `tacsim/src/config.rs`
    `EphemerisConfig` (the constant sky; `epoch` in IC seconds on the game calendar for a body
    without its own `epoch_ic`; rate 0 holds every body at its authored phase),
    `mush-config/src/config.rs` (the table cell and `EphemerisCfg`), `mushd.toml.example` and
    `deploy/instance/mushd.toml.tmpl` (`[tacsim.time.ephemeris]` comments),
    `tacsim/tests/ephemeris.rs`'s module doc.
14. **Tests** (`crates/tacsim/tests/lane.rs` and `capacity.rs` new; `content.rs`,
    `match_lifecycle.rs`, `stations.rs`, `mush-server/tests/boot_parity.rs`, and unit tests in
    `sky.rs`, `arena/state.rs`, `config.rs`, `config_ext.rs`): `lane.rs` —
    `ic2_the_lane_pack_is_pinned` (re-sign mode, the ten entries, one `epoch_ic` = 2351-05-13, the
    places, the routes), `lane_legs_derive_at_epoch` (both routes walked as body pairs at the
    origin's epoch with `kms` from the fielded drive table, within 100 s; a bodiless
    `kirkwood_depot` refused by name), `sky_circular_agrees_with_the_law_of_cosines` (rate 1000 and
    0, ±1 year, every pair and a mixed-instant pair within 2 km, z = 0),
    `body_epoch_holds_its_phase_at_that_instant` (lane bodies at their own epoch and one period
    later; corridor@1's Mars still advances from the config epoch),
    `fixture_digests_unmoved_by_optional_sky_fields` (F6 branch B: every fixture body re-encodes
    exactly as authored with no `epoch_ic` key, the three declared digests are the e7r1/e7r2 pins
    and equal the computed ones, both corridor packs still rebuild to their pins; `tacsim.core@1`
    does not rebuild at base because `formation` entries re-encode `allowed`/`required_support`,
    unrelated to bodies), `one_drive_class_one_law`,
    `ic2_an_operator_seed_cannot_claim_the_lane_id`, and
    `ic2_the_lane_boots_and_corridor_show_reads_the_constant_sky` (the production roster with
    `tacsim_module_with(cfg, true)`: the lane latched with three rooms; the chord pin equal to the
    sky's at the lane epoch; `route_source` is the lane and `None` for a forged route; both routes'
    `@corridor/show` as a player matched line for line against lines built from the pack, its sky
    and the placed rooms; GOD's `@tacsim/pack list` with the four packs and their pinned digests; a
    second boot over the folded store creating no object and moving no latch; corridor@1 seeded
    afterwards pins exactly `pinned_km` with its own pack as source; a rate-1000 world pins the same
    chord). `capacity.rs` — the five validator tests and the ten capacity tests over the shipped
    table and the Dray's compartments (volume binds on ice and fittings, mass on alloys and
    concentrate; 86 t of ice refused on volume with fits 85; 19 + 81 on volume alone with mass
    exactly at the rating; 101 t of alloys on mass; 20 + 81 on both; two loads that meet both limits
    exactly then nothing more fits; no more ice by volume after 19 + 80; the tank mass-only and the
    locker both at 120; errors are not refusals; the formatters truncate). `content.rs` —
    `ic2_the_dray_is_pinned_and_checks`, `ic2_a_dray_note_past_the_bound_fails_the_load` (1,025
    bytes refused), `ic2_the_dray_is_a_hauler_and_binds_class_zero`,
    `ic2_an_interior_refuses_what_the_template_cannot_build` (six mutations plus the two
    missing-logistics refusals), `ic2_logistics_refuses_a_rail_below_the_stowage_maxima` (180,679
    refused, 180,680 accepted, `kms 0`, a weightless compartment), `ic2_the_goods_table_is_pinned`.
    `match_lifecycle.rs` `ic2_sim_unit_never_offers_a_hauler`; `arena/state.rs`
    `ic2_arena_new_refuses_a_hauler`; `config.rs` `ic2_arena_config_refuses_a_hauler_cap`;
    `config_ext.rs` `ic2_a_hauler_cap_is_refused` and
    `ic2_an_operator_seed_claiming_the_lane_refuses_at_the_host` (through `tacsim_config`, the
    operator file named as the offender; a mutation reverting the preflight fails it);
    `boot_parity.rs` gains `lane_resolves` on the real split boot (`TacsimStore::pack(lane, 1)` and
    `route_in_world(ceres_stickney)` at steps (b) and (c); a mutation reverting `boot.rs` to
    `tacsim_module` fails it at :604); `sky.rs`'s three unit tests.

**Found by review and fixed in the same cut:** `route_source`'s first fallback returned the first
applied candidate without the placed/materializes check or any comparison against the def it was
handed, so `@corridor/show` could print one pack's legs under another pack's bodies and date; the
host preflight's switch and the production `tacsim_module_with` switch were each proven by no test
(reverting either left the suite green); the `@corridor/show` leg lines were matched as two
fragments; and the "frozen"/"Unix seconds" rewording had missed the deploy template, `EphemerisCfg`,
three `route.rs` sites and the ephemeris test's module doc.
### Deferred, and why

**Lead rulings taken by the planner** (recorded here so the owner sees them):

- **R1 — Lane pack shape across cuts.** v1 ships without `port` entries; IC3 adds them to the same
  v1 and re-blesses the pin, keeping version 1, because no world has applied it (pre-launch; dev1
  has never seen it). Consequence, in `LANE_PACK`'s doc and `lane.rs`'s pin: **a persisted world —
  the local all-in-one `mushd` included — that booted the IC2 bytes must be wiped, or have its
  `tacsim:pack:tacsim.lane.ceres_phobos@1` row removed, before it boots IC3**, or `boot_reconcile`'s
  digest gate refuses the boot; **never deploy IC2 without IC3**. If a persisted world must survive,
  IC3 bumps the lane to v2 instead.
- **R2 — The game-packs list is a composition-root switch, not `cfg.enabled`.** Keying it on
  `[tacsim].enabled` would load the lane into every fixture world (`fixture::config()` is enabled)
  and move e7c1's pinned pack counts; it rides the same `game_content` boolean as
  `content_module_with`, with the host preflight (`config_ext.rs`) passing the same game sources on
  `typed.enabled` (reviewer F3), so `--check-config`, the host check and the module cannot disagree.
- **R3 — The calendar: the tree wins over doc 18 §4.8.** `epoch_ic` is
  `mush_core::CivilDate::parse(date).to_ic_seconds()` (365-day years, year 0 at IC 0: 2351-05-13 →
  74,152,540,800), what `time` shows players and what `[game].ic_epoch` parses through; Horizons is
  queried for the same Y-M-D label (Gregorian, 00:00 TDB); `epoch_date` renders through `CivilDate`.
  Doc 18 §4.8's "the IC clock is Unix seconds … 2350-01-01 = 11,991,628,800" paragraph needs
  correcting at the next doc revision (the builder may not edit `docs/design/`).
- **R4 — The date, made deterministic.** Within 2350–2352, among dates with Ceres's and Mars's true
  longitudes within 11°, the one minimizing the Ceres–Mars chord, ties to the earlier; the offline
  fallback uses full osculating element sets (the lead note's a/e/i-only values cannot place a
  body).
- **R5 — The leg seconds: an override of D-18-30 and of the IC2 row's acceptance, pending the
  owner** (carried below).
- **R6 — Interior room text rides `interior.rooms[]`** (optional `short_desc`/`desc`/`details`) so
  IC4 instantiates from the template alone; doc 18's JSON stays valid; two detail keys folded per
  the IC1 carry-forward (`builders plate`, `tie points`).
- **R7 — `DriveLaw` lives in `definitions/logistics.rs`** (the decoder needs it now); doc 18 §4.8
  sketches it in `port/passage.rs`, which IC9 builds against this type.
- **R8 — Ids are `CanonicalId`** for measures, stowage tags, compartments and goods (doc 18 §4.3
  wrote `SmolStr`).
- **R9 — `Sky::revision` returns `Result`** (the canonical encoder is fallible; no panic, no
  sentinel).
- **R10 — Hauler exclusion at four doors with one sentence** (`fieldable_ids`, `Arena::new`,
  `class_caps`, `ArenaConfig::validate`); `UnitClass::read` still knows the word.
- **R11 — `materialize_route` asks `Sky::distance_km` at the origin body's own epoch** (reviewer F4;
  bit-identical for fixtures); the atlas drawing stays an `Ephemeris` projection at the config epoch
  (D-18-49).
- **R12 — `goods.json` is a typed document, not a pack**; canonical digest under `tacsim.goods.v1`;
  the catalog rows carry ruleset `tacsim`, pack `tacsim.goods`, lot, baseline = the origin ask in
  minor units, **display only**; loading them is IC8's.
- **R13–R16** — pins in `tests/content.rs` (Dray, goods) and `tests/lane.rs` (lane); the lane joins
  the boot list (seven game packs) and the atlas shelf after the stations; a hauler declares
  logistics, an interior requires logistics, every compartment limits `si.mass_kg`, the ceiling is
  checked against the stowage maxima plus occupants; location labels "Ceres Freeport", "Kirkwood
  Depot", "Stickney Yards".
- **F6 branch B** (the fixture re-encode property): checked at base first — `tacsim.core@1` does not
  rebuild from its typed entries (`formation` re-encodes `allowed` and `required_support`, defaults
  unrelated to bodies), the two corridor packs do; the test asserts every fixture body re-encodes
  exactly as authored plus the declared-digest pins, and keeps the rebuild assertion for the packs
  that carry bodies.
- **Test rosters (reviewer F8)**: only `stations.rs`'s production roster and `lane.rs`'s boot roster
  switched to `tacsim_module_with(cfg, true)`; every other roster composing `content_module_with(_,
  true)` beside plain `tacsim_module` is left as a **known, harmless divergence** (its latch applies
  the lane's three rooms and two connections while its store does not load the pack, so
  `route_in_world` answers `None`): tacsim `cockpit.rs`, `match_lifecycle.rs`, `reset.rs`,
  `resolution.rs`, `deliberation.rs`, `harness.rs`, `match_/centre.rs`; mushd `sim_*`, `front_door`,
  `observer_rule`, `arena_story`, `typeahead`; `mush-bot/tests/support/arena.rs`. No dbref,
  object-count or room-count assertion moved in any suite the gate ran.
- **The capacity test names 20 + 80 and 55 + 45** (`ic2_a_balanced_load_meets_both_limits_exactly`)
  under the brief's carve-out from D-18-39: it tests that both limits are met exactly and that
  nothing more fits, asserts nothing about optimality, and does not move with prices. The owner may
  hold D-18-39's letter; the remedy is to derive the whole-tonne vertex from `goods.json` and the
  hold by §4.3's integrality rule inside the test.
- Also taken: the refusal sentences (the brief gave none; goods row errors name `goods.json: <id>`);
  `Copy` on `DriveLaw`, `InteriorRoomKind`, `GoodClass`, `Consumable`; `payload_floor_kg` and
  `InteriorDef::room` as public helpers; `GOODS_JSON` in `goods.rs`; the store-exit check runs
  before the walkable check so an exit into the store gets its own sentence; `route_source` prefers
  the legacy entry `resolve` picks, then any legacy entry holding the def, then the latch candidate;
  `Arena::new`'s `UnfieldableClass` after the duplicate check and before the cap.

**Owner-critical defaults taken, carried to the owner:**

- **The lane's leg figures (R5).** The real-date sky at 2351-05-13 and 205 km/s gives legs of about
  0.812 / 0.622 AU and `base_s` 592,332 + 454,177 s, **overriding D-18-30's binding 540,800 +
  409,200 s ("a brimmed hull lands in 22 h") and the IC2 row's own acceptance values (0.741 / 0.561
  AU)** under D-18-42 (every slice figure is the sky's output) and D-18-48 (true positions on a real
  date). At rate 24 a full outbound (142,184 kg) takes about 21.63 h and 23 crew-days (was 19.63 h /
  21), a brimmed hull (180,680 kg) about 24.2 h (was 22 h), **which breaks §4.4's two-hour slack at
  the constant sky**. No qualifying real date from 2349 to 2362 reaches D-18-30's figures;
  2351-05-13 is the nearest. No IC2 test, help text, comment or pack note asserts a duration
  (D-18-50). IC9's `plan_passage_reproduces_the_slice_figures` pins re-derive from the pack (about
  1,057,637 + 810,955 s, 23 crew-days at 142,184 kg today). **The owner's ready alternatives, each a
  one-file swap printed by the script and carried by no pack:** (i) `next_season` — 2354-06-09,
  `epoch_ic` 74,249,481,600, Δλ −0.443°, season 2354-04-10..2354-08-05: Ceres 2.556792 AU / 154.049°
  / 1681.67 d, Mars 1.666119 / 154.492 / 687.007, Pallas 2.694352 / 22.581 / 1684.092, Kirkwood 2.06
  / 154.249 / 1080; chord 133,264,169 km (0.8908 AU); legs 74,328,688 km (0.4969 AU) → 362,579 s and
  58,935,482 km (0.394 AU) → 287,491 s; 142,184 kg → 647,402 + 513,329 s, 13.43 h, 14 crew-days;
  180,680 kg → 724,517 + 574,474 s, 15.03 h, 16 crew-days (the offline fallback gives 2354-06-08,
  approximate); (ii) `kms_matching_d18_30` — the same 2351-05-13 fields at a coast rate of 226 km/s:
  `base_s` 537,293 / 411,975; 142,184 kg → 959,362 + 735,601 s, 19.62 h, 21 crew-days; 180,680 kg →
  1,073,637 + 823,222 s, 21.95 h, 23 crew-days — D-18-30's seconds and crew-days within 1 %, at the
  cost of D-18-29's 205 km/s. **Owner's call.**
- **Kirkwood's orbit (Q-18-36).** Authored at 2.06 AU on the Ceres–Mars chord at the epoch (λ
  279.506°) with a placeholder `period_days` 1,080; inert at rate 0; nothing reads its orbit
  (D-18-48 provisional). The owner rules on it for P-A (the recommended default there puts Kirkwood
  on Ceres's own orbit a few degrees astern, which re-authors the legs in lane v2).
- **Doc 18 §4.8's calendar paragraph** reads IC seconds as Unix/Gregorian; the tree's `CivilDate`
  wins (R3); the paragraph is corrected at the next doc revision.
- **18a interior text that reads as author meta-text, shipped verbatim and flagged for the owner and
  IC12:** Cargo Hold `load cells` — "(the cargo gauge of interior.gauges, a look_rendered line, not
  this text)", "nothing here carries a load", and "LIMIT 100 t / 120 m³"; Central Passage `manifest
  slate` — "the slate reads what +ship/hold reads"; Bridge `sailing plan` — "A paper copy of
  +ship/orders"; Bridge `displays` — "the ETA in IC and real time"; Engineering `tank gauges` —
  "against 80 t"; the Hold Stowage desc's "a hundred and twenty cubic metres … beside the hundred
  tonnes". These are the hull's authored limits, never loads (D-18-39 holds); amend 18a and the pack
  together.

**Stopped and reported, not improvised:** none; every brief item is built.

**Deferred to named cuts:**

- **IC3**: the `port` entries in the same v1 and the pin re-blessed (R1); address lane symbols by
  (`LANE_PACK`, 1), never through a bare `TacsimStore::resolve` — the lane shares `sol`, `ceres`,
  `mars` and `ceres_freeport` with the corridor fixtures, and `resolve` picks across packs by
  version number; a test that each `reference_major` equals the authored origin ask, or retire the
  field once listings exist (carried from reviewer F11; IC3/IC8).
- **IC4 / IC9 (reviewer F9)**: once loaded and latched the lane is resolvable through
  `TacsimRouteCatalog::resolve` (`composition/sim_routes.rs`), so mush-sim's `@journey/plan|depart`
  would fly it at the authored `base_s` under mush-sim's own duration rule — unreachable in IC2 (no
  hull until IC4); **IC9 must make `plan_passage → depart(leg_s)` the only path for a lane route, or
  refuse lane routes on the manual `@journey` arm.** IC9 also bounds or checks the laden
  denominator: `payload_ceiling_kg` and the stowage limits are bounded from below only, so an
  operator hull may declare a ceiling near `u64::MAX` (checked `u128` or refuse).
- **IC8**: the refusal sentences from `CapacityRefusal`'s fields; the LP test deriving both
  vertices; loading `catalog_entries()` through `api::load_pack`.
- **IC12**: the meta-text above; `help travel`'s "its length is the pinned distance" reworded once
  the drawing follows the sky.
- **P-A**: the atlas drawing (`content.rs` `position_gm`) still projects at the config epoch, so
  under a non-zero rate a lane body is drawn at a phase unrelated to the chord and date
  `@corridor/show` prints (harmless at rate 0); `Sky::Kepler`/`Tabulated`; lane v2 re-authoring
  every `base_s` and retiring `lane_legs_derive_at_epoch`.

**Known edges left as they are:**

- **Pre-existing, found by this cut, outside its scope and untested:** content-latch rows written by
  `@tacsim/seed <pack> commit` (for `tacsim.core` and `tacsim.corridor`) did not survive
  `fold_module_ops` + `reboot_into` in the testkit while the boot packs' rows did; the lane boot
  test seeds corridor@1 after its second boot for that reason. Likely cause (correctness lens):
  `seed::apply` persists `content` and `atlas` namespace ops under tacsim's module identity with no
  `persist_delegation` declared. Reproduce at base with a seed → fold → reboot test and file it
  before IC3/IC4 rely on staff-seeded packs surviving a restart.
- Pre-existing, now reachable through the lane: `@corridor/show|rules|domain <id>@<version>` ignores
  the version on the content-latch fallback (`corridor_of` → `route_in_world`, the newest applied
  version), so `ceres_stickney@9` answers the v1 corridor.
- Fixture routes under the default config print `constant sky at epoch 0000-01-01 (T17).` and gain
  an AU suffix on a cross-body leg; e7r1/e7r2 assert only "not refused" there.
- `mush-config` `EphemerisCfg` still says "`rate = 0` is the ruling and the default" where tacsim's
  `EphemerisConfig` and `ephemeris.rs` now say "the default" (D-18-42 rules it for the slice); test
  fn names `a_frozen_sky_holds_its_phases_on_any_day` and
  `with_the_sky_frozen_a_distance_is_identical_across_any_ic_span` keep the word (renaming moves the
  gated-test inventory).
- Cosmetic: `ic2_the_dray_is_a_hauler_and_binds_class_zero`'s doc says "the ten other unit schemas"
  and asserts eleven; `class_caps`' `fieldable` filter is redundant with `ArenaConfig::validate` and
  the host test cannot tell the two doors apart; the catalog-rows test checks the class tag but not
  the family tag or `legality`; the show output's chord line is asserted up to "between " (the body
  pair is pinned on the `RouteDef`); `route_source`'s legacy-index branch has no test;
  `stations.rs`'s "and the lane after them" comment asserts presence, not order.
- The raw Horizons JSON (`horizons.json`, `horizons_offline.json`) lives in the run's scratchpad,
  not in the tree; every figure it yields is recorded above.

### Pins re-blessed

- **New digest pins**, version 1, each in the pack's own `digest` field (the lane and the Dray;
  signed through the tests' `TACSIM_WRITE_CONTENT=1` re-sign mode, never by hand) and in a test
  constant: `tacsim.pack.dray` `dc5cae81692853bca737049009e4fb9d768ea045b22789f6e2bf5c2786078583`
  (`crates/tacsim/tests/content.rs` `DRAY_DIGEST`); the goods table's canonical digest under
  `tacsim.goods.v1` `bc28e435bcef2717a4122c4a2b2c9d648a6292d1445bcdfa8f9bbb9271a6d4d9` (`content.rs`
  `GOODS_DIGEST`; a typed document, so no manifest field); `tacsim.lane.ceres_phobos`
  `d8074b883e92746e054e45070e62477d661a0535773b77b5ef2b245bdacea678` (`crates/tacsim/tests/lane.rs`
  `LANE_DIGEST`). Reason: new content; the Dray pin decodes through `UnitSpec::check` so an
  over-long note or a bad interior fails the cut, never a boot; the lane pin is re-blessed in place
  at IC3 (R1).
- **Moved assertions, each the behaviour the brief changes and nothing else** (old → new):
  `tests/content.rs` `q8_the_fielded_sources_…` `stock.len() + 19` → `+ 20` (the Dray, the twentieth
  extension file); `m3_every_shipped_chassis_binds_its_core` 11 rows → 12 with `(content::DRAY,
  None)` (no core); `e1_the_ranges_the_shipped_machines_fight_at` `catalog.specs.len()` 11 → 12,
  plus a new assertion that the fieldable subset is 11 and the arena loop runs over
  `class_of().fieldable()` specs (the reach table unchanged); `tests/definitions.rs`
  `FIELDED_EXTRAS` `[&str; 10]` → `[&str; 11]` with `content::DRAY`; `tests/extensions.rs` ae1/ae2
  `fielded.specs.len()` 11 → 12, ae3 11 → 12 with a `Hauler` bucket `["dray"]` and "only the drone
  writes a class word" → "only the drone and the Dray"; the four `Body { … }` literals gain
  `epoch_ic: None` (`src/ephemeris.rs` ×3, `tests/ephemeris.rs:58`); `tests/stations.rs`
  `production_roster_with_starter` `tacsim_module(cfg)` → `tacsim_module_with(cfg, true)`,
  `ic1_the_boot_list_and_the_shelf_…` `want.push(LANE_PACK)`, `full.len()` 9 → 10, the `LANE_PACK =>
  requires.is_empty()` arm, the lane absent from `starter_only`, the shelf containing `
  tacsim.lane.ceres_phobos@1 (`; `ic1_a_kept_world_takes_the_stations_at_its_next_boot`
  `after.len()` 9 → 10 plus the lane latch at v1; `mush-server/tests/boot_parity.rs`
  `ic1_a_kept_world_…_switched_on` `after.len()` 9 → 10 plus the lane latch at v1 and the
  `lane_resolves` check on the real boot; the host preflight (`config_ext.rs`) now passing the game
  sources on `typed.enabled`. No assertion was weakened or deleted.
- **Unmoved, as the brief requires**: `tests/content.rs` `DIGESTS` and `PACK_FILES` (`SOURCES`
  untouched); every extension pin in `extensions.rs`; `LINE_TANK_DIGEST` and every shipped
  `DefinitionRef` (asserted equal before and after the Dray); the fixture digests `db270263…`,
  `6ad21275…`, `f89b7310…` (asserted declared = computed, and every fixture body re-encoding
  byte-identically with no `epoch_ic` key); `mush-atlas/tests/starter.rs` and `tests/stations.rs`
  `PINNED`; the pinned `SIM UNIT` list at `match_lifecycle.rs:1563-1569`; the panic budgets (the new
  non-test code has no unwrap/expect/index/panic; `Sky::revision` is a `Result`); the
  restriction-lints census (only `#[allow(clippy::cast_*)]` on f64 conversions, as `ephemeris.rs`
  already does); `state-slot-owners.toml`; `module-ns-owners.toml`; `legacy-handlers.toml` and
  `PINNED_BASELINE` 97 (no new verb; `@corridor` exists); the composition-edge pin (no `Cargo.toml`
  change); the CI gated-test inventory (no ignored or network test; the two `frozen` test fn names
  kept for that reason); `help-commands-allowlist.toml`. No gate file is in the diff;
  `mush-contract-tests` ran green twice.
### How it was verified

On Windows at the working tree over 07232d09 with the cut uncommitted, the gate runner's round 1
(logs under the run's `scratchpad/cuts/IC2/gate1/`, 22 steps), every command first time, no reruns:

- `cargo fmt --all -- --check` (twice): exit 0, no output.
- `cargo clippy --workspace --all-targets -- -D warnings` (twice): ``Finished `dev` profile
  [unoptimized + debuginfo] target(s) in 25.33s`` / `0.62s`, exit 0 (the builder's first run failed
  on one `collapsible_if` in a test; fixed before the gate).
- `cargo test -p tacsim --test lane`: `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 1.57s`.
- `cargo test -p tacsim --test capacity`: `test result: ok. 15 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 0.12s`.
- `cargo test -p tacsim --test content`: `test result: ok. 29 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 0.82s` (`DIGESTS` unmoved).
- `cargo test -p tacsim --test definitions`: `ok. 25 passed`; `--test extensions`: `ok. 88 passed`;
  `--test ephemeris`: `ok. 8 passed`; `--test stations`: `ok. 13 passed; … finished in 1.56s`;
  `--test match_lifecycle`: `ok. 136 passed; … finished in 25.69s`.
- `cargo test -p tacsim --test s9_corridor --test e7c1 --test e7r1 --test e7r2`: `ok. 38 passed` /
  `ok. 21 passed` / `ok. 5 passed` / `ok. 6 passed`, 0 failed.
- `cargo test -p tacsim --lib`: `test result: ok. 854 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 3.61s`.
- `cargo test -p tacsim --no-fail-fast`: 37 binaries, every one ok, 0 failed (lib 854, cockpit 180,
  match_lifecycle 136, minds 89, extensions 88, deliberation 43, resolution 42, e7c1 38, content 29,
  definitions 25, harness 24 with 9 ignored, e7r1 21, reset 17, capacity 15, stations 13, lane 8,
  ephemeris 8, …).
- `cargo test -p mush-server --lib config_ext`: `test result: ok. 13 passed; 0 failed; 0 ignored; 0
  measured; 280 filtered out; finished in 0.05s`.
- `cargo test -p mush-server --test boot_parity`: `test result: ok. 18 passed; 0 failed; 0 ignored;
  0 measured; 0 filtered out; finished in 2.04s`.
- `cargo test -p mush-contract-tests` (twice): 29 binaries, all ok, 0 failed (`restriction_lints`
  `ok. 11 passed`, `panic_budget` `ok. 7 passed`, `ci_gated_test_inventory` `ok. 6 passed; 8
  ignored`, `state_slot_owners` `ok. 4 passed`, `module_ns_owners` `ok. 3 passed`,
  `legacy_handler_allowlist` `ok. 4 passed`).
- `cargo test -p mushd --test sim_units --test sim_lobby --test observer_rule --test front_door`:
  `front_door` `ok. 3 passed` (2.07s), `observer_rule` `ok. 32 passed` (36.95s), `sim_lobby` `ok. 13
  passed` (10.75s), `sim_units` `ok. 8 passed` (9.46s) — the load-sensitive paint-timing suites held
  first time.
- `cargo test --workspace --exclude mush-bot --no-fail-fast`: exit 0; 382 `test result:` lines,
  every one `ok.`; aggregate 7187 passed, 0 failed, 92 ignored; about 60 minutes, about 45 of them
  compiling and linking test binaries under heavy memory pressure (about 130 MB of 64 GB free, 22
  link.exe and 25 rustc live). The one panic message in the log is
  `mush-server/src/process_fault.rs:197`'s armed world-turn fault in `ws01_s8_server_worker`, whose
  binary reported ok.
- `cargo test -p mush-bot` (on its own line): 22 binaries, all ok, 0 failed (lib 469, `arena_lane`
  87, `e2e` 49, `arena_contract` 16, `arena_e2e` 9 in 192.76s, `playtesting` 9 in 127.01s; 5 ignored
  as before).
- Signing: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test content ic2_` printed the Dray and
  goods digests; `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test lane
  ic2_the_lane_pack_is_pinned` → `ok. 1 passed`; then the pin runs green.
- The Horizons script: `python -I crates/tacsim/content/sky/fetch_horizons.py` → exit 0, source
  `horizons` (the figures in "What landed" 9); `--offline` → the same chosen date and fields, next
  season 2354-06-08 marked approximate.
- `git status` after the sweep matched the starting set exactly (26 modified, 10 untracked): no test
  rewrote content or digests.

Flakes: none; no LNK1104, no hung binary, no rerun.

Lenses: a correctness review (one should-fix, `route_source`'s mismatched candidate rule; six notes,
three of them pre-existing gaps the lane now reaches), a conformance review (the Dray compared to
doc 18 §4.2 byte for byte and every room's text to 18a §3.1 by script after markdown stripping, the
lane's fields to the saved Horizons output, a 35-item checklist; one should-fix, the sibling
"frozen"/"Unix seconds" sites; seven notes), and a tests review (three should-fixes on untested
production wiring and fragment matching; seven notes). The fixer took all five, none rebutted, and
mutation-checked the two wiring tests (`boot.rs` reverted to `tacsim_module` → `boot_parity` FAILED
at :604; the preflight reverted to `verify_sources` → the new `config_ext` test FAILED; both files
restored byte for byte), then re-ran fmt, clippy, lane/ephemeris/stations, config_ext, boot_parity,
the tacsim and contract sweeps, mush-sim's lib and the four mushd suites before the gate. The judge
re-read the whole diff and every new file against the brief, doc 18 §5's IC2 row, §4.2–4.4, §4.8, §3
beats D/N/Q/R, D-18-13/28/29/30/39/40/42/43/45/48/50 and Q-18-36, recomputed `epoch_ic`, the chord's
AU, both legs' `base_s` and the `au_text` rounding by hand, and verified each disposition in the
code rather than from the reports.

### Surface for the next cuts

**tacsim definitions (`tacsim::definitions`)**

- `pub mod logistics` (re-exported from `definitions`: `LogisticsDef, StowageDef, DriveDef,
  DriveLaw, InteriorDef, InteriorRoomDef, InteriorRoomKind, InteriorExitDef, BerthsSlotDef`).
  `LogisticsDef { pub dry_mass_t: u32, pub stowage: BTreeMap<CanonicalId, StowageDef>, pub berths:
  u32, pub occupant_allowance_kg: u32, pub payload_ceiling_kg: u64, pub drive: DriveDef }` with `pub
  fn payload_floor_kg(&self) -> Option<u64>` and `pub fn check(&self, entry: &str) -> Result<(),
  DefinitionError>`; `StowageDef { pub accepts: BTreeSet<CanonicalId>, pub limits:
  BTreeMap<CanonicalId, u64> }`; `DriveDef { pub class: CanonicalId, pub law: DriveLaw }`;
  `#[derive(Copy)] pub enum DriveLaw { Coast { kms: u32, burn_permille: u32 } }` (externally tagged
  `{"coast": {...}}`; `Display` = `coast {kms 205, burn_permille 105}`) — IC9's `port/passage.rs`
  builds `HullDrive`/`passage_burn` against this type; `InteriorDef { pub space, pub store, pub
  berths_slot: BerthsSlotDef, pub rooms: Vec<InteriorRoomDef>, pub exits: Vec<InteriorExitDef>, pub
  gauges: BTreeMap<CanonicalId, Vec<CanonicalId>> }` with `pub fn room(&self, id) ->
  Option<&InteriorRoomDef>` and `pub fn check(&self, entry, logistics: &LogisticsDef)`;
  `InteriorRoomDef { pub id, pub name: String, pub kind: InteriorRoomKind, pub short_desc: String,
  pub desc: String, pub details: BTreeMap<String, String> }` (the last three serde-defaulted);
  `#[derive(Copy)] InteriorRoomKind::{Walkable, Store}`; `InteriorExitDef { pub from, pub to, pub
  names: Vec<String> }` (`names[0]` leaves `from`, `names[1]` leaves `to`); `BerthsSlotDef { pub id,
  pub maximum_items: u32 }`. Constants at `tacsim::definitions::logistics::`: `MASS_KG =
  "si.mass_kg"`, `VOLUME_L = "si.volume_l"`, `MAX_DRY_MASS_T`, `MAX_COMPARTMENTS`,
  `MAX_COMPARTMENT_KEYS`, `MAX_BERTHS`, `MAX_OCCUPANT_ALLOWANCE_KG`, `MAX_COAST_KMS`,
  `MAX_BURN_PERMILLE`, `MIN_INTERIOR_ROOMS`, `MAX_INTERIOR_ROOMS`, `MAX_INTERIOR_EXITS`,
  `MAX_INTERIOR_SHORT_DESC_BYTES`, `MAX_INTERIOR_DESC_BYTES`, `MAX_INTERIOR_DETAILS`,
  `MAX_DETAIL_NAME_BYTES`, `MAX_DETAIL_TEXT_BYTES`. IC4 instantiates the hull from
  `UnitSpec.interior` alone (rooms in authored order, `space` = the airlock, `store` = the stowage
  room with no exits, `berths_slot` = the occupancy compartment, `gauges` = compartment id → rooms
  that render its `look_rendered` line in IC6).
- `UnitSpec { …, pub logistics: Option<LogisticsDef>, pub interior: Option<InteriorDef>, pub note }`
  (both after `class`, serde-defaulted, never read by `bind()`). `UnitClass::Hauler` (word `hauler`;
  in `ALL`, now `[UnitClass; 4]`) and `pub const fn fieldable(self) -> bool` (false for `Hauler`
  only).
- `Catalog.drives: BTreeMap<CanonicalId, DriveLaw>` and `pub fn drive_for_class(&self, class:
  &CanonicalId) -> Option<&DriveLaw>`; `Catalog::build_fielded(&[],
  tacsim::NS).drive_for_class("coast_hauler") == Some(&DriveLaw::Coast { kms: 205, burn_permille:
  105 })`. A second fielded unit flying one class under a different law refuses `Catalog::build*`
  with "drive class `<c>` is already flown under `<law>` by `<first>`; one class, one law".
- `pub mod goods` (re-exported from `definitions`: `Consumable, GoodClass, GoodDef, GoodsTable,
  MeasureDef` **only**; `GOODS_JSON`, `GOODS_PACK = "tacsim.goods"`, `GOODS_DIGEST_DOMAIN =
  "tacsim.goods.v1"`, `MAX_MEASURES`, `MAX_GOODS`, `MAX_UNIT_BYTES`, `MAX_PER_UNIT`,
  `MAX_REFERENCE_MAJOR`, `pub fn goods_digest(bytes: &str) -> Result<DefinitionDigest,
  DefinitionError>` and `pub fn goods_table() -> Result<GoodsTable, DefinitionError>` are at
  `tacsim::definitions::goods::`). `GoodsTable { pub version: u32, pub measures: Vec<MeasureDef>,
  pub goods: Vec<GoodDef> }` with `load(name, bytes)`, `good(&id) -> Option<&GoodDef>`,
  `is_measure(&id)`, `check_stowage(entry, &LogisticsDef)`, `catalog_entries() ->
  Result<Vec<mush_market::CatalogEntry>, DefinitionError>` (ruleset `tacsim`, code = the good id,
  pack `tacsim.goods`, `definition_version` = the table's version, `name` = label, `baseline` =
  reference × 100 minor (display only), legal, lot, stackable, tags = class word plus family — IC8
  loads these through `api::load_pack`). `GoodDef { pub id, pub label, pub family:
  Option<CanonicalId>, pub class: GoodClass, pub ledger_unit: SmolStr, pub quote_unit: SmolStr, pub
  measures: BTreeMap<CanonicalId, u64>, pub stowage: CanonicalId, pub consumable:
  Option<Consumable>, pub reference_major: u64, pub note }` with `per_unit(&measure) -> Option<u64>`
  and `mass_kg_per_unit() -> Option<u64>` (the burn/payload accessor). `GoodClass::{Trade,
  ShipSupply}` with `as_str()` (`trade`/`ship_supply`); `Consumable::{Propellant, Provisions}`. The
  shipped rows: `metals` (1,000 kg / 400 L, `stow.bulk`, ref 200), `volatiles` (1,000 / 1,400,
  `stow.bulk`, 100), `structural_alloys` (1,000 / 300, `stow.bulk`, 400), `industrial_parts` (1,000
  / 2,300, `stow.bulk`, 400), `propellant` (1,000 / 1,000, `stow.propellant`, consumable propellant,
  40), `provisions` (4 kg / 10 L per `crew_day`, `stow.provisions`, consumable provisions, no
  family, class `ship_supply`, 10).
- `tacsim::definitions::content`: `DRAY_JSON`, `DRAY_PACK = "tacsim.pack.dray"`, `DRAY =
  "tacsim.unit.dray"`, `EXTENSIONS_HAULER: [(&str, &str); 1]` (the tenth table in
  `fielded_sources`); `LANE_PACK = "tacsim.lane.ceres_phobos"`, `LANE_JSON`, `LANE_SOURCES: [(&str,
  &str); 1]`, `LANE_ROUTE_OUT = "ceres_stickney"`, `LANE_ROUTE_BACK = "stickney_ceres"`, `pub fn
  lane_packs() -> Result<Vec<mush_content::Pack>, String>`, `pub fn game_seed_sources() ->
  Vec<PackSource>` (named `<built-in tacsim/content/lane/ceres_phobos.json>`). The Dray's
  compartments are `cargo` (accepts `stow.bulk`; limits `si.mass_kg` 100,000, `si.volume_l`
  120,000), `propellant` (`stow.propellant`; 80,000 kg), `provisions` (`stow.provisions`; 480 kg,
  1,200 L); rooms `airlock`, `passage`, `bridge`, `crew`, `hold`, `engineering` (walkable) and
  `stowage` (store); gauges cargo → hold/passage/bridge, propellant → engineering/passage/bridge,
  provisions → crew/passage/bridge.

**The capacity function (`tacsim::port::capacity`)** — `Compartment { pub id, pub accepts:
BTreeSet<CanonicalId>, pub limits: BTreeMap<CanonicalId, u64> }` with
`Compartment::from_stowage(&id, &StowageDef)`; `pub type Load = BTreeMap<CanonicalId, u64>`;
`CapacityError::{UnknownGood { good }, NotAccepted { good, compartment }, MissingMeasure { good,
measure }, Overflow}` (thiserror); `CapacityRefusal { pub compartment, pub bound: Vec<CanonicalId>,
pub fits: u64, pub room: Load }`; `Admission::{Admitted(Load), Refused(CapacityRefusal),
Error(CapacityError)}`; `pub fn load(&GoodsTable, &Compartment, contents: &BTreeMap<CanonicalId,
u64>) -> Result<Load, CapacityError>`; `pub fn admits(.., add: (&CanonicalId, u64)) -> Admission`
(`bound` in the limit map's order, mass before volume); `pub fn max_more(.., good) -> Result<(u64,
Vec<CanonicalId>), CapacityError>`; `pub fn format_m3(litres: u64) -> String` (`120_400` →
`"120.4"`, truncated); `pub fn format_tonnes(kg: u64) -> String`. IC7/IC8's `CapacityProvider` maps
endpoint → hull → stowage → `compartment_for` → `admits`; IC8 renders the refusal sentences from
`CapacityRefusal`'s fields; `tacsim::port` is where IC5's `port_tick.rs` and IC9's `passage.rs` go
as separate files.

**The sky (`tacsim::sky`, root re-exports `PosKm, Sky, SkyError, epoch_date`; `SKY_DIGEST_DOMAIN =
"tacsim.sky.v1"` at `tacsim::sky::`)** — `#[derive(Copy)] PosKm { pub x: i64, pub y: i64, pub z: i64
}`; `enum Sky { Circular(Ephemeris) }` with `from_pack(&RulesPack, &EphemerisConfig) -> Sky`,
`model() -> &'static str` (`"circular"`), `config_epoch() -> i64`, `epoch_of(&body) -> Option<i64>`
(`epoch_ic`, else the config epoch), `position_km(&body, t_ic) -> Option<PosKm>` (z = 0),
`distance_km(&a, ta, &b, tb) -> Option<u64>` (the one query; IC9's `plan_passage` calls it per leg
with `t0`/`t1`), `revision() -> Result<DefinitionDigest, SkyError>` (IC9's `PassagePlan.sky_rev`;
IC12's readiness line); `SkyError::Encoding(String)`; `pub fn epoch_date(ic_s: i64) -> String`
(`YYYY-MM-DD` on `mush_core::CivilDate`). `tacsim::pack::BodyDef.epoch_ic: Option<i64>`,
`tacsim::pack::MAX_EPOCH_IC: i64 = 1_000_000_000_000`, `BodyDef::from_authored(orbit_au,
period_days, phase_deg, epoch_ic: Option<i64>)`; `tacsim::ephemeris::Body.epoch_ic: Option<i64>`;
`Ephemeris::phase_mdeg_at` counts from a body's own epoch. The calendar is `mush_core::CivilDate`
(365-day years, year 0 at IC 0): the lane's `epoch_ic` 74,152,540,800 is 2351-05-13.

**The store and the command** — `TacsimStore::route_source(&self, w: &World, def: &RouteDef) ->
Option<(&RulesPack, &RouteEntry)>` (the loaded pack and symbolic route that produced exactly `def`:
the legacy index entry `resolve` picks, then any legacy entry holding the def, then the
content-latch candidate `route_in_world` would materialize, only when it materializes to `def`; else
`None`); `route_in_world` unchanged in behaviour. `materialize_route` pins `RouteDef.distance_km` as
the direct chord at the origin body's own epoch (the config epoch for a body without `epoch_ic`);
IC9 plans over the symbolic `RouteEntry` and the sky, never from `distance_km` or `base_s`.
`@corridor/show` lines as printed on the game world: `Corridor ceres_stickney: #<from> -> #<to>, 2
legs, drive class coast_hauler, propellant 0. Held by unclaimed.` / `  direct chord at the epoch:
214534234 km (1.434 AU) between ceres and mars; constant sky at epoch 2351-05-13 (T17).` / `  leg 1:
-> #<kirkwood_depot> (592332s unladen); ceres to kirkwood, 0.812 AU at the epoch.` / `  leg 2: ->
#<stickney_yards> (454177s unladen); kirkwood to mars, 0.622 AU at the epoch.`

**Wiring** — `tacsim::verify_sources_with(cfg: &TacsimConfig, game: &[PackSource]) ->
Result<Vec<LoadedPack>, String>` (`verify_sources(cfg)` = `verify_sources_with(cfg, &[])`; the
game's sources are read first and not counted against `MAX_CONFIGURED_PACKS`);
`TacsimRuleset::with_game_content(cfg, game_content: bool)` (`with_config(cfg)` = false);
`tacsim::composition::tacsim_module_with(cfg, game_content: bool) -> Box<dyn GameModule>`
(`tacsim_module(cfg)` = false); `boot_pack_list(starter_area, game_content)` = starter three, then
`boot_packs()`, `station_packs()`, `lane_packs()` (the game's seven boot packs); `atlas_module`
shelves the lane after the stations; `mush_server::boot::game_modules` passes `tacsim_enabled` to
both `tacsim_module_with` and `content_module_with`; `mush-server` `config_ext::tacsim_config`
verifies the game sources beside the operator's when `enabled`. The production latch holds
`tacsim.lane.ceres_phobos@1` with three placed rooms (`ceres_freeport`, `kirkwood_depot`,
`stickney_yards`, the corridor's domain hosts IC3 ties to the berth galleries) and two connections;
`TacsimStore.loaded` holds the pack, so `route_in_world(ceres_stickney)` and `(stickney_ceres)`
answer on every enabled production world and `boot_reconcile` writes
`tacsim:pack:tacsim.lane.ceres_phobos@1`.

**Lane pack entry ids (version 1, to be grown in place by IC3)**: region `sol`; bodies `ceres`,
`mars`, `pallas`, `kirkwood` (all `epoch_ic` 74,152,540,800); locations `ceres_freeport` (port,
ceres), `kirkwood_depot` (depot, kirkwood), `stickney_yards` (port, mars); routes `ceres_stickney`,
`stickney_ceres` (`coast_hauler`, `propellant 0`, legs 592,332 / 454,177 s). **IC3 must address
these by (`LANE_PACK`, 1)** — `ceres`, `mars`, `sol` and `ceres_freeport` are also ids in the
corridor fixtures, and `TacsimStore::resolve` picks across packs by version. Digests: Dray
`dc5cae81692853bca737049009e4fb9d768ea045b22789f6e2bf5c2786078583`, goods
`bc28e435bcef2717a4122c4a2b2c9d648a6292d1445bcdfa8f9bbb9271a6d4d9`, lane
`d8074b883e92746e054e45070e62477d661a0535773b77b5ef2b245bdacea678`.

**Arena doors** — `tacsim::arena::state::ArenaError::UnfieldableClass { team: &'static str, class:
&'static str }` ("the {team} side fields a {class}, which no arena fields"), returned by
`Arena::new` before the class-cap check; `tacsim::arena::test_support::hauler(id, team, pose)`
(crate-private); `ArenaConfig::validate` and the host's `class_caps` both refuse `hauler` with
"[tacsim.arena.max_units_by_class] names a class that is not armour, walker or drone: hauler".

**For IC4 and IC9**: once loaded and latched the lane is resolvable through
`TacsimRouteCatalog::resolve` (`composition/sim_routes.rs`), so mush-sim's `@journey/plan|depart`
would fly it at the authored `base_s` under mush-sim's own duration rule; unreachable until IC4
gives a hull. **IC9 must make `plan_passage → depart(leg_s)` the only path for a lane route, or
refuse lane routes on the manual `@journey` arm.** IC9's reproduction pins re-derive from this pack
(about 1,057,637 + 810,955 s and 23 crew-days at 142,184 kg; a brimmed hull about 24.2 h at rate 24)
unless the owner swaps the date or the coast rate. `payload_ceiling_kg` is bounded from below only;
IC9's laden arithmetic must be checked.

**Test fixtures later cuts copy**: `crates/tacsim/tests/lane.rs` `production_roster(cfg)` (the
production roster with `tacsim_module_with(cfg, true)` and `content_module_with(true, true)`),
`lane_config(ephemeris)` (`fixture::config()` plus both corridor fixtures as operator seeds),
`expected_lines(id, placed)`, `latches(tw)`, `live_objects(tw)`, `derived_legs(pack, route, kms)`,
`dray_copy(kms)` (the Dray re-signed as an operator pack through `support::ground::sign`);
`crates/tacsim/tests/capacity.rs` `dray()`, `compartment(id)`, `contents(&[(good, qty)])`,
`edited(|doc| …)` and `row(doc, id)` (the goods document with one row changed);
`crates/tacsim/tests/content.rs` `dray_spec()` and `refusal(&spec)`;
`crates/tacsim/tests/stations.rs` `production_roster_with_starter(cfg)` now on
`tacsim_module_with(cfg, true)`.
