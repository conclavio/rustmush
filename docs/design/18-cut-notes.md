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

## IC3

**Ports, orgs, capital and readiness** (doc 18 §5, the IC3 row and paragraph), built 2026-10-16 on
main at 484647bc. One commit, pushed to origin/main.

### What landed

1. **Lane v2 and the cruise re-fit** (`crates/tacsim/content/lane/ceres_phobos.json`,
   `content/units/dray.json`, `content/sky/fetch_horizons.py`, `src/definitions/content.rs`). The
   lane is republished as `tacsim.lane.ceres_phobos` **v2** (same id, same file): the manifest at
   version 2, v1's ten entries byte-for-byte except the four leg seconds — `ceres_stickney` 537,293
   then 411,975 s, `stickney_ceres` 411,975 then 537,293 s, each `ceil(km / 226)` of IC2's chords
   121,428,001 and 93,106,232 km at 2351-05-13 — plus the two `port` entries of item 3. Digest
   `3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0`, re-signed through
   `tests/lane.rs`'s `TACSIM_WRITE_CONTENT=1` mode. The Dray flies `coast {kms 226, burn_permille
   105}` (lead ruling L0a), nothing else in the file moved, version 1 kept (a fielded extension pack
   with no ledger row), digest `da86f3899788dd9b2ff3141fbf97a5243cb1746b489276173c67dff26caf49fa`;
   the Horizons script's `COAST_KMS = 226` with the ruling in its comment. `pub const LANE_VERSION:
   u32 = 2` beside `LANE_PACK`, whose doc now states the version rule (L0b): a published version is
   never mutated; a world that latched v1 keeps v1's ledger row and takes v2 by `@atlas/pack plan` /
   `apply`, because the boot pass applies only never-latched packs; lane symbols are addressed by
   (`LANE_PACK`, version), never by bare id; only v2's bytes are compiled in. For the record (no
   test asserts a duration, D-18-50): legs 0.812 / 0.622 AU at 2351-05-13; unladen 537,293 + 411,975
   s; a full outbound (142,184 kg) about 959,362 + 735,601 s, 19.62 h, 21 crew-days; a brimmed hull
   (180,680 kg) about 1,073,637 + 823,222 s, 21.95 h, 23 crew-days, at rate 24 on the constant sky.
2. **The `port` kind** (`crates/tacsim/src/port/def.rs`, new; `src/pack.rs`; `src/port/mod.rs`).
   `PortDef { location, venue, area, gallery, rooms: BTreeMap<CanonicalId, String>, treasury, orgs:
   Vec<PortOrgDef>, programme: Option<ProgrammeDef>, vendors: Vec<VendorDef> (schema only;
   `signatory` required, no serde default), routes: BTreeMap<CanonicalId, Vec<CanonicalId>> }` with
   `PortOrgDef { id, name, capital_major, faction }`, `ProgrammeDef` (fifteen fields, D-18-25),
   `VendorDef`/`StockRowDef`/`AskDef`/`BidDef` (`as` → `as_name`), all `deny_unknown_fields`,
   integers only; the constants `PORT_ROOM_ROLES: [&str; 12]`, `MAX_PORT_ORGS` 16, `MAX_PORT_ROUTES`
   8, `MAX_PORT_VENDORS` 16, `MAX_VENDOR_ROWS` 16, `MAX_PORT_CAPITAL_MAJOR` 100,000,000,
   `MAX_TRANCHE_MAJOR` 800,000 (D-18-14), `PORT_MINT_CAP_MAJOR` 1,000,000, `MAX_PROGRAMME_POOL` 16,
   `MAX_REGISTRY_PREFIX_BYTES` 16, `MAX_DORMANCY_REAL_DAYS` 365. `tranches(capital_major)` is greedy
   800,000 chunks, remainder last (3,100,000 → `[800000, 800000, 800000, 700000]`). `room_key(area,
   reference)` is the one split rule (L2): `"<area>." + entry`, refused outside the port's area.
   `PortDef::check(&self) -> Result<(), String>` returns the detail only (orgs 1..=16 and unique,
   labels ≤ 120 bytes, capital 1..=100,000,000, every room reference by `room_key`, every `rooms`
   key a role word, `treasury` and `issuer` canonical `SystemPrincipalId`s, routes non-empty and ≤ 8
   with no repeat, the programme's bounds and `floor_major + pool × stake_major ≤ its org's
   capital_major` in checked arithmetic, the vendors' shape). `check_ports(entries) -> Result<(),
   (CanonicalId, String)>` spans the pack: `location` a `location` entry, venues and locations
   unique, org ids unique across ports, every `routes` key another port's venue and every listed id
   a `route` from this location to that one (D-18-47), every programme `reference` a trade good
   (family, no `consumable`) naming `<port entry>/<org>` of this pack (L8). In `pack.rs`:
   `PackEntry::Port(PortDef)`, label `"port"`, a no-op `check` arm (the id is out of scope there),
   `from_content` running `def.check()` per port entry and `check_ports` after the symbol checks,
   both wrapped as the new `PackError::PortRefused { port, detail }` (`"port entry {port}:
   {detail}"`); `OWNED_KINDS: [&str; 16]`; a port in a `rules_definition` pack is `WrongKindEntry`
   (a port instantiates, D-18-20); `load_pack` split into `pub fn rules_pack_of(Pack) ->
   Result<RulesPack, PackError>` (decode + every rule, no digest compare) and the digest half.
   `@tacsim/pack show` summarizes a port as `port <venue> at <location>, <n> orgs`
   (`src/commands.rs`).
3. **The two port entries** (in the lane): `ceres_port` (location `ceres_freeport`, venue `ceres`,
   area `neighborhood.ceres_freeport`, gallery C01, seven rooms, treasury `tacsim.treasury`, six
   orgs — Ceres Clearing 50,000, Belt Carriers' Cooperative **3,100,000** (L6; D-18-27/D-18-40),
   Co-op First Captain Trust 250,000, Freeport Exchange 4,000,000, Occator Water & Propellant
   50,000, Tycho Mutual Assurance & Salvage 50,000 (`concord`) — the First Captain programme exactly
   as doc 18 §4.7 writes it, `routes: { "stickney": ["ceres_stickney"] }`) and `stickney_port`
   (location `stickney_yards`, venue `stickney`, gallery P01, seven rooms, orgs Kasei Metals
   600,000, Stickney Yards 50,000, Yard Commissary 50,000, all `republic`, `routes: { "ceres":
   ["stickney_ceres"] }`). No `vendors` (IC8's v3). Nine orgs, sixteen tranches, 8,200,000 cr; no
   test types the total.
4. **The port map** (`crates/tacsim/src/port/map.rs`, new; `src/store.rs`). `PORT_MAP_PREFIX =
   "tacsim:port:"`, `PORT_MAP_SCHEMA = 1`, `port_map_key(pack, entry)` →
   `tacsim:port:{pack}/{entry}`; `PortMap { schema, pack, entry, domain: Option<u64>, host:
   Option<u32>, presentation: Option<u32>, treasury: Option<u64>, orgs: BTreeMap<CanonicalId,
   PortOrgRow> }`; `PortOrgRow { org, root_scope, capital_major, account: Option<u64>, capital:
   Vec<TrancheRow> }` — **`capital_major` is the authored capital the row was created for, written
   once with the org** (the reviewers' must-fix); `TrancheRow { n, obligation, amount_minor }`;
   `PortMapState::{Absent, Present(&PortMap), Unreadable}`. `TacsimStore` gains `port_maps`,
   `port_map_keys` (every key claimed, readable or not), `test_world`; `port_map(pack, entry)` (a
   claimed key with no body reads `Unreadable`), `port_maps()`, `test_world()`, crate-private
   `port_map_claims()`; `put_port_map(ctx, &PortMap)` is the single write path — RAM install plus
   one `JsonSet` of the whole document, and if serialization ever failed it pages and still writes
   `{schema, pack, entry}` so the key exists. `store::load` has a `tacsim:port:` branch: the tail
   split on its one `/` (an unsplittable tail is skipped loudly), the key claim recorded, the body
   decoded under the identity rule (`pack`/`entry` must equal the key's; anything else is skipped
   loudly and stays `Unreadable`). The module-doc table gains the row: skipped loudly, **and the key
   still bars re-creation**.
5. **`PortApplier`, parts a–c** (`crates/tacsim/src/port/apply.rs`, new; registered fifth in
   `src/content.rs`'s `register_appliers`, kind `port`, phase 11). `room_by_key(w, &RecordKey)`
   resolves a room by `RoomGeo.key` in `AtlasState.records.roomgeo`, live, a Room, not `GOING` (L1:
   the latch table is `mem::take`n during every apply, so `latched_object` is never used). `check`
   runs `rules_pack_of(pack.clone())` then the preflight, in this order: a non-empty `vendors` list
   refused; the TacsimStore, EconomyStore, OrgStore and SpatialState layers present; the map
   readable (an `Unreadable` map refuses naming the key and logs at `error!`); every room reference
   resolves and the gallery carries `port.dock`; then every drift against a present map —
   presentation, domain (Active, host, presentation), treasury (Active, owned by `System(treasury)`,
   in `tacsim.credit`), per recorded org: still authored, org exists, account exists, every
   obligation exists, name unchanged, and `capital_drift` — then the mint cap against the largest
   tranche still to create, then the frozen economy when any tranche is still to create.
   `capital_drift(authored, row, scale)` answers `Changed { recorded }` on any difference from
   `row.capital_major` (so 4,000,000 → 4,800,000 is refused although its five recorded rows match),
   `Inconsistent { recorded }` when the rows are not distinct indices of `tranches(recorded)` with
   that tranche's amount, else `None` and the unrecorded indices are completed. `put` (create and
   update alike, both `Ok(None)`) re-runs the preflight, then (0) host = `placed[location]` (a live
   Room) with host drift refused, gallery by `room_by_key`, the present map or `PortMap::new`; (a)
   with no recorded domain, a host already carrying one is refused (never adopted) else
   `mush_sim::open_passive_domain(ctx, host, gallery)` recorded and written; (b)
   `crate::economy::ensure_instrument` create-only (L9), then the treasury adopted as the lowest-id
   Active `tacsim.credit` account of `PrincipalId::System(treasury)` or opened in the
   `@account/system` shape (name `"tacsim.treasury"`, `CreditPolicy::unbounded()`, scope `None`),
   recorded and written — two ports naming one treasury share one account; (c) per org in authored
   order: `alloc_org`/`alloc_scope` and one `OrgMutationPlan::of(vec![CreateOrg { org, root }])`
   through `mush_org::submit` with action `org.create` and detail `{port, org, name}` (the
   `mush-faction/src/econ.rs` shape), the row recorded with `capital_major`; the scoped account in
   `open_root_account`'s shape (owner `Organization(org)`, scope the root, name `"<org name>/root"`,
   `CreditPolicy::forbid()`); then per `(n, major)` of `tranches(capital_major)` with no recorded
   row `n`, `create_system_obligation` from the treasury to the org account, amount `major ×
   10^scale` checked, `EconomicSchedule::new(RealUtc, ctx.w.now(), None, 1)` (one-shot),
   `GracePolicy::default()`, `cause: Ruleset { namespace: "tacsim.port" }`, the row written after
   each. The applier submits nothing authoritative and moves no money; the capital settles on the
   next sweep. `retire_check` is `Ok(())`; `retire` logs at `info!` and writes nothing (the map is
   kept).
6. **Passive opens skip the window guard** (`crates/mush-sim/src/window.rs`). The
   `windows(ctx.w).count() >= MAX_WINDOWS` refusal is deleted from `open_passive_domain` (the
   `domain_of_host` refusal stays; `open_window` keeps its guard); the summary and `# Errors` doc
   now say a passive domain holds no window and no active-domain slot, so `MAX_WINDOWS` does not
   bound it (doc 18 §4.4, §9 risk 1). Proven in mush-sim's own rig (`tests/custody.rs`
   `ic3_a_passive_domain_opens_with_every_window_open`: eight window records installed into
   `SpatialState`, `@domain/create … =sim.passive` succeeds, the count stays 8) and in
   `tests/ports.rs` test 6.
7. **The composition** (`crates/tacsim/src/lib.rs`, `src/composition/mod.rs`,
   `deploy/instance/mushd.toml.tmpl`, `mushd.toml.example`). `TacsimRuleset.test_world` (false in
   every constructor) with `#[must_use] with_test_world(self, bool)`, handed to the slot in
   `register` **and** to the `loaded` store in `on_world_loaded` before `*slot = loaded`;
   `tacsim_test_world_module(cfg, game_content)` for in-process test engines only (L15;
   `game_modules` never calls it). `server_modules_with` now takes `mut economy` and contributes
   `instruments["tacsim.credit"] = InstrumentCaps { mint_obligation_cap_major: 1_000_000 }` with
   `or_insert` — an operator's per-instrument value stands, the global
   `[economy].mint_obligation_cap_major` no longer applies to the credit once contributed, and the
   cap is per instrument, not per faucet (the doc comment says all three). The template gains the
   `[economy.instruments."tacsim.credit"]` stanza verbatim after `[economy]`; the example declares
   the same table live (deviation 1 below).
8. **`+credit`** (`crates/tacsim/src/port/credit.rs`, new; registered in `src/commands.rs` inside
   its own `classify_commands(ReadOnly)` … `classify_commands(LegacyPostImage)` bracket,
   `ArgShape::None`, `CREDIT_HELP` verbatim). TacSim off → `TacSim is not enabled on this server.`;
   the actor's own account (Active, `PrincipalId::Object(actor)`, `tacsim.credit`, unscoped, lowest
   id) absent → `No credit account is open in your name.`; else `Credit: <posted> posted, <held>
   held, <available> available.` through `mush_economy::money` (`25000.00cr`);
   `Account::available()` `None` → `Your account's balance cannot be read right now.` and an
   `error!` naming the account. `credit_line` and `own_account` are public. No argument form (L12).
9. **The readiness report and `@tacsim/port status`** (`crates/tacsim/src/port/readiness.rs`, new;
   `src/commands.rs`). `Binding::{Lane, Sky, Rate, Treasury, Map, Rooms, Domain, Org, Programme}`,
   `LineState::{Ready, Pending, Missing, Flagged}` with `word()`, `ReadinessLine { port, binding,
   state, text }`, `Readiness { lines }` with `is_ready`, `not_ready`, `summary`, `render`;
   `PLAY_RATE = 24`; `readiness(w)` is a pure read. The report's lane pack is the latched version
   when loaded, else the shipped `LANE_VERSION`, else none (an exact `(id, version)` lookup, never
   `resolve`); port entries come from it plus any map whose entry it does not author (lines from the
   map alone, capital from `row.capital_major`). Lines in order with the brief's exact texts: the
   lane line (Ready at `LANE_VERSION`; Flagged `… Take it with @atlas/pack plan …` when older;
   Flagged `… and does not load @{v}` when newer and not loaded; Missing when not latched); the sky
   line `sky: constant (rate 0), epoch 2351-05-13, rev <12 hex>` (epoch = the first port's location
   body; Flagged `rev unavailable` on a revision error; Missing `sky: no lane pack is loaded on this
   server.`); the rate line (Ready at 24, Ready `(test world)` elsewhere in a test world, Flagged
   `…; play runs at 24.` otherwise, Missing with no time generation); one treasury line per distinct
   treasury named; per port the map line (`{entry}: its port map {key} is unreadable.` / `{entry}:
   not applied.`), the rooms line (`{n} of {m} resolved` over distinct references), the domain line,
   one line per org (L14: Ready when every tranche is Satisfied; Pending only while every unsettled
   tranche is Active or in Grace; Missing naming the org not created, the org missing, the account
   not open, an obligation gone, `{r} of {t} capital tranches are created`, or a tranche that is
   `cancelled|delinquent|suspended, and no sweep will settle it`), and the programme line; last
   `Ready: every binding above is in place.` or `Not ready: {k} of {n} lines are pending, missing or
   flagged.`. `@tacsim`'s switches gain `port`; `port_cmd` is `require_enabled` then
   `require_staff`, `status` or empty renders (`Port readiness:`, each `"{text} [{word}]"`, port
   lines indented two spaces, the summary), anything else `Usage: @tacsim/port status`; the help
   line appended.
10. **`ic_return::PORTS` kept** (`crates/tacsim/src/ic_return.rs`; L10): both doc lines now say IC3
    kept the table and pins its agreement with the port entries — venue, name, area, arrival — by
    test 12.
11. **Gates and stanzas** (`crates/mush-contract-tests/legacy-handlers.toml`,
    `state-slot-owners.toml`): the `+credit` stanza in byte order, `read-only`, with the brief's
    note; the `@tacsim` note refreshed (not joined) with the fifth switch; `PINNED_BASELINE` 97
    unchanged. Two `state-slot-owners` rows for `crates/tacsim/src/port/apply.rs` (`OrgStore` →
    `org`, `EconomyStore` → `economy`: the allocator bump only; the record is created through the
    owner's own op).
12. **Tests** (`crates/tacsim/tests/ports.rs`, new, 17 tests; unit tests in `def.rs`, `map.rs`,
    `apply.rs`, `credit.rs`, `readiness.rs`; `mush-sim/tests/custody.rs`;
    `mush-server/tests/boot_parity.rs`). The production roster composes `server_modules_with` +
    worldgen + npc + `sheet_modules` + `tacsim_module_with(cfg, true)` + `content_module_with(true,
    true)` exactly as `mush_server::boot::game_modules` does; one sheet-less roster for L9; a
    test-world roster; the shelf route for v3 replaces the atlas module at its index. Every
    `boot`/`reboot` asserts `refused_namespace_writes` is empty. The twelve brief tests plus:
    `ic3_the_applier_ensures_the_credit_without_the_sheet`,
    `ic3_an_update_creates_a_new_org_and_keeps_every_recorded_row` (a tenth test-only org on
    Stickney: one org, one account, two obligations, every existing row byte-identical, only the
    Stickney map written), `ic3_a_map_that_stopped_part_way_is_completed` (the Exchange's fifth
    tranche row and obligation document removed before any sweep; the report reads `4 of 5 capital
    tranches are created. [missing]`; the next apply makes exactly one obligation, n=4, 80,000,000
    minor; the balance settles to 400,000,000 once),
    `ic3_a_frozen_economy_refuses_the_lane_before_anything_is_made`,
    `ic3_a_cancelled_tranche_reads_missing_not_pending`. Test 4 carries three drift variants (coop
    3,200,000; Kasei renamed; freeport_exchange 4,800,000 — the prefix-extending top-up). Test 8
    compares the whole 18-line report line for line. `boot_parity` asserts both port maps, nine orgs
    and `test_world() == false` on the real split boot. The mushd `@domain` assertions in
    `arena_story.rs` and `observer_rule.rs` read the whole answer (`ask_whole`) and assert both port
    domain lines present and the muster absent.

### Deferred, and why

**Lead rulings taken by the planner** (recorded here so the owner sees them):

- **L0a — The cruise is 226 km/s** (D-18-45 makes the coast rate drive data): at 205 the real-date
  legs broke R3's "under a day" for a brimmed hull (24.2 h); at 226 every doc 18 §4.4 figure holds
  within 1 %. Overrides D-18-29's 205; **owner-critical**, below.
- **L0b — Lane v2, never v1 in place**: overrides IC2's R1. A published version is never mutated
  (`boot_reconcile`), and the boot pass never upgrades a latched pack
  (`mush-content/src/module.rs:143-150` — the tree wins over §4.7's "applies at ANY boot", true only
  of never-latched ids). A v1-latched world keeps v1's ledger row and takes v2 by the shelf
  (`@atlas/pack plan` / `apply`; ports create, routes update, places and bodies unchanged — test 7).
- **L1 — Rooms resolve by `RoomGeo.key`, never the latch** (the tree wins over doc 18 §4.5/§4.7 and
  18a §0.2's "through the latch"): `with_state` empties the latch slot during every apply.
- **L2 — The reference split rule**: every `<pack>.<entry>` in a port entry is `"<area>." + entry`;
  a reference outside the port's area is refused.
- **L3 — The map is one typed whole document per port entry**, version-agnostic, persisted after
  every act in the apply's one batch (the `ArenaRoster` pattern); the lead note's row names
  `content.capital/<pack>/<entry>/<org>` are the fields `orgs[org].capital[n]` of this document.
- **L4 — Update never re-capitalizes, renames or re-hosts**; a new org in a later version is created
  and capitalized; a changed `faction` is accepted and writes nothing; a map row whose org, account
  or obligation is gone is refused naming the org and the key, never re-created. **The rename half
  is a tree-wins deviation from §4.7**'s "Update rewrites … names … in place": `mush-org` has no
  rename op (`plan.rs`, `CreateOrg` only); renames are P5. Owner-critical, below.
- **L5 — `PortApplier` lives in `port/apply.rs`**, registered from `content.rs` at phase 11 (doc 18
  named `content.rs`).
- **L6 — The Co-op's capital is 3,100,000 in v2**, not §4.7's sample 50,000 with a bump at IC8: a
  later version can never re-capitalize (L4; D-18-27, D-18-40).
- **L7 — The `vendors` schema lands now, empty in v2**; the applier refuses a non-empty list until
  IC8, which owns the semantics and may amend the schema.
- **L8 — `reference` is checked for shape now**; "that org has an ask for the good" is IC8's.
- **L9 — The applier ensures `tacsim.credit` create-only** before opening accounts (several rosters
  compose no sheet module).
- **L10 — `ic_return::PORTS` stays**, pinned to the entries by test 12.
- **L11 — `@org/list` is unchanged**; "shows the orgs and capital" reads as `@org/list` shows the
  orgs, `@tacsim/port status` their capital. Owner-critical, below.
- **L12 — `+credit` takes no argument** (staff read others through `@account/show`).
- **L13 — The mint cap is contributed by the composition and written into the template and
  example.** Owner-critical, below.
- **L14 — One readiness line per org** (the worst state of org, account, capital); the report is
  typed so IC6 gates on states, never text.
- **L15 — `test_world` is a `TacsimRuleset` builder flag**, set only by `tacsim_test_world_module`;
  it relaxes only the rate line.
- **L16 — The `faction` word is carried, validated as a canonical id and read by nothing** until the
  faction cut; the Stickney three are `republic`.

**Owner-critical defaults taken, carried to the owner:**

- **The 226 km/s re-fit (L0a)** overrides D-18-29's binding 205 km/s. The alternative is IC2's
  cut-note option (i): the 2354-06-09 season at 205 km/s — one data swap (the epochs, `kms` back to
  205) plus a lane v3. A world that booted IC2 keeps lane v1's row and takes whichever ships by the
  shelf.
- **Org renames refused on a port update (L4)**, against §4.7's stated update semantics, because
  `mush-org` has no rename op and adding one is outside the slice.
- **`@org/list` shows the orgs, not their capital (L11)**: re-reads the IC3 row's verify criterion;
  a balance column would give mush-org an economy read it does not have.
- **The contributed `tacsim.credit` cap (L13)** supersedes dev1's rendered global
  `[economy].mint_obligation_cap_major = 50000` for this one instrument, and because the cap is per
  instrument, every `tacsim.credit` faucet obligation — including a staff `@market/vendor … fund
  faucet=tacsim.treasury` drip once the market composes (IC7/IC8) — shares the 1,000,000 cr ceiling.
  The alternative (template-only, no code contribution) would make dev1's next boot refuse the lane
  until the config is edited.

**Builder's and fixer's decisions beyond the brief:**

- The preflight also refuses when the `TacsimStore` slot is absent (the map could not be read and a
  re-apply could mint twice). `put` re-runs the whole preflight so no refusal can follow a partial
  write of the same entry; the drift checks run before the cap and the freeze so a changed capital
  is reported as drift, never as a cap problem.
- **The frozen-economy refusal** (`{id}: the economy is frozen on this world, so {n} capital
  tranche(s) cannot be issued; nothing is applied while it is frozen`) fires only when a tranche
  remains to be created, so a fully applied, write-free re-apply on a frozen world is still allowed.
- **The partial-apply completion path**: only when the recorded `capital_major` equals the authored
  one are the unrecorded tranche indices completed; a mismatch of rows against `tranches(recorded)`
  is `Inconsistent` (a damaged map, never re-issued). There is no production path to a partial apply
  now that both the cap and the freeze refuse in `check`, so the completion test synthesizes the
  state (one map row and its obligation document removed in the folded store).
- `PORT_MAP_SCHEMA` stays 1 although `PortOrgRow` gained the required `capital_major` (pre-launch;
  no port map exists outside test runs; a body without it loads as `Unreadable`, fail-closed).
- Treasury adoption takes the lowest-id Active `System(treasury)` account in `tacsim.credit`, scoped
  or not; the report's treasury line uses the same rule and defaults to `tacsim.treasury` when no
  port entry is loaded. System principal ids must already be canonical (`SystemPrincipalId::new(s)`
  equal to `s`); nothing is case-folded on the wire.
- The sky line's epoch is the first port entry's location body, falling back to the first place on a
  body, then the config epoch. In the rooms line `m` counts distinct references (the office that
  repeats the registry counts once: 8 of 8 for both ports). A map present whose entry the report's
  pack does not author gets a domain line plus org lines from the map alone, capital from
  `row.capital_major`. The disabled-layer `+credit` refusal comes from the handler itself through
  `TacsimStore::enabled`.

**Deviations from the brief:**

1. **`mushd.toml.example` declares `[economy.instruments."tacsim.credit"]` live, not as two
   commented lines**:
   `shipped_configs::the_shipped_example_and_the_deployed_template_declare_the_same_sections`
   requires the example and the template to declare the same section headers, and a commented header
   failed it. The value equals the contributed one.
2. **Test 6 did not open eight windows by verb**: one window was opened with GOD's `@atlas/window
   open #<C07>` and its composite finished; its sim `domain:<id>` document **and** its atlas
   `window:<id>` document were cloned under seven fresh ids on seven other station rooms in the
   folded store, with the sim meta `next_domain_id` bumped (the atlas closes, at boot, every sim
   window it holds no composited entry for).
3. **Tests 9 and 11 run on the sheet-less roster** (test 9 because the onboarding gate refuses an
   unapproved character at the Limbo door; test 11 needs no sheet), as does the L9 test; every other
   `ports.rs` test runs the production roster with the sheet.
4. `@domain/create` needs `@behavior/add #<room>=sim.domain` first (existing, unchanged); test 6 and
   the mush-sim test both do it.
5. Test 10's bare-string `routes` case: the serde decode error names the entry and "expected a map",
   not the field name.

**Left as notes, not fixed (honestly small):** host drift and an already-hosted domain are refused
in `put`, not `check` (`check` is not handed `placed`; a lane retire-then-re-apply latches with an
anomaly rather than refusing clean); the drift preflight checks a recorded org account for existence
only, not Active/owner/scope/instrument; at the authored bounds (~2,000 tranches per entry) the
whole map is rewritten after every tranche in one batch; `PORT_MINT_CAP_MAJOR`'s doc comment
(def.rs:79) reads backwards; test 9's disabled-layer boot skips `no_refused_writes`; `+credit`'s
verb path never has held > 0 and its "never sees an org account" assertion cannot fail; test 11 uses
the bare roster where the brief said production; test 4's drift words are loose (`coop`/`capital`,
`kasei`/`rename`); no test exercises the map identity rule or an unsplittable key tail; test 1's
gallery oracle is `room_by_key`, the function under test (mitigated by the marker and name checks).

**Deferred to named cuts:** the eight Drays, interior instantiation, titling, `berths`,
`Transition::Members` — IC4 (`PortApplier` part f; the hulls readiness line); the action provider,
typed families, the `tacsim.port` tick — IC5; `+coop`, `+coop/enrol`, `+coop/statement`, `+ship`,
the readiness **gate** and the floor check — IC6; market seams — IC7; vendors, vaults, clerks,
grants, stock rows, Wanted docs, the goods catalog load (part d–e), the pools/listings lines, the
"reference org has an ask" check, lane v3 — IC8; `plan_passage`, `choose_route`, `via=` — IC9; help
topics, the full report and its gate, threading `test_world` into a mush-server boot path — IC12;
buyout, dormancy, release — IC13; org renames, top-ups, restock — never in the slice (P5). IC2's
carry-forward that each `reference_major` equals the authored origin ask still waits on listings
(IC8). Deploying anything and `@tacsim/time rate=24` on dev1 are the owner's.

### Pins re-blessed

- **`tacsim.lane.ceres_phobos`**: version 1 → **2** and digest
  `d8074b883e92746e054e45070e62477d661a0535773b77b5ef2b245bdacea678` →
  `3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0`, in the pack's own `digest`
  field and `crates/tacsim/tests/lane.rs` `LANE_DIGEST` (re-signed through the test's
  `TACSIM_WRITE_CONTENT=1` mode, never by hand). Reason: lane v2 — the four leg seconds re-timed for
  226 km/s (L0a) and the two `port` entries (What landed, item 3); a published version is never
  mutated (L0b), so v1's row is history and v2 is a new pin.
- **`tacsim.pack.dray`**: digest `dc5cae81692853bca737049009e4fb9d768ea045b22789f6e2bf5c2786078583`
  → `da86f3899788dd9b2ff3141fbf97a5243cb1746b489276173c67dff26caf49fa` in the pack and
  `crates/tacsim/tests/content.rs` `DRAY_DIGEST`; the asserted law `kms: 205` → `kms: 226` at
  `ic2_the_dray_is_pinned_and_checks`. Reason: the cruise re-fit (L0a); version 1 kept because the
  Dray is a fielded extension pack with no ledger row, so no boot gate reads its version.
- **`tests/lane.rs`, each the behaviour the brief changes** (old → new): the module doc and the
  `LANE_DIGEST` doc from the in-place rule (R1) to the version rule (L0b);
  `ic2_the_lane_pack_is_pinned` `manifest.version() == 1` → `== LANE_VERSION` and `== 2`, the entry
  list gains `("ceres_port","port")` and `("stickney_port","port")` ("the twelve entries"), and four
  new `base_s` assertions (537,293 / 411,975 and 411,975 / 537,293); `lane_legs_derive_at_epoch`'s
  bodiless-place check `derived_legs(…, 205)` → `226` (its 100 s tolerance and logic unchanged,
  D-18-45); `one_drive_class_one_law` `kms: 205` → `226` and `dray_copy(205)` → `dray_copy(226)`
  (`dray_copy(300)` stays the refused one); `ic2_an_operator_seed_cannot_claim_the_lane_id`
  `…ceres_phobos@1, which ` → `@2, which `;
  `ic2_the_lane_boots_and_corridor_show_reads_the_constant_sky` `applied.version() == 1` → `==
  LANE_VERSION` and `"tacsim.lane.ceres_phobos@1 [seed]"` → `@2 [seed]` (`placed.len() == 3`
  unchanged: ports place no object).
- **`tests/stations.rs`**: `" {LANE_PACK}@1 ("` → `@2 (` (:420) and `after.get(LANE_PACK) ==
  Some(1)` → `Some(2)` (:938, not listed by the brief but the same lane-version pin).
  **`mush-server/tests/boot_parity.rs`**: `store.pack(&lane, 1)` → `LANE_VERSION` and the latch
  `Some(1)` → `Some(LANE_VERSION)`, plus the new port-map, nine-org and `test_world` assertions.
  **`mush-server/src/config_ext.rs`** test: `declares tacsim.lane.ceres_phobos@1` → `@2`.
- **Incidental moves allowed by the brief's section 6**: `mushd/tests/arena_story.rs` `say(&mut god,
  "@domain", "No domains.")` and `mushd/tests/observer_rule.rs` `ask(&mut god, "@domain", "No
  domains.")` — the old assertions pinned a world-wide domain count of zero, and the two port
  domains now exist on every roster that composes `content_module_with(_, true)`. Both are now
  `ask_whole(&mut god, "@domain")` asserting `host Stickney Yards presents in Visiting Berth
  Gallery` and `host Ceres Freeport presents in …` present and the muster line absent on the whole
  answer (stronger than the slice-to-first-needle the builder first wrote, which depended on
  domain-id order). No replay hash, mind-lane hash, `harness_baseline` or other digest moved; the
  tacsim harness, replay and minds suites pass unchanged.
- **`legacy-handlers.toml`**: the `+credit` stanza added in byte order (`read-only`, the brief's
  note); the `@tacsim` stanza's note refreshed with the fifth switch (not joined). `PINNED_BASELINE`
  stays **97** (no new primary verb on the legacy lane). **`state-slot-owners.toml`**: two rows for
  `crates/tacsim/src/port/apply.rs` — `OrgStore` owner `org`, `EconomyStore` owner `economy` — the
  allocator bump only; the records are created through `mush_org::submit` and
  `mush_economy::open_account`. `TacsimStore` writes stay row-free (same crate).
- **Unmoved, as the brief requires**: the panic budgets (`Option`/`Result` throughout; no
  `unwrap`/`expect`/`panic!`/indexing in the new non-test code), `module-ns-owners.toml` (no refused
  namespace write on any `ports.rs` boot or reboot), the restriction-lints census, the
  composition-edge pin (no new crate edge), the starter and station digests, `GOODS_DIGEST`, every
  shipped `DefinitionRef`, `tacsim.core` and both corridor fixture digests, and the CI gated-test
  inventory.

### How it was verified

**Reviewed, not run, by the judge** (read-only; no cargo): the whole uncommitted diff at base
484647bc and the six new files, against the brief, doc 18 §4.1/§4.4/§4.7/§5 and D-18-14/20/21/25/27;
`mush-content/src/apply.rs`'s check-before-mutate loop, `mush-sim/src/window.rs`'s remaining
`open_passive_domain` body, `mush-economy`'s `is_frozen` and the six `ObligationStatus` variants,
`tacsim/src/economy.rs`'s `ensure_instrument`, `ic_return::PORTS` and the mushd `ask_whole` helpers
were read to confirm the fixes are what the reports say.

**Three lenses** (correctness, conformance, tests) on the builder's tree agreed on one must-fix
defect — the "consistent prefix" rule let a later version top up an org whose recorded capital was a
multiple of 800,000 (Freeport Exchange 4,000,000 → 4,800,000), against L4 and D-18-27 — plus two
should-fixes (a frozen economy was not refused in `check`, so an apply would open the domain,
treasury, an org and its account before `create_system_obligation` answered `EconomyFrozen`; a
cancelled/delinquent/suspended tranche read "pending"), two test should-fixes (the mushd absence
checks depended on domain-id order; test 4 never exercised the update body creating anything), and
notes. The fixer took every must-fix and should-fix: `PortOrgRow.capital_major` + `capital_drift`
(unit test over 4,800,000 / 4,500,000 / 3,200,000 / 800,000 against five matching rows; test 4's v3‴
variant; a mutation check — with the `Changed` arm disabled, `cargo test -p tacsim --test ports
ic3_an_update_reuses` → `test result: FAILED. 0 passed; 1 failed`, restored afterwards); the
frozen-economy preflight with its test; `stuck_word` with its test; `ask_whole` in both mushd files;
the new-org and partial-completion tests. No repair pass was needed.

**Builder's pre-fix gate** (every brief command green): `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim
--test lane ic2_the_lane_pack_is_pinned` → `ok. 1 passed` (printed `LANE_DIGEST = "3f7e1a96…"`); `…
--test content ic2_the_dray_is_pinned_and_checks` → `ok. 1 passed` (`DRAY_DIGEST = "da86f389…"`);
`cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` →
`Finished` (exit 0); `cargo test -p mush-config --test shipped_configs` first failed on the
section-drift test and passed after deviation 1 (`ok. 12 passed`); `cargo test -p mushd
--no-fail-fast` first exit 101 on `arena_story` and `observer_rule`'s "No domains." (the section-6
incidental moves), then `ok. 1 passed` / `ok. 32 passed`.

**Final gate after the fixer, 21 steps, all pass** (logs under the run's
`scratchpad/cuts/IC3/gate1/`):

- `cargo fmt --all -- --check` → exit 0, no diff.
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s) in 0.67s`` (exit 0, fingerprint-cached, no warnings).
- `cargo test -p tacsim --lib` → `test result: ok. 861 passed; 0 failed; 0 ignored`.
- `cargo test -p tacsim --test ports` → `test result: ok. 17 passed; 0 failed`; `--test lane` → `ok.
  8 passed`; `--test stations` → `ok. 13 passed`; `--test content` → `ok. 29 passed`; `--test
  capacity` → `ok. 15 passed`.
- `cargo test -p mush-sim --lib` → `ok. 34 passed`; `cargo test -p mush-sim --test custody` → `ok.
  11 passed`.
- `cargo test -p mush-server --test boot_parity` → `ok. 18 passed`; `cargo test -p mush-server --lib
  config_ext` → `ok. 13 passed; … 280 filtered out`.
- `cargo test -p mush-config --test shipped_configs` → `ok. 12 passed`.
- `cargo test -p mush-contract-tests` → 29 `ok` lines, aggregate 127 passed; 0 failed; 8 ignored
  (state_slot_owners `ok. 4 passed`, legacy_handler_allowlist `ok. 4 passed`, panic budget,
  module-ns owners, restriction lints, help commands, CI inventory all ok).
- `cargo test -p tacsim --no-fail-fast` → 38 `ok` lines, aggregate 2058 passed; 0 failed; 12 ignored
  (cockpit `ok. 180 passed`, match_lifecycle `ok. 136 passed`; harness, replay and minds unchanged).
- `cargo test -p mush-server --no-fail-fast` → 29 `ok` lines, 445 passed; 0 failed; 2 ignored.
- `cargo test -p mushd --no-fail-fast` → 44 `ok` lines, 243 passed; 0 failed; 1 ignored (arena_story
  `ok. 1 passed` in 82.27s, observer_rule `ok. 32 passed` in 40.17s).
- `cargo test -p mush-bot` → 22 `ok` lines, 704 passed; 0 failed; 5 ignored (run twice, both exit
  0).
- `cargo test --workspace --exclude mush-bot --no-fail-fast` → first run exit 101, 383 result lines,
  7211 passed; **1 failed**; 92 ignored.

**The one flake, identified:** `mushd --test arena_story ::
two_pilots_walk_in_fight_and_one_tank_is_destroyed` failed at `arena_story.rs:1151` in the workspace
sweep only — `poll_whole(WEAPONSTATUS, "Locked on")` returned a snapshot where "Locked on BA, 68% to
hit." was present but the weapon rows did not yet print their band and to-hit (a race between the
lock line and the per-weapon projection, `WeaponRow::render` in `tacsim/src/match_/cockpit.rs`)
while the box ran 32 rustc + 12–23 link.exe at 100 % CPU. IC3 touches neither
`crates/tacsim/src/match_` nor `crates/mushd/src`; its only edit to that file is the `@domain`
assertion at step (4), which ran and passed before the failure point. The same test passed in the
per-package run (the `mushd` step above) and three times alone afterwards (76.36 s, 69.70 s, and
88.79 s on the exact workspace-feature-unified binary `arena_story-8aef5d71ff49236a.exe`).
`arena_story` is not on the known load-sensitive list; only its gateway twin `arena_story_split` is
— it evidently also races under a saturated box. The `panicked at
crates\mush-server\src\process_fault.rs:197:9` line in the sweep is the injected fault inside a
passing fault test. No LNK1104, no hung binaries, no timeouts; the working tree was not modified by
the gate.

### Surface for the next cuts

**Pack and lane** — `tacsim::definitions::content::LANE_VERSION: u32 = 2` beside `LANE_PACK`;
`LANE_JSON` is v2 (digest `3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0`, twelve
entries; `ceres_stickney` legs 537,293 then 411,975 s, `stickney_ceres` 411,975 then 537,293 s; port
entries `ceres_port` and `stickney_port`); IC8's v3 is a new version with a new pin, never v2
re-signed. `tacsim.pack.dray` digest
`da86f3899788dd9b2ff3141fbf97a5243cb1746b489276173c67dff26caf49fa`, law `coast {kms 226,
burn_permille 105}`, version 1. `tacsim::pack::PackEntry::Port(tacsim::port::PortDef)` (label
`"port"`, seed-only: in a `rules_definition` pack `PackError::WrongKindEntry { id, entry: "port",
kind: "rules" }`); `PackError::PortRefused { port: String, detail: String }` displayed `port entry
{port}: {detail}`; `pub fn tacsim::pack::rules_pack_of(mush_content::Pack) -> Result<RulesPack,
PackError>` (decode + every rule, no digest compare); `OWNED_KINDS: [&str; 16]` ending `"port"`.

**`tacsim::port`** (re-exports from `def`, `map`, `readiness`; `apply` and `credit` as public
modules) — `PortDef { location: CanonicalId, venue: CanonicalId, area: CanonicalId, gallery: String,
rooms: BTreeMap<CanonicalId, String>, treasury: String, orgs: Vec<PortOrgDef>, programme:
Option<ProgrammeDef> (default, skip if None), vendors: Vec<VendorDef> (default, skip if empty),
routes: BTreeMap<CanonicalId, Vec<CanonicalId>> }`; `PortOrgDef { id, name: String, capital_major:
u64, faction: CanonicalId }`; `ProgrammeDef { id, org, office: String, floor_major, unit, issuer:
String, pool: u32, registry_prefix: String, hull_names: Vec<String>, stake_major, share_permille:
u32, reference: BTreeMap<CanonicalId, String>, buyout_major, dormancy_real_days: u32, per_account:
u32 }`; `VendorDef { org, storefront: String, agent: CanonicalId, signatory: String (required),
stock: Vec<StockRowDef>, asks: Vec<AskDef>, bids: Vec<BidDef> (each default) }`; `StockRowDef {
good, qty: u64, grade: u8, issue: u32 }`; `AskDef { good, price_major: u64 }`; `BidDef { good,
price_major, intake: u64, as_name: Option<String> (serde "as") }`; all `Clone, Debug, PartialEq, Eq,
Serialize, Deserialize`, `deny_unknown_fields`. Constants: `PORT_ROOM_ROLES: [&str; 12]` = arrivals,
registry, clearing, exchange, chandlery, dockmaster, spine, customs, harbourmaster, receiving,
commissary, yard; `MAX_PORT_ORGS` 16, `MAX_PORT_ROUTES` 8, `MAX_PORT_VENDORS` 16, `MAX_VENDOR_ROWS`
16 (usize); `MAX_PORT_CAPITAL_MAJOR` 100_000_000, `MAX_TRANCHE_MAJOR` 800_000, `PORT_MINT_CAP_MAJOR`
1_000_000 (u64); `MAX_PROGRAMME_POOL` 16 (u32), `MAX_REGISTRY_PREFIX_BYTES` 16,
`MAX_DORMANCY_REAL_DAYS` 365. `pub fn tranches(capital_major: u64) -> Vec<u64>`; `pub fn
room_key(area: &CanonicalId, reference: &str) -> Result<mush_atlas::RecordKey, String>` (error
texts: `` `<ref>` is not in the port's area `<area>` `` / `` `<ref>` does not name an entry of
`<area>` ``); `impl PortDef { pub fn check(&self) -> Result<(), String> }` (detail only, e.g. `org
coop: capital_major must be 1..=100000000, not 0`); `pub fn check_ports(&BTreeMap<CanonicalId,
PackEntry>) -> Result<(), (CanonicalId, String)>`.

**The port map** — `PORT_MAP_PREFIX = "tacsim:port:"`, `PORT_MAP_SCHEMA: u16 = 1`,
`port_map_key(pack, entry) -> SmolStr` (`tacsim:port:{pack}/{entry}`); `PortMap { schema: u16, pack,
entry, domain: Option<u64>, host: Option<u32>, presentation: Option<u32>, treasury: Option<u64>,
orgs: BTreeMap<CanonicalId, PortOrgRow> }` with `PortMap::new(pack, entry)` and `key()`; `PortOrgRow
{ org: u64, root_scope: u64, capital_major: u64, account: Option<u64>, capital: Vec<TrancheRow> }` —
`capital_major` is the authored capital the row was created for, written with the org, and the one
number a later apply compares against; `TrancheRow { n: u32, obligation: u64, amount_minor: i64 }`;
`PortMapState<'a> { Absent, Present(&'a PortMap), Unreadable }`. `TacsimStore::port_map(&self, pack,
entry) -> PortMapState<'_>`, `port_maps() -> impl Iterator<Item = &PortMap>`, `test_world() ->
bool`; crate-private `put_port_map(ctx, &PortMap)` (the single write path) and `port_map_claims()`.
Stored under the identity rule: a body whose `pack`/`entry` disagree with its key, or that will not
parse, loads as `Unreadable` and bars re-creation.

**`PortApplier`** (`tacsim::port::apply::PortApplier`, kind `port`, phase 11; `pub fn room_by_key(w:
&World, key: &RecordKey) -> Option<ObjId>`; crate-private `room_references(&PortDef) -> Vec<&str>`
(gallery, rooms in map order, office), `capital_drift(authored_major, &PortOrgRow, scale) ->
Option<CapitalDrift::{Changed { recorded }, Inconsistent { recorded }}>`). Preflight order: vendors,
layers, map readability, rooms, gallery marker, drift (presentation, domain, treasury, per org:
authored, exists, account, obligations, name, capital), mint cap over the largest still-to-create
tranche, frozen economy when any tranche is still to create. `create`/`update` both run `put` and
return `Ok(None)`; `retire_check` `Ok(())`; `retire` logs, writes nothing, keeps the map. What it
makes: the treasury account named `"tacsim.treasury"`, owner
`PrincipalId::System("tacsim.treasury")`, `CreditPolicy::unbounded()`, scope `None`; per org
`Organization { name, aliases [], Active, hidden false, charter "", root_scope }` + root `OrgScope`
named `"root"` in one `CreateOrg`, audit `org.create` detail `{port: "<pack>/<entry>", org, name}`;
the account `"<org name>/root"`, owner `Organization(org)`, scope `Some(root)`,
`CreditPolicy::forbid()`; each tranche an `ObligationSpec` treasury → org account, amount `major ×
10^scale`, `EconomicSchedule::new(RealUtc, now, None, 1)`, `GracePolicy::default()`,
`notify_on_success: false`, `cause: Ruleset { namespace: "tacsim.port" }`. Refusal texts (each
prefixed `{id}: `): `port vendors are not applied by this server yet`; `the port map {key} is
unreadable; it is never re-created — repair or delete the document by hand`; `the room {ref} is not
on this world (no room carries its RoomGeo)`; `the gallery {ref} does not carry port.dock`; `a
capital tranche is {t} cr but this world's mint cap for tacsim.credit is {cap} cr; set
[economy.instruments."tacsim.credit"].mint_obligation_cap_major to at least {t}`; `the economy is
frozen on this world, so {n} capital tranche(s) cannot be issued; nothing is applied while it is
frozen`; `org {org} is in the port map {key} but no longer authored; orgs are never retired by a
port update`; `org {org} in the port map {key} names org #{n}, which does not exist; it is never
re-created — repair the map by hand` (account and obligation variants); `org {org} is named {live}
on this world but {authored} in the pack; this server cannot rename an org`; `org {org} capital
{authored} cr differs from the {recorded} cr already issued; a port update never re-capitalizes`;
`org {org}'s tranche rows in the port map {key} are not the tranches of its {recorded} cr; they are
never re-issued — repair the map by hand`; `{host} already hosts domain #{d}, which this port's map
does not name`; the presentation/domain/host re-host refusals end `a port update never re-hosts its
domain`. Lane first-boot refusals surface as `content: first-boot pack tacsim.lane.ceres_phobos@2
refused: …`.

**Readiness** (`tacsim::port::readiness`) — `Binding::{Lane, Sky, Rate, Treasury, Map, Rooms,
Domain, Org, Programme}`, `LineState::{Ready, Pending, Missing, Flagged}` + `word()`, `ReadinessLine
{ port: Option<CanonicalId>, binding, state, text }`, `Readiness { lines }` with `is_ready()`,
`not_ready()`, `summary()`, `render() -> Vec<String>`; `pub fn readiness(&World) -> Readiness`;
`PLAY_RATE: u32 = 24`. IC6 gates on `is_ready()` / the typed states, never on text. Texts: `lane:
tacsim.lane.ceres_phobos@{v} applied; this server ships @2.` (Ready) / `… ships @2. Take it with
@atlas/pack plan tacsim.lane.ceres_phobos.` (Flagged, older) / `… ships @2 and does not load @{v}.`
(Flagged, newer unloaded) / `lane: tacsim.lane.ceres_phobos is not applied on this world.`
(Missing); `sky: constant (rate 0), epoch 2351-05-13, rev <12 hex>` (`moving (rate {r})` when
non-zero; `rev unavailable` Flagged; `sky: no lane pack is loaded on this server.` Missing); `world
rate: {r} IC seconds per real second.` / `… (test world).` / `…; play runs at 24.` (Flagged) /
`world rate: this world has no time generation.` (Missing); `treasury: system:{t}, account #{a}.` /
`treasury: no account is open for system:{t}.`; per port `{entry}: its port map {key} is
unreadable.` / `{entry}: not applied.`; `{entry} rooms: {n} of {m} resolved; the gallery carries
port.dock.` / `…; unresolved: {refs}.` / `…; the gallery does not carry port.dock.`; `{entry}
domain: #{d} on {host}, presents in {gallery}.` / `{entry} domain: none is recorded in {key}.` /
`#{d} is not active on this world.` / `#{d} is hosted on {name}, not the room its map names.` /
`#{d} presents in {name}, not the gallery.`; `{entry} org {id}: {name}, org #{o}, account #{a},
capital {money}: {s} of {t} tranches settled.` (Ready/Pending) / `{entry} org {id}: {name}; the org
is not created.` / `…; org #{o} does not exist.` / `…, org #{o}; its account is not open.` / `…,
account #{a}; capital obligation #{x} does not exist.` / `…; {r} of {t} capital tranches are
created.` / `…; capital obligation #{x} is {cancelled|delinquent|suspended}, and no sweep will
settle it.` (all Missing); `{entry} programme {pid}: {org name}, account #{a}, floor {money}, stake
{money} for each of {pool} hulls.` / `{entry} programme {pid}: {org name} has no account open.`;
last `Ready: every binding above is in place.` / `Not ready: {k} of {n} lines are pending, missing
or flagged.`. Money renders through `mush_economy::render_amount` (`4000000.00cr`).

**Verbs** — `@tacsim/port [status]` (staff; `require_enabled` then `require_staff` → `Permission
denied.`; `Port readiness:` header, `"{text} [{word}]"` per line with port lines indented two
spaces, the summary; any other argument `Usage: @tacsim/port status` =
`tacsim::commands::PORT_USAGE`). `+credit` (`tacsim::port::credit`: `CREDIT_VERB`, `CREDIT_HELP`,
`NO_ACCOUNT = "No credit account is open in your name."`, `UNREADABLE = "Your account's balance
cannot be read right now."`, `pub fn credit_line(posted, held, available) -> String` = `Credit: {p}
posted, {h} held, {a} available.`, `pub fn own_account(&World, ObjId) -> Option<&Account>`;
ReadOnly, `ArgShape::None`; disabled layer → `TacSim is not enabled on this server.`).
Legacy-handler stanza `+credit` (read-only); `@tacsim` note refreshed; `PINNED_BASELINE` 97.

**Composition and config** — `TacsimRuleset::with_test_world(self, bool) -> TacsimRuleset`;
`tacsim::composition::tacsim_test_world_module(cfg, game_content) -> Box<dyn GameModule>` (test
engines only); `server_modules_with(weather, mut economy, atlas, onboarding, game, recovery)`
inserts `instruments["tacsim.credit"] = InstrumentCaps { mint_obligation_cap_major: 1_000_000 }`
only when absent; `EconomyConfig::cap_major(tacsim.credit)` is 1,000,000 on every roster through it
unless an operator per-instrument value is given. Template and example both declare
`[economy.instruments."tacsim.credit"] mint_obligation_cap_major = 1000000`.
`state-slot-owners.toml` carries the two `port/apply.rs` rows (`OrgStore`/`org`,
`EconomyStore`/`economy`).

**mush-sim** — `open_passive_domain(ctx, host, presentation)` refuses only a gone/going host, a host
that already has a domain, a presentation that is not a live room, or a spent allocator; it is no
longer bounded by `MAX_WINDOWS`. `open_window` keeps its guard.

**Test rig patterns later cuts can copy** (`crates/tacsim/tests/ports.rs`): `Roster::{production,
bare, test_world}` + `with_shelf` (replace the atlas module at its index with
`AtlasModule::with_config(default).with_shelf(vec![pack])` for a shelf-applied version;
`AtlasConfig.packs` would be shadowed by the compiled v2); `sweep(tw, secs)` (set_now + fire_due);
`plan_and_apply`; `lane_variant(version, edit)` re-signed with `support::ground::sign`; `counts()`
over orgs/accounts/obligations/domains/journal; `map_docs(ops)`; `assert_nothing_bound(&World)`; a
refused first boot through `Engine::with_modules` + `modules_loaded` hands back `engine.world`;
`mush_economy::obligation_doc_key(ObligationId)` and `account:{id}` for folded-store surgery. Every
boot asserts `refused_namespace_writes()` is empty.

## IC4

**Walking aboard: the boundary rule and the hull template** (doc 18 §5, the IC4 row and paragraph),
built 2026-10-16 on main at ea52634d, minus the contract walk of the Dray's exit words, which rests
on an owner ruling. One commit, pushed to origin/main.

### What landed

1. **One interior-space record governs several walkable rooms** (`crates/mush-sim/src/data.rs`,
   `transition.rs`, `graph.rs`, `lib.rs`). `MAX_SPACE_ROOMS = 16` is re-exported;
   `InteriorSpaceRecord.rooms` is documented as the member set (empty on a legacy record, which
   stays single-room, L3); `Derived.member_of` maps every member to its space and the space object
   to itself (L4), filled by `rebuild` in key order, first wins, and checked by `verify_indexes`
   with the rest of `Derived`. `Transition::Members { space, rooms }` is a pure plan
   (`plan_members`) refusing, in order, a dead or unrecorded space, a set without the space, more
   than 16, and for each other room: dead, not a Room, a space of its own, a domain host, a live
   domain's presentation room, a member of another space; then the removal of a room an occupant
   stands in (`"{r} still holds {name}; move them first"`); on success `set_spaces` with the
   record's `rooms` replaced, no projections, and the audit `{"op":"members","space","rooms"}` on
   the recovery ledger. `Transition::Space` refuses a member before `live` (`"{space} is a room of
   interior space {owner}; it cannot be a space itself"`). `SpatialError::Members(String)` renders
   as its sentence.
2. **Projection keeps a walker where it walked** (`crates/mush-sim/src/graph.rs` `project`): the
   Interior arm answers the occupant's current location when the space's `rooms` lists it, else the
   space object, resolved on `cur` so an exposed mount riding an occupant follows the occupant. Boot
   reconcile and every `CarrierMove`/`Place` projection bundle therefore leave a walker in its
   member room (hulls test 6, members test 5: a reboot keeps the walker in the Bridge/Deck with no
   `placement:` write; a walker standing outside the members at boot lands in the lock).
3. **The destination-aware gate** (`crates/mush-sim/src/lib.rs` `move_reason`): journeying, held,
   quarantined, then the member arm — an `Interior`-placed actor with `exit.is_some()` whose `dest`
   is in its own space's `rooms` is allowed and writes nothing — then the by-parent refusal. `home`,
   `@teleport`, `+ic` and `@dig/teleport` pass no exit and stay refused for everyone, GOD included.
   `gate_reason` (so `carry_attempt`) has no member arm. The multi-room aboard sentence is `"{name}
   is aboard {host}; it moves only between {host}'s rooms, or leaves through the simulation or staff
   recovery (@spatial)."` with host the `MovingObject` anchor, else the space; a single-room space
   keeps the old sentence byte for byte. The gate was split into `held_or_quarantined`,
   `parent_reason`, and the crate-private `aboard_host` and `is_multi_room`.
4. **A member cannot be destroyed or made a space, and a programmatic destroy fails closed**
   (`crates/mush-sim/src/lib.rs`): the destroy gate's member arm (`"{room} is a room of the interior
   space {space}; @spatial/unspace that space first."`); `on_obj_destroyed` captures the husk's
   `member_of` space and `drop_destroyed_member` removes it from `rooms`, rebuilds, persists the
   space, warns, and moves every `Interior` child of that space whose location disagrees with its
   projection back into the space object (`World::destroy` had sent them home or to Limbo), `warn!`
   per move, `error!` on failure; `verify_projections` is empty afterwards (members test 7).
5. **The custody and egress guards** (`crates/mush-sim/src/custody.rs`, `commands.rs`):
   `backing_valid` answers `Some(true)` only for a record with at most one room (the bay principle:
   a walked room never backs a vault); the player `egress`/`disembark` in a multi-room space answer
   `"You cannot egress from {host}; use +ship/leave in the {space}."`; staff `@spatial/egress` is
   untouched.
6. **`pub fn declare_compartment(ctx, space, slot, maximum_items) -> Result<(), String>`**
   (`crates/mush-sim/src/window.rs`, re-exported): the `@spatial/slot` write without its parser;
   `spatial_slot` now calls it (one writer, L1). `"That is not an interior space."` for a room
   without a record.
7. **Boot drops invalid members in RAM and pages** (`crates/mush-sim/src/boot.rs`): the pure
   `pub(crate) fn invalid_members(st, w) -> Vec<(space, Vec<(room, reason)>)>` applies the plan's
   per-room tests only to `r != space`, keeps at most 16 (the space first, then id order) and never
   names the space object whatever its kind, so the atlas's Thing pods boot silent (unit test
   `ic4_invalid_members_spares_the_space_object`, members test 6a); `drop_invalid_members`, after
   the dangling-anchor pass and before the reconcile rebuild, removes them from the RAM record only,
   pages with `tracing::error!` naming the space, every room and its reason, plus `runbook = "repair
   the space:<id> document by hand or @spatial/unspace the space"`, and re-inserts a missing live
   space object unpaged. A walker standing in a dropped room then projects to the space and
   reconcile moves it there (members test 6: no `space:` write, the walker in Lock).
8. **Narration aboard** (`crates/mush-sim/src/journey.rs`): `onboard_rooms(st, root)` (every space
   in the carrier's bounded subtree, its object and its `rooms`, deduplicated and sorted) is shared
   by the scene-floor raise and the new `narrate_aboard`; at an intermediate boundary every room
   aboard hears `"{root} passes {waypoint}. Leg {n} of {m}."` (`n = target + 1`), and inside the
   arrival's `!already` block `"{root} arrives, and the passage ends."` — `NarrativeKind::Emit`,
   `actor: None`, through `to_room_from(room, [], root)` (members test 10).
9. **`EconomyStore::is_title_issuer` is `pub`** (`crates/mush-economy/src/module.rs`), `#[must_use]`
   kept, so a ruleset's applier preflights its issuer before any write. Nothing else in mush-economy
   moved.
10. **The port map's hull rows** (`crates/tacsim/src/port/map.rs`): `PortMap.hulls: Vec<HullRow>`
    with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so a map without hulls encodes
    byte-identically to IC3's and `PORT_MAP_SCHEMA` stays 1 (L9); `HullRow { n, mark, hull, rooms:
    BTreeMap<CanonicalId, u32>, exits: Vec<u32> }`, `deny_unknown_fields`, objects only — unit,
    spaces, members, compartment, placement and title are read from their owners; the space and the
    store are `rooms[interior.space]` and `rooms[interior.store]`. Re-exported from `tacsim::port`.
11. **`PortApplier` part (f)** (`crates/tacsim/src/port/hull.rs`, new; wired from `apply.rs` after
    part c, L8). `REGISTRY_ISSUER = "tacsim.registry"`, `HULL_DETAIL_PREFIX = "tacsim:detail:"`,
    `HULL_SPACING_CM = 10_000`, `registry_mark(prefix, n)` = `{prefix}{n+1:02}`. `preflight(reg, w,
    pack, id, def, map) -> Result<usize, String>` refuses, each prefixed `"{id}: "`, a
    programme-less entry whose map records hulls, units/TacSim/economy not composed, a unit without
    `logistics` + `interior` or without a binding, an issuer that is not a registered title issuer,
    any exit segment that is a kernel command, a berths id that is not a `SlotId`, and every drift
    against the recorded rows (`n >= pool` "never retired", a mark not `registry_mark(prefix, n)`
    "never re-issued", a recorded hull/room/exit not live "never re-created — repair the map by
    hand", a room the template does not author, more exits than `2 × pairs` "exits are never
    retired"); it answers the rows absent plus the recorded hulls without an asset, which `apply.rs`
    adds to the frozen-economy refusal (`"… so {to_make} capital tranche(s) and {h} hull title(s)
    cannot be issued; …"`, IC3's sentence kept exactly when `h == 0`). `lay_up(ctx, id, programme,
    &mut map, domain, gallery)` runs per hull in pool order (so *Slack Water*'s Thing has the lowest
    id, L12), the map written after every object (L11): (1) the Thing named from `hull_names`, owner
    the applying actor, alias the mark, in the gallery; (2) `create_unit` from the bound spec, only
    for a hull no `UnitStore.units` entry and no `records` claim names; (3) the seven Rooms, then on
    every pass `dress` (name when it differs, `desc` and the tree's `short_desc` attribute removed
    when blank, every `tacsim:detail:*` attribute removed and the template's details rewritten under
    `fold_detail_keyword`); (4) the five exit pairs in `open_exit`'s shape (first segment the name,
    the rest aliases, `link_exit`, `ObjCreated`), index `2i`/`2i+1`, each only when the row's
    `exits.len()` equals its index; (5) the store and (6) the airlock as spaces anchored
    `MovingObject { host: hull }` through `attach sim.interior` + `Transition::Space` only when
    unrecorded, then `Transition::Members` with the six walkable rooms when the set differs and
    `declare_compartment(berths, Some(2))` when the cap differs; (7) `mush_sim::place` `Direct` at
    `(0, n × 10,000 cm)` with the passive posture only when the hull has no placement record, a held
    one included; (8) `issue_title` to `Organization(coop_trust)` at the Trust's root scope under
    the issuer, no catalog, only when the hull has no asset record. `put`'s doc, the module doc,
    `retire_check`'s comment and `retire`'s log now name the hulls (they stand on retire; the leased
    refusal is IC6's).
12. **The issuer and the detail shelf** (`crates/tacsim/src/composition/mod.rs`, `lib.rs`):
    `tacsim.registry` joins the title issuers unconditionally beside industry's;
    `TacsimRuleset::register` claims attribute namespace `tacsim` and detail prefix `tacsim:detail:`
    before `commands::register`, so `look builders plate` in a hull's Airlock reads the template
    (hulls test 2).
13. **The hulls readiness line** (`crates/tacsim/src/port/readiness.rs`, `hull.rs` `hull_line`):
    `Binding::Hulls` after the programme line, the first failing check — `{k} of {pool} are made.`,
    `{mark} (#{obj}) does not exist.`, `{mark}'s interior is incomplete.` (a template room missing,
    fewer than `2 × pairs` exits, or the airlock or store record absent), `{mark} is not titled to
    {org name}.`, `{mark} is not laid up at a port.` (not `Direct` on a domain some port map
    records) — else `{pool} of {pool} made, titled to {org name} and laid up.` IC6 reads the typed
    state (L17).
14. **Tests.** `crates/mush-sim/tests/members.rs` (new, 11): the plan's refusals, persistence and
    index across a reboot, walking with no placement write, every other way off (non-member exit,
    `home` with home set to a member, the player's and GOD's `@teleport` to a non-member and to a
    member room with JUMP_OK set so the gate answers, a single-room occupant walking an exit into
    another space's member), projection across a reboot, the boot drop, the Thing pod, the destroy
    and space gates with the backstop, custody and egress, `declare_compartment`, narration aboard;
    the rig keeps one ordered op log of the engine's `take_module_ops()` and the `Deferred::Persist`
    ops of its throwaway outboxes (F4). `crates/mush-sim/src/boot.rs` unit test 6b.
    `crates/tacsim/src/port/hull.rs` unit test on `registry_mark`. `crates/tacsim/tests/hulls.rs`
    (new, 15): the eight Drays row by row (marks, names, ascending ids, owner GOD, alias, C01, the
    Ceres domain, the unit, seven rooms, ten exits with names/aliases/source/destination, both
    spaces, members, berths 2, the title and scope, Stickney empty, `look` in C01), the room text
    and `look <detail>`, `@asset/show`, lens 1 (ten moves, zero placement writes, the revision
    unchanged), lens 2 (`o`, `s`, `c`, `home`, `+ic` as `Sys`, the player's and GOD's `@teleport`,
    `egress`, `disembark`), lens 3 (the reboot), the berths cap, the member gates on the records,
    the vault backings, the second boot, the grown pool (+18 objects, +1 title, the eight rows
    byte-identical), the held-document skip (correctness F1), the four drift refusals, the readiness
    line, the issuer. `crates/tacsim/tests/ports.rs` test 8 expects the hulls line (+1) and the
    frozen-economy test checks the `8 hull title(s)` sentence and no *Slack Water* Thing;
    `crates/mush-server/tests/boot_parity.rs` checks the Ceres map's eight hulls each titled to the
    Trust.

**Found by review and fixed in the same cut:** on a kept world, a hull whose `placement:` or `unit:`
document boot had held stopped the whole lane apply partway through (`Transition::Place` and
`create_unit` refuse a held object); `place_and_title` and step 2 now count a held placement and any
`UnitStore.records` claim as "already there" and skip the hull
(`ic4_a_held_hull_document_is_skipped_not_refused`). The member arm's `exit.is_some()` and own-space
clauses had no test that failed without them, and the player `@teleport` steps were
`perm::can_teleport` refusals, not the gate's; both fixed in the tests.

### Deferred, and why

**Lead rulings taken by the planner** (recorded here so the owner sees them):

- **L1 — One new mush-sim function only**: `Transition::Members` and `pub declare_compartment` (doc
  18 §4.2 schema additions); a space is declared the way `mush-atlas::apply::make_interior` does it
  (attach `sim.interior` + `mush_sim::run(Transition::Space)`); no `declare_space`/`declare_members`
  wrappers; `@spatial/slot` delegates to `declare_compartment`.
- **L2 — No `@spatial/members` staff switch**: members are content-owned; staff recovery is
  `@spatial/unspace`; tests reach the transition through `plan`/`apply`/`run`.
- **L3 — The gate's member arm**: Interior-placed actor, `exit.is_some()`, `dest` in its own space's
  `rooms`, after the journeying, held and quarantined arms; `carry_attempt` unchanged (a carry of a
  multi-room occupant answers the new aboard sentence through `placed_or_quarantined`, intended);
  "multi-room" means `rooms.len() > 1`, a legacy empty `rooms` is single-room.
- **L4 — `member_of` maps the space object to itself**; the destroy and `Space` arms exclude the
  self-entry.
- **L5 — Boot drops invalid members in RAM only, pages, never rewrites the document** (§4.2 (c); the
  held-placement precedent: staff repair the raw document). The space object is never dropped or
  paged whatever its kind (the atlas's Thing pods; first-review F1).
- **L6 — The engine sentences** (the aboard refusal, the egress refusal, the two narration lines)
  are mush-sim's generic lines in the voice of the existing ones; the trade run's flavoured
  narration is IC9's. **Owner-critical**, below.
- **L7 — Narration rides the existing arrival conditions** (inside `!already`, no `Source` gate) and
  one `onboard_rooms` walker shared with the scene-floor raise.
- **L8 — Part (f) lives in `port/hull.rs`**, called from `put` after part c; IC8 inserts d–e before
  it.
- **L9 — The map records objects only** (`HullRow { n, mark, hull, rooms, exits }`), the space and
  store derived from `rooms`; `PORT_MAP_SCHEMA` stays 1, `hulls` skipped when empty (IC3's
  precedent).
- **L10 — Update semantics**: a later apply creates missing rows, re-dresses rooms, never renames a
  hull (IC6 owns the rename), never re-places a hull with any placement record, never re-titles one
  with any asset record; a shrunk pool, a changed prefix, a recorded object gone, an unauthored room
  or excess exits is refused as drift before any write. Against §4.7's "Update rewrites names in
  place" for hulls, by the IC3 L4 precedent.
- **L11 — The map is written after every object created** (about 150 whole-map rewrites in the one
  first-boot batch).
- **L12 — Registry mark = prefix + two-digit `n + 1`**, the Thing's alias and the row; hulls made in
  pool order so 0401's Thing id is the lowest (IC6's `next_loaner`).
- **L13 — Title** to `Organization(coop_trust)` at the Trust's root scope, no catalog, issuer
  `tacsim.registry` registered unconditionally (industry's precedent), preflighted through
  `is_title_issuer` made `pub`.
- **L14 — Room text on tacsim's own shelf**: `desc`, the tree's `short_desc`, details on namespace
  `tacsim` with prefix `tacsim:detail:`, never atlas's; `dray.json` not edited, its pin unmoved. The
  re-dress runs only inside a lane apply, so hull room text follows `dray.json` only through one
  (IC6's meta-text strip must bump the lane or ride IC8's v3).
- **L15 — Exits** follow `open_exit`'s shape with the kernel-verb refusal preflighted; the
  `exit_command_words` half is **stopped**, below.
- **L16 — Laid-up pose** `(0, n × 10,000 cm)`, passive posture; the hull Thing has no description.
  **Owner-critical**, below.
- **L17 — One readiness line per programme port**, `Binding::Hulls`, "laid up" = `Direct` on a
  domain a port map records; IC9 adds under way; IC6 reads the state, never the text.
- **L18 — No lane bump.** Part (f) runs only inside the lane's apply, so a world that latched v2
  under IC3 gets its hulls at IC8's v3 or a wipe; dev1 never booted IC1–IC3. **Owner-critical**,
  below. Consequence: IC3 reviewer F9 is now reachable, staff only — a staff `@journey/depart` on
  one of the eight `Direct`-placed hulls flies the lane at the authored `base_s`; IC9's to close, no
  code changed for it here.
- **L19 — Tests in two new files**, each with its own copy of the minimal rig; `ports.rs` edited
  only where C7 says.

**Owner-critical defaults taken, carried to the owner:**

- **The engine sentences (L6)**: the aboard refusal `"{name} is aboard {host}; it moves only between
  {host}'s rooms, or leaves through the simulation or staff recovery (@spatial)."`, the egress
  refusal `"You cannot egress from {host}; use +ship/leave in the {space}."` (true only once IC6
  lands `+ship/leave`), and the narration `"{root} passes {waypoint}. Leg {n} of {m}."` / `"{root}
  arrives, and the passage ends."` are player-facing text doc 18 does not give verbatim; the
  fiction's voice is the owner's.
- **The hull Thing has no gallery description (L16)**: 18a authors none; `look` in C01 lists the
  eight names only.
- **No lane bump (L18)**: changes when hulls appear on any kept world; nothing deployed is affected.

**Stopped and reported, not improvised** (an owner ruling): **C7's `exit_command_words.rs` walk of
the Dray's exit segments is not landed.** The template's `airlock → passage` pair carries `names[1]
= "Airlock <A>;airlock;lock;forward"` (`dray.json:186`), so every hull's Central Passage has an exit
aliased `lock`, and `lock` is tacsim's cockpit verb LOCK (`cockpit.rs:222`) — the production walk
found `(tacsim.unit.dray/airlock-passage, lock, lock)`, which is not in `DECIDED`, and the brief
says to STOP on a module-verb collision. The builder reverted the file so the gate stays green. Two
options for the owner: a DECIDED row with its reason (LOCK means nothing outside a seated cockpit,
and no seat exists aboard a Dray, D-18-23 — the F.4 `board` precedent), or a renamed alias in a new
`tacsim.pack.dray` version with the pin re-blessed. Either way the prepared extension
(`load_pack(DRAY_JSON)` → `PackEntry::Unit` → `.interior.exits`, entry id
`tacsim.unit.dray/<from>-<to>`) lands after; the apply-time `hull::preflight` checks kernel words
only.

**Builder's and fixer's decisions beyond the brief:**

- `lay_up(ctx, id, programme, &mut map, domain, gallery)` drops the brief's `pack` and `def`
  arguments (`pack` is `map.pack`, `def` was unused) to stay under clippy's argument count without
  an `#[allow]`; steps 1–6 are `lay_up_one` over a `HullPlan`, 7–8 `place_and_title`, run per hull
  in pool order.
- `hull_line` lives `pub(crate)` in `hull.rs` and also requires `exits.len() == 2 × pairs` for
  "complete"; its existence check walks the hull, then its rooms, then its exits and names the first
  gone object.
- `declare_space` skips an existing record entirely (no `space:` rewrite on a re-apply); members and
  berths are written only when they differ.
- A hull is skipped at step 2 when `UnitStore.units` has it OR any `UnitStore.records` row claims it
  by `key_object` or `object`, and at step 7 when `SpatialState.placements` has it OR
  `held_placement` is `Some` (correctness F1): a held, unreadable document is still the hull's
  record, staff's to repair, never shadowed and never a mid-pass refusal; the readiness line then
  reads that hull as not laid up.
- The berths `SlotId` is converted in `preflight` and again in `lay_up` (both fallible, same
  refusal; `put` re-runs preflight anyway) rather than converted once and handed on.
- Boot's drop reasons are fixed strings: `the room does not exist`, `not a room`, `an interior space
  of its own`, `hosts a domain`, `the presentation room of a live domain`, `already a room of an
  earlier interior space`, `over the interior space room cap`.
- The arrival aboard-line is inside `!already` but outside the existing `if let Some(to)` guard;
  harmless, since a successful `CarrierMove` always leaves the root a location.

**Left as notes, not fixed (honestly small):** hull exits are recorded by position with a count
check only, so the Dray template's `interior.exits` are **append-only** — a later version that
reorders or inserts a pair would duplicate the last pair's exits and still read complete; the hulls
readiness line goes Missing for any hull not Trust-titled and `Direct` at a port, so IC6's gate must
read "the programme was made" (or IC9 must add the leased and under-way arms) or the first lease
would refuse every later enrolment; `Transition::Members` does not refuse a vault-backing space,
LIMBO, the recovery room or the in-transit room as members (no production caller can reach it);
boot's RAM-only drop becomes permanent on the next `persist_space` of that record; the `rooms` field
doc says "written only by `Transition::Members`" while `drop_destroyed_member` also writes it; the
player-side `s`/`c`/`home` steps in hulls test 5 assert only the location; the laid-up pose spacing,
four of the six readiness states, four of boot's seven drop reasons and the ninth hull's spaces,
placement and title owner in the grown-pool test are not asserted; "no reconcile warning" in members
test 5 is proven by the unchanged location and the absence of writes, not by log capture; the hull
Thing's owner is `ctx.actor` (GOD at boot and on `@atlas/pack apply`).

**Deferred to named cuts:** `+ship`, `+ship/board`, `+ship/leave`, `+ship/hold`, the `look` status
line and the per-room gauges, the meta-text strip, the hull rename at enrolment, the hold vault on
the stowage, `+coop`/`+coop/enrol`/`+coop/statement`, the lease, the operate gate, the readiness
**gate** and the leased refusal in `retire_check` — IC6; the action provider, the typed families and
the `tacsim.port` tick — IC5; the market seams — IC7; vendors, vaults, stock, the goods catalog
load, lane v3 — IC8; `plan_passage`, the burn, `+ship/undock`, departure and docking, the flavoured
narration, the readiness line's under-way arm, `+travel/status` aboard, refusing exits into anchored
interiors at apply time — IC9. Never in the slice: a destination seal, a seat-reach re-check,
stations, `BerthLink`, gangway exits, a Moved observer (P-G/P2), berth slots (P-F), the under-way
inventory seal (P-H). Deploying anything is the owner's.

### Pins re-blessed

- **`crates/tacsim/tests/ports.rs`
  `ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`**: the expected report gains
  `  ceres_port hulls: 8 of 8 made, titled to Co-op First Captain Trust and laid up. [ready]` after
  the Ceres programme line, so `total` (= `want.len()`) moves 18 → 19 and the "9 pending" count is
  unchanged. Reason: the new readiness line (C7).
- **`crates/tacsim/tests/ports.rs`
  `ic3_a_frozen_economy_refuses_the_lane_before_anything_is_made`**: additive — the refusal now also
  contains `capital tranche(s) and 8 hull title(s) cannot be issued` and no Thing named *Slack
  Water* exists; no earlier assertion changed.
- **`crates/mush-server/tests/boot_parity.rs`
  `ic1_a_kept_world_takes_the_stations_when_the_game_is_switched_on`**: additive inside
  `lane_resolves` — the Ceres map records eight hull rows, each asset titled to
  `Organization(coop_trust)`.
- **Unmoved:** every panic budget (no `unwrap`/`expect`/`panic!`/indexing added outside
  `#[cfg(test)]`); `legacy-handlers.toml` (`PINNED_BASELINE` 97; no new verb — `@spatial`, `egress`
  and `@tacsim/port` keep their specs); `state-slot-owners.toml` (every write goes through the
  owner's function: `mush_sim::run`, `place`, `declare_compartment`, `create_unit`, `issue_title`);
  `module-ns-owners.toml`; the composition-edge pin (the tacsim →
  mush-sim/mush-economy/mush-tactics/mush-org/mush-atlas edges already exist); the restriction-lints
  census (no `#[allow]` added in `src/`); every content digest (`tacsim.pack.dray`, lane v2, goods,
  starter, stations, corridor fixtures) and every shipped `DefinitionRef`; the replay, mind-lane and
  `harness_baseline` hashes (hull units reached no match world); the CI gated-test inventory;
  `exit_command_words.rs` and its two `DECIDED` rows (the Dray walk is the stopped item above);
  `PORT_MAP_SCHEMA` 1; no IC3 first-boot object or journal count moved; `e7c6` (68) and `placement`
  (31) green with no assertion changed.

### How it was verified

**Reviewed, not run, by the judge** (read-only; no cargo): the whole uncommitted diff at base
ea52634d (18 modified files, 789 insertions, 73 deletions; the three new files
`mush-sim/tests/members.rs`, `tacsim/src/port/hull.rs`, `tacsim/tests/hulls.rs` read in full)
against the brief, doc 18 §4.2, §4.7 part f, D-18-22/23/29/34 and §3 beats C, D and J; the gate
order and sentences, the projection arm, the backstop, the boot pass, the applier's steps and the
readiness line checked line by line; `git diff -U0` on the touched tests removes no line; the
`lock`/LOCK collision confirmed at `dray.json:186` and `cockpit.rs:222`; `exit_command_words.rs`
confirmed unmodified.

**Three lenses** (correctness, conformance, tests) on the builder's tree agreed on no must-fix.
Correctness's one should-fix (a held `placement:` or `unit:` document refused the apply partway
through) and tests' two should-fixes (the member arm's narrowing clauses untested; the player
`@teleport` steps were permission refusals) were fixed by the fixer with mutation checks (each
reverted clause fails the new assertions; `lib.rs` restored byte for byte). Conformance's should-fix
(the C7 contract walk) is the owner item above, correctly deferred.

**Builder's pre-fix gate** (every brief command green): mush-sim members 11, lib 35, placement 31,
e7c6 68, custody 11, whole crate 293; mush-atlas 163; mush-economy 533; tacsim hulls 14, ports 17,
content 29 (digest pins unmoved), lane 8, stations 13, lib 862, whole crate 2073;
mush-contract-tests 127 after reverting the Dray walk (the walk itself failed on
`("tacsim.unit.dray/airlock-passage", "lock", "lock")`); mush-server boot_parity 18; mushd 243;
mush-bot 704; `cargo fmt --all -- --check` exit 0; `cargo clippy --workspace --all-targets -- -D
warnings` exit 0.

**Final gate after the fixer, 24 steps, all pass on the first attempt** (logs under the run's
`scratchpad/cuts/IC4/gate1/step01..24.log`): `cargo fmt --all -- --check` → exit 0, no output;
`cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
debuginfo] target(s) in 1.76s``, exit 0; `cargo test -p mush-sim --test members` → `test result: ok.
11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`; `--lib` → ok, 35
passed; `--test placement` → ok, 31 passed; `--test e7c6` → ok, 68 passed; `--test custody` → ok, 11
passed; `cargo test -p mush-sim --no-fail-fast` → 14 binaries ok, 293 passed, 0 failed; `cargo test
-p mush-atlas --no-fail-fast` → 16 binaries ok, 163 passed; `cargo test -p mush-economy
--no-fail-fast` → 13 binaries ok, 533 passed; `cargo test -p tacsim --test hulls` → `test result:
ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.30s`; `--test ports` →
ok, 17 passed; `--test content` → ok, 29 passed; `--test lane` → ok, 8 passed; `--test stations` →
ok, 13 passed; `--lib` → ok, 862 passed; `cargo test -p mush-contract-tests` → 29 binaries ok, 127
passed, 0 failed, 8 ignored (twice, identical); `cargo test -p mush-server --test boot_parity` →
`test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.21s`;
`cargo test -p tacsim --no-fail-fast` → 39 binaries ok, 2074 passed, 0 failed, 12 ignored; `cargo
test -p mushd --no-fail-fast` → 44 binaries ok, 243 passed, 0 failed, 1 ignored (15m35s build);
`cargo test -p mush-bot` → 22 binaries ok, 704 passed, 0 failed, 5 ignored (twice); `cargo test
--workspace --exclude mush-bot --no-fail-fast` → 385 binaries, every one `test result: ok.`, 7240
passed, 0 failed, 92 ignored, exit 0 (37m33s).

**No flake this run:** no `LNK1104`, no hung binary, no rerun. The workspace log's one panic line
(`process_fault.rs:197 WS01-S8 armed world-turn fault`) is `ws01_s8_server_worker` arming its own
fault, and that binary passed; the "running for over 60 seconds" warnings
(`parse_operator_soup_never_panics`, mushd `arena_story` at 69.5 s, `ae2_a_missile_in_flight_…`) all
passed. The IC3 flake (`arena_story`) did not recur.

### Surface for the next cuts

**mush-sim (`mush_sim`, root re-exports)**

- `pub const MAX_SPACE_ROOMS: usize = 16` (`data.rs`, re-exported). `InteriorSpaceRecord.rooms:
  BTreeSet<ObjId>` is the member set; empty on a legacy record (single-room); when non-empty the
  space object is among them. "Multi-room" everywhere (the aboard sentence, `backing_valid`,
  `egress`) means `rooms.len() > 1`.
- `Derived.member_of: BTreeMap<ObjId, ObjId>` — room → the space whose `rooms` lists it; the space
  object maps to itself; `verify_indexes` checks it.
- `Transition::Members { space: ObjId, rooms: BTreeSet<ObjId> }` through `mush_sim::run` / `plan` /
  `apply`; refusals in order: `NoSuchObject(space)` / `NoSuchSpace(space)`, `Members("{space} must
  be a room of its own interior space")`, `Members("an interior space holds at most 16 rooms; {n}
  were named")`, per other room `NoSuchObject(r)` / `Members("{r} is not a room")` / `"{r} is an
  interior space of its own"` / `"{r} hosts domain #{d}"` / `"{r} is the presentation room of domain
  #{d}"` / `"{r} is already a room of interior space {other}"`, then `Members("{r} still holds
  {display_name}; move them first")`; plans `set_spaces` only, audit
  `{"op":"members","space":id,"rooms":[ids]}`. Ids display as `#n`. `Transition::Space` on a member:
  `Members("{space} is a room of interior space {owner}; it cannot be a space itself")`.
  `SpatialError::Members(String)` renders as `"{0}"`. A space is declared as before: attach
  `sim.interior` + `run(Transition::Space)` (the `make_interior` shape); `declare_space` in
  `tacsim::port::hull` is crate-private.
- `pub fn declare_compartment(ctx: &mut Ctx<'_>, space: ObjId, slot: SlotId, maximum_items:
  Option<u32>) -> Result<(), String>` (`window.rs`): writes `compartments[slot] = CapacitySpec {
  maximum_items, ..Default::default() }` and persists; `"That is not an interior space."` without a
  record; `"the simulation module is not composed"` otherwise. `@spatial/slot` calls it.
- `graph::project`: `Interior` → the occupant's current location when the space's `rooms` contains
  it, else the space object.
- The movement gate (`MoveAttempt` hook): journeying → held → quarantined → **member arm** (allowed
  when `exit.is_some()` and `dest` is in the actor's own space's `rooms`; no placement write) →
  by-parent. `CarryAttempt` has no member arm. Multi-room aboard sentence: `"{name} is aboard
  {host}; it moves only between {host}'s rooms, or leaves through the simulation or staff recovery
  (@spatial)."` (host = the `MovingObject` anchor, else the space); single-room keeps `"{name} is
  inside {space}; it leaves only through the simulation, or staff recovery (@spatial)."`. `+ic`
  delivers it as `Sys`.
- Destroy gate on a member: `"{room} is a room of the interior space {space}; @spatial/unspace that
  space first."`. A programmatic destroy of a member drops it from `rooms`, persists the space, and
  moves every `Interior` child of that space whose location disagrees with its projection into the
  space object.
- `InteriorCustodyProvider::backing_valid` = `Some(rooms.len() <= 1 && live && !quarantined)`;
  `@vault/create a=interior:<airlock>` is refused, `interior:<stowage>` accepted.
- Player `egress`/`disembark` inside a multi-room space: `"You cannot egress from {host}; use
  +ship/leave in the {space}."`; staff `@spatial/egress` unchanged.
- Boot: `pub(crate) fn invalid_members(st, w) -> Vec<(ObjId, Vec<(ObjId, &'static str)>)>`; the drop
  is RAM-only, paged with `tracing::error!` (`space`, `dropped`, `runbook`); a dropped room's
  occupant projects to the space and reconcile moves it there. Any later `persist_space` of that
  record (a compartment write, a re-anchor, a destroyed member, a lane re-apply) makes the drop
  durable.
- Narration aboard (`journey.rs`): `onboard_rooms(st, root)` is private; at an intermediate boundary
  every room aboard receives `OutputEvent::Narrative { kind: Emit, actor: None, text: "{root} passes
  {leg.to()}. Leg {target+1} of {total}." }` and at arrival (inside `!already`) `"{root} arrives,
  and the passage ends."`, both via `to_room_from(room, [], root)`. IC9's flavoured lines are
  additional, not replacements.

**mush-economy** — `pub fn is_title_issuer(&self, issuer: &mush_org::SystemPrincipalId) -> bool` on
`EconomyStore`, `#[must_use]`.

**tacsim (`tacsim::port`)**

- `PortMap.hulls: Vec<HullRow>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`);
  `PORT_MAP_SCHEMA` stays 1. `pub struct HullRow { pub n: u32, pub mark: String, pub hull: u32, pub
  rooms: BTreeMap<CanonicalId, u32>, pub exits: Vec<u32> }`, `deny_unknown_fields`, re-exported as
  `tacsim::port::HullRow`. The airlock is `rooms["airlock"]` (`interior.space`), the stowage
  `rooms["stowage"]` (`interior.store`); walkable rooms `airlock`, `passage`, `bridge`, `crew`,
  `hold`, `engineering`; exits pair i at `2i` (names[0], from → to) and `2i+1` (names[1], to →
  from), **append-only** in the template. Rows are in pool order; *Slack Water* is `hulls[0]` with
  the lowest Thing id (IC6's `next_loaner`); the Stickney map has no rows.
- `tacsim::port::hull`: `pub const REGISTRY_ISSUER: &str = "tacsim.registry"`; `pub const
  HULL_DETAIL_PREFIX: &str = "tacsim:detail:"`; `pub const HULL_SPACING_CM: i64 = 10_000`; `pub fn
  registry_mark(prefix: &str, n: u32) -> String` (`FPC-D-0401` for `("FPC-D-04", 0)`); `pub(crate)
  fn preflight(reg, w, pack, id, def, map: Option<&PortMap>) -> Result<usize, String>` (the count of
  hulls this pass would title); `pub(crate) fn lay_up(ctx, id, programme, map: &mut PortMap, domain:
  DomainId, gallery: ObjId) -> Result<(), String>`; `pub(crate) fn hull_line(w, store, entry, def,
  map) -> Option<(LineState, String)>`.
- Each hull Thing: owner = the applying actor (GOD at boot and on `@atlas/pack apply`), name from
  `hull_names`, alias the mark, location C01 by projection, `tactics.unit` bound from
  `tacsim.unit.dray`, placed `Direct` on the Ceres port domain at `(0, n × 10,000 cm)` with the
  passive posture, no description, titled `EconomicAsset { title_owner: Organization(coop_trust
  org), scope: Some(the Trust's root scope) }` under `tacsim.registry`, no catalog entry. Its
  airlock record: `anchor: MovingObject { host: hull }`, `rooms` = the six walkable rooms,
  `compartments["berths"].maximum_items == Some(2)`; the stowage record: the same anchor, `rooms
  {stowage}`. Room text: `desc`, `short_desc` (`mush_atlas::def::SHORT_DESC_ATTR`) and details under
  `tacsim:detail:{fold_detail_keyword(key)}`, re-dressed on every lane apply only.
- Update semantics (L10): a re-apply creates nothing; a grown pool creates only the new hulls; a
  hull with any placement record (held included) is never re-placed; one with any asset record is
  never re-titled; the Thing is never renamed (IC6 renames at enrolment; the mark stays); exits are
  never re-made; refused as drift before any write: a row `n >= pool`, a mark ≠ `registry_mark`, a
  recorded object not live, an unauthored room, more than `2 × pairs` exits, hulls on a
  programme-less entry. A frozen economy refuses with `"{id}: the economy is frozen on this world,
  so {to_make} capital tranche(s) and {h} hull title(s) cannot be issued; nothing is applied while
  it is frozen"` when `h > 0`.
- `preflight` in `PortApplier` now takes `reg: &mush_core::Registry` (both `check` and `put` pass
  `ctx.reg`). `retire_check` still `Ok(())`; hulls stand on retire.
- Readiness: `Binding::Hulls` after the programme line, one per programme port: Missing `"{entry}
  hulls: {k} of {pool} are made."` / `"{mark} (#{obj}) does not exist."` / `"{mark}'s interior is
  incomplete."` / `"{mark} is not titled to {org name}."` / `"{mark} is not laid up at a port."`
  (not `Direct` on a domain any port map records); Ready `"{entry} hulls: {pool} of {pool} made,
  titled to {org name} and laid up."`. IC6 reads the `LineState`; note a leased or under-way hull
  turns it Missing until IC9's arm lands.
- Composition: the title issuers are housing (if on), market (if on), industry, and
  `tacsim.registry` (unconditional). `TacsimRuleset::register` claims attribute namespace `tacsim`
  and detail prefix `tacsim:detail:`.
- Content: `tacsim.pack.dray` and the lane v2 digests are unmoved; `dray.json` is not edited. The
  Airlock exit alias `lock` collides with the cockpit verb LOCK and awaits the owner's ruling before
  `exit_command_words.rs` walks the Dray.

**Test rig patterns later cuts copy**: `crates/mush-sim/tests/members.rs` (the ordered op log
draining `tw.take_module_ops()` and the throwaway `Outbox`'s `Deferred::Persist` ops around direct
`plan`/`apply`/`emit_world_event`; `reboot_with(kv, setup)` recreating walkers before
`modules_loaded`; `fold_kv_ops`, `doc`/`doc_mut`, `listed_rooms`); `crates/tacsim/tests/hulls.rs`
(`Roster::{production, bare}` + `with_shelf`, `ceres_map`, `hull_rooms(row)`, `aboard(tw) -> (pilot,
row)` placing a spawned participant in *Slack Water*'s berths, `placement_writes(ops)`,
`lane_v3`/`plan_and_apply`, the held-document corruption of a folded store before `reboot`,
`destroy(tw, obj)` for a programmatic destroy).

## IC5

**Tacsim's authoritative plumbing and the port tick** (doc 18 §5, the IC5 row and paragraph), built
2026-10-16 on main at 3c9cded5. The shared machinery every later port arm stands on, and no arm
itself. One commit, pushed to origin/main.

### What landed

1. **Tacsim is `mush-inventory`'s third quantity delegate, and the delegation travels with the
   layer** (`crates/tacsim/src/composition/mod.rs`). The body of `server_modules_with` moved into
   the private `compose_server_modules(.., tacsim_delegate: bool) -> (Vec<Box<dyn GameModule>>,
   InventoryHandleCell)`; the delegates list is `["industry", "market"]` plus `crate::NS` when the
   flag is set. `server_modules_with` keeps its exact signature and behaviour (the roster WITHOUT
   the TacSim layer, so the dozens of arena rosters that compose it without the layer are not
   refused by the freeze's `UnknownOwner`, `action_state.rs:815-833`). New:
   `server_modules_with_tacsim` (the same six arguments, returning the roster and the cell),
   `tacsim_module_wired(cfg, game_content, inventory)` with **no test-world flag** (IC3 L15:
   production can never be a test world), and `tacsim_test_world_module_wired`. Three of
   `MAX_STATE_FAMILY_DELEGATES = 4` are now used.
2. **The layer holds the cell** (`crates/tacsim/src/lib.rs`). `TacsimRuleset` gains `inventory:
   Option<InventoryHandleCell>` (`None` in `with_game_content`) and `with_inventory(cell)` —
   composition-root data like `IndustryModule::with_inventory`, a capability whose authority is the
   delegation. `register` gains one line after `ic_return::register`: `port_tick::register(reg,
   self.test_world, self.inventory.as_ref())?`, unconditional like its neighbours. `pub mod
   port_tick` is registered at the crate root per doc 18 §5's lane note.
3. **The production roster composes it** (`crates/mush-server/src/boot.rs`). `game_modules` calls
   `server_modules_with_tacsim` and pushes `tacsim_module_wired(tacsim, tacsim_enabled,
   inventory)`; nothing else in it changed, the content module stays last, and `boot_parity`'s
   roster golden and its `!test_world` fact are unmoved. `crates/mushd/src/boot.rs` is untouched
   (its `mushd_delegates_the_composition_root` tripwire still passes).
4. **The two production typed families, declared as data** (`crates/tacsim/src/port/families.rs`,
   new; re-exported from `tacsim::port`). `tacsim.port_stock`: `StockReceipt { pack, entry, org,
   good, issue, qty, grade, vault, lot, clerk }` keyed by `StockIssueId`, a `CanonicalHash` over
   `{pack, entry, org, good, issue}` in domain `tacsim.stock_issue_id.v1` (version 1, 1024 bytes),
   whose `key_text` is 64 lowercase hexadecimal digits and whose `lot_id` is its first 8 bytes
   big-endian (§4.3 "LotId derived from the issue id"); `check` holds `qty` to
   `1..=MAX_LOT_QUANTITY`, a valid `Grade`, and `lot` equal to its own issue's. `tacsim.ticket`:
   `Ticket { hull, route, route_pack, route_version, unit_definition, manifest, payload_kg, plan,
   order, attempt, burned_at, launched }` keyed by `TicketId { hull, seq }` (`"{hull.0}:{seq}"`,
   §4.4's `"ticket": "5011:9"`, strictly parsed); the ticket carries the whole `PassagePlan {
   route, law, depart_ic, sky_rev, legs: Vec<LegPlan>, total_s, burn_t, crew_days }` and `LegPlan {
   to, body, dist_km, secs, t0, t1 }` of §4.8 (D-18-46), with `check` enforcing
   1..=`mush_sim::MAX_LEGS` contiguous legs, `secs > 0`, `t1 == t0 + secs`, `legs[0].t0 ==
   depart_ic`, `total_s == last.t1 - depart_ic > 0`, `plan.route == route`, `attempt >= 1`, and
   `burn_t <= tank` and `crew_days <= locker` (a plan that could not have come from the burn never
   hydrates). `Manifest { cargo, tank, locker, occupants }` records the hold BEFORE the burn's
   debit; at most 16 goods, none zero, 1..=16 occupants. Every record is `deny_unknown_fields`.
   Both families register unconditionally through one suffix pattern (`{value}.v1`,
   `.decode_json.v1`, `.checks.v1`, `.json.v1`), ids built with `map_err`, never `expect`.
   `PortFamilies` and `PortHandles { families, inventory: Option<InventoryFamilies>, probe }` are
   published in module state by the tick's `register`.
5. **The provider machinery, and the one validator per kind** (`crates/tacsim/src/port_tick.rs`,
   new). `register` follows the inventory provider's interleaved order: the families; in a test
   world the probe family `tacsim.port_probe`; ONE `outcome` validator (`exact_validator` over
   `outcome_descriptors(test_world)`) and those definitions; ONE `action` validator over
   `action_descriptors(probe_outcome)`, whose entries reference the outcome's exact
   `DefinitionRef`; the definitions and, in a test world, the probe provider; `PortHandles`;
   `PortTickState`; the `tacsim.port` tick; the `Connected` hook. The two descriptor lists are where
   IC8's `tacsim_port_stock` and IC9's `tacsim.port.burn` append; in production both are empty.
   Production composes no tacsim action provider: **IC8's `tacsim_port_stock` is tacsim's first
   production provider; IC5 landed the provider machinery** (lead ruling L4).
6. **The `tacsim.port` tick skeleton** (`port_tick.rs`; Coarse, 5 s). The fire returns at once while
   the layer is off (no cursor move, `last_fire` untouched); collects the agents as the union over
   the arms (an id belongs to one arm, a duplicate is logged and the first keeps it); prunes `owed`
   and `warned`; walks the pure campaign `rotation` from the cursor with `visited <= 64` and
   `worked <= 16` checked before each visit; moves the cursor past the last visited agent (a
   deferral never moves it); and takes its RAM slot with `mem::take` for the fire, writing it back
   whole. `Step::{Idle, Worked, Submitted, Deferred, Refused}`; `step` dispatches an exhaustive
   `match` over `enum Arm { Probe }`, where IC8, IC9 and IC11 add variants ahead of the probe.
   `PortFireStats` and `last_fire(&World)` report each fire.
7. **The swap and the turn rule** (`port_tick.rs` `Fire`, `SwapRefused`, `arbitrate`). `Fire::turn`
   refuses `God` (the principal is GOD) and `Gone` (the agent or principal is absent or `GOING`),
   then the pure `arbitrate`: a principal that already acted this fire records the agent as its
   debt if none is recorded and answers `AlreadyActed`; a recorded debt to another agent answers
   `Owed`. `Fire::as_principal` runs ONE closed call with `ctx.actor = principal` and restores it
   (the `mush-npc` and `seed.rs` precedent), counting the principal as acted whether or not the
   call submitted. `settle` drops any debt owed to an agent that acted, idled or was refused, so
   service passes around a principal's due agents in cyclic id order and none waits more than k
   sweeps (lead ruling L8). **Nothing in the tick calls `ctx.submit_authoritative` outside a closure
   passed to `as_principal`**, so GOD never submits.
8. **Reading inventory pools through the delegation** (`port_tick::read_pool`).
   `PoolReadError::{Unwired, Key, Unreadable, Corrupt}`; the read goes only through
   `ctx.action_state(&inventory.pools, &key)`, an absent pool is 0, and every lens refusal
   (`Unavailable`, `UnownedContext`, `ForeignOwner`, `State`) maps to `Unreadable` through the
   exhaustive `pool_read_error`, never to 0 (doc 18 §9 risk 17).
9. **The probe, a test world's only arm** (`port_tick.rs`; lead ruling L3). `ProbeAgent {
   principal, pool, remaining, next_issue }`, `enlist_probe` (refused outside a test world, without
   a probe action, or past 256 agents), `ProbeKey`/`ProbeMark` in the test-only family
   `tacsim.port_probe`, the action `tacsim_port_probe` → outcome `tacsim_port_probe_result`,
   idempotency key `tacsim.port.probe/{agent.0}/{issue}`. The step refuses a gone agent, idles at
   `remaining == 0`, takes its turn, reads the pool (a refused read uses no turn and submits
   nothing), submits inside the swap, advances `next_issue` (checked), and notifies the principal.
   `ProbeProvider` validates exactly `agent`/`issue`/`seen`, offers only to an enlisted principal
   as `SelfActor`, refuses a replayed `(agent, issue)` with `Conflict` at prepare (the replay guard
   IC8's receipt copies), inserts the mark and names the principal the eligible witness. Its one
   `probe_error` helper holds the file's only `.expect`.
10. **The bounded reconnect-notice store** (`port_tick.rs`; `crates/tacsim/src/store.rs`). One
    document per PLAYER at `tacsim:notice:{obj.0}` (`notice_key`, and the strict inverse
    `notice_key_owner` — the numeric field, never `ObjId`'s `#`-prefixed Display), `NoticeQueue {
    schema: 1, who, lines }` of at most 32 `NoticeLine { at, text }` of at most 240 bytes cut on a
    char boundary, newlines flattened, FIFO, no de-duplication. `notify` tells a connected player
    at once over `sys_to_actor(.., "tacsim", ..)` and otherwise `queue_notice`s, which serializes
    before it mutates RAM or writes, covers players only (a Thing principal such as IC8's clerk
    gets a debug trace), and logs a serialization failure instead of panicking. `ReconnectNotices`
    renders `"While you were off watch:"` then the lines in order on `Connected`, whether or not
    the layer is on, then `Del`s the document. `TacsimStore` gains `notices`; `load` gains the
    `tacsim:notice:` arm through `notice_key_owner`, `load_notice_queue` decoding, checking and
    holding the identity rule (`queue.who` is the key's), with any failure a `tracing::error!` that
    drops that character's queue and bars nothing; the table at the head gets its row, and the
    `test_world` docs now say it also registers the probe.
11. **The composition-edge pin** (`crates/mush-server/src/modules.rs`, tests only).
    `ic5_wiring_tacsim_is_the_third_quantity_delegate_on_the_game_roster` composes
    `boot::game_modules` under the default config and asserts exactly `inventory → {industry,
    market, tacsim}`, one-directional, tacsim a stranger to industry and market in both directions.
    `e7c3_wiring_the_quantity_delegation_is_declared_and_narrow` keeps every assertion value; its
    message and doc now say "a roster without the TacSim layer carries exactly the two".
12. **Every test roster that claims production shape composes it**
    (`crates/tacsim/tests/{ports,hulls,lane,stations,definitions}.rs`, `tests/support/mod.rs`
    `production_roster` and `roster_booting`): `server_modules_with_tacsim` plus
    `tacsim_module_wired` or `tacsim_test_world_module_wired`, each roster's `game_content` and
    content switches kept (`g = false` where it used `tacsim_module(cfg)`). None had to be
    reverted; every suite passed unchanged.
13. **Comments re-pointed at the post-IC5 constructors** (`boot.rs` `game_modules` doc and its R4
    paragraph, `config_ext.rs`, `tests/boot_parity.rs`, `composition/mod.rs`
    `server_modules_with`'s doc, `tests/ports.rs` `Roster.test_world`), and `Fire`'s doc now states
    the one-act guarantee is per tacsim fire, not per drain (see Deferred).

**Tests added.** `port/families.rs`: `ic5_stock_issue_id_is_stable_and_names_its_parts` (the golden
`1c6b17275ecaeac34f09a75be41768970921acb2b29b65c0b86dc5acb9c8ff4d` for
`("tacsim.lane.ceres_phobos", "ceres_port", "kasei", "metals", 1)`, each part names the id,
uppercase refused, the lot is the first 8 bytes), `ic5_stock_receipt_wire_is_strict`,
`ic5_ticket_wire_is_strict_and_the_plan_checks` (the slice-shaped ticket 965,623 + 730,645 s / burn
41 / 21 crew-days round-trips; an unknown field refused at the ticket, manifest, plan and leg; all
thirteen `check` refusals the brief lists), `ic5_ticket_id_key_text_is_strict`. `port_tick.rs`:
rotation, cursor, `ic5_agents_sharing_a_principal_take_turns` (a, b, c, a, b, c over six fires; an
idling debtor drops the debt; another principal is never deferred), the foreign-owner map, notice
key round-trip (`#5`, `05`, empty and `-1` refused), newest-32, char-boundary clamp, strict wire.
`crates/tacsim/tests/port_tick.rs` (new, 15 tests on the composition root with the ic0 land path): a
probe lands as its player principal and as a **Thing** principal (IC8's clerk shape; no notice
written for the Thing); two agents in one fire do not void each other; agents sharing a principal
alternate lo, hi, lo with `deferred: 1`; GOD and a destroyed principal are refused twice over;
`seen == 42` through a real `@vault/create` vault and a rooted pool; an unwired layer refuses the
read and submits nothing; the budgets (20 agents: 16/16/16 then 4 of 20 visited; 70 agents: 64
visited 0 worked, then 6 of 64); a disabled layer fires nothing; production registers no probe and
both families under `tacsim`; the replay refused with the provider's own `Conflict` at prepare;
notices survive a kept-world reboot, render once with the heading first and a `Del`, and a real
second `Connected` (after ConnDown + SessionGone, with the room's "has connected." as the positive
control) renders nothing; a connected principal is told live with no document written and exactly
one non-GOD submission; a corrupt document is dropped for that character only; and test 15, 22
agents under a biting budget: `(submitted 16, deferred 1)` each fire, the exact per-fire sets of the
brief's table, every agent marked by the end of fire 2, s1/s2/s1 alternating, no fan-out line.
Every test drains through `drain_tacsim`, which asserts no kept tacsim submission has actor `#1`.

### Deferred, and why

**Lead rulings taken by the planner** (recorded here so the owner sees them):

- **L1 — The delegation travels with the layer.** `mush-inventory` names `tacsim` only on a roster
  that composes the TacSim layer (`server_modules_with_tacsim`, which `boot::game_modules` uses),
  never inside `server_modules_with`: the state-family freeze refuses a delegation to an uncomposed
  owner (`action_state.rs:815-833`) and dozens of arena rosters compose the built-ins without the
  layer. Doc 18's "tacsim joins `with_quantity_delegates`" (row IC5; §4.4) is honoured where it
  matters, in production and every roster that claims production shape.
- **L2 — The cell reaches the layer at composition, not at runtime.** `TacsimRuleset::with_inventory`
  holds the `InventoryHandleCell` and `PortHandles` publishes the families, because IC8's and IC9's
  providers need inventory's families at `register` (§4.2 "families cloned at composition"); a
  runtime read of `InventoryHandles` cannot serve a provider.
- **L3 — The probe is a test-world registration with its own test-only family, not `cfg(test)`,
  and production has no test-world switch.** `cfg(test)` would hide it from integration tests that
  must drive the composition root; a fake receipt or ticket would bend the production wire.
  `tacsim_module_wired` takes no flag; `boot_parity` pins `!test_world`.
- **L4 — "IC5's provider"** (doc 18 §5's IC5 paragraph "Register the provider"; IC8's row "on IC5's
  provider") **is read as the registration machinery**: the one `action` and one `outcome`
  validator built from two descriptor lists, `PortHandles`, the two families and the swap. **No
  tacsim action provider is composed in production after IC5; IC8's `tacsim_port_stock` is
  tacsim's first.** A provider with no operation would be meaningless.
- **L5 — Family ids are doc 18's names verbatim**; the stock key is the issue id's 64 hexadecimal
  digits, the ticket key `{hull.0}:{seq}` per §4.4's orders example, `seq` minted by IC9.
- **L6 — `PassagePlan` and `LegPlan` live in `port/families.rs`**, not §4.8's `passage.rs`: the
  ticket's wire must exist before its writer (D-18-46); IC9's `port/passage.rs` imports them.
  "Where tacsim's definitions live" is read as the `port` module (`port/def.rs`);
  `crate::definitions` is the content-pack registry and holds no typed families.
- **L7 — The ticket's "definition digest" (§4.2) is `unit_definition`**, the hull's fielded
  unit-definition digest whose drive law the burn used; the route is pinned by `route_pack` +
  `route_version` (published versions are immutable), not by this digest. The manifest records
  quantities BEFORE the burn's debit, the plan carries the debit; `order: None` for a live undock;
  `attempt >= 1`. Recorded here so IC9 does not re-open it.
- **L8 — One swapped call per actor per fire, with round-robin debts** (`owed`, the pure
  `arbitrate` + `settle`), replacing the brief's revision-1 "cursor to the first deferred agent",
  which starved agents whenever a budget bit. The campaign cursor rule is kept (past the last
  visited; deferrals never move it).
- **L9 — The cursor, debts, warned set and stats are RAM-only**: a few dozen agents, a restart costs
  one fire, a persisted cursor would be a write every 5 s.
- **L10 — Notices live in `TacsimStore`**, one document per PLAYER, no de-duplication (housing's
  dedup answers a ladder's retries this store does not have), rendered regardless of
  `[tacsim].enabled`, `Del` on render; a non-player principal's line is dropped with a debug trace;
  a corrupt document is dropped loudly and its key bars nothing.
- **L11 — Panic budget +1** for the single `probe_error` helper, the idiom of every provider in the
  tree; `queue_notice` logs instead.
- **L12 — The tick registers unconditionally and returns at once while the layer is off.**

**Builder decisions recorded:** `step` takes `&mut PortTickState` and the `Arm` beyond the brief's
signature (the probe arm mutates its records); the slot is `mem::take`n for the fire and written back
whole (nothing during the fire reads it; `project_offer` reads it at landing, after); `turn` checks
both the agent and the principal for `Gone`; a refused read uses no turn; `Step::Worked` carries
`#[allow(dead_code)]` until IC8's first pull-verifying arm returns it; the integration rig installs
an offer-key ring at boot (the `ic0.rs` rig) because landing fails without one.

**Residuals for IC8/IC9/IC11, named here so they are not lost:**

1. **The one-act ledger is per tacsim fire, not per drain.** `Engine::fire_due` dispatches every due
   `@wait` line as its actor and every other module tick into one outbox before its one
   `drain_deferred` (`engine.rs:3213-3263`), so a principal's own `@wait` command, or another
   module's swapped tick, due in the same slice fans out with an arm's submission and the engine
   voids both ("That command tried to submit more than one authoritative action; nothing was
   staged."). An arm's step recovers by pull (D-18-32); the player's `@wait` command is the
   collateral. Closing it needs a core seam that asks the outbox whether an actor already has a
   deferred submission.
2. **`Step::Submitted` means "deferred to the drain", not "admitted"**: the drain can still drop it
   as back-pressure or void it as fan-out, so a real arm verifies completion by pull on a later fire
   and **notifies only on the pull-verified result, never at `Submitted`** — the probe's "was
   staged." line at deferral is the wrong pattern for a production arm.
3. **Reverse back-pressure**: while a tick-queued submission for a connected pilot sits in the
   64-slot engine queue, the pilot's own next authoritative command is refused with "Your earlier
   action is still committing; try again in a moment." (`engine.rs:4066-4074`); IC9/IC11 decide
   whether to skip connected pilots with a pending submit (`engine.queued_authoritative_actors`) or
   document the message.
4. The engine's best-effort "Queued." line still reaches a connected principal when the tick submits
   for them.
5. A destroyed character's `tacsim:notice:{obj}` document is never dropped (ObjIds are not recycled,
   so it leaks to no one).
6. An agent whose principal stays refused is logged once per boot but stays in its arm's set until
   the arm drops it.
7. IC8's receipt prepare must re-derive or validate `(pack, entry, org, good, issue)` from the clerk
   actor and authoritative state, not trust the invocation's parameters as the probe does.
8. A family validator sees only the value, so `Ticket.hull` cannot be checked against
   `TicketId.hull` nor a receipt's `issue_id()` against its key; IC8/IC9 writers should derive the
   key from the value and re-check on read, and `families.rs:19-21`'s "both gates" sentence
   overclaims by that much.

**Out of this cut, by the brief:** three production-claiming rosters outside item 1.6's list were
left as they were and now differ from `game_modules` by the delegation and the cell —
`crates/tacsim/src/match_/centre.rs` `whole_centre()`,
`crates/mush-contract-tests/tests/legacy_handler_allowlist.rs` `production_modules()`,
`crates/mush-server/tests/e7r2_proving_loop.rs` `boot()`; no assertion is wrong today; migrate or
relabel them in IC8. The arena unit rosters (`cockpit`, `harness`, `match_lifecycle`, `resolution`,
`reset`, `deliberation`, `extensions`) compose no delegation and no cell by design: the probe refuses
and the tick is idle there.

**Test gaps carried forward:** no pin that `game_modules` hands the layer a FILLED cell
(`PortHandles.inventory.is_some()` is asserted on the rig's copy of the roster, not on
`game_modules`); `store::load`'s identity-rule, `check`-failure and bad-key branches are untested
(only the serde failure is); an empty loaded queue is removed from RAM but its document is never
`Del`'d (only a hand-edited document reaches this); the Thing-principal proof covers the rig's
offer/resolve/prepare/install path, not the world task's `command_ingress` and durable preflight —
IC8 should land one Thing-principal tick submission through `CommandSubmitDriver`.

**Owner-critical defaults taken: none.** The cut adds no content, price, name, verb or fiction;
nothing under `docs/design/` was edited by the build (this note is the committer's).

### Pins re-blessed

- **`panic_budget/tacsim.txt` 4 → 5**, blessed with `PANIC_BUDGET_BLESS=1 cargo test -p
  mush-contract-tests --test panic_budget`: the one new production panic site is
  `port_tick::probe_error`'s `.expect` on `ActionError::new` over a static message, the bounded
  idiom of every provider in the tree (`mush-sim/src/handoff.rs:228-241`). Nothing else panics:
  `queue_notice` maps a serialization failure to `tracing::error!`, every descriptor id is built
  with `map_err`, every arithmetic step is checked, and the walk uses `saturating_add`. The bless
  also lowered `panic_budget/mush-tactics.txt` 54 → 50 (pre-existing slack, not this cut's); that
  file was reverted and the gate still passes with mush-tactics at 54.
- **The composition-edge pin** (`mush-server/src/modules.rs`):
  `e7c3_wiring_the_quantity_delegation_is_declared_and_narrow` keeps every asserted value (the
  `server_modules_with` roster still has exactly the two delegations); only its message moved from
  "exactly two delegations in the whole production composition, both from `inventory` outward" to
  "a roster without the TacSim layer carries exactly the two delegations, both from `inventory`
  outward", because the production composition now carries three — pinned by the new
  `ic5_wiring_tacsim_is_the_third_quantity_delegate_on_the_game_roster`.
- **Unmoved, and shown unmoved by the full `mush-contract-tests` run (29 binaries ok),
  `boot_parity` (18 ok) and `mushd --test boot_composition` (6 ok):** `boot_parity`'s roster golden
  and its `!test_world` fact; the legacy-handler baseline (97; no verb); `module-ns-owners.toml`
  (reads of delegated families need no persist delegation); `state-slot-owners.toml`
  (`enlist_probe`'s and `queue_notice`'s slot reaches are inside the owning crate); the
  restriction-lints census (`dead_code` is not a denied tacsim lint); the typed-state-lens pin;
  `ci_gated_test_inventory` (no `#[ignore]` or env gate added); `provider_rng_policy` (no RNG in
  `port_tick.rs`); `action_state_republish` (no `install_prepared_action(` in `src/`);
  `hex_vocabulary`; every content digest pin (`tacsim/tests/content.rs` 29 ok; no atlas content
  touched); the `mushd_delegates_the_composition_root` tripwire (no new name contains a forbidden
  needle).

### How it was verified

**The gate runner's sixteen steps**, on Windows at the working tree over base 3c9cded5, all green on
the first attempt, no reruns, no `LNK1104`:

- `cargo fmt --all -- --check` → exit 0, no output (run twice, before and after the fixer's pass).
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s)``, exit 0, 0 warnings (twice).
- `cargo test -p tacsim --lib` → `test result: ok. 874 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 3.86s`.
- `cargo test -p tacsim --test port_tick` → `test result: ok. 15 passed; 0 failed; 0 ignored; 0
  measured; 0 filtered out; finished in 1.89s`.
- `cargo test -p tacsim --test ports --test hulls --test lane --test stations --test definitions
  --test composition_edges --test content` → composition_edges 2, content 29, definitions 25, hulls
  15, lane 8, ports 17, stations 13, each `test result: ok … 0 failed`.
- `cargo test -p tacsim --test match_lifecycle --test cockpit` → cockpit `ok. 180 passed; 0 failed
  … 34.76s`; match_lifecycle `ok. 136 passed; 0 failed … 26.34s` (both compose the migrated
  `support/mod.rs` rosters).
- `cargo test -p mush-server --lib modules::` → `test result: ok. 31 passed; 0 failed; 0 ignored; 0
  measured; 263 filtered out; finished in 0.79s` (includes `e7c3_wiring_…` and `ic5_wiring_…`).
- `cargo test -p mush-server --test boot_parity --test market_issuer` → boot_parity `ok. 18 passed;
  0 failed … 2.19s`; market_issuer `ok. 4 passed; 0 failed … 0.60s`.
- `cargo test -p mush-server --test e7r2_proving_loop` → `test result: ok. 6 passed; 0 failed …
  0.27s`.
- `cargo test -p mush-contract-tests` → 29 result lines, all ok, 0 failed (run twice; the
  panic-budget binary `ok. 7 passed` with tacsim at 5 and mush-tactics at 54).
- `cargo test -p mushd --test boot_composition` → `test result: ok. 6 passed; 0 failed … 2.07s`.
- `cargo test --workspace --exclude mush-bot --no-fail-fast` → exit 0, 386 `test result:` lines all
  ok, aggregate passed=7268 failed=0 ignored=92; among them mush_server lib `ok. 292 passed; 0
  failed; 2 ignored`, boot_parity 18, boot_composition 6, tacsim lib 874, hulls 15, port_tick 15,
  ports 17.
- `cargo test -p mush-bot` → exit 0, 22 result lines all ok, aggregate passed=704 failed=0
  ignored=5.

The builder's first run failed only in `panic_budget` with "tacsim: 5 production panic sites, budget
4 (+1)", the expected raise, then blessed. **Environment note:** the workspace step sat about 24
minutes in `Compiling mush-gateway` with ~29 `link.exe` processes paging a 64 GB box to 1–3 MB free;
it finished in 69 minutes inside the 90-minute timeout. This is linker memory pressure from cargo's
default parallel link jobs, not the cut. No flake was identified in any step.

**Three review lenses** (correctness, conformance, tests) found no must-fix defect; their nine
should-fix findings reduced to five distinct issues (the per-drain overclaim in `Fire`'s doc, test
11 accepting any landing failure, test 13 not draining, test 12's second connect never running the
hook, stale constructor comments), all fixed and re-run green, each verified in the tree by the
judge. The judge also re-traced test 15's three-fire walk by hand against
`arbitrate`/`settle_debts`/`next_cursor` (it matches the brief's table), confirmed the only non-test
panic site added is `probe_error`'s, confirmed the store load arm cannot be captured by an earlier
prefix, and confirmed `docs/design/` and `crates/mushd/` are untouched. Not run separately:
`crates/mush-atlas/tests/starter.rs` (no atlas content touched; it ran inside the workspace sweep).

### Surface for the next cuts

**Composition (`tacsim::composition`, glob re-exported by `mush_server::modules`)**

- `pub fn server_modules_with(weather, economy, atlas, onboarding: &OnboardingConfig, game:
  &GameEconomyConfig, recovery) -> Vec<Box<dyn GameModule>>` — unchanged; delegates `[industry,
  market]`; the roster WITHOUT the TacSim layer.
- `pub fn server_modules_with_tacsim(same six) -> (Vec<Box<dyn GameModule>>,
  mush_inventory::InventoryHandleCell)` — delegates `[industry, market, tacsim]`; the production
  root's spelling; the cell is filled by `mush-inventory`'s own `register`, so any module composed
  after it finds it filled.
- `pub fn tacsim_module_wired(cfg: TacsimConfig, game_content: bool, inventory:
  InventoryHandleCell) -> Box<dyn GameModule>` — no test-world flag.
- `pub fn tacsim_test_world_module_wired(cfg, game_content, inventory) -> Box<dyn GameModule>` —
  also registers the probe.
- `TacsimRuleset::with_inventory(self, InventoryHandleCell) -> TacsimRuleset`.
- `tacsim_module`, `tacsim_module_with`, `tacsim_test_world_module` still exist for rosters that
  compose no delegation (the probe then refuses and the tick is idle).

**`tacsim::port::families` (every name re-exported from `tacsim::port`)**

- `PORT_STOCK_FAMILY = "tacsim.port_stock"`, `TICKET_FAMILY = "tacsim.ticket"`, `MAX_MANIFEST_GOODS:
  usize = 16`, `MAX_TICKET_OCCUPANTS: u32 = 16`.
- `LegPlan { to: CanonicalId, body: CanonicalId, dist_km: u64, secs: u32, t0: i64, t1: i64 }`.
- `PassagePlan { route: CanonicalId, law: CanonicalId, depart_ic: i64, sky_rev: DefinitionDigest,
  legs: Vec<LegPlan>, total_s: i64, burn_t: u64, crew_days: u32 }`; `check()`: 1..=`mush_sim::MAX_LEGS`
  (32) legs, `legs[0].t0 == depart_ic`, every `secs > 0` and `t1 == t0 + secs`, contiguous,
  `total_s == last.t1 - depart_ic > 0`.
- `Manifest { cargo: BTreeMap<CanonicalId, u64>, tank: u64, locker: u64, occupants: u32 }`;
  `check()`: ≤ 16 goods, no zero quantity, 1..=16 occupants. Quantities are BEFORE the burn's
  debit.
- `Ticket { hull: ObjId, route: CanonicalId, route_pack: CanonicalId, route_version: u32,
  unit_definition: DefinitionDigest, manifest: Manifest, payload_kg: u64, plan: PassagePlan, order:
  Option<u32>, attempt: u32, burned_at: i64, launched: bool }`; `check()`: plan and manifest checks,
  `plan.route == route`, `attempt >= 1`, `plan.burn_t <= manifest.tank`, `u64::from(plan.crew_days)
  <= manifest.locker`. `unit_definition` is the hull's fielded unit-definition digest (L7); the
  route is pinned by `route_pack` + `route_version`; `order: None` for a live undock.
- `TicketId { hull: ObjId, seq: u32 }` (Copy, Ord, Hash): `key_text() -> String` =
  `"{hull.0}:{seq}"`; `parse(&str) -> Option<TicketId>` strict (one `:`, unsigned decimal, no
  leading zero but `0`, each side fits `u32`). `seq` is IC9's to mint.
- `StockIssueId(CanonicalHash)` (Copy, Ord, Hash; no serde): `derive(pack, entry, org, good:
  &CanonicalId, issue: u32) -> Result<StockIssueId, String>` over domain `tacsim.stock_issue_id.v1`,
  version 1, 1024 bytes; `key_text() -> String` (64 lowercase hexadecimal digits); `parse(&str) ->
  Option<StockIssueId>` strict; `lot_id() -> mush_inventory::record::LotId` (first 8 bytes,
  big-endian). **Golden pin later cuts must not move:** `("tacsim.lane.ceres_phobos", "ceres_port",
  "kasei", "metals", 1)` → `1c6b17275ecaeac34f09a75be41768970921acb2b29b65c0b86dc5acb9c8ff4d`.
- `StockReceipt { pack, entry, org, good: CanonicalId, issue: u32, qty: u64, grade: u8, vault:
  mush_economy::VaultId, lot: u64, clerk: ObjId }`; `issue_id() -> Result<StockIssueId, String>`;
  `check()`: `qty` in `1..=MAX_LOT_QUANTITY`, `Grade::new(grade)` ok, `lot ==
  issue_id().lot_id().0`.
- Every record derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize` with
  `#[serde(deny_unknown_fields)]`.
- `PortFamilies { port_stock: StateFamilyHandle<StockIssueId, StockReceipt>, tickets:
  StateFamilyHandle<TicketId, Ticket> }` (Clone, Debug).
- `PortHandles { families: PortFamilies, inventory: Option<mush_inventory::InventoryFamilies>,
  probe: Option<StateFamilyHandle<ProbeKey, ProbeMark>> }` (Clone, Debug), published in module
  state by `port_tick::register`; read with `w.module_state::<PortHandles>()`.
- Crate-private helpers IC8/IC9 reuse: `families::family_descriptor(family, key_schema,
  value_stem) -> Result<StateFamilyDescriptor, ModError>` (the suffix pattern `{stem}.v1` /
  `.decode_json.v1` / `.checks.v1` / `.json.v1`, version 1), `families::strict_decimal_u32(&str) ->
  Option<u32>`, `families::canonical(&str) -> Result<CanonicalId, ModError>`,
  `families::register(reg) -> Result<PortFamilies, ModError>`.
- Descriptor ids: `tacsim.port_stock` key schema `tacsim.stock_issue_id.v1`, value
  `tacsim.port_stock.v1`; `tacsim.ticket` key `tacsim.ticket_id.v1`, value `tacsim.ticket.v1`;
  `tacsim.port_probe` (test world only) key `tacsim.port_probe_key.v1`, value
  `tacsim.port_probe.v1`.

**`tacsim::port_tick`**

- `PORT_TICK = "tacsim.port"`, `PORT_TICK_PERIOD = Duration::from_secs(5)` (Coarse),
  `MAX_AGENTS_VISITED_PER_FIRE: usize = 64`, `MAX_AGENTS_WORKED_PER_FIRE: usize = 16`,
  `MAX_PROBE_AGENTS: usize = 256`.
- `pub fn register(reg: &mut Registrar<'_>, test_world: bool, inventory:
  Option<&InventoryHandleCell>) -> Result<(), ModError>`. **Where IC8/IC9 extend:** the private `fn
  outcome_descriptors(test_world: bool) -> Result<Vec<DefinitionDescriptor>, ModError>` and `fn
  action_descriptors(probe_outcome: Option<&DefinitionRef>) -> Result<Vec<DefinitionDescriptor>,
  ModError>` are the two lists; registered refs are collected by id into `BTreeMap<String,
  DefinitionRef>` after each `reg.definition`, and a provider is registered with
  `reg.action_provider(action_ref, provider)` after both validators. There is exactly one
  `definition_validator` per kind for the crate; a second call is a registration conflict. **Where
  arms extend:** `enum Arm { Probe }` (add variants ahead of `Probe`), `fn step(ctx, &mut
  PortTickState, &mut Fire, agent, principal, Arm) -> Step`, and the agent collection in `walk`
  (`offer(agent, principal, arm)`; an agent id belongs to one arm, a duplicate is logged and the
  first keeps it).
- `pub struct PortTickState` (RAM only, private fields: cursor, last_fire, owed, warned,
  probe_action, probes; `Default`).
- `pub struct PortFireStats { visited, worked, submitted, deferred, refused: u32 }` (Copy, Default,
  Eq); `pub fn last_fire(&World) -> Option<PortFireStats>` (`None` before the first fire that found
  the layer on).
- Crate-private, for the arms: `pub(crate) enum Step { Idle, Worked, Submitted, Deferred, Refused }`
  (`worked` counts `Worked` and `Submitted`); `pub(crate) struct Fire` with `turn(&mut self,
  &World, agent, principal) -> Result<(), SwapRefused>`, `as_principal<R>(&mut self, &mut
  Ctx<'_>, agent, principal, impl FnOnce(&mut Ctx<'_>) -> R) -> Result<R, SwapRefused>` (the ONLY
  place the tick acts as anyone; every `submit_authoritative` goes inside its closure),
  `settle(&mut self, agent, Step)`; `pub(crate) enum SwapRefused { God, Gone, AlreadyActed, Owed
  }`; `pub(crate) fn arbitrate(&BTreeSet<ObjId>, &mut BTreeMap<ObjId, ObjId>, agent, principal) ->
  Result<(), SwapRefused>`. A `Submitted` step is deferred to the drain, not admitted: verify by
  pull on a later fire and notify only then.
- `pub enum PoolReadError { Unwired, Key(String), Unreadable(String), Corrupt }`; `pub fn
  read_pool(&Ctx<'_>, mush_inventory::record::EndpointRef, &CanonicalId) -> Result<u64,
  PoolReadError>` — only through `ctx.action_state(&inventory.pools, &key)`; an absent pool is
  `Ok(0)`; never 0 on a refusal.
- Probe (test world only): `pub struct ProbeAgent { principal: ObjId, pool: Option<(EndpointRef,
  CanonicalId)>, remaining: u32, next_issue: u32 }`; `pub fn enlist_probe(&mut World, agent: ObjId,
  ProbeAgent) -> Result<(), &'static str>`; `pub struct ProbeKey { agent: ObjId, issue: u32 }` with
  `key_text()` = `"{agent.0}:{issue}"`; `pub struct ProbeMark { agent, principal: ObjId, issue:
  u32, seen: u64 }` with `check()`; `PROBE_ACTION = "tacsim_port_probe"` (payload
  `{"subject":"self_actor","targets":0,"family":"tacsim.port_probe","test_only":true}`, references
  `[probe_outcome]`), `PROBE_OUTCOME = "tacsim_port_probe_result"` (payload
  `{"fields":["actor","agent","issue","seen"]}`), `PROBE_FAMILY = "tacsim.port_probe"`; idempotency
  key `tacsim.port.probe/{agent.0}/{issue}`; notice text `"Probe {issue} for #{agent.0} was
  staged."`.
- Reconnect notices: `NOTICE_PREFIX = "tacsim:notice:"`, `NOTICE_SCHEMA: u16 = 1`,
  `MAX_NOTICE_LINES: usize = 32`, `MAX_NOTICE_BYTES: usize = 240`, `NOTICE_HEADING = "While you were
  off watch:"`; `pub fn notice_key(ObjId) -> SmolStr` (`"tacsim:notice:{who.0}"`) and `pub fn
  notice_key_owner(&str) -> Option<ObjId>` (strict decimal; `#`, leading zero, sign refused); `pub
  struct NoticeLine { at: i64, text: String }`; `pub struct NoticeQueue { schema: u16, who: ObjId,
  lines: Vec<NoticeLine> }` with `new(who)`, `push(at, &str)` (newlines flattened, clamped on a char
  boundary, FIFO) and `check()`; `pub fn notify(&mut Ctx<'_>, who: ObjId, text: &str)` (live if
  connected, else queued); `pub fn queue_notice(&mut Ctx<'_>, who, text)` (PLAYERS ONLY, not GOING;
  writes the whole document through `store::push_op` as `JsonSet` at `notice_key(who)`; no
  `.expect`); `pub fn pending_notices(&World, who) -> Vec<String>`; `pub struct ReconnectNotices`
  (`EventHook`, `connected` only; heading then lines over `sys_to_actor(who, "tacsim", ..)`, then
  `Del`).
- `TacsimStore.notices: pub(crate) BTreeMap<ObjId, NoticeQueue>`, hydrated by `store::load`'s
  `tacsim:notice:` arm (decode, `check`, identity rule `queue.who == key id`; any failure dropped
  loudly for that character; the key bars nothing).

**Pins and facts later cuts depend on:** the production roster carries exactly `inventory →
{industry, market, tacsim}` (`ic5_wiring_tacsim_is_the_third_quantity_delegate_on_the_game_roster`);
`server_modules_with` alone still carries two (`e7c3_wiring_…`); production registers no
`tacsim_port_probe` definition and no probe family; `panic_budget/tacsim.txt` is 5; the kernel
accepted a Thing principal on the rig's offer/resolve/prepare/install path
(`ic5_a_thing_principal_lands`), the world-task ingress not yet exercised.

**Test rig patterns later cuts copy:** `crates/tacsim/tests/port_tick.rs` (`wired`, `unwired`, `boot`
with the offer-key ring, `offline_player`, `fire`, `drain_tacsim`, `land_one`, `mark`,
`disconnect`) is the shape IC8's and IC9's tests copy.

## IC6

**The Co-op programme and boarding** (doc 18 §5, the IC6 row and paragraph), built 2026-10-16 on
main at 10b89d58. A pilot becomes a First Captain in one command, the live lease is the operate
gate, and the chartered pilot boards, leaves and reads the hull's hold. One commit, pushed to
origin/main.

### What landed

1. **Live leases are never "stale"** (`crates/mush-economy/src/contract.rs`, `src/contract_live.rs`,
   `src/sweep.rs`, `src/lib.rs`). One predicate, `pub fn is_lease_shaped(c) -> bool` beside
   `contract_encumbrance` (issuer collateral plus an Obligation), re-exported in alphabetical place;
   the private `owns_lease` is now that shape plus its two key-specific checks (this lessor, these
   premises), behaviour unchanged; `page_stale_contracts` gains one filter after the state filter,
   `!(Active && is_lease_shaped)`, with the doc 18 §4.1 comment. Housing and recovery leases are
   exempted by the same line. A `ReadyToSettle` or escrowed `Draft` lease still pages.
2. **The enrolment document and the goods cache** (`crates/tacsim/src/store.rs`, `src/lib.rs`).
   `TacsimStore` gains `enrolments: BTreeMap<AccountId, Enrolment>`, `enrolment_keys:
   BTreeSet<SmolStr>` (every `tacsim:coop:` key seen, readable or not) and `goods:
   Option<Arc<GoodsTable>>`; accessors `enrolment_state(account) -> EnrolmentState` (Present /
   Unreadable when the key is claimed with no body / Absent), `enrolment_of(pilot)`,
   `enrolment_for_lease(u64)`, `enrolments()`, `goods()`; `pub(crate) put_enrolment` as the single
   write path on `put_port_map`'s pattern (RAM install, one whole-document `JsonSet`, a minimal
   `{schema, account}` body on the structurally unreachable serialize failure so the key still bars
   re-enrolment); a `tacsim:coop:` load arm (strict decode, `Enrolment::check`, the identity rule
   `enrolment_key(e.account) == key`; any failure skipped loudly, the key kept); the module-doc
   table row. The goods table is parsed **once** in `TacsimRuleset::with_game_content` into the new
   private field `goods`, handed to the slot in `register` and again in `on_world_loaded` next to
   `test_world` (the first review's must-fix F1), never re-parsed on a look or a verb.
3. **The enabling edits** (`crates/tacsim/src/port/{apply,hull,readiness,def}.rs`). `credit_scale`,
   `minor_of`, `template`, `berths_slot` are `pub(crate)`; `hull_spec(store, programme) ->
   Option<(&InteriorDef, &LogisticsDef)>`; `report_ports(store, w) -> (Option<&RulesPack>,
   Vec<(CanonicalId, &PortDef)>)` factored out of `readiness` byte-identically (`ports.rs` test 8's
   report did not move); `PortDef::check` refuses `per_account != 1` ("per_account must be 1; this
   server keeps one enrolment per gateway account (D-18-25)"); the shipped lane says 1, so no digest
   moved.
4. **`port/coop.rs` (new): the programme's records and rules.** `EnrolmentStatus { Active,
   BuyoutPending, Graduated, Revoked }` (snake_case), the strict `Enrolment` record
   (`deny_unknown_fields`, the three IC10 counters `#[serde(default)]`, `check()`),
   `EnrolmentState<'_>`, `enrolment_key(account) = "tacsim:coop:{canonical_hash}"`,
   `stake_key(lease) = tacsim.backer.stake/<id>`, `ProgrammePort { pack, entry, def, programme, map
   }` with `trust()`, `trust_account()`, `trust_scope()`, `programme_port(w)` through
   `report_ports`, `port_name(def)`, `trust_lease_on(w, trust, hull)` (`live_lease_on`'s shape on
   `is_lease_shaped`), `Charter { contract, hull }` and **`live_charter(w, pilot)`, the operate gate
   (D-18-23): an Active Trust lease naming the pilot and pledging a programme hull, lowest id, never
   the document**; `hull_is_free` (`next_loaner`'s rule plus docked at the programme port and no
   vault on the stowage), `free_hulls`, `enrol_gate` (Lane, Rate, Map, Rooms, Domain, Org,
   Programme, Hulls must be Ready), `check_hull_name` (trim/collapse, 1..=32 bytes, the charset, a
   leading letter, not any programme hull's current name or mark, ASCII case-insensitive).
5. **The readiness hulls line's leased and under-way arms, and the leased retire refusal**
   (`crates/tacsim/src/port/hull.rs`, `port/apply.rs`). `hull_line` counts `free`, `leased` (with
   each tenant's `display_name`, `nobody` for a lease with no object tenant) and `under_way`; the
   Ready text is **unchanged** when nothing is leased or under way, else `"{entry} hulls: {pool} of
   {pool} made and titled to {org}; {free} laid up and free, {k} leased ({mark} to {tenant}, …)[,
   {u} under way]."`, still Ready. `retire_check` calls the new `retire_refusal`: any hull row with
   a `trust_lease_on` from any of the map's org rows refuses `"{id}: hull {mark} is chartered to a
   pilot; a port with a leased hull cannot be retired"` (`leased_refusal`, unit-pinned).
6. **`+coop`, `+coop/enrol`, `+coop/statement`** (`crates/tacsim/src/port/coop.rs`,
   `src/commands.rs`). `+coop` anywhere: the registrar's line, the Terms line (live
   stake/share/buyout, the 120,000.00cr new-Dray comparator), `Drays free: {free} of {pool}.`, then
   the charter line **with the live lease first**: the enrolment for that lease (`Your charter: the
   {name} ({mark}); buyout {frozen}. +coop/statement reads your lay.`), `CHARTER_BOOKS_DARK` when
   the lease stands but the document does not read, else by gateway account through the private
   `account_books` (`ENROLLED_ONCE` for an alt on an enrolled account; `… has ended.` when Revoked;
   the new `CHARTER_NOT_IN_FORCE` for an Active document with no live lease, e.g. disputed;
   `BOOKS_UNREADABLE`; or the `Enrol at the {office} in {port}: …` invitation). `+coop/enrol <name>`
   at the office only: every check in the brief's order before the first write (usage, office,
   gateway account fail-closed, one enrolment ever by document and by live lease, the readiness
   gate, the name, the free hull, the capital at `available − stake ≥ floor`, **a frozen economy
   refused with FAILED before any write**, every resolution), then account if absent → vault id →
   `offer_lease` (name `First Captain charter, {mark}`, the Trust's root scope, rent 1 minor Manual
   `next_due = i64::MAX`) → `accept_contract` as the pilot and a re-read requiring `Active` → **the
   stake post last** (org actor, `tacsim.backer.stake/<lease>`, two zero-sum lines) → `create_vault`
   `{mark} hold` on `SpatialInterior(stowage)` → `set_name` and the `desc` under the hull's own
   owner when absent → `put_enrolment`. A refusal after the offer ends the lease through
   `end_lease`; **every FAILED past the first write pins the deferred queue**
   (`failed_after_writes`) so the residue L16 names is the same in RAM and on disk. The reply:
   `Ceres Clearing opens account #N in your name.` (when opened here), `The Co-op charters you
   {mark}, laid up as the {laid_up}, now the {name}, cradled at the {gallery}.`, `{stake} staked to
   you. Buyout {buyout}. Ilse Oduya-Brandt countersigns.` `+coop/statement` anywhere: five lines
   from the frozen terms and counters; the Cash line is a whole sentence on each branch; `Charter:
   in force.` only when `live_charter` names the enrolment's lease, else `ended.` (Revoked) or `not
   in force.` Both verbs registered after the `+credit` bracket under `LegacyPostImage` with
   `COOP_HELP`.
7. **`CoopObserver`** (`crates/tacsim/src/port/coop.rs`, `src/composition/mod.rs`): the exhaustive
   `MarketObserver` match; on `ContractChanged { Settled | Cancelled | Expired }` an `Active`
   enrolment for that lease is written back `Revoked` through `put_enrolment` and the pilot told
   `The Co-op has ended your charter of the {name} ({mark}).` through `port_tick::notify`; wired
   into `econ_observers()` after the market's; a no-op on rosters without the layer.
8. **Pool reads from a hook** (`crates/tacsim/src/port_tick.rs`): `read_pool`'s tail factored into
   the private `pool_value`; new `pub fn read_pool_in(&HookReadContext, EndpointRef, &CanonicalId)
   -> Result<u64, PoolReadError>` with the same rule (absent = 0, a refusal never 0).
9. **`port/gauges.rs` (new): the pure renders** over `capacity::load`/`max_more` with the exact
   item-9 texts: `compartment_words` (cargo/HOLD, tank/TANKAGE, locker/STORES), `room_instrument`
   (Load cells, Manifest slate, Displays, Locker tally, Tank gauges, else Gauges), `measures_line`
   (`100/100 t · 120.0/120 m³[ · FULL: mass and packed volume]`), `goods_list`, `single_summary`
   (`42/80 t`, `21/120 crew-days`), `ship_hold_line` (doc 18 §2 step 5's `Hold: cargo 0/100 t ·
   0.0/120 m³ · tank 0/80 t · locker 0/120 crew-days.`), `room_lines` (the load-cell body with every
   accepted good, zeros included, TOTAL/LIMIT/FULL; the single-good sentence; the summary rooms),
   `hold_lines` (`+ship/hold`), `DARK`, the `Reading` alias. FULL names every limited measure whose
   load ≥ its limit.
10. **`port/ship.rs` (new): `+ship`, `/hold`, `/board`, `/leave` and the look hook.** `HullView`,
    `hull_view`, `hull_of_room`, `Berth::{Docked, UnderWay, Elsewhere}`, `berth_of`, `hold_vault(w,
    &HullView)`, `status_line` (18a §3's form with the mark: `Aboard the {name} ({mark}), docked at
    {port}.` / `under way.` / `.`). `+ship` prints doc 18 §2 step 5's form `{name} (Dray): docked
    {port}, {gallery}.` (`SHIP_DOCKED`/`SHIP_UNDER_WAY`/`SHIP_ELSEWHERE`), the hold line
    (`HOLD_DARK` on a read error) and `Programme: {n} sale{s}; buyout {frozen}.` (`SHIP_BOOKS_DARK`
    when the document does not read while the lease stands). `+ship/board` (already aboard → at a
    gallery → the live lease → not under way → cradled at THIS gallery → `place_interior(airlock,
    berths)`), with `You cycle the lock into the {name}.`, the gallery's and the Airlock's
    third-person lines, `show_room` and `Moved`. `+ship/leave` (aboard a programme hull's airlock
    space → standing in the Airlock, else `You can only leave by the {airlock}.` → docked, else
    `UNDER_WAY_LOCK`/`LEAVE_SEALED` → `unplace(keep)` + `move_to(gallery)`, the three lines,
    `show_room`, `Moved`, and **the gallery's port stamped through `ic_return::stamp`**);
    placement-gated, not lease-gated (L7). `ShipLook` on `LookRendered`: nothing while the store is
    absent or the layer disabled; else the status line, then the room's `interior.gauges`
    compartments rendered from the hold vault's pools through `read_pool_in` (no vault → zeros; an
    error → `{Instrument}: dark.`). `register_hooks` called from `register` after
    `port_tick::register`.
11. **The gates** (`crates/mush-contract-tests/legacy-handlers.toml`, `state-slot-owners.toml`): the
    `+coop` (`tacsim.coop`) and `+ship` (`tacsim.ship`) `legacy-post-image` stanzas in byte order,
    `PINNED_BASELINE` 97; the `port/coop.rs` `EconomyStore` row (owner `economy`) and the
    `tests/coop.rs` `AccountOracle` row (owner `sim`).
12. **Tests** (`crates/tacsim/tests/coop.rs`, 16; `crates/mush-economy/tests/e4b1.rs`, 1; unit tests
    in `contract.rs`, `apply.rs`, `def.rs`, `coop.rs` ×5, `gauges.rs` ×6). The rig composes
    `server_modules_with_tacsim` + the wired test-world layer (`Roster::bare`, `bare_real`,
    `.disabled()`, `.freezable()` with a test-only `Latch` module on the e4c/e7c9 precedent), a
    `MapOracle` into `mush_sim::AccountOracle`, `seed_pool` with the lot id as a parameter and the
    typed post-images replayed through `Engine::hydrate_action_state` across a reboot, and asserts
    no refused namespace writes at every boot. The sixteen prove: lens 1 in full (reply, `+credit`,
    the lease's parties/deliverables/rent schedule, `Encumbrance::Lease`, the stake transaction and
    both balances, the vault, the hull's name/alias/desc under its own owner, every enrolment field,
    the four `+coop` lines, the five statement lines, the exact hulls line); one enrolment per
    account including an alt and the alt's `+coop`; the refusals before any write; the rate-4
    closure on the production layer; the ninth pilot with the Trust at exactly its floor; the
    capital floor; board/leave with both sides' narration, the Stickney re-placement and a non-port
    domain; the leave stamp isolated in both directions plus the door pass; no stale page after
    eight days; staff cancel → Revoked, the notice, the gate, `Charter: ended.`, the closed-account
    Cash line; the look status line exactly once and first in every room, the laid-up hull's zero
    block, and silence after a config-off reboot; the gauges line for line after boot; the charter,
    name, board and load cells after a reboot, then an Unreadable document (`+ship` dark programme
    line, board still works, `CHARTER_BOOKS_DARK`, `BOOKS_UNREADABLE` twice, another account
    enrols); the leased retire refusal through `@atlas/pack retire` with an unleased control; a
    frozen economy refused before any write and the thaw; a disputed lease not in force in `+coop`,
    the statement and the gate.

### Deferred, and why

**Lead rulings taken by the planner** (L1–L21 of the IC6 brief, recorded here so the owner sees
them):

- **L1 — The readiness gate**: `+coop/enrol` passes when every Lane, Rate, Map, Rooms, Domain, Org,
  Programme and Hulls line is Ready (§4.7 "orgs capitalized", so all nine org lines); Sky and
  Treasury are not gated; IC8 adds pools and listings.
- **L2 — The hulls line reads "the programme was made"**: a leased hull counts as leased, an
  under-way one as under way, neither Missing; the Ready text is unchanged while nothing is leased
  or under way (`ports.rs` test 8 did not move). The under-way arm is code-reviewed here and first
  exercised by IC9.
- **L3 — Rent 1 minor on the Manual clock, `next_due = i64::MAX`** (D-18-21; Q-18-27's answer). The
  code admits a zero rent: `offer_lease` runs only `check_structure` + `check_offerable`, and
  `create_lease_obligations` writes the obligation directly; so the 1-minor rent and Q-18-27's
  optional-rent change are belt-and-braces and the owner may drop the latter.
- **L4 — The hold vault is opened at enrolment** on `CustodyRef::SpatialInterior(stowage)`, owner
  `Object(pilot)`, named `{mark} hold`.
- **L5 — One enrolment per gateway account, ever, keyed by the account**
  (`tacsim:coop:{canonical_hash}`, the tutorial precedent); an unreadable document bars only its own
  account; `per_account` must be 1.
- **L6 — Enrolment order and atomicity**: every check first; account → vault id → offer → accept →
  post last → vault record, rename, document; a refusal after the offer ends the lease through
  `end_lease`; past the post nothing is fallible.
- **L7 — `+ship/leave` is gated on placement only** (aboard, in the Airlock, docked), not on the
  live lease, so a revoked tenant or a staff-placed occupant is never stranded (IC4's `egress`
  refusal points here). **An owner item**; see below.
- **L8 — The operate gate is `live_charter`** (D-18-23): an Active Trust lease naming the pilot and
  pledging a programme hull; never the document. `+ship`, `+ship/hold`, `+ship/board`, `+coop`'s
  charter line and the statement's "in force" use it; the free-hull rule uses the non-terminal
  `trust_lease_on`.
- **L9 — The free-hull rule** adds docked at the programme port and no hold vault on the stowage to
  `next_loaner`'s rule, so a staff-revoked hull (its hold owned by the old pilot) leaves the pool
  until IC13's recall re-homes it. The readiness line still counts it "laid up and free"; the two
  counts differ by design until IC13.
- **L10 — The hull description** is one template line written at enrolment when the hull has none,
  under the hull's own owner (D-18-22), no pack change.
- **L11 — The gauges are data-driven** from `interior.gauges` and the goods table with two
  template-id word tables in code (built as the functions `compartment_words` and `room_instrument`,
  not constants); `dray.json` is not edited; burn and reserve lines are IC9's.
- **L12 — Command classes are per verb**: `+coop` and `+ship` both `LegacyPostImage`; the stanzas
  say which switches read.
- **L13 — Money renders through `mush_economy::money`** (`25000.00cr`), the tree over doc 18's
  `25,000.00cr` prose.
- **L14 — `+coop/buyout`, `/release`, `@coop/revoke`, `+ship/plan`, `/undock`, `/orders`, `/order`
  are not registered**; the kernel's own refusal answers them.
- **L15 — The leave stamp** is `+ship/leave`'s own `ic_return::stamp`; the door-pass re-stamp is
  IC1's `IcReturnStamp`, pinned by test 8, not rebuilt.
- **L16 — The economy's own lease notices** ("Lease #N has been offered to you", "Contract #N is
  active") reach the pilot during `+coop/enrol` and are left alone; FAILED therefore says "your
  charter was not signed". The residue of a post-offer failure (an opened empty `tacsim.credit`
  account and a Cancelled lease record) is now **durable** because the fixer pins the deferred
  queue; before that fix it existed only in RAM until the next reboot, with its ids handed out
  again. A frozen economy now leaves no residue at all.
- **L17 — The observer folds `Settled | Cancelled | Expired`**; `Expired` is a harmless superset of
  §4.1's two.
- **L18 — The status line carries the mark** (18a §3's form), where doc 18 §2 step 8 and §6 omit it;
  the mark tells eight identical interiors apart. An owner text.
- **L19 — `+ship`'s first line is doc 18 §2 step 5's form**, with the gallery's full room name
  `Visiting Berth Gallery` (step 5 abbreviates it) and rendered as three lines, not one run. A
  deviation and an owner text.
- **L20 — IC6 lands before IC7** against §5's "mush-economy touches IC7 before IC6"; the edits are
  disjoint (`contract.rs`'s one new function, the private `owns_lease`, `sweep.rs`'s one filter,
  `lib.rs`'s re-export list). IC7 rebases on these lines if it touches them.
- **L21 — Unreadable enrolment beside a live charter: the lease wins** (D-18-23).
  `+ship`/`/hold`/`/board` keep working; `+ship`'s programme line and `+coop`'s charter line go
  dark; `+coop/statement` and `+coop/enrol` answer `BOOKS_UNREADABLE`.

**Fixer rulings beyond the brief, taken as the tree's rule:** `+coop`'s charter line and
`+coop/statement`'s Charter line read the gateway account's books and the live lease, never the
character's document alone (an alt on an enrolled account reads `ENROLLED_ONCE`; a disputed lease
reads `CHARTER_NOT_IN_FORCE` / `Charter: not in force.`); `plan_enrolment` reads
`EconomyStore::is_frozen()` before the first write; `failed_after_writes` pins the deferred queue on
the `mush_economy::post` / `mush_canon::commit` precedent.

**Builder decisions recorded:** `retire_check`'s lessors are every org row of the port map, not the
programme org alone (the check has no `PortDef`; the Trust is among the rows, same result today);
`hold_vault` takes `&HullView` (the stowage's template id is the interior's); `ProgrammePort` is
`Clone` not `Copy`; a compartment accepting zero or several goods renders multi-good; readings
insert only non-zero quantities; a pre-existing `tacsim.credit` account is reused and the "opens
account" line omitted; the test rig replays the typed inventory post-images through
`Engine::hydrate_action_state` across a reboot and keeps a cumulative folded store; no log names an
enrolment key (its tail is the account's canonical hash, the tutorial loader's rule); test 14 goes
through the real `@atlas/pack retire` path; the brief's FULL example (52 t of fittings) is
arithmetically wrong (2,300 L/t × 52 = 119,600 L < 120,000), so the "load ≥ limit" rule is kept and
`FULL: packed volume` is proven with 40 t fittings + 20 t ice = 120,000 L.

**Owner-critical defaults taken** (each rewordable or reversible; none changes a number the owner
set):

1. **D-18-34 read as board-only for the lease gate (L7).** Proposed amendment for the owner to
   accept or reject: *"D-18-34 Boarding by verb. `+ship/board` in the gallery and `+ship/leave` in
   the airlock (and nowhere else, in both documents); board is gated on the live lease and the
   hull's placement, leave on the hull's placement alone, so a pilot whose charter ends aboard can
   always step ashore; real gangway exits are P-G."*
2. **`+ship`'s first line** uses step 5's form with the gallery's full name as three lines (L19);
   **the status line carries the mark** in 18a's form, not doc 18 §2 step 8's (L18).
3. **The 1-minor Manual rent** kept although zero would be admitted (L3).
4. **A staff-revoked hull leaves the pool until IC13** (L9): its hold vault, owned by the old pilot,
   stands.
5. **The player-facing texts this cut wrote** (all rewordable): `REGISTRAR_LINE` (doc 18's own), the
   three enrol reply lines, `NAME_USAGE`, `NO_ACCOUNT_MATCH`, `ENROLLED_ONCE`, `BOOKS_UNREADABLE`
   (which says "staff have been told" while only a log backs it), `NOT_OPEN`, `BAD_NAME`,
   `NAME_TAKEN`, `NO_HULL`, `NO_CAPITAL`, `FAILED`, `NO_CHARTER`, `CHARTER_BOOKS_DARK`,
   `CHARTER_NOT_IN_FORCE`, `ALREADY_ABOARD`, `BOARD_WHERE`, `NOT_HERE`, `UNDER_WAY_LOCK`,
   `LOCK_STUCK`, `LEAVE_NOT_ABOARD`, `LEAVE_WHERE`, `LEAVE_SEALED`, `SHIP_BOOKS_DARK`, `HOLD_DARK`,
   the board/leave narration, the revocation notice, the statement lines (`Charter: in force. /
   ended. / not in force.`, the two Cash sentences), the gauge words, `HULL_DESC_TEMPLATE`,
   `COOP_HELP`, `SHIP_HELP`, the readiness hulls-line wording including `nobody`.
6. **The failed-enrolment residue** (L16): a post-offer failure leaves an opened empty account and a
   Cancelled lease record, durably, and the pilot may already have seen the economy's lease-offered
   notice; acceptable or not is the owner's call.

**Deviations from doc 18 recorded:** L18, L19, L20 above; and **the gauge fixtures store the 20/80
vertex** that D-18-39's letter forbids any test to store. The brief's lens 7 mandated those
quantities, the tests assert a FULL-both reading and never optimality, and IC8's LP test still
derives the vertex; the owner may prefer a non-vertex load in the gauge fixtures.

**Residuals named for later cuts:** `page_stale_contracts` exempts by shape alone, so a player-built
Active contract with issuer collateral, an obligation and escrow is no longer paged (narrow to
`escrow.is_empty()` if money sitting still must page); `+coop/enrol` writes the `Enrolment` without
its own `check()` (a staff rename of a laid-up Dray over 120 bytes makes an `Unreadable` document
after the next reboot); `hull_line` would count a hull both `free` and `under way` if IC9 ever
leaves the root Direct on a port domain while `under_way` (close with `None if laid_up && !moving`);
`+ship/leave` unplaces before `move_to`, so a `move_to` failure leaves the placement gone behind
LOCK_STUCK (practically unreachable; `home` then works); the deferred-queue pin and the `end_lease`
unwind are reviewed in code only, nothing in the slice forcing a post-offer failure; test gaps: the
notice assertion in test 10 is a disjunction although the pilot is connected, the revocation is
asserted in RAM only, the `enrol_gate` unit test shuts on 3 of 8 bindings, no
ReadyToSettle/escrowed-Draft paging control, no test of the `live_charter` second guard,
`load_enrolment`'s identity rule, the reused-account branch or the FAILED path; the surface's
refusal order lists NO_CAPITAL before FAILED but a misconfigured world (scale overflow, no economy,
no Trust account) answers FAILED first. Nothing under `docs/design/` was edited by the build; this
note is the committer's.

### Pins re-blessed

- **`crates/mush-contract-tests/legacy-handlers.toml`**: two new non-baseline `legacy-post-image`
  stanzas, `+coop` (family `tacsim.coop`, after `+contacts`) and `+ship` (family `tacsim.ship`,
  after `+sheet`), in byte order, each naming doc 18 §4.1/§4.2 and its decisions, which switches are
  reads, and a future typed family (`+ic`'s wording). `PINNED_BASELINE` stays **97**;
  `legacy_handler_allowlist` 4 passed.
- **`crates/mush-contract-tests/state-slot-owners.toml`**: two new rows.
  `crates/tacsim/src/port/coop.rs` / `World::module_state_mut` / `EconomyStore` / owner `economy`
  (the pilot's account and hold vault take their ids from the economy's allocators before
  `open_account` / `create_vault` write them, the `port/apply.rs` precedent; the site refuses when
  the slot is absent), and `crates/tacsim/tests/coop.rs` / `AccountOracle` / owner `sim` (the
  enrolment rule is per gateway account, fail-closed on an unmapped character; the rig installs a
  fixed oracle on the `match_lifecycle.rs` precedent). `state_slot_owners` 4 passed; the tests reach
  no other foreign slot through `module_state_mut`.
- **Unmoved, and shown unmoved by the green runs:** every panic budget (no
  `unwrap`/`expect`/`panic!`/indexing outside `#[cfg(test)]` in the new code; `panic_budget` 7
  passed); `module-ns-owners.toml` (tacsim writes its own namespace through `push_op`, the observer
  included; economy and sim writes go through their owners' functions); the restriction-lints census
  (11 passed); the composition-edge pin (no new crate edge: tacsim →
  mush-economy/mush-sim/mush-inventory/mush-org already existed); every content digest (lane v2,
  `tacsim.pack.dray`, goods, stations, starter: `content.rs` 29, `stations.rs` 13, `lane.rs` 8, no
  pack edited, `per_account` already 1); `ports.rs` test 8's readiness report (the hulls Ready text
  unchanged with nothing leased; `report_ports` byte-identical to the inline code it replaced);
  `boot_parity` 18 passed (the roster golden and `!test_world`); the CI gated-test inventory (no
  `#[ignore]` or env gate added); `ci_gated_test_inventory` 6, `typed_state_lens` 6,
  `action_state_republish` 6, `hex_vocabulary` 2 passed. The `def.rs` unit test keeps its
  `per_account = 0` assertion and gains `= 2`; no existing assertion anywhere was changed or
  removed.

### How it was verified

**The gate runner's sixteen steps**, on Windows at the working tree over base 10b89d58 (the
builder's and the fixer's passes folded in), all green on the first attempt, no reruns, no
`LNK1104`, no hang; `git status` identical before and after (17 modified, 4 untracked), so no test
rewrote a golden:

- `cargo fmt --all -- --check` → exit 0, no output (twice: before and after the fixer's pass).
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s)``, exit 0, 0 warnings (twice; the builder's first run flagged one
  `clone_on_copy`, the fixer's one `large_enum_variant` on `Books`, both fixed).
- `cargo test -p mush-economy --lib` → `test result: ok. 36 passed; 0 failed; 0 ignored`.
- `cargo test -p mush-economy --test e4b1 --test e4c --test e4b2` → e4b1 `ok. 83 passed`; e4b2 `ok.
  117 passed`; e4c `ok. 44 passed` (the deadline-less non-lease control `e6s1_s7_…` still pages).
- `cargo test -p tacsim --lib` → `test result: ok. 886 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 3.84s`.
- `cargo test -p tacsim --test coop` → `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured;
  0 filtered out; finished in 2.11s`.
- `cargo test -p tacsim --test ports --test hulls --test port_tick --test lane --test stations
  --test content --test capacity` → capacity 15, content 29, hulls 15, lane 8, port_tick 15, ports
  17, stations 13 passed; 0 failed in each.
- `cargo test -p mush-contract-tests` (twice) → lib 31 passed; every one of the 30 binaries `test
  result: ok`, 0 failed (`legacy_handler_allowlist` 4, `state_slot_owners` 4, `panic_budget` 7,
  `restriction_lints` 11, `module_ns_owners` 3, `ci_gated_test_inventory` 6, `help_commands` 4);
  `scale_harness` `6 passed; 8 ignored` is pre-existing.
- `cargo test -p mush-server --test boot_parity --test e6a_alerts` → `ok. 18 passed` / `ok. 6
  passed`.
- `cargo test -p mush-housing --no-fail-fast` → lib 1, e5a 54, e5b 81, s2b_site 1, doc-tests 0
  passed; 0 failed (the housing lease producer under the stale exemption).
- `cargo test -p mush-campaign --no-fail-fast` → lib 54, e7c10 58, e7c11 15, e7c9 85, e7r1 28, e7r2
  5 passed; 0 failed (the recovery lease producer).
- `cargo test --workspace --exclude mush-bot --no-fail-fast` → 387 `test result:` lines, every one
  `ok`: **7,298 passed; 0 failed; 92 ignored**; exit 0 (about 20 minutes of it a full test-profile
  rebuild and link). `mushd arena_story` (69.82 s), `arena_story_split` (46.32 s) and the
  `parse_operator_soup_never_panics` proptest drew libtest's over-60-s notice under full load and
  passed; none of the brief's load-sensitive suites flaked.
- `cargo test -p mush-bot` → lib 469 passed; every binary ok (`arena_e2e` 9 passed in 202.94 s,
  `playtesting` 9 in 124.53 s, `e2e` 49); 0 failed.

**Mutation checks reported by the builder and the fixer** (not re-run by the judge): the stale-page
filter disabled → `ic6_a_live_lease_is_never_paged_as_stale` and coop test 9 both fail; the frozen
guard disabled → test 15 fails "a frozen refusal wrote something", accounts 11 vs 10.

**The judge's lenses** (read-only, no cargo): the whole diff and all four new files read against the
brief, doc 18 §4.1, §4.2, §5 and D-18-6/21/23/24/25/34/39; every reviewer finding re-checked in the
code (correctness F1/F2 fixed and pinned, conformance F1 fixed and pinned, tests T1–T4 fixed; F3–F6
and T5–T10 stand as the residuals above); the builder's surface list and the fixer's
`surface_changes` checked against the code; no existing assertion weakened (every removed diff line
is production code); `report_ports` compared line by line with the inline code it replaced;
`owns_lease`'s new form shown equivalent; `retire_check` shown preflighted on every entry by
`mush-content/src/apply.rs:676`; the FULL arithmetic confirmed (fittings 2,300 L/t). Flakes
identified: none.

### Surface for the next cuts

**mush-economy (re-exported at the crate root)**

- `pub fn is_lease_shaped(c: &EconomicContract) -> bool`: issuer collateral plus an Obligation;
  `page_stale_contracts` exempts `Active && is_lease_shaped`; `owns_lease` (private) is this plus
  `issuer == lessor` and `Collateral{premises, lessor}`.

**`tacsim::port::coop` (re-exported from `tacsim::port`: `Charter, Enrolment, EnrolmentState,
EnrolmentStatus, ProgrammePort, live_charter, programme_port`)**

- Constants: `COOP_VERB = "+coop"`, `COOP_HELP`, `COOP_PREFIX = "tacsim:coop:"`, `ENROLMENT_SCHEMA:
  u16 = 1`, `STAKE_KEY_NAMESPACE = "tacsim.backer.stake"`, `MAX_HULL_NAME_BYTES = 32`,
  `REGISTRAR_LINE`, `COUNTERSIGN = "Ilse Oduya-Brandt"`, `ACCOUNT_OPENER = "Ceres Clearing"`,
  `NEW_DRAY_MAJOR: u64 = 120_000`, `HULL_DESC_TEMPLATE` (`{mark}`); the refusal/status texts
  `NOT_OPEN_HERE`, `NAME_USAGE`, `NO_ACCOUNT_MATCH`, `ENROLLED_ONCE`, `BOOKS_UNREADABLE`,
  `NOT_OPEN`, `BAD_NAME`, `NAME_TAKEN`, `NO_HULL`, `NO_CAPITAL`, `FAILED`, `NO_CHARTER`,
  `CHARTER_BOOKS_DARK` (`{name}`, `{mark}`), `CHARTER_NOT_IN_FORCE` (`{name}`, `{mark}`).
- `pub enum EnrolmentStatus { Active, BuyoutPending, Graduated, Revoked }` (snake_case on the wire;
  IC13 writes the middle two).
- `pub struct Enrolment { schema: u16, account: mush_core::AccountId, pilot: ObjId, programme:
  CanonicalId, pack: CanonicalId, entry: CanonicalId, hull: ObjId, mark: String, laid_up_name:
  String, name: String, lease: u64, vault: u64, credit_account: u64, stake_minor: i64,
  share_permille: u32, buyout_minor: i64, enrolled_at: i64, status: EnrolmentStatus,
  #[serde(default)] sales: u32, #[serde(default)] margin_minor: i64, #[serde(default)] shares_minor:
  i64 }`: `deny_unknown_fields`; `check()` (schema 1; mark/name/laid_up_name 1..=120 bytes; stake
  and buyout > 0; share ≤ 1000; the counters ≥ 0). **IC10 folds
  `sales`/`margin_minor`/`shares_minor` through `store::put_enrolment`; IC13 writes
  `BuyoutPending`/`Graduated`.**
- `pub enum EnrolmentState<'a> { Absent, Present(&'a Enrolment), Unreadable }`.
- `pub fn enrolment_key(AccountId) -> SmolStr` (`tacsim:coop:{canonical_hash}`; never log it); `pub
  fn stake_key(ContractId) -> IdempotencyKey`.
- `pub struct ProgrammePort<'a> { pack: CanonicalId, entry: CanonicalId, def: &'a PortDef,
  programme: &'a ProgrammeDef, map: &'a PortMap }` (Clone, Debug) with `trust() ->
  Option<PrincipalId>`, `trust_account() -> Option<AccountId>`, `trust_scope() ->
  Option<OrgScopeId>`; `pub fn programme_port(&World) -> Option<ProgrammePort<'_>>` (the first port
  entry in `report_ports` order with a programme and a Present map under `LANE_PACK`); `pub fn
  port_name(&PortDef) -> String`.
- `pub fn trust_lease_on(&World, &PrincipalId, ObjId) -> Option<&EconomicContract>` (non-terminal,
  issuer the trust, `Collateral{hull, trust}`, lease-shaped).
- `pub struct Charter { contract: ContractId, hull: ObjId }` (Copy); **`pub fn live_charter(&World,
  pilot: ObjId) -> Option<Charter>`, the operate gate (D-18-23): Active, issued by the Trust, the
  pilot a party, pledging a programme-map hull; lowest id; never reads the document.** IC9's undock,
  IC11's queue and every executed step gate on it.
- `pub fn hull_is_free(&World, &ProgrammePort, &HullRow) -> bool`; `pub fn free_hulls(&World,
  &ProgrammePort<'a>) -> Vec<&'a HullRow>` (ascending hull id).
- `pub fn enrol_gate(&Readiness) -> Result<(), &ReadinessLine>` (Lane, Rate, Map, Rooms, Domain,
  Org, Programme, Hulls must be Ready). **IC8 adds its pools/listings bindings here.**
- `pub fn check_hull_name(&World, &ProgrammePort, &str) -> Result<String, &'static str>`.
- `pub struct CoopObserver` (an `EconomyObserver`, composed in `econ_observers()` after the
  market's; folds `Settled | Cancelled | Expired` on an Active enrolment to `Revoked` and notifies).
  **IC13's buyout observer extends its `revoke` arm (BuyoutPending → Graduated on Settled).**
- `pub(crate) fn name_of(&World, ObjId) -> String`.

**`tacsim::port::gauges` (pure)**

- `pub const DARK = "dark"`; `pub type Reading = (CanonicalId, Compartment, BTreeMap<CanonicalId,
  u64>)`.
- `compartment_words(&CanonicalId) -> (String, String)`, `room_instrument(&CanonicalId) -> &'static
  str`, `measures_line(goods, comp, contents, with_full) -> Result<String, CapacityError>`,
  `goods_list(..) -> String`, `single_summary(..) -> Result<String, CapacityError>`,
  `ship_hold_line(goods, &[Reading]) -> Result<String, CapacityError>`, `room_lines(room_tid, goods,
  shown: &[Reading]) -> Result<Vec<String>, CapacityError>`, `hold_lines(name, mark, goods,
  &[Reading]) -> Result<Vec<String>, CapacityError>`. **IC9 adds the burn and reserve lines beside
  these; IC9's `+ship/plan` reuses `measures_line` for the two slacks.**

**`tacsim::port::ship`**

- Constants: `SHIP_VERB = "+ship"`, `SHIP_HELP`, `HULL_CLASS = "Dray"`, `STATUS_DOCKED = "Aboard the
  {name} ({mark}), docked at {port}."`, `STATUS_UNDER_WAY = "Aboard the {name} ({mark}), under
  way."` (**IC9 appends the leg and ETA**), `STATUS_ELSEWHERE`, `SHIP_DOCKED = "{name} ({class}):
  docked {port}, {gallery}."`, `SHIP_UNDER_WAY`, `SHIP_ELSEWHERE`, `SHIP_BOOKS_DARK`, `HOLD_DARK`,
  `ALREADY_ABOARD`, `BOARD_WHERE`, `NOT_HERE` (`{name}`), `UNDER_WAY_LOCK` (`{name}`), `LOCK_STUCK`,
  `LEAVE_NOT_ABOARD`, `LEAVE_WHERE` (`{airlock}`), `LEAVE_SEALED` (`{name}`).
- `pub struct HullView<'a> { port: ProgrammePort<'a>, row: &'a HullRow, interior: &'a InteriorDef,
  logistics: &'a LogisticsDef }` with `hull()`, `room(&CanonicalId)`, `airlock()`, `stowage()`; `pub
  fn hull_view(&World, ObjId) -> Option<HullView<'_>>`; `pub fn hull_of_room(&World, room: ObjId) ->
  Option<(HullView<'_>, CanonicalId)>`.
- `pub enum Berth<'a> { Docked { map: &'a PortMap, def: &'a PortDef }, UnderWay, Elsewhere }`
  (Copy); `pub fn berth_of(&World, ObjId) -> Berth<'_>`.
- `pub fn hold_vault(&World, &HullView) -> Option<VaultId>`; `pub fn status_line(&World, &HullView)
  -> String`.
- `pub struct ShipLook` (an `EventHook`, `look_rendered` only); `pub fn register_hooks(&mut
  Registrar) -> Result<(), ModError>` (called from `TacsimRuleset::register` after
  `port_tick::register`).
- `+ship` (`ArgShape::None`, switches `board`, `leave`, `hold`); **IC9 adds `plan` and `undock`,
  IC11 `orders` and `order`, to this spec's switch list and `ship_cmd`'s dispatch.**

**`tacsim::port_tick`**

- `pub fn read_pool_in(&HookReadContext<'_>, EndpointRef, &CanonicalId) -> Result<u64,
  PoolReadError>`: `read_pool`'s rule from a hook (absent = 0, a refusal never 0).

**`tacsim::store` / `TacsimStore`**

- Fields `pub(crate) enrolments`, `enrolment_keys`, `goods`; `pub fn enrolment_state(AccountId) ->
  EnrolmentState<'_>`, `enrolment_of(ObjId) -> Option<&Enrolment>`, `enrolment_for_lease(u64) ->
  Option<&Enrolment>`, `enrolments() -> impl Iterator<Item = &Enrolment>`, `goods() ->
  Option<&GoodsTable>` (the one read; never re-parse `goods.json`); `pub(crate) fn
  put_enrolment(&mut Ctx, &Enrolment)`, the single write path.
- `TacsimRuleset.goods: Option<Arc<GoodsTable>>` (private; parsed once in `with_game_content`,
  copied into the slot in `register` and `on_world_loaded`). **Any new RAM-only slot field must be
  copied at both sites.**

**`tacsim::port` (crate-private helpers later cuts reuse)**

- `apply::credit_scale(&World) -> u8`, `apply::minor_of(u64, u8) -> Option<i64>`,
  `apply::leased_refusal(&CanonicalId, &str) -> String`; `hull::template`, `hull::berths_slot`,
  `hull::hull_spec(store, programme) -> Option<(&InteriorDef, &LogisticsDef)>`;
  `readiness::report_ports(store, w) -> (Option<&RulesPack>, Vec<(CanonicalId, &PortDef)>)`.
- `PortDef::check` refuses `per_account != 1`.
- The readiness hulls line with anything leased or under way: `"{entry} hulls: {pool} of {pool} made
  and titled to {org}; {free} laid up and free, {k} leased ({mark} to {tenant}, …)[, {u} under
  way]."`, state Ready; `free` = laid up with no Trust lease (a revoked hull with its old hold vault
  still counts here, not in `free_hulls`).

**Gates and facts later cuts depend on:** `legacy-handlers.toml` has `+coop` (`tacsim.coop`) and
`+ship` (`tacsim.ship`) stanzas; `PINNED_BASELINE` 97; `state-slot-owners.toml` has the
`port/coop.rs` `EconomyStore` and `tests/coop.rs` `AccountOracle` rows; `panic_budget/tacsim.txt`
unchanged at 5; no pack version or digest moved (lane still v2, `per_account: 1`).

**Test rig patterns later cuts copy** (`crates/tacsim/tests/coop.rs`): `Roster::{bare, bare_real}` +
`.disabled()` + `.freezable()` (the test-only `Latch` module with `@econfreeze`), `MapOracle` into
`mush_sim::AccountOracle`, `boot`/`boot_unsettled`/`sweep`, `reboot_with(rig, roster, edit)` folding
module ops and replaying the typed inventory post-images through `Engine::hydrate_action_state`,
`pilot`/`pilot_on`, `enrol`, `enrolled()` (one chartered pilot, *Salt Moth*, FPC-D-0401),
`seed_pool(rig, vault, good, qty, LotId)`, `seed_outbound`, `hold_vault`, `replace_hull`,
`placement`, `has_block`, `counts`, `lease_of`, `trust_account`, `set_rate`.

## IC7

**Market seams for content ports** (doc 18 §5, the IC7 row and paragraph), built 2026-10-16 on main
at 3bf2bc90. The six market and economy seams §4.3/§4.4 name for content ports, game-neutral, with
test implementations only; tacsim's gate, party-cap source and capacity provider are IC8's. One
commit, pushed to origin/main.

### What landed

1. **The content-resolved party-cap source** (`crates/mush-economy/src/contract.rs`,
   `src/module.rs`, `src/contract_live.rs`, `src/lib.rs`). `pub trait PartyCapSource { fn
   has_cap(&self, w: &World, principal: &PrincipalId, cap: PartyCap) -> bool }` beside `PartyCap`;
   `EconomyModule::with_party_cap_source(Arc<dyn PartyCapSource>)` frozen into
   `EconomyStore.party_cap_source` at boot beside `party_caps`; one crate-visible predicate
   `module::party_cap(w, principal, cap)` that asks the wiring-frozen table first and the source
   only when it said no, so a source can only add a principal and never remove one, and confers no
   money authority (escrow still posts under the org account's own debit authority). All three
   programmatic-acceptance sites go through it: `accept_contract_as`, `deliver_settled`, and the
   consideration-naming rule in `consideration_for`, which is reordered viewable → party →
   capability so the predicate is never asked about a buyer's own account or a non-party owner (same
   truth table as before for every table-named vendor). `pub fn has_fact_provider(w, namespace) ->
   bool` in `facts.rs`, `false` with no economy; `PartyCapSource` and `has_fact_provider`
   re-exported.
2. **The stale-page narrowing** (`crates/mush-economy/src/contract.rs`, `src/sweep.rs`). IC6's
   exemption is now `pub(crate) fn exempt_from_stale_page(c) = Active && is_lease_shaped(c) &&
   c.escrow.is_empty()`: the lead note's literal reading of the record's own escrow list, so a stale
   entry pages rather than hides. `page_stale_contracts` filters on it with IC6's comment kept. An
   `Active` lease holding escrow, a `ReadyToSettle` lease and an escrowed `Draft` lease all page;
   the in-crate test `ic7_only_an_active_escrow_free_lease_is_exempt_from_the_stale_page` pins the
   five cases on IC6's `shaped()` fixture, and both best-effort integration tests were written in
   `tests/e4b1.rs` through the `@contract` builders (helper `escrowed_lease_draft`).
3. **The market's three seams** (`crates/mush-market/src/module.rs`, `src/lib.rs`).
   `Presence::{Here, Ship(ObjId)}`, `VenueRefusal { line }`, `VenueGate` with five methods
   (`venue_of_room`, `endpoint_for`, `admit`, `governed`, `port_seller`; the fifth is L7's),
   `CapacityPlan(serde_json::Value)` with `within_bounds()` against `MAX_CAPACITY_PLAN_BYTES =
   2_048`, `CapacityReadError`, `CapacityRefusal::{Refused { fits, line }, Unreadable}`, and
   `CapacityProvider::{plan(&World, EndpointRef) -> Option<CapacityPlan>, check(&plan, commodity,
   qty, read_pool) -> Result<(), CapacityRefusal>}` (L3's split, because the Draw prepare has no
   World). `MarketModule::with_venue_gate` / `with_capacity_provider` freeze them into
   `MarketTables` beside the inventory handles; `Debug` reports their presence; `pub fn
   venue_gate(w)`, `pub(crate) fn capacity_provider(w)`. Unwired is the pre-IC7 behaviour exactly;
   the tacsim implementations are IC8's.
4. **Draw listings** (`crates/mush-market/src/record.rs`, `src/store.rs`, `src/api.rs`,
   `src/commands.rs`). `LotMatter::{Hold { endpoint, commodity, grade }, Draw { source, commodity
   }}` with `source()`, `commodity()` and `kind()`, and `MarketStore::lot_matter(&Listing)` as the
   one accessor: `Supply::Lot` is `Hold`; `Supply::Stock { vault }` on a `Representation::Lot` entry
   whose catalogue code parses as a `CanonicalId` is `Draw`; everything else is E6c's counter
   listing. `reindex` resolves every live listing's pool through `lot_matter` (after the catalogue
   index) so `by_lot_pool` and the one-live-listing-per-pool rule cover both kinds, while
   `lot_listings()` stays hold-backed only for the sweep's reconcile arm. `list_goods`'s `(Lot,
   None, Supply::Stock { vault })` arm runs `draw_listing_precheck` before any write: the vault is
   the vendor's own storefront, the actor `may_take` from it (else the existence-hiding
   `NO_SUCH_LISTING`), the code is a commodity id, no live listing sits on the pool, and the
   storefront's **available** stock (`lot::pool_available`, balance minus live holds) covers the
   quantity, an unreadable read refusing. Nothing is staged and no TTL cap applies. `@market/vendor
   stock #<v>=<code> …` with no `endpoint=` builds it, and the vendor org's own officer may publish
   it as well as staff.
5. **The accept path, the window, the cutoff and the Fact gate** (`crates/mush-market/src/trade.rs`,
   `src/record.rs`, `src/facts.rs` new, `src/interface.rs`). `PaidKind::{Hold, Draw}` (stored
   snake_case), `PaidLot { kind, source, destination, commodity, grade, accepted_at, draw_by }` with
   `check()`, `Quote.lot` (the terms frozen at activation of every lot sale, both kinds) and
   `Quote.drawn_at` (the mirror, written only by the sweep's mirror arm), both carried by
   `QuoteWire` with the checks re-run on load. `DRAW_WINDOW_SECS = 3_600` and
   `DRAW_SETTLE_MARGIN_SECS = 600`, both real seconds, with the margin's arithmetic in the comment.
   `QuoteRequest.presence`; `quote` and `sell` ask the gate after the vendor-trading check and write
   nothing on a refusal. `pub fn accept_at(ctx, quote, destination, presence)` is the one accept
   path (`accept_into` is it with `Here`): a non-lot purchase meets the gate with no endpoint; a lot
   purchase runs `lot_precheck`, every refusal before `offer_contract`: the named destination else
   the gate's `endpoint_for` else the E7r2 refusal; custody and not-the-source; the gate's `admit`
   with the endpoint; the governed rule (`PORT_ONLY_ENDPOINT` unless the matter is a Draw from a
   `port_seller`); the capacity pre-check reading pools through `Ctx::action_state`
   (`CAPACITY_UNREADABLE` rather than zero); and for a Draw the storefront's available stock and
   `has_fact_provider(w, "market")`. `draw_by = accepted_at + 3_000` is computed with checked
   arithmetic before the money. `purchase_spec` gives a Draw sale the conditions `[Before { RealUtc,
   accepted_at + 3_600 }, Fact { market / market.lot.drawn / draw_fact_id(quote, accepted_at) }]`,
   the same deadline and `Expiry::Refund`; every other sale is unchanged. `pub fn restage_draw(ctx,
   quote) -> Result<Option<ContractId>, MarketError>` with its four results (`Ok(None)` not
   re-stageable, `DRAW_TOO_LATE` past `draw_by`, the undeliverable line when nothing staged,
   `Ok(Some)` re-staged byte-identically). `MarketFactProvider` (namespace `market`, the sixth)
   answers `market.lot.drawn` only from `drawn_at`, with the F8 note on who can see the id.
6. **`LotOp::Draw`, the paid-leg projection, the receipt and the contract key**
   (`crates/mush-market/src/lot.rs`, `src/module.rs`, `src/record.rs`). The fourth op `draw` and
   `LotOp::is_paid()`. `DrawReceipt { contract, quote, listing, source, destination, commodity,
   quantity, actor, at_ic }` with `check()`, in the market-owned typed family `market_lot_draws`
   (descriptors `market.lot_draw_*.v1`, key `{:016x}`, validator `check`) registered by
   `register_draws` before `reg.state` and parked in `MarketTables.draws`; `pub type DrawReceipts`,
   `pub fn draw_receipts(w)`. `project_offer` builds the actor's **paid legs** from their own quotes
   (`paid_legs`): a draw entry is an open, activated Draw quote with `drawn_at` none, an `Active`
   contract, `now <= draw_by` and a destination the actor holds, carrying the capacity plan (null
   with no provider or an ungoverned endpoint; an oversized plan omits the entry); a consume entry
   is an activated hold-backed quote whose contract is `Active`, `ReadyToSettle` or `Settled`,
   unsettled first then settled newest-first; both capped at `MAX_PAID_ENTRIES_PER_KIND = 8`. The
   fixed parameters become `{"now", "paid": {"<contract decimal>": {op, quote, listing, source,
   destination, commodity, quantity, grade, plan}}}`; the op choices are `reserve`, `consume` (only
   with an entry), `release`, `draw` (only with an entry); a `contract` Choice over the entries'
   decimal ids joins the schema only when an entry exists, so a supplied `contract` from anyone else
   is `InvalidParameters` at resolution. The Consume arm requires a consume entry and `matches_paid`
   field for field before IC0's logic, **which closes IC0's direct-consume path** (any endpoint
   holder could take a live hold-backed lot with no contract). `prepare_draw`: the entry, the
   cross-check, the receipt as the epoch-free replay guard, the capacity backstop against the
   carried plan through `families.pool` in the txn (fail closed), `families.transfer(storefront →
   buyer)` as pool quantity with no lot, the receipt inserted in the same bundle, the outcome.
   `LotLeg.contract`; `stage` keys a paid leg `market.lot.{op}.{contract:x}` and writes `"contract"`
   as a decimal string, while `reserve`/`release` keep `market.lot.{op}.{listing:x}.{ic second:x}`
   and never write it. `lot::draw_receipt` reads a receipt through `ctx.action_state` on the
   market's own handle (owner-scoped: it answers only on the market's own tick and boot contexts).
   `pool_balance` and `pool_available` are the command layer's pool reads.
7. **The draw mirror and the lapsed-Draw release** (`crates/mush-market/src/boot.rs`,
   `src/store.rs`, `src/trade.rs`). `reconcile_draws(ctx, MAX_DRAWS_MIRRORED_PER_FIRE = 64)` is a
   third arm on the existing `market.sweep` tick (no second tick, nothing staged): it visits up to
   64 activated, unsettled, undrawn Draw quotes round-robin from the RAM cursor
   `MarketStore.draw_cursor` through the pure `draw_lap(undrawn, after, budget)`, which resumes
   after the last id visited and wraps to the lowest, so a full lap over N quotes takes exactly
   `ceil(N / 64)` fires (unit-tested at 576 → 9, 512 → 8, 100 → 2). A receipt found sets `drawn_at =
   now` with the revision bumped; absent leaves it; unreadable leaves it and warns once per fire. It
   is the only writer of `drawn_at`, so the Fact is behind the receipt and never ahead. The same arm
   **releases an undrawn Draw quote whose contract is `Cancelled` or `Expired`** by calling
   `release_quote` on the market's own context; the settlement ear `on_contract_changed` leaves such
   a quote alone (`awaits_receipt_read`), because it runs on the emitting module's context, where
   the owner scope refuses the market's family. **The no-relist guard** in `release_quote`: for an
   activated Draw quote, `goods_moved` (the mirror, else the receipt; unreadable or no contract
   counts as moved, fail closed) skips `restore_quantity`, still returns the budgets and deletes the
   row, logs an error and opens one `BootReconciliation` escalation on the contract with the brief's
   text and the two recommended commands.
8. **The verbs** (`crates/mush-market/src/commands.rs`). `+market/list` with no filter word and
   `+market/depth` with no `market=` word default to the gate's `venue_of_room` for the actor's room
   (`gate_venue`), the pre-IC7 page otherwise. `+market/accept` parses `<quote>=<vault>` and
   `<quote> to=<vault>` as one command (a second word must be `to`, any case; anything else is `No
   such quote.`), with the destination defaulting to the gate's `endpoint_for`; it calls
   `restage_draw` first: `Ok(Some)` replies "Asking again for the hand-over on contract #N. It
   settles once the goods have moved.", `Err` refuses, `Ok(None)` falls through to `accept_at(..,
   Here)`. `+market/quote` and the economist's `market.quote` pass `Presence::Here`. Three help
   additions: `+market/list` ("With no filter, where you stand in a port, you see that port's
   board."), `+market/accept` (the `to=` sentence, the berthed-hold default, "Taking up a drawn
   quote whose goods have not moved yet asks for the hand-over again.") and `@market/vendor stock`
   (the DRAWN listing sentence).
9. **Tests** (`crates/mush-market/tests/ic7.rs` new, 18 tests; `crates/mush-market/src/boot.rs` one
   unit test; `crates/mush-economy/src/contract.rs` one unit test;
   `crates/mush-economy/tests/e4b1.rs` two tests; `crates/mush-market/tests/e7r2.rs` the four-ops
   rename). The ic7 rig is IC0's on the production composition in miniature: org, economy with an
   **empty** frozen table plus `TestCapSource`, `MarketFactProvider` and `MarketObserver`, inventory
   delegating to `market`, the market with one lot entry, `TestGate` (venue `luna`, the buyer's hold
   governed, the vendor org a port seller, `Here` admitted only in the counter room and `Ship` only
   for one hull) and `TestCapacity` (plans for the hold only, records its last answer), plus the
   `ic7probe` closure module and `take`/`land_submit` so one submission can be held while another
   lands. The sixteen brief tests plus `ic7_a_draw_refused_at_its_prepare_is_restaged_and_lands` and
   `ic7_the_party_cap_source_reaches_the_consideration_rule_and_the_delivery_seam`; every test
   asserts no `SettlementRefused` page. Test 6 is sized at 130, not the brief's 120: at 120 the two
   60-unit accepts take the listing to 0, `commit_quantity` deletes it and `restore_quantity` never
   re-mints one (L1), so "restored by 60" could not be asserted; at 130 it is 10 → 70. Test 12(c)
   gives Quinn a paid entry of her own so the forged draw fails on the `contract` choice rather than
   a missing parameter (stronger than asked).
10. **The pinned allowlist** (`crates/mush-contract-tests/legacy-handlers.toml`). The `+market` and
    `@market` notes are refreshed, not joined, with an IC7 paragraph each; the category stays
    authoritative and `PINNED_BASELINE` does not move.

### Deferred, and why

**Lead rulings taken by the planner** (L1–L14 of the IC7 brief, recorded here so the owner sees
them):

- **L1 — A refused Draw is made safe by a Fact gate and its escrow comes back by expiry, not a
  cancel.** Doc 18 §4.3/§5 say a Draw refused at prepare "cancels its contract and returns the
  escrow with no page". The tree cannot do that: nothing tells the market a submission was refused
  (`world_task.rs` tells only the actor), a party may cancel only a `Draft`/`Offered` contract while
  the matter leg is staged after the contract is `Active`, and the contract would otherwise settle
  on the next economy sweep and pay for nothing (§9 risk 3). So the Draw sale's contract carries the
  `market.lot.drawn` Fact, answered from a receipt the Draw writes in its own bundle, and expires
  with `Expiry::Refund` and no page at `DRAW_WINDOW_SECS` when no receipt appears.
  **Owner-critical.** Visible consequences: a refused Draw locks the buyer's cash until the hour
  ends; the listing under-reports by the refused quantity until the expiry restores it; a listing
  that reached zero and was deleted is not re-minted, so those tonnes sit un-listed in the
  storefront until IC8's readiness line shows them and someone relists.
- **L2 — `DRAW_WINDOW_SECS` = 3,600 and `DRAW_SETTLE_MARGIN_SECS` = 600 real seconds.** The cutoff
  exists because readiness is evaluated before expiry but `Before` is false after the deadline: a
  Draw landing in the window's last seconds would be mirrored too late, the contract would refund
  while the goods had moved, and the listing would be restored over goods that left. No Draw is
  projected, so none is prepared, after `draw_by` = accept + 3,000 s. With the mirror's 64 per 60 s
  fire and the economy's 30 s sweep the Fact is true before the deadline while at most 576 undrawn
  Draw quotes stand; past that the no-relist guard and its page are the backstop. Hold-backed sales
  keep `DEAL_WINDOW_SECS`.
- **L3 — The capacity hook is split `plan(&World, endpoint)` + `check(&plan, …, read_pool)`.** Doc
  §4.3's single `check(endpoint, …)` cannot be implemented inside `prepare_authoritative`, which has
  an `ActionTxn` and no World; the plan is resolved where a World exists and carried, bounded at 2
  KiB, in the server-authenticated fixed parameters.
- **L4 — The Draw's source is projected from the buyer's own paid quote, not the live listing.** Doc
  §4.3 says `project_offer` projects the source "from the live listing"; a listing alone would let
  any endpoint holder draw for free (submissions are untrusted). Stronger than the doc, same intent.
- **L5 — Only the paid legs are contract-keyed.** `consume` and `draw` stage under
  `market.lot.<op>.<contract>`; `reserve` and `release` keep the listing + IC-second key.
- **L6 — `to=` is accepted as doc 18's spelling of the existing destination.** `<quote>=<vault>` and
  `<quote> to=<vault>` are one command, defaulting to the gate's `endpoint_for`; IC10's sell arm
  adds `from=`. **Owner-critical: the grammar is the owner's to confirm.**
- **L7 — The governed rule needs a fifth gate method, `port_seller`.** Lead 2e's "refuse any
  non-port listing into a governed endpoint" needs the gate to say who a port institution is; a
  hold-backed `Supply::Lot` listing never qualifies.
- **L8 — One `Option<Arc<dyn PartyCapSource>>` through one predicate at all three sites**, asked at
  the consideration site only for a non-viewable account owned by a party of the contract, so the
  source is never asked about a buyer's own account or a stranger.
- **L9 — The stale narrowing is the literal `c.escrow.is_empty()`**, extracted and unit-tested; the
  looser `qualifying_escrow` reading was dropped.
- **L10 — The draw receipt is a market-owned typed family and the Fact a sixth namespace, `market`**
  (D-E7-30's one namespace per crate; `sim.station.served` is the precedent). `transfer_marked` is
  not built; it is IC10's.
- **L11 — Draw listings are checked against the pool at publication** (available ≥ quantity,
  unreadable refuses); the doc's equality check and drift page are IC8's readiness and report lines.
- **L12 — IC0's direct-consume path is closed in this cut.** At HEAD any holder of an inventory
  endpoint could submit `market_lot` op `consume` naming a live hold-backed listing's `(listing,
  source, commodity)` and take its lot with no contract. The paid projection built here is the exact
  mechanism that closes it, so it covers activated consume quotes too and the Consume arm requires a
  matching entry. The hold mechanics (hold read, TTL, sold whole, the reservation, the derived lot
  ids) are IC0's unchanged; only who may fire the consume narrows, to the buyer of a paid contract.
  **Owner-critical: a security change to IC0's shipped lane, made under lead 2b; the default taken
  is to close it.**
- **L13 — "A re-staged lot leg replays as a HIT" is proven by proxy.** The market crate cannot reach
  the driver's idempotency store. Test 9 proves the re-stage is the same request (identical key,
  parameters, actor, action, subject and targets: every field `command_ingress` digests) after 300 s
  have passed, and that the receipt guard refuses it if prepared anyway; the driver's own HIT is
  pinned by `mush-server`
  `world_task::tests::vf_r1_a_replayed_move_across_an_epoch_change_moves_nothing_twice` and
  `vf_b12_the_first_commit_and_the_hit_read_the_same`, both run green.
- **L14 — Hold-backed sales stay exposed to §9 risk 3.** A consume refused at prepare, or dropped by
  back-pressure, leaves a `Before`-only contract that settles and pays the vendor for nothing; IC0's
  live-hold reads at quote and accept bound it but do not close it. Lead 2b keeps the hold path on
  IC0's rules, so it is not Fact-gated here and no cut is scheduled to do it. **Owner-critical: a
  known money-for-nothing risk on a player-facing path; whether and when to close it is a priority
  call.** The accept help now promises a re-stage only for a *drawn* quote; a hold-backed quote
  whose consume was dropped still reads "already been taken up".

**Owner-critical defaults taken** (beyond L1, L6, L12 and L14 above):

- **The new player-facing texts**, none given verbatim by doc 18 or 18a, in the voice of the
  existing market lines: `PORT_ONLY_ENDPOINT` "That endpoint takes goods only from its port's own
  counters, so the sale was not made. Nothing was charged."; `CAPACITY_UNREADABLE` "That endpoint's
  load cannot be read right now, so the sale was not made. Nothing was charged."; `DRAW_TOO_LATE`
  "It is too late to ask for those goods again. The deal lapses and your payment comes back."; the
  re-stage line "Asking again for the hand-over on contract #N. It settles once the goods have
  moved."; the five stocking refusals ("A lot-represented good is stocked out of an inventory
  endpoint (endpoint=<vault>) or drawn from the vendor's own storefront.", "A drawn listing sells
  out of the vendor's own storefront vault.", "That catalogue code is not a commodity id.", "That
  storefront holds less than that, so nothing was listed.", "That storefront's stock cannot be read
  right now, so nothing was listed."); the three help additions; and the staff-only page "market
  quote #q's goods were drawn but contract #c lapsed unpaid, so the listing was not restored. The
  buyer holds the goods and the payment came back."

**Decisions the builder and fixer took, recorded:**

- **The receipt is read through `Ctx::action_state`, and an undrawn Draw's release moved to the
  market's own sweep.** The builder first read the receipt through the raw typed-state lens (a
  pattern no crate outside `mush-core` used), because the settlement ear runs on the economy's
  context where the owner scope refuses the market's family. The fix keeps the brief's
  `ctx.action_state` read and instead has `on_contract_changed` leave an activated, undrawn Draw
  quote alone on `Cancelled`/`Expired` (`awaits_receipt_read`); `reconcile_draws`, on the market's
  own tick context, checks each candidate's contract state and hands a lapsed one to
  `release_quote`. Consequence: a refused Draw's listing comes back within one mirror lap of the
  expiry (normally the next 60 s market sweep) rather than in the economy's own fire. The orphan
  this could have made is not possible: the economy's only contract GC collects abandoned `Draft`s
  (`sweep.rs` `collect_draft`), so an `Expired` contract stays in the snapshot for the arm to find.
- **`register_draws` is a separate function** called before `reg.state`, while `lot::register` stays
  after the tick: the only way both of the brief's placement requirements hold.
- **`draw_receipts` is public**, in the `InventoryFamilies`/`mush-industry` handle pattern: a read
  capability only, still owner-scoped through `Ctx::action_state`; the tests and IC8's readiness
  line point-read receipts through the engine's store.
- The capacity pre-check applies to both lot kinds (the governed rule already blocks hold-backed
  sales into governed endpoints); consume entries carry `plan: null` on lead 2e's assumption that a
  hold-backed sale never reaches a governed endpoint. **IC8 must keep `plan()` `Some` only where
  `governed()` is true, or carry plans on consume entries.**
- `pool_available` treats a half pool (a scalar with no book) as unreadable. The `contract` value
  must round-trip as canonical decimal. If `stage` fails after the money is taken in `accept_at`, an
  error is logged and nothing else happens (a Draw then expires with a refund; a hold sale is L14).

**Not in this cut** (as the brief directs): tacsim's `VenueGate`, `PartyCapSource` and
`CapacityProvider` and the wiring of them and `MarketFactProvider` into the game's root (IC8; until
then the real roster has no Draw listing, and a composition with one but no `market` fact namespace
refuses Draw sales before money moves); the stock mint and the clerk tick's Draw listings; the
listing-vs-pool readiness and drift page; SOLD OUT rendering; the lapsed-hold release arm (never the
GOD sweep); the core "actor already has a queued submission" seam `+market/accept` should consult
before money moves (IC8 lead 2c-i); `Supply::Wanted`, `LotOp::Deliver`, `transfer_marked`, `from=`
(IC10); the executor's `accept_at(.., Ship)` and `restage_draw` calls (IC11); a Fact gate on the
hold path (L14, unscheduled); retention of `market_lot_draws` rows (later hygiene).

**Small residuals carried forward:**

- `module.rs:109`'s "Three more composition-frozen hooks" should read two (the third IC7 seam is the
  economy's).
- `party_cap`'s comment says the store borrow ends before the source is asked, which it does not
  (harmless, both shared).
- The no-relist page says the goods were drawn even on the fail-closed unreadable-receipt branch.
- `lot_matter` re-derives Draw-ness from the live catalogue entry, so a pack reload flipping an
  entry's representation changes an existing `Supply::Stock` listing's mechanics (money still
  protected by the accept-time pool check and the Fact gate; packs are digest-pinned).
- The e4b1 integration tests count any escalation on the contract rather than the stale page's kind.
- Untested branches: the gate on `sell`, the `depth` default, the mirror across more than 64 undrawn
  quotes in a composed world, a Draw accept refused on storefront availability, the fact provider's
  id parsing, and the race variant where the Draw listing survives the first accept.

### Pins re-blessed

- **`crates/mush-market/tests/e7r2.rs`**: `e7r2_p1_the_lot_action_has_exactly_three_ops` →
  `e7r2_p1_the_lot_action_has_exactly_four_ops`; `[Reserve, Consume, Release]` → `["reserve",
  "consume", "release"]` becomes `[Reserve, Consume, Release, Draw]` → `["reserve", "consume",
  "release", "draw"]`, the "exactly one market-owned action provider" assertion kept and the doc
  comment naming the paid projection as the fourth op's authority sentence. Reason: lead note 2f;
  IC7 adds `LotOp::Draw`. Nothing else in e7r2 moved: `e7r2_p1_no_path_in_the_market_mints` stays
  true (the Draw is a `transfer`), the edge set gains nothing, every `ProviderRejected` carries
  `LOT_REFUSED`, and `boot.rs` stages no submission.
- **`crates/mush-contract-tests/legacy-handlers.toml`**: the `+market` and `@market` notes refreshed
  (not joined) with a doc 18 cut IC7 paragraph each; category authoritative unchanged;
  `PINNED_BASELINE` unchanged (no new primary verb).
- **Not moved, by inspection and by the gate:** the panic budgets (no new
  `unwrap`/`expect`/`panic!`/indexing in production; the new slices use
  `get(..).unwrap_or_default()`, the one `unwrap_or(ContractId(0))` is a log field);
  `state-slot-owners.toml` (the new `module_state_mut::<MarketStore>` in `boot.rs` writes the
  market's own slot); `module-ns-owners.toml` (the `market` fact namespace is a fact provider, not a
  module namespace); the composition-edge pin and every content digest (no tacsim, mush-sim,
  mush-atlas or mush-server change); the restriction-lints census (green). The new typed family
  `market_lot_draws` enters the state-family fingerprint; nothing pins it.
- `panic_budget/*.txt`, `docs/design/*` (this note aside) and `crates/mush-server`, `mush-core`,
  `tacsim`, `mush-sim`, `mush-atlas` show no diff against 3bf2bc90.

### How it was verified

Three lenses reviewed the uncommitted tree (correctness, conformance, tests) and a fixer closed all
seven should-fix findings (R1 the mirror cursor, R2 the help/doc texts, C1 the lens read, T1–T4 the
coverage gaps); two repair passes rebutted the gate's two environmental failures with reruns; the
judge then re-read the whole diff and the new files against the brief and doc 18. **The final
gate**, on Windows at the tree about to be committed, all fourteen commands green on the first run,
no reruns (logs under the run's `cuts/IC7/gate3/`):

- `cargo fmt --all -- --check` → exit 0, no output (run twice).
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s)``, exit 0 (run twice).
- `cargo test -p mush-market` → lib `test result: ok. 21 passed` (incl.
  `boot::tests::ic7_a_draw_mirror_lap_is_ceil_n_over_the_budget_fires`); e7r2 `21 passed`; ic0 `5
  passed` (unchanged); **ic7 `test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0
  filtered out; finished in 0.05s`**; projection_bounds `2 passed`; projection_cost `1 passed`;
  doc-tests `0 passed; 1 ignored`.
- `cargo test -p mush-economy` → lib `37 passed` (incl.
  `ic7_only_an_active_escrow_free_lease_is_exempt_from_the_stale_page`); e4b1 `test result: ok. 85
  passed; 0 failed` (IC6's test unchanged and green, the two IC7 tests added); e2a 12, e2b 33,
  e3_verify 12, e3a 46, e3b 35, e3b2 33, e4a 83, e4b2 117, e4c 44, e7c11 1, all ok.
- `cargo test -p mush-inventory` → e7c2 `65 passed`, e7r2 `16 passed`, all ok.
- `cargo test -p mush-contract-tests` → 29 result lines all ok (legacy_handler_allowlist 4,
  panic_budget 7, restriction_lints 11, state_slot_owners 4, module_ns_owners 3, typed_state_lens 6,
  ci_gated_test_inventory 6), run twice.
- `cargo test -p mush-server --test e7r2_proving_loop --test market_issuer` → `6 passed` and `test
  result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.62s`.
- `cargo test -p mush-server --lib -- vf_r1_a_replayed_move_across_an_epoch_change
  vf_b12_the_first_commit_and_the_hit_read_the_same` → both ok; `test result: ok. 2 passed; 0
  failed; 0 ignored; 0 measured; 292 filtered out` (the driver's HIT pins L13 leans on).
- `cargo test -p tacsim --test coop --test economy --test port_tick` → `16 passed`, `4 passed`,
  `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.91s`.
- `cargo test --workspace --exclude mush-bot --no-fail-fast` → 388 `test result:` lines all ok,
  aggregate **7,320 passed, 0 failed, 92 ignored**, exit 0 (the `process_fault.rs:197` panic in the
  log is the WS01-S8 injected fault its suite expects; arena_story 75.88 s, ipc_authoritative_action
  8 passed, two_process_e2e 4 passed, scripting_e2e 1 passed).
- `cargo test -p mush-bot` → 22 result lines all ok (lib 469, arena_lane 87, e2e 49, arena_e2e 9 in
  210.68 s, playtesting 9), exit 0.

**Mutation check** (the fixer's, reverted): disabling the Consume arm's `matches_paid` call → `test
ic7_the_paid_legs_project_only_paid_contracts ... FAILED` at `ic7.rs:1905` (case f), so the L12
cross-check is load-bearing.

**Flakes identified, none in this cut's code:**

- A `link.exe` exit `0xc0000142` (STATUS_DLL_INIT_FAILED) on the first gate's step 8 under heavy
  parallel linking; rerun alone, 2 passed.
- `mush-server --lib world_task::tests::what_still_waits_when_the_loop_ends_is_answered_not_dropped`
  failed once in the first workspace sweep and once in the second gate; run alone it fails about 10%
  of the time both at 3bf2bc90 (4/40) and on this tree (3/40; 18/20 in the second repair), a race in
  the 16b B.5 defer-queue shutdown rig (`db273b1c`) on a minimal world with no market or economy
  code; `git diff 3bf2bc90 -- crates/mush-server` is empty. It passed in the final sweep; it belongs
  on the backlog and the load-sensitive list beside `ipc_authoritative_action`.
- `mushd --test sim_cockpit the_cockpit_verbs_are_contextual_and_the_window_is_a_map`, a TUI
  paint-timing test on the known load-sensitive list, failed once under the sweep and passed alone
  three times (the whole binary 12 passed in 16.27 s).

### Surface for the next cuts

Verified against the diff at the tree committed.

**mush-economy (re-exported at the crate root)**

- `pub trait PartyCapSource: Send + Sync + Debug { fn has_cap(&self, w: &World, principal:
  &PrincipalId, cap: PartyCap) -> bool; }` (contract.rs).
- `EconomyModule::with_party_cap_source(self, Arc<dyn PartyCapSource>) -> EconomyModule`;
  `EconomyStore.party_cap_source: Option<Arc<..>>` (pub(crate), default None, injected at boot);
  `pub(crate) fn module::party_cap(w, principal, cap) -> bool` (table first, then the source; false
  with no economy) used by `accept_contract_as`, `deliver_settled` and `consideration_for`. **IC8's
  tacsim source should answer `PartyCap::Accept` for the port institution orgs it names.**
- `pub fn has_fact_provider(w: &World, namespace: &str) -> bool` (facts.rs).
- `pub(crate) fn exempt_from_stale_page(c: &EconomicContract) -> bool` = Active && is_lease_shaped
  && escrow.is_empty().

**mush-market: seams (module.rs, all re-exported)**

- `pub enum Presence { Here, Ship(ObjId) }` (Copy, Eq); `pub struct VenueRefusal { pub line: String
  }`.
- `pub trait VenueGate: Send + Sync + Debug`: `venue_of_room(&World, ObjId) -> Option<MarketId>`;
  `endpoint_for(&World, actor: ObjId, &MarketId, Presence) -> Option<EndpointRef>`; `admit(&World,
  actor, &MarketId, Option<EndpointRef>, Presence) -> Result<(), VenueRefusal>` (Ok for an
  ungoverned venue); `governed(&World, EndpointRef) -> bool`; `port_seller(&World, &MarketId,
  &PrincipalId) -> bool`.
- `pub struct CapacityPlan(pub serde_json::Value)` with `within_bounds() -> bool`; `pub const
  MAX_CAPACITY_PLAN_BYTES: usize = 2_048`; `pub struct CapacityReadError`; `pub enum CapacityRefusal
  { Refused { fits: u64, line: String }, Unreadable }`.
- `pub trait CapacityProvider: Send + Sync + Debug`: `plan(&World, EndpointRef) ->
  Option<CapacityPlan>` (None = ungoverned; **must be Some only where the gate's `governed` is true,
  or IC8 must carry plans on consume entries**); `check(&CapacityPlan, &CanonicalId, u64, &mut dyn
  FnMut(&CanonicalId) -> Result<u64, CapacityReadError>) -> Result<(), CapacityRefusal>` (read_pool:
  Ok(0) absent, Err unreadable).
- `MarketModule::with_venue_gate(Arc<dyn VenueGate>)`, `MarketModule::with_capacity_provider(Arc<dyn
  CapacityProvider>)`; `pub fn venue_gate(&World) -> Option<Arc<dyn VenueGate>>`; `pub(crate) fn
  capacity_provider(&World)`.
- `pub type DrawReceipts = StateFamilyHandle<ContractId, DrawReceipt>`; `pub fn
  draw_receipts(&World) -> Option<DrawReceipts>` (Some exactly when a lot supply is composed).
- `pub struct MarketFactProvider` (facts.rs; `provider() == "market"`; answers only
  `market.lot.drawn`, revision None or Some(1), id `"{quote}/{accepted_at}"`, from
  `Quote.drawn_at`). **IC8 wires it into `EconomyModule::with_facts`.**

**mush-market: records (record.rs, re-exported)**

- `LotOp::Draw` (word "draw"); `LotOp::is_paid()` true for Consume and Draw.
- `pub enum LotMatter { Hold { endpoint: EndpointRef, commodity: CanonicalId, grade: u8 }, Draw {
  source: EndpointRef, commodity: CanonicalId } }` with `source()`, `commodity()`, `kind() ->
  PaidKind`; `MarketStore::lot_matter(&Listing) -> Option<LotMatter>` (the one accessor; Draw only
  when the entry is `Representation::Lot` and the code parses as a CanonicalId).
- `pub enum PaidKind { Hold, Draw }` (serde snake_case: "hold"/"draw"); `pub struct PaidLot { kind,
  source: LotEndpoint, destination: LotEndpoint, commodity: LotCommodity, grade: u8, accepted_at:
  i64 /* real */, draw_by: Option<i64> /* real; Some exactly for Draw */ }` with `check()`.
- `Quote.lot: Option<PaidLot>` (set at activation of every lot sale, both kinds); `Quote.drawn_at:
  Option<i64>` (real; written only by the mirror; Some only on a Draw quote).
- `pub struct DrawReceipt { contract: ContractId, quote: QuoteId, listing: ListingId, source:
  VaultId, destination: VaultId, commodity: SmolStr, quantity: u64, actor: ObjId, at_ic: i64 }` with
  `check()`; family `market_lot_draws` (descriptors
  `market.lot_draw_id.v1`/`market.lot_draw.v1`/1/`market.lot_draw.decode_json.v1`/`market.lot_draw.invariants.v1`/`market.lot_draw.json.v1`,
  key `format!("{:016x}", contract.0)`).
- `pub const FACT_LOT_DRAWN: &str = "market.lot.drawn"`; `pub fn draw_fact_id(QuoteId, i64) ->
  String`.

**mush-market: constants and functions (trade.rs, boot.rs, lot.rs; re-exported)**

- `DRAW_WINDOW_SECS = 3_600`, `DRAW_SETTLE_MARGIN_SECS = 600` (real seconds);
  `MAX_DRAWS_MIRRORED_PER_FIRE = 64`; `MAX_PAID_ENTRIES_PER_KIND = 8` (=
  `MAX_OPEN_QUOTES_PER_BUYER`).
- `PORT_ONLY_ENDPOINT`, `CAPACITY_UNREADABLE`, `DRAW_TOO_LATE` (texts as above).
- `QuoteRequest.presence: Presence` (commands and the economist pass Here).
- `pub fn accept_at(ctx, QuoteId, Option<EndpointRef>, Presence) -> Result<ContractId,
  MarketError>`: **IC11's executor calls it with `Presence::Ship(hull)` and `None`** (the gate's
  `endpoint_for` supplies the berthed hold); `accept_into` = `accept_at(.., Here)`; `accept`
  unchanged.
- `pub fn restage_draw(ctx, QuoteId) -> Result<Option<ContractId>, MarketError>`: Ok(None) not
  re-stageable (not the actor's, not activated, not a Draw, drawn/settled, no contract, contract not
  Active); Err(DRAW_TOO_LATE) past draw_by; Err(undeliverable) when stage failed; Ok(Some) re-staged
  byte-identically. **IC11 re-stages through it.**
- The Draw sale's contract: conditions `[Before { RealUtc, accepted_at + 3600 }, Fact { market,
  market.lot.drawn, "{quote}/{accepted_at}", None }]`, deadline the same instant, `Expiry::Refund`.
- Stage keys: paid legs `market.lot.{consume|draw}.{contract:x}` with parameters `{op, listing,
  endpoint, source, commodity, quantity, grade, contract: "<decimal>"}` (nothing time-varying);
  listing legs `market.lot.{reserve|release}.{listing:x}.{ic second:x}` with no `contract`.
- The `market_lot` offer: fixed parameters `{"now": <ic>, "paid": {"<contract decimal>": {op, quote,
  listing, source, destination, commodity, quantity, grade, plan}}}`; op choices `reserve`,
  `consume` (iff a consume entry), `release`, `draw` (iff a draw entry); `contract` Choice over the
  entries' decimal ids, `required: false`, present iff any entry. A draw entry requires `now <=
  draw_by`; a consume entry covers Active, ReadyToSettle and Settled hold contracts.
- `pub(crate)`: `lot::draw_receipt(ctx, ContractId) -> Result<Option<DrawReceipt>, ()>`
  (owner-scoped through `Ctx::action_state`; answers only on the market's own contexts),
  `lot::pool_balance` / `lot::pool_available(ctx, EndpointRef, &CanonicalId) -> Result<u64, ()>`,
  `lot::register_draws(reg) -> Result<DrawReceipts, ModError>`, `lot::register(reg,
  &InventoryFamilies, DrawReceipts, Option<Arc<dyn CapacityProvider>>)`,
  `trade::awaits_receipt_read(&Quote) -> bool`, `trade::release_quote` (the no-relist guard inside),
  `MarketStore.draw_cursor: Option<QuoteId>` (RAM only).

**Behaviour IC8 and IC11 can rely on:** an undrawn Draw quote whose contract is Cancelled/Expired is
released by `boot::reconcile_draws` on the market's 60 s sweep (not in the economy's fire); a Draw
listing deleted at zero is never re-minted by `restore_quantity`; `+market/accept <q>` and `<q>
to=<vault>` are one command; `+market/list`/`/depth` default to `venue_of_room` of the actor's room
when a gate names one.

## IC8

**Finite stock and port commerce** (doc 18 §5, the IC8 row and paragraph), built 2026-10-16 on main
at d8427081. The two IC lanes join: IC7's market and economy seams get their tacsim implementations
and the content that uses them. Every unit of trade matter at the two ports is minted once by a
per-port stock clerk Thing acting on the `tacsim.port` tick, receipted in `tacsim.port_stock`,
dissolved into its storefront pool and listed as a Draw; the applier writes the plan rows, vaults,
vendors, grants and Wanted documents and submits nothing. A chartered pilot at the Exchange lists,
quotes and accepts with no destination and the Draw loads the berthed hold under the port's presence
gate, the hull's capacity provider and the port institutions' party caps, with every refusal in
§4.3's own sentences and nothing charged; the boards 18a names count down to SOLD OUT. One commit,
pushed to origin/main.

### What landed

1. **The core seam** (`mush-core/src/output.rs`, `engine.rs`, `cmd.rs`):
   `Outbox::note_pending_authoritative`/`authoritative_pending` and `Ctx::authoritative_pending`.
   `fire_due` notes the engine's queued actors before it pops anything and clears the note after its
   one `drain_deferred`, so a tick acting as a principal defers instead of drawing the back-pressure
   refusal or fanning out with an earlier-in-slice submission. A scope extension past the §5 row's
   crates column (L22), with its unit test
   `ic8_authoritative_pending_sees_the_engine_queue_and_this_outbox`.
2. **The tick's two arms** (`tacsim/src/port_tick.rs`): `SwapRefused::Pending` asked first in
   `Fire::as_principal` (no act, no debt); `Arm::{Stock, Release{listing}, Probe}` in that order;
   `walk` runs `release_verify`, offers every present lane port map's clerk while the lane is
   latched, then at most one release agent, then the probes; `PortTickState` gains `stock_action`,
   `releasing`, `fires`, `paged`, `release_warned` and `release_backoff`; `Step::Worked` is live;
   `probe_error` became the crate-shared `port_error` so the panic budget did not move;
   `drift_pages(&World)` reads the boot's pages.
3. **`tacsim_port_stock`, tacsim's first production action provider** (`tacsim/src/port/stock.rs`):
   the outcome and action descriptors register on every roster, the `StockProvider` only when the
   inventory cell is filled. The offer carries the clerk's stock rows as server-authenticated fixed
   parameters derived from the port map at projection (L4); the prepare re-derives the
   `StockIssueId`, requires the row's clerk to be the actor and its key and lot to match, refuses a
   receipted row with `Conflict` ("that stock issue is already minted"), and writes `mint_origin`
   (`Lineage::Origin`, `ExternalRef{tacsim, port.stock, <pack>/<entry>/<org>/<good>, Some(issue)}`)
   plus the `StockReceipt` in one bundle. Keys `tacsim.port.stock/<key>` and
   `tacsim.port.dissolve/<key>`. Every reader re-checks `receipt.issue_id() == key` and fails
   closed, logged once (`receipt_at`/`receipt_in`; `families.rs`'s key rule narrowed).
4. **The stock arm** (`stock.rs` `stock_step`): one step per clerk per fire in authored order — mint
   a row with no receipt, dissolve a minted lot still standing (`inv_dissolve` as the clerk), then,
   every row seeded, list the first ask with no live listing, no Draw in flight and a pool above 0
   (`list_goods`, `Supply::Stock` on the storefront, `min(pool, MAX_QUANTITY)`,
   `MAX_LISTING_TTL_SECS`): L5's one rule seeds, relists after a lapse or a re-price, and relists
   refunded Draw tonnes. The drift check (`pool < listed`, or `pool > listed + undrawn` under a live
   listing) pages once per boot per pool with a `runbook` field (L19).
5. **The lapsed-hold release arm** (`stock.rs` `release_verify`/`release_agent`/`release_step`): the
   lowest-id lapsed hold-backed listing whose vault a live player owns — org-owned, GOD-owned and
   gone-owner vaults skipped and logged once, refused listings backed off `RELEASE_GRACE_FIRES`
   (24), everything filtered through `mush_market::lapsed_holds_where` before the window is taken so
   nothing starves a later release — is released as the holder through `release_lapsed_hold`;
   `release_verify` runs at the top of every fire, tells the holder once ("Your market listing #{id}
   lapsed; the {qty} of {name} it held are free again.") only when the listing is gone (the pull
   verification), and drops a release not seen to land in 240 fires. The GOD sweep never releases
   anything.
6. **`PortApplier` parts d–e** (`tacsim/src/port/vendor.rs` new; `apply.rs`, `def.rs`, `map.rs`):
   the goods catalog loaded on every apply; per vendor in authored order the port's one stock clerk
   Thing (DARK, in the staff spine, owned by `ctx.actor`; L2), the board rooms (L15), the storefront
   item and signatory npc resolved by authored name in their authored room from the station pack on
   the atlas shelf at first apply only and by map row ever after (L3), the org-owned storefront
   vault on `CustodyRef::Container`, the market vendor, the clerk's direct `vault.withdraw` grant in
   the org's root scope (L25), the stock plan rows (`StockRow{good, issue, qty, grade, id, lot}`),
   the standing bids through `post_wanted` with revise-in-place and the lapsed/exhausted re-post
   rule, and an ask re-price withdrawing the stale Draw for the arm to relist. `preflight` refuses
   before any write: market composed, catalogue entries, every price through `minor_of`, every
   afresh bid's intake, no unrecorded twin, the vendor listing cap, recorded objects live and the
   storefront in the spine, board rooms, recorded stock-row drift (L6, paged with a runbook) and bid
   drift. `retire_check` refuses a vendor with an open deal; `retire` withdraws only Draw and Wanted
   listings, logs any other, and forgets the bid rows it withdrew so a later apply posts them
   afresh. The allocator reaches stay in `apply.rs` (`alloc_vault_id`/`alloc_grant_id`). `PortMap`
   gains `clerk`, `vendors` and `boards` (skipped when empty; `PORT_MAP_SCHEMA` stays 1);
   `AskDef`/`BidDef` gain `boards`; `PortDef::check` and `check_ports` gain the one-clerk,
   unique-goods, ask-has-stock, no-ask-and-bid, board-role, intake-cap, cross-port-agent,
   goods-existence and vendor-scoped reference rules.
7. **`Supply::Wanted`, the lapsed-hold rule and the release entry**
   (`mush-market/src/{record,store,api,trade,commands,projection,boot,lot,lib}.rs`):
   `Supply::Wanted{vault, commodity, min_grade}` with price and remaining on the listing (L21),
   wire-checked, excluded from `lot()`, `lot_matter`, `lot_listings` and the projection;
   `list_goods` refuses it first; `quote` and `accept_at` refuse it before any write with
   `WANTED_IS_A_BID`; the `+market/list` row with ` — not buying`, the `+market/show` line;
   `WantedSpec`, `post_wanted`, `revise_wanted`, `MarketStore::wanted_of`. The sweep keeps a lapsed
   hold-backed listing while its hold reads Live, Lapsed, Unreadable or None, filtered out before
   `take(budget)`, and deletes it on Gone (L7). `LapsedHold`, `lapsed_holds`, `lapsed_holds_where`
   and `release_lapsed_hold` (staged as `ctx.actor` through `lot::stage`, deleting nothing; L20).
8. **The tacsim market seams** (`tacsim/src/port/market.rs` new; `capacity.rs`;
   `composition/mod.rs`): `governed_hold` as the one predicate; `PortVenueGate` (`venue_of_room`
   over the gallery, every room but the spine and the office — L11; `endpoint_for(Here)` the charter
   hull's hold wherever it lies — L10; `Presence::Ship` wired for IC11; the five texts verbatim);
   `PortCapacity` (`HoldPlan{v, goods digest, compartments}`, no goods rows — L12; `check` through a
   `OnceLock` of the compiled-in table, every accepted pool read, `Unreadable` on any failure;
   `capacity_line` in §4.3's sentences); `compartment_for`; `PortPartyCaps` (vendor orgs and the
   programme org, no money authority). Wired only where the TacSim layer composes;
   `MarketFactProvider` wired on every roster as the sixth namespace (L13).
9. **Readiness and the gate** (`tacsim/src/port/readiness.rs`, `coop.rs`, `commands.rs`):
   `Binding::{Pools, Listings, Bids, MarketFacts}` and `readiness_in(&Ctx)` splicing the
   market-facts line after the treasury and, per port, one Pools line per vendor, one Listings line
   per ask and one Bids line per bid (L9); `@tacsim/port status` and `+coop/enrol` both read it;
   `enrol_gate` also requires Pools, Listings and MarketFacts Ready, Bids reported only (L8).
   `+coop/enrol` checks the `Enrolment` document at plan time and again before `put_enrolment`,
   clamps the laid-up name to 120 bytes on a char boundary (L18), and `BOOKS_UNREADABLE` is reworded
   (L16).
10. **The boards** (`tacsim/src/port/board.rs` new; `lib.rs`): a `LookRendered` hook rendering, for
    a room the port maps record under a board role, every ask then every bid of every vendor whose
    `boards` names that role — `{price}/{unit}, {q} {unit} left ({house}; signed {signatory})`,
    `SOLD OUT: seeded {s} {unit}, 0 left`, `not yet on the board`, `wanted, …: {price}/{unit}, {r}
    {unit} more` and `wanted, …: not buying` — thousands grouped, read through the module's own
    lens, nothing written.
11. **Content**: lane v3 (`content/lane/ceres_phobos.json`; `LANE_VERSION = 3`) adds only the two
    `vendors` arrays (Kasei's intakes D-18-27's 4,000/9,600; the boards transcribed from 18a room by
    room); Dray v2 (`content/units/dray.json`) drops the Airlock's `lock` alias and every figure and
    engine word from the room prose (L24) except the `note`, which stays at its v1 wording; Stickney
    v2 (`content/atlas/neighborhood.stickney_yards.json`) rewrites the P11 fittings card. `hull.rs`
    re-dresses exits already made (name and aliases follow the template; `interior.exits` are
    append-only by position). `exit_command_words.rs` walks the Dray's interior exits; `DECIDED` is
    unchanged.
12. **Rosters and the two mush-server pins** (`match_/centre.rs`,
    `mush-contract-tests/tests/legacy_handler_allowlist.rs`,
    `mush-server/tests/e7r2_proving_loop.rs`, `mush-server/src/{modules,world_task}.rs`): the three
    rosters moved to `server_modules_with_tacsim` + `tacsim_module_wired`;
    `ic8_the_game_roster_hands_the_layer_a_filled_inventory_cell`;
    `ic8_the_clerks_first_mint_drives_through_the_command_ingress` (the clerk's mint through
    `command_ingress` → preflight Vacant → offer → stage → reserve/reconcile/install/checkpoint,
    then a replay after an epoch change refused by the receipt).
13. **Tests**
    (`tacsim/tests/{commerce,capacity,coop,ports,port_tick,hulls,lane,stations,content}.rs`,
    `tests/support/vertex.rs`, `mush-market/tests/ic8.rs`): `vertex.rs` (`both_limits_vertex`,
    `best_integer_load`); the LP test and `compartment_for`; `commerce.rs`'s twenty tests over
    lenses 1–8 plus the Ship arm, the unreleasable-listing head of line, retire and re-apply, v2→v3
    by the shelf against `LANE_V2_DIGEST`, the re-price relist and IC7's three gaps; `coop.rs`'s
    derived gauge fixtures, the stocking loop, the closed-until-stocked and clamp tests;
    `ports.rs`'s fourteen refusal cases and three drift variants; `port_tick.rs`'s `wired(..,
    game_content)` and the Pending test; `ic8.rs`'s nine market tests with exact `MarketError`s.

The LP test derives both vertices from `goods.json`, the Dray and the posted prices and asserts them
optimal; no code, fixture or test stores 20/80 or 55/45.

### Deferred, and why

The owner-critical and owner-reviewable rulings first, then the planner's other lead rulings, the
builder's one deviation, the residuals and the cuts that inherit the rest.

- **The Dray `note` sentence.** The brief's wording takes the note to 1,046 bytes against
  `MAX_NOTE_BYTES = 1_024` (`definitions/mod.rs:89`) and every catalog bind refuses the pack, so the
  note stays at its v1 text while the author notes and figures it describes are gone. The owner owes
  a wording under 1,024 bytes; a respin follows it. Every other Dray v2 edit is made.
- **L1 (owner-critical): Kasei's intakes are D-18-27's 4,000 t of metals and 9,600 t of volatiles**,
  not the lead note's 8,500/3,600; D-18-27 and §4.3/§4.7 are owner-binding and the lead note is not.
- **L6 (owner-critical, stricter than Q-18-19's default): any change to a recorded stock row's
  `issue`, `qty` or `grade`, or its removal, is refused as drift and paged, receipted or not.** The
  applier runs on the content context and cannot read tacsim's receipts, so it cannot tell a minted
  row from an unminted one. Q-18-19 is the owner's question.
- **L7 (owner-critical): the market sweep no longer deletes a lapsed hold-backed listing until its
  hold reads Gone**, on every roster; kept listings never spend the sweep budget; the release is the
  holder's submission by the tacsim arm, and on a roster without TacSim the seller withdraws by hand
  (IC0's note). A generic-path behaviour change for D-18-26 and Q-18-31.
- **L15 (owner-critical): 18a contradicts itself on the Yard's fittings ask** (P11's Items call the
  card "the one place the figure is written"; P08's Items and P11's Purpose put it on P08's board
  too). Default: render it on both P08 and P11. The Co-op provisions ask gets no board, because 18a
  gives none. P07's "funded balance" figure is IC10's.
- **L17 (owner-critical): the accept line naming the listing's signatory (§4.3) is deferred to
  IC12**; the market's accept text is generic and IC7's seams are frozen; the boards name each
  line's signatory now.
- **L24 (owner-reviewable): the Dray's room prose carries no figures and no engine words** — the
  lead note's meta-text and the load cells' LIMIT figures are out, the gauges render every figure
  from `logistics`. The Stickney P11 card text is the brief's. The gate lines, readiness lines,
  board lines, `BOOKS_UNREADABLE` and the release notice are the brief's voice.
- **L2: one stock clerk per port**, `VendorDef.agent` kept as its id; §4.7's per-vendor sample
  agents are illustrative (D-18-26 names a per-port clerk).
- **L3: storefronts and signatories resolve by authored name in their authored room** from the
  station pack on the atlas shelf, at first apply only, by map row after; the latch is unreachable
  inside an apply (IC3 L1) and items and NPCs carry no record key.
- **L4: the mint's offer carries its rows as server-authenticated fixed parameters** and the prepare
  re-derives the issue id and refuses a receipted row (the IC7 `paid` precedent).
- **L5: the list step is one rule** (no listing, no open Draw, pool > 0), closing IC7 L1's un-listed
  tonnes without a market change.
- **L8: the enrol gate takes Pools, Listings and MarketFacts Ready and does not gate Bids**;
  Listings Ready includes the draw-mirror slack; a Pending listing closes it for the fires the arm
  takes.
- **L9: `readiness(&World)` stays world-only; `readiness_in(&Ctx)` adds the IC8 lines**, because
  pools and receipts are typed state.
- **L10/L11: `endpoint_for(Here)` answers the charter hull's hold wherever it lies and `admit`
  refuses in the gate's words; "the port's rooms" are the gallery, every `rooms` value but the
  spine, and the programme office.**
- **L12: the capacity plan carries the compartments and the goods table's digest, no goods rows**;
  the check loads the compiled-in table through a `OnceLock` and fails closed on a digest mismatch;
  the "packs to" figure is the load after the addition.
- **L13: `MarketFactProvider` on every roster; the gate, the capacity provider and the party-cap
  source only where the TacSim layer composes.**
- **L14/L21: `Supply::Wanted` lands as a document `{vault, commodity, min_grade}` with price and
  remaining on the listing**, a recorded deviation from §4.3's shape so there is one truth for each;
  selling into it is IC10's.
- **L16: `BOOKS_UNREADABLE` reworded rather than paged**; no staff-alert path exists for a tacsim
  command.
- **L18: the laid-up name is clamped to 120 bytes at plan time, not refused.**
- **L19: pages are `tracing::error!` with a `runbook` field**, once per boot per pool.
- **L20: the release is keyed as every market release is** (`market.lot.release.{listing:x}.{ic
  second:x}`); the IC8 row's "derived reservation id" is the reservation released, not the key; a
  re-stage past the grace is refused by `require_held`, never double-applied.
- **L22: the mush-core seam is a scope extension** past the row's crates column, for IC5's residual
  and lead 2c-i.
- **L23: the reference-has-an-ask rule applies only to an entry that authors vendors**, so the
  published v2 keeps loading (test 16 rebuilds its bytes against `LANE_V2_DIGEST`).
- **L25: the clerk's `vault.withdraw` is a direct grant**, no membership; `authorize` honours direct
  grants.
- **`BidRow.expires` added** (a builder deviation): with the brief's three fields an applier cannot
  tell a bid gone because it lapsed (re-post) from one gone because it was used up ("not buying").
  Retire now forgets the rows it withdrew, so only the staff-withdraw case reads "its intake is
  taken" before its expiry — IC10's exhausted marker closes it.
- **Item 1's residual, stated and not fixed:** a principal's own `@wait` line that fires later in
  the same `fire_due` slice than the tick still fans out with the arm's submission.
- **Not covered, latent and content-bound:** a stock row above `MAX_QUANTITY` reads as permanent
  drift; a port with more than `MAX_CLERK_ROWS` (64) rows, or one row whose mint is always refused,
  stalls that port's arm; a storefront that already backs a vault is not refused by preflight;
  standing bids lapse at 90 days with no re-post arm (IC10's call); a retired port entry's clerk
  still runs while the lane stays latched (a later ruling); the four Pools missing forms and the
  "its intake is taken" text have no test.
- **IC10:** the `+market/sell` lot arm into `Supply::Wanted`, `LotOp::Deliver`, `transfer_marked`,
  the `inventory.transferred` Fact gate, `ProceedsPolicy`, the Wanted `remaining` restore, P07's
  funded balance. **IC9:** the passage planner, the burn, undock and docking. **IC11:** the orders
  document, the executor and the callers of `Presence::Ship`. **IC12:** `trade.md`, the signatory
  accept line, the full readiness gate. **IC13:** buyout, release, revoke, dormancy. **P5:** restock
  and top-ups.

### Pins re-blessed

- **`tacsim.lane.ceres_phobos` 2 → 3**: digest
  `3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0` →
  `8d20125693884a8c6739d6dd032d858e474c015726eab54b05dccb17164d0ad4` in the pack and `tests/lane.rs`
  `LANE_DIGEST`; `LANE_VERSION = 3`; every `@2`/`Some(2)` lane pin moved one (`tests/lane.rs`
  :4/:176/:582/:814, `tests/stations.rs`, `mush-server/src/config_ext.rs`, `boot_parity.rs`'s
  comment); the test-only `lane_v3` helpers in `tests/ports.rs` and `tests/hulls.rs` became
  `lane_v4` at version 4 with every "Plan for"/"Applied"/"ships @N"/latch pin moved one;
  `ic3_a_world_on_lane_v1_takes_v2_by_the_shelf` now takes v3 (counts "2 create, 2 update, 0 retire,
  8 unchanged" unchanged). Reason: v3 adds the vendors; a published version is never edited; the old
  v2 digest lives on as `LANE_V2_DIGEST` in `tests/commerce.rs` and test 16 rebuilds the published
  v2 bytes against it.
- **`tacsim.pack.dray` 1 → 2**: `DRAY_DIGEST` `da86f389…` →
  `2f11e37f4de1f02df1f5d19aba599c4235c74307c55a6a13201e74184741cabb`; `version() == 2`;
  `tests/content.rs:2696` `"Airlock <A>;airlock;lock;forward"` → `"Airlock <A>;airlock;forward"`.
  Reason: the `lock` alias and the prose figures go (L24, IC4 C7); the note edit did not fit
  `MAX_NOTE_BYTES`.
- **`neighborhood.stickney_yards` 1 → 2**: `PINNED` digest `6a2cb600…` →
  `39aab74e2f7a56e2b71ee7cffcdd34168ad706fbc1317240d989d2dc95c42d42`; the "version 1 for every
  station pack" pins in `tests/stations.rs` (:331/:418/:810/:937) and
  `mush-server/tests/boot_parity.rs` became a `station_version(pack)` helper (2 for Stickney, 1 for
  the rest). Reason: the P11 fittings card's text.
- **The D-18-39 literals**: `src/port/gauges.rs`'s three unit tests and two doc examples moved from
  `metals 20 / volatiles 80` to the three-good FULL-both load `metals 29, volatiles 61,
  industrial_parts 10` (`29 t · 11.6 m³`, `61 t · 85.4 m³`, `10 t · 23.0 m³`), which is no two-good
  vertex; `tests/capacity.rs` `ic2_a_balanced_load…` re-derived from `both_limits_vertex` and
  `ic2_20_plus_81_is_refused_on_both` renamed `ic2_one_past_the_vertex_is_refused_on_both` with `(d,
  b+1)` computed; `tests/coop.rs`'s `LOAD_CELLS`, `SUMMARY_BODY` and `seed_outbound` became
  functions over the vertex. Reason: no code, fixture or test stores 20/80 or 55/45.
- **The readiness report** (`tests/ports.rs`
  `ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`): the market-facts line after
  the treasury and, per port, 3 Pools + 4 Listings + 2 Bids lines read before any tick (Pools
  Pending "0 of n", Listings "not seeded yet", Bids "not buying until its capital settles"); the
  total "9" became "9 + 18". Reason: `readiness_in` adds them.
- **The ports.rs behaviour pins narrowed, then tightened** (`ic3_boot_creates_the_nine_orgs…`,
  `ic3_an_update_reuses_every_map_row…`, `ic3_a_cancelled_tranche_reads_missing_not_pending`):
  `!contains("[pending]")` and `ends_with("Ready: every binding above is in place.")` became "every
  vendor/ask line ends `[pending]`, every other bracketed line `[ready]`, at least one stock line
  present" and "every pending line is a vendor, ask or bid line". Reason: this file never lands the
  port tick, so the stock lines wait on it; the capital claim is kept whole and no stock line may
  read missing or flagged.
- **First-boot counts**: `tests/stations.rs` `items` 6 → 8 (the two DARK clerk Things in the
  spines). `mush-inventory/tests/e7r2.rs` `FactProvider` count 5 → 6 (the sixth namespace,
  `market.*`).
- **`tests/port_tick.rs`**: `wired(tw, cfg)` gained `game_content`; the IC5 probe tests pass `false`
  and `unwired` turns game content off (with it on, the two clerks join the walk and move the exact
  per-fire sets the tests pin); `ic5_production_registers_no_probe` also asserts the production
  `tacsim_port_stock` definition.
- **`tests/coop.rs` `ic6_the_desk_closes_at_a_play_world_rate_other_than_24`**: stocks the ports
  before `set_rate(4)` and asserts the rate line is the only non-ready one, so the rate is the one
  binding that refuses (fixer T1).
- **`legacy-handlers.toml`**: the `+market`, `@market` and `@tacsim` notes refreshed (Wanted rows,
  the lapsed rule, the readiness lines); `PINNED_BASELINE` stays 97 — no new primary verb.
  **`state-slot-owners.toml`**: one row for `crates/tacsim/tests/commerce.rs`
  (`World::module_state_mut`, `AccountOracle`, owner `sim`, the coop.rs reason); no row for
  `vendor.rs` or `stock.rs`, whose allocator reaches stay in `apply.rs`.
- **Unchanged**: the panic budgets (`tacsim.txt` 5, `mush-market.txt` 2, `mush-core.txt` 121;
  `port_error` is shared), the module-namespace owners, the composition-edge pin, the CI gated-test
  inventory, `exit_command_words.rs`'s `DECIDED`, `mush-market/tests/e7r2.rs`'s market-never-mints
  pin, and `families.rs`'s golden `StockIssueId` pin.

### How it was verified

Three lenses reviewed the uncommitted tree: a correctness lens (the mint's receipt guard, the
fixed-parameter rows against a submitter, the Draw accounting across readiness/drift/board/IC7's
restore, the venue gate and plan from one predicate — one must-fix, the release head of line), a
conformance lens (every brief text checked verbatim against the code, every §3 item against the
diff, the four declared deviations, the `Presence::Ship` test gap) and a tests lens (vacuous and
weakened tests: the rate test, the depth test, the plan/governed skip, the missing `PortDef` cases,
the untested Ship arm). All 23 findings were handled by the fixer — 22 fixed with new or tightened
tests, and conformance F3's staff-withdraw half deferred to IC10 — and the judge re-read the whole
diff afterwards (verdict: ship with notes).

**The gate**, run in order from the run's `cuts/IC8/gate1/run.sh`, each command under `timeout
5400`, all green on the first pass with no rerun, no LNK1104 and no hung binary:

- `cargo fmt --all -- --check` → exit 0, no output.
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s) in 0.61s``, exit 0, no warning lines.
- `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test lane ic2_the_lane_pack_is_pinned` → `test
  result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out`; `--test content
  ic2_the_dray_is_pinned_and_checks` → `ok. 1 passed … 28 filtered out`; `--test stations
  ic1_the_station_packs_load_and_their_digests_are_pinned` → `ok. 1 passed … 12 filtered out`. The
  sha256 of all 40 files under `crates/tacsim/content` was unchanged by the three re-sign steps: the
  cut's content was already signed.
- `cargo test -p mush-core --lib output` → `ok. 13 passed; 0 failed; 383 filtered out`.
- `cargo test -p mush-market` → lib 21, e7r2 21, ic0 5, ic7 18, ic8 9, projection_bounds 2,
  projection_cost 1, all `0 failed`; doc-tests 0 passed, 1 ignored.
- `cargo test -p mush-inventory` → e7c2 65, e7r2 16, `0 failed`. `cargo test -p mush-economy --lib`
  → `ok. 37 passed`.
- `cargo test -p tacsim --lib` → `ok. 891 passed; 0 failed`.
- `cargo test -p tacsim --test commerce --test ports --test hulls --test coop --test port_tick
  --test capacity` → capacity 17, commerce 20, coop 18, hulls 15, port_tick 16, ports 17, all `0
  failed`.
- `cargo test -p tacsim --test lane --test stations --test content --test definitions --test
  composition_edges` → composition_edges 2, content 29, definitions 25, lane 8, stations 13, `0
  failed`.
- `cargo test -p tacsim --test match_lifecycle --test cockpit` → cockpit 180, match_lifecycle 136,
  `0 failed`.
- `cargo test -p mush-server --lib world_task::tests::ic8` → `ok. 1 passed; 295 filtered out`;
  `--lib modules::` → `ok. 32 passed; 264 filtered out`; `--test boot_parity --test
  e7r2_proving_loop --test market_issuer` → 18, 6, 4, `0 failed`.
- `cargo test -p mush-contract-tests` → 29 result lines all ok (127 passed, 0 failed, 8 ignored),
  among them `exit_command_words` 2, `legacy_handler_allowlist` 4, `state_slot_owners` 4,
  `module_ns_owners` 3, `restriction_lints` 11, `panic_budget` 7.
- `cargo test -p mushd --test boot_composition` → `ok. 6 passed`.
- `cargo test --workspace --exclude mush-bot --no-fail-fast` → rc 0, 390 `test result:` lines every
  one ok (**7,362 passed, 0 failed, 92 ignored**), no FAILED line; the one panic printed is
  `server_panic_relaunch`'s own armed fault (`WS01-S8`), all 5 of its tests passed. 68 minutes,
  about 45 of them linking some 300 test binaries under memory pressure (25 parallel link.exe at
  ~109 GB private on a 64 GB box); progress never stalled.
- `cargo test -p mush-bot` → 22 result lines all ok (704 passed, 0 failed, 5 ignored): lib 469,
  arena_e2e 9 (184 s), playtesting 9 (125 s), e2e 49.

No flake was identified.

### Surface for the next cuts

**mush-core** (`output.rs`, `cmd.rs`)

- `Outbox::note_pending_authoritative(&mut self, impl IntoIterator<Item = ObjId>)` (replaces the
  set; called only by `Engine::fire_due`, once before the pop and once empty after the drain).
- `#[must_use] Outbox::authoritative_pending(&self, ObjId) -> bool` (true when noted or when this
  outbox holds a `Deferred::AuthoritativeSubmit` for the actor; blind to a later-in-slice `@wait`).
- `Ctx::authoritative_pending(&self, ObjId) -> bool`.

**mush-market** (`lib.rs` re-exports)

- `Supply::Wanted { vault: VaultId, commodity: LotCommodity, min_grade: u8 }` (serde
  `{"wanted":{…}}`; `seller` = the bidding org, `price` = the bid per unit, `quantity` = the intake
  still wanted; `Supply::is_wanted()`; excluded from `lot()`, `lot_matter`, `lot_listings`,
  `publishes()`).
- `pub const WANTED_IS_A_BID = "That listing is a bid: its house buys that good and sells none.
  Nothing was charged."` (quote and accept_at before any write).
- `WantedSpec { vendor, catalog, price: Amount, remaining: u64, vault: VaultId, min_grade: u8,
  ttl_secs: i64 }`; `post_wanted(ctx, WantedSpec) -> Result<ListingId, MarketError>` (refusals in
  order: NoStanding(NO_SUCH_VENDOR); "That vendor is not trading, so it posts no bid.";
  NoStanding(NO_SUCH_ENTRY); "A standing bid buys a lot-represented good."; "That catalogue code is
  not a commodity id."; "A standing bid receives into the vendor's own storefront vault."; "A
  standing bid offers a positive price."; "That is not a quantity this market buys."; "That grade
  floor is out of range."; "That vendor already posts a standing bid for that good."; the vendor and
  market caps; writes revision 1, Public, RealUtc, ttl clamped 60..=MAX_LISTING_TTL_SECS).
- `revise_wanted(ctx, ListingId, Amount, u64) -> Result<(), MarketError>` (Wanted only, else
  NoStanding(NO_SUCH_LISTING); revision bumped; expiry unchanged); `MarketStore::wanted_of(VendorId,
  &CanonicalId) -> Option<ListingId>`.
- `list_goods` with a Wanted → Bad("A standing bid is posted with post_wanted, never listed.")
  first; `withdraw_listing` of a Wanted or a Draw deletes and stages nothing.
- `+market/list` row `#{id} {name} — wanted {q} × {price} [{market}] by {shop}` plus ` — not buying`
  when `vendor.account` available < price or unreadable; `+market/show` `  supply: a standing bid;
  the house buys this`.
- `LapsedHold { listing, endpoint, commodity, quantity, grade, name }`; `lapsed_holds(&World, limit)
  -> Vec<LapsedHold>` and `lapsed_holds_where(&World, limit, impl FnMut(&LapsedHold) -> bool)` (live
  hold-backed listings past their RealUtc deadline, ascending id, filtered BEFORE the limit).
- `release_lapsed_hold(&mut Ctx, ListingId) -> Result<(), MarketError>` (no listing or no custody →
  NoStanding(NO_SUCH_LISTING); not hold-backed → Bad("That listing holds no stock, so there is
  nothing to release."); not lapsed → Bad("That listing has not lapsed; withdraw it to give its
  stock back."); staging failure → Bad; stages a Release as `ctx.actor`, keyed
  `market.lot.release.{listing:x}.{ic second:x}`, deletes nothing).
- Sweep rule: a lapsed listing whose `lot_matter` is Hold is deleted only when `hold_standing ==
  Some(Gone)`; kept otherwise and never counted against `MAX_SWEPT_PER_FIRE`.

**tacsim::port** (re-exports)

- `PortMap.clerk: Option<ClerkRow { agent: CanonicalId, thing: u32 }>`, `PortMap.vendors:
  BTreeMap<org, VendorRow { storefront: u32, vault: u64, vendor: Option<u64>, signatory: u32, grant:
  Option<u64>, stock: Vec<StockRow>, bids: BTreeMap<good, BidRow> }>`, `StockRow { good, issue: u32,
  qty: u64, grade: u8, id: String /*64 hex*/, lot: u64 }`, `BidRow { listing: u64, price_major: u64,
  intake: u64, expires: i64 }`, `PortMap.boards: BTreeMap<role, u32>` (all deny_unknown_fields,
  skipped when empty, `PORT_MAP_SCHEMA` 1); `AskDef.boards`/`BidDef.boards: Vec<CanonicalId>`;
  `VendorDef.agent` = the port's one clerk id (unique across ports).
- Constants `STOCK_ACTION = "tacsim_port_stock"`, `STOCK_OUTCOME = "tacsim_port_stock_result"`
  (fields actor, issue, lot, qty, vault), `MAX_CLERK_ROWS = 64`, `STOCK_KEY_PREFIX =
  "tacsim.port.stock/"`, `DISSOLVE_KEY_PREFIX = "tacsim.port.dissolve/"`, `STOCK_SOURCE_KIND =
  "port.stock"`; the action payload
  `{"subject":"self_actor","targets":0,"family":"tacsim.port_stock"}`; the offer's fixed
  `{"rows":{<issue key>:{pack, entry, org, good, issue, qty, grade, vault, lot, clerk}}}` and one
  `issue` Choice; a replay refused `Conflict("that stock issue is already minted")`; the minted
  lot's `ExternalRef { provider: "tacsim", kind: "port.stock", id: "<pack>/<entry>/<org>/<good>",
  revision: Some(issue) }` with `LotId = StockIssueId::lot_id()`.
- `pub fn stock::release_notice(listing: u64, qty: u64, name: &str) -> String` = "Your market
  listing #{id} lapsed; the {qty} of {name} it held are free again.";
  `port_tick::drift_pages(&World) -> Vec<(u64 /*vault*/, CanonicalId)>`; crate-private
  `SwapRefused::Pending`, `PortTickState.{stock_action, releasing, fires, paged, release_warned,
  release_backoff}`, `stock::RELEASE_GRACE_FIRES = 24`, `MAX_RELEASING = 64`.
- `governed_hold(&World, EndpointRef) -> Option<HullView>` (the vault exists, backing
  `SpatialInterior(space)`, `space` is a programme hull's stowage); `PortVenueGate`, `PortCapacity`,
  `PortPartyCaps` (unit structs, `Debug`); `HoldPlan { v: u8, goods: String /*digest hex*/,
  compartments: Vec<PlanCompartment { id, accepts, limits }> }`, `HOLD_PLAN_VERSION = 1`,
  `hold_plan(&HullView) -> Option<HoldPlan>`; `capacity::compartment_for(&GoodsTable,
  &[Compartment], &CanonicalId) -> Option<Compartment>`; `capacity_line(goods, &Compartment, good,
  qty, &capacity::CapacityRefusal, &Load) -> String` (§4.3's sentences; limits print as whole units,
  m³ truncated to a tenth of the load after the addition); gate texts `NOT_IN_PORT`,
  `HOLD_UNDER_WAY`, `HOLD_ELSEWHERE`, `HOLD_NOT_HERE`, `NOT_YOUR_CHARTER` with
  `{port}`/`{name}`/`{other}` placeholders.
- Note: a player's `to=` into another pilot's hold is refused by the market's custody rule
  (NO_SUCH_QUOTE) before the gate; `NOT_YOUR_CHARTER` is reachable through `Presence::Ship` and a
  hold the asker may take from. No non-stowage hull interior can back a vault (the airlock is a
  multi-room space, a walked room is no space), so `governed_hold`'s stowage clause is defense in
  depth.
- `readiness_in(&Ctx) -> Readiness` with `Binding::{Pools, Listings, Bids, MarketFacts}` and the
  brief's texts (Pools Ready "{entry} vendor {org}: {name}, storefront vault #{v}; {n} of {n} stock
  rows seeded."; Listings Ready "… listing #{l} at {price}/{unit}, {q} {unit} listed; the storefront
  holds {p} {unit}[, {u} {unit} in hand-over]." and "SOLD OUT: seeded {s} {unit}, 0 left."; Pending
  "not seeded yet." / "{p} {unit} in the storefront wait to be listed."; Flagged "listing #{l} lists
  {q} {unit} but the storefront holds {p} {unit}."; Bids Ready "listing #{l} at {price}/{unit}, {r}
  of {intake} {unit} wanted." / Pending "listing #{l}; not buying until its capital settles." /
  Ready "not buying; its intake is taken." / Missing "no bid is posted."; MarketFacts "market facts:
  answered, so drawn sales can settle." / "nothing answers the market namespace, so drawn sales
  refuse."); `enrol_gate` requires Pools, Listings and MarketFacts Ready.
- `board::BOARD_HEADING = "The board:"` and the five line forms (`  {label}: {price}/{unit}, {q}
  {unit} left ({house}; signed {signatory}).`, `  {label}: SOLD OUT: seeded {s} {unit}, 0 left
  ({house}).`, `  {label}: not yet on the board ({house}).`, `  wanted, {label}: {price}/{unit}, {r}
  {unit} more ({house}; signed {signatory}).`, `  wanted, {label}: not buying ({house}).`),
  thousands grouped; `board::group_thousands(u64)`.
- `coop::BOOKS_UNREADABLE = "The Co-op's books for your account cannot be read right now. Ask staff
  to look at them."`

**Composition and content**

- `server_modules_with_tacsim` + `tacsim_module_wired` carry `PortPartyCaps`, `PortVenueGate` and
  `PortCapacity`; every roster carries `MarketFactProvider` (six fact namespaces).
- Pack versions: lane 3 (`8d20125693884a8c6739d6dd032d858e474c015726eab54b05dccb17164d0ad4`), Dray 2
  (`2f11e37f4de1f02df1f5d19aba599c4235c74307c55a6a13201e74184741cabb`), Stickney 2
  (`39aab74e2f7a56e2b71ee7cffcdd34168ad706fbc1317240d989d2dc95c42d42`); `LANE_V2_DIGEST =
  3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0` in `tests/commerce.rs`.

**Test helpers later cuts can copy:** `tests/support/vertex.rs` (`both_limits_vertex`,
`best_integer_load`, a `#[path]` include); `commerce.rs`'s `stocked()`, `chartered()`,
`buy_and_land()`, `held_listing()`, `land()`; `coop.rs`'s `ensure_stocked()`/`land_typed()` (the
desk opens only once the ports are stocked).

**Behaviours to depend on:** the applier makes no Draw listing (the arm lists once a pool exists); a
retire withdraws Draws and bids and forgets the withdrawn BidRows so a re-apply posts them afresh;
`interior.exits` are append-only by position and re-dressed on every hull update.

## IC9

**Undock and fly: the first player-runnable flight** (doc 18 §5, the IC9 row and paragraph), built
2026-10-17 on main at ee1d831f. A chartered pilot aboard a docked Dray types `+ship/plan stickney`
for a write-nothing dry run and `+ship/undock stickney` to fly. The command plans the passage over
the sky at departure with one pure planner, files the plan in the hull's sailing plan document and
submits one `tacsim_port_burn` as the pilot; the prepare recomputes the payload from the hold's
pools and the burn and crew-days from the filed plan's seconds and the drive table the provider
holds, and debits reaction mass and provisions and writes the ticket carrying the whole
`PassagePlan` in one bundle, matter first. On the next `tacsim.port` fire the launch arm finds the
burned ticket and calls the new `pub mush_sim::depart` directly with the plan's frozen per-leg
seconds and its `depart_ic`, so nothing after the burn re-reads the sky or the clock. The crew hear
the lock seal, Kirkwood Depot slide past and the docking at Stickney in every member room; arrival
places the hull on Stickney's passive domain and `+ship/leave` lands in P01. One commit, pushed to
origin/main.

### What landed

1. **The one laden step, sky-free** (`mush-sim/src/route.rs`). `route_duration`'s per-leg arithmetic
   is factored into the private `laden_leg_i64` (`base × 1000 / thrust`, then `× laden / capacity`,
   floor, every step checked) and `route_duration` sums it exactly as before, so its i64 contract
   and every pin hold. `pub fn laden_leg_durations(unladen_s: &[u32], drive, payload) ->
   Result<Vec<u32>, RouteDefError>` (D-18-44) refuses `NoLegs`, `Overloaded` and `Overflow` (any
   step, or a leg past u32), with no class check, collected with no indexing. Re-exported from
   `lib.rs`.
2. **Frozen leg times on the journey** (`mush-sim/src/journey.rs`). `CarrierJourney.leg_s:
   Option<Vec<u32>>` with a serde-defaulted, skip-if-none wire field (a journey without times
   encodes byte-identically); a fallible `with_leg_times` (`LegTimes{legs, times}`, `ZeroLegTime`)
   re-run by `TryFrom`, so a stored mismatch is HELD at boot; `leg_due_at` sums the frozen times
   when present (checked), so `due_cursor`, `arrival_at`, interception and `@journey/show` inherit
   them; `leg_secs(k)`, `leg_times()`; `describe_journey` prints the flown seconds; `reanchor` keeps
   them (it moves the record whole).
3. **`pub mush_sim::depart` and `depart_ineligible`** (`mush-sim/src/commands.rs`).
   `journey_depart`'s body after `build_route` extracted whole: `depart_ineligible` =
   `journey_ineligible` + origin anchor equality (`ROUTE_NOT_FROM_HERE`) + `arrival_parent` +
   `refuse_unreachable_destination`; `depart(ctx, carrier, route, leg_s, departed_at) ->
   Result<Departure{journey, aboard, arrival_at}, CmdError>` refuses the future, validates the
   record (and its leg times) before the allocator spends an id, books before the move, moves with
   rollback, persists, links, emits `Moved` and `" sets out."`; `journey_depart` keeps its texts and
   calls `depart(.., None, now.at)` — `e7c6`, `e7r1` and `members` byte-identical.
4. **The `CarrierHooks` composition seam** (`mush-sim/src/journey.rs`, `lib.rs`). `pub trait
   CarrierHooks` (`refuse_manual_departure(w, carrier, &RouteDef)`, `boundary(ctx, &journey,
   reached) -> bool`, `arrived(ctx, &journey) -> bool`, all defaulted), `CarrierHooksState`
   registered beside `RouteCatalogState`, `SimulationModule::with_carrier_hooks`. The manual arm
   asks the hook after `build_route` for every actor; `step` offers a boundary on `Source::Tick`
   only; `apply_arrival` captures `healed` on entry and offers `arrived` only on `Tick && !healed`
   (reviewer F11); `true` suppresses mush-sim's generic aboard line, and the outside line stays.
5. **`anchored_space_of`** (`mush-sim/src/graph.rs`): the space a room belongs to (via `member_of`,
   or itself) when its anchor is `MovingObject`; else `None`.
6. **mush-atlas** (`travel_cmd.rs`, `apply.rs`). `+travel/status`'s first arm: "You are aboard the
   {root}, under way: leg {n} of {m}, bound for {destination}; it arrives in {d} IC days
   ({about_real})." read from mush-sim's book (L15, no game edge). `create_exit` refuses, before any
   geometry, an exit whose `to` (or `from` when the row names the way back) is aboard a moving hull:
   "{id}: {room} is a room aboard a moving hull; no exit leads into one".
7. **The passage planner** (`tacsim/src/port/passage.rs`; `DriveLaw::word()` in
   `definitions/logistics.rs`). `HullDrive{class, law, payload_ceiling_kg}`, `PlanError` (§4.8's
   six + `ViaReserved`, `SkyUnavailable`, `Unreadable{what}`), `plan_passage` walking legs as body
   pairs with the fixed three-pass intercept through `pub intercept_passes` and `INTERCEPT_PASSES =
   3` (a same-body leg uses its `base_s` through the same laden step), `plan.check()` before return;
   `choose_route` (the first unplannable listed route refuses, the least `total_s` wins, ties go to
   the earlier, `via` → `ViaReserved`); `passage_burn` (Coast: whole tonnes,
   `ceil((dry+payload)×‰/1e6)` in u128), `crew_days` (occupants × Σ ceil(secs/86 400)), `payload_kg`
   (one function for the plan, undock and the prepare), `DAY_SECS`. No slice indexing anywhere.
8. **The ticket wire and the sailing plan** (`tacsim/src/port/families.rs`, `port/voyage.rs`,
   `store.rs`). `Ticket` gains `seq` and loses `launched`; `Ticket::ticket_id()`;
   `PortHandles.burn`. `Voyage{schema, hull, last_seq, pending}` and `PendingVoyage{seq, pilot,
   to_venue, vault, occupants, payload_kg, route_pack, route_version, unit_definition, plan, order,
   attempt, filed_ic, filed_at}`, both `deny_unknown_fields`; `Voyage::check` (schema 1, `seq ==
   last_seq + 1`, the plan checks, occupants 1..=16, attempt ≥ 1); `tacsim:voyage:{hull}` with
   strict `voyage_key`/`voyage_key_owner`; store `voyages`/`voyage_keys`, `voyage_state` (`Absent |
   Present | Unreadable`), `voyages()`, `put_voyage` on `put_enrolment`'s pattern, a load arm with
   the identity rule that keeps an unreadable key claimed; the module-doc table row.
9. **`tacsim_port_burn`** (`tacsim/src/port/burn.rs`; `port_tick.rs` `register(.., burn:
   Option<BurnTable>)`; `lib.rs`). `BurnTable{goods, hulls by unit digest}` built by `burn_table`
   from every fielded spec with logistics (a class with no law left out, a propellant good not
   counted at 1,000 kg refuses the table); `hull_spec_of` shared with the verbs (L34).
   `BurnProvider::project_offer` projects only for `burnable` (live charter → hull, a pending filed
   by the actor, the actor aboard, the hull docked) with fixed `{now, voyage: {hull, pending}}` and
   one `seq` Choice; `prepare_authoritative` runs in the brief's order (shape, seq, pilot, future
   depart, hull spec, the ticket as replay guard, the payload recomputed from pool quantities =
   filed, the ceiling, plan/law/burn/crew-days add up, tank and locker against available), then
   writes the manifest BEFORE the debit, the two debits and the ticket in one bundle. Outcome
   `tacsim_port_burn_result`; the action descriptor is unconditional, the provider registers only
   with the inventory cell and the table. Key `tacsim.port.burn/{hull}/undock/{seq}`.
10. **The launch arm and its terminal** (`tacsim/src/port/voyage.rs`, `port_tick.rs`,
    `commands.rs`). `Arm::Launch{hull}`; `launch_verify` and `launch_prune` at the top of every
    fire; one agent per pending hull, backoff skipped. `launch_step`: the ticket read through the
    module lens with the key re-checked (unreadable → hold); no ticket → `Deferred` while the
    pilot's submission is pending, else a lapse after `LAUNCH_GRACE_FIRES = 24` (`last_seq`
    advanced, CLEARANCE_LAPSED); already launched → clear; the live lease re-checked (L31); **the
    pilot re-checked aboard through the shared `voyage::aboard` predicate (fixer, L14)**;
    `route_in_world` + `route_source` at the ticket's `(pack, version)`; `mush_sim::depart(..,
    Some(leg_s), plan.depart_ic)`; the cleared voyage written in the same outbox; LOCK_SEALS to
    every walkable room; `Launching` recorded and CLEARANCE_GRANTED told once by pull. `launch_hold`
    pages once per hull per boot, tells the pilot LAUNCH_HELD once, backs off and keeps the pending.
    `@tacsim/port clear #<hull>` (`PORT_USAGE` updated) stands a clearance down through `clear_cmd`:
    the pending cleared, `last_seq` kept, the ticket kept, CLEARANCE_STOOD_DOWN to the pilot.
11. **tacsim's carrier hooks** (`tacsim/src/composition/sim_routes.rs`, `composition/mod.rs`,
    `port/coop.rs`, `port/hull.rs`). `TacsimCarrierHooks` wired beside the route catalog on every
    roster: a `LANE_PACK` route refused on the manual arm for every carrier and actor
    (LANE_UNPLANNED), a programme hull on any manual route (MANUAL_DEPARTURE); BOUNDARY (Kirkwood's
    detail from `WAYPOINT_DETAILS`, keyed by lane place id) and DOCKS narrated to every walkable
    room, the UTC-prefixed line / ARRIVAL_NOTICE queued for an absent tenant (`charter_tenant`,
    factored with `lease_tenant` out of `hull_line`); `REPLACE_GENERIC_ABOARD_LINES = false`.
12. **The verbs and the renders** (`tacsim/src/commands.rs`, `port/ship.rs`, `port/voyage.rs`,
    `port/market.rs`, `port/hull.rs`, `port/gauges.rs`). `+ship` is `ArgShape::Raw` with switches
    `board leave hold plan undock` in its own Authoritative bracket (L11). `+ship/plan [<port>]
    [at=now]` renders lines 1-8 (the slack line from `gauges::measures_line` plus free amounts from
    `capacity::load`; the room line from `compartment_for` + `max_more`; `Overloaded` replaces lines
    4-7; it writes nothing). `+ship/undock <port>` runs every check in the brief's order — usage,
    via/at, charter, `UNDOCK_UNWIRED`, `VOYAGE_DARK`, `UNDOCK_CLEARED`, `UNDOCK_UNDER_WAY`,
    `NOT_ABOARD`, `NOT_DOCKED`, `NO_ROUTE`, `goods_pending_aboard` (GOODS_COMING / LISTING_HOLDS
    over mush-market's public `quotes()` and `lot_listings()`, mush-market untouched), the plan
    (`OVERLOADED` / `PLAN_DARK`), `TANK_SHORT`, `STORES_SHORT`, the launch dry run
    (`route_in_world`, the lane version, `depart_ineligible` → `PORT_CLOSED`) — then `put_voyage`
    and ONE `submit_authoritative` as the pilot, replying CLEARANCE_FILED. While filed,
    `+ship/leave` and `+ship/board` refuse LEAVE_CLEARED and the venue gate answers HOLD_UNDER_WAY.
    `SHIP_CLEARED`/`SHIP_UNDER_WAY` and `STATUS_CLEARED`/`STATUS_UNDER_WAY` ("…, leg {n} of {m}{,
    Kirkwood astern}, ETA {hhmm} UTC.") read the passage first; the time helpers (`ic_days`,
    `real_span`, `utc_hhmm`, `utc_day_word`, `civil_from_days`) are `pub(crate)` with unit tests.
    Readiness: a moving hull is never free and ", {d} departing" is appended.
13. **Tests** (`tacsim/tests/passage.rs` 4, `tacsim/tests/voyage.rs` 13,
    `mush-sim/tests/leg_times.rs` 6, `mush-atlas/tests/anchored_exits.rs` 2, plus unit tests in
    `route.rs`, `passage.rs`, `burn.rs`, `voyage.rs`, `families.rs`). The seven mandatory tests
    under their exact names pin lane v3's figures (959,361 + 735,600 s, 41 t, 21 crew-days;
    653,642 + 852,473 s, 37 t, 18) and the freeze across a sky change and reboots; the verify lens
    at rate 3600 asserts the plan lines, the debits 42→1 / 21→0, the frozen `leg_s`,
    LOCK_SEALS/Kirkwood/DOCKS in the Crew Compartment, GRANTED once, the under-way renders, the
    venue gate and the departing count while filed, Direct on Stickney and `+ship/leave` into P01.
    The fixer's additions: `ic9_a_pilot_moved_ashore_holds_the_launch`,
    `ic9_an_unreadable_sailing_plan_fails_closed` (corrupt and misfiled), the replay guard and the
    hold-changed prepare refusal, the clock-advance and unplaced-pilot checks on the burn offer, and
    `assert_nothing_moved` asserting the exact prior voyage and no `tacsim:voyage:` op.
14. **Gates** (`mush-contract-tests/legacy-handlers.toml`, `state-slot-owners.toml`;
    `tacsim/tests/commerce.rs`): the `+ship` stanza moved to `authoritative` with the IC9 note, the
    `@tacsim` stanza refreshed for `/port clear`, a `voyage.rs` AccountOracle row, and the IC8
    venue-gate test's setup replaced with the §6 `mush_sim::depart` lines.

### Deferred, and why

The owner-critical defaults first, then the lead rulings the planner took, the builder's declared
deviations, the residuals, and what later cuts inherit.

**Owner-critical defaults (each a one-line change once the owner rules):**

- **Generic aboard lines kept beside the flavoured ones** (L8; IC4's recorded surface "additional,
  not replacements"). Doc 18 §6 shows only the flavoured line; `REPLACE_GENERIC_ABOARD_LINES =
  false` in `composition/sim_routes.rs` is the one-token flip.
- **The manual `@journey/depart` arm refuses every lane route for every carrier and actor, staff
  included, and a programme hull on any route** (L32; closes IC3's F9 on the route). Staff keep the
  arm for non-programme carriers on non-lane routes. A staff power is narrowed.
- **The pilot is held aboard while a clearance is filed or a burned launch is held** (L14/L29):
  `+ship/leave`/`+ship/board` refuse and the hold cannot load; a filed-but-unburned clearance lapses
  after 24 fires (~2 min); a held launch lasts until staff fix the cause or run `@tacsim/port
  clear`. There is no player abandon verb.
- **No refund on a stood-down clearance** (L30): the burned matter is gone; the ticket is the
  record.
- **The two-step undock voice** (L37): CLEARANCE_FILED at once, CLEARANCE_GRANTED on the launch
  fire's pull verification (doc 18 §6 shows "Clearance granted" as the reply). Every new line the
  brief wrote (LANE_UNPLANNED, MANUAL_DEPARTURE, LAUNCH_HELD, CLEARANCE_STOOD_DOWN, PORT_CLOSED,
  AT_NOT_YET, the `+ship/plan` lines, the status form) is the owner's to re-voice; the builder's own
  wordings are "#{hull} is under way; its clearance has flown.", `UNDOCK_USAGE`, and the
  short-clause form "(tank 39 t; 1 t short)".
- **The listing rule** (L13): any hold-backed listing whose stock sits in the hull's hold, live or
  lapsed, refuses undock until it is withdrawn or released.
- **HOLD_UNDER_WAY for a docked, cleared hull** (conformance F5): the brief directs the under-way
  text while the hull sits at the berth; a HOLD_CLEARED variant is a voice item.
- **For IC12's docs pass** (L2/L27/L36): lane v3's figures replace doc 18's v1 figures (outbound
  959,361 + 735,600 = 1,694,961 s, 41 t, 21 crew-days; return 653,642 + 852,473 = 1,506,115 s, 37 t,
  18 crew-days; the boundary at ~266.5 s real at 3600, not ~268); the tank case is 39 t against a 40
  t burn (105‰ includes the fuel's own mass, so "40 against 41" is impossible); the ticket's
  `launched` field is gone.

**Lead rulings taken** (L1-L37 of the brief, condensed): floor not ceil, `route_duration`'s exact
step (L1); the journey departs at `plan.depart_ic` (L3); the plan reaches the prepare only through
the filed voyage document as a server-authenticated fixed parameter, the submission naming only
`seq` (L4); the ticket +`seq` −`launched` (L5); the names `tacsim_port_burn` /
`tacsim_port_burn_result` / key `tacsim.port.burn/{hull}/undock/{seq}` (L6); the launch is a direct
`mush_sim::depart`, no submission, no swap, the boot reconcile = the first fire (L7); one
`CarrierHooks` seam, Tick-only, never on the heal re-apply (L8); Kirkwood's detail and short word as
code data keyed by lane place id, no pack moves (L9/L35); `PlanError` + `ViaReserved`,
`SkyUnavailable` (L10); `+ship` Authoritative and Raw (L11); occupants = persons Interior in the
airlock space, payload = pools × mass + occupants × allowance (L12); undock waits for promised goods
and the venue gate treats a filed hull as under way (L13); `+travel/status`'s arm is generic (L15);
the apply-time refusal in `create_exit` through `anchored_space_of` (L16); the Engineering/Bridge
burn-and-reserve gauge lines go to IC11 (L17); readiness reads the voyage pendings (L18); a
whole-tonne burn, and the table refuses a non-1,000 kg propellant (L19); `PassagePlan.law` is the
law's word (L20); the destination = a routes key or a port-name prefix, `at=now` only (L21);
`choose_route` refuses on the first unplannable listed route (L22); the lane addressed by its
latched version and the launch flies only that version (L23); notices only to an absent tenant,
GRANTED through `notify` (L24); real time only on §4.4's OOC surfaces (L25); a lapse after 24 fires,
the seq burned (L26); the launch dry-run at undock (L28); the lease re-checked at launch (L31); the
IC8 venue-gate test reaches "under way" through `mush_sim::depart` directly (L33); `unit_definition`
= the fielded reference's digest (L34).

**The builder's declared deviations:** `PlanError::Unreadable{what}` as a ninth variant (a pool that
will not read, or an unweighed good, cannot honestly be `Overflow`); `BurnTable`/`HullSpec` are
`pub` with crate-private fields (`private_interfaces` under `-D warnings`); the fixed `voyage`
parameter is nested `{hull, pending}` (`deny_unknown_fields` cannot be flattened) and `now` is the
IC `at` integer; an unreadable ticket goes through `launch_hold` rather than a separate log-once
path; the tank and locker are checked against `available` while the payload sums `quantity`;
`+ship/plan` counts the pilot aboard (`max(count, 1)`), so a plan read ashore includes them. The
fixer's addition: the launch also holds with "the pilot is no longer aboard".

**Residuals (named, small):** no ticket-vs-pending cross-check before flying (a hand-repaired
`last_seq` could re-mint a used seq); the atlas refusal fires after the framework has retired an
existing exit pair on an UPDATE row; a held launch re-trips the tick's generic "logged once" warning
every backoff; CLEARANCE_FILED precedes the durable receipt (a burn refused at ingress leaves a
~2-minute LEAVE_CLEARED window); a lapse could clear a pending whose burn is staged but not
reconciled after ~2 minutes; an `Unreadable` voyage leaves board/leave/loading open and uncounted in
readiness; the over-rail render rounds half up ("181.0 t against 181 t" on staff-seeded loads); the
rate-0 `+travel/status` keeps its parenthesis; `PortTickState` is RAM, so GRANTED is not told across
a reboot and the lapse grace restarts; a boundary or arrival caught up at boot gets the generic line
and no notice (R9); `+ship/plan` is allowed while a clearance is filed. Untested: the non-staff
clear's denial text, the `Deferred` branch with the burn still queued, `goods_pending_aboard`'s
terminal-contract and lapsed-listing cases, a mid-journey rate change with `leg_s`, and a one-way
exit out of a room aboard.

**Not done:** the hand-played smoke at rate 24 (no Redis for the staff bootstrap, no scripting
client); the rate-3600 verify-lens test stands in and no log is offered. **Unowned (not in the IC9
row):** the C01/P01 berth boards and the C14 departures board as `LookRendered` lines.
**Inherited:** the queue, `+ship/order`, the time parser and the Engineering/Bridge gauge lines
(IC11); selling (IC10); help topics, the readiness gate's full form, the doc 18 text fixes and the
smoke (IC12); buyout/release/revoke (IC13); the moving sky, `via=` honoured and a route policy
(P-A).

### Pins re-blessed

- **`mush-contract-tests/legacy-handlers.toml`, the `+ship` stanza**: category `legacy-post-image` →
  `authoritative`, `family` dropped (the `+market` stanza's shape), the note rewritten to name
  `/plan` (a pure read), `/undock` (one `tacsim:voyage:<hull>` whole-document write and exactly one
  `tacsim_port_burn` submission) and doc 18 §4.2/§4.4, D-18-11/23/30/44/46. Reason: the ceiling rule
  — a CommandSpec takes the class of its highest switch and `/undock` stages an authoritative
  submission. `plan` and `undock` are switches of the existing verb, so `PINNED_BASELINE` stays
  at 97.
- **The `@tacsim` stanza**: REFRESHED (not joined) for the staff argument `@tacsim/port
  clear #<hull>` on the existing `/port` switch (one voyage write and a notice; no new switch).
  Category and family unchanged; `PINNED_BASELINE` does not move.
- **`state-slot-owners.toml`**: a row for `crates/tacsim/tests/voyage.rs` (`World::module_state_mut`
  → `AccountOracle`, owner `sim`), with the coop.rs/commerce.rs reason: every passage test charters
  through `+coop/enrol`, whose rule is per gateway account and fails closed without the oracle a
  `TestWorld` lacks.
- **The `tacsim/src/port/families.rs` ticket fixture** (IC5's L5/L36): `launched: false` → `seq: 9`;
  assertions added only — `ticket_id() == TicketId{5011, 9}` and a wire carrying `"launched"`
  refused. Reason: no world holds a ticket; the key is derived from the value and no writer could
  ever set `launched`.
- **`tacsim/tests/commerce.rs::ic8_the_ship_presence_and_the_gate_refusals`**: setup only — GOD's
  manual `@journey/depart #hull=ic8_fixture #gallery:100` replaced by the §6 in-test
  `mush_sim::depart` through `support::with_ctx` on the same one-leg route; every assertion
  including `HOLD_UNDER_WAY` unchanged; one doc-comment line added. Reason: L32 now refuses a
  programme hull on the manual arm by design.
- **Unmoved, asserted by the gate**: the restriction-lints census (`tacsim [0, 3, 2, 0, 0, 0]`,
  `mush-sim [3, 35, 5, 0, 0, 0]`, `mush-atlas` zeros — no new unwrap/expect/indexing in non-test
  code; mush-sim's one `&legs[..=k]` kept), the panic budgets, every content digest (lane v3, Dray
  v2, goods v1 untouched), the composition-edge pin, the CI gated-test inventory (no `#[ignore]`
  added), module-ns-owners and typed-state-lens (they enumerate nothing this cut touches), and the
  help allowlist.

### How it was verified

Three lenses reviewed the uncommitted tree against the brief: a correctness lens (the burn's
one-bundle matter-first order, the fixed-parameter plan against a submitter, the launch's atomicity
and the Tick-only hooks, the dry-run/launch equivalence — one should-fix, the launch never
re-checking the pilot aboard, plus six notes), a conformance lens (every brief text verbatim against
the code, all 15 items and the 37 lead rulings against the diff, the gates — two should-fix:
`assert_nothing_moved` checking "no pending" rather than "no voyage write", and `DAY_SECS` claimed
re-exported but not, plus six notes) and a tests lens (five should-fix: no prepare refusal or replay
guard exercised, the burn-offer test unable to tell the filed plan from a re-plan, the filed-hull
venue gate and departing count untested, no fail-closed load test, the weak refusal helper; five
notes). The fixer fixed all seven must/should-fix findings and rebutted none: `voyage::aboard` as
the one predicate behind undock, the offer and `launch_step` (mutation-checked — with the check
disabled `ic9_a_pilot_moved_ashore_holds_the_launch` fails 0/1), `DAY_SECS` re-exported,
`assert_nothing_moved` asserting the exact prior voyage and no `tacsim:voyage:` op, the replay guard
and the hold-changed refusal driven to the prepare, the clock-advance and unplaced-pilot checks on
the offer, the venue-gate and ", 1 departing." assertions in the verify lens,
`ic9_an_unreadable_sailing_plan_fails_closed` (corrupt and misfiled) and the `Voyage::check` unit
cases. The judge then re-read the whole diff and every new file (verdict: ship with notes).

**The gate**, 22 commands in order, all green on the first pass with no rerun, no LNK1104 and no
hung binary; logs under the run's `cuts/IC9/gate1/`:

- `cargo fmt --all -- --check` → no output, exit 0 (twice).
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s) in 0.66s``, exit 0, no warning lines (twice).
- `cargo test -p mush-sim --lib route` → `test result: ok. 9 passed; 0 failed; 0 ignored; 0
  measured; 28 filtered out`.
- `cargo test -p mush-sim --test leg_times` → `ok. 6 passed; 0 failed`.
- `cargo test -p mush-sim --test e7c6 --test e7r1 --test members` → `ok. 68 / 19 / 11 passed; 0
  failed`.
- `cargo test -p mush-sim --no-fail-fast` → lib 37 passed; custody 11, deployed 5, e7c10 18, e7c11
  3, e7c5 34, e7c6 68, e7c8 32, e7r1 19, e7r2 2, leg_times 6, members 11, placement 31, transport 24
  — all ok, 0 failed.
- `cargo test -p mush-atlas --test anchored_exits` → `ok. 2 passed; 0 failed`.
- `cargo test -p mush-atlas --no-fail-fast` → lib 51 passed; every binary ok, 0 failed.
- `cargo test -p tacsim --lib` → `ok. 899 passed; 0 failed; finished in 3.99s`.
- `cargo test -p tacsim --test passage --test voyage` → `passage: ok. 4 passed` / `voyage: ok. 13
  passed; 0 failed; finished in 3.10s`.
- `cargo test -p tacsim --test commerce --test coop --test hulls --test ports --test port_tick
  --test capacity` → capacity 17, commerce 20, coop 18, hulls 15, port_tick 16, ports 17 passed; 0
  failed each.
- `cargo test -p tacsim --test lane --test stations --test content --test definitions --test
  composition_edges --test e7c1 --test e7r1` → composition_edges 2, content 29, definitions 25, e7c1
  38, e7r1 21, lane 8, stations 13 passed; 0 failed.
- `cargo test -p mush-contract-tests --test restriction_lints --test legacy_handler_allowlist` →
  `legacy_handler_allowlist: ok. 4 passed` / `restriction_lints: ok. 11 passed; 0 failed`.
- `cargo test -p mush-contract-tests` → 29 `test result: ok.` lines, 0 failed (scale_harness 6
  passed, 8 ignored as before) — run twice.
- `cargo test -p mush-server --lib modules::` → `ok. 32 passed; 0 failed; 264 filtered out`.
- `cargo test -p mush-server --test boot_parity --test e7r2_proving_loop` → `ok. 18 passed` / `ok. 6
  passed; 0 failed`.
- `cargo test -p mushd --test boot_composition` → `ok. 6 passed; 0 failed`.
- `CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast` → 394 `test result:`
  lines, 394 ok, summed **7,397 passed, 0 failed, 92 ignored**; 29 minutes wall. The log's one
  `panicked at crates\mush-server\src\process_fault.rs:197:9` line is
  `tests::a_panicked_world_task_is_a_failure` panicking on purpose; that test passed.
- `cargo test -p mush-bot` → lib 469 passed; arena_e2e 9, playtesting 9, e2e 49; all 22 binaries ok,
  0 failed (ignored: arena_live_commander 1, arena_muster_rate 2, arena_win_rate 2).

No flake was identified. The hand-played smoke (item 15) was not run (no Redis, no interactive
client in the run environment) and is recorded as owed.

### Surface for the next cuts

**mush-sim** (verified in `route.rs`, `journey.rs`, `commands.rs`, `graph.rs`, `lib.rs`)

- `pub fn laden_leg_durations(unladen_s: &[u32], drive: &DriveProfile, payload: u64) ->
  Result<Vec<u32>, RouteDefError>` — `NoLegs` on empty, `Overloaded{payload, capacity}`, `Overflow`
  on any step or a leg past u32; no drive-class check. `route_duration`'s contract unchanged.
- `CarrierJourney::with_leg_times(self, Vec<u32>) -> Result<CarrierJourney, CarrierJourneyError>`
  (`LegTimes{legs, times}`, `ZeroLegTime`); `leg_secs(&self, k: usize) -> Option<u32>`;
  `leg_times(&self) -> Option<&[u32]>`; `CarrierJourneyWire.leg_s: Option<Vec<u32>>`
  (`serde(default, skip_serializing_if = "Option::is_none")`);
  `leg_due_at`/`arrival_at`/`due_cursor` read the frozen times when present.
- `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Departure { pub journey: JourneyId, pub aboard:
  usize, pub arrival_at: Option<i64> }`.
- `pub fn depart_ineligible(ctx: &Ctx<'_>, carrier: ObjId, route: &RouteDef) -> Option<String>`.
- `pub fn depart(ctx: &mut Ctx<'_>, carrier: ObjId, route: RouteDef, leg_s: Option<Vec<u32>>,
  departed_at: i64) -> Result<Departure, CmdError>` — refuses with `depart_ineligible`'s sentence,
  then "A journey cannot depart in the future."; never consults `refuse_manual_departure`; no
  authority check.
- `pub trait CarrierHooks: Send + Sync { fn refuse_manual_departure(&self, &World, ObjId, &RouteDef)
  -> Option<String>; fn boundary(&self, &mut Ctx<'_>, &CarrierJourney, reached: u32) -> bool; fn
  arrived(&self, &mut Ctx<'_>, &CarrierJourney) -> bool }` (all defaulted; `true` suppresses the
  generic aboard line; Tick only; `arrived` never on the heal re-apply). `pub struct
  CarrierHooksState` (`new`, `unwired`, `is_wired`, `hooks`);
  `SimulationModule::with_carrier_hooks(Arc<dyn CarrierHooks>)`.
- `pub fn anchored_space_of(w: &World, room: ObjId) -> Option<ObjId>`.

**mush-atlas**

- `+travel/status`'s aboard arm "You are aboard the {root}, under way: leg {n} of {m}, bound for
  {destination}; it arrives in {d} IC days ({about_real})." (rate 0: "(while the clock is
  stopped)"); the `create_exit` refusal "{id}: {room} is a room aboard a moving hull; no exit leads
  into one".

**tacsim, the planner** (`port/passage.rs`, all re-exported at `tacsim::port`)

- `HullDrive { pub class: CanonicalId, pub law: DriveLaw, pub payload_ceiling_kg: u64 }`;
  `PlanError::{UnknownRoute{route}, UnknownDriveClass{route, expected, actual}, UnknownBody{place},
  NoSkyLeg{place}, Overloaded{payload_kg, ceiling_kg}, Overflow, ViaReserved, SkyUnavailable,
  Unreadable{what}}`.
- `plan_passage(sky, pack, route, drive, dry_kg, payload_kg, occupants, depart_ic) ->
  Result<PassagePlan, PlanError>`; `choose_route(sky, pack, routes: &[CanonicalId], drive, dry_kg,
  payload_kg, occupants, depart_ic, via: Option<&CanonicalId>)`; `intercept_passes(sky, a, b, t0,
  drive, payload_kg, passes) -> Result<Vec<(u64, u32)>, PlanError>`; `INTERCEPT_PASSES: usize = 3`;
  `DAY_SECS: u32 = 86_400`.
- `passage_burn(law, dry_kg, payload_kg, legs) -> Result<u64, PlanError>` (whole tonnes);
  `crew_days(occupants, legs) -> Result<u32, PlanError>`; `payload_kg(goods, accepted, read: impl
  FnMut(&CanonicalId) -> Result<u64, PlanError>, occupants, allowance_kg) -> Result<u64,
  PlanError>`; `DriveLaw::word() -> &'static str` ("coast"); `LegPlan`, `PassagePlan` and `DriveLaw`
  re-exported.

**tacsim, the burn** (`port/burn.rs`; `tacsim::port::{BURN_ACTION, BURN_OUTCOME, BURN_KEY_PREFIX}`)

- `BURN_ACTION = "tacsim_port_burn"`, `BURN_OUTCOME = "tacsim_port_burn_result"` (fields `actor,
  burn_t, crew_days, hull, seq`), `BURN_KEY_PREFIX = "tacsim.port.burn/"` (the live key
  `tacsim.port.burn/{hull}/undock/{seq}`; IC11 keeps `…/{order}/{attempt}`), `PROPELLANT_KG_PER_UNIT
  = 1_000`.
- `pub struct BurnTable`, `pub struct HullSpec` (fields crate-private); crate-private
  `burn_table(&Catalog, Option<Arc<GoodsTable>>) -> Option<BurnTable>`, `hull_spec_of(&Catalog,
  &GoodsTable, &CanonicalId) -> Result<(DefinitionDigest, HullSpec), String>`, `consumable_good`,
  `burnable(w, actor) -> Option<(ObjId, PendingVoyage)>`.
- The action descriptor `{"subject":"self_actor","targets":0,"family":"tacsim.ticket"}`. Offer fixed
  parameters `{"now": <ic i64>, "voyage": {"hull": n, "pending": PendingVoyage}}`, one `seq` Choice;
  a submission sends `{"seq": "<decimal>"}`.
- `port_tick::register(reg, test_world, inventory, burn: Option<BurnTable>)`; `PortHandles.burn:
  Option<DefinitionRef>` (`Some` exactly when the provider registered).

**tacsim, the ticket and the sailing plan** (`port/families.rs`, `port/voyage.rs`; re-exported at
`tacsim::port`)

- `Ticket { hull, seq: u32, route, route_pack, route_version, unit_definition, manifest, payload_kg,
  plan, order, attempt, burned_at }` (strict; `launched` removed), `Ticket::ticket_id() ->
  TicketId`.
- `VOYAGE_PREFIX = "tacsim:voyage:"`, `VOYAGE_SCHEMA: u16 = 1`, `voyage_key(ObjId) -> SmolStr`,
  `voyage_key_owner(&str) -> Option<ObjId>` (strict decimal).
- `Voyage { pub schema: u16, pub hull: ObjId, pub last_seq: u32, pub pending: Option<PendingVoyage>
  }`, `PendingVoyage { seq, pilot, to_venue: CanonicalId, vault: u64, occupants: u32, payload_kg:
  u64, route_pack, route_version: u32, unit_definition: DefinitionDigest, plan: PassagePlan, order:
  Option<u32>, attempt: u32, filed_ic: i64, filed_at: i64 }` (both `deny_unknown_fields`; `pending`
  always written, `null` when empty); `Voyage::check()`; `VoyageState::{Absent, Present(&Voyage),
  Unreadable}`; `pub fn pending_of(w, hull) -> Option<&PendingVoyage>`; `LAUNCH_GRACE_FIRES: u64 =
  24`, `LAUNCH_WATCH_FIRES: u64 = 24`.
- Store: `voyage_state(&self, hull) -> VoyageState<'_>`, `voyages() -> impl Iterator<Item =
  &Voyage>`, crate-private `put_voyage(ctx, &Voyage)` (the single write path; `last_seq` is the
  per-hull seq minter IC11 reuses).
- Crate-private helpers IC11 will call: `goods_pending_aboard(w, EndpointRef) ->
  Option<Pending::{Quote, Listing}>`, `aboard(w, hull, person) -> bool`, `passage_of`/`Passage`, the
  time renders `ic_days`, `real_span`, `utc_hhmm`, `utc_day_word`, `civil_from_days`, `real_until`,
  `tonnes_one_decimal`; `launch_step`/`launch_hold`/`launch_verify`/`clear_cmd`; `narrate_hull`,
  `tell_absent_tenant`, `boundary_line`, `arrival_lines`.

**tacsim, the verbs and texts**

- `+ship/plan [<port>] [at=now]`, `+ship/undock <port> [at=now]` (switches of the Authoritative
  `+ship`, `ArgShape::Raw`); `@tacsim/port clear #<hull>` (staff; `PORT_USAGE = "Usage: @tacsim/port
  status | @tacsim/port clear #<hull>"`).
- Every text constant of the brief verbatim in `port::voyage` (LOCK_SEALS, BOUNDARY, DOCKS,
  ARRIVAL_NOTICE, CLEARANCE_GRANTED, CLEARANCE_LAPSED, MANUAL_DEPARTURE, LANE_UNPLANNED,
  LAUNCH_HELD, CLEARANCE_STOOD_DOWN, CLEARANCE_FILED, UNDOCK_UNDER_WAY, UNDOCK_CLEARED, NOT_ABOARD,
  NOT_DOCKED, NO_ROUTE, VIA_NOT_YET, AT_NOT_YET, PORT_CLOSED, GOODS_COMING, LISTING_HOLDS,
  OVERLOADED, TANK_SHORT, STORES_SHORT, PLAN_DARK, PLAN_UNDER_WAY, VOYAGE_DARK, UNDOCK_UNWIRED,
  LEAVE_CLEARED, plus UNDOCK_USAGE, NOTHING_BURNED, NOTHING_WRITTEN, WAYPOINT_DETAILS,
  WAYPOINT_SHORT) and in `port::ship` (STATUS_CLEARED, STATUS_UNDER_WAY "Aboard the {name} ({mark}),
  under way, leg {n} of {m}{astern}, ETA {hhmm} UTC.", SHIP_CLEARED, SHIP_UNDER_WAY).
- `tacsim::port::charter_tenant(w, hull) -> Option<ObjId>`;
  `composition::sim_routes::{TacsimCarrierHooks, REPLACE_GENERIC_ABOARD_LINES = false}`.
- The readiness hulls line gains ", {d} departing". No pack version moves: lane v3, Dray v2, goods
  v1.

## IC10

**Selling and the Co-op share: the loop closes interactively** (doc 18 §5, the IC10 row and
paragraph), built 2026-10-17 on main at 1137c373. A chartered pilot docked at Stickney sells pool
quantity out of the hold into a house's standing bid with `+market/sell <listing>=<qty> [from=]`.
Inventory exports the two things the market needs and owns neither: `derived_marker_id`, a pure
`{at}{sha256}` id of 80 hex a caller can name in a contract before the bundle that writes it exists,
and `transfer_marked`, a transfer that writes its own `inventory.transferred` marker under that id
and refuses a replay before any write. The market's `sell_into_bid` runs every refusal before
`offer_contract`, then offers the sale with the pilot as issuer, writes the quote, accepts in-verb
as the pilot and as the house through the party-cap source, stages exactly one `market_lot` deliver
keyed by the contract, and commits the bid's intake; the contract's Exact payouts to the seller and
at most one proceeds cut are gated on `Before` plus the inventory Fact, so money never moves before
the matter. The deliver prepare ties listing, destination, source, commodity and quantity to the
server-built paid entry and reads the marker from it, never from the submission. `restage_deliver`
re-stages the same leg byte for byte behind `+market/accept`; a dropped leg refunds at the window
and restores the bid, a landed leg whose contract lapsed pages and does not. An exhausted bid stands
at quantity 0 as the readiness report's exhausted marker, listed as not buying, kept by the sweep,
blocking a second bid and withdrawn by a retire; the stock clerk re-posts a lapsed bid at its full
authored intake and an applier intake cut saturates at 0. tacsim's `CoopProceeds` reads the
programme's reference ask live at contract build (else the goods table's baseline with a memo),
floors 250‰ of the margin to the Trust, writes the sale line with the house's signatory, and folds
sales, margin and shares into the enrolment once per settled sale keyed by pilot and hold; undock
refuses `GOODS_LEAVING` while a hand-over is pending. `Engine::step` now notes queued authoritative
submitters for every command so a verb that stages one refuses `SELL_BUSY` before it offers
anything. The story test's part 1 plays one leg by hand on the production layer with every reply
computed from the posted prices and the vertex. One commit, pushed to origin/main.

### What landed

1. **The derived marker id and the marked transfer** (`mush-inventory/src/pool.rs`, `src/lib.rs`).
   `pub const MARKER_CAUSAL_MAX_BYTES = 512` caps the canonical JSON *input* of `pub fn
   derived_marker_id(at_ic, &Value) -> Result<SmolStr, InventoryError>`, which is `{(at as u64) ^
   1<<63:016x}{sha256 under "inventory.marker.derived.v1"}` — always 80 lowercase hex, inside the
   family's 1..=96 rule, so the mirror's newest-kept order holds; any hash error (an over-cap input
   included) is `Identity`. `InventoryFamilies::transfer_marked(txn, from, to, commodity, amount,
   marker, actor, now)` refuses `from == to` with the new `InventoryError::SameEndpoint`, a non-hex
   id with `Identity` and an existing marker with `AlreadyExists` (the replay guard, read before any
   write), then `transfer` and one `inventory.transferred` applied marker in the same txn. Both are
   re-exported; `write_marker` and the five verbs are untouched.
2. **The sale records** (`mush-market/src/record.rs`). `PaidKind::Deliver` (`"deliver"`);
   `PaidLot.marker: Option<SmolStr>` with `check()` requiring a cutoff and a well-formed marker on
   exactly the Deliver kind (a Draw keeps its cutoff and no marker; a Hold neither); `ProceedsCut {
   to, amount, label }` (`deny_unknown_fields`, `check()`: amount > 0, label 1..=32 bytes),
   `MAX_PROCEEDS_CUTS = 1`, `MAX_PROCEEDS_LABEL_BYTES = 32`; `Quote.cuts` and `Quote.basis` carried
   on the wire and re-checked on load by `check_proceeds` (≤ 1 cut, every cut checks, cuts or a
   non-zero basis only on a Deliver, basis ≥ 0, Σ cuts ≤ price × quantity); the listing wire admits
   `quantity == 0` for `Supply::Wanted` only, and the Wanted doc now says an exhausted bid stands at
   0.
3. **The live index and the seller index** (`mush-market/src/store.rs`). `reindex` builds `live`
   from `quantity > 0 || source.is_wanted()` so a zero bid is listed, blocks a second bid
   (`wanted_of`) and is withdrawn by a retire; `by_lot_pool` keeps `quantity > 0`; a new `by_seller`
   index keyed as `by_buyer` is, maintained in `reindex` and compared in `indexes_agree`, with `pub
   fn quotes_of_seller`.
4. **The proceeds seam** (`mush-market/src/module.rs`). `ProceedsPolicy { proceeds(&World,
   &SaleTerms) -> Result<Proceeds, ProceedsRefusal>; settled(&mut Ctx, &SettledSale) {} }`,
   `SaleTerms`, `Proceeds` (with a `Default` of no cuts, zero basis, the market's line, no memo),
   `ProceedsRefusal`, `SettledSale`, `MAX_SALE_LINE_BYTES = 512`, `MAX_PROCEEDS_MEMO_BYTES = 24`;
   `MarketModule::with_proceeds_policy` frozen into `MarketTables.proceeds` (`Debug` reports
   presence) behind `pub(crate) proceeds_policy(w)`; the IC7 block comment now says the market adds
   three hooks of its own.
5. **The lot sell arm** (`mush-market/src/trade.rs`). `pub fn sell_into_bid(ctx, &BidSaleRequest {
   listing, quantity, source, presence }) -> Result<BidSale { contract, quote, gross, cuts, net,
   line }, MarketError>`: fourteen refusals in the brief's order, every one before `offer_contract`
   — listing/viewable/Wanted (`NO_SUCH_LISTING`), house trading (`NOT_BUYING`), deadline
   (`BID_LAPSED`), quantity against the intake (`NOT_BUYING` at 0, `OVER_INTAKE`), grade
   (`GRADED_BID`), a queued submission (`SELL_BUSY`), the source named or placed by the gate then
   `may_take` and not the house's own vault, the gate's own line and `HOLD_ONLY_SOURCE`, the
   `inventory` fact namespace (`UNSETTLEABLE`), the house's available ≥ gross (`UNFUNDED`), the pool
   less every in-flight sale (`SELL_SHORT`, `SOURCE_UNREADABLE`) through `pub(crate)
   delivery_pending` reading the applied family exactly via `Ctx::action_state`, the seller's
   account, the standing-sales and store caps, and the policy's split validated by `proceeds_valid`
   (one cut, same instrument, Σ ≤ gross, line and memo caps). Then `alloc_quote`, the Draw's window
   and cutoff, the marker from `peek_next_contract` + listing + quantity, `sale_spec` (seller
   issuer, house party and consideration, one outcome `delivered` paying `Exact(net)` to the seller
   then each cut, remainder to the house, `Before` + `Fact{inventory, inventory.transferred,
   marker}`, `Expiry::Refund`, `Parties`, deterministic), the id verified against the peek (else
   cancel + `UNSETTLEABLE`), the quote written before either acceptance, the seller's in-verb
   `accept_contract` (`SELL_NEEDS_APPROVAL`), the house's `accept_contract_as` (`HOUSE_CANNOT_PAY`),
   the leg through `paid_leg`'s new Deliver arm, `commit_quantity` (a Wanted is written back at
   `quantity − sold` including 0), activation, and the policy's line or `SOLD_GENERIC`. `pub fn
   restage_deliver(ctx, QuoteId) -> Result<Option<ContractId>, MarketError>` mirrors `restage_draw`
   (`Ok(None)` not re-stageable, `DELIVER_TOO_LATE` past the cutoff, `UNDELIVERABLE`, `Ok(Some)`
   byte-identical). The settlement ear calls `settled_sale` once behind the `q.settled` guard;
   `awaits_receipt_read` and `goods_moved` take the Deliver arm (marker read fail-closed);
   `release_quote` pages `deliver_lapsed_page` instead of restoring over goods the house received.
   Seventeen texts verbatim from the brief, re-exported.
6. **The fifth lot op** (`mush-market/src/lot.rs`). `LotOp::Deliver` (`"deliver"`, paid);
   `PaidLegs.deliver` and the seller-side walk over `quotes_of_seller` (Deliver, activated,
   unsettled, contract `Active`, `now <= draw_by`, the source among the actor's endpoints, ≤ 8
   entries, `"marker"` added to the entry); the optional `destination` parameter (Text 32);
   `prepare_deliver` requiring `endpoint == source`, a matching `deliver` entry through
   `matches_paid` (listing, destination, source, commodity, quantity), the marker from the entry,
   then `transfer_marked`; `LotLeg.destination` written by `stage` only when `Some`, so every other
   leg's parameters are byte-identical to IC9's; the size comment restated for three kinds.
7. **The bid's lifecycle** (`mush-market/src/{boot,api,commands}.rs`). `reconcile_draws` takes
   activated, unsettled Deliver quotes and does only the terminal check (Cancelled/Expired/gone →
   `release_quote`); the sweep keeps a lapsed Wanted at 0, filtered before the budget;
   `revise_wanted` accepts 0 (the exhausted marker) while `post_wanted` still refuses 0 and its
   duplicate guard now holds a zero bid; the list row prints ` — not buying` at 0; `+market/accept
   <quote>` calls `restage_deliver` after `restage_draw` with the same line; the lot arm's grammar
   `lot_sell_args` (bare digits on the left naming a viewable listing, a quantity, optional
   `from=<vault>`/`from=#<vault>`; `SELL_USAGE`, `SELLS_NOT_BUYS`; every `#`-prefixed left side
   stays the object arm); the help entry.
8. **`Ctx::authoritative_pending` on the command path** (`mush-core/src/engine.rs`, `output.rs`,
   `cmd.rs` — outside the brief, see Deferred). `Engine::step` notes the engine queue's keys before
   dispatch and clears the note after its one `drain_deferred`, as `fire_due` already did, so a
   command that stages a submission can refuse before it offers anything; the docs on
   `Outbox.pending_authoritative`, `note_pending_authoritative` and `Ctx::authoritative_pending`
   name both callers.
9. **`CoopProceeds`** (`tacsim/src/port/proceeds.rs`, new; `port/mod.rs`, `port/market.rs`,
   `port/board.rs`, `composition/mod.rs`). `pub fn share_of(price, reference, qty, permille) ->
   Option<(basis, share)>` in checked `i128`, floored. `CoopProceeds::proceeds`: no port for the
   venue → the market's own line and no cuts; the house from the map's vendor rows, the label and
   unit from the goods table, the house name through the factored `pub(crate) house_name(def, org,
   Option<&BidDef>)`, the signatory by name; enrolled iff `governed_hold` → the Trust's live lease →
   an enrolment with `vault == source` and `Active | BuyoutPending` (`SALE_BOOKS_DARK` for a live
   lease with dark books, `SALE_NOT_PILOT` for a seller who is not the enrolment's pilot); the
   reference read live from the programme's `reference[good]` entry's storefront ask through
   `lot_listing_of`, else the goods table's `reference_major` with `memo "baseline ref"` and one
   `tracing::warn!`, never zero (`REFERENCE_DARK`); one cut to the Trust's account when the share is
   positive; `SALE_LINE` / `SALE_LINE_NO_SHARE`. `settled` folds `sales`, `margin_minor`,
   `shares_minor` once, keyed by `pilot == seller && vault == source`, checked, through
   `put_enrolment`. Installed by `with_port_seams_if` beside the gate and capacity provider; `ports`
   and `port_of_venue` made `pub(crate)`; texts re-exported from `tacsim::port`.
10. **The clerk's bid re-post** (`tacsim/src/port/stock.rs`, `port_tick.rs`, `port/vendor.rs`).
    `bid_step`, reached when no ask is due: the first authored bid whose `BidRow` exists, whose
    listing is gone, whose deadline has passed and which `posts_afresh`, posted as the clerk through
    `post_wanted` with the applier's own factored `wanted_spec`; only that row's `listing` and
    `expires` rewritten; the actor swap's refusals mapped exactly as the ask list step maps them;
    one warn per `(vault, good)` per boot through `PortTickState.bid_warned`.
11. **The applier over a standing or gone bid** (`tacsim/src/port/vendor.rs`). The revise arm and
    the preflight drift check both saturate at 0 (an intake cut below what is left leaves the bid
    exhausted instead of failing the apply); a gone bid whose authored price or intake changed has
    its `BidRow` re-recorded with the new terms (listing and deadline kept) so the clerk posts it
    once the deadline passes; `posts_afresh` is `pub(crate)` with its comment rewritten;
    `retire_refusal` also refuses a quote **bought** by a vendor org with a contract and no
    settlement; a retire withdraws a zero bid and drops its row.
12. **Readiness, board, enrol gate** (`tacsim/src/port/readiness.rs`, `board.rs`, `coop.rs`).
    `Binding::InventoryFacts` right after `MarketFacts` ("inventory facts: answered, so sales into
    bids can settle." / "…nothing answers the inventory namespace, so sales into bids refuse."),
    required `Ready` by `enrol_gate`; the Bids line arms: a zero bid `Ready` "listing #n not buying;
    its intake is taken.", absent before its deadline `Missing` "no bid is posted.", absent past it
    `Pending` "lapsed; the clerk posts it again." when `posts_afresh`, else `Missing` "lapsed; it
    waits for the next apply."; the board renders a zero bid "wanted, {label}: not buying
    ({house})." and its head comment defers P07's funded-balance figure to IC12.
13. **Undock waits for the hand-over** (`tacsim/src/port/voyage.rs`). `Pending::Sale(QuoteId)`;
    `goods_pending_aboard`'s rule (0): a Deliver quote out of this hold, activated, unsettled,
    contract non-terminal, `now <= draw_by`, whose marker the inventory mirror has not seen; the
    destination loop's exhaustive match gains a Deliver arm reading the same mirror; `+ship/undock`
    refuses `GOODS_LEAVING` naming the quote and the remedy.
14. **Gates** (`mush-contract-tests/legacy-handlers.toml`, `state-slot-owners.toml`;
    `mush-market/tests/e7r2.rs`, `ic8.rs`). The `+market` note gains an IC10 paragraph (category and
    baseline unchanged); three new state-slot rows (`AccountOracle` for `sale.rs` and
    `ic_slice_loop.rs`; `module_state_insert` slot `store` for the market's fact-provider unit
    tests); the lot action has exactly five ops; the `(600, 0)` revise-refusal row left ic8 because
    the behaviour changed.

### Deferred, and why

The lead rulings first, then the builder's and the fixer's own calls, the one change outside the
brief, the story test's roster, the owner-critical defaults, the residuals, and what IC12's as-built
pass inherits.

- **Lead rulings taken** (the brief's decisions 1–24, each as written): the sale reuses `Quote`
  (buyer = house, seller = pilot, `PaidKind::Deliver`) with a `by_seller` index; the marker id is
  derived at offer time from (peeked contract, listing, qty) by inventory's `derived_marker_id` and
  verified after the offer; the Fact is inventory's own `inventory.transferred`, no market-side
  receipt family; the window and cutoff are the Draw's (3,600 / 600 real seconds); "no seller
  accept" is read as no step **asked** of the seller — the pilot accepts inside the verb before the
  house's programmatic accept, because `fully_accepted` needs every party; the exhausted marker is
  the Wanted listing itself at quantity 0, admitted by the wire for Wanted only, kept in `live`,
  never deleted by the sweep; the clerk re-posts a lapsed bid at its full authored intake and
  rewrites only the row's listing and deadline; the reference is read at contract build from the
  origin listing's current price, else the goods table's `reference_major` with a `(baseline ref)`
  memo and a warn, never zero; the share is floored in minor units at the enrolment's frozen
  permille; the counters fold through `ProceedsPolicy::settled`, called once by the market's ear
  behind `q.settled`, keyed by `pilot == seller && vault == source`; the sale line is the policy's;
  `MAX_PROCEEDS_CUTS = 1`; over-selling counts in-flight sales through `delivery_pending` reading
  the applied family exactly; undock refuses `GOODS_LEAVING` until the cutoff; `ic_slice_loop.rs`
  part 1 lives in `crates/tacsim/tests/`; the boards gain only the zero-bid guard and P07's
  funded-balance figure is deferred again to IC12; the hold-backed player path stays un-gated by any
  Fact; no pack moves; `restage_deliver` is public behind `+market/accept <quote>` and
  byte-identical through the one `paid_leg` builder; the lot arm's left side is bare digits and `#n`
  is always the object arm; the policy refuses `SALE_NOT_PILOT` as a belt behind the gate; an
  authored intake cut over a standing bid saturates at 0; the clerk's post maps the actor swap's
  refusals as the ask step does; the readiness Bids line reads `posts_afresh`.
- **Builder's own calls, recorded**: a missing vendor instrument at check 12 refuses `UNSETTLEABLE`;
  the quote id is allocated and `persist_meta` run after the policy check, so a later refusal burns
  an id; "gone" counts as ended for a Deliver quote in `reconcile_draws`; `CoopProceeds` answers
  `Proceeds::default()` for a venue no port authors or a house that is not a vendor row; the house
  name is the bid's `as` name, else the org's; a grammar refusal renders as `MarketError::Bad`; B
  naming A's hold with `from=` is refused by `may_take` ("No such listing."), pinned as the one arm
  that fires.
- **Fixer's rulings, recorded**: a **Disputed** sale stays in flight past its cutoff
  (`delivery_pending` skips the `draw_by` test when the contract is Disputed) so a seller who
  disputes their own sale cannot sell the same tonnes again while the house's escrow is frozen — the
  hull is still freed at the cutoff (`goods_pending_aboard` unchanged), which is safe because the
  count is per source; a gone bid whose authored terms changed has them **recorded in its `BidRow`**
  by the apply (listing and deadline kept) so the clerk posts it once the deadline passes, and the
  "waits for the next apply" line is truthful.
- **Outside the brief — the lead's sign-off is owed**: `Engine::step` now notes the queued
  authoritative submitters before dispatch and clears the note after the drain
  (`mush-core/src/engine.rs`). At HEAD only `fire_due` set the note, so a command always read
  `false`, the sale went through and its leg was dropped by back-pressure with the money already
  escrowed; brief check 6 and every `SELL_BUSY` test were impossible without it. The change is one
  line each side of the dispatch with no early return between them, the server's reused Outbox is
  cleared every step, and `ic10_a_command_sees_its_actors_queued_authoritative_submission` pins all
  four cases. It applies to every command, not only the sell arm.
- **The story test's roster**: `ic_slice_loop.rs` composes the production layer
  (`tacsim_module_wired`), but a TestWorld boots at world rate 1 and the production Rate line gates
  `+coop/enrol`, so the test types `@tacsim/time rate=24` at boot — doc 18 §4.7's dev1 deploy step —
  and makes `rate=3600` the one staff act during play, after the enrolment and the purchases (§6's
  order). Doc 18 §6's "rate 3600 the only staff act, none after boot" cannot hold verbatim on the
  production composition; the wording and the `build_engine` promotion are IC12's.
- **Owner items shipped on the brief's defaults** (owner-critical): **O1** every new player-facing
  text (the sell refusals, `SOLD_GENERIC`, `DELIVER_TOO_LATE`,
  `SALE_LINE`/`SALE_LINE_NO_SHARE`/`SALE_BOOKS_DARK`/`REFERENCE_DARK`/`SALE_NOT_PILOT`, the
  readiness and board lines, `GOODS_LEAVING`) is the planner's wording. **O2** a cross-port or
  under-way sale is refused in `PortVenueGate`'s load-worded IC8 lines (`HOLD_ELSEWHERE` "…so its
  hold cannot load here. Nothing was charged."), asserted as they are; sale variants through a
  purpose argument on `VenueGate::admit` are the owner's or IC14's. **O3** an unlanded sale holds
  Kasei's payment, the bid's intake, the tonnes and the hull for up to the 3,000 s cutoff and
  refunds at 3,600 s; the remedy is `+market/accept <quote>`. **O4** (a) the clerk re-posts a lapsed
  bid at full authored intake, (b) a refund of the last tonnes past an exhausted bid's deadline
  re-opens it, the sweep then deletes it and the clerk re-posts it at full intake (pinned in ic10.rs
  test 10), (c) a retire withdraws a zero bid and a re-apply posts it afresh — re-posting at the
  lapsed remainder is a P5 matter. **O5** the share is floored.
- **Residuals carried** (IC12/IC14 and later): the reference drift between build and settlement, at
  most 3,600 s; settled sales into bids on the trade tape (pinned); Kasei's 32-live-contract party
  cap counting unlanded legs for up to an hour; a settlement healed at boot not folded into the
  counters; no Fact gate on the hold-backed player path; **no retention for `inventory_applied` past
  4,096 markers — now on the money path, to land before any deploy that enables selling**; a full
  submit queue still drops a deliver leg after escrow and the sale line does not name the remedy;
  `restage_deliver` past the cutoff tells a seller whose leg landed in the last ~90 s "Nothing
  moved" before it settles; the market's duplicate of inventory's marker-id rule (`is_marker_id`,
  `MAX_MARKER_ID_BYTES`); `settled_sale`'s `i64::MAX` gross fallback; no `+market/orders` view of a
  seller's own sales; P07's funded-balance board line; `sale.rs` teleports into P07 where the exits
  allow a walk.
- **For IC12's as-built pass**: the §5 IC10 row's "70 t … 21,000 / 19,250 / 1,750" are pre-R7
  figures at a 300 bid; lane v3's 280 bid gives 5,600 / 5,200 / 400. Doc 18 §6's sale line reads
  "Sold 20 t metal concentrate … 5,600.00cr; … you received 5,200.00cr."; the shipped line reads
  "Sold 20 t of metal concentrate to Kasei Metals at 280.00cr/t: 5600.00cr; Co-op share 400.00cr;
  you receive 5200.00cr when the goods are in. Jonas Achterberg-Nkemelu countersigns." (the "of", no
  thousands grouping in money — the economy's one render — and the future tense because money moves
  only after the marker). Every Stickney exit word §6 writes is a pack alias; the Ceres words used
  are `c`, `registry`, `out`, `concourse`, `exchange`, `chandlery`, `arrivals`, `lobby`, `gallery`.

### Pins re-blessed

- **`mush-market/tests/e7r2.rs`**: `e7r2_p1_the_lot_action_has_exactly_four_ops` →
  `…_exactly_five_ops`, the word list gaining `"deliver"`. Reason: IC10 adds the paid deliver leg
  (doc 18 §4.3); the market-never-mints pin stays green because `transfer_marked` lives in
  mush-inventory.
- **`mush-market/tests/ic8.rs`, `ic8_post_and_revise_wanted_refusals`**: the `(600, 0, "That is not
  a quantity this market buys.")` row left. Reason: `revise_wanted(.., 0)` now writes the exhausted
  marker (the brief's item 6.6; item 7 above); `post_wanted`'s refusal of 0 is still pinned there,
  and `tests/ic10.rs` test 10 pins the new acceptance.
- **`tacsim/tests/ports.rs`, `ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`**:
  the exact `want` vector gains `"inventory facts: answered, so sales into bids can settle.
  [ready]"` right after the market-facts entry. Reason: the new `Binding::InventoryFacts` line (the
  brief's item 9.5; item 12 above).
- **`tacsim/tests/commerce.rs` `stock_ready`, `coop.rs` `commerce_ready`, `voyage.rs` `stock`**:
  each gated-line filter gains `|| line.starts_with("inventory facts")`. Reason: the line now gates
  enrolment, so a helper that waits for readiness waits for it too (stricter, not weaker).
- **`mush-contract-tests/legacy-handlers.toml`**: the `+market` stanza's note refreshed with an IC10
  paragraph; category `authoritative` and `PINNED_BASELINE` unchanged (no new primary verb, no new
  switch).
- **`mush-contract-tests/state-slot-owners.toml`**: three new rows — `World::module_state_mut` /
  `AccountOracle` for `crates/tacsim/tests/sale.rs` and `crates/tacsim/tests/ic_slice_loop.rs` (the
  voyage.rs reason: `+coop/enrol` is per gateway account and a TestWorld has no store), and
  `World::module_state_insert` / slot `store` for `crates/mush-market/src/facts.rs`'s unit tests
  (test-only; the composed path is `Registrar::state`).
- **Did not move**: the panic budgets, the restriction-lints census, every content digest (lane v3,
  Dray v2, goods v1, Stickney v2 — no file under `crates/tacsim/content` changed; the test lane
  variants exist only inside tests), the composition-edge pin, `module-ns-owners.toml`, the CI
  gated-test inventory (no `#[ignore]` added), the `mush-inventory/tests/e7r2.rs` FactProvider
  count, `families.rs`'s golden `StockIssueId` pin.

### How it was verified

The judge read the whole uncommitted diff (33 files, +2,410/−148) and the five new files
(`mush-inventory/tests/ic10.rs`, `mush-market/tests/ic10.rs`, `tacsim/src/port/proceeds.rs`,
`tacsim/tests/sale.rs`, `tacsim/tests/ic_slice_loop.rs`) against the brief, doc 18
§4.1/§4.3/§4.7/§5/§6 and D-18-24/26/27, and ran no cargo of its own (verdict: ship with notes).
Three review lenses ran before the fixer pass (correctness: F1–F5; conformance: C1–C9; tests:
T1–T10); the fixer took every should-fix and note it could (F1, F2, F3/C2, C1 docs + core test, C3,
C4, C5, T1–T6) and mutation-checked the two production changes (reverting the Disputed clause fails
`ic10_a_disputed_sale_keeps_its_tonnes_committed_past_the_cutoff`; reverting the BidRow recording
fails `ic10_a_gone_bid_with_a_changed_intake_is_posted_at_the_new_intake`).

**The gate**, all green on the first run; logs under the run's `cuts/IC10/gate1/`:

- `cargo fmt --all -- --check` → exit 0, no output.
- `cargo clippy --workspace --all-targets -- -D warnings` → ``Finished `dev` profile [unoptimized +
  debuginfo] target(s)``, 0 warnings (the fixer's first run flagged one
  `clippy::manual_is_multiple_of` in the story test's `grouped`, fixed).
- `cargo test -p mush-inventory` → e7c2 `ok. 65 passed`; e7r2 `ok. 16 passed`; ic10 `ok. 4 passed`.
- `cargo test -p mush-market` → lib `ok. 27 passed`; e7r2 `ok. 21 passed`; ic0 `ok. 5 passed`; ic10
  `ok. 13 passed`; ic7 `ok. 18 passed`; ic8 `ok. 9 passed`; projection_bounds `ok. 2 passed`;
  projection_cost `ok. 1 passed`; doc `0 passed; 1 ignored` (existing).
- `cargo test -p mush-economy --lib` → `ok. 37 passed`.
- `cargo test -p mush-core --lib` → `ok. 397 passed` (the new
  `ic10_a_command_sees_its_actors_queued_authoritative_submission` included).
- `cargo test -p tacsim --lib` → `ok. 900 passed`.
- `cargo test -p tacsim --test sale --test ic_slice_loop` → sale `ok. 15 passed`; ic_slice_loop `ok.
  1 passed`.
- `cargo test -p tacsim --test commerce --test coop --test voyage --test ports --test port_tick
  --test hulls --test capacity` → capacity 17, commerce 20, coop 18, hulls 15, port_tick 16, ports
  17, voyage 13, all `ok`.
- `cargo test -p tacsim --test lane --test stations --test content --test definitions --test
  composition_edges` → composition_edges 2, content 29, definitions 25, lane 8, stations 13, all
  `ok`.
- `cargo test -p mush-contract-tests` → 29 binaries all ok, 0 failed (lib 31;
  legacy_handler_allowlist 4; state_slot_owners 4; module_ns_owners 3; panic_budget 7;
  restriction_lints 11; scale_harness 6 passed / 8 ignored, existing). The builder's first run
  failed `every_cross_owner_state_site_is_baselined` on the facts.rs `module_state_insert` site; the
  row was added and it went green.
- `cargo test -p mush-server --lib modules::` → `ok. 32 passed`.
- `cargo test -p mush-server --test boot_parity --test e7r2_proving_loop --test market_issuer` → 18
  / 6 / 4 passed.
- `cargo test -p mushd --test boot_composition` → `ok. 6 passed`.
- `CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast` → 398 `test result:
  ok` lines, 0 FAILED; **7,438 passed, 0 failed, 92 ignored**; about 30 minutes. None of the known
  load-sensitive suites failed. The three `panicked` lines in the log are the WS01-S8
  fault-injection test's own armed panic and two passing process_fault test names.
- `cargo test -p mush-bot` → all 22 binaries ok (lib 469; arena_e2e 9 in 201 s; playtesting 9 in 125
  s; 5 ignored across arena_live_commander / arena_muster_rate / arena_win_rate, existing).

No flake was seen; nothing was rerun. The verify lens's figures, confirmed from `sale.rs` test 1's
assertions: Kasei −560,000 = pilot +520,000 + Trust +40,000 from the settlement journal, summing to
zero; hold metals 20 → 0 and Kasei's storefront 0 → 20 with the storefront root inheriting the
hold's root and the applied marker naming from/to/20/the pilot; four sweeps between the sale and the
landing leave the contract Active with nobody paid and 560,000 held; after the landing `unswept`
asserts the Fact answers nothing and the contract still stands until the inventory mirror sweeps;
the bid 4,000 → 3,980 on readiness and the board; enrolment (1, 160,000, 40,000); the statement
line; the trade-tape row. The share-0 case (test 2), the live reference at 220 (test 2b), the
baseline fallback with the `(baseline ref)` memo (test 3), and the story test's two sale lines
computed from the posted bid, the live origin ask and the enrolment's permille are each asserted as
exact lines.

### Surface for the next cuts

**mush-inventory** (`src/pool.rs`, re-exported from `lib.rs`)

- `pub const MARKER_CAUSAL_MAX_BYTES: usize = 512` (the canonical-input cap).
- `pub fn derived_marker_id(at_ic: i64, causal: &serde_json::Value) -> Result<SmolStr,
  InventoryError>` (always 80 lowercase hex; any hash error → `Identity`).
- `InventoryFamilies::transfer_marked(&self, txn: &mut ActionTxn, from: EndpointRef, to:
  EndpointRef, commodity: &CanonicalId, amount: u64, marker: &SmolStr, actor: ObjId, now:
  &ClockStamp) -> Result<Moved, InventoryError>` (refuses `SameEndpoint`, a bad id with `Identity`,
  an existing marker with `AlreadyExists`; nothing written on error);
  `InventoryError::SameEndpoint`.

**mush-market** (`lib.rs` re-exports)

- Records: `PaidKind::Deliver` (`"deliver"`), `PaidLot.marker: Option<SmolStr>`, `ProceedsCut { to:
  AccountId, amount: Amount, label: SmolStr }` with `check()`, `MAX_PROCEEDS_CUTS = 1`,
  `MAX_PROCEEDS_LABEL_BYTES = 32`, `MAX_MARKER_ID_BYTES = 96`, `is_marker_id(&str) -> bool`,
  `Quote.cuts: Vec<ProceedsCut>`, `Quote.basis: Amount`; a `Supply::Wanted` listing may stand at
  `quantity == 0`.
- Store: `MarketStore::quotes_of_seller(&PrincipalId) -> impl Iterator<Item = &Quote>`; `live`
  admits a zero Wanted.
- Seam: `trait ProceedsPolicy { fn proceeds(&self, &World, &SaleTerms<'_>) -> Result<Proceeds,
  ProceedsRefusal>; fn settled(&self, &mut Ctx<'_>, &SettledSale<'_>) {} }`, `SaleTerms { seller:
  ObjId, source: EndpointRef, house: &PrincipalId, market: &MarketId, listing: ListingId, commodity:
  &CanonicalId, quantity: u64, price: Amount, gross: Amount }`, `Proceeds { cuts, basis, line:
  Option<String>, memo: Option<SmolStr> }` (+ `Default`), `ProceedsRefusal { line: String }`,
  `SettledSale { quote, contract, seller: &PrincipalId, source, commodity: &CanonicalId, quantity,
  price, gross, cuts: &[ProceedsCut], basis }`, `MAX_SALE_LINE_BYTES = 512`,
  `MAX_PROCEEDS_MEMO_BYTES = 24`, `MarketModule::with_proceeds_policy(Arc<dyn ProceedsPolicy>)`.
- Trade: `pub fn sell_into_bid(&mut Ctx, &BidSaleRequest { listing: ListingId, quantity: u64,
  source: Option<EndpointRef>, presence: Presence }) -> Result<BidSale { contract: ContractId,
  quote: QuoteId, gross: Amount, cuts: Vec<ProceedsCut>, net: Amount, line: String }, MarketError>`
  — IC11's executor calls it with `Presence::Ship(hull)` and `source: None`; `pub fn
  restage_deliver(&mut Ctx, QuoteId) -> Result<Option<ContractId>, MarketError>` — IC11 verifies an
  in-flight sale after a restart through it (`Ok(None)` not re-stageable, `Err(DELIVER_TOO_LATE)`
  past `draw_by`, `Err(UNDELIVERABLE)`, `Ok(Some(contract))` re-staged byte-identically).
- Texts: `NOT_BUYING`, `BID_LAPSED`, `OVER_INTAKE` (`{remaining}`), `GRADED_BID`, `SELL_BUSY`,
  `NAME_SOURCE`, `HOLD_ONLY_SOURCE`, `UNSETTLEABLE`, `UNFUNDED`, `SOURCE_UNREADABLE`, `SELL_SHORT`
  (`{held}`, `{qty}`), `SELL_NEEDS_APPROVAL`, `HOUSE_CANNOT_PAY`, `SELLS_NOT_BUYS`, `SELL_USAGE`,
  `SOLD_GENERIC`, `DELIVER_TOO_LATE`.
- Lot action: five ops `reserve, consume, release, draw, deliver`; the `paid` fixed parameter's
  deliver entry `{op:"deliver", quote, listing, source, destination, commodity, quantity, grade,
  plan:null, marker}` keyed by contract id in decimal, offered only to the seller while the contract
  is `Active` and `now <= draw_by`; the staged leg's parameters `{op:"deliver", listing,
  endpoint:<hold>, source:<hold>, commodity, quantity, grade:0, destination:"<storefront vault>",
  contract:"<id>"}` under key `market.lot.deliver.{contract:x}`; `LotLeg.destination:
  Option<EndpointRef>`; `revise_wanted(.., 0)` accepted.
- The sale contract: name `market sale #{quote} to bid #{listing}` (+ ` ({memo})` when it fits 64
  bytes), parties `[Organization(house)]`, consideration `(house account, gross)`, outcome
  `delivered` paying `Exact(net)` to the seller then each cut, remainder to the house, conditions
  `[Before{RealUtc, accept+3600}, Fact{inventory, inventory.transferred, <marker>, None}]`,
  `Expiry::Refund`, deterministic; cutoff `draw_by = accept + 3000`.

**mush-core**

- `Ctx::authoritative_pending(actor)` is now true on the command path when the engine's queue
  already holds that actor's submission (noted by `Engine::step`, cleared after its drain).

**tacsim** (`tacsim::port` re-exports)

- `CoopProceeds` (unit struct, `ProceedsPolicy`), `share_of(price: Amount, reference: Amount,
  quantity: u64, permille: u32) -> Option<(Amount, Amount)>`, `SALE_LINE`, `SALE_LINE_NO_SHARE`,
  `SALE_BOOKS_DARK`, `REFERENCE_DARK` (`{label}`), `SALE_NOT_PILOT` (`{name}`), `SHARE_LABEL =
  "Co-op share"`, `BASELINE_MEMO = "baseline ref"`; `tacsim::port::voyage::GOODS_LEAVING`
  (`{quote}`, `{name}`); `Binding::InventoryFacts`.
- Readiness Bids lines `"{head}: listing #{l} not buying; its intake is taken."` [ready], `"{head}:
  no bid is posted."` [missing], `"{head}: listing #{l} lapsed; the clerk posts it again."`
  [pending], `"{head}: listing #{l} lapsed; it waits for the next apply."` [missing]; board `"
  wanted, {label}: not buying ({house})."`.
- `pub(crate)`: `port::market::{ports, port_of_venue}`, `port::board::{unit_of, label_of, money,
  house_name}`, `port::vendor::{posts_afresh, wanted_spec}`, `PortTickState.bid_warned`,
  `Pending::Sale(QuoteId)` in `goods_pending_aboard` (IC11's depart asks it).
- `Enrolment.{sales, margin_minor, shares_minor}` are now written, once per settled sale, through
  `store::put_enrolment`. No pack version moved: lane v3, Dray v2, goods v1, Stickney v2.

## IC11

**Captain's orders: the first cut with the full daily loop** (doc 18 §5, the IC11 row and paragraph;
§4.4; D-18-31, D-18-32, D-18-41), built 2026-10-17 on main at 7435f954. A chartered pilot files the
day's sailing plan with `+ship/order sell|buy|depart|stop` (and `del`, `move`, `clear`, `pause`,
`resume`, `skip`), reads it with `+ship/orders`, dry-runs it with `+ship/plan` and logs off; the
port tick's new order arm executes it as the pilot. The plan is one durable legacy document per
hull, `tacsim:orders:<hull>`, written whole by `put_orders`, which runs `Orders::check` first so no
writer can store a shape the loader refuses; an unreadable one fails closed and pages once a boot.
Every rendered log line names the document's author as signatory; the stored text stays bare.

The executor rotates over hulls with no pending voyage, re-checks the live lease before every act,
pre-checks the next document before any closed call, and swaps to the author for ONE call: `quote`
then `accept_at` with `Presence::Ship` (the clamp min(qty, stock, room by mass, room by volume,
funds less the next departure's unmet fuel and stores), every bound logged), `sell_into_bid`,
`restage_draw`/`restage_deliver` at 60 s and again 160 s after the first re-stage (a second miss
holds over the step and heals on a landing), or the `tacsim_port_burn` submission keyed
`tacsim.port.burn/<hull>/<order>/<attempt>` through the one `prepare_departure` that `+ship/undock`
also uses. The inflight record rides the same outbox; completion is verified by pull on later fires;
a reboot verifies and never re-runs; a queued depart whose burn produced no ticket is re-filed under
the next attempt and a fresh seq, never under its old key. `fill` is the joint least fixed point at
or above what is aboard over the projected load, planned at the depart's `not_before` through the
moving sky and never now; the cargo clamp reserves that departure's needs. Daily departures resolve
at the reach the executor records, so one that becomes the cursor under way does not sail on
arrival. Holds consume nothing and notify; every done, held or skipped step queues a reconnect
notice under "While you were off watch:". A lapsed lease is a waiting note; `NO_ROUTE`, a past
window, a consumable short of the passage, three refusals, three drops, a second unverified miss and
a staff stand-down are stopped holds that need `resume`. The launch arm tells `CLEARANCE_LAPSED`
only for a live undock and holds a launch whose ticket disagrees with the filed passage (IC9's F2);
`@tacsim/port clear` records a stood-down queued depart on its plan. One commit, pushed to
origin/main.

### What landed

1. **The UTC time helper** (`crates/tacsim/src/port/when.rs`, new). `NotBefore::{Now, Utc(i64),
   UtcDaily("HH:MM")}` as the depart's `not_before` wire; `parse_at` (`now`, `HH:MM` to the next
   such UTC minute, `daily HH:MM` consuming its next token, `+Nh`/`+Nm`, at most `MAX_AHEAD_SECS` =
   7 days), `parse_until` (`+Nh`/`+Nm` up to `MAX_WINDOW_SECS` = 24 h), strict `hhmm_minutes`, and
   `resolve_due`, the one rule for when a depart is due (lead ruling L9): a daily departure resolves
   at its **reach** and is never missed — inside its window it leaves now, past it, it waits for the
   next day's hour. `ic_at_utc` and `plan_instant` derive every IC figure live through the current
   timeline generation (never in the past), so a rate change relabels nothing; `render_due` is the
   OOC departure clock (`HH:MM UTC today (IC <civil>; in <span>)`, `IC clock stopped` at rate 0,
   `now` for a past due). Texts `BAD_TIME`, `BAD_UNTIL`. Four unit tests, including two consecutive
   daily departs resolving at their own reach (reviewer F8).
2. **The sailing-plan document** (`crates/tacsim/src/port/orders.rs`, new;
   `crates/tacsim/src/store.rs`). `tacsim:orders:<hull>` (D-18-31): `Orders { schema 1, hull,
   author, rev, state running|paused|held, hold, cursor, next_id, inflight, orders ≤ 16, log ≤ 32 ×
   240 bytes }`, the closed vocabulary `What::{Buy, Sell, Depart, Stop}` on the nested wire (L3;
   `Qty` writes `"all"`, `"fill"` or an integer), `Order { id, state, attempt, refusals: u8 (no
   serde default, L26), retry_at, announced, reached: Option<Reached{utc, venue}>, what }`,
   `Inflight { order, attempt, key, refs, staged_utc, issued_ic, restaged ≤ 2, want, qty, bounds,
   line }`, every struct `deny_unknown_fields`. `Orders::check` names the first broken rule and
   admits exactly the four hold shapes (stopped, stopped over inflight, waiting, waiting over
   inflight; reviewer F2), plus one rule the builder added: a `Held` order is named by a stopped
   note. Helpers `fresh`, `derive_cursor` (clears `reached` off every order but the new cursor),
   `push_log` (flattened, cut on a char boundary, FIFO), `clear_hold_naming`, `stop_at`, `wait_at`,
   and `seed_next_id` (the replaced document's `next_id`, else one past the highest order id on the
   hull's last 64 tickets; L29). The store gains `orders`/`orders_keys`, `orders_state` (`Absent |
   Present | Unreadable`), `orders_docs`, `orders_unreadable`, the load arm with the identity rule
   (an unreadable body keeps its key claimed and pages), the module-doc table row, and `put_orders`,
   the single write path, which **runs `check` first** and installs and writes nothing on a failure.
   Unit tests: strict keys, every bound, the four shapes round-tripped through serde and `check`,
   `put_orders` refusing a broken document with nothing written then writing a good one once, and
   the doc 18 §4.4 example on the nested wire with an extra field refused at every level.
3. **The verbs** (`orders.rs`; `crates/tacsim/src/port/ship.rs`; `crates/tacsim/src/commands.rs`).
   `+ship` gains the switches `orders` and `order` on its one Authoritative `CommandSpec`,
   dispatched ahead of the rest. `+ship/orders` reads anywhere with a live charter: the header with
   revision and state, each order's summary and state word (`next`, `pending`, `at the counter`,
   `done`, `skipped`, `held`) with `, at {port}` where it runs elsewhere, `Next due:` in its forms,
   `Held:`/`Waiting:`, and the last eight log lines each rendered `HH:MM UTC, order n, signed
   {pilot}: …` from the document's author (Q-18-24, L27; the stored text stays bare). `+ship/order
   sell|buy|depart|stop|del|move|clear|pause|resume|skip` behind the gates in the brief's order —
   charter, document (an unreadable one `ORDERS_DARK`; another author's reads as absent unless its
   step is in flight, `ORDER_INFLIGHT`), where (aboard, or a `port_rooms` room of the berth;
   `market.rs`'s `port_rooms` made `pub(crate)`), grammar, then ONE checked `put_orders` with `rev`
   bumped. Goods by id or unique label prefix, `fill` only on a consumable, `depart` with
   `at=`/`until=`, `via=` refused `VIA_NOT_YET`, the listed passengers taken from `souls_aboard` at
   filing (L11), a 17th order dropping the oldest finished one, `del`/`skip` clearing a note that
   names the order, `clear` keeping the inflight order, `resume` re-arming the re-stage clock over
   an inflight step and "sailing now" past a fixed window. Every text verbatim from the brief;
   additions `CLEARED_INFLIGHT`, `RESUMED_EMPTY` and `ORDER_FINISHED` (skip of a done or skipped
   order refuses, writing nothing). `SHIP_HELP` gains the two sentences; `AT_NOT_YET` now names
   `+ship/order depart <port> at=<time>`.
4. **The planning core** (`crates/tacsim/src/port/executor.rs`, new; D-18-41, lead note 2i). One
   module `+ship/plan` and the executor share: `ask_at` (the lowest live Draw ask through
   `stock::ask_reading`) and `bid_at` (the best standing Wanted bid, else the IC10 exhausted marker,
   with `house_units`); `Bound` in log order (`stock, hold mass, hold volume, funds`…) and
   `join_bounds`; `clamp_buy`/`clamp_sell` naming every term that attains a short result;
   `funds_units`; `room_for` as `capacity::max_more` over the good's compartment, the tank and
   locker told apart by `compartment_for` over the two consumable goods (never guessed:
   `CoreError`); `project_cargo` (sells then clamped buys in queue order, explicit consumable buys
   as extra kilograms, no funds or listing term); `fill_targets`, the joint least fixed point at or
   above what is aboard in ≤ `FILL_ITERATIONS` (6) with `TankTooSmall`/`LockerTooSmall`;
   `reserve_for` over the next depart's unmet fuel and stores at this venue's asks, excluding the
   order's own good (L17); `Planner`/`planner`, `next_depart_targets` (the first open depart at or
   after `from`, over the projection from `from` inclusive, at its `not_before` through
   `plan_instant`) and `reserve_at`. Unit tests derive the vertex with `max_more` (never the
   literals) and reproduce doc 18's 37 t and 19 crew-days for the 73 t projection through the real
   planner at lane v3.
5. **`+ship/plan` over the queue, the status line and `+ship`** (`executor.rs::plan_queue`;
   `crates/tacsim/src/port/voyage.rs::plan_cmd`; `ship.rs`). After the per-destination blocks and
   before `NOTHING_WRITTEN`, the dry run of the author's queue from the cursor through the first
   depart over a projection of the live readings and funds: each sell's clamp and the net the
   `CoopProceeds` policy would pay, each buy's clamp and bound, each fill at the depart's
   `not_before`, the depart planned on the projected load (a refusal's sentence inlined without
   "Nothing was burned." and without a second full stop), `Orders a to b run at {port}; they are
   planned there.`, and the affordable line for every ask on the board less the next departure's
   reserve (that departure itself when it is the cursor).
   `STATUS_DOCKED_DEPARTS`/`SHIP_DOCKED_DEPARTS` name the hour of a pending depart at the cursor
   through `orders::departs_at`, which requires the plan to run under its author's live charter of
   this hull (L14); unchanged otherwise.
6. **The shared departure filing, the launch arm's order hooks, the stand-down and IC9's F2**
   (`voyage.rs`). `undock_cmd`'s body is factored into `prepare_departure(ctx, pilot, hull,
   dest_word, Queued::{Live, Order{order, attempt}}) -> Result<DepartFiling{voyage, port, key, seq,
   burn}, DepartRefusal{kind, line}>` with the fifteen `DepartRefusalKind`s in today's order
   (reviewer F7); `undock_cmd` goes through it byte-identically. The plan is made now — the burn
   refuses a future `depart_ic` — so a queued `at=` is the executor's due gate (L8). A queued
   depart's key is `tacsim.port.burn/<hull>/<order>/<attempt>`. `launch_step` sends
   `CLEARANCE_LAPSED` only for a live undock (a queued depart's lapse is its "not admitted", logged
   at `info!`, L7/L12) and, before flying, holds the launch when the ticket's plan, order, attempt,
   route pack, route version or unit definition disagree with the pending (L19, IC9 residual F2
   closed). `clear_cmd` records a stood-down queued depart on its plan through
   `executor::stand_down` (a stopped `HELD_STOOD_DOWN` hold, the attempt consumed, the inflight
   record dropped, `HELD_LINE` logged) and appends ` The sailing plan holds at order {n}.` (L28).
   `souls_aboard`, `lane_ports`, `venue_name`, `ticket_at`, `launched` are `pub(crate)`; `no_route`
   returns a `String`.
7. **The order arm** (`executor.rs`; `crates/tacsim/src/port_tick.rs`; D-18-32). `Arm::Order { hull
   }` ahead of the probe; `order_verify` at the top of every fire (unreadable plans paged once a
   boot through `orders_warned`; inflight records judged in hull order rotating from
   `order_verify_after`, ≤ `MAX_ORDER_VERIFY_PER_FIRE`): a buy Landed at `drawn_at` or `settled`
   (L5), a sale at settlement, a depart when its ticket reads and `launched`; Dropped on a gone
   quote or a terminal unsettled contract, or no ticket with the pending gone → `attempt + 1`,
   `NOT_ADMITTED` logged, `retry_at`, a stopped hold `HELD_UNCONFIRMED` past `MAX_ORDER_ATTEMPTS`; a
   ticket with the pending gone and no journey → the stand-down fallback; an unreadable ticket with
   the pending gone → a stopped hold over the step. A Landed step is `Done`, its hold self-heals,
   its line is logged and told (a depart's `CLEARANCE_GRANTED` line logged only), and the
   left-behind rule tells the pilot and each listed passenger ashore (L11). `order_agents` offers
   every readable plan with no pending voyage that has a market leg in flight (plan not held) or
   runs with a cursor. `order_step` → `decide`: the act due (a re-stage at `staged_utc + 60 s`, the
   second deadline `ORDER_SETTLE_GRACE_SECS` = 160 s after the **first re-stage**, the clock
   re-armed by a late first re-stage; or the cursor order past `retry_at`), the lease before any act
   (a waiting `HELD_CHARTER` note, dropped silently when the charter returns; L25), then ONE closed
   call as the author inside `Fire::as_principal`: `restage_draw`/`restage_deliver` (a second miss
   holds over the step and pages; `_TOO_LATE` logs `AWAITING_REFUND`); **sell** through `bid_at`,
   `read_pool` (a refused read is never 0), `clamp_sell`, skip-or-hold on
   `NO_BID`/`NOTHING_ABOARD`/`UNDER_MIN`/`INTAKE_TAKEN`/`HOUSE_CANNOT_PAY_SKIP` (the market's
   `HOUSE_CANNOT_PAY` mapped, never the retry path), then `sell_into_bid(.., Presence::Ship(hull))`;
   **buy** through `ask_at`, the fill targets or the explicit count, `room_for`, the reserve
   excluding its own consumable, `clamp_buy` (no live ask → stock alone binds),
   `HELD_SHORT_FOR_DEPART`, `BOUGHT_NONE`, then `quote` + `accept_at(.., None,
   Presence::Ship(hull))`; **depart** through the durable reach, `resolve_due`, `DEPART_SET` once,
   `HELD_WINDOW`, then `prepare_departure` — transient refusals as waiting notes (a dark plan paged
   once a boot), `NO_ROUTE` a stopped hold, `Ok` → `put_voyage`, the burn submission and the
   inflight record in the one outbox; **stop** marks itself done and pauses. The pre-check
   (`staged`) builds the next document and runs `check` before any call; `refs` are filled after.
   Refused calls count `refusals`, log `REFUSED_RETRY` once and hold at `MAX_ORDER_REFUSALS` with
   the market's line (`market_line`, never `refuse_market`). Holds and skips narrate the Bridge
   chime; every Done, Held and Skipped line is a reconnect notice. Every executor text verbatim;
   additions `BOUGHT_SHORT`, `SHORT_CLAUSE`, `SALE_SHORT`, `HELD_FILL_PLAN`.
8. **Gates** (`crates/mush-contract-tests/legacy-handlers.toml`, `state-slot-owners.toml`). The
   `+ship` stanza's note refreshed with the IC11 paragraph ahead of the IC9 text (category and
   `PINNED_BASELINE` unchanged); one `[[site]]` row for `crates/tacsim/tests/orders.rs`
   (`World::module_state_mut` / `AccountOracle` / owner `sim`).
9. **Tests** (`crates/tacsim/tests/orders.rs`, new, 26; `crates/tacsim/tests/voyage.rs`, +1; 16 lib
   tests across `when.rs`, `orders.rs`, `executor.rs`). Lens 1 in full
   (`ic11_the_queue_runs_the_day_unattended`: the two burn keys `…/1/1` and `…/8/1`, two settled
   sales, four Draws from the tick, the hold's pools, the funds path from journals, the nine log
   lines in order with no waiting line, the alloys cut to 20 t by funds, the reconnect heading once
   and every notice once in queue order, the render signed); lens 2
   (`ic11_a_reboot_mid_step_reruns_nothing` phases (a), (b), (c) with every post-boot market
   submission counted, and (c')); `ic11_a_draw_lost_to_a_reboot_is_restaged_from_the_tick`; the
   stand-down; the lapsed charter (waiting over inflight pinned, nothing staged through the refund
   sweeps, the note moving with the drop and told once); the false second miss (exactly three legs,
   `since` past the whole grace, told once after the heal); the long downtime; the sold-out partial
   and the no-ask `BOUGHT_NONE … stock`; the unfunded bid; the house that cannot pay; the depart
   short of fuel; `NO_ROUTE`; the cargo clamp leaving the reserve (the pilot's real balance read
   once the alloys are in flight, and the proof the reserve bit); every bound that bit (24 of 55
   `hold volume`, 85 `hold volume`, `hold mass and hold volume`);
   `fill_and_clamp_plan_at_the_depart_instant` with the funds set so the not-before and "now"
   reserves afford different whole tonnages; two pilots at one hour; the pilot ashore; the listed
   passenger left behind; a rate change; stop and resume; held steps; the dry run with the full
   depart and affordable lines and no tacsim-namespace op; the status line, the lapsed tenant's
   clause gone, and another author's plan showing no hour. `voyage.rs`:
   `ic9_f2_a_ticket_that_disagrees_with_the_filed_plan_holds_the_launch`.

### Deferred, and why

The lead rulings L1–L29 first, then the builder's own forks, the owner items built on the brief's
defaults, the lead questions this cut raises, and what is not in this cut.

- **L1 Scope and split.** IC11a (items 1–6) and IC11b (items 7–9) both landed; no half-built
  executor.
- **L2 The sell order names a good, not a listing** (doc 18 §4.4 "Verbs" over lead note 2k's `sell
  <listing>=<qty>`): the bid is resolved at the venue when the step runs (`bid_at`); a queue filed
  at Ceres cannot name a Stickney listing.
- **L3 Nested wire** for `What` (`deny_unknown_fields` cannot flatten; IC9's `FixedVoyage`
  precedent); doc 18's field names and values kept.
- **L4** The buy entry is `quote` then `accept_at(.., None, Presence::Ship(hull))`.
- **L5 A buy is done when drawn** (`drawn_at` or `settled`), not when its contract settles — a
  recorded deviation from §4.4's "lot in the endpoint and contract settled": the escrow is posted at
  accept and the gates read pools, so the settlement leg is skipped per buy; a sale is done only at
  settlement.
- **L6 Re-stage cadence**: the first re-stage at `staged_utc + 60 s`, the second deadline 160 s
  after the first re-stage (the fixer re-arms `staged_utc` on a late first re-stage so the grace
  always runs from it — a small extension of the brief's "re-armed by resume"); a second miss holds
  over the step and self-heals on a landing. Lead note 2k's "once per fire" narrowed.
- **L7 "Not admitted"**: a queued depart whose burn produced no ticket is re-filed under `attempt +
  1` and a fresh seq, **never re-submitted under its old key** — a recorded deviation from §4.4's
  "re-staged under the same key" for departs only (a refused burn would replay its refusal; the seq
  is the ticket's guard); bounded by `MAX_ORDER_ATTEMPTS` = 3.
- **L8 The queued `at=` is a due gate**; the plan is made now through the one `prepare_departure`;
  `+ship/undock` keeps `at=now` only.
- **L9 `daily HH:MM` resolves at the reach**, stored durably as `Order.reached {utc, venue}`.
- **L10 Holds**: waiting (self-clearing) = a depart's transient refusals and a lapsed lease; stopped
  (need `resume`) = `short=hold`, a consumable short of the depart's need, an unmeetable fill, a
  past window, **`NO_ROUTE`** (a deviation: doc 18 lists no such case; only an edit cures it), three
  refusals, three drops, a second unverified miss, a staff stand-down.
- **L11 Listed passengers** = aboard at filing, the pilot aside; one ashore at the launch is left
  behind and both are told.
- **L12 Notices**: Done, Held and Skipped lines are told; a depart's Done line is logged only;
  `DEPART_SET` once; deferrals, re-stages and retries never.
- **L13 `fill`** sized at each fill step from live pools for the first depart after it over the
  projected span.
- **L14 One author per document**; a new tenant's first edit replaces it (and no edit while another
  author's step is in flight).
- **L15 The order arm's agent is the hull**, offered only with no pending voyage; the depart's fate
  is learned by pull.
- **L16 Cursor** = the first unfinished order, checked. **L17 The reserve** at this venue's live
  asks, excluding the order's own good. **L18 Edits write no log line.** **L19 IC9 F2 closed.**
- **L20 The verify lens lives in `crates/tacsim/tests/orders.rs`**; `ic_slice_loop.rs` part 2 is
  IC12's. **L21** The Engineering/Bridge burn and reserve gauge lines go to IC12's content pass.
- **L22 Money renders without grouping** (`8000.00cr`) where doc 18 prints `8,000.00cr`. **L23**
  Every lens figure reproduced at lane v3 (36 t, 37 t, 19 crew-days, 73 t, 20 t "funds", 24 t "hold
  volume", 85 t, and §2 step 7's 42 t / 21 crew-days leaving tank 1 and locker 0); no doc correction
  owed.
- **L24 Holds over the inflight step** never discard the record; **L25** the lease gate precedes
  every act, re-stages included; **L26** refusals counted apart from drops (`refusals: u8`, no serde
  default); **L27** the signatory rendered, not stored; **L28** a stand-down recorded on the plan;
  **L29** fresh documents never reuse recent order ids.
- **Builder's own forks**, all O1-adjacent or conservative: a buy's done line pre-composed at
  staging into `Inflight.line`; `pause` over a hold drops the note and re-queues a held order;
  `move` that changes the cursor under a hold re-queues the named order; non-filing sub-verbs with
  no plan answer `NO_PLAN`; `ORDER_FIRES_NEXT` also requires docked and running; `Next due:` for an
  inflight cursor reads "order {n} is at the counter."; a depart's `on_fail` honoured as
  skip-or-hold for `HELD_WINDOW` and `NO_ROUTE`; `Partial` reads as skip for a sale with no bid,
  nothing aboard, a clamp of 0 or a house that cannot pay; a reserve that cannot be computed is 0
  (the depart then holds on its own); `HELD_UNCONFIRMED` over an unreadable depart ticket written
  once; the `HELD_CHARTER` drop written immediately; a deferred depart persists its reach alone.
- **Owner items, built on the brief's defaults, not decided here:** **O1** every new player-facing
  text (the verbs, renders, executor lines, `, signed {pilot}`, and the additions
  `CLEARED_INFLIGHT`, `RESUMED_EMPTY`, `ORDER_FINISHED`, `BOUGHT_SHORT`, `SALE_SHORT`,
  `HELD_FILL_PLAN`). **O2** a `daily HH:MM` reached after its `until=` window rolls to the next day;
  a fixed instant reached past `until=` holds. **O3** "listed passenger" = aboard at filing (doc 18
  §9 refuses a manifest beyond the tenant). **O4** which holds wait and which stop (L10's list) —
  and, under it, the literal reading taken by the brief and the code that an explicit `buy
  propellant N` holds under `HELD_SHORT_FOR_DEPART` whenever aboard + clamped < the passage's need,
  even unclamped and with a `fill` queued after it (doc 18's wording is "when the shortfall would
  fail the next depart"). **O5** `stop` is a barrier that marks itself done and pauses the plan
  until `+ship/order resume`; it never discards later orders. **O6** the new limits: `at=`/`+Nh` at
  most 7 days ahead, `until=` at most 24 h, at most 15 listed passengers, `min=`/`max=` at most
  1,000,000 major units.
- **Lead questions raised by this cut, not settled here:** the 60 s/60 s tie between
  `ORDER_RESTAGE_SECS` and the market sweep (a harmless replay re-stage after a reboot; closing it
  needs a mush-market read seam for the draw receipt or a later first re-stage); whether a queued
  depart should match its venue exactly (`def.routes.get(to)`) rather than through the live undock's
  prefix matcher.
- **Not in this cut** (per the brief): the story test part 2 and its promotion, `docs/help`
  (`orders`, `trade`, `ships`), the readiness gate's full form (IC12); the gauge lines and the
  C14/P05 departure boards (IC12); pruning the plans of ended charters (a waiting `HELD_CHARTER`
  plan is visited every fire; IC13); buyout, release, revoke, dormancy (IC13, which must refuse
  while an order is inflight); repeating plans, `via=`, customs, locality on every inventory path;
  any mush-market, mush-economy, mush-inventory, mush-sim or mush-core change. **The IC10 pre-deploy
  blocker stands** (inventory_applied retention, IC14's first item).

### Pins re-blessed

- **`crates/mush-contract-tests/legacy-handlers.toml`**, the `+ship` stanza: the note refreshed with
  the IC11 paragraph ("+ship/orders is a pure read; +ship/order writes ONE bounded whole-document
  tacsim:orders:<hull> sailing plan … and submits nothing; the plan is executed by the tacsim.port
  tick's order arm …") ahead of the IC9 and IC6 text. Category stays `authoritative` and
  `PINNED_BASELINE` does not move: `orders` and `order` are two new switches of the existing verb,
  not a new primary verb.
- **`crates/mush-contract-tests/state-slot-owners.toml`**: one new `[[site]]` for
  `crates/tacsim/tests/orders.rs` (`World::module_state_mut` / `AccountOracle` / owner `sim`), for
  the voyage.rs row's reason: every sailing-plan test charters its pilot through `+coop/enrol`,
  whose rule is per gateway account and fails closed on an unmapped character, so the rig installs
  the fixed account map. Tests that seed an inflight record do it through the folded store
  (`reboot_editing`), never `module_state_mut::<TacsimStore>()`, so no further row.
- **Unmoved, asserted by the gate:** `panic_budget/tacsim.txt` (no new `unwrap`/`expect`/`panic!` in
  non-test code; `port_error` stays the crate's one `.expect`), the restriction-lints census (no
  indexing, no `as`, only `clippy::too_many_lines`/`too_many_arguments` allows with a reason
  comment), every content digest (lane v3, Dray v2, goods v1, Stickney v2, Ceres v1 — no file under
  `crates/tacsim/content/` changed; test lane variants are built in the tests), the composition-edge
  pin, `module-ns-owners.toml`, the CI gated-test inventory (no `#[ignore]` added), the
  `families.rs` `StockIssueId` golden.
- **Existing assertions:** none changed or deleted. `tests/voyage.rs` only gains the F2 test;
  `AT_NOT_YET`'s one assertion asserts the constant and passes with the new wording; the `+ship`
  status lines are unchanged without a plan. Two of this cut's own new tests were rewritten in the
  repair round and are recorded here: `ic11_the_status_line_names_the_departure_hour` dropped
  `assert_eq!(other_row.hull, row.hull, "the freed Dray is leased again")` because IC6's
  `coop::hull_is_free` never frees a hull whose hold vault stands (until IC13's recall), and now
  checks the lapsed tenant's own `look` after the cancel and another author's plan re-signed onto a
  second hull; `ic11_a_reboot_mid_step_reruns_nothing` phase (c) now mirrors the landing
  (`drawn_at`) before the reboot, stepping the clock a second at a time so the sweep's fire is not
  the port tick's, with its "nothing re-staged" assertion unchanged, and the unmirrored case is
  pinned separately as phase (c').

### How it was verified

On Windows at the working tree over `7435f954`, the brief's §7 commands in order, then the workspace
sweep — all green on the first run of the second gate, no reruns:

```
cargo fmt --all -- --check                                        (no output; rc=0)
cargo clippy --workspace --all-targets -- -D warnings             Finished `dev` profile [unoptimized + debuginfo] target(s)
cargo test -p tacsim --lib                                        test result: ok. 916 passed; 0 failed; 0 ignored
cargo test -p tacsim --test orders                                test result: ok. 26 passed; 0 failed; 0 ignored
cargo test -p tacsim --test voyage --test sale --test ic_slice_loop --test passage
    voyage 14 passed; sale 15 passed; ic_slice_loop 1 passed; passage 4 passed; 0 failed
cargo test -p tacsim --test commerce --test coop --test ports --test port_tick --test hulls --test capacity
    commerce 20; coop 18; ports 17; port_tick 16; hulls 15; capacity 17 passed; 0 failed
cargo test -p tacsim --test lane --test stations --test content --test definitions --test composition_edges --test e7c1 --test e7r1
    lane 8; stations 13; content 29; definitions 25; composition_edges 2; e7c1 38; e7r1 21 passed; 0 failed
cargo test -p mush-contract-tests                                 29 result lines, all ok (legacy_handler_allowlist 4, state_slot_owners 4, panic_budget 7, restriction_lints 11, module_ns_owners 3, ci_gated_test_inventory 6, scale_harness 6 passed 8 ignored, …); 0 failed
cargo test -p mush-server --lib modules::                         test result: ok. 32 passed; 0 failed
cargo test -p mush-server --test boot_parity --test e7r2_proving_loop
    boot_parity 18 passed; e7r2_proving_loop 6 passed; 0 failed
cargo test -p mushd --test boot_composition                       test result: ok. 6 passed; 0 failed
CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast
    399 'test result:' lines, 399 ok, 0 FAILED; passed=7481 failed=0 ignored=92; 31 min
cargo test -p mush-bot                                            22 result lines ok, 0 failed (lib 469; arena_e2e 9; arena_lane 87; e2e 49; playtesting 9; 5 ignored)
```

The three lenses (correctness, conformance, tests) each read the whole diff without cargo; their one
shared must-fix (the missing `put_orders` refusal test) and the tests lens's three further
must-fixes (lens 2(c) unfalsifiable, the not-before reserve non-discriminating, `restage_draw` never
shown from the tick) were fixed and re-verified in the code by the judge:
`orders.rs::ic11_every_hold_shape_the_executor_writes_checks` (a hand-built `Ctx`, `Err`, `Absent`,
no key, no op, then one write), `tests/orders.rs` phase (c) counting every post-boot market
submission, `fill_and_clamp_plan_at_the_depart_instant` with the funds set to the larger reserve + k
tonnes − 1 minor, and `ic11_a_draw_lost_to_a_reboot_is_restaged_from_the_tick`.

The round-1 gate's two `orders` failures were test-side (the L14 leg expecting a re-charter IC6
refuses while the hold vault stands; phase (c) racing the market mirror at the 60 s/60 s tie) and
were repaired without loosening an assertion. One flake identified: `mushd
sim_solo::one_pilot_alone_starts_a_match_and_fights_the_machine` failed once under load in the first
gate (the sim_* TUI timing family common-ic.md lists as load-sensitive), passed alone twice (15.15 s
/ 14.90 s) and in the second gate. The `panicked at crates\mush-server\src\process_fault.rs:197:9`
line in the sweep log is `tests::a_panicked_world_task_is_a_failure`'s intended panic.

Every market closed call — `quote` + `accept_at(.., Ship)`, `sell_into_bid`, `restage_deliver`,
`restage_draw` — ran from the tick inside a tacsim-owned context without a module-scope refusal,
with its effect asserted (contracts, quotes, `drawn_at`, settlement, pools, the bid decrement), so
the brief's F14 stop never triggered.

### Surface for the next cuts

**Re-exported from `tacsim::port`** (verified in `crates/tacsim/src/port/mod.rs`)

- From `orders`: `Orders`, `PlanState`, `HoldNote`, `Order`, `Reached`, `OrderState`, `What`, `Qty`,
  `OnShort`, `Inflight`, `InflightRefs`, `LogLine`, `OrdersState`, `orders_key`, `orders_key_owner`,
  `orders_state(w, hull)`, `ORDERS_PREFIX`, `ORDERS_SCHEMA` (1), `MAX_ORDERS` (16),
  `MAX_ORDER_LOG_LINES` (32), `MAX_ORDER_LOG_BYTES` (240), `ORDERS_SHOWN_LOG` (8), `MAX_DEPART_CREW`
  (15), `MAX_PRICE_MAJOR` (1_000_000).
- From `executor`: `Ask`, `Bid`, `Bound` (+ `word()`), `FillError`, `FillTargets`, `ask_at`,
  `bid_at`, `clamp_buy`, `clamp_sell`, `funds_units`, `join_bounds`, `project_cargo(goods, readings,
  &[&What]) -> Result<(BTreeMap<CanonicalId,u64>, u64), CoreError>` (no `aboard` parameter: the
  readings carry it), `fill_targets(goods, readings, cargo, extra_kg, tank, locker, occupants,
  allowance_kg, plan_fn)`, `reserve_for(targets, aboard_tank, aboard_locker, ask_tank, ask_locker,
  excluding)`, `room_for`, `FILL_ITERATIONS` (6), `ORDER_RESTAGE_SECS` (60),
  `ORDER_SETTLE_GRACE_SECS` (160), `ORDER_RETRY_SECS` (60), `MAX_ORDER_ATTEMPTS` (3),
  `MAX_ORDER_REFUSALS` (3), `MAX_ORDER_VERIFY_PER_FIRE` (64).
- From `when`: `NotBefore::{Now, Utc(i64), UtcDaily(String)}`, `MAX_AHEAD_SECS` (7 days),
  `MAX_WINDOW_SECS` (24 h).

**Not re-exported but `pub` at their module paths**

- `tacsim::port::executor::CoreError` and every executor text (`BOUGHT`, `BOUGHT_SHORT`,
  `BOUGHT_NONE`, `BOUGHT_FILL_TANK`, `BOUGHT_FILL_LOCKER`, `FILL_ENOUGH`, `SALE_SHORT`,
  `DEPART_SET`, `STOPPED`, `SKIP_LINE`, `HELD_LINE`, `WAITING_LINE`, `NO_BID`, `NOTHING_ABOARD`,
  `INTAKE_TAKEN`, `HOUSE_CANNOT_PAY_SKIP`, `UNDER_MIN`, `SHORT_CLAUSE`, `HELD_SHORT_FOR_DEPART`,
  `HELD_NO_DEPART`, `HELD_FILL`, `HELD_FILL_PLAN`, `HELD_ASHORE`, `HELD_WINDOW`, `HELD_CHARTER`,
  `HELD_UNCONFIRMED`, `HELD_STOOD_DOWN`, `NOT_ADMITTED`, `AWAITING_REFUND`, `REFUSED_RETRY`,
  `LEFT_BEHIND_PILOT`, `LEFT_BEHIND_PASSENGER`, `HOLD_NARRATION`, `SKIP_NARRATION`).
- `tacsim::port::orders::*` texts (`ORDER_USAGE`, `NOT_ORDERABLE`, `ORDERS_WHERE`, `ORDERS_DARK`,
  `NO_PLAN`, `ORDERS_FULL`, `ORDER_INFLIGHT`, `AHEAD_OF_INFLIGHT`, `NO_SUCH_ORDER`, `UNKNOWN_GOOD`,
  `FILL_ONLY_CONSUMABLE`, `UNKNOWN_PORT`, `ORDER_FILED`, `ORDER_FIRES_NEXT`, `ORDER_AFTER_DEPART`,
  `DELETED`, `MOVED`, `SKIPPED`, `ORDER_FINISHED`, `CLEARED`, `CLEARED_INFLIGHT`, `PAUSED`,
  `RESUMED`, `RESUMED_EMPTY`, `NOT_HELD`); `tacsim::port::when::{BAD_TIME, BAD_UNTIL}`;
  `tacsim::port::ship::{STATUS_DOCKED_DEPARTS, SHIP_DOCKED_DEPARTS}`; `Orders::{check,
  first_unfinished, order, position}` are `pub`.

**Store (`TacsimStore`)**

- `orders_state(hull) -> OrdersState<'_>`, `orders_docs()`, `orders_unreadable()` (`pub`);
  `put_orders(ctx, &Orders) -> Result<(), String>` (`pub(crate)`, runs `check` first).

**`pub(crate)` inside tacsim**

- `orders::{fresh, derive_cursor, push_log, order_mut, clear_hold_naming, stop_at, wait_at,
  clamp_line, seed_next_id, order_summary, render_reach, depart_due, departs_at, orders_cmd,
  order_cmd}`.
- `executor::{Planner, planner, DepartTargets, TargetError, next_depart_targets, reserve_at,
  no_route_line, plan_queue, fill_reason, market_line, order_agents, order_verify, order_step,
  stand_down}`.
- `voyage::{prepare_departure(ctx, pilot, hull, dest_word, Queued) -> Result<DepartFiling,
  DepartRefusal>, DepartFiling{voyage, port, key, seq, burn}, Queued::{Live, Order{order, attempt}},
  DepartRefusalKind (15 kinds), DepartRefusal{kind, line} with From<DepartRefusal> for CmdError,
  souls_aboard, lane_ports, venue_name, ticket_at, launched}`.
- `market::port_rooms`; `when::{parse_at, parse_until, hhmm_minutes, resolve_due, ic_at_utc,
  plan_instant, render_due}`.

**Tick, verbs and keys**

- Tick: `Arm::Order { hull }` ahead of `Probe`; `PortTickState::{orders_warned, depart_dark_warned,
  order_verify_after}` (RAM); `order_verify` after `launch_prune`, `order_agents` offered after the
  launch agents.
- Verbs: `+ship/orders`, `+ship/order …` as two new switches on the Authoritative `+ship` spec.
- Keys: `tacsim:orders:{hull.0}`; a queued depart's burn `tacsim.port.burn/{hull}/{order}/{attempt}`
  (the live undock keeps `…/undock/{seq}`); a market leg's audit key `market.lot.draw.{contract:x}`
  / `market.lot.deliver.{contract:x}`.

**Launch-arm behaviour later cuts rely on:** a pending with `order: Some` lapses silently (no
`CLEARANCE_LAPSED`); a ticket disagreeing with the pending holds the launch; `clear_cmd` writes the
stopped `HELD_STOOD_DOWN` hold and bumps `attempt` when the plan names the pending in flight.

**For IC13:** a buyout, release or recall must refuse while `Orders.inflight` is `Some`, and the
lapsed-author plan (waiting `HELD_CHARTER`, visited every fire) is what pruning removes;
`departs_at` already hides a lapsed author's hour.

## IC12

**The story test, help, readiness and the docs pass** (doc 18 §5, the IC12 row and paragraph; §6;
§4.7; §10), built 2026-10-17 on main at 26e3b1fa. The story test moves onto the production root,
`+coop/enrol` is gated on every readiness line but the sky, the five help topics and the amendments
are written, the station packs are re-published as Ceres v2 and Stickney v3, and doc 18 becomes
Draft 0.4 "as built" through IC12. The hand-played two-session smoke is owed to the owner (no Redis
on the build machine). One commit, pushed to origin/main.

### What landed

1. **The story test on the production root** (`crates/mush-server/tests/ic_slice_loop.rs`, new,
   3,969 lines; `crates/tacsim/tests/ic_slice_loop.rs` deleted; the vertex included by
   `#[path = "../../tacsim/tests/support/vertex.rs"]`). Five `#[tokio::test]`s, each booting
   `mush_server::boot::build_engine` over a fresh `InMemoryStore` with `[game].ic_ratio = 24` and
   `[tacsim.time.ephemeris].rate = 0` set explicitly, asserting the timeline's generation at 24 and
   `!TacsimStore::test_world()`. `land` (`:342-409`) is the world task's path — `flush`
   (`persist::batch_from_dirty` + `ObjectStore::write`), `ActionCoordinator::command_ingress` in the
   rig's epoch, `preflight` (Retired raises the epoch and retries ≤ 3; Pending/Finalized is a
   counted `Landing::Replay`; Quarantined/Conflict panics), the actor's own `legal_action_query` →
   `project_legal_actions` → `projected_for_action` → `resolve_legal_action` →
   `prepare_and_stage_resolved`, then take, reserve, reconcile (`matches_bundle`),
   `install_prepared_action_announcing`, checkpoint, and `assert_holds_within_limits` after every
   landing. `restart` (`:230-255`) flushes, drops the engine with whatever it still queued, re-boots
   `build_engine` over the kept store and spool, bumps the epoch, restores the clock, and asserts
   every pilot's account through `engine.account_of` and a typed probe read back through the store.
   No `module_state_mut`, no `reboot_into`, no hand-hydrate.
2. **Part 1** (`ic12_part_one_a_leg_by_hand`): the sky line in §6's form with no duration;
   `help ports|ships|coop|orders|trade` answered by the booted registry; every §2 word walked and
   asserted against the room re-derived from the port map; the vertex buys and the planner's own
   fuel and stores; the `+ship/hold` render; enrolment at 24, then `@tacsim/time rate=3600` as the
   one staff act; the Bridge `STATUS_DOCKED` line; IC9 T7 as `PortFireStats.deferred ≥ 1` with the
   burn left in the engine across the fire, the queue then exactly `tacsim.port.burn/<hull>/undock/1`,
   no journey, no `CLEARANCE_LAPSED` (the constant), `STATUS_CLEARED`; the same-boot HIT (one
   ticket, one debit); the Kirkwood line absent one fire before the frozen boundary and present at
   it, the docking likewise at the ticket's total; the two `SALE_LINE`s exactly; journals (pilot +
   Trust = Kasei's payment), the statement, seeded − sold on every listing and pool, lineage to a
   seeded receipt, no pages.
3. **Part 2** (`ic12_part_two_the_queue_runs_the_return`): §2 step 10's queue with `at=+10m`, the
   pilot disconnected. **Restart (a)** after the depart's burn lands and before its launch: the kept
   burn landed again in the new epoch before any fire is `Landing::Refused` with "that passage is
   already burned" (the ticket's `seq`), one ticket, the tank unchanged, then one launch and no burn
   key on four fires. **Restart (b)** with the first sale's deliver leg lost: re-staged once under
   `market.lot.deliver.<contract>`, every other market key proven to be order 3's sale, one settled
   contract, one bid decrement, Kasei +20 once, `sales` 1 then 2. The Stickney buys are asserted
   line by line from `stickney_buys` (`:2330-2517`: `project_cargo` + `fill_targets` over the hold as
   it docked, with `choose_route` over the world's sky; the fittings at 49 t unclamped; the alloys'
   affordable tonnage computed and shown to be below 55; the room and the stock) —
   `BOUGHT_FILL_TANK`, `BOUGHT_FILL_LOCKER`, `BOUGHT`, `BOUGHT_SHORT` "short of 55 t: funds"
   exactly; orders 4–7 Done. The return docked at its frozen instant; the reconnect heading once and
   every notice once in queue order; `, signed Wren:`; conservation over Wren, the Trust and every
   port org and over every touched listing, pool and both legs' burns. **Restart (c)** on the kept
   world: four fires with every queued submission landed and the queue asserted empty, no receipt
   minted, no listing moved, no stake re-posted, no obligation added, no object made; `boots == 4`.
4. **The rate change** (`ic12_a_rate_change_mid_passage_relabels_the_arrival`): at 1800 mid-passage
   `leg_times()` equals the ticket's frozen legs before and after, `arrival_at()` unchanged, the
   status-line and `+ship` ETA re-derived with the time left doubled (±1 s), the daily depart's IC
   label re-derived through `plan_instant` and the old label gone, the hull docked at the frozen
   instant.
5. **The negative checks** (`ic12_negative_checks_refuse_in_fiction_and_move_nothing`): fifteen
   checks, each against a shipped constant and a `Still` snapshot (posted and available, pools,
   location, every listing quantity, quote and contract counts, tickets) — `ENROLLED_ONCE`;
   `NOT_OPEN` after the rate switch, with the flagged Rate line on the staff view and `enrol_gate`
   naming `Binding::Rate`; `NO_CHARTER`; "compartment `berths` is full (2 max)"; `NOT_IN_PORT` and
   `HOLD_ELSEWHERE`; `capacity_line` for 86 t, 19 + 81, 101 t and ice into a full hold with an empty
   tank; `NOT_ABOARD`, `TANK_SHORT` at 39 against 40, `UNDOCK_UNDER_WAY` twice; `HELD_ASHORE` with
   nothing burned; `HOUSE_CANNOT_PAY_SKIP` (never `UNFUNDED`) with the house drained to 100.00cr,
   then `UNFUNDED` typed; the boundary line for `o`/`s`/`home` and "Permission denied." for
   `@teleport`; `left_at_line` after `o` → `x` → `c`; `SELL_SHORT`; the pilot's `@journey/depart`
   "Permission denied" with staff reading `LANE_UNPLANNED`; IC9 T6 (non-staff clear refused, staff
   clear stands a held launch down, `+ship/leave` lands in the gallery); `@inv/move` out of the hold
   refused identically docked and under way — the endpoint rule, named as such.
6. **The sailing plan's skips and holds** (`ic12_the_sailing_plan_skips_and_holds`): on the
   production root `NO_BID`, `HELD_SHORT_FOR_DEPART` (the need cross-checked against the fill within
   one tonne), `HELD_NO_DEPART`, `FILL_ENOUGH`, three dropped burns → `HELD_UNCONFIRMED` with ≥ 3
   `NOT_ADMITTED` and no ticket, `UNDER_MIN`, `AWAITING_REFUND` after `DELIVER_TOO_LATE` and after
   `DRAW_TOO_LATE` (the cutoff from `DRAW_WINDOW_SECS − DRAW_SETTLE_MARGIN_SECS`; the escrow shown as
   a hold on available funds, the contract `Expired`, posted and available restored exactly),
   `NOTHING_ABOARD`.
7. **`TestWorld::from_engine`** (`crates/mush-testkit/src/lib.rs:151-161`): wraps a booted engine
   and connects GOD as `with_modules` does; unit test
   `ic12_from_engine_wraps_a_booted_engine_and_connects_god`.
8. **The gate over every line but the sky** (`crates/tacsim/src/port/coop.rs:526-576`):
   `pub const fn gated(Binding) -> bool`, an exhaustive match with `Binding::Sky => false` citing
   D-18-50; `enrol_gate` refuses on the first gated non-Ready line; both re-exported from
   `tacsim::port` (`port/mod.rs:115`). Treasury and Bids now gate. Unit test
   `ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky` (15 bindings ×
   Pending/Missing/Flagged; the summary still counts the sky).
9. **The moving-sky promise** (`crates/tacsim/src/port/readiness.rs:316-329, 816-827`;
   `crates/tacsim/src/port_tick.rs:361-363, 452-481, 554`): `body_without_epoch(pack, ephemeris)`
   (None at rate 0, else the first lane body without `epoch_ic`); the Sky line Flagged with
   "; {body} has no epoch_ic, so it moves from the server's epoch."; `PortTickState.sky_paged`,
   `pub fn sky_paged(w)`, and `page_moving_sky` raising one `tracing::error!` with a runbook field
   per boot. Tests `ic12_a_moving_sky_with_a_body_lacking_epoch_ic_flags_and_pages_once` and
   `ic12_a_constant_sky_never_flags_a_body_lacking_epoch_ic` (`crates/tacsim/tests/ports.rs`).
10. **`sale_none_reason`** (`crates/tacsim/src/port/executor.rs:2855-2887`), a behaviour-preserving
    extraction of the sale's four skip reasons, with unit tests
    `ic12_a_sale_that_takes_nothing_names_its_reason`, `ic12_fill_reason_renders_each_hold`,
    `ic12_fill_refuses_a_locker_too_small` and `ic12_a_refused_call_logs_once_and_holds_at_three`;
    the resume reset in `crates/tacsim/tests/orders.rs::ic12_resume_clears_the_refusal_count`.
11. **IC9 T8 and T10**:
    `crates/tacsim/tests/voyage.rs::ic12_t8_goods_pending_aboard_releases_a_terminal_contract_and_holds_for_a_lapsed_listing`;
    `crates/mush-atlas/tests/anchored_exits.rs::ic12_t10_a_one_way_exit_out_of_a_room_aboard_applies`.
12. **`+ic`'s refusals** (`crates/tacsim/tests/stations.rs`): `ic12_ic_refuses_a_non_character` (a
    Thing run as the actor; there is no puppet `@force`), `ic12_ic_refuses_a_world_without_a_corridor`
    (the starter and game packs only), `ic12_the_ic_door_has_no_staff_exemption` (a WIZARD stamped at
    Stickney refused at `c`).
13. **The station packs re-published**: `neighborhood.ceres_freeport` v1 → **v2** (digest
    `641e1a39…4d8538`; adds C09 `hull registry board` and C14 `sailing plan forms`; strikes the verb
    meta-text from C04 `map`, C07 `price board`, C08 `loading schedules` and C08 `chit board`) and
    `neighborhood.stickney_yards` v2 → **v3** (`df2e5af1…b2eb`; adds P05 `sailing plan pad`, P11
    `yard time board` and `insurers mark`); pins in `stations.rs` `PINNED`/`station_version` and
    `boot_parity.rs`'s closure; tests `ic12_the_five_details_ship_under_plain_keys` (no painted price
    but the Dray's 120,000cr, no `LookRendered`, no `+` before a letter in any Ceres detail) and
    `ic12_a_world_latched_at_the_old_station_versions_takes_the_new_through_plan_and_apply` (v1 and
    v2 rebuilt to their old pinned digests, a kept world never upgraded at boot, taken through
    `@atlas/pack plan` then `apply`, no object made).
14. **The validator bounds** (`crates/mush-atlas/src/validate.rs:2065-2066, 2240-2294`):
    `budget.text` errors for `short_desc` over 256 bytes ("one line"), ambience over 24 lines, a
    blank ambience line, a line over 512 bytes; `topology.unreachable`'s hint names `port.dock`. Five
    tests in `crates/mush-atlas/tests/validate.rs` with boundary passes; the starter digests unmoved.
15. **Help and in-code help**: new `docs/help/{ports,ships,coop,orders,trade}.md`; amended
    `getting-started`, `chargen`, `travel`, `spatial`, `cockpit`, `npc`, `atlas`, `faction`,
    `chronicle`, `clocks` (`intel` checked, nothing stale); `SHIP_HELP` (`port/ship.rs:75-76`) and
    `ORDER_USAGE` (`port/orders.rs:715`) name `+Nh|+Nm`; `@journey`
    (`mush-sim/src/commands.rs:3591-3596`) states the real gate and that a lane route or Co-op hull
    is flown only by `+ship/undock` or the sailing plan; `@agent` (`mush-npc/src/module.rs:211-220`)
    lists `interfaces` and the one-sheet `/sheet` sentence; `+market/depth <catalogue>:<code>` and
    `/accept`'s re-stage promise made true (`mush-market/src/commands.rs:67-71, 83-87`).
    `docs/ops.md` gains "The trade run (doc 18): the port tick, readiness and the play rate"
    (executor posture, asker-authority bridge, the status report and gate, the moving-sky page,
    `@tacsim/port clear`, the rate-24 deploy line, kept-world pack upgrade, the pre-deploy blocker).
    The §10 stale comments fixed in `mush-market/tests/ic0.rs`, `e7r2.rs`, `mush-sim/src/custody.rs`,
    `tacsim/src/ephemeris.rs`; the rest found already true.
16. **Doc 18 Draft 0.4** ("as built" through IC12; +850/−0 words: zero word-level deletions, the P-A
    brief byte-identical): the status paragraph, the §1–§4.8 "As built (ICn)" notes, §3's banner and
    beat list, §4.8's calendar corrected to `mush_core::CivilDate`, §5's rows marked landed with
    their hashes and the by-row list with the unowned presentation residuals, §6's as-built record of
    every test and restart, §7's annotations (D-18-3, 11/46, 12, 18, 23, 24, 25, 26, 27, 29, 30, 31,
    32, 34, 36, 41, 47, 50), §8's Q-18-32..41 status and Q-18-42..65, §9 risk 3/11/14 notes and the
    new risk 19, §10's done/owed marks. 18a revised with 19 "As built" notes; the README rows
    rewritten. The builder left this file untouched; the committer appended this section.
17. **Readiness helpers made stricter** in `tacsim/tests/{commerce,coop,voyage,orders,sale}.rs`
    (every bracketed line but `sky:` must read `[ready]`); the state-slot row for the deleted file
    removed from `crates/mush-contract-tests/state-slot-owners.toml`.

### Deferred, and why

- **The hand-played two-session smoke** (the §5 IC12 row's lens). Not done and not claimed: the
  build machine has no Redis (`Test-NetConnection 127.0.0.1 -Port 6379` → `TcpTestSucceeded
  False`, 2026-10-17). The story test's rate-24 set-up half stands in for the smoke's set-up half;
  the real smoke — two sessions at rate 24 on local `mushd`, or a dev1 session after an
  owner-approved redeploy — is owed to the owner (O5).
- **Lead ruling L1 / owner item O6 — the story test's rate.** §6's "rate 3600 the only staff act,
  none after boot" cannot hold on the production composition: the Rate line refuses enrolment at
  any rate but 24 on a non-test world, and threading `test_world` into `build_engine` is forbidden
  (IC3 L15). The test boots at `[game].ic_ratio = 24`, enrols at 24 and sets 3600 as its one staff
  act after the enrolments; §4.7's "at rate 3600 in a test world the gate passes" is noted
  superseded for the story test; IC3's deferral of the threading is closed as not needed. Recorded
  as Q-18-61.
- **Lead ruling L2 — landing and restarts as production does them.** Through the coordinator
  exactly as `CommandSubmitDriver::drive_one`, restarts as plain `build_engine` re-boots over the
  kept store with the epoch bumped (production draws it at random per process, so the cross-restart
  replay guard is the ticket's `seq`, not the coordinator's HIT). The named fallback (request-keyed
  landing + hand-hydrate) was not needed.
- **Lead ruling L5 / owner item O2 — the gate is every line but the sky.** Treasury and Bids join
  the gate (IC8 L8's "reported, not gated" was a lead deferral to this cut); the sky stays reported
  and paged, never gated (D-18-50). **Owner-critical, Q-18-60:** the Bids line also reads Pending
  while a house cannot fund one unit of its bid, so ordinary play can close enrolment world-wide —
  on lane v3 Kasei's 600,000cr against 3,040,000cr of intakes drains by about 11,000cr per
  story-shaped round trip, so on the order of fifty trips, sooner while sale escrows are held; from
  then every new player hears `NOT_OPEN` and the gate logs at `info`, never paged. Before IC12
  enrolment stayed open and sales skipped with `HOUSE_CANNOT_PAY_SKIP`. Alternatives recorded (gate
  Bids only when Missing; or page at `error` once a boot when a funds-Pending bid closes it). Not
  built either way: it changes O2, the owner's.
- **Lead ruling L13 — the sky page lives on the tick**, latched in `PortTickState.sky_paged`;
  `readiness` stays a pure read. The test asserts the latch, not a page count (the brief accepted
  "the log itself is not asserted").
- **Lead rulings L3/L4 — one file, several tests, each its own engine**; the old tacsim test
  deleted, not kept beside; T8 in `voyage.rs`, T10 in `anchored_exits.rs`, the `+ic` refusals in
  `stations.rs`. `ALREADY_IN` has no test (doc 18 §4.5's as-built note overstates this by one
  clause).
- **Lead rulings L7/L8 / owner item O3 — detail keys and painted prices.** Seven detail keys ship
  respelled under `[A-Za-z0-9_ ]` (five at the stations, two aboard in IC8); the validator was not
  widened. Live market figures and verb meta-text are struck from the Ceres details; the Dray's
  `120,000cr`, insured values and the C14 departures board's "two to three weeks in a good season"
  stay as fiction (Q-18-59). Stickney's P04 `shift boards` (`+atlas`), P07 `intake board` and P08
  `price board` (`+market/list`) still name verbs — an unowned residual, named in 18a §0.2.
- **Lead ruling L9 — help-only for `@agent`**: the code default `"mortal"` is left for IC14.
- **Lead ruling L10 / owner item O4 — the presentation residuals** (the Engineering/Bridge
  burn-and-reserve gauge lines, the C14/P05 departure boards and C01/P01 berth boards as
  `LookRendered` lines, P07's funded-balance figure, the accept line naming the signatory) are not
  built; the §5 IC12 row does not name them; recorded as unowned in doc 18 §5 and Q-18-47 for the
  owner to place (IC14 or a content cut).
- **Lead ruling L12 — `+600s` → `+10m`**: the shipped grammar has no seconds form; §6 is noted as
  built, not changed.
- **Lead rulings L11/L14 — the docs pass split in four, history never rewritten, the cut notes read
  by grep; comment-only edits in `mush-market`, `mush-sim` and `tacsim`.** The `+market` price lines
  resolved as `/depth <catalogue>:<code>` (a port good reads `tacsim:<good>`).
- **Owner item O1 — every player-facing text** (the five help topics, the amendments, the `+Nm`
  grammar, the `@journey`/`@agent`/`+market` lines, the Sky flag text, the ops section) is the
  builder's wording in the house voice; the owner may revise. `ships.md`'s "nothing crosses the hull
  under way" is player-side true only (below).
- **Q-18-62 (new, owner):** a staff `@inv/move` out of a hold under way lands — the under-way
  quantity seal on every inventory path is D-18-37's deferral to P-H; the pilot's own move is
  refused by the endpoint rule (one endpoint), docked and under way alike, which is what the test
  now pins. §6's check 10 as written stays open there.
- **IC11 paths covered only at the unit level**: `REFUSED_RETRY`, `INTAKE_TAKEN`,
  `HELD_FILL`/`HELD_FILL_PLAN` (no step on the shipped packs reaches `refused_call` or exhausts a
  4,000 t intake; the brief forbids fixtures that fake a refusal); the resume reset through
  `orders.rs`'s `reboot_editing` idiom.
- **Declared deviations kept as built**: the plan burn at the test's load is 40 t (the doc's 41 is
  the burn of a 42 t tank); the timings are asserted from the frozen ticket, never literals; the
  pilot's raw `@journey/depart` and a non-staff `@tacsim/port clear` hear "Permission denied." (the
  permission gate runs first); the Draw cutoff is `DRAW_WINDOW_SECS − DRAW_SETTLE_MARGIN_SECS` =
  3,000 s; `HELD_SHORT_FOR_DEPART`'s need projects the order's own tonne (Q-18-51, IC14).
- **Pre-deploy blocker** (doc 18 §9 risk 19): `MAX_APPLIED_MARKERS` (4,096) has no retention;
  selling must not be enabled on a long-lived world before IC14's first item lands.
- **Still owed outside this cut's files**: the §10 amendments to docs 06, 08a, 10 (T1, C2A–C4A),
  10b, 11, 14/14b, 15, 16c and `mushd.toml.example`. The `18-cut-notes.md:109-110` re-point owed to
  the committer is made by the note at the end of this section; IC0's lines stay as written.
- **Two small inaccuracies left standing**: `crates/tacsim/src/ephemeris.rs:9` is still a
  148-character `//!` line (the fixer reported it rewrapped; fmt and clippy are green); doc 18
  §4.5's "`ALREADY_IN` … tested since IC12" (it is not).

### Pins re-blessed

- **`crates/tacsim/tests/stations.rs` `PINNED`**: `neighborhood.ceres_freeport` `383e0b13…1dc1cf` →
  `641e1a3983447cca1a05bed6a839f98fe68c2afa683e0deee0c5e4e2904d8538` and
  `neighborhood.stickney_yards` `39aab74e…42d42` →
  `df2e5af11f18c4a5c71bea9e0b5b187b7cdb97c037aecc6b433b9ee3a0d1b2eb`; `station_version` Ceres 2,
  Stickney 3. Reason: the packs are new published versions (the five 18a details under plain keys;
  Ceres' verb meta-text struck, including the C08 chit board after the fixer). Both digests
  re-derived by the signing helper, never hand-edited; the old versions are rebuilt to their old
  pinned digests by `old_station_json`, which pins that a kept world latched at v1/v2 holds exactly
  those bytes.
- **`crates/mush-server/tests/boot_parity.rs`**, the `station_version` closure and comment: Ceres 2,
  Stickney 3, the regions 1. Same reason.
- **`crates/mush-contract-tests/state-slot-owners.toml`**: the `crates/tacsim/tests/ic_slice_loop.rs`
  row removed. Reason: the file is gone and the new story test mutates no foreign slot (the account
  oracle is linked through `Engine::link_account_character`, as the world task does).
  `state_slot_owners` passes (4 tests).
- **`tacsim` lib**: `ic6_the_gate_reads_only_its_bindings` replaced by
  `ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky`. Reason: the gate's rule changed
  (every line but the sky); the new test is a superset (15 bindings × 3 states).
- **The readiness helpers in `tacsim/tests/{commerce,coop,voyage,orders,sale}.rs`**: wait on every
  bracketed line but `sky:` instead of the vendor/ask/facts subset. Reason: the gate as now defined;
  strictly stronger. `ic6_the_desk_closes_at_a_play_world_rate_other_than_24` sets the rate to 24
  before stocking (a real world reads Ready only at 24); its assertion is unchanged.
  `ic11_an_unfunded_bid_skips` drains Kasei after stocking to 100.00cr (under one tonne, not zero)
  instead of authoring a 100-major lane variant; its assertions are unchanged.
- **Not moved** (checked): the panic budgets, the restriction-lints census, `legacy-handlers.toml`
  (no new verb or switch), `module-ns-owners.toml`, the composition-edge pin, the lane v3 / Dray v2 /
  goods v1 / region digests, the starter digests, the `StockIssueId` golden,
  `help-commands-allowlist.toml` (no new row; T-45 green), the CI gated-test inventory (nothing
  `#[ignore]`d).

### How it was verified

On Windows at the working tree over `26e3b1fa`, the final gate (gate1, 21 steps, logs under the
run's `scratchpad/cuts/IC12/gate1/`) — every step a first-run pass, no rerun, no LNK1104, no hung
binary:

```
cargo fmt --all -- --check                                        (no output; rc=0)
cargo clippy --workspace --all-targets -- -D warnings             Finished `dev` profile … in 0.78s; rc=0
cargo test -p mush-testkit                                        lib 14; acceptance 9; ledger_boundary 3; vehicle_console 7; doc-tests 2 passed, 1 ignored (pre-existing)
cargo test -p mush-server --test ic_slice_loop                    test result: ok. 5 passed; 0 failed; finished in 2.96s
cargo test -p mush-server --test boot_parity                      test result: ok. 18 passed; finished in 2.35s
cargo test -p tacsim --lib                                        test result: ok. 920 passed; finished in 4.37s
cargo test -p tacsim --test ports --test coop --test commerce --test voyage --test sale --test orders
    commerce 20; coop 18; orders 27; ports 19; sale 15; voyage 15; all ok
cargo test -p tacsim --test stations --test content --test lane --test hulls --test port_tick --test capacity --test passage
    capacity 17; content 29; hulls 15; lane 8; passage 4; port_tick 16; stations 18; all ok
cargo test -p mush-atlas --lib                                    test result: ok. 51 passed
cargo test -p mush-atlas --test validate --test anchored_exits --test starter
    validate 14; anchored_exits 3; starter 5; all ok
cargo test -p mush-market --lib                                   test result: ok. 27 passed
cargo test -p mush-market --test ic0 --test e7r2                  ic0 5; e7r2 21; all ok
cargo test -p mush-npc --lib                                      test result: ok. 73 passed
cargo test -p mush-sim --lib                                      test result: ok. 37 passed
cargo test -p mush-contract-tests   (twice)                       29 result sets all ok, 0 failed (lib 31, ci_gated_test_inventory 6, help_commands 4 (T-45), legacy_handler_allowlist 4, module_ns_owners 3, panic_budget 7, restriction_lints 11, scale_harness 6 passed 8 ignored (pre-existing), state_slot_owners 4, hex_vocabulary 2)
cargo test -p mushd --test boot_composition                       test result: ok. 6 passed; finished in 25.22s
CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast
    rc=0; 42 min; 399 'test result:' lines, all ok; passed=7505 failed=0 ignored=92
cargo test -p mush-bot                                            rc=0; 22 result sets all ok; 704 passed, 0 failed, 5 ignored (arena_e2e 9 in 203 s, playtesting 9 in 129 s: long, not hung)
```

The load-sensitive suites passed inside the saturated sweep (ipc_authoritative_action 8,
two_process_e2e 4, scripting_e2e 1, world_fault 2, module_api 77, arena_story 1 in 90.56 s,
arena_story_split 1 in 53.61 s). The `process_fault.rs:197` panic in the log is the gate5 fault
matrix's deliberate injection (its binary ok).

Fixer re-checks after the fix pass: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test stations
ic1_the_station_packs_load_and_their_digests_are_pinned` → ok (Ceres v2 re-signed, only its digest
moved); stations 18 ok; `ic_slice_loop` 5 ok in 3.03 s with boot_parity 18 ok.

The three lenses: correctness (8 findings: F1–F5 fixed, F6 deferred to the owner as Q-18-60, F7/F8
notes — F7's stale `readiness.rs:71/735` comments and the `executor.rs` doc placement were
addressed by the fixer's F5 fix for the doc line; the Bids comments stand as a note), conformance
(9: F1–F5 fixed, F6–F9 notes), tests (14: T1 and T2 must-fix fixed, T3–T8 fixed, T9–T14 notes).
The judge read the whole diff and every new file; confirmed the `Step::Deferred` path at
`voyage.rs:1761` → `port_tick.rs:671`; confirmed `sale_none_reason` byte-equivalent to the base arm;
found doc 18's word-level deletions zero (`git diff --word-diff=porcelain`) and the P-A brief region
byte-identical to base lines 1876–1960; `help-commands-allowlist.toml` untouched; `ALREADY_IN`
found untested; `ephemeris.rs:9` found unwrapped. No flakes identified. The Redis check verbatim:
`ComputerName 127.0.0.1, RemotePort 6379, TcpTestSucceeded False` — the smoke owed, not claimed.

### Surface for the next cuts

Public items later cuts (IC13, IC14, P-A) build on, verified against the diff:

- **`mush_testkit::TestWorld::from_engine(engine: Engine) -> TestWorld`**
  (`crates/mush-testkit/src/lib.rs:151`): the way a test wraps the production root booted by
  `mush_server::boot::build_engine`.
- **The story-test rig** in `crates/mush-server/tests/ic_slice_loop.rs` (file-local, the pattern
  IC13's buyout test extends): `boot(tag)`, `config(tag)` (memory store, scratch data dir,
  `help_dir` = `docs/help`, `tacsim.enabled`, `ic_ratio = 24`, `ephemeris.rate = 0`), `flush`,
  `land -> Landing::{Landed, Replay, Refused(String)}` (the coordinator path),
  `drain`/`land_all`/`land_quietly`/`fire`/`fire_quietly`/`sweep`/`to_ic`/`to_ic_queued`,
  `restart(rig, probe)`, `pilot`/`alt` (store accounts + `link_account_character`),
  `enrol`/`outfit`/`board_from_chandlery`/`set_rate`, `assert_holds_within_limits`,
  `assert_no_pages`, `assert_sky_line`, `assert_help`, `still`/`refused`/`accept_refused`/
  `inv_move_out`, `stickney_buys`, `fire_until`, `disconnect`. The vertex helper is shared by
  `#[path = "../../tacsim/tests/support/vertex.rs"]`.
- **`tacsim::port::gated(Binding) -> bool`** (const, exhaustive; `Binding::Sky => false`) and
  **`tacsim::port::enrol_gate(&Readiness) -> Result<(), &ReadinessLine>`**, both re-exported from
  `crates/tacsim/src/port/mod.rs:115`; a new `Binding` variant must be placed in `gated` by hand.
  `Readiness::summary`/`is_ready` unchanged (the sky still counts there).
- **`tacsim::port_tick::sky_paged(&World) -> bool`** beside `drift_pages`; `PortTickState.sky_paged`
  (crate-private); `readiness::body_without_epoch(&RulesPack, &EphemerisConfig) ->
  Option<CanonicalId>` (crate-private) — the Sky line's flagged text is `"{base}; {body} has no
  epoch_ic, so it moves from the server's epoch."`.
- **`tacsim::port::executor::sale_none_reason(w, &Bid, label, unit, floor, want, &[Bound]) ->
  String`** (crate-private), the single place a queued sale's skip reason is worded.
- **The pure planning functions the story test proved usable from outside the crate** (already
  `pub`, now load-bearing for tests): `tacsim::port::{project_cargo, fill_targets, choose_route,
  HullDrive}`, `tacsim::port::capacity::{max_more, admits, load, compartment_for, format_m3,
  format_tonnes}`, `tacsim::port::capacity_line`, `tacsim::Sky::from_pack`,
  `tacsim::EphemerisConfig`, `tacsim::definitions::Catalog::build_fielded`,
  `mush_market::{DRAW_WINDOW_SECS, DRAW_SETTLE_MARGIN_SECS}`,
  `mush_sim::CarrierJourney::{leg_times, arrival_at}`, `tacsim::port_tick::{last_fire,
  PortFireStats.deferred, pending_notices, NOTICE_HEADING}`.
- **The station packs**: `neighborhood.ceres_freeport` v2 (digest
  `641e1a3983447cca1a05bed6a839f98fe68c2afa683e0deee0c5e4e2904d8538`) and
  `neighborhood.stickney_yards` v3 (`df2e5af11f18c4a5c71bea9e0b5b187b7cdb97c037aecc6b433b9ee3a0d1b2eb`);
  the detail keys `hull registry board`, `sailing plan forms`, `sailing plan pad`, `yard time board`,
  `insurers mark`; the `stations.rs` helpers `old_station_json`, `plan_and_apply`, `names_a_verb`,
  `paints_a_price`, `IC12_DETAILS`, `FICTION_FIGURE`. A kept world takes a new version only through
  `@atlas/pack plan` then `apply`; the next re-publication is Ceres v3 / Stickney v4.
- **The atlas validator's `budget.text` errors** for `short_desc` (> `MAX_SHORT_DESC_BYTES`),
  ambience lines (> `MAX_AMBIENCE_LINES`, blank, > `MAX_AMBIENCE_LINE_BYTES`) and the
  `topology.unreachable` hint text "join it to a reachable room with an unlocked exit, mark it
  `port.dock` if ships reach it, or make it restricted".
- **Player text**: `SHIP_HELP` and `ORDER_USAGE` carry `at=now|HH:MM|daily HH:MM|+Nh|+Nm` /
  `until=+Nh|+Nm`; `+market/depth <catalogue>:<code>`; the help topics `ports`, `ships`, `coop`,
  `orders`, `trade` (T-45 checks every verb token in them); `docs/ops.md` "The trade run (doc 18)".
- **Doc conventions**: doc 18 is Draft 0.4 "as built" through IC12 with `**As built (ICn):**` notes;
  §8's cut-raised questions run Q-18-42..Q-18-65 (IC13 numbers from Q-18-66); §5's rows carry
  "(**landed**, `<hash>`)"; each cut's committer appends its `## ICn` here.

**Re-point of IC0's settle-gate note (`18-cut-notes.md:109-110`).** IC0's "doc 18 cut IC5's
`with_party_cap_source` is what brings it to `server_modules_with`, and IC5's own verify lens names
that run" was written against Draft 0.2's cut numbering. Read it as: cut IC7 built
`with_party_cap_source` (the content-resolved party-cap source), cut IC8 wired the TacSim layer's
`tacsim::port::PortPartyCaps` through it on the roster that composes the layer, and the port's own
stock → quote → accept → settle gate is `crates/tacsim/tests/commerce.rs` — as
`crates/mush-market/tests/ic0.rs:19-25` and `crates/mush-server/tests/e7r2_proving_loop.rs:34-40`
now word it. IC0's text is left as written (history is not rewritten); this note is the re-point.

## IC13

**Buyout, graduation and dormancy** (doc 18 §5, the IC13 row and paragraph), built 2026-10-17 on
main at 715151c4. A First Captain can now buy the hull out, hand it back, or lose it to a week
without a departure, and staff can revoke a charter; the economy's contracts stay the record and the
enrolment document follows them. The hand-played smoke owed since IC12 is still owed (no Redis on
the build machine). One commit, pushed to origin/main.

### What landed

1. **The economy's one seam to re-own a vault** (`mush-economy/src/api.rs` `rehome_vault`,
   re-exported from `lib.rs`). A doc-authority record write beside `set_vault_status`: through
   `put_vault` so the owner index moves, `revision` bumped with `saturating_add(1)`, only the vault
   document persisted (no allocator moved, so no meta write), `false` and nothing written for a
   `System` owner, an unknown vault or no `EconomyStore`. Reached in tests through a test-only
   `@testrehome` module on e2b's `TestPostMod` pattern (`mush-economy/tests/e3b.rs` `RehomeMod`).
2. **The enrolment record grows** (`tacsim/src/port/coop.rs`). `Enrolment` gains `leased_at`,
   `buyout: Option<u64>`, `ended_at: Option<i64>` and `ended: Option<EndCause>` after `status`;
   `EndCause { BoughtOut, Released, Dormant, Revoked { by, why }, LeaseEnded }` (snake_case,
   `deny_unknown_fields`); `MAX_REVOKE_WHY_BYTES = 200`; `check()` holds them together
   (`leased_at >= enrolled_at`; `ended_at` and `ended` set together; an end iff Graduated or
   Revoked; Graduated ⇒ `BoughtOut` with a buyout, and `BoughtOut` only on a Graduated enrolment;
   BuyoutPending ⇒ a buyout; a revocation's reason 1..=200 bytes). `ENROLMENT_SCHEMA` stays 1: no
   kept world holds an enrolment (dev1 runs 4ab3d30d, before IC6). The IC6 observer's fold now
   stamps `ended_at` and `LeaseEnded`. The store gains `enrolment_for_buyout` and
   `enrolment_for_hull` (the governing enrolment: the Active, BuyoutPending or Graduated one, else
   the Revoked one with the greatest `ended_at`; `tacsim/src/store.rs`).
3. **The operate gate is the title or a live charter** (`coop.rs` `Standing { Charter(ContractId),
   Title }`, `Helm`, `helm_of`, `helm_holder`, the private `titled_to`). `helm_of` replaces
   `live_charter` at every operate site: `burn.rs` `burnable`; `executor.rs` `decide`'s per-step
   check; `market.rs` `hull_admits` and `endpoint_for`; `orders.rs` `departs_at`, `orders_cmd`,
   `order_cmd`; `ship.rs` `status`, `hold`, `board`; `voyage.rs` `plan_cmd`, `undock_cmd` and the
   launch re-check (its log now "the pilot neither holds this hull's title nor a charter of it");
   `tell_absent_tenant` reads `helm_holder`. `live_charter` stays for the programme's own charter
   lines. `+ship` prints `SHIP_GRADUATED` on `Standing::Title` (`ship.rs`).
4. **`+coop/buyout`** (new `tacsim/src/port/buyout.rs`; the switch on `+coop` in `commands.rs`,
   dispatched from `coop_cmd`). Checks 0–8 in L14's order (books by gateway account, the berth,
   nothing filed or at the counter or moving, the office, the economy and parties, the funds), each
   with its verbatim refusal, before the first write. Then L1's write order: the pilot offers the
   market's asset-purchase shape (`buyout_spec`: `TitleHeld{hull, Trust}` + a real-clock
   `Before(now + BUYOUT_WINDOW_SECS = 3600)`, one `Exact` payout to the Trust's account,
   `TitleTransfer` Trust → pilot, `deterministic`, `Refund` on expiry, the pilot author and payer);
   `accept_contract` places the pilot's escrow; the enrolment is written `BuyoutPending` **before**
   `end_lease` so the IC6 fold ignores the lease's Cancelled event; `end_lease`;
   `accept_contract_as(Trust)` through `PortPartyCaps` activates it last. Every cancel goes through
   `cancel_own`, which treats `cancel_contract`'s `Ok(false)` as not done and answers
   `BUYOUT_REFUND_PENDING`; every refusal after a write pins the deferred queue. The interim reply
   is `BUYOUT_FILED`; nothing settles in the verb.
5. **The observer's verdict** (`coop.rs` `on_terminal`; `buyout.rs` `settle_buyout`,
   `buyout_verdict`, `graduate`, `relet`). On a lease or buyout reaching Settled, Cancelled or
   Expired, a `BuyoutPending` enrolment's outcome is read from both contracts in the snapshot, never
   the event: Settled → Graduated with `BoughtOut`, the pilot told doc 18 §2 step 19's line verbatim
   (`GRADUATED`, the price through the economy's one render) via `port_tick::notify`; Cancelled or
   Expired with the lease terminal → the Trust re-offers the lease byte-for-byte on `enrol`'s 10c
   terms, the pilot accepts through a plain save/set/restore of `ctx.actor` (never
   `Fire::as_principal`), the enrolment is Active again with the new lease id and `leased_at = now`
   (`BUYOUT_REFUNDED`); otherwise wait. A failed relet ends the half-made lease and writes
   Revoked/`LeaseEnded` (`BUYOUT_UNLEASED`).
6. **The share after graduation** (`tacsim/src/port/proceeds.rs`): `takes_share(status)` in
   `enrolled()`'s filter (belt: a graduated hull has no Trust lease, so `enrolled()` already answers
   `Ok(None)`); `settled()` folds nothing for a Graduated enrolment; a Revoked one still folds a
   programme sale accepted before its charter ended.
7. **The recall in two halves** (new `tacsim/src/port/recall.rs`). `recall_block` (the berth at
   Ceres, nothing in flight through `buyout::in_flight`, an empty cargo pool read good by good
   through `port_tick::read_pool`, the economy), `recall` (the enrolment written Revoked with its
   `EndCause` **first**, then `end_lease`, then the return; on `Retrying` stop with
   `lease_retrying`, the Return arm finishes it), `return_hull`/`return_unchecked` (the pilot's plan
   stopped once with `HELD_RECALLED`, everyone aboard put ashore at the gallery through
   `ship::put_ashore` with `PUT_ASHORE` and the gallery line, the hold vault **re-homed to the
   Trust** with the fuel and stores forfeit inside it and never closed, the laid-up name restored).
   `+coop/release` (the switch on `+coop`; `RELEASED` + `FORFEIT_LINE`, the stake untouched) and
   staff **`@coop/revoke <pilot>=<why>`** (a new `@coop` primary, `EqPair`, `LegacyPostImage`; the
   pilot resolved globally by `#N` or `World::resolve_player` after one leading `*`, never
   `match_object`; attributed in the record, the notice and a `tracing::warn!`) run the same path
   under the same rule. `hull_is_free`'s last clause is now `stowage_held` ("a vault that is not the
   Trust's own Active vault"), so a recalled hull is free again; `+coop/enrol` proves an existing
   Trust vault in step 9 (`EnrolPlan.existing`), allocates no id for it in 10b and re-owns it to the
   pilot in 10f after the stake post (IC6 L6 holds; a `false` there is a logged invariant breach, no
   unwind). `ship.rs` `put_ashore` and `gallery_line` are factored out of `+ship/leave`, whose three
   lines keep their order through a `narrate` callback.
8. **The recall arm on the tick** (`tacsim/src/port_tick.rs` `Arm::Recall`,
   `PortTickState.recall_warned` / `recall_backoff`, `recall_pages`; `recall.rs` `recall_agents`,
   `recall_step`, `dormant`, `idle`; `when.rs` `ic_at_real`). The agents are offered after the
   launches and before the orders, and their hulls are dropped from the order agents that fire
   (L12). Dormancy reads durable departures only (L3): the latest burned ticket scanned down from
   `Voyage.last_seq` (at most `DORMANCY_TICKET_SCAN = 16`), its IC `burned_at` against
   `ic_at_real(now − D)` under the epoch then in force, the pilot's plan's Done departs'
   `reached.utc`, and `leased_at`; `Err` never recalls and pages once. A dormant hull that cannot be
   recalled where she is has her Running plan stopped once (`HELD_DORMANT`), is paged once per boot
   and backed off `RECALL_RECHECK_FIRES = 24`; a revoked hull not back in the pool is returned when
   docked empty at Ceres (`RETURNED_NOTICE`) or held (`HELD_RECALLED`), paged and backed off. No
   authoritative submission, no actor swap.
9. **The hulls readiness line** (`tacsim/src/port/hull.rs` `hull_line`, the private `held`): a hull
   titled to a player is a graduate (`", {g} graduated ({mark} to {owner})"`, the head becoming
   `"{pool} of {pool} made, {pool − g} titled to {org};"`), a lease-less hull whose hold is still
   someone's is `being bought out` or `recalled, not yet returned`; always Ready, byte-identical with
   every count zero. `+coop` gains `GRADUATED_LINE` and `BUYOUT_PENDING_LINE`; `+coop/statement`
   "Charter: bought out; the hull is yours." and "Charter: buyout being entered."
10. **The buyout window's waiting note** (`tacsim/src/port/executor.rs` `buying_out`,
    `HELD_BUYOUT`; the fixer's correctness F2). Between `+coop/buyout` and the settlement a due plan
    step waits under "your buyout of the {name} is being entered; she sails on your title once it
    passes." instead of the lapsed charter's note, and the leased branch drops either note silently
    once the title has passed.
11. **The acceptance tests** (new `crates/mush-server/tests/ic_buyout_dormancy.rs`, 2,286 lines, the
    `ic_slice_loop.rs` rig copied file-local on `build_engine` at rate 24; seven tests):
    `ic13_a_buyout_passes_title_once_ends_the_lease_and_silences_the_share` (title passed exactly
    once with the lease Cancelled, the Trust up by exactly the price, a restart, and the graduate's
    Stickney sale with no share and no counter moved);
    `ic13_a_buyout_is_refused_under_way_cleared_elsewhere_or_with_goods_moving` (the five refusals
    writing nothing, `INFLIGHT_WAIT` reached with an order-arm buy staged);
    `ic13_a_refunded_buyout_re_leases` (a staff-cancelled buyout refunding and re-leasing, then
    graduating);
    `ic13_seven_idle_days_recall_a_docked_empty_hull_and_the_next_enrolment_charters_it` (seven idle
    days recalling a docked empty hull, the forfeit riding with the re-homed vault to the next
    captain of that very hull, a restart and the reconnect notice);
    `ic13_a_departure_keeps_the_charter_and_a_dormant_hull_with_cargo_holds_its_plan` (a burn
    resetting the clock, and a dormant hull at Stickney with cargo holding its plan once and paging
    once); `ic13_release_and_revoke_recall_through_one_path` (three pilot spellings, every refusal,
    Bo put ashore with the Ceres stamp replacing a Stickney one);
    `ic13_a_queued_order_waits_out_the_buyout_and_runs_on_the_title`. Plus `tacsim/tests/coop.rs`
    `ic13_the_helm_is_the_charter_or_the_title` and
    `ic13_a_staff_cancelled_hull_reads_recalled_until_the_return_arm_takes_it_back` (the exact hulls
    lines), and unit tests in `coop.rs`, `buyout.rs`, `proceeds.rs`, `when.rs`, `recall.rs` and
    `e3b.rs`.
12. **Gates, help and ops**: `legacy-handlers.toml` (`+coop` and `+ship` notes refreshed, the
    `@coop` stanza in byte order, baseline 97); `docs/help/coop.md` ("The charter" ends three ways;
    "Buying the hull out", "Handing the hull back", "A week without a departure"; `@coop/revoke` not
    named); `docs/ops.md` "Buyout, release, revocation and dormancy" (the revoke runbook, the arm's
    two pages, the launch arm's page for a filed hull, the buyout's two error pages, the recall's
    Retrying window).

### Deferred, and why

**Lead rulings the planner took** (recorded as taken, not chosen):

- **L1** — the buyout's step order is offer → the pilot's escrow → `BuyoutPending` → `end_lease` →
  the Trust's acceptance, which **wins over doc 18 §4.1's (3)(4)(5)**: `cancel_contract` refuses an
  Active contract with deliverables (`contract_live.rs:5418-5435`), so "on Retrying the verb cancels
  its own contract" is buildable only while the contract is still Offered. The economy is unchanged.
- **L2** — `helm_of` (live charter, else title) at every former `live_charter` operate site; the
  programme's own lines keep `live_charter` (lead 2b/2i, D-18-23 read for a graduate).
- **L3** — dormancy reads durable departures only: the latest burned ticket (16-deep scan,
  `burned_at` vs `ic_at_real(now − D)`), Done departs' `reached.utc`, `leased_at`; a read error
  never recalls (lead 2f/2i).
- **L4** — the recall has two halves; the tick's Return arm finishes every charter ended elsewhere
  (staff `@contract/cancel`, a Retrying lease end, a failed re-lease), closing IC6's residual 3.
- **L5** — the hold vault is **re-homed** to the Trust through the new `rehome_vault` seam, never
  closed or emptied: the Trust has no vault, moving pools needs multi-fire authoritative
  submissions, and a Closed vault keeps its backing index so the hull could never be chartered
  again. Re-homed after the post on the next enrolment, so IC6 L6 holds.
- **L6** — `leased_at`, `buyout`, `ended_at`, `ended: Option<EndCause>` replace the lead's
  `graduated_at`/`revoked_at`; schema stays 1 (lead 2j).
- **L7** — the observer decides from the economy's state, never the event alone; the re-lease is a
  plain `ctx.actor` save/set/restore around one `accept_contract` (not `Fire::as_principal`) and
  skips the readiness gate; every verb cancel through `cancel_own`.
- **L8** — the hulls line counts graduated, being-bought-out and recalled hulls and is always
  Ready; `free` excludes a non-Trust-Active stowage vault but still differs from `hull_is_free` on
  place (IC6 L9 kept).
- **L9** — `@coop` is a new primary with one staff switch (doc 18 §4.1 and 18a §4 name
  `@coop/revoke`, R8); `+coop/buyout` and `+coop/release` are switches.
- **L10** — no lease = no share; `takes_share(Graduated) == false`; a graduate folds no counters;
  Revoked still folds a settling programme sale (lead 2h).
- **L11** — `BUYOUT_WINDOW_SECS = 3_600` real.
- **L12** — recall agents after the launches, before the orders; their hulls removed from the order
  agents; hulls with a pending voyage skipped (the launch arm's `launch_hold` page is lead 2f's
  `@tacsim/port clear` page; no second page).
- **L13** — the acceptance tests are a sibling file with a copied rig; `ic_slice_loop.rs` untouched.
- **L14** — the buyout's check order: books, berth, voyage, in flight, goods, office, funds (makes
  "under way" reachable).
- **L15** — the recall restores the laid-up name.
- **L16** — `@coop/revoke` and `+coop/release` use the full recall rule (docked at Ceres, empty
  cargo, nothing in flight); the Revoked-but-lease-Retrying window is documented in ops.md, not
  gated (changing `helm_of` for a pending terminal would alter every retrying lease end).

**Owner-critical defaults taken** (for the owner's read; none changed in the cut):

- **O1 / Q-18-66** — the forfeited fuel and stores ride with the hull: the hold vault is re-homed to
  the Trust with its propellant and provisions, also on a voluntary `+coop/release`, and the next
  captain inherits them. Doc 18 §4.1 says "no confiscation of matter in the slice" (`:1063`); the
  lead's 2(c) says fuel and stores are forfeit. The cut reads the doc's words as covering cargo
  only. **Contradicts the doc's plain text; the owner should rule.**
- **O2 / Q-18-67** — revoke and release use the full recall rule; a charter ended away from Ceres
  leaves the hull out of the pool with nobody to fly her home; no staff "bring her home" verb in
  this cut.
- **O3 / Q-18-68** — the staff reason is shown to the pilot in `REVOKED_NOTICE`.
- **O4** — every player-facing text this cut writes is the builder's voice (the fixer's
  `HELD_BUYOUT` included) except `GRADUATED`, doc 18 §2 step 19's owner-reviewed line used
  verbatim. Step 19 shows it as the reply to `+coop/buyout`; the cut sends it as the **settlement
  notice**, because the title passes at the next sweep, and answers the verb with the interim
  `BUYOUT_FILED`. The owner should rule where step 19's line belongs.
- **O5 / Q-18-69** — the laid-up name comes back at the recall; the mark is untouched.
- **O6** — between `+coop/buyout` and the next sweep (at most 30 s) the pilot holds neither charter
  nor title: board, undock and the queue refuse (a due plan step waits under `HELD_BUYOUT`). Follows
  from doc 18 §4.1's end_lease-before-settlement design.
- **O7 / Q-18-70** — a graduated hull leaves the Trust's pool for ever; the pool of eight shrinks
  per graduate and the desk answers `NO_HULL` once every hull is chartered or graduated; the slice
  buys no replacements.

**Builder's and fixer's technical choices** (declared): blocked plan holds now tell the pilot once
(`hold_plan(tell = true)`, as the executor's stopped holds do); the cause notice is sent from
`recall()` also on Retrying; `recall_warned` is shared by all the arm's pages; an Unreadable
sailing plan fails the dormancy read safe; `put_ashore` takes `reason` and a `narrate` callback so
`+ship/leave`'s output order is unchanged; `buyout_spec` takes `mark` as an eighth argument;
`recall_agents` takes `&mut PortTickState`; `@coop/revoke` checks the programme port after staff;
an extra `check()` rule (`BoughtOut` only on a Graduated enrolment).

**Deferred, and why:**

- **The per-fire enrolment scan** in `recall_agents` (every lifetime enrolment, `live_charter` per
  Active one, `enrolment_for_hull` + `trust_lease_on` per Revoked one). Correct and pre-launch
  cheap; a rewrite over the programme map's hull rows (bounded by the pool) when records
  accumulate.
- **`snapshot.rs:56`'s "Vault.owner is immutable"** is now false (`rehome_vault` re-owns through
  `put_vault`), and the Trust owns a hull's hold vault between a recall and the next enrolment, so
  `owner_home(Trust)` would route a Trust object deposit into a laid-up Dray's stowage. Comment to
  refresh; the routing is a latent surprise, not a present defect.
- **A failed relet with cargo aboard** strands it: `+coop/buyout` never required an empty cargo
  pool, the Return arm does, and a Revoked pilot can neither sell nor fly. ops.md's "staff move her
  cargo or the hull by hand" covers it; no verb does.
- **`put_ashore` unplaces before `move_to`** (inherited from `+ship/leave`): a refused move on a
  tick-driven recall leaves the person unplaced in a hull the recall then returns. Low; staff fix
  by hand as the brief says.
- **The buyout's failure residues** (escrow refused after the offer; `end_lease` Retrying →
  `BUYOUT_HELD`; `end_lease` Err → back to Active; the Trust's acceptance not activating; a relet
  failure) and `RELEASE_RETRYING` / `REVOKED_RETRYING` have no test; the paths were read against the
  brief's residue table.
- **Wire strictness**: `buyout`, `ended_at` and `ended` deserialize as `None` when absent (serde's
  `Option` rule); only `leased_at` is required on the wire. `check()` refuses every
  status-incoherent omission.
- **Test 5** proves the 24-fire backoff, not `hold_plan`'s written-once rule on a re-check; the
  cross-epoch cutoff and the Done-depart arm are proven by pure unit tests only; e3b's "writes
  nothing" checks look through `fold_kv`, which drops `StreamAppend` and other namespaces.
- **The exact graduated and recalled hulls lines** are asserted on the IC6 rig, not the production
  root (test 1 uses contains-fragments).
- **Stale comments**: `orders.rs:992` "left for IC13 to prune"; `commands.rs:154-164` names only
  `enrol` as `+coop`'s writing switch; `coop::row_of` widened to `pub(crate)` with no outside
  caller.
- **A recall whose `end_lease` returns `Err`** (unreachable past check 6 by the brief) leaves the
  enrolment Revoked with the lease live and no pending terminal; the Return arm never fires and the
  pilot keeps operating until staff `@contract/cancel` the lease.
- **The hand-played smoke**, owed since IC12 (no Redis on the build machine).
- **Not in this cut** (the brief's §5): Q-18-60's bid gate, the P-cuts, IC14's registry and
  berth-board `LookRendered` lines, a staff bring-her-home verb, Q-18-14's cooldown, every pack
  version.

### Pins re-blessed

- **`crates/mush-contract-tests/legacy-handlers.toml`**: the `+coop` note refreshed ("REFRESHED (not
  joined) by doc 18 cut IC13: two switches, /buyout … and /release …; PINNED_BASELINE does not
  move"), the `+ship` note gaining "Doc 18 cut IC13: the operate gate is the title OR a live charter
  (`helm_of`)", and a new non-baseline `@coop` stanza (`legacy-post-image`, family `tacsim.coop`)
  between `@contract` and `@corridor` in byte order. `PINNED_BASELINE` stays **97**: the two `+coop`
  switches are arguments to an existing verb, and `@coop` is a new non-baseline handler under the
  module's class (L9).
- **Existing assertions changed, each as the brief directs**:
  - (a) `tacsim/src/port/coop.rs` `ic6_enrolment_wire_is_strict`'s `sample()` gains the four fields
    (an addition).
  - (b) `tacsim/tests/coop.rs` `ic6_eight_simulated_days_raise_no_stale_contract_page` re-places the
    hull at Stickney so the recall is blocked and the lease stays live across the eight days; the
    lease and enrolment are asserted Active (the pre-IC13 claim restored after the builder's interim
    Cancelled/Dormant version) and `recall_pages == [hull]` is added — a stronger guard,
    mutation-checked by the fixer (with `exempt_from_stale_page` forced false it fails "the live
    lease was paged as stale").
  - (c) `tacsim/tests/orders.rs` `ic11_a_lapsed_charter_waits_and_stages_nothing` →
    `ic11_a_lapsed_charter_holds_and_stages_nothing`: the Return arm now reaches the staff-ended
    charter's hull at Stickney with cargo and stops the lapsed author's plan (`HELD_LINE` with
    `HELD_RECALLED`, `!h.retry`, `PlanState::Held`, the dropped order `(Held, 2)`) where IC11's
    order arm wrote a waiting note (`WAITING_LINE` with `HELD_CHARTER`, `h.retry`, `(Pending, 2)`);
    L4/L12 direct this, and "nothing re-staged", "nothing filed again" and "no second sale" are
    unchanged. `ic6_staff_cancel…`'s "Drays free: 7 of 8." did not change (it never fires the tick).
- **Unchanged, run and green**: `state-slot-owners.toml` (no new foreign `module_state_mut`; the
  economy writes go through `rehome_vault`, `end_lease`, `offer_*`, `accept_*`, `cancel_contract`),
  `matcher-call-sites.toml` (no `match_object` call: the pilot resolves by `#N` or
  `World::resolve_player`), the panic budgets, the restriction-lints census, `module-ns-owners.toml`,
  the composition-edge pin, every content digest (lane v3, Dray v2, goods v1, Ceres v2, Stickney v3,
  the starter and regions unmoved), `help-commands-allowlist.toml` (T-45; `+ship/order resume` is
  already named in `docs/help/orders.md`), the CI gated-test inventory, `ports.rs` test 8's hulls
  Ready text, `boot_parity`.

### How it was verified

On Windows at the working tree over `715151c4` with the cut uncommitted, the final gate — all 18
commands green on the first run, nothing rerun, no LNK1104, no hung binary:

```
cargo fmt --all -- --check                                        (no output; exit 0; twice)
cargo clippy --workspace --all-targets -- -D warnings             Finished `dev` profile … in 27.66s; 0 warning/error lines (again 0.58s)
cargo test -p mush-economy --lib                                  37 passed
cargo test -p mush-economy --test e3b --test e4b1 --test e4b2 --test e4c
    e3b 36; e4b1 85; e4b2 117; e4c 44; all passed
cargo test -p tacsim --lib                                        926 passed
cargo test -p tacsim --test coop --test sale --test orders --test voyage --test commerce
    coop 20; sale 15; orders 27; voyage 15; commerce 20; all passed
cargo test -p tacsim --test hulls --test ports --test port_tick --test stations --test content --test lane
    hulls 15; ports 19; port_tick 16; stations 18; content 29; lane 8; all passed
cargo test -p mush-server --test ic_buyout_dormancy               test result: ok. 7 passed; 0 failed
cargo test -p mush-server --test ic_slice_loop --test boot_parity ic_slice_loop 5; boot_parity 18; all passed
cargo test -p mush-market --lib                                   27 passed
cargo test -p mush-contract-tests                                 every binary ok (legacy_handler_allowlist 4, panic_budget 7, restriction_lints 11, state_slot_owners 4, module_ns_owners 3, matcher_call_sites 2, help_commands 4, scale_harness 6 passed with 8 ignored as before)
cargo test -p mush-housing --no-fail-fast                         1, 54, 81, 1 passed
cargo test -p mush-campaign --no-fail-fast                        54, 58, 15, 85, 28, 5 passed
CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast
    400 result lines; 7521 passed, 0 failed, 92 ignored
cargo test -p mush-bot                                            lib 469; arena_e2e 9; e2e 49; playtesting 9; every other binary ok
```

`ipc_authoritative_action` (8 passed) and `arena_story` (1 passed) passed inside the saturated
sweep; the one panic in the log is the WS01-S8 injected world-turn fault, whose tests passed. No
load-sensitive suite flaked. The fixer's mutation check: with `exempt_from_stale_page` forced
false, `ic6_eight_simulated_days` fails with "the live lease was paged as stale"; `contract.rs`
restored, diff empty, the test re-run ok.

Three review lenses on the uncommitted tree (correctness, conformance with a 40-item checklist,
tests), twelve should-fix/note findings across them; the fixer fixed all six should-fix findings
and rebutted none (correctness F1 + conformance F1: the ic6 stale-page test; correctness F2: the O6
window's `HELD_CHARTER` → `HELD_BUYOUT`; conformance F2: the help page's dormant-hold wording;
tests F1: Bo's vacuous last-port stamp; tests F2: the executor gate's lost coverage → test 7 and
the IC11 rename). The judge re-read the whole diff and the three new files and verified each
disposition against the code; the notes left standing are recorded under "Deferred, and why".
Verdict: ship with notes.

### Surface for the next cuts

Public items later cuts (IC14, the P-cuts) build on:

- **Economy** (`crates/mush-economy/src/api.rs`, re-exported from `lib.rs`): `pub fn
  rehome_vault(ctx: &mut Ctx<'_>, id: VaultId, owner: PrincipalId, scope: Option<OrgScopeId>, name:
  SmolStr) -> bool` — re-owns through `put_vault` (index follows), `revision.saturating_add(1)`,
  persists the vault only; `false` and nothing written for a `System` owner, an unknown vault or no
  store. Note: `Vault.owner` is no longer immutable, and the Trust owns a hull's hold vault between
  a recall and the next enrolment.
- **Enrolment** (`crates/tacsim/src/port/coop.rs`, re-exported from `port/mod.rs`): `Enrolment {
  …, status, leased_at: i64, buyout: Option<u64>, ended_at: Option<i64>, ended: Option<EndCause>,
  sales, margin_minor, shares_minor }` (`ENROLMENT_SCHEMA = 1`; the three Option fields read as
  `None` when absent, `leased_at` is required); `pub enum EndCause { BoughtOut, Released, Dormant,
  Revoked { by: ObjId, why: String }, LeaseEnded }` (snake_case wire: `"dormant"`,
  `{"revoked":{"by":1,"why":"…"}}`); `pub const MAX_REVOKE_WHY_BYTES: usize = 200`; `check()` holds
  the end/status/buyout rules together. `pub const GRADUATED_LINE`, `BUYOUT_PENDING_LINE`. Now
  `pub(crate)`: `money_minor`, `say`, `office_of`, `own_enrolment`, `Books`, `account_books`,
  `row_of`, `titled_to(w, hull, &PrincipalId) -> bool`, `stowage_held(&EconomyStore, &PrincipalId
  /*trust*/, stowage: ObjId) -> bool` (the one predicate `hull_is_free` and `hull_line` share).
- **The operate gate** (`coop.rs`, re-exported): `pub enum Standing { Charter(ContractId), Title }`;
  `pub struct Helm { pub hull: ObjId, pub standing: Standing }`; `pub fn helm_of(w: &World, pilot:
  ObjId) -> Option<Helm>` (live charter first, else the lowest-id programme-map hull whose Active
  asset is titled `Object(pilot)`); `pub fn helm_holder(w: &World, hull: ObjId) -> Option<ObjId>`
  (`charter_tenant`, else the `Object` title holder of a programme-map hull). `live_charter` and
  `charter_tenant` remain for the programme's own lines.
- **Store** (`crates/tacsim/src/store.rs`): `TacsimStore::enrolment_for_buyout(&self, contract:
  u64) -> Option<&Enrolment>`; `TacsimStore::enrolment_for_hull(&self, hull: ObjId) ->
  Option<&Enrolment>` (the governing enrolment: Active | BuyoutPending | Graduated, else the Revoked
  one with the greatest `ended_at`).
- **Buyout** (`crates/tacsim/src/port/buyout.rs`, `pub mod buyout`): `pub const BUYOUT_WINDOW_SECS:
  i64 = 3_600`; `pub(crate) fn buyout_spec(pilot_account, trust: &PrincipalId, trust_account, hull,
  pilot, mark: &str, price: Amount, now: i64) -> ContractSpec`; `pub(crate) fn buyout_cmd(ctx) ->
  Result<(), CmdError>`; `pub(crate) enum Busy { Cleared, Inflight, Goods }` and `pub(crate) fn
  in_flight(w, &Enrolment) -> Option<(Busy, String)>` (shared by the recall); `pub(crate) enum
  Verdict { Graduate, Relet, Wait }`, `pub(crate) const fn buyout_verdict(ContractStateKind,
  lease_terminal: bool) -> Verdict`; `pub(crate) fn settle_buyout(ctx, &Enrolment)`. Texts (pub,
  verbatim): `BUYOUT_USAGE`, `ALREADY_GRADUATED`, `BUYOUT_PENDING`, `BUYOUT_UNDER_WAY`,
  `BUYOUT_NOT_HERE`, `CLEARED_WAIT`, `INFLIGHT_WAIT`, `GOODS_WAIT`, `BUYOUT_WHERE`,
  `BUYOUT_FAILED`, `BUYOUT_FUNDS`, `BUYOUT_REFUND_PENDING`, `BUYOUT_HELD`, `BUYOUT_FILED`,
  `GRADUATED` (doc 18 §2 step 19's line), `BUYOUT_REFUNDED`, `BUYOUT_UNLEASED`.
- **Recall** (`crates/tacsim/src/port/recall.rs`, `pub mod recall`): `pub const COOP_STAFF_VERB =
  "@coop"`, `COOP_STAFF_HELP`, `DORMANCY_TICKET_SCAN: u32 = 16`, `RECALL_RECHECK_FIRES: u64 = 24`;
  `pub(crate) enum RecallCause { Released, Dormant, Revoked { by, why } }`; `pub(crate) enum
  RecallBlock { UnderWay, NotAtPort, Cleared, Inflight, GoodsMoving, Cargo, Unreadable, Economy }`;
  `pub(crate) struct RecallDone { tank: u64, locker: u64, ashore: Vec<ObjId>, lease_retrying: bool
  }`; `pub(crate) fn recall_block(&Ctx, &Enrolment) -> Option<(RecallBlock, String)>`; `pub(crate)
  fn recall(&mut Ctx, &Enrolment, RecallCause) -> Result<RecallDone, (RecallBlock, String)>`;
  `pub(crate) fn return_hull(&mut Ctx, &Enrolment) -> Result<RecallDone, (RecallBlock, String)>`;
  `pub(crate) fn release_cmd(ctx)`, `pub(crate) fn revoke_cmd(ctx)`; `pub(crate) fn idle(now,
  leased_at, days: u32, last_burn_ic: Option<i64>, cutoff_ic: i64, last_reach_utc: Option<i64>) ->
  bool`; `pub(crate) fn dormant(&Ctx, &Enrolment, days: u32) -> Result<bool, ()>`; `pub(crate) fn
  recall_agents(&Ctx, &mut PortTickState) -> Vec<(ObjId /*hull*/, ObjId /*pilot*/)>`; `pub(crate)
  fn recall_step(&mut Ctx, &mut PortTickState, hull: ObjId) -> Step`. Texts (pub, verbatim):
  `RECALL_UNDER_WAY`, `RECALL_NOT_AT_PORT`, `RECALL_CARGO`, `RECALL_HOLD_DARK`, `RECALL_ECONOMY`,
  `HELD_RECALLED`, `HELD_DORMANT`, `PUT_ASHORE`, `DORMANT_NOTICE`, `REVOKED_NOTICE`,
  `RETURNED_NOTICE`, `RELEASE_USAGE`, `RELEASE_WHERE`, `RELEASED`, `FORFEIT_LINE`,
  `RELEASE_RETRYING`, `REVOKE_USAGE`, `REVOKE_WHY_BOUNDS`, `NO_SUCH_PILOT`, `NO_COOP_CHARTER`,
  `NOT_REVOKED`, `REVOKED_REPLY`, `REVOKED_RETRYING`.
- **Ship** (`crates/tacsim/src/port/ship.rs`, not `recall.rs`): `pub const SHIP_GRADUATED`;
  `pub(crate) fn put_ashore(ctx, person, hull, gallery, reason: &str, narrate: impl FnOnce(&mut
  Ctx<'_>)) -> Result<(), String>` (unplace, move_to, narrate, show_room, `Moved`, the last-port
  stamp); `pub(crate) fn gallery_line(ctx, who, gallery, name: &str)`.
- **Executor** (`crates/tacsim/src/port/executor.rs`): `pub const HELD_BUYOUT: &str = "your buyout
  of the {name} is being entered; she sails on your title once it passes."`; `decide()` waits under
  it when the author's governing enrolment is BuyoutPending and drops either note silently on the
  leased branch.
- **Share** (`crates/tacsim/src/port/proceeds.rs`): `pub(crate) const fn
  takes_share(EnrolmentStatus) -> bool`; `settled()` folds nothing for a Graduated enrolment.
- **Time and the tick** (`when.rs`, `port_tick.rs`): `pub(crate) fn when::ic_at_real(w: &World,
  real: i64) -> i64` (the epoch in force at `real`); `pub fn port_tick::recall_pages(w: &World) ->
  Vec<u32>`; `Arm::Recall { hull: u32 }` offered after `Launch`, before `Order`, with recall hulls
  removed from the order agents; `PortTickState.recall_warned: BTreeSet<u32>`,
  `PortTickState.recall_backoff: BTreeMap<u32, u64>`.
- **Verbs and gates**: `+coop` switches `enrol | statement | buyout | release`; new `@coop`
  CommandSpec (`ArgShape::EqPair`, switch `revoke`, `LegacyPostImage`, staff; `commands.rs`
  `is_staff`, `require_staff`, `require_enabled` now `pub(crate)`); `legacy-handlers.toml` `@coop`
  stanza, baseline 97. **Hulls line clauses** (after `{k} leased{leases}`): `, {g} graduated ({mark}
  to {owner}, …)`, `, {b} being bought out ({mark}, …)`, `, {r} recalled, not yet returned ({mark},
  …)`; with g > 0 the head is `{entry} hulls: {pool} of {pool} made, {pool−g} titled to {org};`;
  always Ready.
- **Test rig** (`crates/mush-server/tests/ic_buyout_dormancy.rs`): file-local copies of
  `ic_slice_loop.rs`'s rig plus `top_up_to`, `title_moves`, `title_owner`, `vault_owner`,
  `untouched`/`assert_untouched`, `hulls_line`, `registry_from_chandlery`, `gallery_from_registry`,
  `launch`, `arrive`; `tacsim/tests/coop.rs` gains `hulls_line(rig)`. IC14 and later cuts that need
  a tacsim `Ctx`-level economy write should follow `e3b.rs`'s `RehomeMod` pattern.
- **Doc conventions**: this cut's owner items O1/O2/O3/O5/O7 take the numbers Q-18-66..Q-18-70 in
  this note only; doc 18 and 18a are not edited by this cut, so §8 does not yet list them and the
  next docs pass adds them (the next cut-raised question is Q-18-71).

## IC14

**The residuals sweep** (doc 18 §5, the IC14 row this cut adds), built 2026-10-17 on main
at 94679739. The sweep closes the residuals the IC0–IC13 cut notes recorded and no cut
owned, so the slice can run on a long-lived world: risk 19 is bounded, not abolished; money
cannot move with no leg behind it on the paths the brief's table names; every closed state
converges. The hand-played smoke owed since IC12 is still owed (no Redis on the build
machine). One commit, pushed to origin/main.

### What landed

1. **Risk 19 bounded, not closed: a durable per-endpoint marker ring with a real-clock age floor**
   (`mush-inventory/src/record.rs:68-98` the four constants `MAX_RETAINED_MARKERS_PER_ENDPOINT` =
   32, `MARKER_RETENTION_SECS` = 86,400, `MAX_MARKER_RING` = 128, `MAX_RING_EVICTIONS_PER_PUSH` = 8;
   `:1197-1332` `RingEntry`, `MarkerRing` with its checked `Deserialize`, `push`; `pool.rs:66-73`
   `InventoryFamilies::marker_ring`, `:571-638` `retain_marker`; `module.rs:317-343` the family
   `inventory_marker_ring`). A marker is evicted only when its source endpoint's ring holds more
   than 32 **and** the front entry is a real day old at the push; at most eight per push; a ring of
   128 young entries refuses the move (`InventoryError::MarkerRingFull`, nothing written). The real
   stamp rides every marker-writing offer's fixed parameters as `"wall"` beside `"now"`
   (`provider.rs:611-616`, `salvage.rs:438-443`, `mush-market/src/lot.rs:759-767`) and is read by
   `prepare_wall` (`provider.rs:343-355`, `lot.rs:348-363`), so a prepare still reads no clock and
   two replays stamp identically; `write_marker`, salvage and `transfer_marked` (its signature gains
   `wall: i64`) record the marker in the source's ring in the same bundle. The mirror and `facts.rs`
   are untouched: E7c2 fold R3's fail-closed rule stands.
2. **The market reads an absent sale marker as unconfirmed past the horizon**
   (`mush-market/src/trade.rs` `goods_moved`, `marker_horizon_passed`; review F1): inside
   `accepted_at + MARKER_RETENTION_SECS` an absent Deliver marker means never written
   (`GoodsMoved::No`, the bid restored); past it (a sale disputed for days, its hold moving on) it
   is `Unknown` — paged, never restored over goods that may have arrived (L5).
3. **The `Markers` headroom line and the incomplete-mirror gate** (`tacsim/src/port/readiness.rs`
   `Binding::Markers`, `markers_line`, `inventory_facts_line`; `coop.rs` `gated`; `port_tick.rs`
   `page_marker_headroom`, `marker_pages`): Ready to three quarters of `MAX_APPLIED_MARKERS`,
   Flagged above it and paged once a boot, never gated (the sky's manner, proposed Q-18-72); the
   `InventoryFacts` line reads Missing — and gates — when `InventoryStore::is_complete()` is false.
   `docs/ops.md` "Applied-marker retention (doc 18 cut IC14)" replaces the pre-deploy blocker and is
   worded as a bound.
4. **Two mush-core read seams** (`mush-core/src/output.rs:460-488`
   `Outbox::authoritative_queue_full` — distinct actors in the noted set plus every
   `Deferred::AuthoritativeSubmit` already in this outbox against `AUTHORITATIVE_SUBMIT_QUEUE_MAX`;
   `cmd.rs:352-370` `Ctx::authoritative_queue_full`, `Ctx::authoritative_commit_pending`;
   `action_state.rs:620-631` `ModuleActionState::commit_pending` over `has_prepared()`).
5. **A sale is refused before the escrow when the engine queue is full** (`trade.rs` step 6b,
   `SELL_QUEUE_FULL`, before any contract); the typed verb adds `SELL_REMEDY` as a second line
   (`commands.rs` the lot arm); the executor treats `SELL_QUEUE_FULL` and `SELL_BUSY` as
   back-pressure — `Step::Deferred`, no log, no count, no write (`tacsim/src/port/executor.rs`
   `is_back_pressure`, the `Plan::Sell` arm).
6. **`restage_deliver` reads the marker before the cutoff** (`trade.rs` `restage_deliver`:
   `DELIVER_LANDED` for a landed leg, `DELIVER_UNCONFIRMED` when it cannot be read, else the
   cutoff); the executor's `restage_outcome` advances a landed re-stage exactly as `Ok(None)` and
   lets an unconfirmed one fall into the existing `Err` arms, so it ends in `HELD_UNCONFIRMED` at
   the second deadline (L32).
7. **A seller cannot dispute their own sale into a bid** (`mush-economy/src/contract.rs:1056-1075`
   `DisputeGuard`; `module.rs` `EconomyModule::with_dispute_guard`, `EconomyStore::dispute_guard`;
   `contract_live.rs` `contract_dispute` asks it for a non-admin party before the state check;
   `mush-market/src/trade.rs` `SaleDisputeGuard`, `SALE_DISPUTE_REFUSED`; wired unconditionally in
   `tacsim/src/composition/mod.rs`).
8. **Money-safety small items**: `settled_sale` folds nothing and logs on an overflowing gross
   (`trade.rs` `sale_gross`/`gross_of`); a settlement the economy healed at boot is folded on the
   market's first sweep, once (`boot.rs` `fold_healed_sales`, `MarketStore::healed_sales`, pushed
   under `reconcile_contracts`' `q.settled.is_none()` arm; L4); `release_quote` pages "could not be
   confirmed" when it could not (`GoodsMoved::{Yes, No, Unknown}`, `lapsed_page_text`); `owner_home`
   never answers a `SpatialInterior` vault (`contract.rs:2776-2784`; `snapshot.rs:56-58` reworded);
   `Listing.draw: Option<bool>` freezes the Draw kind at publication with IC7's derivation kept for
   a pre-IC14 row until `put_listing` stamps it (`record.rs`, `store.rs` `derived_draw`,
   `put_listing`; `api.rs` two sites); `release_lapsed_hold` checks custody first (`lot.rs`); the
   kept-lapsed `hold_standing` read backs off per listing (`boot.rs` `HOLD_RECHECK_SWEEPS` = 10,
   `MarketStore::{hold_checked, release_staged, sweeps, hold_reads}`); the market's
   `is_marker_id`/`MAX_MARKER_ID_BYTES` are deleted and inventory's exported
   (`mush-inventory/src/lib.rs`, `pool.rs:176-185`).
9. **Every closed state converges** (`tacsim/src/port/recall.rs`): a Revoked enrolment whose own
   lease is still live is ended by the recall arm under the backoff and the hull returned
   (`revoked_with_live_lease`, `end_revoked_lease`; L10); `recall_agents` walks the programme map's
   hull rows, one governing enrolment each (`governing_rows`), and skips an unreadable plan as it
   skips a filed one; `recall`'s answer to `end_lease` is the pure `recall_outcome`.
10. **A failed relet retries, bounded** (`buyout.rs` `relet(ctx, e, attempt, give_up)`,
    `relet_pending`, `MAX_RELET_ATTEMPTS` = 6, `MAX_RELET_BACKOFF_FIRES` = 720, `relet_backoff` 24
    doubling; `recall.rs` `relet_due`, `relet_step`; `port_tick.rs` `PortTickState.relet_attempts`):
    the observer's first attempt fails, the pilot is told `RELET_PENDING` once and the enrolment
    stays BuyoutPending; the recall arm records attempt 1 at first sight, pages once, retries at 2,
    4, 8, 16 and 32 minutes, and the sixth failure ends exactly as IC13 built it (Revoked,
    `LeaseEnded`, `BUYOUT_UNLEASED`, its own error line). The plan waits under `HELD_RELET`
    meanwhile and the silent re-check drops it (`executor.rs` `reletting`, `decide`).
11. **An unreadable sailing plan fails closed everywhere and pages once** (`voyage.rs`
    `voyage_blocks`, `voyage_dark`; `ship.rs` board and leave say `VOYAGE_DARK`; `market.rs`
    `HOLD_PAPERS_DARK`; `hull.rs` counts it departing and keeps the line's state; `coop.rs`
    `hull_is_free && !voyage_blocks`; `recall.rs`; `store.rs` `voyages_unreadable`; `port_tick.rs`
    `page_dark_voyages`, `voyage_pages`). `+coop/enrol` stays open on the other hulls (L17, proposed
    Q-18-73).
12. **The launch arm's residuals** (`voyage.rs`, `port_tick.rs`): the lapse also waits while a
    prepared action owns the typed store (`authoritative_commit_pending`; L15); a held launch is
    `Step::Held`, counted in `PortFireStats.held`, never the walk's generic warning (L16);
    `Voyage.told: u32` (required on the wire, `<= last_seq`; L8b) is written by `launch_verify`'s
    `mark_told` and by `launch_retell` on the first fire after a boot, so `CLEARANCE_GRANTED` is
    told once across reboots (L20); `tonnes_over` rounds an over-rail payload's tenths up at the
    plan, the refusal and the executor's render (L18); `HOLD_CLEARED` for a docked hull with a filed
    clearance (L19); `sky_pages: u32` replaces `sky_paged`.
13. **The executor's and the plan's residuals** (`executor.rs`, `orders.rs`, `def.rs`, `stock.rs`,
    `vendor.rs`): `ORDER_RESTAGE_SECS` = 90 (`SWEEP_PERIOD_SECS` + 30, asserted; L21); a queued
    depart resolves its venue word exactly (`match_destination_exact`); `HELD_SHORT_FOR_DEPART`
    projects the later consumable orders before the next depart (`Cover`, `later_cover`; L22,
    Q-18-51's default changed); resume from Paused re-arms an inflight re-stage; `PortDef::check`
    bounds a stock row's `qty` to `1..=MAX_QUANTITY` and a port's stock rows to `MAX_CLERK_ROWS`;
    the clerk skips an uncarried row and a mint refused `MAX_STOCK_ATTEMPTS` = 3 times
    (`stock_skipped`, `stock_attempts`, `stock_uncarried`; L24); `vendor::preflight` and `put`
    refuse a storefront that already backs a vault (`backs_no_vault`); a retired port entry's clerk
    idles (`stock::entry_latched` per entry from the content latch; the walk's clerk list; L25 — the
    base run failed as F9 required).
14. **Atlas, sim and corridor** (`mush-atlas/src/apply.rs` `check_exit_unanchored`, before any
    retire, with `mush-content/src/apply.rs:263-269`'s `EntryApplier::check` contract amended to
    allow a read-only world check for exactly this case; `mush-sim/src/transition.rs` `plan_members`
    refuses Limbo, the recovery room, the in-transit room and a vault-backing room, `data.rs` naming
    both writers of `rooms`; `tacsim/src/store.rs` `route_in_world_at`, `latched_route_at` and
    `commands.rs` `corridor_of` honouring `@<version>` on the latch fallback).
15. **The seed-latch gap, reproduced and fixed** (`tacsim/src/lib.rs` two `persist_delegation`s,
    tacsim -> `content` and tacsim -> `atlas`, with
    `crates/mush-contract-tests/module-ns-owners.toml` rows; L28): at base `@tacsim/seed
    tacsim.corridor@1 commit` left `refused_namespace_writes` = [(tacsim, atlas), (tacsim, content)]
    and the latch row was `None` after a fold and reboot.
16. **`world_task`'s race** (`mush-server/src/world_task.rs:18495-18508`, test-side only; L29):
    sixteen `yield_now`s hand the world its `Wake::Closed` turn before the receipt exists, plus
    `assert!(!world_join.is_finished())`; 30 of 30 alone, and inside the saturated sweep.
17. **Stickney v4** (`crates/tacsim/content/atlas/neighborhood.stickney_yards.json`, digest
    `f1bdc803…16f6`; L26): exactly the three clauses struck — P04 `shift boards` "(+atlas has the
    same map, in words)", P07 `intake board` "(it is what +market/list prints here)", P08 `price
    board` "; +market/list has them to the credit" — and nothing else, proven by rebuilding v3 to
    its pinned digest `df2e5af1…b2eb` (`tests/stations.rs` `stickney_v3_json`).
18. **The small presentation residuals of Q-18-47** (L27): the accept line's signatory clause
    (`mush-market/src/module.rs` `VenueGate::signatory`, default `None`; `commands.rs`
    `ACCEPT_SIGNS`, `accept_signatory`; `tacsim/src/port/market.rs` `PortVenueGate::signatory`);
    P07's funded balance (`board.rs` `BID_FUNDS_LINE`, `FUNDS_BOARD_ROLE` = "receiving",
    `house_funds`); the burn-and-reserve gauge in the tank rooms (`ship.rs` `BURN_GAUGE`,
    `BURN_GAUGE_NONE`, `insert_burn_gauge`, `burn_landed`; `gauges.rs` `single_qty`), the reserve
    reading the tank itself once the burn's ticket exists.
19. **Hygiene**: `put_ashore` moves first and moves back on a refused unplace (`ship.rs`; L12);
    `ephemeris.rs:9` rewrapped; stale words in `tests/lane.rs`, `e7r2.rs` (renamed
    `…registers_no_seventh_namespace`), `port_tick.rs` `register`'s doc, `def.rs`
    `PORT_MINT_CAP_MAJOR`, `mush-config/src/config.rs:924`, `tests/content.rs`, `orders.rs:992`,
    `commands.rs` `+coop`'s writing switches, `coop::row_of` private, `party_cap`'s comment; every
    rustdoc link warning `cargo doc` printed in the touched crates (the three duplicated outer
    module docs in `tacsim/src/lib.rs` removed); the two cosmetic literal newlines plus a third
    (`travel.rs:965`); `fly_to_stickney` walks P01 to P07.
20. **Docs**: the §10 amendments as "As built" paragraphs in docs 06 §7.5, 08a (the live-seams
    ledger), 10 (T1, the C4 row, §18.1), 10b (the issue/retire note withdrawn; the buyout row), 11
    (the 2320s row, §4.3, §4.5, §4.8, §4.10, §5, §7), 14 (§2.6, §3.3),
    14-spatial-implementation-plan (§3.8, the D-14-35 row), 14b §3, 15 (§3.4, §3.5), 16c §2.4;
    `mushd.toml.example` and `deploy/instance/mushd.toml.tmpl` drop the WoD words;
    `docs/help/ships.md:28-30`; `docs/ops.md` (the retention section, the Markers line, the
    trade-tape and reference-drift sentences, Stickney v4). Doc 18: §5's IC13 row landed at
    `94679739`, the IC14 row added — unmarked, because a commit cannot carry its own hash; the next
    docs pass marks it landed at this commit, as this cut marked IC13's — §8 Q-18-66..70 with IC13's
    defaults, §9 risk 19's as-built note and the new risk 20.
21. **Tests**: 87 new tests across `mush-core` (3), `mush-inventory` (9, `tests/ic14.rs` new),
    `mush-market` (16), `mush-economy` (1), `mush-atlas` (2), `mush-sim` (4), `tacsim` lib (17) and
    integration (29), `mush-server` (7); the rows they prove are cited in the items above. Three
    failed at base as the brief required (the dropped port entry's clerk, the seed survives a
    reboot, the anchored exit update).

### Deferred, and why

**Lead rulings taken, each as given** (the brief §4; recorded here, not chosen):

- **L1** — retention is durable, per source endpoint, floored by real-clock age; the mirror's
  fail-closed rule stands; a read-side horizon not taken.
- **L2** — the queue-full seam counts the noted set plus this outbox's deferreds; its own text; the
  executor defers without counting.
- **L3** — `restage_deliver` reads the marker before the cutoff.
- **L4** — the healed fold runs on the first sweep; the crash window between boot and that sweep is
  recorded, not closed.
- **L5** — an unconfirmed receipt is still treated as moved.
- **L6** — the seller's dispute refused through a game-neutral `DisputeGuard`.
- **L7** — an overflowing gross folds nothing.
- **L8** — `Listing.draw` optional on the wire: the market's boot skips an undecodable listing, so a
  required field would drop dev1's kept listings.
- **L8b** — `Voyage.told` required: voyage documents are slice-only and a defaulted 0 would re-tell
  old grants.
- **L9** — `owner_home` never answers a hull's hold.
- **L10** — the Return arm converges a Revoked charter with a live lease; the walk is over the hull
  rows.
- **L11** — six relet attempts under a doubling backoff, then IC13's outcome; a still-Offered lease
  is never reused because `relet` ends the half-made lease on every failure; `@coop/unload` not
  built.
- **L12** — `put_ashore` moves, then unplaces, and moves back.
- **L13** — failure paths by test modules, never a production switch; an unreachable one reported.
- **L14** — the Option wire claim corrected here, not made strict.
- **L15** — the lapse waits while any prepared action owns the typed store.
- **L16** — `Step::Held`.
- **L17** — an unreadable plan is treated as filed at every gate, counted departing, never lent,
  paged once — and never turns the Hulls line Missing.
- **L18** — over the rail the tenths round up.
- **L19** — `HOLD_CLEARED`.
- **L20** — `told` makes the grant durable.
- **L21** — `ORDER_RESTAGE_SECS` = `SWEEP_PERIOD_SECS` + 30: the draw receipt is market-owned typed
  state that owner scope refuses to the tacsim context, so the read seam cannot be built.
- **L22** — `HELD_SHORT_FOR_DEPART` follows §4.4's wording.
- **L23** — `REFUSED_RETRY` reached by a real staff act, `@org/status #kasei=suspended`;
  `INTAKE_TAKEN` stays unit-only because no staff act on the shipped packs exhausts a 4,000 t bid
  against a 100 t hold.
- **L24** — the clerk-row bound at load and the clerk's skip, both.
- **L25** — the per-entry case tested first; the fix was needed, so the clerk reads the content
  latch per entry.
- **L26** — Stickney v4 deletes three clauses and nothing else.
- **L27** — the small presentation residuals built here; the departure and berth boards to a content
  P-cut.
- **L28** — reproduce before delegating; both pairs were refused.
- **L29** — a race fix test-side or nothing.
- **L30** — the help reworded; the staff move stays ungated.
- **L31** — a full young ring refuses the move.
- **L32** — an unreadable re-stage ends in `HELD_UNCONFIRMED` at the second deadline.

**Builder choices inside them:**

- the holder's staged release lifts the hold-read backoff for ten sweeps (`release_staged`) so IC8's
  pinned next-sweep deletion still holds;
- the markers and incomplete-mirror lines are tested over the pure builders;
- the relet failure is injected by freezing the Trust's account (so no lease is minted; the arm's
  bound is proven from its own record each fire);
- the clerk's uncarried-rows warning is once per port;
- E1's "placed" map is rebuilt from the pack's `roomgeo` records (the latch is out of the slot
  during an apply);
- E2 reads both `Container` and `SpatialInterior` backings;
- the burn gauge's reserve is the tank itself once the burn's ticket exists (otherwise it would read
  0 between the debit and the launch);
- E8's "non-port line unchanged" check lives in `ic7.rs` through a gate that names no signatory;
- A16 asserts 60 lower (the brief's 40 was an arithmetic slip).

**Owner-critical defaults taken and reported** (do not read them as rulings):

- **O7 / proposed Q-18-74 — E7c2 owner decision D1 is weakened past the retention horizon.** The
  applied marker stays the reboot-surviving replay guard for at least one real day and at least
  the newest 32 per endpoint; a request replayed after both is no longer refused by it; a ring of
  128 young markers refuses the next move from that endpoint. The horizon (86,400 s) and the caps
  (32 / 128) are the planner's; the alternative, no eviction, keeps risk 19's world-wide cliff.
- **O8 / proposed Q-18-72 — the `Markers` line never gates**, a departure from Q-18-60's shipped
  default that every line but the sky gates `+coop/enrol`; only an actually incomplete mirror (the
  `InventoryFacts` line) closes enrolment.
- **O9 / proposed Q-18-73 — an unreadable sailing plan does not gate enrolment world-wide**; it
  keeps that hull out of the pool and pages. The stricter reading (the Hulls line Missing) is the
  owner's.
- **O4 / proposed Q-18-71 — a failed relet retries six times under a doubling backoff (2 min to 1
  h), then revokes as IC13 built it**; the pilot is told `RELET_PENDING` once and the plan waits
  under `HELD_RELET`; `@coop/unload` was not built.
- **O2 — Q-18-64's default changed**: a seller cannot dispute their own sale into a bid (staff
  `@contract/resolve` and admin disputes unchanged).
- **O3 — Q-18-51's default changed**: an explicit consumable buy holds under `HELD_SHORT_FOR_DEPART`
  only when the later consumable orders before the next depart would still leave it short.
- **O1 — every new player-facing text is the planner's wording** (Q-18-65): `SELL_QUEUE_FULL`,
  `SELL_REMEDY`, `DELIVER_LANDED`, `DELIVER_UNCONFIRMED`, `SALE_DISPUTE_REFUSED`, `HOLD_CLEARED`,
  `HOLD_PAPERS_DARK`, `RELET_PENDING`, `HELD_RELET`, `InventoryError::MarkerRingFull`'s line,
  `ACCEPT_SIGNS`, `BID_FUNDS_LINE`, `BURN_GAUGE`, `BURN_GAUGE_NONE`, the Markers and InventoryFacts
  readiness texts, the two Unknown page texts, the ships.md sentence. On that sentence: "your own
  moves out of the hold refuse until she docks" implies a docked pilot may move goods out of the
  hold; the story test pins the one-endpoint rule refusing it docked too — the owner's text pass
  should drop "until she docks".
- **O5 — Q-18-47 placed**: the accept-line signatory, P07's funded balance and the
  Engineering/Bridge burn line built here; the C14/P05 departure boards and the C01/P01 berth boards
  go to a content P-cut.
- **O6 — Stickney v4** strikes the three clauses only; 18a is untouched, so 18a §0.2 still names
  them until the owner's next 18a pass.
- **O10 / row 123 — the marker rings of endpoints that stop writing are never pruned**: the
  remaining bound of risk 19; no P-cut owns it.
- Proposed Q-18-71..74 are **not** added to §8 in this cut (the lead note confines §8 to
  Q-18-66..70); they are numbered in §9 risk 20 and here. The next cut-raised question is Q-18-75.

**Left, with the reason:**

- **Row 32's residues untested**: `RELEASE_RETRYING`, `REVOKED_RETRYING`, `BUYOUT_HELD`, the
  buyout's `end_lease` `Err` path and the Trust-acceptance residue. The only production-root
  injection (an `Abandoned` asset-lifecycle document) froze the economy at boot, and L13 forbids a
  production switch; `recall_outcome`'s unit test pins the Retrying/Err mapping. Likewise `relet`'s
  half-made-lease end arm is unreachable: `check_offerable` refuses every account and asset staff
  act at the offer, and the pilot's acceptance has no consideration, so no lease is minted that then
  fails.
- **The stock clerk's skip is boot-scoped and consults `authoritative_pending` only**: a healthy row
  whose third mint is in writer custody at the decision fire is skipped for the boot (its lot
  minted, not dissolved or listed until a reboot; one warn line). A re-read of the receipt before
  honouring `stock_skipped`, or an `authoritative_commit_pending` check, is a small later item.
- **`MAX_RELET_BACKOFF_FIRES`'s doc overstates**: with six attempts the longest gap is
  `relet_backoff(5)` = 384 fires; the cap is a ceiling the schedule does not reach. `relet_attempts`
  is RAM, so a reboot resets the count (bounded by reboots).
- **The staff settle of a sale disputed past the horizon** is untested (the refund and lapse paths
  are); settling such a sale may need to be done by hand after `@inv/show` on the house's stores.
- **The `world_task` fix is a fixed yield count**, deterministic on the current-thread runtime
  today; it observes no close signal.
- **The mirror test's name** ("stays complete") holds after the rings drain; at its mid-point the
  mirror is incomplete, which is the documented bound.
- **Row 67 (L14), correcting IC13's note**: the IC13 note's "strict wire" claim for `Enrolment` is
  wrong as to the three `Option` fields `buyout`, `ended_at` and `ended`, which deserialize to
  `None` when absent; `leased_at` is required and `Enrolment::check()` refuses every
  status-incoherent omission, so no behaviour changes.
- **Owner rows 89–101, 107 and 123 of the brief's table** left for the owner (the hold-backed path's
  Fact gate Q-18-42, org renames, `+ic`'s landing room, Q-18-32..41, the Dray v2 pack note, Q-18-60,
  the two render readings, `7.00cr`, the reference re-read Q-18-50, the hand-played smoke at rate 24
  — no Redis on the build machine — Q-18-66..70, the origin remote, a bring-her-home verb /
  `@coop/unload`, the unpruned rings).
- **Doc 18 §5's IC14 row is unmarked in this commit**: the brief asked the committer to mark it
  landed with the IC14 hash, which a commit cannot carry in itself, and the committer's edits under
  docs/design are confined to this note; the next docs pass marks it landed at this commit's hash.
- **Not in this cut**: doc 17's F3 row (not in this run's allow list); doc 18 §10's two stale
  sentences (`:3527` the `mortal` default — dispositioned edge, the single-game cleanup; `:3530` the
  `18-cut-notes.md:109-110` re-point — done in IC12's note), carried by risk 20; the Draw accept's
  full-queue exposure (row 124, edge: the buyer's escrow waits for the re-stage or the lapse refund;
  no money is lost); `INTAKE_TAKEN` at the unit level; every row the table marks **edge** or
  **P-cut** (P-A, P-H, P5, P-G/P-F, P2, the content P-cut).

### Pins re-blessed

- **`neighborhood.stickney_yards` content digest**
  `df2e5af11f18c4a5c71bea9e0b5b187b7cdb97c037aecc6b433b9ee3a0d1b2eb` (v3) →
  `f1bdc803c5adca043d576013fb5c94bc2b4b6f0434e91f82fe123fcbab2f16f6` (v4), in the pack's own
  `digest` field, `tacsim/tests/stations.rs` `PINNED` and `station_version` (3 → 4), and
  `mush-server/tests/boot_parity.rs:691`. Reason: row 50 / E6, the three verb meta-text clauses
  struck; `stickney_v3_json` rebuilds v3 to its old pin so the change is proven to be exactly those
  clauses; `old_station_json` restores them for the kept-world test (Stickney 2 → 4). No other
  digest moved: lane v3, Dray v2, goods v1, Ceres v2, the starter and region packs are unchanged
  (`mush-atlas/tests/starter.rs`, `tacsim/tests/content.rs`, `lane.rs` all green unmoved).
- **`crates/mush-contract-tests/module-ns-owners.toml`**: two new `[[delegation]]` rows, `tacsim` →
  `content` and `tacsim` → `atlas`, declared by tacsim. Reason: row 48 / E4 (L28): the reproduction
  `ic14_a_staff_seed_survives_a_fold_and_reboot` showed exactly these pairs refused at base and the
  corridor's latch row lost across a fold and reboot; `seed::apply` drives
  `mush_content::apply_pack` and the atlas appliers under tacsim's identity.
- **Existing assertions changed, each as the brief directs (old → new)**:
  - `mush-market/tests/ic10.rs` the dropped-leg re-stage after a landing (re-staged and refused at
    prepare by the marker → `+market/accept` answers `DELIVER_LANDED` with nothing re-staged; A5;
    the prepare-side replay guard stays pinned at `ic10.rs:1453`) and the disputed-sale test's
    disputer (`r.pat` → GOD; A6); the ic10 rig gains `.with_dispute_guard(SaleDisputeGuard)` and
    `Opts.bid_ttl_secs` (default 86,400, every existing test unchanged).
  - `mush-market/tests/e7r2.rs` the `Listing` literal gains `draw: Some(false)`.
  - `mush-inventory/tests/ic10.rs` three `transfer_marked` calls gain `T_WALL`.
  - `mush-economy/tests/e4b1.rs` three stale-page tests ("any escalation" → `SettlementRefused` +
    the contract subject + the in-force / escrowed-draft sentence; A15, stronger).
  - `tacsim/src/port/coop.rs` `ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky`
    (`[Binding; 15]` → 16; `gated(b) == (b != Sky)` → `!(Sky | Markers)`; A3).
  - `tacsim/tests/ports.rs` test 8 gains the `markers:` line and the moving-sky test asserts
    `sky_pages() == 0/1/1` (was a bool latch; C8, stronger).
  - `tacsim/tests/sale.rs` and `voyage.rs` `HOLD_UNDER_WAY` → `HOLD_CLEARED` for a docked cleared
    hull (C5; the under-way case still pinned at `commerce.rs:1342`); `fly_to_stickney` walks
    instead of teleporting.
  - `tacsim/src/port/executor.rs` the LFP unit test's step 400 t → 150 t with the `greater` case's
    aboard figures re-derived (D11, both fixed points over the same 100 t of cargo).
  - `tacsim/tests/coop.rs` the two IC6 gauge blocks gain `BURN_GAUGE_NONE` under the tank line
    (E10).
  - `tacsim/tests/commerce.rs` the two P07 board tests go 2 → 3 lines with the funds line (E9).
  - `tacsim/tests/orders.rs` the status-line test also renders from the Bridge (D11, stronger).
  - `tacsim/tests/stations.rs` `names_a_verb` now covers every Stickney detail and the kept-world
    test takes Stickney 2 → 4 (E6, stronger).
  - `mush-server/tests/ic_buyout_dormancy.rs` test 1 gains the exact graduated hulls line (B7,
    stronger).
  - `mush-server/src/world_task.rs` the race test gains sixteen yields and an assertion (E5; nothing
    weakened).
  - `mush-inventory/tests/e7r2.rs` `…registers_no_sixth_namespace` →
    `…registers_no_seventh_namespace` (D12).
  - `mush-market/tests/ic8.rs` `ic8_a_lapsed_hold_listing_survives…` was loosened mid-build and
    restored: no net change. `e7c2.rs`'s truncation test is untouched.
- **Run and unchanged**: the panic budgets (no new `unwrap`/`expect`/`panic!`/indexing in non-test
  code; `no_crate_exceeds_its_production_panic_budget` green), `legacy-handlers.toml` (baseline 97;
  no new verb), `help-commands-allowlist.toml` (T-45), `matcher-call-sites.toml`, the
  restriction-lints census, the CI gated-test inventory (a renamed test is not a new binary;
  `ic14.rs` is a new mush-inventory test binary the sweep ran), `state-slot-owners.toml` and
  `typed-state-lens.toml` (the new `inventory_marker_ring` family needed no re-bless;
  `production_action_migrations`, `authoritative_boot` and `boot_parity` green), the
  composition-edge pin (`with_dispute_guard` adds no Cargo edge; not re-blessed, as the brief
  forbade), every offer-parameter golden (none pinned the fixed parameters byte for byte).

### How it was verified

On Windows at the working tree over `94679739` with the cut uncommitted (98 files changed, plus the
new `crates/mush-inventory/tests/ic14.rs`), the final gate's 29 steps all exited 0 on the first run
— no reruns, no LNK1104, no hung binary, the stall watcher never fired:

```
cargo fmt --all -- --check                                        (no output; exit 0)
cargo clippy --workspace --all-targets -- -D warnings             Finished `dev` profile … in 45.96s (exit 0, no warnings)
cargo test -p mush-core --lib                                     400 passed
cargo test -p mush-inventory                                      1, 65, 16, 4, 8 passed (lib, e7c2, e7r2, ic10, ic14)
cargo test -p mush-market                                         30, 21, 5, 21, 22, 11, 2, 1 passed (1 ignored)
cargo test -p mush-economy --lib                                  37 passed
cargo test -p mush-economy --test e3b --test e4b1 --test e4b2 --test e4c   37; 85; 117; 44 passed
cargo test -p tacsim --lib                                        943 passed
cargo test -p tacsim --test ports --test sale --test commerce --test coop --test orders   24; 23; 32; 21; 16 passed
cargo test -p mush-contract-tests                                 29 binaries, every one ok (31/6/5/6/3/2/3/1/4/2/1/4/2/2/3/1/7/4/1/11/6+8 ignored/3/2/4/5/1/6/1/0 passed)
cargo test -p mush-server --test ic_slice_loop --test production_action_migrations --test authoritative_boot --test boot_parity   1; 18; 6; 3 passed
cargo test -p mush-housing --no-fail-fast                         1, 54, 81, 1 passed
cargo test -p mush-campaign --no-fail-fast                        54, 58, 15, 85, 28, 5 passed
cargo test -p tacsim --test coop --test stations --test port_tick --test voyage   23; 16; 20; 24 passed
cargo test -p mush-server --test ic_buyout_dormancy --test ic_slice_loop   13; 6 passed
cargo test -p tacsim --test voyage --test port_tick --test ports --test coop --test orders   23; 32; 16; 21; 24 passed
cargo test -p tacsim --test orders --test port_tick --test ports --test commerce --test sale --test lane --test content   24; 29; 10; 32; 16; 21; 16 passed
cargo test -p mush-inventory --test e7r2                          16 passed
cargo test -p mush-config --lib                                   112 passed
cargo doc -p tacsim --no-deps                                     Finished … in 1.72s (no rustdoc warnings)
cargo test -p mush-content                                        11; 8 passed
cargo test -p mush-atlas                                          17 binaries, all ok (51/5/5/9/6/5/8/13/4/3/6/2/5/21/14/16/0)
cargo test -p mush-sim --no-fail-fast                             15 binaries, all ok (37/11/5/18/3/34/68/32/19/2/6/15/31/24/0)
cargo test -p tacsim --test lane --test s9_corridor --test stations --test commerce --test ports --test coop --test voyage --test content   24; 29; 23; 10; 21; 6; 20; 24 passed
cargo test -p mush-server --lib world_task::tests::what_still_waits_when_the_loop_ends_is_answered_not_dropped   1 passed (and 30 of 30 alone with --test-threads=1 in the builder's run)
cargo test -p mush-server --test ic_slice_loop --test ic_buyout_dormancy --test boot_parity   18; 13; 6 passed
cargo test -p mush-contract-tests                                 (again) every binary ok
CARGO_BUILD_JOBS=8 cargo test --workspace --exclude mush-bot --no-fail-fast
    exit 0 in 37 min (build 10m10s); 401 result lines, all ok; 7609 passed, 0 failed, 92 ignored
cargo test -p mush-bot                                            exit 0 in 10 min; lib 469; arena_e2e 9 (206 s); e2e 49; playtesting 9; every other binary ok
```

Load-sensitive suites passed inside the saturated sweep: `world_task::tests::what_still_waits…`,
`arena_story two_pilots_walk_in_fight_and_one_tank_is_destroyed` (over 60 s, then passed), `mush-sim
journey::tests::e7c6_b2_*`, `ipc_authoritative_action`, `two_process_e2e`, `scripting_e2e`. The one
panic in the sweep log is the WS01-S8 injected world-turn fault (`process_fault.rs:197`), whose test
passed; no flake was identified.

Three tests were shown failing at base first, as the brief required:
`ic14_a_dropped_port_entrys_clerk_idles_while_the_lane_stays_latched`,
`ic14_a_staff_seed_survives_a_fold_and_reboot` (refused pairs `[(tacsim, atlas), (tacsim,
content)]`, the latch row `None` after the reboot) and
`ic14_an_update_row_into_a_hull_room_refuses_the_pack_before_any_retire` ("STOPPED PART WAY … What
was made is latched"). The fixer's mutation checks, each reverted: `hull_is_free` without
`voyage_blocks` fails `ic14_a_free_hull_whose_sailing_plan_does_not_read_is_never_lent`;
`launch_verify` without `mark_told` fails `ic14_a_grant_told_live_is_not_retold_after_a_reboot`;
`goods_moved` with the horizon guard disabled fails
`ic14_a_refund_past_the_marker_horizon_never_restores_the_bid`. `cargo doc` is clean for tacsim,
mush-market, mush-inventory, mush-atlas, mush-economy, mush-core, mush-sim and mush-content. Every
modified file is LF (the builder's helper had converted 34 to CRLF mid-build; all converted back and
checked).

Three review lenses on the uncommitted tree (correctness, conformance with a 68-item checklist,
tests): one must-fix each from correctness (F1, the disputed sale past the horizon) and tests (T1,
the vacuous `hull_is_free` assertion), twelve should-fix and notes across them. The fixer fixed
every finding but one and rebutted that one in part (conformance F2 / tests T2: a lease that is
minted and then fails cannot be reached on the production root — `check_offerable` refuses at the
offer — so the bound is proven from the arm's own record each fire instead). The judge re-read the
whole diff and the new file and verified each disposition against the code; the notes left standing
are recorded under "Deferred, and why". Verdict: ship with notes.

### Surface for the next cuts

Public items later cuts (the P-cuts) build on:

- **mush-core** (`crates/mush-core/src/{output,cmd,action_state}.rs`): `pub fn
  Outbox::authoritative_queue_full(&self) -> bool` (distinct actors in the noted set ∪ this outbox's
  `Deferred::AuthoritativeSubmit` actors `>= engine::AUTHORITATIVE_SUBMIT_QUEUE_MAX`; `false`
  outside a note save for the outbox's own deferreds); `pub fn Ctx::authoritative_queue_full(&self)
  -> bool`; `pub fn ModuleActionState::commit_pending(&self) -> bool` (the store's `has_prepared()`,
  `false` when unavailable); `pub fn Ctx::authoritative_commit_pending(&self) -> bool`.
- **mush-inventory** (re-exported from `lib.rs`): `pub const MAX_RETAINED_MARKERS_PER_ENDPOINT:
  usize = 32`, `MARKER_RETENTION_SECS: i64 = 86_400`, `MAX_MARKER_RING: usize = 128`,
  `MAX_RING_EVICTIONS_PER_PUSH: usize = 8`, `MAX_MARKER_ID_BYTES: usize = 96`; `pub struct RingEntry
  { pub key: SmolStr, pub wall: i64 }`; `pub struct MarkerRing(Vec<RingEntry>)` (`Serialize`,
  checked `Deserialize`; `new(Vec<RingEntry>) -> Result<Self, InventoryRecordError>`, `const fn
  empty()`, `entries()`, `len()`, `is_empty()`, `push(&self, RingEntry, now_wall: i64) ->
  Result<(MarkerRing, Vec<SmolStr>), InventoryRecordError>`); `InventoryRecordError::{MarkerRingFull
  { max }, MarkerRingKey, MarkerRingDuplicate}`; `InventoryError::MarkerRingFull` (Display: "this
  endpoint has recorded too many moves in the last day to record another; try again later"); `pub fn
  is_marker_id(&str) -> bool`; `InventoryFamilies.marker_ring: StateFamilyHandle<EndpointRef,
  MarkerRing>` (family id `inventory_marker_ring`, descriptors `inventory.marker_ring.v1` /
  `.decode_json.v1` / `.invariants.v1` / `.json.v1`, keyed `inventory.endpoint_id.v1`); `pub fn
  InventoryFamilies::retain_marker(&self, &mut ActionTxn, EndpointRef, &SmolStr, wall: i64) ->
  Result<(), InventoryError>`; `transfer_marked(.., now: &ClockStamp, wall: i64)` (new last
  argument); every marker-writing offer's `fixed_parameters` carry `"wall": World::now()` beside
  `"now"`, read by `pub(crate) provider::prepare_wall` (absent → `Unavailable` "the world clock is
  unavailable").
- **mush-market** (re-exported from `lib.rs`): texts `SELL_QUEUE_FULL`, `SELL_REMEDY` (`{quote}`),
  `DELIVER_LANDED`, `DELIVER_UNCONFIRMED`, `SALE_DISPUTE_REFUSED`, `ACCEPT_SIGNS` (" {signatory}
  signs for {shop}."); `pub struct SaleDisputeGuard` (impl `mush_economy::DisputeGuard`); `pub const
  HOLD_RECHECK_SWEEPS: u64 = 10`; `pub fn MarketStore::hold_reads(&self) -> u64`; `Listing.draw:
  Option<bool>` (`#[serde(default)]`; `Some(true)` only on a `Supply::Stock` listing, refused
  otherwise on the wire; `None` only on a pre-IC14 row, stamped by `put_listing` on its next write);
  `VenueGate::signatory(&self, &World, &MarketId, &PrincipalId) -> Option<String>` (default `None`);
  `restage_deliver` answers `Err(DELIVER_LANDED)` for a landed leg and `Err(DELIVER_UNCONFIRMED)`
  when the marker cannot be read, before the cutoff; `release_quote` reads an absent Deliver marker
  as unconfirmed (paged, never restored) once `World::now() >= PaidLot.accepted_at +
  MARKER_RETENTION_SECS`; `trade::settled_sale` is `pub(crate)`; `MarketStore` RAM fields
  `healed_sales`, `hold_checked`, `release_staged`, `sweeps`, `hold_reads` (`pub(crate)`).
- **mush-economy** (re-exported from `lib.rs`): `pub trait DisputeGuard: Send + Sync + Debug { fn
  refusal(&self, w: &World, contract: &EconomicContract, party: &PrincipalId) -> Option<&'static
  str>; }`; `pub fn EconomyModule::with_dispute_guard(self, Arc<dyn DisputeGuard>) -> EconomyModule`
  (composition-frozen; asked by `@contract/dispute` for a non-admin party after the party resolves
  and before the state check); `pub(crate) fn EconomyStore::dispute_guard()`; `owner_home` answers
  `None` for a `CustodyRef::SpatialInterior` vault.
- **mush-content / mush-atlas / mush-sim**: `EntryApplier::check`'s contract now allows a read-only
  world check when the refusal must take the whole pack because the default `update` retires before
  it creates; the atlas `check` refuses an exit row whose resolved end (`to`, and `from` for a
  two-way row) is anchored aboard a hull with "{id}: {authored} is a room aboard a moving hull; no
  exit leads into one"; `Transition::Members` refuses Limbo, the recovery room, the in-transit room
  ("{r} is Limbo" / "… the recovery room" / "… the in-transit room") and a vault-backing room ("{r}
  backs vault #{n}"), after the space-of-its-own check and before the domain-host check.
- **tacsim, readiness and the tick**: `port::readiness::Binding::Markers` (never gated;
  `gated(Markers) == false`), `pub(crate) fn markers_line(held: Option<usize>, max: usize) ->
  ReadinessLine`, `pub(crate) fn inventory_facts_line(provider: bool, complete: Option<bool>) ->
  ReadinessLine` (the `Markers` line is spliced two after the treasury, after `InventoryFacts`);
  `port_tick`: `Step::Held`, `PortFireStats.held: u32`, `PortTickState.{sky_pages: u32,
  marker_pages: u32, voyage_warned, voyage_pages: u32, stock_skipped: BTreeSet<String>,
  stock_attempts: BTreeMap<String, u8>, stock_uncarried: BTreeSet<CanonicalId>, relet_attempts:
  BTreeMap<ObjId, (u8, u64)>}` (RAM), `pub fn sky_pages(w) -> u32` (replaces `sky_paged`), `pub fn
  marker_pages(w) -> u32`, `pub fn voyage_pages(w) -> u32`, `pub fn recall_blocked_at(w, hull: u32)
  -> Option<u64>`, `pub fn relet_attempts(w, hull: ObjId) -> Option<(u8, u64)>`, `pub fn fires(w) ->
  u64`.
- **tacsim, voyage and market**: `port::voyage`: `Voyage.told: u32` (required on the wire, `<=
  last_seq`), `pub fn voyage_blocks(w, hull) -> bool`, `pub fn voyage_dark(w, hull) -> bool`,
  `pub(crate) fn tonnes_over(kg, ceiling_kg) -> String`, `pub(crate) fn launch_retell(ctx,
  &PortTickState)` (the first fire after a boot), `launch_hold` returns `Step::Held`;
  `port::market`: `HOLD_CLEARED`, `HOLD_PAPERS_DARK` (re-exported from `port`),
  `PortVenueGate::signatory` answers the port map's signatory NPC's name for an org seller.
- **tacsim, executor, buyout and recall**: `port::executor`: `pub const ORDER_RESTAGE_SECS: i64 =
  90`, `pub const HELD_RELET`, `pub(crate) enum Cover { Fill, Units(u64) }`, `pub(crate) fn
  later_cover(&Orders, index, &CanonicalId) -> Cover`; `port::buyout`: `pub const RELET_PENDING`,
  `MAX_RELET_ATTEMPTS: u8 = 6`, `MAX_RELET_BACKOFF_FIRES: u64 = 720`, `pub fn relet_backoff(n: u8)
  -> u64` (24, 48, 96, 192, 384, 720), `pub(crate) fn relet_pending(w, &Enrolment) -> bool`,
  `pub(crate) fn relet(ctx, &Enrolment, attempt: u8, give_up: bool) -> bool`; `port::recall`:
  `pub(crate) enum RecallOutcome { Return, Retry }`, `pub(crate) fn
  recall_outcome<E>(&Result<LeaseEnd, E>) -> RecallOutcome`, `recall_agents` walks the programme
  map's hull rows and offers a Revoked-with-live-lease hull and a relet-due hull.
- **tacsim, stock, ship, boards and definitions**: `port::stock`: `pub const MAX_STOCK_ATTEMPTS: u8
  = 3`, `pub(crate) fn entry_latched(w, &CanonicalId) -> bool`; `port::ship`: `pub const BURN_GAUGE`
  ("Burn for the filed plan: {burn} t; reserve after it: {reserve} t."), `BURN_GAUGE_NONE`,
  `put_ashore` moves first and moves back on a refused unplace; `port::board`: `pub const
  BID_FUNDS_LINE`, `FUNDS_BOARD_ROLE = "receiving"`; `port::gauges::single_qty` (`pub(crate)`);
  `port::def`: a stock row's `qty` in `1..=mush_market::MAX_QUANTITY`, a port's stock rows `<=
  MAX_CLERK_ROWS`.
- **tacsim, store, co-op and composition**: `TacsimStore::voyages_unreadable(&self) -> impl
  Iterator<Item = ObjId>`, `TacsimStore::route_in_world_at(&self, w, &CanonicalId, version: u32) ->
  Option<RouteDef>`; `coop::row_of` is private; `hull_is_free` also requires `!voyage_blocks`; the
  composition installs `with_dispute_guard(Arc::new(SaleDisputeGuard))` on every roster;
  `TacsimRuleset::register` declares `persist_delegation(tacsim, content)` and `(tacsim, atlas)`.
- **Content and pins**: `neighborhood.stickney_yards` v4, digest
  `f1bdc803c5adca043d576013fb5c94bc2b4b6f0434e91f82fe123fcbab2f16f6` (v3 was `df2e5af1…d1b2eb`;
  `tests/stations.rs` `STICKNEY_V3_DIGEST`, `STICKNEY_V3_CLAUSES`, `stickney_v3_json`,
  `restore_stickney_clauses`); `module-ns-owners.toml` carries the two tacsim delegations.
- **Test rigs later cuts may lean on**: `mush-inventory/tests/ic14.rs` (`boot`, `apply`, `go`,
  `ring`, `applied`, `land` through the provider); `mush-market/tests/ic10.rs` `Opts.bid_ttl_secs`,
  `fill_engine_queue`; `tacsim/tests/ports.rs` `install_markers`, `markers_line`;
  `tacsim/tests/commerce.rs` `kasei_funds_line`, `fires`; `tacsim/tests/voyage.rs` `grants_heard`,
  `look_aboard`; `mush-server/tests/ic_buyout_dormancy.rs` `write_docs`, `freeze_account`,
  `notices_of`, `trust_name`, `refunded_with_the_trust_frozen`; `mush-atlas/tests/anchored_exits.rs`
  `world_booting`, `block_version`.
- **Doc conventions**: this cut's owner items O4/O8/O9/O7 take the proposed numbers Q-18-71..Q-18-74
  in doc 18 §9 risk 20 and this note only; §8 does not yet list them, and the next docs pass adds
  them with doc 18 §5's IC14 row marked landed (the next cut-raised question is Q-18-75).

## P-B

**The Bids line gates on Missing alone** (doc 18 §5's post-loop list; §8 Q-18-60, the owner's
third-round ruling R10 of 2026-10-18), built 2026-10-18 on the playtest branch over `29230ff5`
(IC14 landed; dev1 redeployed from main without this cut, as ruled: a playtest cannot reach the
fifty round trips that drain Kasei). One commit, `e682a7ec`, not pushed by its builder; cut AQ (16c
`B-75`) is the next commit on the same branch and wrote this hash here and in §8.

### What landed

1. **The gate reads the `Bids` line on Missing alone** (`tacsim/src/port/coop.rs` `shuts`, beside
   `gated`; `enrol_gate` now finds the first line that `shuts`). `gated` is untouched — every
   binding but the sky and the markers still gates, the exhaustive match still has no catch-all —
   and `shuts` adds the state rule IC12 left implicit: a gated line in any state but Ready closes
   the desk, except a `Bids` line, which closes it only when Missing (a bid withdrawn by hand before
   its deadline, "no bid is posted"; or lapsed with its intake changed, "it waits for the next
   apply"). A `Bids` line that reads Pending — a house whose available balance is under one unit's
   price, "not buying until its capital settles"; or a lapsed bid the clerk re-posts, "the clerk
   posts it again" — is reported, counted by `Readiness::summary` ("Not ready: …" on
   `@tacsim/port status`), and leaves `+coop/enrol` open. Before this cut a funds-Pending house
   closed the Registry desk world-wide, every new pilot hearing `NOT_OPEN` and the gate logging at
   `info` (IC12 O2's reach, the review's F6). `shuts` is exported from `tacsim::port` beside
   `gated` and `enrol_gate`.
2. **The sales against such a house skip as they did before IC12** — no change was needed:
   `executor::sale_none_reason` still answers `HOUSE_CANNOT_PAY_SKIP` ("{house} cannot pay for
   {label} right now.") for a bid with `house_units == 0`, through `clamp_sell`'s house bound, and
   `ic11_an_unfunded_bid_skips` still pins the skipped order, no quote, no contract and the
   pilot's notice. The board's "not buying ({house})" line and P07's funded-balance line are
   likewise untouched.
3. **The port tick pages once a boot while a house cannot fund a tonne**
   (`tacsim/src/port_tick.rs` `page_unfunded_houses`, in `page_moving_sky`'s manner, called on
   every fire after the marker-headroom page; `PortTickState.funds_pages: u32`, RAM; `pub fn
   funds_pages(w) -> u32` the status read for tests beside `sky_pages` and `marker_pages`). The
   condition is one pure read in the readiness module, `readiness::unfunded_bids(w) ->
   Vec<(port entry, org, good)>`: every lane port with a present map, every vendor bid recorded
   there whose listing stands Wanted with intake left, **whose house's capital has settled**
   (every tranche Satisfied, the port's `Org` line Ready) and which still fails
   `board::funds_one` — the bids the report reads Pending for play's reasons. The settled-capital
   clause is the builder's one addition to the ruling, found by the tick test's first run: a
   fresh world posts its bids at apply, before the first economy sweep settles the houses'
   capital, so without it every fresh boot paged once on its first fire ("not buying until its
   capital settles" was written for exactly that window, beside an `Org` line that says why).
   The page is
   `tracing::error!` with a `runbook` field ("pay the house (`@econ/pay #<treasury
   account>=#<house account> <amount>`) or let players buy from it; enrolment stays open and the
   sales against it skip meanwhile"), the port, house, good and the count of unfunded bids; once a
   boot, however many fires and however many houses (the first names the page, `bids` counts the
   rest). Never a gate.
4. **Docs**: doc 18 §4.7's gate paragraph gains an "As built (P-B)" sentence, §5's post-loop list
   marks P-B landed, §8's Q-18-60 bullet is flipped to built (its hash named by cut AQ's commit);
   `docs/ops.md`'s "`@tacsim/port status` and the enrolment gate" says which bids line closes the
   desk, what a pending one means and the runbook for it.

### Deferred, and why

- **A `Flagged` Bids line** does not exist (the line reads Ready, Pending or Missing only), and
  `shuts` follows the ruling's words — Missing alone — rather than guess at one; the IC12 state
  walk pins Pending open and Flagged open for `Bids`, so a cut that adds a Flagged reading must
  decide here.
- **The page names one house per boot.** A second house drained later in the same boot is counted
  only if it was unfunded at the first page (the `bids` field); it does not page again. The
  readiness report still reads every pending line, and the per-house wording is the operator's
  to read there. A per-house once-a-boot set (as `paged` is per pool for stock drift) is the
  obvious widening if a lane ever carries more than one buying house.
- **The `info` log at the gate** (`+coop/enrol is closed by the readiness gate`) is unchanged for
  the lines that still close the desk; the review's "page at `error` when a funds-Pending bid
  closes enrolment" alternative is moot, since it no longer does.
- **The stocking helpers** in `tacsim/tests/{orders,commerce,coop}.rs` still wait for every line
  but the sky to read `[ready]` — a stricter condition than the gate, on purpose: a stocked fixture
  is one whose houses can pay. `ic11_an_unfunded_bid_skips` keeps draining Kasei after stocking
  for the same reason, now through the shared `leave_kasei_under_a_tonne`.
- **Doc 18's header** still reads "as built through IC12"; the docs pass that writes §10's flips
  owns it (the sibling note of 2026-10-18 says the same).

### Pins re-blessed

None. No content digest, no lane or station pack, no help topic and no player-facing sentence moved;
`NOT_OPEN`, `HOUSE_CANNOT_PAY_SKIP`, the board lines and every readiness sentence are byte for byte
IC14's. The one test that pinned the old reading, `coop.rs`'s
`ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky`, now walks `shuts` state by state
and expects `Bids` open on Pending and Flagged and shut on Missing; `orders.rs`'s
`ic11_an_unfunded_bid_skips` keeps its assertions and shares its drain.

### How it was verified

On Windows over `29230ff5` with the cut uncommitted, while another session's full-workspace test
build held the machine (one rustc of this crate died of `LLVM ERROR: out of memory` on the first
attempt and was rerun at `CARGO_BUILD_JOBS=2`; the clippy pass alone took 53 minutes of wall time):

```
cargo fmt -p tacsim --check                                       (no output; exit 0)
cargo clippy -p tacsim --all-targets -- -D warnings               exit 0, no warnings
cargo test -p tacsim --lib                                        1227 passed, 1 ignored
cargo test -p tacsim --test coop --test orders --test ports       23; 33; 21 passed
```

`cargo fmt --all --check` cannot run on this box (os error 206, command line too long; the sibling
docs session of 2026-10-18 hit the same), so the crate-scoped check stands in. The tick test was
shown failing first: on its first run the page had already fired inside `stocked()`, which is how
the settled-capital clause of `unfunded_bids` was found (item 3 above); with it, the funded world
never pages, the drained one pages on its first fire and not again. New tests: `readiness.rs`
`pb_a_funds_pending_bid_leaves_enrolment_open_and_a_missing_one_closes_it` (the two Pending texts
open, the two Missing texts shut, the gate naming the line) and `orders.rs`
`pb_the_tick_pages_once_a_boot_while_a_house_cannot_fund_a_tonne` (both Kasei bids `[pending]`
with "not buying until its capital settles", "Not ready" on the report, a second pilot chartered
through the open desk, `funds_pages` 0 → 1 on the first fire → 1 after three more). No hand-played
smoke: no Redis on this box, and the cut is post-deploy by the ruling.

### Surface for the next cuts

- **tacsim, readiness and the tick**: `pub fn port::coop::shuts(&ReadinessLine) -> bool`
  (re-exported from `tacsim::port` beside `gated` and `enrol_gate`; `gated` unchanged), `pub(crate)
  fn port::readiness::unfunded_bids(&World) -> Vec<(CanonicalId, CanonicalId, CanonicalId)>`,
  `PortTickState.funds_pages: u32` (RAM), `pub fn port_tick::funds_pages(&World) -> u32`.
- **Test rigs**: `tacsim/tests/orders.rs` `leave_kasei_under_a_tonne(&mut Rig) -> i64`.

## A.1

**The astronomy engine's data and model** ([doc 19](19-astronomy-engine.md) §3, cut A.1; doc 18
§5's post-loop cut P-A, sub-cut 1; D-19-1 … D-19-7), built 2026-10-18 on main over `3160cdd0`. One
commit with the Fable trailer, pushed when green. The owner's ask of the day: "an asteroid like
Pallas being so off-ecliptic means there are optimal times to trade there and times when it's
prohibitively expensive to reach it" — this cut gives the sky its third dimension and its motion;
the laws that turn motion into time and fuel are A.2's.

### What landed

1. **`kepler.rs`** (`tacsim::kepler`, new): `ElementRow` — one row of osculating Keplerian elements
   at a game-calendar epoch, eight integers of fixed scale (nano-AU, nano for `e`, micro-degrees,
   nano-degrees per day; `from_authored` / `authored` round-trip the decimals exactly, the bounds
   `e ≤ 0.99`, `0° ≤ i ≤ 180°`, a positive mean motion, a whole epoch in range); `julian_day(ic)` —
   the Julian Day of an IC instant's **calendar label** plus the day fraction (D-19-3: the game's
   2351-05-13 is the real 2351-05-13; the game calendar has no February 29, so every label is a
   Gregorian date); `propagate(row, jd)` — sixteen Newton steps on Kepler's equation, the rotation
   by ω, i, Ω, kilometres in `f64`; `table_position(rows, jd)` — the bracketing rows' propagations
   blended linearly in time, exact at a row, the nearest end row alone outside the table;
   `polar`. Five unit tests, among them Ceres's osculating row of 2351-05-13 landing on the slice's
   authored 2.866568 AU / 281.654° and the exact J2000 and lane-epoch Julian Days.
2. **`BodyDef.elements: Vec<ElementRow>`** (`pack.rs`): the wire gains `elements` as an array of
   `[epoch_ic, a_au, e, i_deg, node_deg, peri_deg, mean_anomaly_deg, mean_motion_deg_day]` rows,
   skipped when empty (no fixture's digest moves — `fixture_digests_unmoved_by_optional_sky_fields`
   still holds); `BodyDef::with_elements` decodes and validates: `1..=MAX_ELEMENT_ROWS` (512) rows
   strictly ascending by epoch, `epoch_ic` present, and **the consistency rule** (D-19-2): the table's
   position at `epoch_ic` within 5 ‰ of the authored `orbit_au` and 0.5° of
   `phase_deg_at_epoch`, `period_days` within 1 % of the nearest row's `360 / n` — a pack cannot
   carry two skies that disagree. `BodyDef` is no longer `Copy`. `is_keplerian()`.
3. **`Sky::Kepler(KeplerSky)`** (`sky.rs`; D-19-1, D-19-4, D-19-5): chosen by `Sky::from_pack`
   whenever any body carries a table, else `Sky::Circular` exactly as before. A circular body inside
   a Kepler sky keeps `Ephemeris::phase_mdeg_at`, and a circular **pair** keeps `separation_km` to
   the bit (`a_circular_pair_in_a_kepler_sky_keeps_the_slices_arithmetic`). A Keplerian body's
   position is its table's, rounded to the kilometre, with latitude; the rate keeps its meaning —
   `0` holds the body at its epoch position (now three-dimensional), `1000` is real motion, any
   other permille scales the elapsed interval about the body's epoch. `PosKm::distance_km` is the
   `u128` integer square root over the integer differences (one rounding path, at the position);
   `distance_km` takes it whenever either body is Keplerian. New: `Sky::ephemeris()`,
   `is_keplerian`, `heliocentric` (r, λ, β for the chart), `light_lag_s` (A.5's seam, whole seconds
   up), `KeplerSky::{circular, table, keplerian_bodies, position_f64}`. The revision digest keeps
   the slice's value for a circular sky and, for a Kepler sky, adds each table's own digest (taken
   once at construction under `tacsim.sky.table.v1`) to its body's entry, so `revision()` stays
   cheap for the chart's scans and a changed row changes the sky.
4. **`fetch_elements.py`** (`content/sky/`, new; D-19-7): stdlib only, one JSON object to stdout,
   `--cache <dir>` for the raw rows, `--write-golden` for the table. Horizons `ELEMENTS` by `TLIST`
   at January 1 and July 1 of 2340–2400 plus the lane's epoch (122 rows; chunks of forty, since
   Horizons answered 80 of 122 and said nothing), `VECTORS` every 91 days for the golden; the gas
   giants by barycentre (the planet-centre records end at 2200). It propagates exactly as the Rust
   does and measures the table against the golden rows per body. **The measurement** (the reason
   for the table, doc 19 §3.2): a single osculating set anchored at the epoch drifted 1.6–4.6 % for
   the asteroids over the window; a single fitted set 0.6–5.6 %; the semi-annual table with the
   blend holds mercury 0.056 ‰, venus 0.053, earth 1.014 (the Moon), mars 0.024, jupiter 0.003,
   saturn 0.013, ceres 0.070, pallas 0.054, juno 0.059, vesta 0.047, hygiea 0.072 ‰ of heliocentric
   distance — 241 rows each.
5. **`golden.json`** (`content/sky/`, new, 131 KB): the eleven bodies' Horizons positions, km, every
   91 days, each row keyed by the game calendar's IC second of its label.
6. **Lane v4** (`content/lane/ceres_phobos.json`, 171 KB; `LANE_VERSION` 4): twelve `body` entries
   — mercury, venus, earth, mars, jupiter, saturn, ceres, pallas, juno, vesta, hygiea from Horizons
   and **kirkwood on Ceres's rows trailing by 2° of mean anomaly** (D-19-6; doc 18 Q-18-36's
   recommended default, 0.0933 AU astern at the epoch, 0.07–0.13 AU through the window, always
   behind) — each with its 122-row table, the slice's circular numbers of ceres, mars and pallas
   **unchanged** (the Kepler sky reproduces them to the kilometre and the milli-degree at the epoch:
   `a1_sky_kepler_is_the_circular_sky_at_the_epoch`). The two routes' `base_s` re-timed to the
   three-dimensional chords at the epoch at 226 km/s: Ceres–Kirkwood **61,776 s** (0.0933 AU),
   Kirkwood–Mars **943,024 s** (1.4246 AU); lane v3's were 537,293 and 411,975 on the circular
   Kirkwood at 2.06 AU. Digest `7cd35e6fa0d7bbd629c9bcb3d1b1acb4d1b24aa099c5573dbd824be93fbb4d82`,
   pinned in `tests/lane.rs`; v1, v2 and v3 ledger rows stand (the in-place rule of IC3).
7. **`@corridor/show`** names the model: `…; the kepler sky at epoch 2351-05-13 (doc 19).` (a
   fixture corridor says `the circular sky`); `@tacsim/pack show` appends `; kepler, 122 element
   rows` to a body on a table. The readiness Sky line is **unchanged in form** this cut (its pins in
   `ports.rs` and the mush-server story test stand); the model word arrives with A.6, which touches
   the templates and that test anyway.
8. **Tests.** `tests/sky.rs` (new, seven): the lane sky is Keplerian and the fixtures are not;
   **`a1_sky_kepler_tracks_horizons`** (every golden row within the recorded per-body tolerance, in
   hundredths of a permille: mercury 6, venus 6, earth 110, mars 3, jupiter 1, saturn 2, ceres 8,
   pallas 6, juno 6, vesta 5, hygiea 8); the epoch identity; the integer distance is the Euclidean
   one for every pair at three instants and one mixed pair; the calendar by its label (Mars half a
   turn, Ceres 69° of true longitude in a game year — the equation of the centre spreads its 78° of
   mean motion); Kirkwood trails Ceres through the window; the epoch legs printed. `tests/lane.rs`:
   v4, twenty entries, twelve epochs, the new legs, `sky_circular_agrees_with_the_law_of_cosines`
   now on the lane with its tables stripped (the circular model is the e = i = 0 case) and asserting
   the shipped lane is `kepler`, the corridor wording, `@4`. `tests/passage.rs`: lane v4's figures.
   The one-past-the-shipped lane variants in `ports.rs` and `commerce.rs` move to `LANE_VERSION + 1`
   (v5; `18 unchanged` with the eight new bodies), the moving-sky tests strip Pallas's table with
   its epoch. `orders.rs` and the story test: 22 crew-days.

### The figures that moved (the latitude term, D-19-6)

The Kepler sky is the slice's sky at the epoch in longitude and distance; it differs by the
ecliptic latitude the slice flattened (Ceres β −4.73°, Mars −1.35°, Kirkwood −4.42°, Pallas
+14.07°) and by Kirkwood's new place. Ceres–Mars at the epoch: circular 214,534,234 km, Kepler
215,278,226 km (+0.35 %). The Dray's passage at the epoch sky, laden (lane v3 → v4): outbound
959,361 + 735,600 s (19.6 h at rate 24, 21 crew-days, 142,184 kg) → **110,305 + 1,683,834 s
(20.8 h, 22 crew-days, 142,188 kg — the extra crew-day weighs 4 kg)**; return 653,642 + 852,473 s (17.4 h, 18) → **1,496,209 + 98,014 s (18.5 h, 20)**; burns
41 t / 37 t unchanged (the coast law's burn is a fraction of mass). The 2° hop costs a whole
crew-day each way under the whole-days-per-leg rule (D-18-30) — the price of Kirkwood as Ceres's
outer harbour; A.2's flip law re-states every figure.

### Lead rulings taken (owner may override, each by data)

- **The table, not a fitted set** (D-19-2 as built): measured, two hundred times more accurate;
  150 KB in the lane pack against tacsim's 512 KiB bound.
- **Semi-annual rows**: yearly would hold the asteroids to 0.25 ‰ but Earth only to 7.6 ‰.
- **Kirkwood trails by 2°** (Q-19-2; Q-18-36's default): under four hours brimmed at the Dray's
  cruise; 1° or 3° is a lane edit.
- **The readiness Sky line is untouched** until A.6, so no pin in the story test moves twice.
- **No new crate**: the engine is `tacsim::kepler` and `tacsim::sky`; mush-sim stays sky-free.

### Verified

`cargo fmt`; `cargo clippy -p tacsim --all-targets -- -D warnings` clean; the whole tacsim suite
(47 binaries) and mush-server's `ic_slice_loop` (the production root, three restarts, now 22
crew-days) and `boot_parity`: **2,668 passed, 0 failed, 13 ignored** at -j 8 on the owner's box.
Pins re-blessed: the lane digest; `passage.rs` (the story load is 142,188 kg since the twenty-second
crew-day weighs 4 kg: 110,305 + 1,683,834 s out, 1,496,209 + 98,014 s back at 106,176 kg);
`voyage.rs`, `orders.rs`, `coop.rs`, `sale.rs`, `commerce.rs`, `ports.rs`, `hulls.rs`,
`stations.rs` (the one-past-the-shipped variants at `LANE_VERSION + 1`, the v2 rebuild stripping
A.1's sky, the plan counts with eight more bodies); `executor.rs`'s fill fixed point (37 t, 20
crew-days). The readiness Sky line and its pins are untouched.

### Deferred

- **A.2** the laws (Flip, the throttle, the keep-out, the policy, `via=`, Dray v3, lane v5);
  **A.3** `+chart`; **A.4** prices; **A.5** doc 15 R5 on `light_lag_s`; **A.6** the flip (templates
  at rate 1000, the readiness model word, the migration tests).
- Moons (Phobos, Luna, Titan) as bodies (Q-19-9); the window past 2400 (Q-19-8).
- The `Ephemeris` type's doc comment still describes itself as the sky; it is the circular model
  under the seam (one sentence in A.6's docs pass).

## A.2

**The laws** ([doc 19](19-astronomy-engine.md) §4, cut A.2; D-19-8 … D-19-13; doc 18 Q-18-35,
Q-18-40 and Q-18-41 ruled by doc 19 §2's defaults), built 2026-10-18 on main over `41e7259d` (cut
A.1). One commit with the Fable trailer, pushed when green. This is the cut where the moving sky
reaches the captain's wallet: under the slice's coast law the burn was a flat fraction of mass and
only time moved with the season; under the flip law both move, and a bad season costs reaction mass
the tank may not hold.

### What landed

1. **`DriveLaw::Flip { accel_mg, exhaust_kms }`** (`definitions/logistics.rs`; D-19-8): constant
   thrust, accelerate to the midpoint, flip, decelerate. `accel_mg` is the acceleration at the
   **rated maximum departure mass** (`dry_mass_t + payload_ceiling_kg`), so it is the acceleration
   every legal load is guaranteed and the drive is thrust-limited — a lighter hull is faster.
   Bounds `1..=MAX_ACCEL_MG` (10,000 mg) and `1..=MAX_EXHAUST_KMS` (light) in the validator; the
   law's word is `flip`; `Display` prints its two numbers. The Coast law is untouched.
2. **The planner** (`port/passage.rs`, rewritten around the law): `leg_seconds(drive, dry, payload,
   dist)` — Coast lades `ceil(dist / kms)` through mush-sim's step as before; Flip is
   `ceil(2√(d / a))` at `flip_accel_mps2` = `accel_mg · g₀ · (dry + ceiling) / (dry + payload) ·
   throttle / 1000`. `passage_burn(drive, dry, payload, legs)` — Coast the doctrine fraction; Flip
   the rocket equation over the passage's `Δv = Σ a·t`, whole tonnes up (`flip_profile` gives the
   acceleration, `Δv` and the longest leg's peak velocity for the verbs). **The throttle**
   (`HullDrive::throttle_permille`, `MIN_THROTTLE_PERMILLE` 250 ..= `MAX_THROTTLE_PERMILLE` 1000,
   `DEFAULT` 1000; D-19-9): half thrust is 41 % longer and 29 % cheaper; outside the band
   `PlanError::Throttle`. **The solar keep-out** (`KEEP_OUT_AU` 0.3; D-19-12; Q-18-40): `detour_km`
   takes a leg's chord when it clears the disc about the Sun (or when either end is inside it) and
   otherwise the shortest path around it — tangent, arc, tangent, in the plane of the two
   positions — and `leg_distance_km` is what the intercept flies; a cost, never a refusal.
   **`INTERCEPT_PASSES` 5** (D-19-11; the test shows each pass moving the leg no more than the
   one before and the sixth within a minute or 2 %). **`RoutePolicy::{Fastest, Cheapest}`**
   (D-19-13) with `word`/`parse`; `choose_route(…, policy, via)` keeps the best under the policy
   (cheapest = least `burn_t`, ties to the faster) over the listed routes that **call at `via`**
   before their destination (`PlanError::NoRouteVia`; `ViaReserved` retired). `PassagePlan` gains
   `throttle_permille` (`#[serde(default)]` = 1000, so every ticket from before the cut reads
   back), and the burn's prepare recomputes the plan's burn at the plan's own throttle.
3. **The verbs** (`port/voyage.rs`): `PassageOptions { throttle_permille, policy, via }` with
   `PassageOptions::for_order`; `+ship/plan` and `+ship/undock` take `throttle=<permille>`,
   `policy=fastest|cheapest` and `via=<place>` (a lane place by id or a prefix of its label,
   `match_waypoint`; `VIA_UNKNOWN`, `THROTTLE_USAGE`, `POLICY_USAGE`); `prepare_departure` takes
   the options, so a live undock and a queued depart fly the same way. The plan's burn line says
   the law's own sentence: the coast law's "Every tonne aboard … costs N kg of burn", or "Flip and
   burn at 0.54 m/s² (50 milli-g rated, throttle 1000‰): Δv 852 km/s, peak 437 km/s, exhaust 6000
   km/s; a farther port costs more."
4. **The orders** (`port/orders.rs`): `What::Depart` gains `throttle_permille: Option<u32>` and
   `via: Option<CanonicalId>` (serde defaults, so every filed sailing plan reads back);
   `+ship/order depart <port> … [via=<place>] [throttle=<permille>]`; the document's check bounds
   the throttle; `+ship/orders` prints `, via Kirkwood Depot, throttle 500‰`; the executor's
   planning core (`Planner::plan(…, &PassageOptions)`) flies the order's options for the fill
   fixed point, the dry run and the departure itself.
5. **The Dray v3** (`content/units/dray.json`; D-19-10): `"law": { "flip": { "accel_mg": 50,
   "exhaust_kms": 6000 } }`, the note rewritten within its 1,024 bytes, digest re-signed and
   pinned (`tests/content.rs`); lane v4 is untouched (its `base_s` are documentary since this cut:
   the planner reads them for no cross-body leg, and `tests/lane.rs` keeps them as the chord at the
   slice's 226 km/s).

### The figures (the flip law over lane v4 at the epoch sky; `tests/passage.rs`)

| Load | Coast (A.1) | **Flip (A.2)** |
|---|---|---|
| Outbound, 100 t cargo + 42 t fuel + 22 crew-days, 142,188 kg | 110,305 + 1,683,834 s (20.8 h at rate 24), 41 t, 22 crew-days | 321,547 + 1,256,313 s (18.3 h), **51 t**, 19 crew-days |
| Return, 106,176 kg | 1,496,209 + 98,014 s (18.5 h), 37 t, 20 crew-days | 1,195,660 + 306,023 s (17.4 h), **48 t**, 18 crew-days |

The 2° hop to Kirkwood takes 3.7 days under the flip law (a brachistochrone's short legs are slow)
and the Kirkwood–Mars chord 14.5; the burn is thirteen percent of departure mass at the epoch. The
slice's 42 t tank no longer covers an outbound, so **the story bunkers 53 t and 19 crew-days**: the
voyage rig's ticket at 153,176 kg flies 326,136 + 1,274,245 s (1,600,381; 18.5 IC days) for 52 t
with a tonne left, the queue's return fill is 50 t and 19 crew-days (49 t bought at Stickney, the
alloys cut to 17 t by funds where the slice bought 20), and IC14's short-buy scenario needs ten
tonnes of margin over the empty-tank burn where the coast law's 105 kg per tonne needed five — the
flip law charges about fifteen percent of the fuel's own mass. The plan line at that load reads
"Burn 52 t (tank 53 t; 1 t left). Flip and burn at 0.53 m/s² (50 milli-g rated, throttle 1000‰):
Δv 840 km/s, peak 335 km/s, exhaust 6000 km/s; a farther port costs more." Seasonality, the point of the
engine: at the best season (Ceres–Mars ≈ 1.24 AU) a brimmed hull runs about 14 h and 40 t, at the
worst (≈ 4.3 AU, the chord inside the keep-out) about 30 h and over 70 t against an 80 t tank
(doc 19 §4.3); `tests/passage.rs`'s half-synodic plan burns more than the epoch's.

### Lead rulings taken (owner may override, each by data)

- **The Dray's numbers** 50 mg / 6,000 km/s (Q-19-1): "slow, efficient" — a twentieth of a gravity,
  two percent of c; 70 mg or 10,000 km/s are a `dray.json` edit.
- **The throttle band** 250–1000 ‰ (Q-19-4); the policy on `+ship/plan`/`+ship/undock`, not on a
  queued depart (the queue flies the fastest route; a captain who wants the thrifty route plans it
  and files the throttle).
- **Direct routes stay out of the lane** this cut (the brief's lane v5 moves to A.3 with `+chart`,
  which is what compares them; the ports' lists are unchanged, Q-19-3's default).
- **Same-body legs** keep mush-sim's laden step under either law (an authored `base_s`).

### Verified

`cargo fmt --all -- --check`; `cargo clippy -p tacsim --all-targets -- -D warnings` clean (two
findings fixed: a negated float comparison and an `unwrap_err`); the whole tacsim suite green
(2,673 passed after the cascade re-pins), then the touched lib, passage and voyage suites again
with mush-server's `ic_slice_loop` (the story test now bunkers 53 t and 19 crew-days, its
one-tonne-short case 51 t against a 52 t burn) and `boot_parity`: 1,296 passed, 0 failed at -j 8.
Pins re-blessed: `passage.rs` (the flip figures), `voyage.rs` (the 53/19 story load, the ticket's
326,136 + 1,274,245 s and 153,176 kg, the rail overload at 192.6 t, the burn gauge), `orders.rs`
(the day's 53 t and 19 crew-days, the return's 50 t fill and 49 t bought, 17 t of alloys, the
short-buy margin), `coop.rs` (the gauges), `lane.rs` (the flip law in the drive table; the Dray
copy edits `accel_mg`), `content.rs` (Dray v3's digest and law), the executor's fill fixed point
(50 t, 19). A.1's readiness Sky line is still untouched.

### Deferred

- **A.3** `+chart`, `+chart <port>` (the best departure in the next IC year), `+chart/body`, the
  `+ship/plan` window line, lane v5's direct routes; **A.4** prices; **A.5** doc 15 R5 on
  `light_lag_s`; **A.6** the flip (templates at rate 1000, the readiness model word, the migration
  tests, the help's "later, the season" → now).
- The Engineering gauge and the help still describe the burn in the coast law's words where they
  do; A.6's docs pass.

## A.3

**`+chart` and the plan's window line** ([doc 19](19-astronomy-engine.md) §5, cut A.3; D-19-16),
built 2026-10-18 on main over `153a267a` (cut A.2). One commit with the Fable trailer, pushed when
green. This is the surface the owner asked for in words — "there are optimal times to trade there
and times when it's prohibitively expensive" — read as text at the 120-column floor; the web and
TUI panel is A.3b.

### What landed

1. **`port/chart.rs`** (new): the `+chart` verb, a pure read in `+credit`'s ReadOnly bracket
   (`commands.rs`), on the one sky seam and the one planner.
   - **`+chart`**: the header names the date, the sky's model and motion, and the vantage — the
     port the actor's charter is docked at, else the port whose domain host or gallery the actor
     stands in (the port map's rooms), else the lane's first port; without a charter the port's
     programme hull (the Dray) is planned **empty**, so a walker on the concourse reads the same
     sky a captain does. Then every lane body by heliocentric distance — `r` in AU, ecliptic
     longitude and latitude, the distance from here in AU and the one-way light lag — and, for
     each port this one clears for, the passage **now** at this load and empty (hours at the world
     rate, burn, crew-days) and **the direct chord beside the listed route** (its AU, hours and
     tonnes: the depot's detour priced, computed from the sky — no direct route entry is needed,
     so lane v5 is not cut).
   - **`+chart <port>`**: the **window** — `plan_passage` once per IC day over the next year
     (`WINDOW_DAYS` 365 evaluations, a few milliseconds): now, then the fastest day, the cheapest
     day (when it differs) and the worst, each dated, "in N IC days", with its hours and burn and
     the difference against now. Under the constant sky the line says every day is today.
   - **`+chart/body <body>`**: the body's row, its distance and light lag from here, and the
     osculating element row it moves on now (`a`, `e`, `i`, Ω, ω, `M`, the period; the table's
     row count and span); a circular body says so.
   - Refusals: `CHART_NO_LANE`, `CHART_NOWHERE`, `CHART_NO_ROUTE` (with the list),
     `CHART_NO_BODY` (with the list); TacSim off answers `NOT_ENABLED`.
2. **`+ship/plan`'s window line** (`voyage.rs` `plan_block`, after the Passage line; D-19-16):
   when the sky moves, the best departure in the next `PLAN_WINDOW_DAYS` (30) IC days against
   today's — "Window: the best departure in the next 30 IC days is 2351-06-02 (in 20 IC days):
   17 h 40 m and 48 t, 1 h 10 m and 4 t less than today." — or "today is the best departure";
   under the constant sky no line (every day is today, and the slice's pinned plan output is
   unchanged). The scan's core, `chart::best_day`, is pure and unit-tested; the basis is cloned
   per day with its `depart_ic` moved.
3. **Seams opened** (`voyage.rs`): `Basis` and its fields, `basis`, `plan_on`, `Destination`,
   `destinations`, `match_destination` and `occupants` are `pub(crate)` for the chart; nothing
   else moved. `docs/help/ships.md` carries the three `+chart` lines and the window line.
4. **Tests**: `tests/chart.rs` (four, on the production composition with the lane, as a walker):
   the sky table (twelve rows, Ceres "here", Pallas's light lag, the Stickney line with the empty
   figure and the direct chord); the constant sky's window and the refused port; the moving sky's
   fastest and worst days (the worst no shorter than the fastest); `+chart/body pallas` (latitude
   +14.07°, the 122-row table, `i 35.`) and the refused body. Unit tests for `best_day` and the
   formatters.

### Lead rulings taken

- **The direct comparison is computed, not authored**: the chord from the vantage's body to the
  destination's, through `leg_distance_km`, `leg_seconds` and `passage_burn`; lane v5's direct
  route entries are not needed for the chart and stay out (Q-19-3's default).
- **The window is one IC day at a time** (365 and 30 evaluations); a finer scan is a knob for
  later, not a need now.
- **The vantage falls back to the lane's first port** for a player standing nowhere the lane
  knows, so `+chart` always answers something on a lane server.

### Deferred

- **A.3b** the web cockpit panel and the TUI screen (top-down XY, z-exaggeration, ships under way
  along their tickets' frozen legs); **A.4** prices; **A.5** doc 15 R5 on `light_lag_s`; **A.6**
  the flip (templates at rate 1000, the readiness model word, the migration tests).
- Lane v5 (direct routes as routes a captain can fly) when the fiction wants them.

## A.6

**The flip** ([doc 19](19-astronomy-engine.md) §8, cut A.6; D-19-16; doc 18 Q-18-37 ruled by doc
19 §2's default), built 2026-10-18 on main. One commit with the Fable trailer, pushed when green.
The sky moves on the next deploy — and only then: nothing here touches a running world, and **no
deploy without the owner**.

### What landed

1. **The shipped templates say real motion** (`mushd.toml.example`,
   `deploy/instance/mushd.toml.tmpl`): `[tacsim.time.ephemeris] rate = 1000` with the stanza's
   comment rewritten — the sky's date is the IC date, which the lane's Horizons element tables
   (2340–2400) require; `0` is the first slice's constant sky and stays the Rust default, so
   every pinned figure in the tree is still the epoch's; the knob stays a permille (a slower sky
   is one line of config, Q-18-37's alternative). `shipped_configs.rs` parity held.
2. **The readiness Sky line names the model and the motion** (`port/readiness.rs`): `sky: moving
   (rate 1000, real motion), kepler, epoch 2351-05-13, rev {12 hex}` / `sky: constant (rate 0),
   kepler, epoch …`; a fixture lane says `circular`. Its pins in `tests/ports.rs` and the
   mush-server story test moved once, as A.1 and A.2 promised. The flagged form (a moving sky
   over a body without `epoch_ic`) and the once-per-boot page are unchanged.
3. **The migration tests stand as written** (`tests/voyage.rs`, since IC9/IC14):
   `journey_arrives_at_its_frozen_instant_across_a_reboot_with_a_new_sky` (a mid-passage journey
   keeps its frozen `leg_s` across a reboot into rate 1000) and
   `ticket_freezes_the_plan_across_a_sky_change` (a burned ticket's plan and `sky_rev` read back
   identical; a fresh plan under the new sky differs) — nothing to rewrite (D-19-16: in-flight
   journeys, burned tickets, orders and quotes all carry what they froze).
4. **Docs**: doc 10 §22 already superseded (A.1); doc 18 §4.8's readiness sentence and the
   `+ship/plan` help; doc 19 §9's A.6 row.
5. **A drive-by**: `mush-config`'s `shipped_configs.rs` pinned dev1's clock at rate 4 from the
   September epoch, while `deploy/dev1/instance.env` has run at rate 24 from `0987-04-20 06:00`
   since the 2026-10-18 deploy (commit `29230ff5`) — the test had been red since; its pin now reads
   the box as it is. The same test's "every shipped config says the sky rate explicitly" now
   expects 1000.

### Verified

`cargo fmt --all -- --check`; `cargo clippy -p tacsim --all-targets -- -D warnings` clean;
`tests/ports.rs` (21) and `tests/chart.rs` (4) green on the new line; `mush-config`'s
`shipped_configs.rs` 12 of 12 (both drive-bys); mush-server's `ic_slice_loop` and `boot_parity`
green with the moved prefix. The voyage migration tests ran green in A.2's and A.3's full sweeps
and are untouched here.

### What the owner decides

- **When to redeploy dev1.** The playtest world sits at the lane's epoch date (2351-05-13 on
  2026-10-19); the next deploy turns the sky on from there, so the trade run's season starts at
  the chord minimum and worsens over the following real weeks (Ceres–Mars opens about 0.05 AU a
  real day at rate 24; the full cycle is about 48 real days). A redeploy also takes lane v4 and
  the Dray v3 (the flip law: 53 t bunkered where the slice bunkered 42) — the figures in doc 18
  §2's story change under the pilot's feet, and `+ship/plan` says so.

### Deferred

- **A.3b** the chart panel; **A.4** prices; **A.5** doc 15 R5 on `light_lag_s`.

## A.4

**Prices — the reach premium** ([doc 19](19-astronomy-engine.md) §6, cut A.4; D-19-14; doc 18
Q-18-39 ruled by doc 19 §2's default; Q-19-5's caps), built 2026-10-18 on main. One commit with the
Fable trailer, pushed when green. Nothing here touches a running world until the next deploy, and **no
deploy without the owner**.

### What landed

1. **`crates/tacsim/src/port/price.rs`** — the pure arithmetic and the one read. `Reach { now_s,
   median_s }`; `premium_permille(k, cap, reach)` = `min(cap, k · (now / median − 1))`, never below
   zero, i128 arithmetic with the ratio in permille first; `priced(authored, premium)` =
   `authored × (1000 + premium) / 1000` in whole major units, floor, saturating. **Reach** for a good
   is `choose_route` (`Fastest`, no `via`) for the lane's Dray **empty with one soul at full
   throttle** over each supplier's listed routes to this venue, the least over the suppliers — a
   supplier is another lane port whose vendors **ask** the good and that lists a route here; both
   must have a body. The **synodic median** is the lower median of that reach sampled from now over
   the synodic period of the port's body and the supplier's (the longest over the suppliers):
   `Sky::period_days` (new; the table row nearest now, `360 / n`; the circular `period_mdays`
   otherwise), `synodic_days` (`1 / |1/P₁ − 1/P₂|`; `None` for the same orbit), `scan_days` (rounded
   up, `1..=MAX_SCAN_DAYS` 2,000 — Ceres–Kirkwood never closes and takes the cap, so does an unknown
   body), at most `MAX_SCAN_SAMPLES` 400 samples (Ceres–Mars's 1,162-day season every three days);
   **one sample under the constant sky** (rate 0), so every bid posts at its authored price and the
   pinned worlds are untouched. `bid_price_major(w, def, bid)` is the authored price when the port
   has no `premium` block or no reach.
2. **Read at posting** (the as-built deviation from §6's once-per-IC-day re-write, recorded there):
   the applier's bid loop (`vendor.rs`, "(e).8 The standing bids") and the stock clerk's re-post of a
   lapsed bid (`stock.rs` `bid_step`) both price the bid through `bid_price_major` — the posting path
   already carries the officer's signature (11 §2), and a bid's TTL is the cadence the board
   re-prices on; the season turns over years. `MarketStore` holds the posted price; the port map's
   `BidRow.price_major` stays the authored figure (the map is the authored record, as before). The
   median is computed at each posting, not cached per boot: at most 400 plans per bid.
3. **The port entry's block** (`def.rs`): `PortDef.premium: Option<ReachPremiumDef { k_permille,
   cap_permille }>`, serde default + skipped when absent, `deny_unknown_fields`, each
   `0..=MAX_PREMIUM_PERMILLE` (10,000) in `PortDef::check`; exported with the def types.
4. **Lane v5** (`content/lane/ceres_phobos.json`, `LANE_VERSION` 5): both ports gain
   `"premium": { "k_permille": 300, "cap_permille": 500 }` (Q-19-5's defaults) and nothing else moves;
   signed, digest `319aa8a0d273dc6980937d7b8ec7682eab25c37be964ceaaef687bff3562744e` pinned in
   `tests/lane.rs`. The suites' remaining lane-version literals (`@4`, `@5`, `Some(5)`,
   `lane_variant(5, …)` in `ports.rs`, `commerce.rs`, `lane.rs`) now read `LANE_VERSION` /
   `LANE_VERSION + 1`, so the next bump moves one constant and one digest; the commerce suite's v2
   rebuild drops `premium` as it drops `vendors`.
5. **Docs and help**: doc 19 §6 "As built", §9's A.4 row landed and an **A.4b** row (the emergency
   Wanted bids and `emergency_ratio`, `+chart <port>` naming the premium), §10 Q-19-5; README; the
   trade help's new paragraph ("A port's bids follow its season …", pointing at `+chart <port>`).

### Verified

- **`tests/price.rs`** (new, 4): both ports carry the block at 300 / 500; at rate 0 Kasei's metals
  bid posts 280 and the Exchange's alloys bid 520 with reach now = median; at rate 1000 every bid
  of the four (metals, volatiles, alloys, parts) posts exactly `priced(authored, premium(reach))` for
  the reach `reach_for` reports — at the test world's instant Stickney's reach 1,758,164 s against a
  median of 1,730,327 s (+4 ‰: 280 → 281, 200 → 200) and Ceres's 1,903,004 s against 1,796,755 s
  (+17 ‰: 520 → 528, 560 → 569) — never below the authored price, never above the cap; a port without
  the block posts authored prices; `k_permille 10001` is refused by `check`; Ceres's and Mars's
  periods from the tables (1,681.6 / 687 days within the asserted bands), their synodic period about
  1,162 days = the scan, Ceres–Kirkwood and an unknown body at the cap, the lower median.
- Unit tests in `price.rs` (3): the premium's cases (equal, better season, +80 % → 240 ‰, the cap,
  k 0, a zero median), `priced` (280 → 347 at 240 ‰; saturation), `synodic_days` and `median_of`.
- The suites that pin the lane: lib 1,248 passed / 0 failed (1 ignored); `chart` 4; `commerce` 24;
  `content` 29; `lane` 10; `ports` 21; `price` 4; `sale` 16 — all green.
- **The hull suite** (`tests/hulls.rs`, 15) had built its update variant at the literal version 4 ("one
  past the shipped v3" when IC8 wrote it; equal to the shipped v4 since A.1), which the bump to v5 made
  an older pack — three tests red; its `lane_v4` helper and the two "Applied @4" pins now read
  `LANE_VERSION`, and the suite is green.
- `cargo fmt --check` clean; `cargo clippy -p tacsim --all-targets -- -D warnings` clean; the whole
  tacsim suite green across two runs at this source state (the first stopped at the hull suite's three
  reds after 1,955 passed over the 24 suites before it; the second, `--no-fail-fast` after the fix,
  ran every suite from `content` on and the hull suite with 0 failed); mush-server `boot_parity` 19 and
  the story `ic_slice_loop` 6 green (the story's figures unchanged: the premium reads at posting under a
  constant sky, so every pinned world posts its authored prices); mush-config `shipped_configs` 12
  green.

### What the owner decides

- **The caps** (Q-19-5): 300 ‰ per unit of reach ratio, capped at 500 ‰ — a lane data edit on each
  port. At Ceres–Mars's season (reach about 15–27 h empty, median near 21 h) the worst day carries
  roughly +90 ‰ at k 300; a Pallas-like port's swing is the point and would run to the cap.
- **The baseline**: the synodic median as designed (the authored price is the mid-season price;
  good seasons pay no less). The alternative — the lane's epoch reach, the chord minimum, as the
  baseline, making the authored price the best-season floor and the swing about 2.5× larger — is a
  one-line change in `reach_for` and was not taken without the owner.

### Deferred

- **A.4b** the emergency Wanted bids (life-support goods at the cap when reach crosses
  `emergency_ratio`, funded from finite capital) and `+chart <port>` naming the premium; **A.3b**
  the chart panel.

## A.4b

**The emergency Wanted bids** ([doc 19](19-astronomy-engine.md) §6, cut A.4b; D-19-14; doc 18
Q-18-39's second half; Q-19-5's defaults), built 2026-10-19 on main. One commit with the Fable
trailer, pushed when green. Nothing here touches a running world until the next deploy, and **no
deploy without the owner**.

### What landed

1. **The port entry's block** (`port/def.rs`): `PortDef.emergency: Option<EmergencyDef { org,
   ratio_permille, goods, intake }>`, serde default and skipped when absent, `deny_unknown_fields`;
   `PortDef::check` refuses an emergency without a `premium` (the cap comes from it), a ratio
   outside `1000..=MAX_EMERGENCY_RATIO_PERMILLE` (100,000), no goods or more than
   `MAX_EMERGENCY_GOODS` (16) or one named twice, an intake outside `1..=mush_market::MAX_QUANTITY`,
   an `org` that is not one of the port's vendors, and an org that already asks or bids any of the
   goods (a storefront holds one line per good; `post_wanted` would refuse the duplicate);
   `check_ports` refuses a good the goods table does not know, as it does a vendor's. The types and
   bounds are exported with the def types.
2. **The clerk's emergency step** (`port/stock.rs` `emergency_step`, ahead of the stock steps:
   life support before stock). Once a real hour per port (`EMERGENCY_CHECK_SECS` 3,600 — one IC day
   at the shipped rate 24; `PortTickState.emergency_checked` by entry id, spent only when nothing
   was due) the clerk reads each emergency good's reach against its synodic median
   (`price::reach_for`, the A.4 reading). A good at or past the ratio (`emergency_crossed`:
   `now·1000 ≥ median·ratio`, integer arithmetic) with no standing emergency bid gets one: the org's
   vendor posts a Wanted for `intake` units at the premium's **cap** over the good's **reference**
   (`emergency_reference_major`: the port's highest authored ask or bid for the good, else the goods
   table's `reference_major`) — through the applier's own `wanted_spec`, as the clerk under
   `Fire::as_principal`, signed as every bid is — and the row goes into `PortMap.emergencies` (good →
   `BidRow`, the reference as `price_major`). A house that cannot fund one unit
   (`board::vendor_funds`, factored out of `funds_one`) posts nothing and is warned once a boot; a
   row gone before its recorded deadline (its intake taken) is not posted again until the deadline
   passes. A good whose reach fell back under the ratio while its bid stands has it withdrawn
   (`withdraw_listing` as the clerk) and the row forgotten. One act a fire, authored order; the
   swap's refusals map exactly as the bid re-post's (a clerk with a mint in flight defers). A good no
   other port supplies has no reach and no emergency — Ceres's volatiles, which nobody at Stickney
   asks. A port retire withdraws the emergency bids with the standing bids and forgets their rows.
3. **The readings** (`port/price.rs`): `EmergencyReading { good, reach, crossed, price_major }`,
   `emergency_readings(w, def)` (authored order; empty without the block or a premium),
   `reach_ratio_permille`; exported.
4. **`+chart <port>`** (`port/chart.rs` `demand_lines`): after the "now" line, the destination's
   demand side — each standing bid as `<org> bids <good>: list <authored>, posts <posted> (+<p>
   permille; reach <now> now against a median of <median>).` (or `(no port supplies it)`), then
   `emergency bids (at <ratio> permille of the median): none due; reach is <r> permille of the
   median.` or `… : <good> at <price>, … — <intake> each, at the cap (+<cap> permille), posted by
   <org>.`; a port without a premium says it posts its list prices whatever the season. Under the
   constant sky the same lines print at a zero premium before "every day is today".
5. **Lane v6** (`content/lane/ceres_phobos.json`, `LANE_VERSION` 6): both ports gain
   `"emergency": { "org": …, "ratio_permille": 1500, "goods": ["provisions", "volatiles",
   "propellant"], "intake": 100 }` — the Freeport Exchange at Ceres, Stickney Yards at Stickney (each
   the port's one vendor that neither asks nor bids any of the three) — and nothing else moves;
   signed, digest `f7f568f43ac917131c029c3cbffbcc4917448f6157a6486dce97142a5a4a8e15` pinned in
   `tests/lane.rs`. The commerce suite's v2 rebuild drops `emergency` as it drops `premium`.
6. **Docs and help**: doc 19 §6 "As built (cut A.4b)", §9's row landed, §10 Q-19-5; README; the
   trade help's second season paragraph (the emergency bids, `+chart <port>` says whether they are
   up).

### Verified

- **`tests/price.rs`** (7, the A.4 four plus three): both ports author the block as above and
  `check` passes; the nine refusals (ratio 999 and 100,001, no goods, a good twice, intake 0, an org
  that is not a vendor, Kasei — which asks propellant — as the org, an emergency without a premium,
  `unobtainium` at load); under the constant sky four fires post nothing on either port, every
  Stickney reading has a reach equal to its median and is not crossed, Ceres's volatiles has none,
  and the prices the bids would post at are the port's own highest authored price under the cap
  (Stickney volatiles 200 → 300, provisions 10 → 15, propellant 60 → 90; Ceres provisions 15,
  propellant 60); **the clerk**: a lane at 1,000 ‰ booted as an operator seed under the moving sky
  has Stickney's clerk post, one a fire, a Wanted of the Yards' vendor for each of the three goods —
  100 units at `priced(reference, 500)` (propellant 40 → 60, provisions 10 → 15, volatiles 200 →
  300), recorded in the map, the capital rows unchanged — and a reboot into a revision at
  100,000 ‰ taken as an update has it withdraw all three and forget the rows. The rig lands no
  authoritative submission, so the variant strips Stickney's stock rows and asks (and the Co-op's
  margin references that read them): a clerk with a stock mint in flight defers every act, which is
  what the first attempt measured (the readings crossed, the house funded, the vendor registered,
  nothing posted) — the deferral is right in production, where mints land within a fire.
- Unit tests in `price.rs` (4): the crossing at the ratio, a ratio of 1,000 always on, a zero
  median never, the ratio for the chart.
- **`tests/chart.rs`** (5): `+chart stickney` under the moving sky prints Kasei's metals and
  volatiles lines with the premium and the reach against the median, and the emergency line "none
  due; reach is … permille of the median"; under the constant sky the metals line posts 280 at
  +0 permille and the emergency line reads 1000 permille before "every day is today".
- `tests/lane.rs` (10) at v6; `tests/hulls.rs`, `ports.rs`, `commerce.rs`, `sale.rs` through the
  version constant.
- `cargo fmt --check` clean; `cargo clippy -p tacsim --all-targets -- -D warnings` clean; the whole
  tacsim suite (`--no-fail-fast`, 49 suites, 2,666 passed) had one red — `ports.rs`'s
  `ic3_port_entries_refuse_what_the_applier_cannot_bind`, twice: the emergency check ran before the
  vendors' own rules, so a vendor's fault was named as the emergency's (the check now runs last), and
  the suite's no-vendors acceptance case kept the emergency block (it now drops it, as the v2 shape
  has none) — after which `ports` 21, `price` 7, `hulls` 15, `commerce` 24 and the lib (1,249) ran
  green again; mush-server `boot_parity` 19 and the story `ic_slice_loop` 6 green (the story's
  figures unchanged: the shipped ratio is not crossed, and a constant sky posts nothing); mush-config
  `shipped_configs` 12 green.

### What the owner decides

- **The ratio and the depth** (Q-19-5): 1,500 ‰ of the median and 100 units, lane data on each
  port. At Ceres–Mars's season (reach about 15–27 h empty, median near 21 h) the worst day runs to
  about 1,300 ‰, so the shipped ratio never fires on the lane as authored — it is the Pallas-like
  port's lever, and a lane edit brings it into the lane's own season if wanted.
- **Which house posts them**: the Exchange and the Yards (the one vendor at each port with no line
  for the three goods). A port with no such vendor needs a new one authored before it can carry the
  block.

### Deferred

- **A.3b** the chart panel. The emergency bids on the room boards (they carry no `boards`;
  `+market/list` shows them). Readiness has no line for them.

## A.7

**The moons** ([doc 19](19-astronomy-engine.md) §3.6, cut A.7; D-19-17; Q-19-9 ruled by the owner
2026-10-19 — "Luna Phobos Deimos Titan are worth modeling … the moons are really where the commerce
is going to happen" — and Q-19-1 through Q-19-8's defaults accepted as proposed), built 2026-10-19 on
main. One commit with the Fable trailer, pushed when green. Nothing here touches a running world
until the next deploy, and **no deploy without the owner**.

### What landed

1. **A body may orbit a body** (`pack.rs`): `BodyDef.parent: Option<CanonicalId>` (wire `parent`,
   skipped when absent, so no fixture's re-encoding moves; `with_parent`, `is_moon`), the pack's
   cross-entry rule that a parent is a `body` of the same pack about the Sun and not the moon itself
   (`PackError::UnknownParent`; one level, so a position is a parent's plus its own), and a 150 km
   floor on the element table's distance consistency rule (`ELEMENTS_DISTANCE_FLOOR_KM`: a moon's
   authored distance is held in micro-AU, 75 km of quantization on Phobos's 9,376 km). `Body.parent`
   on the circular model (`Body` is no longer `Copy`).
2. **The sky composes** (`sky.rs`): `KeplerSky::position_f64` and `Sky::position_km` add the
   parent's position to a moon's own, on both models; `Sky::parent_of`; `Sky::distance_km` measures
   any pair with a moon in it between composed positions, never by the coplanar law (the slice's
   fixtures, with no moons, keep their byte-identical law-of-cosines figures); `Sky::period_days`
   answers a moon's **primary's** period — a moon's season is its planet's, so the reach premium's
   synodic scan over a port on Phobos is Mars–Ceres's 1,162 days and not a 7.65-hour one; the
   revision digest names the parent.
3. **The element blend for moons** (`kepler.rs`): `blended_elements_position` interpolates a moon's
   rows in element space — scalars and the node linearly in time, the mean longitude ω + M along
   the shorter arc with each row's M first advanced to the instant (a near-circular orbit's
   pericentre swings between rows while the longitude stays smooth; Deimos's e is 0.0002), the
   blended row propagated over zero days (`propagate_dt`; an epoch interpolated in IC seconds lands
   up to a real leap day off the instant, most of a turn for Deimos). Why: a moon's node and
   pericentre precess in months (Phobos's node by 158° a year), two rows a half-year apart put it at
   different phases of a small orbit, and the planets' position blend takes the chord's midpoint —
   Phobos read 4,794 km from Mars, half its radius, before this. The planets keep the position blend
   as measured and pinned in A.1.
4. **The data** (`fetch_elements.py --moons`): the moons' ELEMENTS asked about the parent (Horizons
   `CENTER` the planet's centre, the ecliptic frame), their golden VECTORS heliocentric like every
   other body's, the measure composing the parent's table with the moon's; **Titan has no Horizons
   ephemeris past 2250-01-05**, so its rows are the last decade Horizons knows (nineteen, 2240–2249)
   and the last row propagates alone through the window — self-consistent rather than Horizons-true,
   no golden rows, recorded in the pack's `extrapolated_from`. `golden.json` gains luna, phobos and
   deimos (241 rows each).
5. **Lane v7** (`content/lane/ceres_phobos.json`, `LANE_VERSION` 7): the four moon bodies —
   `luna` about earth (0.00258 AU, 27.53 d), `phobos` (63 µAU, 0.319 d) and `deimos` (157 µAU,
   1.263 d) about mars, `titan` about saturn (0.00793 AU, 15.95 d) — each with `parent` and its
   planetocentric table; **Stickney Yards stands on `phobos`**; the Kirkwood–Stickney legs re-timed
   to the derived 942,997 s (27 s inside the Kirkwood–Mars chord's 943,024); twenty-four entries;
   signed, digest `11fb2c9228799918184cc872a01d683e85de91e830104f170b966c7311f23541` pinned in
   `tests/lane.rs`. The commerce suite's v2 rebuild drops the moons and puts Stickney back on Mars.
6. **`+chart`** rows a moon with "moon of mars" after its from-here column; every other surface
   reads the composed position and needed nothing.
7. **Docs**: doc 19 §3.6 (D-19-17), §9's A.7 row, §10 (Q-19-1..8 accepted, Q-19-9 ruled); README;
   doc 18 §5's P-A row.

### Verified

- **`tests/sky.rs`** (8): the sixteen bodies all Keplerian, the four moons naming their parents and
  Titan's table nineteen rows; Horizons tracking for fourteen bodies — the eleven as before and
  Luna 150.23 (allowed 160), Phobos 8.84 (9), Deimos 2.41 (3) hundredths of a permille, every body's
  worst printed; a moon stands its own orbit from its parent at the epoch and a year either side
  (Luna 356,000–407,000 km, Phobos 9,200–9,600, Deimos 23,400–23,500, Titan 1,180,000–1,260,000);
  the epoch consistency compares a moon planetocentrically within the 75 km quantization; a moon's
  period is its primary's.
- **`tests/lane.rs`** (10): twenty-four entries and sixteen epochs; Stickney on phobos; the legs
  61,776 + 942,997 and the derived legs printed; the route chord Ceres–Phobos; the circular lane's
  sixteen bodies agree with the Euclidean distance; 125 periods of phase hold to the milli-degree
  (a whole number of seconds for a 319-milli-day period, where one period rounded 8 mdeg away).
- **Pins moved by Stickney's 9,400 km** (every figure read from the planner, none by hand):
  `passage.rs` outbound 321,547 + 1,256,295 = 1,577,842 (was 1,256,313 / 1,577,860), return
  1,195,643 + 306,023 = 1,501,666 (was 1,195,660 / 1,501,683); `voyage.rs` the story's ticket
  326,136 + 1,274,226 = 1,600,362 (was 1,274,245 / 1,600,381); `ports.rs` the plans' "22 unchanged"
  and "20 unchanged" (twenty-four entries). Burns and crew-days unchanged.
- `tests/chart.rs` (5): sixteen rows, phobos "moon of mars".
- `cargo fmt --check` clean; `cargo clippy -p tacsim --all-targets -- -D warnings` clean (one red on the
  way: `tests/ephemeris.rs` built a `Body` by hand without the new field); the whole tacsim suite
  (`--no-fail-fast`): 49 suites, 2,668 passed, 0 failed; mush-server `boot_parity` 19 and the story
  `ic_slice_loop` 6 green with no pin moved (the story reads its figures from the planner);
  mush-config `shipped_configs` 12 green.

### What the owner decides

- **Ports on the moons** (P7's kind): the bodies are there; Luna, Deimos and Titan want a `location`
  with `body`, a `port` entry and routes. Titan's is a long way out (Saturn at 9.4 AU: weeks at the
  Dray's thrust).
- **Titan's extrapolation**: a self-consistent orbit, not Horizons-true, until a later ephemeris
  reaches 2340. A later `fetch_elements.py` run re-fits it when one does.

### Deferred

- **A.3b** the chart panel (next).

## A.3b

**The chart panel** ([doc 19](19-astronomy-engine.md) §5, cut A.3b; D-19-18; Q-19-7's default
"text first, the panel after A.6"), built 2026-10-19 on main — the owner's "build out the chart panel
for both web and tui (and a flat render of the tui chart for line-oriented)". One commit with the Fable
trailer, pushed when green. **No deploy without the owner.**

### What landed

1. **One frame** (`crates/tacsim/src/port/chart_panel.rs`): the `sky.chart` panel feed, registered
   with the port verbs (`port::chart_panel::register`: the feed handle dropped, as the arena map's is
   — nothing pushes a chart; a `QueryHandler` answers a `PanelSubscribe` with one keyframe for the
   asker's vantage). The frame is **grid-shaped** (`schema.shape == "grid"`, the shape `atlas.map` and
   the arena map wear, so the one grid printer and the web's grid decoder already read it): its grid is
   the server's **flat render** — 60 by 28, the Sun `*` at the centre, every orbit a ring dotted every
   5° at a **logarithmic radial scale** (`radial_fraction` = `ln(1 + r/0.3 AU) / ln(1 + r_max/0.3 AU)`;
   a square root packed the six inner orbits into seven rows), the bodies by letter (`LETTERS`: m v E M
   J S C P j V H K L p d T, else the first free letter of the id; nearest the Sun first keeps a shared
   cell — Kirkwood trails Ceres by 2°, one cell — and the vantage's body is drawn last over everything;
   a moon shares its parent's cell and draws no letter), the listed routes' legs as `:` body to body
   (`line_cells`, Bresenham), the vantage port as the viewer's `you` and a `units` entry `@`; the
   legend names the Sun, each body (with its moons, "(here)"), the orbit and route glyphs. Beside it
   a **`chart` block**: `date`, `ic_now`, `model`, `rate_permille`, the `vantage` (port, body, hull,
   payload, souls), the `scale`, every body (`id`, `glyph`, `parent`, `r_au`, `lon_deg`, `lat_deg`,
   `x/y/z_au`, `period_days`, `here`, `from_here_au`, `lag_s`, `cell`), the `routes` (each
   destination's legs, the passage now at this load and empty, the direct chord), the `window` (now
   and the fastest, cheapest and worst day of the next IC year when the sky moves), and `lines` — the
   text `+chart` and `+chart <port>` print, for a renderer that composes nothing itself. A vantage
   with no lane answers a frame with no grid and the reason as its title.
2. **The vantage without a helm** (`chart.rs` `vantage_for_world(w, actor)`): the port whose room the
   actor stands in, else the first lane port, planned with the port's programme hull empty — what
   `+chart` does for a walker; `vantage(ctx)` keeps the docked charter's branch for `+chart` from a
   helm. `sky_lines` and `window_lines_for` factored out of `chart_sky`/`chart_window` for the panel.
3. **Line mode**: `+chart/map` prints the title, the legend, the whole grid north up with its blank
   edge rows trimmed, and the scale line — the TUI's chart pane flattened for a line player.
4. **The TUI** (`mush-net-ssh`): **the lettered-screen policy** — the ten digits are spent, so a screen
   past them has a lowercase letter's byte for its `ScreenId`, reached by `Alt+<letter>` when the
   active screen binds nothing to the letter (`TuiApp::binds`; the cockpit's `c` stays its
   contacts), the tab strip printing the letter, `validate_screens` admitting `a`–`z` that are no
   global or jump key, `MAX_SCREENS` 11. The **chart screen** `c`: gated on `sky.chart` (a world
   without the lane never grows the tab), `on_enter` asking for the frame; its Main pane
   (`render_chart_rows`) windows the grid to the pane around `you` (`mush_ansi::viewport` and
   `window_rows`, the cockpit map's way) with the clipped title above and the legend wrapped below;
   its Right pane (`render_chart_table_rows`) prints the server's lines, the window first; `r`
   refresh, `w` prefills `+chart `, `m` runs `+chart/map`. The screen sits before the cockpit, which
   stays last. The help card names Alt+letter.
5. **The web**: the same policy (`ScreenDef.digit: number | null` with `hotkey`; `validateScreens`
   holds a letter screen to one letter, no global key, no twins; `switchHotkey`; `keys.ts` answers
   `screenKey` for an unbound Alt+letter, never a global or jump letter; the tab strip shows `⌥c`),
   `CHART_SCREEN` as a `FeedScreen` on a `.chart` feed registered once per hello beside the unit
   screen; `lib/chart.ts` (the wire type, a tolerant decoder over `parseMapFrame`, `radialFraction` —
   the server's curve — `bodyPoints`, `orbitRadius`, `routePolyline`, and the pure table composers
   `bodyLines`, `routeLines`, `windowLines`, `chartHeading`); `stores/chart.svelte.ts`; the hello
   pass subscribes `sky.chart` last when the manifest carries it; `#routePanel` routes it;
   `refreshChart` on `r`; `ChartScreen.svelte` draws an SVG picture (orbits, bodies with labels, the
   routes' polylines, the vantage ringed, moons labelled small on their parents) with the tables
   beside it and the flat map under a toggle. `+chart` and `+chart/map` joined `RUN_ALLOWLIST` in both
   copies and the committed fixture (sorted).
6. **The fixture**: `web/src/lib/fixtures/chart/sky_chart.json`, a frame the server made for a walker
   at Ceres under the moving sky, blessed by `tests/chart.rs` (`TACSIM_BLESS=1`), decoded by the web
   tests and drawn by the TUI test.
7. **Docs and help**: doc 19 §5 "As built (cut A.3b)" (D-19-18), §9's row landed; README; the ships
   help's `+chart/map`.

### Verified

- `tests/chart.rs` (7): the frame's shape, size, title, legend (the Sun, `C ceres (here)`, Mars with
  its moons, the route glyph), `you`, the letters on the map and no moon letters, the chart block
  (sixteen bodies, Phobos on Mars's cell, the one route's legs ceres → kirkwood → phobos, empty and
  direct figures, the window's fastest and worst, the lines), `+chart/map`'s lines (title, legend,
  the rows with the blank edge rows trimmed, the scale), the fixture byte for byte; a world without
  a lane refuses with `CHART_NO_LANE`. Unit tests in `chart_panel.rs` (4): the scale, the raster
  (rings, route, letters, no moon letter, the vantage over a neighbour, run lengths), the line, the
  letters.
- `mush-net-ssh/tests/chart_screen.rs` (2): eleven screens validate, the chart on `c`, gated and
  asking on entry; Alt+c opens it where the feed is published — the frame draws the title, the Sun,
  the legend's `ceres (here)`, the tables pane's window and `Stickney Yards` — and is inert without
  the feed. The suite's own tests moved with the eleventh screen (count, the two gated screens, the
  bound verbs, the lettered tab).
- Web: `chart.test.ts` (9: decoder, tolerance, the committed fixture, the scale, points, polylines,
  the tables, the constant sky), the hotkey screen and Alt+letter tests, the allowlist fixture;
  `svelte-check` 0 errors.
- `cargo fmt --check` clean; `cargo clippy -p tacsim -p mush-net-ssh -p mush-server --all-targets --
  -D warnings` clean (three lints on the way: a reborrow artifact of the chart refactor, the
  lettered-id range check, a negated float comparison); the whole tacsim suite (`--no-fail-fast`):
  49 suites, 2,673 passed, one red — this cut's own raster unit test shadowing its grid text — fixed
  and the lib (1,253) and `chart` (7) re-run green; `mush-net-ssh` 259 lib tests and every
  integration suite green with the two new ones; mush-server `boot_parity` 19 (the manifest with
  `sky.chart`) and the story `ic_slice_loop` 6 green; mush-config `shipped_configs` 12 green; the
  web: 54 files, 621 tests green, `svelte-check` 0 errors, `npm run build` clean.

### What the owner decides

- **The picture's scale**: logarithmic past 0.3 AU (`R0_AU`), the edge at the farthest body plus
  2 %. A linear inner zone wider than Mercury's orbit, or a square root, is one constant.
- **A moon's place on the chart**: on its parent's cell, named in the legend and the table; a port on
  a moon draws as its planet. Zooming the picture to a planet's system (Luna, the Martian moons,
  Titan) is a later cut when a port stands there.

### Deferred

- The web picture's own zoom and the TUI pane's zoom; a `.chart` feed per vantage on a moving ship.

## A.8

**The first ports on the moons** ([doc 19](19-astronomy-engine.md) §3.8, cut A.8; D-19-19 … D-19-25;
the owner's ruling of Q-19-9 on 2026-10-19 — "the moons are really where the commerce is going to
happen"), built 2026-10-19 on main: **Aitken Highport** on
Luna, **Voltaire Anchorage** on Deimos and **Ligeia Terminal** on Titan. The design came from four
designs (commerce, lore, engine, play) scored by three judges (arithmetic, canon, engineering): the
play design as the base — the Concord ring plus a frontier spur, six routes, the smallest reach
footprint, every pair a whole-tonne vertex — with the commerce design's pricing discipline (Deimos a
pure import bond and fuel stop; no moon run beats the reference trade outright), the lore design's
setting (the ark moorings, the 2308 blockade depot, the Republic Strategic Reserve, Concord Orbital
Access, the Company against the townships, the three social rooms), the engine design's seams (one in
`passage::leg_distance_km`, one line in `+ship/plan`, the strip helpers) and the judges' corrections
(two goods, not three; the `instrument_boards` id; Titan's margins cut to a prize; a names pass;
Titan's `base_s` and throttle from the engine; the 600 s convergence bound). Two commits with the
Fable trailer, each pushed when green: **A.8a** (`b0d036d3`) goods v2 and the two engine seams,
green alone; **A.8b** the content — lane v8, the six station packs, the registrations, the pins, the
new suite and the docs. Nothing here touches a running world until the next deploy, and **no deploy
without the owner**.

### What landed

1. **Goods v2** (`content/goods.json`, A.8a): two rows, eight in all, no new family —
   `instrument_boards` ("instrument boards, crated", family `electronics`, 1,000 kg / 2,550 L,
   reference 900: crated boards from the chartered Earth-basin fabs, forwarded through Luna) and
   `carbon_feedstock` ("carbon billet, pressed", family `carbon_feedstock`, 1,000 kg / 600 L,
   reference 250: Titan's billet). **The integrality finding**: among the six shipped hold goods only
   {metals, volatiles} and {structural_alloys, industrial_parts} give a whole-tonne both-limits
   vertex, and those are the reference loop's two directions, so a moon port of shipped goods is a
   second Ceres or Stickney or flies empty; boards at 2,550 L pair with the shipped alloys at 40 t
   of boards and 60 t of alloy, billet at 600 L with the shipped ice at 25 t of billet and 75 t of
   ice. The id is not `electronics`:
   doc 17 §16 and 17f M7.9 keep `tacsim.electronics` for quantity-one certified die lots graded
   die.c1..c3, and a 1,000 kg bulk row under that id would make a die lot weigh a tonne. The table's
   `version` 2, `GOODS_DIGEST` `50cb8d9fbf56710f15ac0caaa9419e9d9d7100de95aba01e6e9a3cfe2e518c57`,
   the catalog count 8, one zero load-cell line per new good (`gauges.rs`'s unit tests,
   `tests/coop.rs`'s `load_cells()`, `ic_slice_loop`'s cells), `UNKNOWN_GOOD`'s list eight ids, the
   goods module doc, `resources.json`'s carbon gloss naming Titan.
2. **Sibling moons in the parent's frame** (`passage::leg_distance_km`, A.8a; D-19-21): when both
   bodies of a leg share a parent, the leg is each body less its primary at its own instant, with no
   solar keep-out; `Sky::distance_km`, light lag and `scan_days` are untouched. In the Sun's frame
   Mars's 24 km/s turned the 29,500 km Deimos–Phobos hop into about 1.7 IC days (40 hours) and five tonnes, still
   diverging at twenty intercept passes; in Mars's frame it plans in hours, a tonne and a crew-day both
   ways. A same-instant chord is frame-invariant, so every authored `base_s` and every pinned leg
   stands. **The convergence exception**: five passes and twenty agree within 600 s, not doc 19
   §4.4's 60 s — Phobos turns about half its orbit during the hop (Phobos→Deimos converges inside
   2 s); §4.4 records it. The 2,000-day scan cap for the Phobos–Deimos pair stays.
3. **The `Throttle:` line** (`+ship/plan` only, A.8a; D-19-22): when the plan at the asked throttle
   still fails with the tank and the locker filled to capacity, one line after the Stores line names
   the highest throttle that fits — bisection over 250..=1000 on the flip law's monotone burn, the
   locker then checked at it — as `Throttle: with the tank and locker full, throttle={throttle}
   fits: burn {burn} t, {days} crew-days, {passage} IC days.` (`THROTTLE_FITS`), or `Throttle: no
   throttle from {min} permille fits this load even with the tank and locker full.`
   (`THROTTLE_NONE`). It never starts with `Burn ` or `Stores `, so `plan_numbers` and its copies
   stand; TANK_SHORT, STORES_SHORT and the queue's hold line are unchanged, so no refusal pin moves.
4. **Lane v8** (`content/lane/ceres_phobos.json`, `LANE_VERSION` 8; D-19-19): thirty-six entries.
   **Three places** — `aitken_highport` on `luna`, `voltaire_anchorage` on `deimos`,
   `ligeia_terminal` on `titan`, each role `port`. **Six direct routes**, one leg each, `coast_hauler`,
   `base_s` from the engine (`tests/sky.rs`'s a1 print widened to every route): `ceres_aitken`
   1,609,856 s; `aitken_voltaire` 728,633 s; `aitken_ligeia` and `ligeia_aitken` 6,803,208 s;
   `ligeia_voltaire` 6,477,235 s; `voltaire_stickney` 131 s (the hop, declared empty: Deimos asks no
   hold good Stickney bids). The graph: Ceres lists {luna, stickney}, Stickney {ceres} as before, Luna
   {deimos, titan}, Deimos {stickney}, Titan {deimos, luna}; no Luna→Ceres (nothing at Ceres buys what
   Luna sells), no Stickney→Deimos (the Republic does not bond its own yard stock), no Ceres↔Titan
   (ceiling about 430). **Three ports**, each `premium {300, 500}` and an `emergency` block at
   1,500 ‰ and intake 100: `luna_port` — Concord Orbital Access (the port authority, no vendor),
   Cabeus Works (asks alloys 400, 3,120 t; bids concentrate 270 for 800 t), the Lunar Habitat Board
   (bid-only: ice 195 for 4,100 t, billet 380 for 300 t), Crisium Instruments (asks boards 900,
   2,080 t; the emergency org for provisions, volatiles and propellant), the Highport Victualling
   Office (reaction mass 60, provisions 10); 1,400,000 cr in six tranches. `deimos_port` — the
   Voltaire Bond Authority (operator, no vendor), the Republic Strategic Reserve (reaction mass 60,
   provisions 10; bids ice 190 for 900 t, an authored floor, and billet 380 for 300 t), Syrtis Bonded
   Imports (bid-only: alloys 460 for 2,400 t, boards 1,100 for 1,600 t; the emergency org for the
   three goods); 3,450,000 cr in six. `titan_port` — the Titan Concession Company (asks billet 250,
   600 t; ice 40, 1,800 t; reaction mass 30, 5,000 t; faction `company`), Concession Stores (provisions 20,
   2,600 crew-days; `company`), the Selk Townships Board (bid-only: alloys 760 for 720 t, boards
   1,500 for 480 t; faction `townships`; the emergency org for provisions alone); 1,450,000 cr in
   four. **Ceres's `luna` key** (`ceres_port.routes.luna = ["ceres_aitken"]`, direct: the charter's
   depot rule binds Ceres-bound legs and the Ceres–Mars corridor, not a Concord-bound leg out of
   Ceres). **Two programme reference rows** (`instrument_boards` → `luna_port/crisium_instruments`,
   `carbon_feedstock` → `titan_port/titan_concession`), so an enrolled sale of a new good logs no
   false "reference listing is gone". 6,300,000 cr new in sixteen tranches, eleven new orgs (twenty in
   all), three new clerks; no recorded row of v7 changes, so v8 applies over a kept world. Signed,
   digest `be733686a1630175e541346c693fcb2cce9f658db876121c73c4d476e04ca330` pinned in
   `tests/lane.rs`.
5. **The six station packs** (`content/atlas/`; region `region.<location>` v1 with one `surface`
   region on the moon body and a `locality.<location>` of kind `port`; neighborhood v1 requiring it).
   Common: the gallery "Visiting Berth Gallery" with `markers ["port.dock"]`; a restricted spine
   holding the storefront items and the principals (`profile.enabled: false`, no interfaces), reached
   by one hatch with `lock: "staff"`; every signatory an enabled NPC in a public room with a non-LLM
   fallback; a social room with a keeper drawing the port's lore pool; exit words drawn from the two
   shipped stations' alias set plus each station's own codes.
   - **Aitken Highport** (`neighborhood.aitken_highport`): the Concord's spun civil drum in low polar
     orbit over the South Pole–Aitken basin; site "Aitken Drum", decks "Hub Deck" (zero-g) and "Drum
     Deck" (spin at lunar g); 12 rooms, the spoke ladder between the decks; fifteen exits. Cast nine,
     four of them signatories (Lorcan Mwangi-Eriksen of Cabeus, Thandiwe Albrecht-Sato of the Board,
     Anneliese Okafor-Thistlewood of Crisium, Imogen Batista-Okeke of the Victualling Office), with
     Dockmaster Aurelio Kowalczyk-Amadi, Deputy Registrar Kwabena Holt-Iversen, Mireille
     Devereux-Anand, Maren Ferro-Wainwright and Ngozi Rasmussen-Oto keeping the Cold Trap; four
     principals (Cosima Ferreira-Lund, Odile Kwarteng-Fairweather, Benedikt Rourke-Haddad, Ingrid
     Moravec-Ansah). Four storefront items (the Cabeus stock cage, the Board's intake manifold, the
     Crisium bonded cage, the Victualling stores locker). `lore.port_aitken`, thirteen entries (ten
     public, three guarded); `secret.half_afloat` about the Board's Chair. One wall (`l01|l12`) and
     five counters. The staff hatch `exit.l08_l15`, Habitat Board Receiving to the Rim Service Spine.
   - **Voltaire Anchorage** (`neighborhood.voltaire_anchorage`): the Republic's bonded anchorage in
     Voltaire crater over the Strategic Reserve; site "Voltaire Roads", structure "Moorings Block",
     decks "Tank Deck" and "Roads Deck"; 11 rooms on the Stickney boxes, the Ark Moorings Gallery with
     the *Long Mercy* in its window; eleven exits. Cast six, two signatories (Caspian Abiodun-Fleck,
     Quartermaster of the Reserve; Emeka Strand-Villanueva of Syrtis), with Femi Rautio-Grant at
     customs, Aurek Szabo-Mensah the Roadsmaster, Marisol Kettunen-Ade keeping the Long Straw and
     Wilhelmina Tsai-Barros, the Company's agent; three principals (Radmila Delgado-Mertens, Anders
     Bergström-Nwachukwu, Liesel Okwuosa-Brandão). Two storefront items (the Reserve intake manifold,
     the Syrtis bonded cage). `lore.port_voltaire`, twelve entries (ten public, two guarded);
     `secret.reserve_level` about the Reserve Warden. Seven walls and four counters. The staff hatch
     `exit.d07_d15`, the Reserve Receiving Hall to the Reserve Spine.
   - **Ligeia Terminal** (`neighborhood.ligeia_terminal`): the Titan Concession Company's unspun spar
     in low Titan orbit; site "Ligeia Spar", decks "Cargo Deck" and "Company Deck"; 12 rooms, the
     Lifter Gallery's long window and the Ninety-Four; fourteen exits. Cast six, three signatories
     (Ysolde Marangoni-Teague of the Company, Barnabas Oyelowo-Krauss of the Stores, Saoirse
     Tupou-Lindahl of the Townships Board), with Kenji Abara-Vos the dockmaster, Noor Adichie-Brun at
     the arrivals desk and Ottoline Quayle-Ibekwe keeping the Ninety-Four; two principals (Augustin
     Varela-Nwosu, Chief Factor; Hallgerd Asante-Pryce, Speaker of the Selk Townships). Three
     storefront items (the Concession export manifold, the Company Store locker, the Townships
     receiving cage). `lore.port_ligeia`, twelve entries (nine public, three guarded);
     `secret.retainer` about the Chief Factor. Four walls and four counters. The staff hatch
     `exit.t11_t15`, the Bunkering Yard to the Spar Service Spine.
   Signed digests: `region.aitken_highport` `4dd766c3…`, `neighborhood.aitken_highport` `2d412668…`,
   `region.voltaire_anchorage` `b95670c9…`, `neighborhood.voltaire_anchorage` `790d3363…`,
   `region.ligeia_terminal` `73123abe…`, `neighborhood.ligeia_terminal` `11091438…`, pinned in full in
   `tests/stations.rs`.
6. **The registrations** (`definitions/content.rs`, `ic_return.rs`): the six pack ids and
   `include_str!` constants; `STATION_SOURCES: [(&str, &str); 10]`, each region before its
   neighborhood, appended after Stickney; `AITKEN_ROOMS` 12, `VOLTAIRE_ROOMS` 11, `LIGEIA_ROOMS` 12;
   `LANE_VERSION` 8 and the doc comments; `ic_return::PORTS: [IcPort; 5]` — Aitken Highport arriving
   at `room.l03_arrivals`, Voltaire Anchorage at `room.d01_berth_gallery`, Ligeia Terminal at
   `room.t03_arrivals`, none with a Limbo door — and `IC_HELP` naming the five ports.
7. **The pins moved** (each figure read from a run, none by hand): `tests/lane.rs` (the digest, version
   8, thirty-six (id, kind) entries, six places, the six routes' legs, `lane_legs_derive_at_epoch` over
   every route within 100 s, Titan's three included against their own propagated 2249 row — a
   self-consistency check, not accuracy against Horizons, until an ephemeris past 2250 exists);
   `tests/ports.rs` (the treasury at −14,500,000.00 cr, five port docs, a kept world's "0 create, 2
   update, 0 retire, 34 unchanged", v1 → v8 "11 create, 2 update, 0 retire, 23 unchanged", the
   five-port Absent loops, the shipped lane's 46 vendor, ask and bid readiness lines pinned and the
   not-ready count derived from the list, `ic_return`'s "five and five"); `tests/commerce.rs` (the four
   other clerks as a set; the v2 rebuild through `support::moon::strip_moon_trade` and
   `strip_moon_places`, `LANE_V2_DIGEST` held byte for byte); `tests/price.rs` (`stickney_variant`
   through `strip_moon_trade`, so Ceres is again Stickney's only supplier; the volatiles `None` pin
   stands; `a8_the_old_ports_suppliers_move_as_designed` holds doc 19 §6's old-port rows, v8 against a
   v7-shaped lane at rate 0 — Stickney's provisions and propellant reach 11,718 s on v8 against
   1,250,608 s on v7, its volatiles, Ceres's readings and the four old bids unmoved); `tests/chart.rs`
   (`CHART_NO_ROUTE` "Aitken Highport, Stickney Yards", two routes and two windows, Stickney found by
   `to`) and `web/src/lib/fixtures/chart/sky_chart.json` re-blessed for Ceres's Luna route, its window
   and Luna's demand lines; Stickney's emergency line keeps its text ("none due; reach is 1016 permille
   of the median": the volatiles reading, supplied through Ceres alone, still sets the maximum, and
   only the provisions and propellant readings flatten to the hop) and the Kasei bid lines are
   unchanged; the voyage and orders lists (`VIA_UNKNOWN` six labels, `UNKNOWN_PORT` five names in
   port-entry order, `NO_ROUTE` at Ceres "Aitken Highport, Stickney Yards"); `tests/capacity.rs` (the
   catalog 8; `ic8_the_posted_prices_make_the_both_limits_load_optimal` builds its pairs from every
   port's listed routes — eight, the five new ones among them — derives the seven that trade through
   the LP, and asserts the Deimos → Stickney hop has no tradeable pair); `tests/stations.rs`
   (`PINNED`/`FILES` ten, the boot list, the requires arms, the dock set {c01, p01, l01, d01, t01}, the
   walls and seams, the cast and principals, the locked hatches); mush-server `boot_parity` (twenty
   orgs, sixteen packs). `tests/support/moon.rs` holds the two strip helpers.
8. **`tests/moon_ports.rs`** (new, pure: the shipped lane through `rules_pack_of`, the Dray from the
   fielded catalog, no world): `a8_the_lane_carries_five_ports_and_six_direct_routes` (the three places
   on their moons, the six routes one leg each, the graph exactly, the declared absences — Luna→Ceres,
   Stickney→Deimos, Ceres↔Titan — by route and by listing, thirty-six entries, Titan's bunkers at 30
   and 20); `a8_the_hop_plans_in_hours_on_the_real_route` (`voltaire_stickney` under the moving sky
   every four IC days over forty: under six IC hours, at most 2 t, one crew-day);
   `a8_titan_fits_the_tank_only_throttled` (at the epoch on a still sky, laden with the tank and locker
   full and one soul, the three listed Titan legs only: each at full throttle burns past the tank; at
   450 it fits the tank and the locker and its ceiling keeps the 15 ‰ margin — 530, 530 and 557 ‰
   printed; two souls exceed the locker past sixty days); `a8_titan_throttle_fits_a_saturn_year`
   (twenty-four departures across 11,000 IC days plus a ±14-day window at one-day steps around each
   replica worst departure — day +7526 Luna→Titan, +7106 Titan→Luna, +7812 Titan→Deimos — 53 a leg,
   every one fitting the tank and the locker at 450 and the tank at 465; each leg's ceiling bisected at
   its departure with the largest burn at 465 and printed with its range of IC days at 450; empty at
   that departure the leg is faster and the ceiling plus one fits; and **Ceres→Titan**, on a synthetic
   `ceres_ligeia` route and in that direction only, fitting 465 at the epoch and failing it at one of
   48 sampled departures, the first such printed with its ceiling);
   `a8_the_ring_outranks_the_shuttle_by_ordering_at_the_epoch` (net per IC day at the epoch only:
   ring > reference > an empty Luna-and-back on a synthetic route the lane does not list; every leg's load
   solved from the authored prices by `support/vertex.rs` and asserted the both-limits vertex, none
   typed; no multiplier asserted; printed reference 459, ring 493, Luna-and-back empty 94 and the Titan
   shuttle 425 cr a day). The sibling-frame test is `tests/passage.rs`'s
   `a8_sibling_moons_are_measured_in_the_parents_frame`, and
   `plan_passage_reproduces_the_slice_figures` now checks every route's `base_s` exactly (eight routes,
   the hop's 131 s); the bisection's unit test is `src/port/voyage.rs`'s
   `a8_the_highest_fitting_throttle_is_found_by_bisection`, and `tests/voyage.rs`'s
   `a8_the_plan_names_the_throttle_that_fits` runs the `Throttle:` line through a real `+ship/plan` (a
   full-hold Ceres→Titan plan at the epoch on a seeded lane: one line after Stores, the same line at
   N+1 and N+20, none at N with the plan's own Burn, Stores and Passage lines matching it, none on
   Ceres→Stickney; printed "1000 permille burns 106 t (41 crew-days); throttle=539 fits: 80 t, 56
   crew-days, 55.3 IC days"). Also new: `tests/commerce.rs`'s `a8_the_moon_ports_stock_list_and_bid`
   (ten moon receipts minted once and read back, every moon pool at its authored quantity, the three
   bid-only vendors Ready at 0 of 0) and `a8_a_world_on_lane_v7_takes_v8_by_the_shelf`;
   `tests/sale.rs`'s `a8_the_programme_reference_names_the_moon_asks` and its control
   `a8_without_the_boards_reference_row_the_sale_stamps_the_baseline_memo` (a boards sale into a
   variant Kasei bid, basis 400,000 minor, the share 100,000, the memo only without the row);
   `tests/stations.rs`'s `ic1_the_seams_are_walled_on_the_map` over all five stations (25 walls) and
   `ic1_every_cross_deck_portal_stands_inside_both_rooms_and_clear_of_restricted_ones` (7 portals);
   `ic_return`'s unit test `a8_two_markers_read_as_one_of_the_characters_own_stamps`.
9. **Docs and help**: doc 19 §3.8 (D-19-19 … D-19-25) and §4.3, §4.4 (the 600 s exception), §4.6
   (rewritten to the as built: A.8 ships the first direct routes; D-19-13's rationale narrowed), §5,
   §6 (the supplier table), §9's A.8 row, §10; doc 18 §1, §4.3 (the eight-row table and the
   integrality finding), §4.5 (five stations), §4.7 (the moon port entries as built), §5 (P7's row),
   §9, and D-18-1, D-18-27 (the moon stock basis amended), D-18-35, D-18-36, D-18-39; doc 11 §2 (the
   ark moorings 2150s–2200s, the blockade depot 2308, the Reserve in the 2312 Settlement row, the
   Saturn Charter of 2291), §3 (the Needs list's light alloy; the as-built notes), §4.13–§4.23 (the
   eleven houses), §5 (the three domains), §10 (the counts); doc 10 §11; doc 15 (Aitken under
   Malapert, Voltaire in the Phobos half's coverage, no Titan half); 18a §5–§7 (the three stations on
   the §0.2–§0.4 template); the README rows; the help — `trade.md`, `npc.md`, `ports.md`, `travel.md`,
   `ships.md`, `orders.md`, `getting-started.md`.

### Verified

The runs on the tree as committed (2026-10-19, after the review-fix pass; one cargo at a time, `-j 8`):

- `cargo fmt --all -- --check` clean; `cargo clippy -p tacsim -p mush-server -p mush-net-ssh
  --all-targets -- -D warnings` clean.
- The whole tacsim suite (`--no-fail-fast`): 2,705 passed, 0 failed, 13 ignored, among them
  `tests/stations.rs` 21 (the ten packs validate clean with no warning, the digests pinned, the
  adjacency and the walls on the map at all five stations, every cross-deck portal inside both rooms,
  the dock set, the requires arms, the counts), `tests/lane.rs` 10, `tests/ports.rs` 21,
  `tests/commerce.rs` 26, `tests/price.rs` 8, `tests/chart.rs` 7 (the fixture re-blessed),
  `tests/capacity.rs` 17, `tests/voyage.rs` 25, `tests/orders.rs` 33, `tests/coop.rs` 23,
  `tests/sky.rs` 8, `tests/hulls.rs` 15, `tests/ephemeris.rs` 8, `tests/passage.rs` 5,
  `tests/content.rs` 29 (`GOODS_DIGEST`, the table's version 2, the eight rows), `tests/sale.rs` 18
  (`plan_numbers` unmoved by the `Throttle:` line) and `tests/moon_ports.rs` 5 — the printed Titan
  ceilings at the hardest sampled departure 491 ‰ Luna→Titan (day +7537), 491 ‰ Titan→Luna (+7096)
  and 466 ‰ Titan→Deimos (+7816), the worst burns at 450 77, 77 and 79 t, a leg 54.4–63.5, 54.2–63.4
  and 52.4–65.2 IC days at 450 (53–66 crew-days a soul over the three); the Ceres→Titan sweep's first
  sampled departure past 465 ‰ day +1604 at a 461 ‰ ceiling, and an earlier run of the same sweep,
  bisecting all 48 samples, read its minimum at 429 ‰ (day +7562).
- mush-server `boot_parity` 20, the story `ic_slice_loop` 6 and `ic_buyout_dormancy` 13; mush-net-ssh
  (all suites) green, `chart_screen` 2; mush-contract-tests (all) green, the exit-word gate over the
  ten packs with `DECIDED` at its two rows; mush-config `shipped_configs` 12.
- The web: `npm test` 621 passed in 54 files, the chart fixture decoded; `svelte-check` 0 errors,
  0 warnings.

### Recipes

- The lane: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test lane ic2_the_lane_pack_is_pinned`
  re-signs `ceres_phobos.json`; the plain run prints `LANE_DIGEST` for `tests/lane.rs`.
- The stations: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test stations
  ic1_the_station_packs_load_and_their_digests_are_pinned` re-signs the ten files; the plain run
  prints the `(pack, digest)` lines for `PINNED`.
- The goods: `TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test content ic2_the_goods_table_is_pinned`
  prints `GOODS_DIGEST` for `tests/content.rs`.
- The chart fixture: `TACSIM_BLESS=1 cargo test -p tacsim --test chart` rewrites
  `web/src/lib/fixtures/chart/sky_chart.json`.
- The routes' `base_s`: `cargo test -p tacsim --test sky a1_lane -- --nocapture` prints every route's
  legs at the epoch.

### What the owner decides

- **O1 Goods**: two new rows (default), or three with `composites` (1,900 L, reference 800) for a
  richer Titan backhaul.
- **O2 The Luna good's id**: `instrument_boards` (default), or `electronics`, when doc 17 §16's die
  lots take another id.
- **O3 Titan**: `throttle=450`, one soul, with the `+ship/plan` hint (default; 440 the fallback word
  if the engine puts the Titan→Deimos ceiling under 465), or an authored per-route throttle.
- **O4 The hop**: the sibling-moon frame (default), or the Sun-frame hop of about 1.7 IC days (40 hours) and 5 t.
- **O5 The graph**: six routes (default), or eight — Luna→Ceres via Kirkwood as an empty way home and
  Stickney→Deimos, re-deriving Ceres's premium pins and removing Deimos's alloy bid.
- **O6 The programme's reference**: the two rows (default), or the `proceeds.rs` warn demotion.
- **O7 Luna's bids**: 270 / 195, a shade under Kasei's (default), or 290 / 210, when Luna dominates
  the shuttle until its 40-load intake fills; 260 / 185 is the ready lower alternative.
- **O8 Titan's prices**: alloy 760 and boards 1,500, the shuttle about 1.5× the reference per IC day
  over a Saturn year (default), or richer (850 / 1,700, about 3×).
- **O9 Titan's polity**: the Concord's Saturn Charter of 2291 with faction words `company` and
  `townships` (default), or a Registry grant with `concession`, or the Board under `concord`.
- **O10 Deimos**: the Republic's bonded anchorage and Strategic Reserve with the new history rows
  (default), or a Freeport-leaning anchorage.
- **O11 Names** (Q-18-12's "names ok"): Aitken Highport, Voltaire Anchorage, Ligeia Terminal, the
  *Long Mercy*, the eleven houses and the thirty new people (twenty-one cast, nine principals) — owed.
- **O12 Ceres's stock**: not topped up (default; Occator's 9,600 t of ice faces 12,800–13,700 t of
  intakes, the Exchange's 4,000 t of concentrate 4,800 t), or a fresh-store-only resize.
- **O13 The shipped lane's tank-short seasons**: the hint only this cut (default), or an auto-throttle
  on a queued depart as a follow-up.
- **O14 Stickney's provisions and propellant emergency**: left inert by the depot next door
  (default); live only if Deimos stops asking them.
- **O15 Ceres→Luna**: direct (default), or via Kirkwood (77 t in the worst season, about 0.7 IC days
  longer; Luna's reach, premium and the ring re-derive by about a percent).
- **O16 The Reserve's ice bid**: 190 kept as an authored floor (default; Titan's Deimos-side ice sells
  at Kasei by the hop), or the Reserve sized to its carbon bid alone, or the ice bid dropped and
  Titan→Deimos re-paired.

### Deferred

- **The chart frame's and the hourly emergency reads' cost — owed a measurement** (doc 19 §6; not
  measured in this cut): Ceres's frame with its second destination's window, Stickney's two
  emergency readings scanning 2,000 days over the three-leg plan and the hop, Deimos's 778- and
  734-day scans. **The supplier-set reach memo** (a pure-function cache, no pin moves) waits until
  that measurement shows a frame over budget (Q15).
- **An auto-throttle on a queued depart** — this cut ships the `Throttle:` line only.
- **The moon zoom on the chart**: the three ports draw on their parents' cells and are named in the
  legend and the table; zooming to a planet's system is a later cut, the owner's call.
- **Stickney→Deimos and Luna→Ceres** as later cuts if wanted (O5); `tests/moon_ports.rs` holds both
  absent until then.
- **O12's fresh-store resize** of Occator's and the Exchange's recorded stock.
- **The shipped loop's tank-short seasons** (Q13): laden at full throttle under the moving sky the
  replica finds Ceres→Stickney TANK_SHORT on about 14 % of departures and Stickney→Ceres on about
  33 % (worst-season fits about 960 and 870 ‰); the engine's figures are read when the auto-throttle cut
  is taken. The hint names the throttle; the queue does not yet take it.
- **Titan's table re-fit** when an ephemeris past 2250 exists: the 2249 row propagates 102 years, so
  Titan's `base_s`, ceilings and crew-days move with a re-fit, and the lane's 100 s derive check on
  Titan's legs then measures accuracy, not only the pack's consistency with its own propagated row.
