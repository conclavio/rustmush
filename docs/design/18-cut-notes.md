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
