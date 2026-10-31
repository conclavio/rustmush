# 34 — TacSim Content Studio & LLM Co-Authoring

*Draft 0.2, 2026-10-30 — owner questions `Q-34-1..4` all ruled (`D-34-1..4`); review pass against `f659d907` folded (`D-34-5..9`); cuts `CS.0a`–`CS.4`.*

## 1. Goal & Scope

Provide a staff-only web workbench under `web/src/components/WorldgenPanel.svelte` (`Builder tools`) to visually browse, author, LLM-draft, tweak, validate, **publish live to a running dev world**, and **download canonically signed JSON assets** for the content families in `crates/tacsim/content/`.

Every file in `crates/tacsim/content/` (20 directories and 2 root files) is partitioned between the three existing studios and the new **Content Studio**:

| Content Path | Format & Loader | Owner Studio / Tool | Scope & Notes |
| :--- | :--- | :--- | :--- |
| `ground/*.draft.json`, `arenas/*.json`, `atlas/{proving_ground,winter_valley,mars_canyon}.json` | `GroundDraft` / `campaign_seed` / `region` | **Terrain & Arena Studio** (`MapEditorStudio.svelte`, doc 30) | **Covered.** Tactical heightfields, geology, and arena sites. |
| `units/*.json` (+ `crates/mush-pixel/looks/`) | `rules_definition` (`RulesPack`) | **Unit Designer** (`UnitDesignerStudio.svelte`, doc 31) | **Covered.** Uses `unit_design` invoke and `tacsim::design::check`. |
| `minds/**` | `mind_package` | **Mind Workshop** (`MindWorkshop.svelte`, doc 17) | **Covered.** Mounted in `Game.svelte` via `mind` invoke. |
| `sky/golden.json` + `lane` `body.elements` | Ephemeris JSON | **Python Horizons Scripts** (`sky/fetch_*.py`) | **Generated.** Read-only in studio; never hand-edited. |
| `atlas/region.*.json` (11 files) & `atlas/neighborhood.*.json` (11 files) — **Spatial entries** (`site`, `structure`, `room`, `exit`, `feature`, `item`, `secret`, `brief.prose`) | `campaign_seed` (`mush_content::Pack`) | **Room Drafts (`@gen` / `+build` in `mush-worldgen/src/proposal.rs`)** + Hand-edited JSON | **Spatial layout stays in `atlas/`.** Room `details` keywords (`atlas:detail:<kw>`) are exposed to the Quest Studio, and **station NPCs + lore pools are split out** into `cast/<station>.json` (`D-34-3`). |
| **`cast/<station>.json` (New, Cut CS.0b)** + `crew/*.json` + `passengers/*.json` | `campaign_seed` (`mush_content::Pack`) | **Content Studio — Slice CS.1 (NPC & Cast Studio)** | **Deliverable 1.** The **52 station NPCs & 20 lore pools** across the 5 port stations (Aitken 13, Ceres 13, Stickney 9, Voltaire 9, Ligeia 8), plus `crew` (2) and `passengers` (3, including `stickney_v1.json`). |
| `quests/*.json` (5 packs) + mixed-pack quests (`quest.leila_second_reading` in `passengers/stickney.json`) | `campaign_seed` (`mush_content::Pack`) | **Content Studio — Slice CS.2 (Quest Studio)** | **Deliverable 2.** `first_steps`, `leila_followup`, `port_connections`, `safe_passage`, `survey_work`, plus embedded quests in passenger/cast packs. |
| `weapons/*.json`, `ammunition/*.json`, `sensors/*.json`, `cores/cores.json`, `hardware/governed_hardware.json`, `resolution.json` | `rules_definition` (`RulesPack`) | **Content Studio — Slice CS.3 (Component & Rules Studio)** | **Deliverable 3.** Verified via `pack_digest` over `RulesPack::canonical_value` and `Catalog::build_fielded`. |
| `tools/*.json`, `market/goods.json`, `resources.json` | Standalone Typed JSON (no manifest/digest) | **Content Studio — Slice CS.3 (Component & Rules Studio)** | **Deliverable 3.** Verified by `tools_table()`, `goods_table()`, and `ResourceTable::from_json()`. |
| `lane/ceres_phobos.json` (v15), `relays/luna_ceres.json` (`campaign_seed`), `relays/manufacture-v1.json` (`rules_definition`) | Signed Packs (`mush_content::Pack` / `RulesPack`) | **Content Studio — Slice CS.4 (Corridor & Relay Studio)** | **Deliverable 4.** 11 `port` entries (embedding `orgs`, `vendors`, `programme`, `vehicles`, `licence`, `site`), 12 `location`s, 20 `route`s, and relay star-pairs. (Historical `luna_ceres-v1.json` preserved). |
| `relays/service-v1.json`, `proving/conduct.json`, `proving/scenarios.json` | Format-versioned Standalone JSON | **Hand-edited JSON (Out of Scope for CS.4)** | Small policy/fixture files with custom loaders (`relay_service`, `conduct`); kept hand-edited. |

---

## 2. Decisions & Rulings (`D-34-1` .. `D-34-9`)

### `D-34-1` (Ruling on `Q-34-1`): Sub-Tabs & Pills under `WorldgenPanel.svelte`
`WorldgenPanel.svelte` (`Builder tools`) adds a `Content Studio` tab alongside `Terrain & Arena Studio`, `Unit Designer`, and `Room Drafts (@gen)`. Inside `Content Studio`, secondary pills switch between **`NPCs & Cast`**, **`Quests`**, **`Components & Goods`**, and **`Corridor & Relays`**.

### `D-34-2` (Ruling on `Q-34-2`): Live Dev-World Publish + Repo Export
Every studio slice supports both **Publish to Running World** (for immediate testing on the dev instance) and **Download for Repo**:
1. **Live Dev-World Publish for `campaign_seed` packs (`cast`, `crew`, `passengers`, `quests`, `relays`, `lane`)**:
   - `@atlas/pack apply` and `@tacsim/seed` only read packs that were pre-loaded into `AtlasState::shelf` or `TacsimStore` at server boot.
   - Instead, the `content_studio` invoke's `Publish` op takes the validated `mush_content::Pack` in memory, bumps `manifest.version` to `max(shipped_version, latched_version) + 1`, inserts/updates the pack in `AtlasState::shelf` (so `@atlas/pack show|plan` sees it), persists a studio shelf record `tacsim:studio_seed:{id}@{version}`, and calls `mush_content::module::apply_pack(ctx, &pack)` directly — the exact live-apply seam used by `mush_worldgen::proposal::go`.
   - Staff can immediately talk to a newly created/edited NPC or run `+quest` / `+crew` / `+passage` in the dev instance **with zero server restart**.
2. **Live Dev-World Publish for `rules_definition` packs (`weapons`, `ammunition`, `sensors`, `cores`, `hardware`, `resolution`)**:
   - Because rules definitions mint boot-time `DefinitionRef`s and have no `ContentState` latch, `Publish` follows `crates/tacsim/src/design.rs` (`D-31-9`): it writes a `tacsim:studio_rules:{id}@{version}` row read at boot alongside `[tacsim].pack_paths` ("fieldable after restart").

### `D-34-3` (Ruling on `Q-34-3`): Splitting Station Cast & Lore Pools into `cast/<station>.json`
Rooms, exits, and station geometry in `atlas/neighborhood.*.json` rarely change, whereas the **52 station NPCs and 20 lore pools** across the 5 port stations (`aitken_highport` 13, `ceres_freeport` 13, `stickney_yards` 9, `voltaire_anchorage` 9, `ligeia_terminal` 8) are the primary narrative surface and are referenced constantly by quests, vendor signatories, and org desks.
- **Cut CS.0b extracts `npc.*` and `lore.*` entries** from the 5 port neighborhood packs into 5 dedicated campaign-seed packs under `crates/tacsim/content/cast/`:
  - `tacsim.cast.ceres_freeport` (`cast/ceres_freeport.json`, requires `neighborhood.ceres_freeport@5`)
  - `tacsim.cast.stickney_yards` (`cast/stickney_yards.json`, requires `neighborhood.stickney_yards@5`, `tacsim.cast.ceres_freeport@1` for shared `corridor_trade` / `tycho` lore pools)
  - `tacsim.cast.aitken_highport` (`cast/aitken_highport.json`, requires `neighborhood.aitken_highport@2`, `tacsim.cast.ceres_freeport@1`)
  - `tacsim.cast.voltaire_anchorage` (`cast/voltaire_anchorage.json`, requires `neighborhood.voltaire_anchorage@2`, `tacsim.cast.ceres_freeport@1`)
  - `tacsim.cast.ligeia_terminal` (`cast/ligeia_terminal.json`, requires `neighborhood.ligeia_terminal@2`, `tacsim.cast.ceres_freeport@1`)
- **Cross-Reference & Kept-World Handoff**:
  - `room` inside `cast/<station>.json` sets `room_pack: "neighborhood.<station>"`.
  - `crates/tacsim/src/port/relationships.rs` (`desk` lookup for `npc.oduya` and `npc.qureshi`), vendor `signatory` references in `lane/ceres_phobos.json`, and `npc_pack` in `quests/*.json` resolve against `tacsim.cast.<station>` with a fallback check to `neighborhood.<station>` so kept pre-split worlds continue to resolve.
  - For kept dev worlds that already latched `npc.*` under `neighborhood.<station>`, `mush-content` gains a latch-adoption rule so moving an `npc.*` entry from a required `neighborhood.*` pack into `tacsim.cast.*` adopts the existing `ObjId` in the new latch instead of calling `NpcApplier::retire` (preserving NPC memories and `ObjId` identity).
- **Mixed-Kind Pack Preservation**:
  - Because packs like `crates/tacsim/content/passengers/stickney.json` contain `npc.leila_mensah_kovacs`, `passenger.leila`, **and** `quest.leila_second_reading` in one file, the studio never assumes a pack is single-kind: editing an NPC or a quest in a mixed pack preserves all sibling entries untouched.

### `D-34-4` (Ruling on `Q-34-4`): Keep `CrewDuty` Enum Fixed
`CrewDuty` stays `{ mining, combat, recovery }` and `HireTerms` always emits `"schema": 1`. Natural-language briefs like *"maintenance/repair specialist"* map mechanically to `duties: ["recovery"]` while expressing maintenance/repair in `persona`, `background`, `goals`, and `knows`.

### `D-34-5`: Compiled-In `include_str!` vs. Adding New Packs
All shipped content files are compiled into the binary via `include_str!` in `crates/tacsim/src/definitions/content.rs`, `crates/tacsim/src/quest/packs.rs`, `crates/tacsim/src/detachment/crew/content.rs`, `crates/tacsim/src/port/passenger/content.rs`, and `crates/tacsim/src/relay_content.rs`.
1. **Default Authoring Path (Zero Rust Edits)**: Creating a new NPC, crew offer, passenger, or quest in the studio **defaults to adding the entry into the existing compiled-in pack for that station/family** (e.g., `cast/<station>.json`, `crew/ceres.json`, `passengers/ceres.json`, `quests/first_steps.json`). Downloading and overwriting that file in `crates/tacsim/content/` requires **no new `include_str!` constant**.
2. **New Pack File Path**: When staff explicitly create a *new* pack file (e.g. `crew/stickney.json`), the studio's Export drawer downloads the signed `<slug>.json` file and renders a copy-pasteable 3-line Rust registration snippet for `crew/content.rs` / `quest/packs.rs` / `definitions/content.rs` in `requires` order.

### `D-34-6`: Digest Pin Blessing & Historical `-vN.json` Preservation
1. **CI Digest Pins**: Tests in `crates/tacsim/tests/shard_2/content.rs` (`DIGESTS`), `crates/tacsim/src/quest/packs.rs`, `crates/tacsim/tests/stations.rs`, `crates/tacsim/tests/sites.rs`, and `mush-server/tests/split_boundary/relay_registry.rs` pin exact hex digests.
   - We extend the `TACSIM_WRITE_CONTENT=1` test-blessing mode and display the updated `(pack_id, digest_hex)` pin line directly in the Studio Export drawer.
2. **Historical `-vN.json` Rule**:
   - The tree keeps historical pack snapshots (`passengers/stickney_v1.json`, `relays/luna_ceres-v1.json`, `relays/manufacture-v1.json`) because upgrade and replay tests (`HISTORICAL_JSON` in `knowing.rs`, `relay_registry.rs`) boot on `v1` and test upgrading to `vN`.
   - Whenever an export bumps `manifest.version` from `vN` to `v(N+1)`, **any existing `-vN` / `_vN` file is untouched**, and if the pack's previous version is referenced by an upgrade test, the Export drawer offers to download the prior bytes as `<slug>_vN.json` alongside the bumped `<slug>.json`.

### `D-34-7`: One Closed Invoke Family (`InvokeRequest::ContentStudio`)
`InvokeRequest` in `mush-proto` is a closed enum mirrored in `mush-core`'s `MODULE_INVOKE_FAMILIES` and covered by doc 33 (`33-gateway-generations.md`) IPC golden frames. We add **one** variant:
- `InvokeRequest::ContentStudio(ContentStudioRequest)` (`service: "content_studio"`)
- `InvokeResult::ContentStudio(ContentStudioResponse)`
with a typed `op` enum inside `ContentStudioRequest`:
- `Catalog`: Returns the live reference index (`ports`, `rooms` with authored + live `details`, `npcs` with `code_bound` flags, `lore_pools`, `goods`, `interfaces`, `command_refs`, and pack latch states).
- `Check { pack_kind, content, live_check }`: Runs `pack_digest` over `RulesPack::canonical_value` (or standalone table validators), infers `manifest.requires`, and runs applier `.check()` against the live world when `live_check: true`.
- `Publish { content, relocate_npcs }`: Applies a `campaign_seed` pack live via `mush_content::module::apply_pack` (or registers a `rules_definition` row for next boot).
- `Assist(ContentAssistRequest)`: Runs a structured LLM co-authoring pass.

### `D-34-8`: `LlmFeature::ContentAssist` & `BUILDER_LLM` Gate
`crates/mush-llm/src/service.rs` adds `LlmFeature::ContentAssist` (`route_key: "content_assist"`).
- Gated on `mush_atlas::is_staff(ctx.w, actor)` and the `BUILDER_LLM` power.
- Uses `mush_llm::structured::call_structured` at `Priority::Interactive`, recording `actor`, `prompt_hash`, token usage, and cost under `"content_assist"`.

### `D-34-9`: Live-World Semantics & Safety Lints in the UI
The UI enforces five live-world invariants rather than hiding them:
1. **Updates Never Move a Live NPC by Default**: `NpcApplier::update` preserves a live NPC's `location`, memories, and inventory. When staff change an already-latched NPC's `room` / `room_pack` in the UI, the inspector displays a notice (*"Pack update changes initial seed room only; live `#<ObjId>` stays in `<current_room>`"*) with an explicit checkbox on **Publish to Running World**: `[x] Also move live NPC #<ObjId> to <new_room> now`.
2. **Latched Entry IDs Are Locked against Rename**: Because `CrewApplier`, `PassengerApplier`, and `QuestApplier` refuse moving a role/quest to another NPC (`"a crew offer cannot move to another person"`) and `CrewApplier::retire_check` refuses removing a live crew role through a pack, any entry ID already present in a shipped or latched pack has its `slug` / entry key locked (`🔒`). Only newly created draft cards can edit their slug before first publish/export.
3. **Code-Bound Entries Are Badged & Protected**: `crates/tacsim/src/port/relationships.rs` binds `npc.oduya` and `npc.qureshi` by entry ID to org desks; `lane/ceres_phobos.json` binds 13 station NPCs as vendor `signatory`s. `Catalog` marks these entries `code_bound: true` with their binding sites, and the UI disables delete/retire on them.
4. **Live Detail & Registry Validation**:
   - `Observation` quest checkpoints check both the authored room's `details` map and the live room object's `atlas:detail:<kw>` attribute (catching details removed in-game by staff).
   - `profile.interfaces` and `profile.command_refs` are validated against the server's registered interface providers (`hire`, `passenger`, `quest`, `help`, etc.) and public command registry (`+crew`, `+passage`, `+ship`, `+quest`, etc.), preventing silent drops during `NpcApplier` apply.
5. **Explicit Retire State**: The pack selector and card grid badge packs and entries that are `Retired` on the attached world, and warn when removing an entry from a pack will execute `retire_check` / `retire` on live apply.

---

## 3. Exact Data Schemas & Visual Editor Design by Slice

### 3.1 Deliverable 1 (Slice CS.1): NPC & Station Cast Studio (`cast/*`, `crew/*`, `passengers/*`)

#### Authoritative Rust Bounds (`crates/mush-npc/src/profile.rs` & `content.rs`)
Every form control and LLM schema validator enforces the exact tree constants:
- **Top-level `NpcDef`**:
  - `name: String`
  - `room: String` (entry ID of room, e.g. `"room.c10_slack_line"`)
  - `room_pack: Option<String>` (e.g. `Some("neighborhood.ceres_freeport")`)
  - `tier: NpcTier` (`"major"` | `"minor"` | `"extra"`, default `"minor"`)
  - `lore: Vec<LoreDef>` — each `{ tier: "public" | "guarded" | "secret", subject: String, fact: String, source: String }`
  - `tags: Vec<String>` — shared lore pool tags (`corridor_trade`, `port_ceres`, `tycho`, etc.)
- **Nested `ProfileDef` (`MAX_AGENT_PROFILE_BYTES = 256 KiB`)**:
  - `persona: String` (≤ `8 KiB` / `8,192` bytes)
  - `background: String` (≤ `32 KiB` / `32,768` bytes)
  - `speech_style: String` (≤ `4 KiB` / `4,096` bytes)
  - `goals: Vec<String>` (≤ `64` items, ≤ `2 KiB` each, non-empty)
  - `knows: Vec<String>` (≤ `64` items, ≤ `2 KiB` each, non-empty)
  - `does_not_know: Vec<String>` (≤ `64` items, ≤ `2 KiB` each, non-empty)
  - `fallback: Vec<String>` (**list of up to `32` canned lines**, ≤ `2 KiB` each)
  - `watch: Vec<String>` (informant watch subjects, ≤ `128` items, ≤ `256` bytes each)
  - `cooldown_s: Option<u32>` (tier default when `None`: Major 5s, Minor 15s, Extra 60s)
  - `daily_tokens: Option<u64>`
  - `enabled: Option<bool>` (default `true`)
  - `interfaces: Vec<String>` (1–32 bytes each, multi-select from registered providers)
  - `command_refs: Vec<String>` (1–64 bytes each, lowercase command names without switches/args, multi-select from registered commands)
- **Coupled Role Entries & Pack-Level Lore Pools**:
  - **`crew_offer` (`OfferDef`)**: `{ npc: CanonicalId, terms: { schema: 1, port: CanonicalId, mission_scope: "subordinate_operator" | "detachment_commander", duties: ["mining" | "combat" | "recovery"] } }`. Note: `crew/ceres.json` requires `neighborhood.ceres_freeport@4` only.
  - **`passenger` (`PassengerDef`)**: `{ npc: CanonicalId, origin: String, destination: String, opening_fare_minor: i64, max_fare_minor: i64, funding_minor: i64 }`.
  - **`lore` (`LorePoolDef`)**: `{ tag: String, entries: Vec<LoreDef> }` — editable in a **Shared Lore Pools** drawer inside the NPC Studio so staff can manage station lore pools alongside the 52 station NPCs that subscribe to them via `tags`.

#### Real 11-Port / Station Catalog for Dropdowns
The **Port / Station selector** is hydrated from `tacsim.lane.ceres_phobos` (v15) and `station_packs()`:
- **5 Habited Station Ports**:
  1. `ceres` → `ceres_port` (`neighborhood.ceres_freeport`, 13 rooms, 13 cast NPCs)
  2. `stickney` → `stickney_port` (`neighborhood.stickney_yards`, 12 rooms, 9 cast NPCs)
  3. `luna` → `luna_port` (`neighborhood.aitken_highport`, 12 rooms, 13 cast NPCs)
  4. `deimos` → `deimos_port` (`neighborhood.voltaire_anchorage`, 11 rooms, 9 cast NPCs)
  5. `titan` → `titan_port` (`neighborhood.ligeia_terminal`, 12 rooms, 8 cast NPCs)
- **6 Surface Extraction Site Ports** (1 pad room each, `room.pad`):
  6. `commons` → `commons_port` (`neighborhood.freeport_commons`, Ceres)
  7. `kerwan` → `kerwan_port` (`neighborhood.kerwan_floor`, Ceres)
  8. `ahuna` → `ahuna_port` (`neighborhood.ahuna_flank`, Ceres)
  9. `juling` → `juling_port` (`neighborhood.juling_shadow`, Ceres)
  10. `occator` → `occator_port` (`neighborhood.occator_rim`, Ceres)
  11. `rheasilvia` → `rheasilvia_port` (`neighborhood.rheasilvia_commons`, Vesta)

#### Interactive LLM Workflow in Slice CS.1
1. **Prompt Bar ("Brief → Complete Card")**:
   - Staff enter: *"I want to create a new crew member who specializes in maintenance/repair, based on Ceres, heavily experienced"*
   - `ContentAssist` grounds the call with `ceres` station rooms, existing Ceres cast names, lore pool tags (`corridor_trade`, `port_ceres`), and role templates, returning a complete `NpcDef` + `OfferDef` (`schema: 1`, `port: "ceres"`, `duties: ["recovery"]`, `interfaces: ["hire", "help"]`, `command_refs: ["+crew", "+ship"]`, `fallback: [...]`).
2. **Dropdown Relocation ("Move Ceres → Stickney / Luna / Deimos / Titan")**:
   - Selecting `stickney` updates `room_pack` to `"neighborhood.stickney_yards"`, populates the `room` dropdown with Stickney's 12 rooms (`room.p10_shift_board`, `room.p09_commissary`, etc.), updates `terms.port` / `origin`, updates `tags` (`port_ceres` → `port_stickney`), updates `manifest.requires`, and offers a one-click **Adapt prose to Stickney (`↻`)** button to rewrite station/room references in `background`, `knows`, and `fallback[]`.
3. **Per-Section Lock (`🔒`) & Regenerate (`↻`)**:
   - Individual `↻` buttons on `persona`, `background`, `speech_style`, `goals[]`, `knows[]`, `does_not_know[]`, `fallback[]`, and `lore[]`.

---

### 3.2 Deliverable 2 (Slice CS.2): Quest Studio (`quests/*.json` & Mixed Packs)

- **Quest Pack & Cross-Pack DAG View**:
  - Visualizes both **intra-pack quest chains** (`prerequisite: "open"` | `"paid_passage"` | `"completed:<pack>/<quest>"`, plus `choice: { group, path }` branches) **and** **cross-pack `manifest.requires` pins** (e.g., `tacsim.quests.survey_work` requiring `tacsim.quests.first_steps`, and `passengers/stickney.json` embedding `quest.leila_second_reading` with default `prerequisite: "paid_passage"`).
- **Giver NPC Picker & Profile Bridge**:
  - Lists all 57+ NPCs across `cast/*`, `crew/*`, and `passengers/*`, showing their current quest count against `MAX_QUESTS_PER_NPC = 8` and whether their `profile.interfaces` includes `"quest"` and `command_refs` includes `"+quest"`.
  - If staff attach a quest to a station NPC whose profile lacks `"quest"` in `interfaces`, a warning pill offers a one-click patch to that NPC's card in `cast/<station>.json`.
- **Checkpoint Editor (1..8 Checkpoints, `MAX_CHECKPOINTS = 8`)**:
  - **Live Room & Detail Picker**: Picking `room_pack` + `room_entry` populates the `detail` dropdown with that room's `details` keywords and verifies against the live world's `atlas:detail:<kw>` attribute on `Check`.
  - **All 8 `SourceObjective` Forms**:
    - `observation` (default when `objective` is omitted in JSON; requires non-empty `detail` ≤ 128 B)
    - `coop_enrolment`
    - `survey_commission`
    - `trade { origin, destination, commodity, quantity }` — `origin` and `destination` use the 11-port venue dropdown; `commodity` uses the real `goods.json` v3 IDs (`metals`, `volatiles`, `structural_alloys`, `industrial_parts`, `instrument_boards`, `carbon_feedstock`, etc.).
    - `passenger { npc_pack, npc }`
    - `contact { npc_pack, npc }`
    - `paid_passage { npc_pack, npc, origin, destination }`
    - `survey { mode: "scan" | "sound" }`

---

### 3.3 Deliverable 3 (Slice CS.3): Component & Rules Catalog Studio

Reuses `tacsim::design::check`'s `requires` inference, `pack_digest` over `RulesPack::canonical_value`, and `Catalog::build_fielded` verification:
- **Signed Rules Packs (`kind: "rules_definition"`)**:
  - `weapons/*.json`, `ammunition/*.json`, `sensors/*.json`, `cores/cores.json`, `hardware/governed_hardware.json`, `resolution.json`.
  - Includes a live **Weapon Hit & Dispersion Curve Preview** computed from `resolution.json`.
- **Standalone Typed JSON Documents**:
  - `market/goods.json` (v3): Edits the 12 goods (`metals`, `volatiles`, `structural_alloys`, `industrial_parts`, `propellant`, `provisions`, `instrument_boards`, `carbon_feedstock`, `metal_ore`, `ice_ore`, `carbon_ore`, `strategic_ore`) with their `measures` (`si.mass_kg` in kg, `si.volume_l` in litres), `stowage` (`stow.bulk`, `stow.ore`, `stow.propellant`, `stow.provisions`), `ledger_unit`, `quote_unit`, and `reference_major`.
  - **Built-In Hold Stowage Calculator**: Uses the file's own `si.mass_kg` and `si.volume_l` per `t` (or `crew_day`) to display the exact tonnage/units at which a Dray (`100 t / 120 m³`) or Dray-P (`86 t / 103.2 m³` + `12 t / 13.2 m³` hopper) **weighs out** vs. **cubes out**.
  - `tools/*.json` & `resources.json`: Card editors validated by `tools_table()` and `ResourceTable::from_json()`.

---

### 3.4 Deliverable 4 (Slice CS.4): Corridor (`lane/ceres_phobos.json`) & Relay Studio

Matches the exact flat-entry structure of `crates/tacsim/content/lane/ceres_phobos.json` (v15) and `crates/tacsim/content/relays/`:
- **Top-Level Entry Kinds in `tacsim.lane.ceres_phobos`**:
  1. `region` (`sol`) — system extent and background.
  2. `body` (16 bodies) — Keplerian `elements` (read-only Horizons tables) + surface geology tables (`ceres`, `vesta`).
  3. `location` (12 locations: `ceres_freeport`, `stickney_yards`, `aitken_highport`, `voltaire_anchorage`, `ligeia_terminal`, `kirkwood_depot`, `freeport_commons`, `kerwan_floor`, `ahuna_flank`, `juling_shadow`, `occator_rim`, `rheasilvia_commons`).
  4. `route` (20 routes: `ceres_stickney`, `stickney_ceres`, `ceres_aitken`, `aitken_voltaire`, `aitken_ligeia`, site hops, etc., with `legs`, `drive_class`, `propellant`).
  5. `port` (11 ports: `ceres_port`, `stickney_port`, `luna_port`, `deimos_port`, `titan_port`, plus the 6 extraction site ports `commons_port` .. `rheasilvia_port`).
- **Rich Nested Inspector for `port` Entries**:
  - **Rooms & Gallery Mapping**: Dropdowns mapping port function keys (`arrivals`, `registry`, `clearing`, `exchange`, `chandlery`, `dockmaster`, `customs`, `harbourmaster`, `receiving`, `commissary`, `yard`, `spine`) to `neighborhood.<station>.room.<id>`.
  - **Organizations (`orgs[]`)**: `id`, `name`, `capital_major`, `faction`, `raises`.
  - **Vendors (`vendors[]`)**: `org`, `storefront` (neighborhood `item.*` picker), `agent`, `signatory` (`cast`/`neighborhood` `npc.*` picker), `stock[]`, `asks[]`, `bids[]` (with `refresh: { per_ic_day, cap }`), and `mill[]` (`ore` → `into` conversion ratios, `batch_t`, `per_ic_day_t`).
  - **Programmes (`programme`) & Vehicle Leases (`vehicles`) & Licences (`licence`)**: Edits `coop.first_captain`, `concord.relief`, `occator.rig_lease`, and prospecting licence fees.
  - **Surface Extraction Site Config (`site` inside site ports)**: Edits `tenure`, `papers` (e.g. `occator_outer`), `blocks` (`A`–`P` with `worked_permille`, `prospectivity`, `tenure`, `holder`, `paper`), and `yard` (`occator` autonomous rig dispatcher).
- **Relay Pack Cascade (`relays/luna_ceres.json` & `relays/manufacture-v1.json`)**:
  - `tacsim.relay.luna_ceres` requires `tacsim.lane.ceres_phobos` at an exact version. Whenever `lane/ceres_phobos.json` bumps its version in the studio, the **Cascade Re-signer** automatically updates `relays/luna_ceres.json`'s `manifest.requires` and `manifest.version`, re-computes its canonical digest, and exports both files (plus historical `-vN` snapshots if applicable) together.

---

## 4. Implementation Slicing Plan

| Cut | Scope | Key Deliverables |
| :--- | :--- | :--- |
| **Cut CS.0a** | **Protocol, Catalog, Signing & Live-Publish Seam** | 1. Add `InvokeRequest::ContentStudio` / `InvokeResult::ContentStudio` in `mush-proto` (with golden frame test) and `LlmFeature::ContentAssist` in `mush-llm`.<br>2. Implement `Catalog`, `Check`, and `Publish` in `tacsim`: live index of 11 ports, rooms (+ authored & live `details`), 57+ NPCs (+ `code_bound` flags), lore pools, goods, registered `interfaces` & `command_refs`, canonical digest signing (`pack_digest`), and live `mush_content::module::apply_pack` publishing.<br>3. Shared `web/src/lib/content/` state engine, `History` undo/redo, and sub-tab bar in `WorldgenPanel.svelte`. |
| **Cut CS.0b** | **Station Cast Split (`cast/<station>.json`)** | 1. Extract the 52 station NPCs and 20 lore pools from the 5 port `atlas/neighborhood.*.json` files into `crates/tacsim/content/cast/{ceres_freeport,stickney_yards,aitken_highport,voltaire_anchorage,ligeia_terminal}.json`.<br>2. Add latch-entry adoption in `mush-content` so kept worlds adopt existing NPC `ObjId`s without retiring/losing memories.<br>3. Update `port/relationships.rs`, `lane/ceres_phobos.json` signatories, `quest/packs.rs`, and station digest pins. |
| **Cut CS.1** | **Deliverable 1: NPC & Cast Studio (`cast`, `crew`, `passengers`)** | 1. `NpcStudio.svelte` card grid & inspector for all 52 station NPCs, 20 lore pools, 2 crew, and 3 passengers (preserving mixed-pack entries like `stickney.json`).<br>2. Full `ProfileDef` editor with exact byte/count bounds, `fallback[]` list editor, `watch[]`, `lore[]`, `HireTerms` (`schema: 1`), and `PassengerDef` fare controls.<br>3. `ContentAssist` LLM brief-to-card drafting, per-field lock (`🔒`) + regenerate (`↻`), and 11-port relocation cascade (+ optional live NPC `ObjId` move). |
| **Cut CS.2** | **Deliverable 2: Quest Studio (`quests/*.json` & Mixed Packs)** | 1. `QuestStudio.svelte` with quest card grid, intra-pack & cross-pack prerequisite/choice DAG, and Giver NPC picker (`MAX_QUESTS_PER_NPC = 8`, code-binding & interface lints).<br>2. Checkpoint builder (1–8) with live room + `atlas:detail:<kw>` dropdowns and sub-forms for all 8 `SourceObjective` kinds.<br>3. LLM quest drafter and per-checkpoint instruction regenerator. |
| **Cut CS.3** | **Deliverable 3: Component & Rules Catalog Studio** | 1. `CatalogStudio.svelte` for `weapons`, `ammunition`, `sensors`, `cores`, `hardware`, `resolution`, `tools`, `market/goods.json`, and `resources.json` built over `design::check`.<br>2. Weapon hit/dispersion curve visualizer and Dray / Dray-P stowage density calculator (`si.mass_kg` vs `si.volume_l`). |
| **Cut CS.4** | **Deliverable 4: Corridor (`lane`) & Relay Studio** | 1. `CorridorStudio.svelte` for `lane/ceres_phobos.json` (`port` inspector with nested `orgs`, `vendors`, `asks`/`bids`/`mill`, `programme`, `vehicles`, `licence`, `site` blocks `A`–`P`, `location`, `route`) and `relays/{luna_ceres,manufacture-v1}.json`.<br>2. Automatic `lane` → `relay` dependency cascade re-signer and historical `-vN.json` export helper. |
