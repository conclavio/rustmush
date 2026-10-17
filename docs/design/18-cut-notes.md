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
