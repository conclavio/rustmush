# Doc 20 cut notes

The per-cut record for [20-surface-mining.md](20-surface-mining.md) §13: what each cut landed, what
it deferred and why, every pin it re-blessed, how it was verified, the surface later cuts build on,
and what doc 20 itself must now say "as built". One heading per cut, in the order they land. SM.0 is
the document itself (Drafts 0.1 to 0.4, commits `11c50e84` and `f3df3205`, 2026-10-19).

## SM.1

**The ground** (doc 20 §13, the SM.1 row): the keyed geology, the fourth durable key family and its
pin, the site ground cache, the atlas elevation fix. Built 2026-10-19 on main from `f3df3205`, beside
the astronomy session's cut A.8 in the same checkout (A.8a `b0d036d3` and A.8b `7d192600` landed
during the gate; this cut's diff was geology-only against `7d192600` when staged). Judged
**ship-with-notes**. One commit, pushed to origin/main. It also closes a red legacy-handler gate at
`f3df3205`: the `+chart` stanza that cut A.3 left out.

### What landed

1. **The geology secret** — `crates/mush-core/src/geology_key.rs` (new; `pub mod geology_key` and the root re-export in `crates/mush-core/src/lib.rs`): `GeologySecret { version, key, mac }` keyed once in `new` (version 0 and the all-zero key refused; a `new_from_slice` failure is `GeologySecretError::Key`, never a third budgeted `expect`), `fingerprint()` = SHA-256 of the 32 key bytes, `mac()` cloning the pre-keyed state; `GeologyMac::draw` = plain HMAC-SHA256; `MAX_GEOLOGY_SECRET_VERSIONS = 1`; `GeologySecretError { ZeroVersion, ZeroKey, Key, Retained }`; both Debug forms redacted, neither type serde. Four unit tests.
2. **The fourth durable key family** — `crates/mush-config/src/geology_keys.rs` (new; `pub mod geology_keys` in `lib.rs`): `GeologyKeyFamily` (`geology-key-` / `geology-ring-`, `MAX_VERSIONS` 1, `build_ring` refusing any retained key), `DurableGeologyKeyStore { new, directory, load_or_create }` with **no rotate**; `GeologyKeyStoreError`. Four unit tests (restart reuse, independence in a shared directory under a resolution rotate, a hand-written retained manifest refused, Debug carries no key bytes).
3. **The config key** — `crates/mush-config/src/config.rs`: `TacsimGeologyCfg { key_dir: Option<PathBuf> }` (`deny_unknown_fields`) as `TacsimCfg.geology`, `Config::geology_key_dir() -> Option<PathBuf>` with deliberately **no default** (absent = the mining layer disabled), the key-table row, and the test `geology_key_directory_has_no_default_and_refuses_a_misspelt_key`. `crates/mush-config/tests/shipped_configs.rs`: the template's geology directory is `Some`, under `data_dir`, and distinct from both the offers and the resolution directories.
4. **The shipped configs and the operator's page** — `deploy/instance/mushd.toml.tmpl` (`[tacsim.geology] key_dir = "__DATA_DIR__/geology-keys"` with its three-line comment), `mushd.toml.example` (the header with `key_dir` commented out), `deploy/dev1/deploy.sh` and `deploy/dev1-split/deploy.sh` (the comment row and `"$ROOT/data/geology-keys"` in the 0700 `install -d`), `deploy/dev1/dev1mush.service` and `deploy/dev1-split/dev1mush-gateway.service` (`geology-keys/` in the provisioned-directory comments), `docs/ops.md` (the geology-key paragraph under "Private action-protocol keys": no default, never rotated, back it up with the store, never the protocol families' directory). Nothing deployed.
5. **The `i16` elevation fix** — `crates/mush-atlas/src/record.rs`: `RegionRec::validate` refuses `datum_cm ± relief_cm` outside `-32768..=32767` with `RecordError::Value("region datum_cm {d} ± relief_cm {r} leaves the cell elevation range of -32768 to 32767 cm")` in `i64` arithmetic; `crates/mush-atlas/src/window.rs`: the private `saturating_i16(i64)` used at both elevation casts (the tile-mean at :470 and the background cell at :555), so a low stays a low. Tests `a_region_datum_past_the_cell_range_is_refused` (exact bounds) and `a_deep_background_cell_saturates_low` (a `datum_cm −40_000` region composites to `i16::MIN`). No atlas pin moved.
6. **The keyed geology function** — `crates/tacsim/src/surface/geology.rs` (new; `pub mod surface` in `crates/tacsim/src/lib.rs`): `OreFamily` and `Word` (the one word set; `code/id/label`, `pocket_range` 0/1–4/5–8/9–16, `tonnes_range` 0/3–6/6–12/12–24, `of_count`, `shifted`); `GeologyTableWire` (strict at every level) and `GeologyTable::from_wire/wire` refusing a missing province, field or pocket row, a `barren` pocket row, a barren weight in a pocket row, a `field.barren` row weighting any ore word (`BarrenProvinceField`, decision 13 held by the table), and any row not summing to 1,000; `BodyGeology { body, radius_km 1..=10,000, table }` and `Lattice::of_radius` (2π ≈ 710/113; Ceres 73,824 × 36,912, Vesta 41,304 × 20,640; x wraps by the level's own width through `norm_pocket`/`norm_field`, y never); the draw `GEOLOGY_DOMAIN ‖ body ‖ 0x00 ‖ level ‖ x i64 LE ‖ y i64 LE ‖ family` read as four LE `u64`s (`as_chunks::<8>`, no indexing); the passes `province_at`, `field_ground` (the shift applied only where the province word is not barren — decision 13's guard, in one place) and `pocket_at` (selection by keyed rank in family order, at most two families a pocket, so `counts[f]` is the number of non-barren pockets by construction); `normalise_pocket`. Truth types `ProvinceGround`, `PocketRow`, `FieldGround` derive no serde. Lattice figures recomputed by hand by the conformance reviewer and by me.
7. **The reading rule, version 1** — `crates/tacsim/src/surface/reading.rs` (new): `READING_RULE_VERSION = 1`, `Footing { Rock, Regolith, Ice }` with `of(TerrainClass)` and `name`, `FieldReading { Exact, Band, NoReturn }` with `words()` ("fair–rich", en dash) and `pockets()`; `read_r0`, `read_r1` (rock exact; regolith a two-word band holding the truth, its side one keyed level-3 draw per (field, family); ice no return), `read_r2`.
8. **The key's boot, pin and disabled state** — `crates/tacsim/src/surface/key.rs` (new): `GEOLOGY_PIN_KEY = "tacsim:geology"`, `GeologyBoot { NotConfigured, Key, Unreadable }`, `GeologyDisabled { NotBooted, LayerOff, NotConfigured, KeyUnreadable, PinUnreadable, Mismatch }` with the brief's reason texts verbatim (12-character prefixes by `chars().take(12)`), `GeologyStatus`, `GeologyPinRecord { fingerprint, key_version }` (`deny_unknown_fields`), `GeologyPinState { Absent, Present, Unreadable }`, `load_geology_pin` (64 lowercase base-16 and a non-zero version, else Unreadable, logged), `live(&World)`, `status(&World)`, the boot pass `boot(ctx)` (LayerOff → NotConfigured → KeyUnreadable → Absent pins through `store::put_geology_pin` → Present compares → Mismatch and Unreadable write nothing; the secret is taken out of the slot and only its `GeologyMac` stays), and `geology_line` for the readiness report. `crates/tacsim/src/store.rs`: the document-table row, `TacsimStore.geology_pin`, the `load` arm before `META_KEY` (the R8 rule), `put_geology_pin` on `put_port_map`'s `push_op` pattern.
9. **The slot and the composition-root door** — `crates/tacsim/src/surface/mod.rs` (new): `GeologyState` (RAM only; `boot`, `status`, `mac`, `tables`, `sites`; default `Disabled(NotBooted)`), `MAX_BODY_GEOLOGIES = 32`, `install_geology` (refused with a warn once the boot pass has run), `install_body_geology` (replaces a body and **drops its cached sites**), `body_geology`, the re-exports. `crates/tacsim/src/lib.rs`: `reg.state(surface::GeologyState::default())` beside the `TacsimStore` slot, `surface::commands::register`, and `surface::key::boot(ctx)` right after `*slot = loaded;`.
10. **The key at the production root** — `crates/mush-server/src/boot.rs`: the geology key loaded in `build_engine` after the decision ring, only when `[tacsim].enabled`, **never with `?`** (`NotConfigured` / `Key` / `Unreadable(error.to_string())` with the info/error logs), and handed over through `tacsim::surface::install_geology` after `mush_tactics::ground::configure` and before `modules::boot_modules`, with a warn when no slot takes it.
11. **The readiness line** — `crates/tacsim/src/port/readiness.rs`: `Binding::Geology`, pushed right after the sky line; `crates/tacsim/src/port/coop.rs`: `gated(Binding::Geology) == false` (mining's key closes no trade-run desk), the unit test's `EVERY` at 17 with `Geology` ungated. Texts: `geology: key v{n}, fingerprint {12}, pinned; reading rule v1.` (Ready), `geology: disabled: not booted yet.` / `not composed.` (Missing), `geology: disabled: {reason}.` (Flagged).
12. **`@geology`** — `crates/tacsim/src/surface/commands.rs` (new): `@geology <body> <x>,<y> [province|field|pocket]` in its own ReadOnly bracket, staff checked before anything is parsed, refusals in the brief's order (`Permission denied.`, the usage line, `Geology is disabled: {reason}.`, `No geology table is loaded for {body}.`, `That point is off {body}: y runs from {south} to {north−1}.`), the header and three lines built by the pub `geology_lines` (a barren pocket reads `barren`, "1 pocket" singular), one `sys_to_actor` message a line, a pure read. `crates/mush-contract-tests/legacy-handlers.toml`: the `@geology` read-only stanza (and the `+chart` stanza A.3 omitted, see the pins).
13. **The site ground: the geology table and the composite LRU** — `crates/tacsim/src/surface/ground.rs` (new): the constants (`SITE_CELLS` 256, `SITE_CELL_CM` 1,000, `SITE_POCKETS` 64, `SITE_FIELDS` 8, `SITE_BLOCKS` 4, `SITE_CACHE_CAPACITY` 16), `SiteGeologySpec { body, origin (a field corner), shifts: [i8; 16] }`, `SiteGeology` (two bytes a family a pocket; `pocket`, `field`, `totals`, `digest` over the documented layout), `site_geology` (refusing a non-corner origin, a shift past ±3, a body mismatch and rows past a pole; the 64 fields through `field_ground` under their blocks' shifts, so the site table **is** the body function), `SiteGroundKey { region, revision, key_version }`, `SiteGround { composite, geology }` (no serde), `site_revision` (the first eight bytes of SHA-256 over the **whole** serialized `CompositeInput` plus the spec), `capture_site`, `build_site_ground` (refusing any window but 256 × 256 at 10 m with "a site's ground is 256 by 256 cells of 10 m"), `SiteGroundCache` (`get` touches, `insert` evicts the least recent at 16, `forget_body`), and the synchronous door `site_ground_sync` (refuses while `live` refuses and with no table; capture, key, hit or build-and-insert).
14. **The acceptance tests** — `crates/tacsim/tests/geology.rs` (new, 15 tests on the production roster through one `boot_keyed` helper): the table's refusals and round-trips; the lattice pins; the §3.2 province sentences over all 1,568 provinces of each body; **the property test** (3,907 blocks, 1,000,192 pockets in about 9.5 s: every field's count is its non-barren pockets and `Word::of_count` its word, ≤ 64 a block a family, ≤ 2 families a pocket, barren ⇔ 0 t, tonnes in range, a province-barren family barren under every shift); same key same ground, another key another; the reading rule over 10,000 fields; **the whole-site pin** (geology digest `67bf968c2b5fbe28d1ee2eb55227b88620efb73a194b8bf194e8f46697459eeb`, composite digest `c6a2ff3e4b3d5b2ed937effe04ed3b82598f90ca2d072f13c05a6af7c803eff6`, totals metals (326, 2,979 t), volatiles (412, 3,945 t), carbon feedstock (172, 1,325 t), strategic elements (0, 0), with every site field's count equal to its own site pockets); the site table is the body function with at least one province-barren pair under the +1 block; the site key moves with every input (eight distinct values); the sixteen-entry LRU; the site refusals; the first boot pins and the next compares (exactly one pin write, Ready, `install_geology` refused after boot, a reboot writes nothing, the wrong key reads the exact mismatch text and `live` errs `Mismatch`); every disabled reason on the readiness line (never gated; three unreadable pin bodies, none rewritten); `@geology` against test-built strings at four points; the synchronous door. `crates/mush-server/tests/boot_parity.rs`: the knob-table row and `sm1_the_geology_key_is_pinned_and_a_wrong_one_disables_the_layer` (Ready and the key file provisioned; Ready again; a fresh directory Flagged with the mismatch; a garbage manifest Flagged with the store's error and the boot still succeeds; no `key_dir` Flagged not configured). `crates/mush-contract-tests/tests/truth_serialization_policy.rs`: seven `assert_not_impl_any!` lines (the two secret types and the five truth types).

### Deferred, and why

- **The off-loop `Effect::ModuleAsync` driver** (lead ruling 4, the brief's decision 6): deferred to SM.4, its first consumer — with no consumer in this cut it would land untested by a real path. This **narrows doc 20 §13's SM.1 row** ("the off-loop site composite LRU"): the LRU, the pure builds and the synchronous door land; the driver does not. Carried into SM.4's brief: the composite half needs no key (the atlas's own `CompositeJob`/`run_async` already runs off-loop); the geology half needs the key, which a host-held runner keyed in `build_engine` can hold; the key never travels in a JSON payload.
- **`@geology <body> <x>,<y> [level]` over the body-fixed lattice** (lead ruling 2): doc 20 §3.2's `@geology <site> <addr>` needs sites, which are SM.2's; the site-address form arrives there.
- **No lane change** (lead ruling 1; D-20-2, Q-20-10): `GeologyTable` and its wire land here; the only tables are the test's two, reproducing §3.2's sentences; SM.2 authors Ceres's and Vesta's as lane data and replaces `install_body_geology`'s test-only caller. The SM.2 builder must author `field.barren = {barren: 1000}` for both bodies (the validator now refuses anything else).
- **The key** (lead ruling 3; D-20-4, Q-20-12, §16): the secret type in `mush-core`, the family in `mush-config` with `MAX_VERSIONS = 1` and no rotate, the directory from `[tacsim.geology].key_dir` with **no default**, the fingerprint pinned in the tacsim module document `tacsim:geology` at the first keyed boot (a module document, never a submission); a key that will not load disables the layer and never refuses the boot; loaded only when `[tacsim].enabled`.
- **The hand-over is the post-composition device** (`install_geology` between `ground::configure` and `boot_modules`), not a new parameter on `game_modules`/`tacsim_module_wired` (about fifteen call sites across five crates).
- **The readiness line is reported, never gated** (D-18-50's sky manner), Flagged whenever disabled, so a keyless world's summary reads "Not ready"; the shipped template provisions the key and the two production story rigs configure one.
- **The cache key digests the whole `CompositeInput` plus the spec, not the body's table or radius** (lead ruling, taken on SM1-A1): the one writer of tables, `install_body_geology`, drops the replaced body's cached sites instead.
- **The property test's budget** (lead ruling 5): doc 20 §13's "10⁶ lattice points" read as ≥ 10⁶ pocket evaluations grouped by field and block, plus the separate pinned whole-site digest.
- **Decision 13 — a block shift never lifts a family its province says is barren** (the lead's technical ruling, read from §3.1 "the strategic shelf is Vesta's", §3.2's Vesta "carbon barren" and §11's Ceres strategic "barren with rare lean"; doc 20 does not say): the field pass skips the shift for a barren-province family (a negative shift still lowers any family), and on SM1-A4 the table validator refuses a `field.barren` row weighting any ore word, so the invariant holds for any lane table. `prospectivity` stays one `i8` per block in -3..=3; a per-family shift would be a doc 20 change for SM.2 to ask for.
- **Ground truth is never serializable** (decision 14; §16): the five truth types and the two secret types derive no serde, asserted in `truth_serialization_policy.rs`.
- **The production hand-over never uses `?` on the geology key** (decision 15; §16), now pinned at the production root by the garbage-manifest boot.
- **The `i16` fix** (lead ruling 6): the rule in `RegionRec::validate` (the applier's validator — and, as built, the atlas boot loader's), both casts through one saturating helper.
- **The regolith band's side is a keyed per-field draw** (never narrows, never tells the truth by position, the barren and rich edges excepted); the modulo bias of a 64-bit draw over a span of at most 25 is accepted and recorded.
- **`state-slot-owners.toml` gained one test-only row** (`crates/tacsim/tests/geology.rs` → `AtlasState`, owner atlas): the door test seeds its 256 × 256 region into the composed atlas's records, RAM only. Both installers remain owner-crate sites, as the brief expected; the brief's "no row" holds for production code.
- **Owner-critical default taken**: the shipped template and both deploy scripts provision `data/geology-keys` at mode 0700 and `docs/ops.md` tells the operator to back it up with the store (losing it disables mining for the life of the world); nothing is deployed without the owner, and whether the production box's protected backups include the directory is the owner's operational call.
- **Not in this cut**: SM.2's lane v8/v9 surface (`BodyDef.surface_mg/radius_km/geology`, `LocationDef.lat/lon`, the `site` block, the region packs, the block places, the site clerk, landing, `+chart`'s survey words); SM.3's TV and tick; SM.4's instruments, chart documents and `site.map`; SM.6's reserves and opener; SM.7's mining; `docs/help/mining.md` (SM.8). Doc 14's amendment for the `i16` rule is the lead's.
- **Not fixed, recorded**: the region/body cross-check in `build_site_ground` (SM1-A7, for SM.2's brief); the literal reading-rule pin (T8, for SM.4); the single-instance canonical check (T7); the two `is_err()`-only refusals (T9); the untested `not booted yet`/`not composed` texts and `@geology`'s `province`/`pocket` levels (T10); `GeologyPinState`/`status` reachable only under `surface::key::` (SM1-C5); the stale "two key directories" comment in `deploy/dev1/deploy.sh` (SM1-C7) and the ops.md paragraph's placement above two pre-existing orphan bullets (SM1-C6).

### Residual risks the judge named

- `build_site_ground`/`site_geology` never compare the captured region's `RegionKind::Surface { body }` with `spec.body`, so a Vesta frame paired with a Ceres spec builds a mixed site and caches it consistently (SM1-A7); no production caller exists in SM.1 — SM.2 must derive the spec from the site block and refuse the mismatch, and its brief should say so.
- The `i16` rule in `RegionRec::validate` also runs in the atlas boot loader (module.rs:585), so a stored region with datum ± relief outside i16 is now dropped loudly at boot instead of composited through the clamp (SM1-A5); no shipped region authors a non-zero datum or relief and the project is pre-launch, but doc 14's amendment must say the rule tightens the load contract too.
- Reading rule v1 has no literal pin (T8): swapping the regolith band's parity or the level-3 message bytes changes every regolith reading while `READING_RULE_VERSION` stays 1 and every test passes; SM.4's chart rows will pin v1, so SM.4 should land a literal band pin under KEY_1 before it does.
- `sm1_the_site_key_moves_with_every_input` hashes one `CompositeInput` instance twice (T7), so it would not catch a `HashMap` entering the atlas record types later; mush-atlas has none today (verified by grep), and the brief's premise holds for this build only.
- Two refusal tests assert only `is_err()` (T9: the retained-manifest refusal in geology_keys.rs; the misspelt `key_dirr` config key); untested texts remain for `not booted yet` and `not composed`, the `province` and `pocket` levels of `@geology`, and a wrapped x in its output (T10).
- `GeologyPinState` and `key::status` are reachable only as `tacsim::surface::key::{GeologyPinState, status}`, not re-exported at `tacsim::surface::` as the builder's SURFACE implied (SM1-C5); the next builder must use the `key::` path.
- deploy/dev1/deploy.sh:34 still reads "The two key directories are DIFFERENT private key families" after the geology row was added above it (SM1-C7), and docs/ops.md's new geology paragraph sits directly above two pre-existing orphan spool bullets with no blank line (SM1-C6); both cosmetic, one-line fixes for the committer or the lead.
- The 64-bit modulo bias of `uniform` and `pick` is accepted and recorded (the brief's rule); the site door builds a 256 × 256 composite and 4,096 pockets synchronously on the world task — acceptable for the staff/test door, and SM.4 owns the off-loop driver.
- Two base debts the gate exposed and this cut does not own: `mush-server --lib config_ext::tests::ic2_an_operator_seed_claiming_the_lane_refuses_at_the_host` hard-codes `tacsim.lane.ceres_phobos@3` (red since the lane left v3), and the 19 `mush-bot arena_*` tests fail the lane digest because mush-bot's dependency graph turns on serde_json `arbitrary_precision` (the lane's `e-05` exponents re-serialize differently) — both fail identically at f3df3205 in an isolated build and want their own tickets.

### Pins re-blessed

- `crates/tacsim/tests/ports.rs` `ic3_the_readiness_report_lists_every_binding_and_flags_the_rate`: `want` gains `geology: disabled: no [tacsim.geology].key_dir is configured. [flagged]` right after the sky line (this world has no key); the summary's count is derived from `want` since A.8b, so it moves by one with no literal edit. `ic3_boot_creates_the_nine_orgs_and_one_sweep_settles_their_capital`: the one-clause filter `!l.starts_with("geology: ")` on the bracketed lines (the same whole-set shape as the brief's list, under its same-shape clause) — the keyless world's Flagged geology line is reported and never gated.
- `crates/tacsim/tests/coop.rs` `ic6_the_desk_closes_at_a_play_world_rate_other_than_24` and `ic8_the_desk_stays_closed_until_the_ports_are_stocked`: `closed` gains a separate `.filter(|line| !line.starts_with("geology: "))` — same reason.
- The five stocked-gate helpers (`tests/commerce.rs` `stock_ready`, `tests/coop.rs` `commerce_ready`, `tests/orders.rs`/`tests/sale.rs`/`tests/voyage.rs` `stock`): the filter gains `&& !line.starts_with("geology: ")` — the gate never reads the geology line.
- `crates/tacsim/src/port/coop.rs` unit test `ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky`: `EVERY: [Binding; 17]` with `Geology`, `ungated` = `Sky | Markers | Geology`.
- `crates/tacsim/tests/orders.rs` `pb_the_tick_pages_once_a_boot_while_a_house_cannot_fund_a_tonne`: the old `assert!(out.contains("Not ready: "))` became vacuous in a keyless world (the geology line is always Flagged); replaced by a before/after count of non-ready lines (`before + 2`) and `Not ready: {before + 2} of ` — stricter than before (SM1-C2).
- `crates/mush-server/tests/ic_slice_loop.rs` and `ic_buyout_dormancy.rs` `config()`: `cfg.tacsim.geology.key_dir = Some(data_dir/geology-keys)` — their `stock()` waits for "Ready: every binding above is in place.", and a production-shaped config carries the key (the shipped template does).
- `crates/mush-config/tests/shipped_configs.rs`: the geology directory joins the under-`data_dir`/distinct-family assertion (strengthened, not re-blessed).
- `crates/mush-contract-tests/legacy-handlers.toml`: the `@geology` read-only stanza (a new staff verb, non-baseline); **and the `+chart` read-only stanza doc 19 cut A.3 omitted** — verified: HEAD f3df3205's file has no `+chart` stanza while `+chart` is registered in `+credit`'s ReadOnly bracket (`tacsim/src/commands.rs`), so `legacy_handler_allowlist` was red at base. `PINNED_BASELINE` 97 unchanged.
- `crates/mush-contract-tests/state-slot-owners.toml`: one test-only row (`crates/tacsim/tests/geology.rs` → `AtlasState`, owner atlas) because the door test writes the composed atlas's records; the gate named the missing row on the fixer's first run and passed on the re-run.
- `crates/mush-contract-tests/tests/truth_serialization_policy.rs`: seven new `assert_not_impl_any!` lines, named in its one test.
- `crates/mush-server/tests/boot_parity.rs`: the knob-table row for `[tacsim.geology].key_dir`.
- **Expected unmoved and unmoved**: the lane digest, `tests/content.rs`, `tests/chart.rs`, `mush-atlas/tests/starter.rs` and every composite digest, `panic_budget/*.txt`, the restriction-lints census, `module-ns-owners.toml` (`tacsim:geology` is tacsim's own namespace), the composition-edge pin (no new crate edge), the boot-parity feed manifest, the TUI screen counts, `RUN_ALLOWLIST`, and `provisioning_smoke` (renders the template with `[tacsim.geology]` and boots it through `build_engine`, which now provisions and pins a key — green with no edit).

### How it was verified

Three review lenses (correctness, conformance, tests) read the whole diff; 20 findings, 13 fixed by the fixer and verified fixed here, 7 notes recorded above. Gate (final tree, HEAD 7d192600 after the peer's A.8a/A.8b commits; every run at `-j 8`):
- `cargo fmt --all -- --check`: exit 0 (the first run's one diff, `voyage.rs:304 THROTTLE_NONE`, was the peer's A.8a hunk, committed formatted as b0d036d3).
- `cargo clippy --workspace --all-targets -j 8 -- -D warnings`: exit 0 (the first run's one error, `tests/passage.rs:511` useless conversion, was the peer's A.8a hunk, fixed in b0d036d3; `--keep-going` showed it was the workspace's only error).
- `cargo test -j 8 -p mush-core --lib geology_key`: `test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 402 filtered out`.
- `cargo test -j 8 -p mush-config`: `ok. 118 passed` (lib), `ok. 4 passed`, `ok. 12 passed`, `ok. 0 passed` (doctests); 134/0.
- `cargo test -j 8 -p mush-atlas`: 17 binaries, 175 passed, 0 failed (lib `ok. 53 passed`).
- `cargo test -j 8 -p tacsim --test geology`: `test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.45s` (the property test: 1,000,192 pockets in about 9.5 s, under the 30 s rule, no profile change).
- `cargo test -j 8 -p tacsim --no-fail-fast`: 51 binaries, 2705 passed, 0 failed, exit 0 on the final tree (2692/0 on lane v7 before the peer's v8 landed).
- `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop --test ic_buyout_dormancy --test provisioning_smoke`: `ok. 20 passed` (boot_parity, the sm1 test included), `ok. 13 passed`, `ok. 6 passed`, `ok. 4 passed`; exit 0 (the first two attempts failed on the peer's in-flight lane v8 — a zero placeholder digest, then unwired moon rooms — and passed once the peer signed and wired it).
- The nine named contract gates: help_commands 4, hex_vocabulary 2, legacy_handler_allowlist 4, module_ns_owners 4, panic_budget 8, provider_rng_policy 1, restriction_lints 11, state_slot_owners 5, truth_serialization_policy 1; all `0 failed`, exit 0. `cargo test -p mush-contract-tests`: 29 result lines, 130 passed, 0 failed.
- `grep rand crates/tacsim/src/surface/ crates/tacsim/tests/geology.rs`: nothing.
- `cargo test --workspace --exclude mush-bot --no-fail-fast`: 409 binaries, 8074 passed, 8 failed, exit 101 — none this cut's: **peer-attributed with diff evidence** (`mush-net-ssh chart_screen a3b_alt_c…` against the peer's re-blessed `sky_chart.json` fixture; `tacsim moon_ports` ×2, the peer's own new untracked test; `tacsim ports ic3` ×2 and `tacsim sale ic10`, the peer's lane v8 ports before its expectation hunks landed — each passed alone afterwards); **load flake** (`mushd observer_rule the_tui_cockpit_shows_a_pilot_no_more_than_their_own_sensors_found`, a listed paint-timing suite, passed alone twice at 20.5 s); **pre-existing at base** (`mush-server --lib config_ext::tests::ic2_an_operator_seed_claiming_the_lane_refuses_at_the_host`, hard-codes `@3`, fails identically in an isolated `git archive f3df3205` build).
- `cargo test -p mush-bot --no-fail-fast`: 25 binaries, 698 passed, 19 failed in 8 `arena_*` targets, all on one panic (the lane's declared digest is not the pack's digest under mush-bot's serde_json `arbitrary_precision` unification); **pre-existing at base** (arena_contract fails identically in the isolated f3df3205 build); this cut changes no Cargo.toml, Cargo.lock, lane or digest code.
Judge's own checks: the lattice figures recomputed by hand; `git show f3df3205:…/legacy-handlers.toml` has no `+chart` stanza; `digest_from_text` enforces 64 lowercase base-16; no `unwrap`/`expect`/`panic!`/indexing in non-test code under `surface/`; `docs/design/` untouched; the gate files not in the diff (panic budget, module-ns-owners) unmoved.

**Gate notes.** Verdict: none of the failures is caused by cut SM.1. Every command that exercises this cut's code passes on the final tree: fmt, clippy, mush-core geology_key, mush-config, mush-atlas, tacsim geology (15/15), the full tacsim suite (51 binaries, 2705/0), the mush-server boot and slice tests (boot_parity 20/0 including the sm1 geology-key test), and the contract tests (provider_rng_policy green; no `rand` under tacsim/src/surface).

Not peer-attributed (please note): I set green=true although two failure groups are pre-existing rather than peer-attributed, and the brief's green rule names only peer-attributed failures. Both groups fail identically at base f3df3205 in an isolated build, with none of this cut's or the peer's hunks present:
(a) mush-server lib config_ext::tests::ic2_an_operator_seed_claiming_the_lane_refuses_at_the_host hard-codes `@3`, while the lane has been at version 7 since A.7.
(b) 19 mush-bot arena_* tests fail the lane digest check because mush-bot's build turns on serde_json `arbitrary_precision`, which keeps the lane's `e-05` exponents verbatim and so changes the canonical digest.
Both look like latent base debts worth a ticket, outside SM.1's scope. If the orchestrator counts pre-existing failures as red, treat this result as red for those two only.

Load flake: mushd observer_rule the_tui_cockpit_shows_a_pilot_no_more_than_their_own_sensors_found failed in the sweep and passed alone twice.

Peer-attributed: chart_screen a3b_alt_c, tacsim moon_ports (2 tests), ports ic3 (2 tests), sale ic10, and the early boot_parity failures. All came from the peer's in-flight doc 19 A.8b work: lane v8 first carried a zero digest, then rooms that were not yet wired, and the fixture and expected tallies had not yet been updated. Each passed alone once the peer's hunks landed.

Tree movement during the run: the peer committed b0d036d3 (A.8a) at 13:32 and 7d192600 (A.8b) by about 14:45. The fmt and clippy failures in the first runs were the peer's A.8a hunks (voyage.rs THROTTLE_NONE, passage.rs:511 i64::from), fixed in b0d036d3. Final fmt and clippy (--keep-going) runs on the current tree exit 0.

Housekeeping:
- The gate1 directory already held stale files from an earlier aborted attempt, dated 13:04 to 13:18: 03-core-geology-key.log, 07-tacsim-all.log, 08-mush-server-tests.log, 11-workspace.log, run_step.sh and wait_step.sh. They are not this run's results.
- This run created an isolated base copy (gate1/base-f3df3205) and its target dir (gate1/target-base), both large. Delete them when no longer needed.
- No repository file was edited, staged or committed. The peer's cargo runs shared the target dir, and cargo's file lock serialized them with mine.

### Surface for the next cuts

Verified against the diff. **mush-core** (crate root): `GeologySecret::{new(u32,[u8;32]) -> Result<_, GeologySecretError>, version() -> u32, fingerprint() -> [u8;32], mac() -> GeologyMac}`, `GeologyMac::draw(&[u8]) -> [u8;32]`, `GeologySecretError { ZeroVersion, ZeroKey, Key, Retained }`, `MAX_GEOLOGY_SECRET_VERSIONS = 1`; no serde, Debug redacted. **mush-config**: `geology_keys::DurableGeologyKeyStore::{new, directory, load_or_create() -> Result<GeologySecret, GeologyKeyStoreError>}` (no rotate; files `geology-key-NNNNNNNNNN.hex` / `geology-ring-NNNNNNNNNN.json`); `config::TacsimGeologyCfg { key_dir: Option<PathBuf> }` at `TacsimCfg.geology`; `Config::geology_key_dir() -> Option<PathBuf>`; config key `[tacsim.geology].key_dir` (no default). **mush-atlas**: `RegionRec::validate` refuses datum ± relief outside i16 (`RecordError::Value("region datum_cm {d} ± relief_cm {r} leaves the cell elevation range of -32768 to 32767 cm")`); the cast helper is private. **tacsim::surface** (re-exports): `GeologyState` (RAM slot; `sites() -> &SiteGroundCache`), `MAX_BODY_GEOLOGIES = 32`, `install_geology(&mut World, GeologyBoot) -> bool` (false once the boot pass has run), `install_body_geology(&mut World, BodyGeology) -> Result<(), String>` (replaces and drops the body's cached sites; "a world holds at most 32 body geology tables"), `body_geology(&World, &CanonicalId) -> Option<Arc<BodyGeology>>`, `live(&World) -> Result<(GeologyMac, u32), GeologyDisabled>`, `GEOLOGY_PIN_KEY = "tacsim:geology"`, `GeologyBoot { NotConfigured, Key(GeologySecret), Unreadable(String) }`, `GeologyDisabled { NotBooted, LayerOff, NotConfigured, KeyUnreadable(String), PinUnreadable, Mismatch { pinned, loaded } }::reason()`, `GeologyStatus { Enabled { version, fingerprint: [u8;32] }, Disabled(_) }`, `GeologyPinRecord { fingerprint, key_version }`; **only under `surface::key::`**: `GeologyPinState { Absent, Present, Unreadable }` and `status(&World) -> Option<&GeologyStatus>`. **tacsim::surface::geology**: constants `GEOLOGY_DOMAIN`, `MAX_FIELD_POCKETS 16`, `POCKETS_PER_FIELD_SIDE 8`, `POCKETS_PER_FIELD 64`, `MAX_FAMILIES_PER_POCKET 2`, `PROVINCE_COLUMNS 56`, `PROVINCE_ROWS 28`, `MAX_SHIFT 3`, `POCKET_M 40`, `FIELD_M 320`; `OreFamily { Metals, Volatiles, CarbonFeedstock, StrategicElements }::{ALL, code, id, label}` (serde snake_case); `Word { Barren, Lean, Fair, Rich }::{ALL, code, of_code, name, pocket_range, tonnes_range, of_count, shifted}`; `GeologyTableWire { province, field, pocket }` (deny_unknown_fields) and `GeologyTable::{from_wire, wire}`; `GeologyError { MissingProvinceRow, MissingFieldRow, MissingPocketRow, BarrenPocketRow, BarrenPocketClass, BarrenProvinceField, RowSum{row,sum}, Radius, OffLattice, Shift, Origin, BodyMismatch{spec,body} }`; `BodyGeology { pub body, pub radius_km, pub table }::{new (radius 1..=10,000), lattice}`; `Lattice { pub columns, pub rows }::{of_radius, field_columns, south, north}`; truth types `ProvinceGround { province: (u32,u32), words: [Word;4] }`, `PocketRow { classes, tonnes }::{BARREN, families}`, `FieldGround { field (x normalised), province, shift, words, counts, pockets: Vec<PocketRow> (64) }::pocket(lx, ly)`; `province_at(mac, body, (u32,u32))`, `field_ground(mac, body, (i64,i64), i8)`, `pocket_at(mac, body, (i64,i64)) -> (FieldGround, PocketRow)` (shift 0), `normalise_pocket(body, (i64,i64))`. **tacsim::surface::reading**: `READING_RULE_VERSION: u16 = 1`, `Footing { Rock, Regolith, Ice }::{of(TerrainClass), name}`, `FieldReading { Exact, Band(low, high), NoReturn }::{words, pockets}`, `read_r0`, `read_r1(mac, body, &FieldGround, Footing) -> [FieldReading;4]`, `read_r2`. **tacsim::surface::ground**: `SITE_CELLS 256`, `SITE_CELL_CM 1000`, `SITE_POCKETS 64`, `SITE_FIELDS 8`, `SITE_BLOCKS 4`, `SITE_CACHE_CAPACITY 16`; `SiteGeologySpec { body, origin: (i64,i64) field corner, shifts: [i8;16] by*4+bx }`; `SiteGeology::{body, origin, key_version, pocket(px,py), field(fx,fy) -> Option<([Word;4],[u8;4])>, totals() -> [(u32,u32);4], digest() -> String}`; `site_geology(mac, key_version, body, spec)`; `SiteGroundKey { region: RecordKey, revision: u64, key_version: u32 }`; `SiteGround { composite: Arc<Composite>, geology: Arc<SiteGeology> }`; `site_revision(&CompositeInput, &SiteGeologySpec) -> Result<u64, String>`; `capture_site(&World, &WindowFrame)`; `build_site_ground(input, spec, body, mac, key_version) -> Result<SiteGround, String>` ("a site's ground is 256 by 256 cells of 10 m"); `SiteGroundCache::{get, insert, len, is_empty}` (`forget_body` crate-private); `site_ground_sync(&mut World, &WindowFrame, &SiteGeologySpec) -> Result<Arc<SiteGround>, String>`. **tacsim::surface::commands**: `geology_lines(body, pocket, key_version, &FieldGround, &PocketRow) -> [String;4]`; the verb `@geology <body> <x>,<y> [province|field|pocket]` (staff). **tacsim::port**: `Binding::Geology` (ungated); line texts `geology: key v{n}, fingerprint {12}, pinned; reading rule v1.` / `geology: disabled: {reason}.` **tacsim::store** (crate-private): `TacsimStore.geology_pin`, `put_geology_pin`. **Pinned values**: Ceres 73,824 × 36,912, Vesta 41,304 × 20,640; the whole-site pin under `*b"rustmush-sm1-geology-test-key-01"` at origin (8000, −1600), block 0 +1, block 15 −1: geology digest `67bf968c2b5fbe28d1ee2eb55227b88620efb73a194b8bf194e8f46697459eeb`, composite `c6a2ff3e4b3d5b2ed937effe04ed3b82598f90ca2d072f13c05a6af7c803eff6`, totals (326, 2979), (412, 3945), (172, 1325), (0, 0). **Rules for SM.2**: author `field.barren = {barren: 1000}`; check the region's body against the spec; `install_body_geology` is the one writer of tables.

### Doc 20 amendments owed

- **§13, the SM.1 row** — "the off-loop site composite LRU and geology table": as built, the in-RAM LRU of sixteen, the pure builds (`build_site_ground`, `site_geology`) and the synchronous door land in SM.1; the off-loop driver is SM.4's (recorded as a narrowing; the composite half is keyless on the atlas's `CompositeJob`, the geology half a host-held runner keyed in `build_engine`). The acceptance cell's "a property test over 10⁶ lattice points" is read as ≥ 10⁶ pocket evaluations grouped by field and block (1,000,192 as built).
- **§3.2, "Only staff read truth, through `@geology <site> <addr>`"** — as built in SM.1: `@geology <body> <x>,<y> [province|field|pocket]` over the body-fixed pocket lattice (no site shift applies); the site-address form arrives with SM.2's sites.
- **§3.2, the `geology(key, body, level, lattice)` line** — as built: one HMAC-SHA256 per (level, cell, family) over `"rustmush.geology.v1\0" ‖ body ‖ 0x00 ‖ level u8 ‖ x i64 LE ‖ y i64 LE ‖ family u8`, read as four little-endian `u64`s; levels 0 province, 1 field, 2 pocket, 3 the reading band's side; every coordinate normalised by its own level's width before it enters the HMAC (x wraps, y never).
- **§3.2 / §3.1, the lattice** — doc 20 never defines the body-level pocket lattice; as built: columns = ⌊⌊radius_km × 1000 × 710 / 113⌋ / 320⌋ × 8, rows = ⌊columns / 32⌋ × 16 (whole fields around the equator and on each side of it); Ceres (470 km) 73,824 × 36,912, Vesta (263 km) 41,304 × 20,640; a pocket's province is (x × 56 / columns, (y + rows/2) × 28 / rows); SM.2 maps `lat_mdeg`/`lon_mdeg` onto it and snaps a site's origin to a field corner.
- **§3.2 / §3.1, the authored `prospectivity` shift** — add: the shift is one `i8` per block in −3..=3, applied to the field word before the count is drawn, saturating at barren and rich, and **never applied to a family whose province word is barren** (decision 13: a rich-authored Ceres block never grows strategic ore; a negative shift still lowers any family); the table validator refuses a `field.barren` row weighting any ore word, so the invariant holds for any lane table. §3.2's "may carry two families" is as built: at most two, placed by keyed rank in family order; the count and tonnes ranges are code constants, not table data.
- **§3.2, "A fingerprint of it is pinned in a world record at first use"** — as built: SHA-256 of the 32 key bytes plus the key version, pinned in the tacsim module document `tacsim:geology` at the first boot that has a key (a boot-time module document, never a submission), compared at every later boot; the five disabled reasons (no `[tacsim.geology].key_dir`, the key directory did not load, `tacsim:geology` unreadable, a mismatch, the layer off) read on the readiness report's `geology:` line, reported and never gated; `[tacsim.geology].key_dir` has **no default** and the key is loaded only when `[tacsim].enabled`.
- **§3.2, the `i16` finding's last sentence** — as built: `RegionRec::validate` refuses `datum_cm ± relief_cm` outside −32,768..=32,767 cm (at the applier **and at the atlas boot load**, where a failing stored region is dropped loudly), and both casts saturate symmetrically through one helper.
- **§3.2, "held in a RAM LRU of sixteen … built beside it once per key version"** — as built: keyed by (region, revision, key version) where revision digests the whole captured `CompositeInput` and the site spec; a replaced body table drops that body's cached sites (`install_body_geology`).
- **§5, R1's regolith band** — add the rule's mechanics as built: barren → barren–lean, rich → fair–rich, otherwise one keyed draw at level 3 per (field, family) chooses the side, so a repeat never narrows and the position does not tell the truth; the reading rule carries a version (`READING_RULE_VERSION` 1) that SM.4's chart rows pin.
- **§11, the Geology row** — add "at most two families a pocket; the ranges are code constants" (as written it is consistent; nothing wrong).
- **§15 Q-20-12 and §16** — add: the key is loaded without `?` (a key that will not load is a disabled layer, never a refused boot), its directory is provisioned 0700 by the deploy scripts under `__DATA_DIR__/geology-keys`, and `docs/ops.md` tells the operator to back it up with the store.
- **§18, Doc 14's amendment** — the `i16` rule tightens both the apply contract and the boot-load contract (an out-of-range stored region is refused at load, not clamped); doc 14 should say so when the lead writes it.
- **§13, the SM.2 row** — add: `build_site_ground` does not yet check that the captured region's `RegionKind::Surface { body }` is the spec's body; SM.2 derives the spec from the site block and must refuse the mismatch. Also: SM.2's lane tables must author `field.barren = {barren: 1000}`.

## SM.2

**Sites and landing** (doc 20 §13, the SM.2 row): lane **v9** (the astronomy session's A.8 took v8),
six mining sites — Freeport Commons, Kerwan Floor, Ahuna Flank, Juling Shadow and Occator Rim on
Ceres, Rheasilvia Commons on Vesta — as vendor-less port entries over twelve atlas packs, the landing
law and the ascent term on legs touching a site, the hop, landed life support charged at lift, the
leg-home reserve, `SITE_CLOSED`, `LEAVE_PAD`, `+ship/lift`, the site lines on `+chart`. Built
2026-10-19 on main from `e7f2da90` (HEAD `6f797974` during the run, a docs-only SM.1 follow-up),
from a planned-ahead brief with a LEAD OVERRIDES section (role word `extraction`, not `field`; site
ports never gate `+coop/enrol`; SM.1's body-mismatch refusal; serde-spelled lane numbers). Judged
**ship-with-notes**, then a lead fix round before the commit (below). One commit, pushed.

### What landed

1. **Lane v9** (`crates/tacsim/content/lane/ceres_phobos.json`, re-signed; 60 entries, digest `5a1c0016af72e82dce23b5b26ee94a461fff8ccea4aa8111430c1949fb82df01`; `LANE_VERSION` 9 in `definitions/content.rs`). The lane's numbers keep the A.8 spelling rule: every number v9 adds is an integer.
   - **Surface figures on eight bodies** (`surface_mg`, `radius_km`): Ceres 28 / 470, Vesta 25 / 263, Pallas 21 / 256, Luna 165 / 1737, Mars 379 / 3390, Titan 138 / 2575, Phobos 1 / 11, Deimos 1 / 6. Phobos and Deimos are ceiled to 1 mg.
   - **Geology tables on Ceres and Vesta**: SM.1's reviewed `CERES_TABLE` and `VESTA_TABLE`, inline and verbatim.
   - **Ceres Freeport's coordinates**: 19.7 N, 240.4 E. Its role stays `port`.
   - **`freeport_registry`** ("Freeport Registry", 50,000 cr, faction freeport) is appended last to `ceres_port.orgs`, and Ceres gains six routes keys.
   - **Six places of role `extraction`** (lead override 1), each with coordinates: Freeport Commons (20.3 N, 240.4 E), Kerwan Floor (−10.8, 123.9), Ahuna Flank (−10.5, 316.2), Juling Shadow (−35.9, 168.6) and Occator Rim (21.3, 239.3; lead override 2) on Ceres; Rheasilvia Commons (−75.0, 301.0) on Vesta.
   - **Six vendor-less site ports** `<venue>_port`, each gallery `neighborhood.<loc>.room.pad`, issuer `freeport_registry`, zone `core`. The Commons is 16 commons blocks at `worked_permille` 400. Occator Rim has F, G, J and K held by `occator`, and its outer twelve blocks carry +1 prospectivity. Rheasilvia is commons; the rest are open.
   - **Twelve routes**: five same-body Ceres hops at `base_s` 3,600 each way, and Rheasilvia at 3,127,945 s each way, read from the engine's epoch chord.
2. **The wire** (`src/pack.rs`, `src/port/def.rs`, `src/port/families.rs`, `src/port/map.rs`). Every new field is skipped when absent, so no v8 shape re-encodes differently.
   - `BodyDef.{surface_mg, radius_km, geology}` through `with_surface`, with refusals naming each bound; `MAX_SURFACE_FIGURE`.
   - `LocationDef.{lat_mdeg, lon_mdeg}` with `PackError::PlaceCoordinates`; `SITE_ROLE = "extraction"`.
   - `PortDef.site` with serde defaults for `rooms`, `treasury` and `orgs`.
   - `SiteDef`, `SiteTenureDef`, `BlockDef`, `BlockTerms`, `Tenure`, `Zone` (`frontier` does not parse), `SITE_BLOCK_COUNT` and `BLOCK_LETTERS`. Letters are read from the north-west, and `SiteDef::shifts` maps them onto SM.1's south-west slots.
   - The zero-orgs rule in `PortDef::check`. `check_ports` adds: role ⇔ site; body, coordinates and body figures present; name equals label; issuer and holders are orgs of the pack.
   - `LegPlan.ascent_mps`, `PassagePlan.landed_crew_days` (with its check), and `PortMap.site` / `SiteRow` (the map schema stays 1).
3. **The landing law, the ascent and the floor** (`src/port/passage.rs`):
   - **Constants**: `LANDING_TWR_PERMILLE` 1,200 and `SITE_MIN_BURN_T` 1.
   - **`landing_twr_permille`**: integer, checked `u128`, floored.
   - **`least_landing_throttle`**: the exact ceiling.
   - **`ascent_mps`**: √(g·R), ceiled, the one `f64` step. Ceres 360 m/s, Vesta 254.
   - **`route_landing_floor`** and **`PlanError::Landing`**.
   - **In `plan_passage`**: only a leg with a site end meets the law, and only a site end pays the ascent. A same-body hop stays `base_s` through the laden step.
   - **The burn**: `flip_profile` adds the ascent to Δv, and `passage_burn` takes at least 1 t after the ceil when any leg pays an ascent.
   - **The burn's prepare** (`burn.rs`) adds `landed_crew_days` to the legs' crew-days.
   - **Measured**: the hop is 5,322 s and 1 t at 86,580 kg; the home hop is 6,893 s.
   - **TWR at the rail**: Ceres 1,785, Luna 303, Titan 362, Mars 131. A Dray cannot land on Luna, Titan or Mars.
4. **`port/site.rs`** (new). `SITES_PRINCIPAL`, `STORES_HERE`, `STORES_SHORT_RATIONS`, `SURVEY_LINE`, `SURVEY_UNAVAILABLE`, `TENURE_LINE`, and the functions:
   - `body_name`;
   - `geology_spec` and `site_province` (`div_euclid`; they refuse a place on another body);
   - `survey_words` (R0 through the live key and an installed table, else "unavailable");
   - `tenure_parts`;
   - `landed_since` (the durable Arrived journey to the host room);
   - `landed_charge` (souls × whole IC days, capped at locker − leg home);
   - `pad_span` and `stores_line` (the effective figure);
   - the applier's `preflight`, `lay_up` and the readiness `site_line`.
5. **Part (g) of the applier** (`src/port/apply.rs`). Per site: sixteen DARK block place Things "{site} block {L}" and a DARK "{site} site clerk" in the host room, each recorded in the port map as it is made. Nothing is submitted; an update re-lays nothing that is recorded; a recorded Thing gone refuses at preflight. The lane's body tables are then installed.
6. **The tables at boot and at apply** (`src/surface/mod.rs`, `src/lib.rs`, `src/port/apply.rs`).
   - `lane_geologies` builds them while the store is borrowed; `install_lane_geology` installs after and skips a body whose table is already equal; `installed_bodies`.
   - Boot installs right after `surface::key::boot`.
   - Deviation from brief item 7(b): when the store does not load the applied version, part (g) decodes the applied pack (`rules_pack_of`) and installs now, instead of warning and waiting for the next boot.
   - `Lattice::province_of_pocket` is now pub. SM.1's stale module docs are updated.
7. **Lead override 4** (`src/surface/ground.rs`). `build_site_ground` refuses a region that is not a surface of the spec's body, and nothing is cached on the refusal.
8. **Readiness and the desk** (`src/port/readiness.rs`, `src/port/coop.rs`).
   - `Binding::Site`, with the line "{entry} site: 16 block places and the site clerk laid up." or "{n} of 16 … {the site clerk|no site clerk} …"; `gated(Site)` is false.
   - `Readiness.sites` is filled before the map is read, and `enrol_gate` skips every line of a site port (lead override 3), so mining content never closes `+coop/enrol`.
   - `port_ready` backs `SITE_CLOSED`, which refuses a departure for a site whose pad is not ready. This applies to `+ship/plan`, `+ship/undock`, `+ship/lift` and the queued depart (`executor.rs` maps `K::Landing` and `K::SiteClosed` to lines).
   - `port_name` returns the site's name.
9. **The voyage** (`src/port/voyage.rs`):
   - `SiteArrival` on `PendingVoyage` and `Voyage`, carried at launch.
   - `charge_stay`, the one helper `+ship/plan` and the filing share: the stay is charged at lift inside the pilot's own burn.
   - **The leg-home reserve**, `site_reserve`: home routes for the berth's venue; the home payload with each pool left after the passage out; planned at the least landing floor; the most over 31 days when the home leg crosses bodies.
   - **Refusals and texts**: `SITE_TANK_SHORT`, `SITE_STORES_SHORT`, `SITE_HOME`, and `LANDING_REFUSED` (body by its display name, TWR to hundredths floored).
   - **Arrival texts**: `SETTLES` and `SETTLED_NOTICE`, each with the stores line, read at the landing instant.
   - **Lift texts**: `LIFT_SEALS` and `LIFT_USAGE`.
   - **The throttle hint** starts its bisection at `hint_floor`, the least landing floor. On a pad it reads the pools aboard (`THROTTLE_FITS_PAD`, `THROTTLE_NONE_PAD`).
   - `lane_pack` is factored out of `plan_on`.
10. **The berth** (`src/port/ship.rs`, `src/commands.rs`):
    - `SHIP_SETTLED`, `SHIP_SETTLED_DEPARTS`, `STATUS_SETTLED` and `STATUS_SETTLED_DEPARTS` on a pad.
    - `+ship`'s stores line after the hold line.
    - `+ship/leave` refused with `LEAVE_PAD`.
    - `+ship/lift <port>` routed to `undock_cmd`, with a `SHIP_HELP` line.
11. **`+chart`** (`src/port/chart.rs`, `src/port/chart_panel.rs`):
    - No direct-chord clause and a null `direct` for any destination on the vantage's own body.
    - `SURVEY_LINE` (or `SURVEY_UNAVAILABLE`) and `TENURE_LINE` under each site.
    - No demand line in a site's window.
    - The vantage with no helm falls back to the first station, never a site.
12. **Twelve atlas packs** (`content/atlas/region.<loc>.json` and `neighborhood.<loc>.json`, signed, version 1).
    - Each region is a 2,560 m surface region on its body at `ground_resolution_cm` 1000, regolith with an ice/rock mix, relief 100–150 cm, seeds 2601–2606, locality kind `extraction`.
    - Each neighborhood has one outdoor `room.pad` at (1260–1300, 1260–1300) carrying `port.dock`, and two to four crater features. The whole-square footprint was accepted.
    - They are registered as `SITE_SOURCES` (12) with `SITE_ROOMS` 1, and `station_packs()` chains them after `STATION_SOURCES`, so the boot list is 28 packs.
13. **Help and gates**:
    - `docs/help/ships.md`: `+ship/lift` and a "Settling at a site" section.
    - `docs/help/travel.md`: "The mining sites".
    - `legacy-handlers.toml`: the `+ship` note gains the `lift` sentence; the baseline is unmoved.
    - `state-slot-owners.toml`: one row for `tests/sites.rs`'s `AccountOracle` (the voyage.rs precedent).

**Found in review and fixed in the same cut:**
- The site reserve underflowed into a dark plan on a short tank.
- An absent or unreadable site map still shut `+coop/enrol`.
- Four bare or private intra-doc links failed strict rustdoc.
- The chart told players a site "posts its list prices whatever the season".
- The help said "about an hour each way laden" and said "the plan refuses".
- The Commons prose claimed "two centuries of working" (re-signed: region `0128a8b6…`, neighborhood `c112498e…`).
- A full-throttle lift from Rheasilvia's pad got no throttle advice; it now gets the on-pad hint.

### Deferred, and why

**Lead rulings taken by the planner (binding; override the brief and doc 20):**
- **Lane v9, not v8.** A.8 took v8 and goods v2, so SM.5 becomes lane v10.
- **Role word `extraction`, not `field`** (lead override 1). "Field" is the geology's R1 level.
- **Occator Rim at 21.3 N, 239.3 E** (lead override 2). Its own "46 km north on the rim" is inconsistent: this point is about 12 km from the centre, on the floor, and the rim 46 km north is about 25.4 N. Deferred to the lead: correct the latitude (re-sign the lane, re-pin `LANE_DIGEST`, the sites table and the fixture) or reword the two Occator packs, before v9 latches anywhere.
- **Site ports never gate `+coop/enrol`** (lead override 3). This supersedes brief decision 15. A site whose pad is not ready refuses a departure with `SITE_CLOSED` instead.
- **SM1-A7 closed** (lead override 4). The site door refuses a spec for another body.
- **Lane numbers follow the A.8 spelling rule** (lead override 5).
- Lead ruling 3: only a leg with a site end meets the landing law and pays the ascent; no pinned port leg moves.
- Lead ruling 4: a same-body hop is `base_s` through the laden step.
- Lead ruling 5: the stay is charged at lift.
- Lead ruling 6: the table lives on the body; two-step install, skip-equal.

**Owner-critical defaults, standing as the lead confirmed them:**
- **Decision 4, charged at lift.** The raw locker, the gauges and the hold manifest stay full while settled. Only the stores lines print the effective figure.
- **Decision 6, surface figures on eight bodies.** Phobos and Deimos are ceiled to 1 mg. No Dray lands on Luna, Mars or Titan.
- **Decision 7, a departure for a site holds back the leg home.** The leg home is planned at the least lifting throttle, taking the worst of the next 30 IC days when it crosses bodies. Rheasilvia fits 0 of 365 days at full throttle and 191 at throttle 457. The off-pad hint ignores the reserve, so the pilot must find the throttle unaided. The owner should rule on a reserve-aware hint, a cheaper reserve, or the site.
- **Decision 8, the once-per-stay notice.** The arrival notice is the once-per-stay short-rations notice; nothing fires at the instant the cap binds.
- **Decision 10, `LEAVE_PAD`.** Nobody steps out onto a pad.
- **Decision 16, the coordinates as authored.**
- **Decision 17, prospectivity.** Only Occator's outer twelve blocks carry +1. "Metals and carbon" at Kerwan and Ahuna, and "ice-rich" at Juling, are whatever the key and the body table make them.
- **Decision 22, now `extraction`.**

**Deferred to later cuts:**
- **SM.3**: the TV, the Dray-P, the bay and `+tv`'s stores line.
- **SM.4**: `@geology <site> <addr>`, readings and cell↔pocket binding.
- **SM.5**: lane v10, the licence and ore goods.
- **SM.6**: `SITES_PRINCIPAL` as owner of the Things, the clerk's grant, and acting on `worked_permille`.
- **SM.9**: `+chart/ground`.

**Residuals recorded, not built:**
- `plan_queue`'s depart dry run knows neither the landed charge, the reserve nor `SITE_CLOSED`. The fire refuses correctly.
- The reserve does not cover a heavier hold coming home (SM.7's capacity rule must).
- A Rheasilvia stay past 30 IC days can find the home burn above the tank. The recovery is a window, a lower throttle, or the dormancy page.
- `choose_route` lets one listed route's Landing refusal sink the others. Latent: one route per destination in v9.
- Body tables install at apply only through a site port's put, so a table-only change waits for the next boot.
- The boot pass installs the shipped version's tables whenever the store loads it, even on a world latched lower (RAM only).
- Lore slips in signed prose: Juling's shading wall should be to the north; "on a clear day" at Occator.
- `lay_up`'s unused `def` parameter.
- The landing suite's year sweep rebuilds the catalog daily (about 34 s).
- The crater size bounds are not asserted.

**Deviation (filed by the fixer):** part (g) decodes the applied pack when the store lacks its version, instead of warning and skipping (brief item 7(b)). It is tested by `sm2_the_apply_installs_the_lane_tables`.

### Residual risks the judge named

- **Occator Rim's coordinate (lead override 2) contradicts its own prose.** At 21.3 N, 239.3 E the pad sits about 12 km from Occator's centre at 19.82 N, on the crater floor. The override's text and the pad's descriptions put it on the rim about 46 km north, which is about 25.4 N (`lat_mdeg` 25400). The lead must either correct the latitude (re-sign the lane, re-pin `LANE_DIGEST`, the sites.rs table, the chart fixture and any geology pins) or reword the two Occator packs, before lane v9 latches on any world.
- **Rheasilvia is reachable only throttled and in season.** A full-tank, empty-hold departure plus its reserve never fits the 80 t tank at full throttle on any day of the year (0 of 365; 64 + 37 = 101 t at the epoch). At the landing floor, throttle 457, it fits on 191 of 365 days. Away from a pad the `Throttle:` hint looks only at the outbound burn (64 t is no more than 80 t), so a pilot at Ceres meets `SITE_TANK_SHORT` with no throttle advice. Q-20-10's "Vesta when the sky favours it" is reachable only by guessing `throttle=`. This is an owner item: a reserve-aware hint, a cheaper reserve, or a change to the site.
- **`plan_queue`'s depart dry run** (`executor.rs`) knows neither `SITE_CLOSED`, the leg-home reserve, nor the landed charge. A queued depart to a closed or under-reserved site reads as fine in `+ship/orders`, and the fire then holds it with the right reason.
- **`choose_route` lets one route's `PlanError::Landing` sink every other route** (passage.rs). The reserve and the hint take the least landing floor over the routes that answer, then plan all listed routes at that throttle. This is latent: every v9 destination lists exactly one route.
- **The lane's geology tables are installed at apply only when a site port is created or updated** (apply.rs part g). A later lane version that edits a body's table and touches no site port installs at the next boot, not at apply. SM.5's lane v10 must touch a site port, or the install should move to the Body applier.
- **During tacsim's boot `report_ports` cannot see the content latch**, so the boot pass installs the shipped `LANE_VERSION`'s tables whenever the store loads that version, even on a world latched lower. This is RAM only and harmless today.
- **Lore slips in signed prose:**
- Juling Shadow (35.9 S) puts Juling's wall to the south shading the pad, but on Ceres the sun stands in the north.
- Occator Rim's faculae are "visible on a clear day" on an airless body.

Fixing either means re-signing and re-pinning two packs.
- **Planning cost per call:**
- A bare `+ship/plan` at Ceres now runs `port_ready` (a `report_ports` plus `port_lines`) for each of the six sites.
- The Rheasilvia reserve runs 31 cross-body intercept plans on the world task, in every plan block and every filing.

None of this is measured.
- **The year sweep in landing.rs rebuilds the fielded catalog for every day**, so the landing suite runs in about 34 s.

Two test gaps:
- The site-pack test does not assert crater size (120–400 m), `z_range` or `blocks`.
- The year sweep reimplements the reserve in test code. Its epoch figures are tied to `site_reserve` only through the world test's literals (37 / 36 / 366).
- **`lay_up` carries an unused `def` parameter** (`let _ = def;`, site.rs).
- **The strict `RUSTDOCFLAGS=-D warnings cargo doc` run is reported by the fixer only**, not by the final gate.

### The lead's fix round (before the commit)

The judge's residuals included an error of the lead's own: override 2 put Occator Rim at 21.3 N and
called it 46 km north of the crater's centre, but a degree of latitude on Ceres is about 8.2 km, so
21.3 N was 12 km out, on the crater floor. A fix round (`sm-fixround`, run `wf_87549cc6-af1`; the
judged tree `f5b88439`, the round's tree `589b6e17`, the delta reviewed alone) worked seven items:

1. **1. Occator Rim's latitude** — fixed: The lane's occator_rim location now reads lat_mdeg 25400, lon_mdeg 239300 (crates/tacsim/content/lane/ceres_phobos.json). Lane v9 was re-signed with TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test lane ic2_the_lane_pack_is_pinned. LANE_DIGEST in tests/lane.rs moved from 5a1c0016… to cce73a4c60ed6213caaeed321d61926be526cb36c6794ee09e2b60891531402c, because the location's coordinate is part of the signed lane. Compared with the judge's tree, the lane diff is exactly those two lines. The sites.rs SITES table now has lat 25_400, with a comment giving the arithmetic. Nothing else moved: the chart fixture re-blessed with TACSIM_BLESS=1 cargo test -p tacsim --test chart came out byte-identical to the judge's tree, because +chart drops the on-body chord. No geology pin reads Occator's province: tests/geology.rs ran 15 of 15 green, and sm2_the_sites_geology_builds_and_no_two_sites_overlap still passes at 25.4 N, so the 64-pocket squares stay disjoint. The Occator packs' prose already said 'northern rim … crater falling away to the south … the Freeport at its centre', which fits a pad 46 km north of the centre, so that prose was left alone. The pad wording changed under item 4.
2. **2. A reserve-aware throttle hint** — fixed: voyage.rs throttle_hint is now pub(crate) and takes `site: Option<(&PortDef, &CanonicalId)>`. For a site destination, a throttle counts as fitting only when the outbound burn plus the leg-home reserve fit the tank (and, at the throttle found, the locker). The search bisects from the destination's landing floor. When no throttle fits today and the sky moves, it tries the floor on days 1..=PLAN_WINDOW_DAYS through a new chart::first_day, placed beside best_day as +chart's window scan; the scan is bounded at 30 and skipped under a constant sky. Three new pub texts: THROTTLE_FITS_SITE, THROTTLE_LATER_SITE (names the date and the offset) and THROTTLE_NONE_SITE. Each has a {pools} slot for 'full' or 'as they stand'. A station destination reads exactly as in A.8. A live +ship/undock refused with SITE_TANK_SHORT or SITE_STORES_SHORT now carries the hint as a second line. A queued depart's waiting note keeps the bare sentence, because the hint changes from day to day and a note is re-told whenever its text changes. To keep the cost down, each reading first plans the arrival day's leg home alone (site_reserve gained a window_days parameter; 0 gives a lower bound) and plans the full 30-day window only when that bound still fits. Measured in debug at the epoch: +ship/plan rheasilvia takes 636 to 978 ms and the refused +ship/undock 487 to 703 ms. Release is not measured. New test sites.rs sm2_the_throttle_hint_counts_the_leg_home, on the moving sky with a full tank and empty hold. At the epoch, +ship/plan prints THROTTLE_NONE_SITE (min 457, window 30) and +ship/undock refuses with SITE_TANK_SHORT 101/80 followed by the same line. The test then steps 10 IC days at a time; each step until the season shows the NONE line, and at day 100 the line reads 'throttle=457 does on … (in 23 IC days)'. On that day, full throttle still refuses; the hint names throttle=N with burn + held back ≤ 80, and +ship/undock rheasilvia throttle=N files a ticket with exactly that burn and throttle. The lead's 'at the epoch, a throttle that then files' cannot happen at the epoch, because nothing fits there until about five months on, so the epoch case is the 'no fitting throttle' case. Unit test chart::sm2_the_first_day_is_the_earliest_that_fits_and_is_bounded. docs/help/ships.md describes the new line.
3. **3. The queue's dry run** — fixed: prepare_departure's checks are split into three pub(crate) functions in voyage.rs. site_gate covers SITE_CLOSED. plan_refusal maps a planner error to LANDING_REFUSED or OVERLOADED. hold_back covers the stay's charge (charge_stay), the site reserve with SITE_TANK_SHORT and SITE_STORES_SHORT, then TANK_SHORT and STORES_SHORT; it mutates the plan so the printed crew-days include the landed charge. prepare_departure and +ship/plan's plan_block both call these. executor::plan_queue's depart branch now builds a Basis from voyage::basis with the projected payload, tank, locker and plan_instant, and calls the same site_gate, plan_on, plan_refusal and hold_back, so the dry run prints the refusal the fire would hold with, inline_reason-formatted. TANK_SHORT and STORES_SHORT are no longer imported by executor.rs. One deviation: +ship/orders prints no dry run at all (orders.rs render_orders lists orders, Next due, the hold note and the log); the dry run is +ship/plan's sailing-plan block (plan_queue). The test therefore reads +ship/plan. New test sites.rs sm2_the_queue_dry_run_reads_a_closed_site: with Ahuna's pad GOING, a depart queued at=+2h reads '  Order 1, depart for Ahuna Flank: it would hold: Ahuna Flank is closed: its pad is not ready.' before it is due. +ship/orders still shows it as next due, and a Kerwan depart reads its plan with no hold. The hint test also checks that a queued Rheasilvia depart at the epoch reads '…; it would hold: The passage to Rheasilvia Commons and the leg home burn 101 t of reaction mass; the tank holds 80 t.'
4. **4. Two lore slips in signed prose** — fixed: neighborhood.juling_shadow room.pad desc now reads: 'To the north the wall of Juling rises dark and steep between the pad and the low sun, and the pad sits at its foot. … The horizon to the south is low and grey.' Its 'crater wall' detail now says 'Juling's north wall'. neighborhood.occator_rim room.pad desc: 'the bright faculae at its centre always in view, white against the grey.' Both packs were re-signed with TACSIM_WRITE_CONTENT=1 cargo test -p tacsim --test sites sm2_the_site_packs_load_clean_and_are_pinned. PINNED in sites.rs moved: juling d8788414… becomes 97655785413243d6e9bac03a59d9af99a2db6bf7d0029db8cb87f969fad05c5a, and occator 78bd670c… becomes 18ac2c2dabf0a50e30fa6ced16b5a6ccfaefb3ddf09d382f08860bfce316a517. The region packs are unchanged. Both packs still validate with no blocking finding and no warning.
5. **5. Test gaps** — fixed: sm2_the_site_packs_load_clean_and_are_pinned now asserts, for each crater: widest vertex-to-vertex chord within 120..=400 m, surface 'ground', z_range [0, 15], and blocks {movement:false, sight:true}. A bounding-box check was tried first and failed on Kerwan: a 120 m decagon is only 114 m across flat-to-flat, so the check uses the widest chord. The reserve logic is factored into a pure, public function passage::home_reserve, returning passage::HomeReserve (both re-exported from tacsim::port). voyage::site_reserve calls it, and voyage::Reserve is a type alias of it. landing.rs sm2_rheasilvia_reachability_over_a_year now calls home_reserve on the epoch day with the sweep's own sky, home payload and arrival, and asserts (burn, throttle) == (home0, floor0), i.e. (37, 366). It also pins PLAN_WINDOW_DAYS == 30.
6. **6. lay_up's unused def** — fixed: The `def: &PortDef` parameter and `let _ = def;` are removed from port/site.rs lay_up, along with the now-unused PortDef import. The one call site, port/apply.rs part (g), is updated.
7. **7. Strict rustdoc** — fixed: The surface module's //! header links were already qualified (crate::surface::install_lane_geology, crate::surface::lane_geologies), and no other module header in this cut brackets an unqualified item. The strict run found one error, in the fixer's own new doc: a redundant explicit link [`PLAN_WINDOW_DAYS`](crate::port::chart::PLAN_WINDOW_DAYS) in voyage.rs after the constant was imported. It is now a bare link. The new public docs (HomeReserve, home_reserve, the THROTTLE_*_SITE texts) bracket only public items. RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8 is clean.

**The round's review** — The review covers the fix round's delta only: `git diff f5b88439fa4865d1e55a1267c1da9fe15aeb37ce 90d7ca023593f0ba843513696727f75836c76863`. I built that tree id with a scratch index and it matches the fixer's. The delta is 14 files, +855/−278. Nothing was staged, committed, stashed or built.

All seven items are done. No must-fix. Two should-fix items and five notes.

**Item 1 (Occator Rim's latitude): done, correct.**
- The lane diff is exactly `lat_mdeg` 21300→25400 plus the re-signed digest (`cce73a4c…`). The arithmetic checks out: 470 km radius gives 8.20 km per degree, so 46 km is 5.6°, and 19.82 N + 5.6° is about 25.4 N.
- `LANE_DIGEST` and the `SITES` row in `tests/sites.rs` moved, with the reason given.
- No stale `5a1c0016`/`d8788414`/`78bd670c` digest and no `21.3`/`21300` coordinate remains in crates, docs, deploy or web. No test pins Occator's province; the survey words are derived, not pinned.
- The two Occator packs' prose already reads "northern rim … crater falling away to the south", which fits a rim pad.

**Item 2 (reserve-aware hint): done, with one undisclosed deviation (should-fix).**
- What is right: `throttle_hint` now counts the leg home. It prunes with an arrival-day lower bound and re-checks the found throttle with `fits`, so it never names a throttle that fails. A station destination reads as in A.8. The live `SITE_TANK_SHORT`/`SITE_STORES_SHORT` refusal carries the hint; a queued note keeps the bare sentence.
- The test is real. At the epoch it asserts the exact NONE line on both `+ship/plan` and `+ship/undock`. It then steps 10 days at a time and asserts NONE on every step until the LATER line appears. On that later day it asserts the full-throttle refusal and that the named throttle has burn + held-back ≤ 80. A filing at that throttle produces a ticket with exactly that burn and throttle.
- The fixer's argument that "the epoch case cannot file" is sound: at the floor the epoch needs 45 + 40 = 85 t against an 80 t tank.
- The deviation: the later-day scan runs over +ship/plan's 30-day window, not over +chart's 365-day window that the lead named.

**Item 3 (queue dry run): done.**
- `site_gate`, `plan_refusal` and `hold_back` are each one function, shared by `prepare_departure` and `plan_queue`.
- The `def` passed is the right berth, because `plan_queue` stops at the first depart.
- Reading the result through `+ship/plan` instead of `+ship/orders` is a valid, disclosed deviation: `+ship/orders` prints no dry run.
- The closed-site test is exact and would have failed on the old code. The reserve case is also covered in the hint test.

**Item 4 (lore slips): done.** Juling's north wall with the pad at its foot is consistent with the sun standing north at 35.9 S. "Always in view, white against the grey" replaces "visible on a clear day". Both pack digests were re-signed and re-pinned. No other south-wall or clear-day wording remains.

**Item 5 (test gaps): done.**
- The crater check uses the widest chord (120–400 m), `surface`, `z_range [0,15]` and `blocks`.
- `home_reserve` was factored out as pure and public. The year sweep now asserts `(burn, throttle) == (home0, floor0)` against it on the epoch day.

**Item 6 (`lay_up`'s unused `def`): done.** The parameter and the import are gone, and the one call site is updated.

**Item 7 (strict rustdoc): done as far as can be checked without cargo.**
- Every new module-header link uses a `crate::` path to a public item.
- The new public docs (`HomeReserve`, `home_reserve`, the `THROTTLE_*_SITE` texts) bracket only public items.

**Standing checks:**
- No assertion weakened: the two changed `sites.rs` assertions are still exact equalities, now with the hint line added.
- Every pin moved has its reason.
- No RNG added, no truth leaked (the hint reads only the public sky).
- The refusals keep the established voice.

**Risk:** the hint runs on the world task and is not measured in release.

- **SM2-FX-1** (should-fix, `crates/tacsim/src/port/voyage.rs:2106`): The lead's item 2 says the hint 'names the next IC day within the chart's window on which one does (reuse +chart's window scan; bounded)'. In this codebase +chart's window is `chart::WINDOW_DAYS` = 365 (chart.rs:55; `window_lines_for` at chart.rs:457 plans each day of the next IC year). The fixer scans `first_day(PLAN_WINDOW_DAYS, …)`, the 30-day +ship/plan Window-line constant. The docs call it '+chart's window scan' (voyage.rs:1968; THROTTLE_LATER_SITE's doc above voyage.rs:350) but never flag it as a deviation. The result: at the epoch, and for about the first 93 IC days, a pilot at Ceres gets THROTTLE_NONE_SITE, 'today or in the next 30 IC days', with no date. Rheasilvia's floor first fits about day 123 (the fixer's own run: day 100 + 23). That is close to the 'no advice' state item 2 was meant to end. The test (sites.rs:2464) had to step to day 100 to see a LATER line; with the chart's window the epoch itself would name the day, which is what the lead's 'a day with no fitting throttle → the hint names a later day' asks.
- **SM2-FX-2** (should-fix, `crates/tacsim/src/port/voyage.rs:2040`): The reserve-aware hint runs on the single-writer world task for every `+ship/plan` to a site that does not fit at the asked throttle. Rheasilvia never fits at full throttle (0 of 365 days), so every default `+ship/plan rheasilvia` pays it, and so does every refused live `+ship/undock` (the hint at voyage.rs:2583-2598).

Each `at(t, offset)` call does a `plan_on`, which builds a fresh `Sky::from_pack` (voyage.rs:1351, cloning every element table). It then does a `site_reserve`, which builds a second Sky. When out + arrival-day reserve fits, it adds the 31-plan window.

The fixer measured 0.64–0.98 s in debug for `+ship/plan rheasilvia` at the epoch, where the arrival-day bound prunes almost everything. Release is not measured. Near the season's closing edge each bisection step can pay the full window (about 10 × 33 plans). The plan block also adds the Window line (30 plans), `SITE_HOME` (31) and a queued site depart's dry-run reserve (31).
- **SM2-FX-3** (note, `crates/tacsim/src/port/executor.rs:1391`): Item 3 asked for 'one function, not a copy'. The checks are now shared, but the composition `site_gate` → `plan_on` → `plan_refusal` → `hold_back` is written twice (voyage.rs `prepare_departure` and executor.rs `plan_queue`). The dry run's `Err(PlanError::Overloaded{..})` arm (executor.rs:1391-1405) still formats OVERLOADED inline: a second copy beside `plan_refusal`'s Overloaded branch. A future check added between those calls in `prepare_departure` would not reach the dry run.
- **SM2-FX-4** (note, `crates/tacsim/src/port/executor.rs:1328`): Several comments are stale or mislabelled:
- The new dry-run comment says 'so `+ship/orders` reads the refusal the fire would hold with'. The fixer's own finding is that only `+ship/plan`'s sailing plan prints the dry run.
- THROTTLE_LATER_SITE's doc (above voyage.rs:350) says 'a later day of `+ship/plan`'s window (`+chart`'s window scan)', and throttle_hint's doc (voyage.rs:1968) says '`+chart`'s window scan, `chart::first_day` over `PLAN_WINDOW_DAYS`'. Those are two different windows.
- **SM2-FX-5** (note, `crates/tacsim/tests/sites.rs:2458`): The doc comment of `sm2_the_throttle_hint_counts_the_leg_home` says '140 IC days on the hint names the first later day'. The fixer's run found it at day 100 ('in 23 IC days'), and no assertion pins either figure.

The fixer's residual (b) is not supported. It says the world's season opens earlier than the sweep's 'near day 174' because the two 'differ in ephemeris rate'. They do not: landing.rs uses `sky_at(&pack, 1_000)` and sites.rs uses `Roster::moving_sky` with `sky_rate: 1_000`, and both agree on the epoch (64 + 37 = 101, reserve 37 at 366). '191 of 365 days fit' does not mean the season starts at day 365 − 191 = 174. The existing in-season check at day 180 (sites.rs:2348, 41 + 38 ≤ 80) and a first fit around day 123 both fit a season of roughly days 123–313.
- **SM2-FX-6** (note, `crates/tacsim/src/port/voyage.rs:2056`): The bisection (`highest_fitting` over `burn_fits`) assumes out + reserve rises with throttle. The reserve's window starts at the arrival, which moves with throttle, so near a window edge the sum is not strictly monotone. The final `.filter(fits)` keeps the line sound: it never names a throttle that refuses. But it can name a lower throttle than the highest that fits, or fall through to LATER/NONE when a higher throttle fits today. The later-day scan also tries only the floor, so a day on which only a faster passage (arriving before a worse home window) fits is skipped.
- **SM2-FX-7** (note, `crates/tacsim/tests/landing.rs:527`): Item 5 asked for a tie to 'the production site_reserve function'. `site_reserve` is pub(crate), so the fixer factored out the pure `passage::home_reserve` and the sweep calls that. The sweep's home-payload formula `EMPTY_HOLD_KG - out.burn_t * 1_000 - crew_days * 4` is still test code. `site_reserve`'s own payload derivation and its route-key choice are tied only transitively, through the world test's `home_line(37, 36, 366)` literal. Crew-days are not compared. Separately, the landed-charge path of the new dry run is not exercised by any test (the lead asked only for SITE_CLOSED).

**Re-fixed after the review:**

- **SM2-FX-1** — fixed: The later-day scan now walks +chart's window, `chart::WINDOW_DAYS` (365), using `first_day(WINDOW_DAYS, ...)` in crates/tacsim/src/port/voyage.rs throttle_hint. The reserve each reading holds back still uses the filing's 30-day window, and the arrival-day bound is kept, so a day that overruns costs 2 plans. THROTTLE_NONE_SITE's {window} is now 365. I corrected the docs on THROTTLE_LATER_SITE and THROTTLE_NONE_SITE, the throttle_hint doc (it now names WINDOW_DAYS and states the scan's cost bound), and the `first_day` doc in chart.rs. The chart.rs unit test's bounded-count check now uses WINDOW_DAYS. docs/help/ships.md now reads 'the first day in the next IC year (the 365 days `+chart <port>` scans)'. Tests in crates/tacsim/tests/sites.rs: (a) sm2_the_throttle_hint_counts_the_leg_home asserts that at the epoch itself the hint is THROTTLE_LATER_SITE with an offset in 1..=WINDOW_DAYS, pinned exactly at '0000-05-04 (in 123 IC days)'. The refused +ship/undock ends with the same line. On the day before, the hint names the same date with '(in 1 IC days)', and so does the refused undock, which shows that day does not fit. On the named day the full-throttle undock refuses, the hint names a fitting throttle, and filing at that throttle launches with the hinted burn (unchanged). The old ten-day stepping loop is gone. (b) sm2_rheasilvia_holds_back_the_leg_home_under_the_moving_sky pins the same LATER line on both refused undocks (full throttle and throttle=457), where it used to expect NONE. This agrees with its own 'files 180 IC days on' figure. (c) The new test sm2_the_throttle_hint_names_no_day_when_the_year_has_none keeps the NONE line covered: with 40 t of metals aboard the floor is 514, no day of the year fits, and both +ship/plan and the refused undock read '...today or in the next 365 IC days.' exactly.
- **SM2-FX-2** — fixed: The lane Sky is now built once per hint. New in voyage.rs: `lane_sky_of(w, version) -> (Sky, &RulesPack)`, `plan_on_sky((&Sky,&RulesPack), basis, routes)`, and `site_reserve_on(w, (&Sky,&RulesPack), basis, out, site, from_venue, window_days)`. `plan_on` and `site_reserve` keep their signatures and now delegate to these, so the filing and the SITE_HOME line are unchanged. throttle_hint builds the Sky once and every `at(t, offset)` reading shares it, both the passage out and the arrival-day and 30-day reserves. The hint's floor reuses the same pack. I also changed chart.rs `plan_window_line` (the +ship/plan Window line, 30 plans) to build one Sky instead of 30. Release timings for the whole `+ship/plan rheasilvia` command (Window line, SITE_HOME and the hint included), measured with a temporary test that is now removed: epoch 17.4 ms (LATER, day 123). Days 25/50/75/100: 16.8/12.6/11.6/9.1 ms. First fitting day (123): 9.8 ms. In season with full-window bisections (days 125-400, sampled every 25 days): 11.0-28.3 ms, worst 28.3 ms on day 275. Full 365-day NONE scans, cargo 40 and 60 t: 34.1 and 34.7 ms. LATER at 196 and 289 days (cargo 10 and 20 t): 22.1 and 30.8 ms. Debug: epoch 113-124 ms, against the reviewer's 0.64-0.98 s before; worst 239 ms (the NONE year scan). Season-closing edge: Rheasilvia's season does not close within 400 IC days of the epoch (the hint reads FITS every day from 123 to 400; the Ceres-Vesta synodic period is many years), so no reachable closing edge exists to time. The closest costly cases measured are the full-year NONE scans and the in-season bisections, which pay the 30-day reserve window at each step; both are listed above.

Carried as notes (not fixed): the composition `site_gate → plan_on → plan_refusal → hold_back` is
written twice (`prepare_departure` and the queue's dry run) rather than once; a few comments say
`+ship/orders` where the dry run is `+ship/plan`'s sailing-plan block; the hint test's doc comment
names day 140 where the run found day 100; the throttle bisection assumes the outbound plus the
reserve rises with throttle, which a window edge can break by a tonne; the year sweep ties to the pure
`passage::home_reserve`, since `site_reserve` is crate-private.

### Pins re-blessed

**Lane and content signatures:**
- **`tests/lane.rs` `LANE_DIGEST`**: `be733686…` → `5a1c0016af72e82dce23b5b26ee94a461fff8ccea4aa8111430c1949fb82df01`. Lane v9 adds 24 entries and fields on 9 existing ones, re-signed with `TACSIM_WRITE_CONTENT=1 … --test lane ic2_the_lane_pack_is_pinned`.
  - The same file's other re-pins: version 8 → 9, kinds 36 → 60 and places 6 → 12, the route checks over `SITE_ROUTES`, and `placed` 6 → 12.
  - `lane_legs_derive_at_epoch` exempts the five same-body hops, whose `base_s` is authored, and checks Rheasilvia's derived legs.
  - The carried `a8_every_number…` test is unedited and green.
- **Twelve new site-pack digests**, pinned in `tests/sites.rs`. The Freeport Commons pair was re-signed after the prose fix: region `0128a8b67e95da3dfef1c587208f249f736e79e37925a033b8ff3fd53ce4a0ff`, neighborhood `c112498ef5d5228518434ba336bea4809b91d118a1b08c0ec6275f0ad2b63375`. The other ten are as the builder printed them.
- **`web/src/lib/fixtures/chart/sky_chart.json`**, re-blessed with `TACSIM_BLESS=1`: eight routes and windows, a null `direct` on the Ceres sites, and survey and tenure lines in `lines.sky`. There is no demand line for a site; the fixer re-blessed again after removing it.

**Gate files:**
- **`crates/mush-contract-tests/state-slot-owners.toml`**: one row for `tests/sites.rs` (`AccountOracle`), because the rig charters through `+coop/enrol` (the voyage.rs precedent).
- **`legacy-handlers.toml`**: the `+ship` note gains the `lift` sentence; `PINNED_BASELINE` is unmoved (a new switch, not a new verb).
- **`port/coop.rs` unit test**: `EVERY` 17 → 18, with `Site` ungated.

**Count re-pins in the old suites, each moved because v9 adds what it counts:**
- **ports.rs**:
  - The treasury moves from −1,450,000,000 to −1,455,000,000, for the Registry's 50,000.
  - Orgs 9 → 10, and port documents 5 → 11.
  - The kept-world counts: 34 unchanged becomes 58.
  - v1 → v9 reads 35 create / 11 update / 14 unchanged.
  - The placed-room check becomes a superset plus id stability.
  - The readiness lines gain the "<Site> Pad" presentation and the site line.
  - The IC-return comparison filters sites.
- **commerce.rs**: v7 → v9 reads 36 create / 10 update / 14 unchanged, and the v2/v7 rebuilds call `strip_sites`.
- **price.rs**: `strip_sites` added to its rebuild.
- **stations.rs**: `station_packs` 10 → 22, the boot list 16 → 28 with the site `requires`, the shelf at @1, the six pad docks, and latches 16 → 28.
- **chart.rs**:
  - The no-route list now names the eight venues.
  - Routes and windows go from 2 to 8.
  - The windows are found by name and strengthened to `>`.
- **passage.rs**: routes 8 → 20, and a same-body hop is asserted to equal the laden step.
- **moon_ports.rs**: Ceres's listing gains six entries, and the entry count goes 36 → 60.
- **capacity.rs**: sites are filtered out; it still panics on an unknown venue.
- **orders.rs and voyage.rs**: the label lists gain the sites.
- **e7c1.rs**: one literal.
- **geology.rs** (SM.1; the boot now installs Ceres and Vesta):
  - The no-table refusals move from vesta to pallas.
  - The cap loop is counted from `installed_bodies`.
  - The helpers move to `support/geology_keyed.rs`.
  - No SM.1 digest moved.
- **mush-server `boot_parity.rs`**: orgs 20 → 21, and latches 16 → 28 with each site pack at 1.

**Stated unmoved, and checked by the gate:** the panic budget, the restriction census, the module-namespace owners, the composition-edge pin, `provider_rng_policy`, `truth_serialization_policy`, `GOODS_DIGEST`, `tests/content.rs`, the starter digests, SM.1's whole-site digests and its key-family pin.

Also re-blessed in the lead round: `LANE_DIGEST`
`cce73a4c60ed6213caaeed321d61926be526cb36c6794ee09e2b60891531402c` (Occator Rim's latitude), the Juling
and Occator neighbourhood pack pins (`97655785…`, `18ac2c2d…`; the lore), the sites table's Occator row.

### How it was verified

The final gate ran 12 steps, one at a time with `-j 8`, at HEAD 6f797974 (e7f2da90 plus a docs-only SM.1 follow-up commit). Every step passed on the first run, with no reruns, no LNK1104 link errors and no peer-attributed failure. git status before and after the sweep is identical, so the astronomy session was not editing.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -j 8 -- -D warnings` | `Finished dev profile … in 18.15s`, exit 0 |
| `cargo test -j 8 -p tacsim --test landing --test sites` | landing `ok. 7 passed; 0 failed`; sites `ok. 21 passed; 0 failed` |
| The fifteen named tacsim suites | lane 11, passage 5, chart 7, geology 15, moon_ports 5, stations 21, ports 21, commerce 26, voyage 25, orders 33, capacity 17, price 8, sale 18, coop 23, hulls 15; all ok, 0 failed |
| `cargo test -j 8 -p tacsim --no-fail-fast` | 53 binaries ok, 2746 passed, 0 failed, 13 ignored; lib `ok. 1268 passed; 0 failed; 1 ignored` |
| `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop --test ic_buyout_dormancy --test provisioning_smoke` | boot_parity 21, ic_buyout_dormancy 14, ic_slice_loop 6, provisioning_smoke 4; all ok |
| `cargo test -j 8 -p mush-server --lib config_ext` | `ok. 14 passed; 0 failed; … 284 filtered out` |
| `cargo test -j 8 -p mush-net-ssh --test chart_screen` | `ok. 2 passed` |
| The ten named contract gates | 1 / 4 / 5 / 8 / 11 / 2 / 4 / 4 / 1 / 2 passed, 0 failed |
| `cargo test -p mush-contract-tests -j 8` | 29 binaries, 130 passed, 0 failed |
| `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8` | 411 result lines all ok, 8133 passed, 0 failed, 93 ignored |
| `cargo test -p mush-bot -j 8` | 25 binaries, 717 passed, 0 failed (arena_contract 19, arena_e2e 10, playtesting 9) |

The sweep's "thread 'tokio-rt-worker' panicked at process_fault.rs:197" line comes from mush-server's deliberate panicked-world-task tests, which pass. It is expected, not a flake.

**Checked by the fixer, not by the gate:**
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8` → `Finished … in 4m 53s`, exit 0. The first run failed on four tacsim intra-doc errors, all fixed.
- `web: npm test` → `Test Files 54 passed (54) / Tests 621 passed (621)`.

**The fixer's mutation checks**, each reverted afterwards:
- Without the lib.rs boot install, `sm2_the_lane_tables_are_section_3_2s` fails.
- Without part (g)'s install, `sm2_the_apply_installs_the_lane_tables` fails.
- With the hint floor forced to 250, `sm2_rheasilvia_holds_back_the_leg_home_under_the_moving_sky` fails.
- With the readiness `sites` insert moved back after the map read, `sm2_an_unreadable_site_map_closes_the_pad_and_never_the_desk` fails.

**Lenses.** Three reviewers:
- Correctness: two must-fix, one should-fix, two notes.
- Conformance: one must-fix, six should-fix, four notes.
- Tests: two must-fix, six should-fix, two notes.

The fixer fixed every must-fix and every should-fix except conformance F6 (Occator's coordinate), which is deferred to the lead. The judge re-read the whole diff against the brief and lead overrides 1–5.

**Measured figures:**
- The hop: 5,322 s and 1 t, against the formula ⌊3,600 × 267,580 / 181,000⌋. The home hop: 6,893 s.
- TWR in permille, at the rail / with an empty hold: Ceres 1,785 / 2,345, Vesta 2,000 / 2,626, Luna 303 / 397, Titan 362 / 475, Mars 131 / 173.
- Least landing throttle at the rail: Ceres 672, Vesta 600.
- Ascent: Ceres 360 m/s, Vesta 254.
- v8 → v9 on the shelf: 24 create / 10 update / 0 retire / 26 unchanged.
- Rheasilvia at the epoch: out 64 t plus a 37 t reserve at throttle 366. Days in the year it fits: 0 at full throttle, 191 at throttle 457.

**The lead round's gate (final tree `589b6e17`), all first run:**

- pass: `cargo fmt --all -- --check` — exit 0, no output (log 01-fmt.log)
- pass: `cargo clippy --workspace --all-targets -j 8 -- -D warnings` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.80s; exit=0 (a full cache hit: cargo's fingerprints matched the current sources, so a clippy run of this exact state had already passed; log 02-clippy.log)
- pass: `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.75s / Generated C:\Users\faeni\cc\rustmush\target\doc\mush_ansi\index.html and 45 other files; exit=0 (a cache hit under the same RUSTDOCFLAGS: a strict run on identical sources had already passed; log 03-doc.log)
- pass: `cargo test -j 8 -p tacsim --test landing --test sites --test lane --test chart --test geology --test passage --test orders --test voyage --test stations` — chart: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.89s
geology: test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.00s
landing: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 33.46s
lane: test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.62s
orders: test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.43s
passage: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
sites: test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.68s
stations: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.33s
voyage: test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.59s
exit=0
- pass: `cargo test -j 8 -p tacsim --no-fail-fast` — 53 result lines, all ok; totals 2750 passed, 0 failed, 13 ignored. lib: test result: ok. 1269 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 88.81s. exit=0 (log 05-tacsim-all.log)
- pass: `cargo test -j 8 -p mush-net-ssh --test chart_screen` — test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s; exit=0
- pass: `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop --test ic_buyout_dormancy` — boot_parity: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.85s
ic_buyout_dormancy: test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.88s
ic_slice_loop: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.68s
exit=0
- pass: `cargo test -p mush-contract-tests -j 8` — 29 result lines, all ok; totals 130 passed, 0 failed, 8 ignored; exit=0 (log 08-contract.log)
- pass: `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8` — 411 result lines, all ok; totals 8137 passed, 0 failed, 93 ignored; exit=0. Load-sensitive suites all passed first time: ipc_authoritative_action 8, two_process_e2e 4, supervision_abandoned 2, server_panic_relaunch 5, scripting_e2e 1, world_fault 2, arena_story 1, arena_story_split 1, observer_rule 32. Final line: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s (Doc-tests tacsim). (log 09-workspace.log)
- pass: `cargo test -p mush-bot -j 8` — 25 result lines, all ok; totals 717 passed, 0 failed, 7 ignored; exit=0. lib: test result: ok. 469 passed; 0 failed; 0 ignored. arena_contract: test result: ok. 19 passed; 0 failed; ... finished in 10.18s. arena_e2e: test result: ok. 10 passed; 0 failed; ... finished in 224.60s. playtesting: test result: ok. 9 passed; 0 failed; ... finished in 124.34s. (log 10-mush-bot.log)

### Surface for the next cuts

Verified against the diff (`tacsim` crate unless stated):

**Content and lane**
- `definitions::content`:
  - `LANE_VERSION` = 9.
  - `SITE_SOURCES: [(&str, &str); 12]` and `SITE_ROOMS` = 1.
  - The pack-name constants `{COMMONS, KERWAN, AHUNA, JULING, OCCATOR, RHEASILVIA}_{REGION_PACK, PACK}` and their `_JSON` sources.
  - `station_packs()` now chains `SITE_SOURCES` after `STATION_SOURCES`.
- `pack`:
  - `SITE_ROLE = "extraction"` and `MAX_SURFACE_FIGURE` = 10,000.
  - `BodyDef.{surface_mg, radius_km}: Option<u32>` and `BodyDef.geology: Option<surface::GeologyTable>`.
  - `BodyDef::with_surface(self, Option<u32>, Option<u32>, Option<&GeologyTableWire>) -> Result<BodyDef, String>`.
  - `LocationDef.{lat_mdeg, lon_mdeg}: Option<i32>` and `PackError::PlaceCoordinates { place, detail }`.

**Port definitions and maps**
- `port::def`, re-exported from `port`:
  - `PortDef.site: Option<SiteDef>`; `rooms`, `treasury` and `orgs` now have serde defaults.
  - `SiteDef { name, issuer, zone, tenure: SiteTenureDef { default }, blocks: BTreeMap<String, BlockDef> }`.
  - `BlockDef { tenure, holder, prospectivity: i8, worked_permille: u16, zone }` and `BlockTerms<'_>`.
  - `Tenure::{Commons, Open, Held}` with `word()`, and `Zone::Core`.
  - `SITE_BLOCK_COUNT` = 16 and `BLOCK_LETTERS` (A–P from the north-west).
  - `SiteDef::block(char) -> BlockTerms`, `SiteDef::shifts() -> [i8; 16]` (`SiteGeologySpec` order) and `SiteDef::check()`.
- `port::map`: `PortMap.site: Option<SiteRow>` and `SiteRow { blocks: BTreeMap<String, u32>, clerk: Option<u32> }`.
- `port::families`: `LegPlan.ascent_mps: u32` and `PassagePlan.landed_crew_days: u32`, both skipped on the wire at zero.

**Planner** (`port::passage`, re-exported)
- `LANDING_TWR_PERMILLE` = 1,200 and `SITE_MIN_BURN_T` = 1.
- `landing_twr_permille(&HullDrive, dry, payload, surface_mg) -> Option<u64>`.
- `least_landing_throttle(..) -> Option<u32>`.
- `ascent_mps(surface_mg, radius_km) -> Option<u32>`.
- `route_landing_floor(&RulesPack, &CanonicalId, &HullDrive, dry, payload) -> Result<u32, PlanError>`.
- `PlanError::Landing { place, body, twr_permille }`.
- `pub(crate) site_end(pack, place)`.

**Sites** (`port::site`)
- Constants: `SITES_PRINCIPAL` = "tacsim.sites"; the texts `STORES_HERE`, `STORES_SHORT_RATIONS`, `SURVEY_LINE`, `SURVEY_UNAVAILABLE` and `TENURE_LINE`.
- `body_name(&CanonicalId) -> String`.
- `geology_spec(&SiteDef, &LocationDef, &BodyGeology) -> Result<SiteGeologySpec, String>`; it refuses a place on another body.
- `site_province(&LocationDef, &BodyGeology) -> Result<(u32, u32), String>`.
- `survey_words(&World, &RulesPack, &CanonicalId) -> Option<String>`.
- `tenure_parts(&RulesPack, &SiteDef) -> String`.
- `landed_since(&World, ObjId, &PortMap) -> Option<i64>`.
- `landed_charge(souls, since, now_ic, locker, home_crew_days) -> u32`.
- `pad_span(i64) -> String`.
- `stores_line(&World, locker, souls, home_crew_days, since, now_ic) -> Option<String>`.
- `pub(crate)`: `preflight`, `lay_up` and `site_line`.

**Surface**
- `lane_geologies(&RulesPack) -> Vec<Result<BodyGeology, String>>`.
- `install_lane_geology(&mut World, Vec<..>) -> usize`, which skips an equal table.
- `installed_bodies(&World) -> Vec<CanonicalId>`.
- `geology::Lattice::province_of_pocket` is now pub.
- `ground::build_site_ground` refuses a region whose body differs from the spec's.

**Readiness and the desk**
- `port::readiness`: `Binding::Site`, `Readiness.sites: BTreeSet<CanonicalId>`, `pub(crate) port_ready(&World, &CanonicalId) -> bool`.
- `port::coop`: `gated(Site)` is false; `enrol_gate` skips every line of a site port; `port_name` returns `site.name`.

**Voyage** (`port::voyage`)
- `SiteArrival { locker, home_crew_days, souls }`, and `Voyage.arrival` / `PendingVoyage.arrival: Option<SiteArrival>`.
- Texts: `SETTLES`, `SETTLED_NOTICE`, `LANDING_REFUSED`, `SITE_TANK_SHORT`, `SITE_STORES_SHORT`, `SITE_HOME`, `LIFT_SEALS`, `LIFT_USAGE`, `SITE_CLOSED`, `THROTTLE_FITS_PAD` and `THROTTLE_NONE_PAD`.
- `DepartRefusalKind::{Landing, SiteClosed}`.
- `pub(crate)`: `lane_pack`, `site_port`, `Reserve` / `site_reserve`, `charge_stay`, `settled_stores_line` and `hint_floor`; `throttle_hint` takes `on_pad`.

**Ship** (`port::ship`)
- `SHIP_SETTLED`, `SHIP_SETTLED_DEPARTS`, `STATUS_SETTLED`, `STATUS_SETTLED_DEPARTS` and `LEAVE_PAD`.
- The `+ship/lift` switch.

**Tests**
- `tests/support/sites.rs`: `SITE_PORTS`, `SITE_PLACES`, `SITE_VENUES`, `SITE_ROUTES`, `SURFACE_BODIES` and `strip_sites` (the v8 shape).
- `tests/support/geology_keyed.rs`: `KEY_1`, `KEY_2`, `CERES_TABLE`, `VESTA_TABLE`, the radii, `secret`, `mac`, `no_refused_writes` and `boot_keyed`.
- The `tests/sites.rs` rig: `Roster::moving_sky()`.

**What later cuts read from this**
- **SM.3**: `+tv`'s stores line through `stores_line` and `landed_since`; `payload_kg` feeds the landing law.
- **SM.4**: `geology_spec` and `site_province` for cell↔pocket binding and `@geology <site>`.
- **SM.5**: lane v10 must create or update a site port for part (g) to re-install the tables at apply.
- **SM.6**: `SiteRow.blocks` / `clerk`, `SITES_PRINCIPAL`, and `BlockTerms.worked_permille` / `holder`.

### Doc 20 amendments owed

**§13, the SM.2 row and the cut table:** "lane **v8**" is now **v9**, and SM.5's "lane **v9**" becomes **v10**. A.8 took v8 and goods v2, so SM.5's "goods v2" is goods v3.

**§3.1:**
- **Role word**: "role `field`" becomes "role `extraction`" (lead override 1) in the prose and in the JSON example.
- **The example's site block**: it carries a required `name` equal to the location's label and has no `geology` key; the table lives on the body.
- **`BodyDef`'s geology**: it gains an inline `geology` table, SM.1's wire checked at load, not "a `geology` table name". `radius_km` is required with it. The validator also requires a site's body to author `surface_mg`, `radius_km` and `geology`.
- **The block place Things**: the sentence "one place Thing per block … owned by `tacsim.sites`" becomes: the Things are owned by the applying actor, DARK in the lane place's host room, and SM.6 makes `tacsim.sites` their industry owner. `SITES_PRINCIPAL` is declared in SM.2. The site clerk Thing is laid up beside them.
- **Occator Rim, as built**: its two inner drone blocks and inner reserve are F, G, J and K held by `occator` (no worked distinction yet), and the outer twelve blocks carry +1 prospectivity.
- **Coordinates**: record them as authored. The Freeport is at 19.7 N, 240.4 E; Occator Rim is at 21.3 N, 239.3 E, which contradicts "the rim", pending the lead.

**§3.3:**
- **Landed life support**: "a hull at rest on a site debits one crew-day per soul per IC day" becomes "is charged at lift, inside the pilot's own departure burn (`PassagePlan.landed_crew_days`): souls × whole IC days settled, capped so the locker keeps the leg home; every stores line prints the effective figure".
- **The short-rations notice**: "a once-per-stay 'short rations' notice in the `tacsim:notice` ring" becomes "the arrival notice to an absent tenant, with its stores line, is the once-per-stay notice".
- **Where the stores print**: "the arrival line, `+tv` and `+ship/plan`" becomes "the arrival line, `+ship` and `+ship/plan`"; `+tv` comes with SM.3.
- **The landing law's scope**: only a leg with a site end meets it, and only a site end pays √(g·R). Ceres Freeport, a port on the Occator floor, pays none, so no pinned port leg moves.
- **Phobos**: authored at 1 mg, not 0.6.
- **Ceres's landing throttle**: the band edge is exactly 672 ‰ laden ("about 670").
- **Hop timings**: "about 5,300 IC s out … about 7,000 home" are 5,322 s and 6,893 s for §3.3's loads, and 7,200 s at the rail.
- **New, not in Draft 0.4:**
  - A departure for a site holds back the leg home: the worst of the next 30 IC days at the least lifting throttle, refused with `SITE_TANK_SHORT` or `SITE_STORES_SHORT`, and printed by `+ship/plan` as `SITE_HOME`.
  - Nobody steps out onto a pad (`LEAVE_PAD`).
  - A site whose pad is not ready refuses departures (`SITE_CLOSED`) and never gates `+coop/enrol` (lead override 3).

**§11, the Landing and Hop rows:** add Ceres 1,785 ‰, the 672 ‰ floor, the ascent 360 m/s (Ceres) and 254 (Vesta), and the hop at 5,322 / 6,893 s.

**§1 beat 2:** this phase, `+chart` prints each site as a destination, then "Registry survey: …" and "Tenure: …" lines under it. The survey reads "unavailable" without a live key. The beat's single-line format is not what shipped.

**§15 Q-20-10:** add a note that as built, Rheasilvia fits a full-tank departure plus the reserve on 0 of 365 days at full throttle and on 191 at throttle 457. This is an owner item.

**§18:**
- **Doc 19 §3**: `BodyDef.surface_mg`, `radius_km` and `geology` (inline), and `LocationDef.lat_mdeg` / `lon_mdeg`, landed in SM.2.
- **Doc 11 §4/§6**: the Freeport Registry is a Ceres org (50,000 cr, faction freeport) and the issuer of all six sites.
- **Doc 18 §4.4**: `+ship/lift`, the settled berth texts, `LegPlan.ascent_mps` and `PassagePlan.landed_crew_days` landed.

Add from the lead round: §3.1's Occator Rim is at **25.4 N, 239.3 E** (not 21.3); `+ship/plan` and a
refused departure for a site print a **reserve-aware throttle hint** (the highest throttle at or above
the landing floor at which the outbound burn plus the leg-home reserve fit, else the first later IC
day in +chart's 365-day window on which the floor fits); the queue's sailing-plan dry run reads the
same site refusals the fire holds with.

## SM.3a

**The vehicle and its carrier** (doc 20 §13's SM.3 row, split by the lead into SM.3a and SM.3b; this
is the first half): **goods v3** (the four ore goods under `stow.ore`, moved here from SM.5 because the
hopper and the Dray-P cargo need the tag), `UnitClass::Vehicle`, optional drive, the tools kind
(drill, scanner, battery), the `tv_prospector` and `dray_p` units, the bay counted in the payload, the
hopper vault owned by `tacsim.vehicles`, `+coop/refit` (re-dressing the chartered hull in place through
`mush_tactics::module::refit_unit`; every programme hull gains a hidden empty bay at lane **v10**) and
Occator's lease `+tv/lease` (1400.00cr a term, a 5000.00cr deposit held as the engine-owned reservation
`tacsim.tv.deposit`; no licence check until SM.5). Built 2026-10-20 on main from `bd718fb2`. Judged
**ship-with-notes**; the lead accepted the four owner-critical defaults (repossession at a charter's
end, the hidden bay on every hull, Occator's counter in the Exchange Hall with rigs OWP-TV-01..08, the
battery row and the engine-owned deposit table) and ran a fix round before the commit. One commit,
pushed.

### What landed

1. **Goods v3** (`crates/tacsim/content/market/goods.json`; header in `src/definitions/goods.rs`):
   - four run-of-mine ores, all class `trade`, unit `t`, stowage `stow.ore`:

     | id | family | kg / L a tonne | reference_major |
     |---|---|---|---|
     | `metal_ore` | metals | 1000 / 500 | 70 |
     | `ice_ore` | volatiles | 1000 / 1000 | 35 |
     | `carbon_ore` | carbon_feedstock | 1000 / 1200 | 50 |
     | `strategic_ore` | strategic_elements | 1000 / 600 | 600 |

   - twelve rows; digest `e046e0ab504c…`;
   - no listing, vendor or bid (those are SM.5's);
   - the Dray's accepted set is unchanged, so every trade-run payload pin stands.
2. **`UnitClass::Vehicle`** (`src/definitions/unit.rs`):
   - `ALL: [_; 5]`, the word `vehicle`, `fieldable()` false for `Hauler | Vehicle`, the noun `terrain vehicle(s)`;
   - `check_logistics` refuses "a vehicle declares its logistics";
   - the door comments in `src/config.rs` and `mush-server/src/config_ext.rs`, whose sentence is byte-identical;
   - `arena::test_support::vehicle` (`src/arena/mod.rs`).
3. **An optional drive** (`src/definitions/logistics.rs`, `mod.rs`):
   - `LogisticsDef.drive: Option<DriveDef>`, with the law checks under `if let`;
   - `drive_table` skips a drive-less unit;
   - `burn::NO_DRIVE` ("unit {unit} carries no drive; it never plans a passage") in `hull_spec_of`;
   - `burn_table` skips a drive-less spec silently (`src/port/burn.rs`).
4. **The tools kind** (`src/definitions/tools.rs`, new):
   - `ToolDef`, `ToolRole::{Drill, Scanner, Battery}` and the bound checks;
   - `tool_digest`, `tools_table`, `TOOLS_DIGEST_DOMAIN = "tacsim.tools.v1"`, `MAX_TOOL_CU`;
   - three typed documents (`content/tools/drill_core_mk1.json`, `scanner_mk1.json`, `battery_mk1.json`) with §4.1's figures verbatim; the battery carries §4.1's charge as data;
   - `TOOL_SOURCES` and the three ids in `content.rs`;
   - not packs: nothing binds a tool until promotion.
5. **Bays** (`logistics.rs`):
   - `InteriorRoomKind::Bay`, `InteriorDef.bays` (last field, skipped when empty), `BayDef`, `BaySlotDef`, `EgressDef`;
   - `MAX_BAYS` 2, `MAX_BAY_ITEMS` 4, `MAX_EGRESS_OFFSET_CM` 10,000, `InteriorDef::bay()`;
   - a sentence for every rule, including "the exit `{from}` -> `{to}` touches the bay `{bay}`; a bay has no exits".
6. **Two unit packs**, both fielded as `EXTENSIONS_VEHICLES`, the eleventh table (`content.rs`):
   - **`content/units/tv_prospector.json`** (`tacsim.pack.tv_prospector@1`, `b2bdc88e4834…`):
     - class `vehicle`;
     - chassis, hopper and boom sections; power and visual_sensor components;
     - tracked 300 cm/s;
     - stowage-only logistics: 6 t dry, a 12 t / 13.2 m³ `stow.ore` hopper, no drive;
     - a cab and hopper interior.
   - **`content/units/dray_p.json`** (`tacsim.pack.dray_p@1`, `67c6558731e3…`):
     - the Dray's def byte for byte except the label, the 86 t / 103.2 m³ bulk+ore cargo, the `bay` room, `bays` (slot `rigs` max 2, egress "stern ramp" 0 / −3500 / 180000) and the note;
     - `dray.json` is untouched.
   - `hull::refit_compatible` (pub) names what differs.
7. **The hull's own unit** (`src/port/hull.rs`, `src/port/ship.rs`):
   - `HullView.unit` and `unit_held`;
   - `HullUnit { Live, Default, Held }`, `resolve_hull_unit`, `unit_record_claims` (lifted from step 2), `live_unit_id`, `spec_for`;
   - held reads dark on v10 worlds only: the planner/basis answer Unreadable, `market::hold_plan` answers None, and +tv and the refit refuse;
   - `HullView::bay()`, `is_prospector()`, `class_word()`, `HULL_CLASS_PROSPECTOR = "Dray-P"`;
   - `+ship` prints the class word.
8. **The payload counts the bay** (`src/port/passage.rs`, `voyage.rs`, `executor.rs`, `burn.rs`):
   - `CarriedRig`, `carried_kg`;
   - `PendingVoyage.carried` (skipped when empty);
   - `Basis.carried` and `voyage::carried_of`;
   - basis plans `view.unit` and adds the rigs;
   - the burn prepare re-reads each frozen rig's hopper in its own transaction;
   - beyond the brief, declared: `Planner.carried_kg`/`carrying` for the queue's dry run and fill sizing.
9. **Lane v10** (`content/lane/ceres_phobos.json`, `3f49d1764cf1…`; `LANE_VERSION` 10):
   - `ceres_port.programme.refits {prospector: tacsim.unit.dray_p}`;
   - `ceres_port.vehicles` (`occator.rig_lease`: pool 8, `OWP-TV-`, 1,400 / 5,000 / 168 IC days, counter `exchange`, shed "Occator Vehicle Shed", three tools);
   - `src/port/def.rs`:
     - `ProgrammeDef.refits` (≤ 4 rows, never `standard`, never the unit);
     - `PortDef.vehicles: Option<Box<VehiclesDef>>`;
     - `check_vehicles`, and the site sentence extended;
     - `check_ports`: one rig lessor a lane, on a port with a programme;
   - `PortMap.vehicles`/`VehiclesRow`/`RigRow` (`map.rs`).
10. **The applier** (`src/port/hull.rs`, `src/port/vehicle.rs` new, `src/port/apply.rs`):
    - part (f) step 6b lays a hidden bay room, its space and its `rigs` compartment on every programme hull;
    - the preflight checks the refits, and accepts refit bay rooms on a row;
    - `declare_space(ctx, room, anchor)` is generalised;
    - part (h): Occator's unanchored shed, then eight rigs, each a Thing, a unit, a cab and a hopper space, a hopper vault owned by `System(tacsim.vehicles)` with scope None, placed in the shed and titled to Occator;
    - `vehicle::preflight` folds into the frozen-economy check ("…nor {n} rig title(s)…");
    - `VEHICLES_PRINCIPAL` (`site.rs`);
    - the bay queries: `rigs_in_bay`, `hopper_of`, `rig_row`, `occator_lease_on`, `RigInBay`.
11. **Readiness** (`readiness.rs`, `coop.rs`):
    - `Binding::Vehicles` after the hulls line, ungated;
    - Ready reads "ceres_port vehicles: 8 of 8 made and titled to Occator Water & Propellant; 8 in the shed, 0 leased.";
    - the unlaid-bay Missing line;
    - `hull_line` skips the refit bay rooms (D20: a lost bay never closes +coop/enrol).
12. **The lease record** (`src/port/tv.rs` new, `src/store.rs`):
    - the `tacsim:tvlease:{lease}` family: `TvLease` (strict; `check` adds mark, terms_paid and ended_at rules), `TvLeaseStatus`, `TvLeaseState::Unreadable{pilot,hull,tv}`;
    - `put_tv_lease`, the load arm (an unreadable record bars only its own lease);
    - the accessors `tv_lease`, `tv_leases_of`, `tv_lease_on` (live means status ≠ Returned) and `tv_lease_unreadable_for`.
13. **The engine-owned deposit** (`mush-economy/src/api.rs`):
    - `ENGINE_OWNED_RESERVATIONS` gains `tacsim.tv.deposit`, with the layering paragraph (D18);
    - the token count stays 5.
14. **`+tv` and `+tv/lease`** (`src/commands.rs`, `tv.rs`):
    - LegacyPostImage; switches 1, 2, lease;
    - status, a pure read with the stores line on a pad;
    - take: checks 1–10, a bare preview, then on go the settle (its own unit), `offer_lease` with a 1-minor Manual rent, `accept_contract`, ONE public post (the fee plus the deposit `HoldOp::Place`), `place_interior` into the bay, the record;
    - return: keyed on the lessee's live record; `end_lease` with AlreadyEnded taken as success; the idempotent deposit settle before the record goes Returned; the rig back in the shed;
    - the term settle: renew whole terms, or capture the whole deposit and mark Forfeit;
    - `licence_seam` (always Ok, for SM.5).
15. **Charter ends with rigs aboard** (`recall.rs`, `buyout.rs`, `tv.rs`):
    - `RecallBlock::Bay` is check 7, last; it counts placements and live records;
    - `RECALL_BAY`, `BAY_PAGE`, `BUYOUT_BAY` (check 5b);
    - `tv::repossess`: every check first; per rig the deposit settles by the term (D16) before `end_lease`; the rig goes back to the shed, the record is written Returned with repossessed, and the pilot is notified;
    - `recall_block_clearing` in `revoke_cmd` (pinning on a Bay block), the dormant path and `return_hull`.
16. **`+coop/refit prospector|standard`** (`src/port/refit.rs` new; the `coop.rs` route and help line):
    - checks 1–10, then ONE write: `mush_tactics::module::refit_unit`, whose doc names its third caller.
17. **Help and gates:**
    - `docs/help/coop.md` gains "The yard's refit" and the release and recall paragraphs;
    - `docs/help/ships.md` gains "The Dray-P and Occator's rigs";
    - `legacy-handlers.toml` gains the `+tv` stanza and the `+coop`/`@coop` refreshes;
    - `state-slot-owners.toml` gains the `tests/vehicles.rs` AccountOracle row;
    - `tests/support/vehicles.rs` `strip_vehicles` is called from `strip_sites` and `strip_moon_trade`;
    - the tacsim dev-dependency `tracing-subscriber`.
18. **Tests:**
    - `tests/vehicles.rs`: 28 world tests (applier, A4, refit, lease, return, revoked return, staff-ended lease, forfeit slots, /3, buyout, revoke, return arm, dormancy, release, v9→v10, term, A1, filed burn with a loaded hopper, queue, restart plus apply, stores line, unresolved deposit, unreadable-record dark, live record out of the bay, the unlisted /n, lost bay);
    - new tests in `content.rs`, `lane.rs`, `passage.rs` and `match_lifecycle.rs`;
    - unit tests in `burn.rs`, `hull.rs`, `tv.rs`, `config.rs`, `arena/state.rs` and `config_ext.rs`.

### Deferred, and why

**SM.3b, by the lead's split (lead ruling 0):**
- the excursion state family and `tacsim:tv`;
- the `tacsim.surface` tick;
- authoritative act starts with the charge reserved;
- `+tv/launch` (and the implied launch), `+tv/recover`'s placement half, `+tv/dump`;
- the go rule on the act verbs and the last-addressed selector;
- `TV_OUT` and `HOPPER_LOADED`;
- the egress record;
- the operate gate that refuses a Forfeit or ended lease (R7);
- moving `+tv` to the authoritative class.

**SM.5:** filling `licence_seam` (doc 20 §4.3's licence refusal), the ore bids and drips, Occator's tranche, the reference_minor rows and lane v11.

**Named residuals:**
- R1: the term re-debit or forfeit at the next dock; it runs at the next `+tv/lease` act today.
- R2: a graduate's refit or lease.
- R3: a recalled Dray-P re-lent as a Dray-P (it stays a Dray-P).
- R4: the desk's `look` listing the `+tv` verbs.
- R5: Occator's own counter room.
- R6: a world test of a held unit record.
- R8: a second rig lessor.

**Lead rulings the planner took:**
1. The refit re-dresses in place through `refit_unit`, with the bay laid on every programme hull at apply (D1).
2. Tools are typed documents, not packs, with a third `battery_mk1` row (D3, D4).
3. The deposit is an engine-owned reservation (D6).
4. The lease is an economy lease with the Trust's 1-minor Manual rent, a public fee post, and the settle on the next `+tv/lease` act (D7, D8).
5. Charter ends: `RECALL_BAY` and `BUYOUT_BAY` refuse; revoke, dormancy and the return arm repossess (D14, D16).
6. "Live" means status ≠ Returned.
7. A held unit reads dark, on v10 worlds only (D19).
8. Bay completeness sits on the ungated Vehicles line (D20).
9. The counter is the Exchange Hall, and `role_room` is `pub(crate)` (D11).
10. `strip_vehicles` keeps the older-lane digests.
11. A bare `+tv/lease` previews and `go` commits (D9).

**Owner-critical defaults, for the owner's word:**
- D16, the repossession's money: within the paid term the deposit is released; past it the deposit is captured whole; the engine never debits a fee. The alternatives are always capture or always release.
- D1: every programme hull, standard Drays included, gains a hidden, empty, exitless bay room at v10.
- D11/D17: the counter in the Exchange Hall, the rigs OWP-TV-01..08, "Occator Vehicle Shed", and texts that name Occator literally.
- D18: `tacsim.tv.deposit` is the first game-namespace string compiled into mush-economy's frozen engine-owned table.
- D8/R1: the term settle runs at the next `+tv/lease` act, where doc 20 says the next dock.

**Fixer departures from the brief, kept:**
- `repossess` settles each deposit before `end_lease`, and a deposit that will not settle stops that rig with nothing written. The brief's "the repossession goes on, the hold stays Held" would strand an engine-owned hold that even staff cannot move. This serves D14; D16 is unchanged.
- `give_back` refuses `LEASE_FAILED` before writing Returned when the deposit will not settle.
- The bay guards (recall check 7, `BUYOUT_BAY`, `REFIT_BAY`) also count live records whose rig is out of the bay.

**Builder deviations, kept:**
- `PortDef.vehicles` is boxed (clippy `large_enum_variant`).
- The arena's display reads "a vehicle", not "a terrain vehicle".
- The upgrade test's enrolled half seeds a v10-stripped world and updates it to v11, because a v9-latched world's Lane line gates `+coop/enrol`.
- The return-arm test asserts that posted and held each fall by 5,000.00 with available unchanged, since a capture necessarily lowers posted.
- `+ship`'s hold gauge prints 103 m³.

**Not fixed, small (judge's notes):**
- the preview/go divergence on a suspended account (correctness F8);
- the bay guards fail open when `hull_view` is None (F5);
- `+tv` switches stop at /2 while `MAX_BAY_ITEMS` is 4 (F10);
- the shed-gone Missing line names a rig's mark;
- the stale "Both reach it" in mush-tactics;
- the "NOT FITTED FOR PHASE ONE" seat stencil;
- the ore notes say "a Ceres site" (Vesta's commons also yields ore);
- `TV_HELP`'s comma money spelling;
- the `tracing-subscriber` dev-dependency;
- the test gaps T5–T10 (the grammar refusals, `vehicle::preflight`'s refusals, the TV pin's per-value asserts).

### Residual risks the judge named

- A return on a pilot whose credit account is not Active diverges from its preview (correctness F8, unfixed). plan_return plans the settle against 0 available, so a past-term rig previews TERM_FORFEIT and RETURN_PREVIEW_FORFEIT. On go, run_settle is skipped when pilot_account is None, so the deposit is released and RETURNED is printed. This fails generous; it only happens when staff suspend an account with a rig past term.
- The bay guards fail open when hull_view is None (correctness F5). recall_block check 7 and buyout 5b sit under `if let Some(view)`, and view_of now returns None when the hull's live unit names a definition missing from the fielded catalog (for example a staff @unit re-create). Release, revoke or buyout could then pass with a rig aboard. Only reachable through staff action.
- The apply preflight still refuses a recorded bay room that is gone (by design: never re-create). A destroyed bay room no longer closes +coop/enrol (fixed), but it does block every later ceres_port content apply until staff repair the map.
- give_back can answer LEASE_FAILED ('nothing ... was signed') after end_lease has already ended the economy lease. This happens only on an economy fault at the deposit step. The record stays live and the return re-runs cleanly (AlreadyEnded plus an idempotent settle), but the reply over-promises.
- The take's post-settle refusal path (refuse_after_writes after step 0) and its hold-0 breach are reachable only through an economy fault; no test reaches the former. The hold-0 record is now caught fail-closed by deposit_hold and has a test.
- +tv addresses only /1 and /2 while MAX_BAY_ITEMS is 4 (correctness F10). A later unit authoring 3 or 4 rigs a bay leaves slots 3 and 4 unaddressable until SM.3b generalises the selector.
- Test coverage gaps (tests T5–T10):
- no positive control that a hopper is a live endpoint for staff, in A4;
- the revoked-pilot status line is matched by its shared prefix;
- the TV pin test does not assert mobility, facets, bands or silhouettes value by value (the digest locks them);
- no grammar test for LEASE_USAGE, NO_RIG, NO_RIG_FREE, REFIT_USAGE or REFIT_NOT_OFFERED;
- no test for vehicle::preflight's refusals or the Vehicles line's other Missing texts;
- no world test of a held unit record (R6).
- Fiction and doc nits left as shipped:
- tv_prospector.json's cab detail says 'NOT FITTED FOR PHASE ONE', design jargon (fixing it re-signs TV_DIGEST);
- the four ore notes say 'drilled from a Ceres site', but Rheasilvia Commons is on Vesta;
- mush-tactics refit_unit's doc still opens its next paragraph 'Both reach it' after naming three callers;
- when the shed is gone, the Vehicles line names a rig's mark with the shed's object id.
- TV_HELP spells money '1,400.00cr' / '5,000.00cr' (the brief's verbatim text), while the house format and docs/help/ships.md use '1400.00cr'.
- A new tacsim dev-dependency on tracing-subscriber (a workspace crate already, not a composition edge; the gates passed). match_lifecycle.rs already shows a hand-rolled capture that would avoid it.
- The upgrade proof for an enrolled hull goes through a proxy: the world is seeded as v10-stripped and updated to v11, because a v9-latched world cannot enrol. It is the same applier path, but no test lifts a v9 world that has a live charter.

### The lead's fix round (before the commit)

the TV cab's world text lost its design jargon, the ore notes name a Registry site, the lease help prints money in the house format, the bay guards fail closed on a dark hull view, and grammar, preflight, readiness and held-record tests were added.

1. **1. Design jargon in the TV cab's seat detail** — fixed: The seat detail in C:/Users/faeni/cc/rustmush/crates/tacsim/content/units/tv_prospector.json now reads, in world: 'Empty, its harness clipped back. A stencil on the headrest says CREW SEAT; the buckle has never been fastened, because rigs like this one are driven from the bridge, and somebody has scratched a question mark after the stencil.' I re-signed the pack in place with TACSIM_WRITE_CONTENT=1 (sm3a_the_terrain_vehicle_is_pinned_and_checks), so the manifest digest is now c55b9b5e30fa4ac21050f5b48bb6bfa9fa8ac2b2d2e7fca3dbfdbeca455f6c1f. TV_DIGEST in crates/tacsim/tests/content.rs moved from b2bdc88e… to that value. Nothing else reads this digest: a repo-wide grep finds no other copy, dray_p.json only requires the pack by id and version (DRAY_P_DIGEST unchanged), and the lane, sites table, chart fixture and geology pins do not move.
2. **2. The ore notes named only Ceres** — fixed: In crates/tacsim/content/market/goods.json, the four ore rows (metal, ice, carbon and strategic ore) now say 'drilled from a Registry site' (doc 20: every site is the Registry's, Vesta included) instead of 'a Ceres site'. Re-pin: GOODS_DIGEST in crates/tacsim/tests/content.rs moved from e046e0ab… to d8e79a2cfdd8468eb1555673babea71cb27847604b6251f991581720b68f088e (printed by ic2_the_goods_table_is_pinned under TACSIM_WRITE_CONTENT=1). market.rs computes the digest at runtime, the web fixtures do not carry it (grep), and no other pin moved.
3. **3. Money spelled with thousands separators** — fixed: Only one literal was wrong: TV_HELP in crates/tacsim/src/port/tv.rs said '1,400.00cr' and '5,000.00cr'. It now says '1400.00cr' and '5000.00cr'. Every lease preview, receipt and settle line already goes through coop::money_minor (the house renderer), as the existing tests' FEE and DEPOSIT constants show. docs/help/ships.md already printed 1400.00cr and 5000.00cr, and grep finds no thousands-separated cr amount in docs/help or in tacsim's src. New test vehicles.rs sm3a_the_lease_help_names_money_as_the_receipts_do checks that both TV_HELP and docs/help/ships.md name FEE and DEPOSIT in the house form and spell neither 1,400.00cr nor 5,000.00cr.
4. **4. Bay guards passed when the hull view could not be read (correctness F5)** — fixed: crates/tacsim/src/port/recall.rs, recall_block check 7: when hull_view returns None, it now logs an error and returns (RecallBlock::Bay, TV_DARK with the name filled in). The pilot's +coop/release therefore hears the rigs' dark line ('The {name}'s rigs cannot be read right now; nothing changes.'). @coop/revoke, the dormancy recall and the return arm go through recall_block_clearing. There, repossess refuses on the same missing view, so the Bay block stands: the revoke answers NOT_REVOKED plus the dark line and writes nothing, and the tick holds the hull and pages BAY_PAGE, as it already did for a non-empty hopper. BAY_PAGE now also names this cause and its fix: '…or the hull's unit names a definition the catalog does not field so its bay cannot be read); staff empty the hopper (…), repair the tacsim:tvlease: record, or bind the hull's unit back to a fielded definition; the recall retries'. crates/tacsim/src/port/buyout.rs, check 5b: a None view now logs and refuses with TV_DARK (name filled in). A test each, in vehicles.rs: sm3a_a_dark_hull_view_refuses_the_release, _refuses_the_buyout, _holds_the_revoke, and _holds_the_dormancy_recall_and_pages. Each uses the dark_view helper, which leases rig 1, has staff run '@unit/remove' then '@unit/create #hull=tacsim.unit.line_tank', and asserts that hull_view is None. Each test then asserts the exact reply (or BAY_PAGE in the captured log), the record still Active, the lease live, the deposit Held, the rig in the bay, and the charter and enrolment Active.
5. **5. refit_unit's doc comment said 'Both' after naming three callers** — fixed: In crates/mush-tactics/src/module.rs, 'Both reach it under the tacsim module's identity…' now reads 'All three reach it…'. All three callers (lobby::start, reset::reset_one, port::refit) are tacsim code.
6. **6. Test gaps** — fixed: All new tests are in crates/tacsim/tests/vehicles.rs (section 11) and drive the real verbs on the composition root. Each grammar test asserts the whole reply.
- sm3a_the_tv_grammar_answers_lease_usage: LEASE_USAGE for /1/2, /1/2/lease, a stray word, an unknown act, a word after go, and 'lease return go twice'.
- sm3a_a_return_names_no_rig_that_is_not_there: NO_RIG for rig 1 with an empty bay, and for rig 2 with rig 1 leased (rig 1 stays Active and in the bay).
- sm3a_an_empty_shed_answers_no_rig_free: four Dray-Ps take all eight rigs, then a fifth pilot hears NO_RIG_FREE on both the preview and go; nothing is written and the balance is unchanged.
- sm3a_the_refit_grammar_answers_refit_usage: no word, two words, and an unauthored word.
- sm3a_a_programme_without_refits_answers_refit_not_offered: the shipped lane with SM.3a's blocks stripped at the shipped version, both words, and the hull stays a Dray.
- sm3a_the_rig_pools_preflight_refuses_each_recorded_breach: each vehicle::preflight refusal through a real @atlas/pack plan and apply of the next lane version from the shelf, with the exact sentence, no 'Applied', and the rig rows unchanged. The cases: the block dropped, pool 4, prefix re-marked, an unregistered issuer, a unit with no interior, and (with a fee edit so the update reaches the preflight) a destroyed rig and a destroyed shed.
- sm3a_the_vehicles_line_names_what_is_missing: the other Missing texts. A destroyed rig and '@asset/transfer' to GOD are real staff acts. '6 of 8 are made', interior incomplete and no hopper vault come from editing the stored port map before a reboot. The hulls line stays Ready.
- sm3a_a_held_unit_record_reads_dark: the world test the lead asked for if it cost under an hour. The unit:<hull> document gets record_version 99 and the world reboots, so the record is quarantined and HullView.unit_held is true. +tv answers TV_DARK and +ship/plan answers PLAN_DARK (from PlanError::Unreadable). With a rig aboard, +coop/refit answers REFIT_BAY, because the refit's bay check comes before its held check by design. A second world with an empty bay gets REFIT_FAILED and no re-bind.
The new reboot_editing(key) helper sits beside reboot_editing_lease.

**The round's review.** I reviewed the delta from the judged tree c4074cbc to the new working tree 0d0992f973cf18113828928263eb31b90eafae0f, computed with a scratch index. The only other difference is Cargo.lock: the working lock hashes to 1a4dd7a7, the same blob as the judged tree's, and the scratch-index diff shows it only because the real index holds HEAD's lock. Nothing was run with cargo. The fixer's logs in scratchpad/fix-sm3a show all 13 new vehicles tests passing (41 in that file), and the tacsim, mush-tactics, mush-economy and server suites green.

Item by item:
1. Done. The seat detail is in-world. The TV pack was re-signed at version 1 (unpublished) and TV_DIGEST moved to c55b9b5e…. Neither the old digest nor the old text appears anywhere else in the tree.
2. Done. All four ore notes say 'a Registry site', which matches doc 20 ('every site is the Registry's'). GOODS_DIGEST moved to d8e79a2c…. Goods notes are authoring notes and are never rendered (check_note only), so nothing reaches players.
3. Done. TV_HELP now prints 1400.00cr and 5000.00cr. Every lease line goes through money_minor, and docs/help/ships.md already matched. There are no thousands-separated cr amounts left in tacsim src or docs/help. A static test pins both texts.
4. Done and correct. recall_block check 7 and buyout check 5b now fail closed with TV_DARK when the hull view is None. Revoke, dormancy and the return arm go through recall_block_clearing. Its repossess refuses on the missing view before any write, so the Bay block stands: the revoke answers NOT_REVOKED plus the dark line, and the tick holds the hull and pages BAY_PAGE. Every other bay reader was already fail-closed or required a view (the refit, the take, the return and the payload). recall_block already returns early when programme_port is None, so the only new None cases are a hull row missing from the map (a store fault) and a unit spec with no interior or no logistics. The four regression tests would fail on the judged tree, which let the release, buyout and revoke through with a rig aboard.
5. Done. The comment now says 'All three reach it', after naming three callers.
6. Mostly done. The grammar tests cover LEASE_USAGE, NO_RIG (both branches), NO_RIG_FREE, REFIT_USAGE and REFIT_NOT_OFFERED, each through the real verb with the whole reply. The Vehicles line's other Missing texts are covered. The held-record world test covers TV_DARK, PLAN_DARK and REFIT_BAY, and REFIT_FAILED in a second world; the bay-before-held order is justified as fail-safe. The preflight test leaves the hopper-vault and unauthored-room refusals untested (R2).

No assertion was weakened: the only changed lines in existing tests are the two digest constants, each moved with its reason. There is no RNG, no truth leak, and every new refusal reuses an existing line.

There is no must-fix. Two should-fix items:
- R1: BAY_PAGE and the recall comment describe the dark cause as an unfielded definition, but the tested trigger, line_tank, is fielded and has no interior. A paged staff member would be sent the wrong way.
- R2: the two missing preflight refusals.

The rest are notes: the RecallBlock::Bay doc, the untested return arm, TV_DARK's 'rigs' wording on a no-vehicles world, a misnumbered test section, and a prose nit.

- **R1** (should-fix): BAY_PAGE (recall.rs:137) now says the hull's unit 'names a definition the catalog does not field' and tells staff to 'bind the hull's unit back to a fielded definition'. The comment at recall.rs:294-296 says the same. But view_of goes dark through spec_for (hull.rs:237-238: `spec.interior.as_ref()?, spec.logistics.as_ref()?`), so a fielded definition with no interior or no logistics also reads None. That is exactly the case all four new F5 tests use: dark_view binds tacsim.unit.line_tank, its own doc comment (vehicles.rs:3112) says 'a definition with no interior', and the preflight case at vehicles.rs:3402 shows line_tank is fielded ('carries no interior', not vehicle.rs:77's 'is not fielded'). A staff member paged for the tested case is sent looking for an unfielded definition, and is told to rebind to 'a fielded definition', which the line tank already is.
- **R2** (should-fix): Item 6 asked for tests of vehicle::preflight's refusals, and the return says each one is covered. sm3a_the_rig_pools_preflight_refuses_each_recorded_breach (vehicles.rs:3349) covers seven: block dropped, pool shrunk, mark changed, issuer, no-interior unit, destroyed rig, destroyed shed. Two data-driven refusals have no test anywhere under crates/tacsim/tests (grep for 'names hopper vault' and 'does not author' finds nothing). The first is the hopper-vault refusal at vehicle.rs:201-212 (vault gone or not owned by tacsim.vehicles), which guards doc 20 section 4.2's rule that no player owns a hopper. The second is the recorded-room-the-template-does-not-author refusal at vehicle.rs:195-200.
- **R3** (note): The RecallBlock::Bay doc (recall.rs:187-189) still reads 'A rig in the hull's bay ... Checked last'. Since this round, Bay also means the hull's view could not be read (recall.rs:301-308), and the callers rely on that to fall into their hold-and-page handling.
- **R4** (note): return_hull (recall.rs:514-527), the tick's return arm for a Revoked charter whose Trust lease is terminal, is a third path into the dark Bay block through recall_block_clearing, and it pages BAY_PAGE at recall.rs:1119-1120. The new tests cover release, buyout, revoke and dormancy but not this arm. Its behaviour follows from the same shared function, so the risk is low.
- **R5** (note): The dark refusal on release, buyout and revoke is TV_DARK, 'The {name}'s rigs cannot be read right now; nothing changes.' (recall.rs:307, buyout.rs:394). This is now reachable on a standard Dray, or on a world whose programme authors no vehicles (pre-v10), if staff re-bind its unit to a non-hull definition. Such a pilot hears about rigs they never had. Two other dark-view paths a Dray-P pilot can reach are unchanged and answer LEASE_FAILED rather than TV_DARK: the take (tv.rs:1154) and the return (tv.rs:1599). These predate this round.
- **R6** (note): The new section header at vehicles.rs:2972 reads '// 11. The lead's pre-commit round'. It follows section 13 (line 2738) and repeats section 11 ('Equal payload, equal plan', line 2362). The new `destroy` helper (vehicles.rs:3319) duplicates the inline destroy-and-emit block in sm3a_a_lost_bay_room_never_closes_the_desk (vehicles.rs:2937-2945). The return also says a destroyed rig is 'a real staff act', but it is the test rig's world-level destroy plus an emitted ObjDestroyed, not a staff command.
- **R7** (note): The rewritten seat detail (line 116) is in-world and the jargon is gone. Its clause order makes the scratched question mark read as part of the 'because' reason: '...; the buckle has never been fastened, because rigs like this one are driven from the bridge, and somebody has scratched a question mark after the stencil.'

**Re-fixed after the review:**

- **R1** — fixed: BAY_PAGE in crates/tacsim/src/port/recall.rs:137 now gives the dark-view cause as 'the hull's unit is bound to a definition that is not a hull (not fielded, or fielded with no interior or no logistics) so its bay cannot be read', and the remedy is now 'bind the hull's unit back to the Dray or the Dray-P'. This matches spec_for in hull.rs, which returns None for a missing spec or a missing interior or logistics block. I reworded the matching recall_block comment at recall.rs:293-301 the same way. buyout.rs:385-389 only says that an unreadable view fails closed and that the bay cannot be seen, which is accurate, so I left it unchanged. The tests read the constant, so no pins moved.
- **R2** — fixed: Added two cases to sm3a_the_rig_pools_preflight_refuses_each_recorded_breach in crates/tacsim/tests/vehicles.rs and extended its doc comment. Both use the existing assert_apply_refused helper, which asserts that each sentence appears, that 'Applied ' does not, and that the rig rows are unchanged. (a) reboot_editing_map(with_next(fee)) renames rigs[0].rooms 'hopper' to 'galley'. It asserts the exact sentence 'ceres_port: rig OWP-TV-01 in the port map {key} records room galley, which the template does not author'. (b) On with_next(fee), a pilot charters the Salt Moth, and the test reads that hull's hold vault through hold_vault. It first asserts that the vault is live and is not owned by tacsim.vehicles. reboot_editing_map then sets rigs[0].vault to that vault id. The test asserts the exact sentence 'ceres_port: rig OWP-TV-01 in the port map {key} names hopper vault #{hold}, which is gone or not owned by tacsim.vehicles; it is never re-created — repair the map by hand'.

**Pins the round moved.** Two pins moved, both re-blessed. TV_DIGEST in crates/tacsim/tests/content.rs: b2bdc88e483450268040abff8f52e7d279ff29bc20a7707f9bddc4ea8113a827 -> c55b9b5e30fa4ac21050f5b48bb6bfa9fa8ac2b2d2e7fca3dbfdbeca455f6c1f, because the seat detail's prose changed. The pack's manifest digest in crates/tacsim/content/units/tv_prospector.json was re-signed in place with TACSIM_WRITE_CONTENT=1, still at version 1. GOODS_DIGEST in the same file: e046e0ab504cb46948c3a15f6ac2d87471e7cdd13ec381880786beda90bdc1ca -> d8e79a2cfdd8468eb1555673babea71cb27847604b6251f991581720b68f088e, because the four ore notes changed; goods.json is still version 3. Unmoved, with grep evidence that none of them embeds either digest or the changed text: DRAY_P_DIGEST, the tool digests, the lane digest and signatures (lane v10), the sites table, the geology pins, and the chart fixture web/src/lib/fixtures/chart/sky_chart.json. No existing assertion was changed apart from those two constants.

### Pins re-blessed

**Content digests:**
- `GOODS_DIGEST`: `50cb8d9f…` → `e046e0ab504c…`. Goods v3 adds four ore rows. `ic2_the_goods_table_is_pinned` now pins version 3 and 12 rows.
- New pins:
  - `TV_DIGEST` `b2bdc88e4834…`
  - `DRAY_P_DIGEST` `67c6558731e3…`
  - `TOOL_DIGESTS`: drill `8120813eafbf…`, scanner `e25ff198480f…`, battery `198bf543540c…`
- `DRAY_DIGEST` is unchanged: `dray.json` is untouched.
- `LANE_DIGEST`: `cce73a4c…` → `3f49d1764cf1…`, version 9 → 10, for the refits row and the vehicles block (`tests/lane.rs`; the `LANE_VERSION` assertion in `tests/sites.rs`). The entry counts and the a8 spelling guard stand unedited.

**Older-lane digests (not re-pinned):**
- The rebuilt v9 lane is pinned at `cce73a4c…` in `vehicles.rs`.
- `LANE_V8_DIGEST` and commerce's published v2 do not move: `strip_sites` and `strip_moon_trade` now call `strip_vehicles`.

**Counts:**
- Catalog rows in `capacity.rs`: 8 → 12 (goods v3).
- `orders.rs` UNKNOWN_GOOD list: 8 → 12 goods (goods v3).
- Two fielded units moved these counts:
  - fielded specs 12 → 14 (`content.rs` m3 and e1; `extensions.rs` ae1, ae2 and ae3);
  - `+1` → `+3` (`ic2_the_dray_is_a_hauler…`);
  - `stock + 20` → `+ 22` (q8);
  - `FIELDED_EXTRAS` 11 → 13 (`definitions.rs`).
- Class buckets: Hauler is now `[dray, dray_p]`, and a new Vehicle bucket holds `[tv_prospector]`. The class-word clause now names four units.
- `hulls.rs`: rooms per hull 7 → 8 and objects `+18` → `+19`, because step 6b lays a bay on every hull. The test also asserts the bay's space, that the bay is not an airlock member, and that `rigs` holds at most 2.
- `ports.rs`: the readiness listing gains the Vehicles line after the hulls line.
- `coop.rs`: `EVERY` 18 → 19, with Vehicles ungated.

**Code-level pins:**
- `mush-economy`'s `vf_c2` and `tests/e7c11.rs` `vf_g2`: the three-entry engine-owned table. The message now reads "three entries, all MONEY holds: …"; the token count of 5 does not move.
- Changed refusal orders, with no existing pin moved: recall check 7 (the bay) and buyout check 5b.
- The bay pages are new text. The other pages are byte-identical.

**Compiler-required edits (no assertion changed):**
- `drive.as_ref()`/`as_mut()` in `tests/{content,landing,moon_ports,orders,passage,sites}.rs` and in `ic_slice_loop.rs`.
- The literals in the `def.rs` tests.
- The `PendingVoyage` fixture's `carried` field.
- The Dray test's exhaustive `Bay` arm.

**Gate files:**
- `legacy-handlers.toml`: the `+tv` stanza and the `+coop`/`@coop` refresh. `PINNED_BASELINE` does not move.
- `state-slot-owners.toml`: a `tests/vehicles.rs` AccountOracle row, named by the gate's first run.

**Unmoved:**
- the panic budget, the restriction census, module-ns owners and the composition edge;
- the boot-parity tallies and feed manifest;
- the TUI counts;
- the `+chart` fixture;
- `provider_rng_policy` and `truth_serialization_policy`.

### How it was verified

**Builder, fixer, final gate.** The builder built and ran the brief's suites. Three reviewers followed: correctness, conformance and tests, with ten findings each and no must-fix. A fixer then closed every should-fix: correctness F1/F2/F3, conformance F1–F5 and tests T1–T4. The final gate ran on the fixed tree at HEAD bd718fb2, and `git status` was identical before and after the sweep (65 entries, all this cut's, no peer edits). Gate results:
- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets -j 8 -- -D warnings`: Finished, exit 0.
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8`: exit 0, no warnings.
- `cargo test -j 8 -p tacsim --test content --test lane --test capacity --test passage --test extensions`: content 35, lane 13, capacity 17, passage 6, extensions 89 passed.
- `cargo test -j 8 -p tacsim --test vehicles`: test result: ok. 28 passed; 0 failed.
- hulls 15, coop 23, ports 21, commerce 26, voyage 25, orders 33, sale 18, sites 24, landing 7, moon_ports 5, price 8, stations 21, chart 7: all ok.
- `--test match_lifecycle sim_unit_never_offers`: ok. 2 passed.
- `cargo test -j 8 -p tacsim --lib`: ok. 1275 passed; 0 failed; 1 ignored.
- `cargo test -j 8 -p tacsim --no-fail-fast`: 54 result lines, all ok; 2,794 passed, 0 failed.
- `cargo test -j 8 -p mush-economy`: 540 passed, 0 failed (e7c11 ok).
- `mush-tactics --lib`: 5 passed.
- `mush-server --lib config_ext`: 15 passed.
- `mush-server` integration: boot_parity 21, ic_slice_loop 6, ic_buyout_dormancy 14, provisioning_smoke 4.
- `cargo test -j 8 -p mush-contract-tests`: 130 passed, 0 failed. This includes legacy_handler_allowlist, state_slot_owners, panic_budget, provider_rng_policy, truth_serialization_policy, restriction_lints and module_ns_owners.
- `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8`: 412 result lines, all ok; 8,182 passed, 0 failed.
- `cargo test -p mush-bot -j 8`: 717 passed, 0 failed.

**Lenses, as judge.** I read the whole diff and every new file myself. Checked:
- each fix against its finding, with a test that would fail on the old code;
- no existing assertion weakened;
- the builder's surface against the code;
- no RNG: the gate's `rand` grep hits only "strand"; `grep -nw` is empty;
- no applier submission: part (h) issues only `create_unit`, `create_vault`, `place_interior` and `issue_title`;
- no truth leak: `+tv` reads only the actor's own records and hull.

**Flakes:** none identified. Nothing was re-run for LNK1104 or load, and nothing is peer-attributed.

**Gate notes.** All 17 commands passed on the first run. Nothing needed a rerun: no LNK1104, no hung binary, and no load flake.

- **Base:** HEAD stayed at bd718fb2 for the whole run.
- **Working tree:** `git status --short` was identical before and after the workspace sweep (65 entries). There were no new peer (astronomy session) edits during the run, and nothing had to be attributed to the peer.
- **Uncommitted files:** all of them are the cut's own. These are the modified tacsim, mush-economy, mush-server, mush-tactics and mush-contract-tests files plus docs/help/coop.md and docs/help/ships.md. The untracked files are content/tools/, units/dray_p.json, units/tv_prospector.json, definitions/tools.rs, port/refit.rs, port/tv.rs, port/vehicle.rs, tests/support/vehicles.rs and tests/vehicles.rs.
- **Clippy and docs were cache hits:** clippy (step 2) finished in 0.90s and the strict rustdoc build (step 3) in 0.73s, because the builder had already run them on this tree. Cargo checks source modification times, so the cached passes still count for the current tree. Neither log has a warning or error line.
- **The brief's rand grep:** the plain substring grep over port/{vehicle,tv,refit}.rs and definitions/tools.rs prints three lines, but every hit is the word "strand"/"stranded" in a comment (tv.rs:1005, tv.rs:1761, refit.rs:168). A whole-word grep (`grep -nw rand`) prints nothing (exit 1), so there is no RNG use. The brief's literal command is not silent, though, and the reviewer may want to know.
- **Logs:** every log is in C:/Users/faeni/AppData/Local/Temp/claude/C--Users-faeni-cc-rustmush/0afc2ac2-d687-46e8-b233-de9758bdcbf4/scratchpad/cuts/SM.3a/gate1/ (01-fmt.log through 17-mush-bot.log, plus status-before-sweep.txt and status-after.txt).

**The lead round's gate, all first run unless noted:**

- pass: `cargo fmt --all -- --check` — exit 0, no output (01-fmt.log empty)
- pass: `cargo clippy --workspace --all-targets -j 8 -- -D warnings` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.67s; exit=0; 0 warning/error lines (cache hit: the lead had already run it on this content)
- pass: `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.65s; Generated C:\Users\faeni\cc\rustmush\target\doc\mush_ansi\index.html and 45 other files; exit=0; 0 warning/error lines (cache hit: target/doc/tacsim/index.html is stamped 03:07:38, after the last source edit, vehicles.rs at 03:02:49)
- pass: `cargo test -j 8 -p tacsim --test vehicles --test content --test coop --test capacity --test hulls --test lane --test sites` — capacity: test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
content: test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.74s
coop: test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.25s
hulls: test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.08s
lane: test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.50s
sites: test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.04s
vehicles: test result: ok. 41 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.26s
exit=0
- pass: `cargo test -j 8 -p tacsim --no-fail-fast` — 54 'test result:' lines, all ok; summed 2,807 passed, 0 failed. Highlights: unittests test result: ok. 1275 passed; 0 failed; 1 ignored; vehicles ok. 41 passed; content ok. 35; extensions ok. 89; match_lifecycle ok. 143; passage ok. 6; hulls ok. 15; coop ok. 23; ports ok. 21; orders ok. 33; voyage ok. 25; sites ok. 24; landing ok. 7; moon_ports ok. 5; stations ok. 21; commerce ok. 26; sale ok. 18; price ok. 8; chart ok. 7; harness ok. 24 passed; 9 ignored; doc-tests ok. 0 passed. exit=0
- pass: `cargo test -j 8 -p mush-tactics` — unittests: test result: ok. 5 passed; 0 failed
e7c11 ok. 1; ground ok. 4; mobility ok. 12; module ok. 75; review_ground ok. 1; s8_ground_npc ok. 1; seams ok. 7; unit ok. 28; window ok. 2; Doc-tests ok. 0
exit=0
- pass: `cargo test -j 8 -p mush-economy` — 13 result lines, all ok; summed 540 passed, 0 failed. unittests ok. 37; e2a 12; e2b 33; e3_verify 12; e3a 46; e3b 37; e3b2 33; e4a 83; e4b1 85; e4b2 117; e4c 44; e7c11: test result: ok. 1 passed; 0 failed; Doc-tests ok. 0. exit=0
- pass: `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop --test ic_buyout_dormancy` — boot_parity: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.31s
ic_buyout_dormancy: test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.86s
ic_slice_loop: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.68s
exit=0
- pass: `cargo test -p mush-contract-tests -j 8` — 30 result lines, all ok; summed 130 passed, 0 failed (scale_harness 6 passed; 8 ignored). legacy_handler_allowlist ok. 4; state_slot_owners ok. 5; panic_budget ok. 8; provider_rng_policy ok. 1; truth_serialization_policy ok. 1; restriction_lints ok. 11; module_ns_owners ok. 4; help_commands ok. 4; unittests ok. 31. exit=0
- pass: `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8` — 412 'test result:' lines, every one ok; summed 8,195 passed, 0 failed, 93 ignored. exit=0. The one 'panicked' line in the log is the armed fault in ws01_s8_server_worker: "panicked at crates\mush-server\src\process_fault.rs:197:9: WS01-S8 armed world-turn fault on \"@ws01s8-fault\"". It is deliberate, and its suite passed.
- pass: `cargo test -p mush-bot -j 8` — 25 result lines, all ok; summed 717 passed, 0 failed, 7 ignored. unittests ok. 469 passed; arena_lane ok. 87; e2e ok. 49; arena_contract ok. 19; arena_e2e ok. 10. exit=0. adversarial_reduction_final_rerun_and_blind_packet printed 'running for over 60 seconds' and then finished ok.

All 11 commands passed on their first run. No reruns, LNK1104 errors, hung binaries, load flakes or peer-attributed failures.

**What was tested.** The scratch-index tree, built with the brief's pathspec (crates docs deploy web mushd.toml.example), is 623ebc110934d10aee86eba45126cb73526827a0. It was the same before and after the run. HEAD stayed at bd718fb2. `git status --short` had 65 entries and did not change across three snapshots: before, before the sweep and after. All 65 are this cut's files, so no peer edits were seen.

**Pathspec caveat.** The brief's pathspec leaves out the root Cargo.lock. Because of that, `git diff c4074cbc… 623ebc11…` falsely shows Cargo.lock losing the tacsim 'tracing-subscriber' line. In fact the working-tree Cargo.lock hashes to 1a4dd7a7, the judge tree's own blob, and crates/tacsim/Cargo.toml still declares the dev-dependency. The tree with Cargo.lock and Cargo.toml added is 1e896e67decd9721b16e3a02a616f78656a9932a.

**Lead-round changes since the judge's tree.** Eight files changed (saved as lead-diff.patch and lead-diff-stat.txt):
- buyout.rs and recall.rs: the bay guards now fail closed when hull_view is None. They answer TV_DARK, and BAY_PAGE is extended (correctness F5).
- tv.rs: TV_HELP money is spelled 1400.00cr / 5000.00cr.
- goods.json: the ore notes say "a Registry site", so GOODS_DIGEST is re-pinned to d8e79a2c….
- tv_prospector.json: the seat stencil text is reworded, so TV_DIGEST is re-pinned to c55b9b5e….
- mush-tactics module.rs: the doc now reads "All three reach it".
- tests/vehicles.rs: +608 lines, taking vehicles from 28 to 41 tests.

The full tacsim count is 2,807, up from 2,794 at the judge's gate. The workspace sweep is 8,195, up from 8,182.

**Cache hits.** Clippy and rustdoc were cache hits. Cargo fingerprints by mtime, and target/doc/tacsim/index.html is stamped 03:07:38, after the newest source edit (vehicles.rs, 03:02:49). So both cached passes cover the current content.

Logs are in C:/Users/faeni/AppData/Local/Temp/claude/C--Users-faeni-cc-rustmush/0afc2ac2-d687-46e8-b233-de9758bdcbf4/scratchpad/cuts/SM.3a/leadgate1/: 01-fmt.log through 11-mush-bot.log, plus status-before.txt, status-before-sweep.txt, status-after-sweep.txt and status-after.txt. Nothing was staged, committed or stashed, and the real index was not touched (the scratch index copies were deleted).

### Surface for the next cuts

**Hull view** (`crates/tacsim/src/port/ship.rs`)
- `HullView.unit: CanonicalId`.
- `HullView.unit_held: bool`, only on worlds whose programme authors `refits`.
- `HullView::bay() -> Option<(&BayDef, ObjId)>`, `is_prospector()`, `class_word()`.
- `HULL_CLASS_PROSPECTOR = "Dray-P"`.

**Hull helpers** (`crates/tacsim/src/port/hull.rs`)
- `pub fn refit_compatible(&UnitSpec, &UnitSpec) -> Result<(), String>`.
- `pub(crate)`: `HullUnit {Live, Default, Held}`, `resolve_hull_unit`, `unit_record_claims`, `live_unit_id`, `spec_for`, `refit_bays`, `bay_laid_up`.
- Also `pub(crate)`: `declare_space(ctx, room, SpaceAnchor)`, `dress`, `live`.

**Bay queries** (`crates/tacsim/src/port/vehicle.rs`)
- `pub fn template(store, &VehiclesDef)`.
- `pub struct RigInBay {slot: u8, tv, mark, hopper: Option<VaultId>, lease: Option<u64>}`.
- `pub fn rigs_in_bay(w, &HullView) -> Vec<RigInBay>`: the numbered rigs first; slot 0 means unlisted.
- `pub fn hopper_of`, `pub fn rig_row`, `pub fn occator_lease_on(w, lessor, tv)`.
- `pub(crate)`: `preflight`, `lay_up`, `vehicles_line`, `bay_rooms`, `carried_rigs`.

**Principal:** `site::VEHICLES_PRINCIPAL = "tacsim.vehicles"`.

**Lease record** (`crates/tacsim/src/port/tv.rs`, `crates/tacsim/src/store.rs`)
- `TvLease` fields: `schema`, `lease`, `tv`, `mark`, `pilot`, `hull`, `slot: u8`, `hold`, `fee_minor`, `deposit_minor`, `term_secs`, `leased_at_ic`, `term_ends_ic`, `terms_paid`, `status`, `ended_at_ic?`, `repossessed`.
- `TvLeaseStatus {Active, Forfeit, Returned}`; `TvLease::check`; `TvLease::live` (status != Returned).
- `TvLeaseState {Present, Unreadable{pilot, hull, tv}}`.
- `TV_LEASE_PREFIX`, `TV_LEASE_SCHEMA = 1`, `tv_lease_key`.
- Store accessors: `tv_lease`, `tv_leases_of`, `tv_lease_on` (the live one), `tv_lease_unreadable_for`.
- Writer: `put_tv_lease` (`pub(crate)`).

**Lease seams and namespaces** (`tv.rs`)
- `pub(crate)`: `licence_seam(w, pilot)` (always Ok; SM.5 fills it), `repossess(ctx, hull) -> Result<usize, String>`, `bay_slot_word`.
- Namespaces: `DEPOSIT_KIND`, `LEASE_KEY_NAMESPACE`, `FEE_KEY_NAMESPACE`, `FORFEIT_KEY_NAMESPACE`.
- Every text constant, `TV_VERB` and `TV_HELP`.

**Recall and buyout**
- `recall::recall_block_clearing`, `RecallBlock::Bay`, `RECALL_BAY`, `BAY_PAGE`; `recall::chartered` is now `pub(crate)`.
- `buyout::BUYOUT_BAY`.

**Refit** (`crates/tacsim/src/port/refit.rs`): `REFIT_*` texts and `REFITTED_*`.

**Payload**
- `port::{CarriedRig {tv, dry_kg, hopper: Option<u64>, goods}, carried_kg}`.
- `PendingVoyage.carried`.
- `voyage::carried_of(ctx, view)` (`pub(crate)`).
- `executor::Planner.carried_kg` and `Planner::carrying`.
- `burn::NO_DRIVE`.

**Definitions**
- `definitions::tools::{ToolDef, ToolRole::{Drill, Scanner, Battery}, DrillDef, YieldDef, ScannerDef, BatteryDef, tool_digest, tools_table, TOOLS_DIGEST_DOMAIN, MAX_TOOL_CU}`.
- `content::{DRILL_CORE_MK1, SCANNER_MK1, BATTERY_MK1, TOOL_SOURCES, TV_PROSPECTOR, DRAY_P, EXTENSIONS_VEHICLES}`; `LANE_VERSION = 10`.
- `BayDef {room, slot: BaySlotDef {id, maximum_items}, egress: EgressDef {label, x_cm, y_cm, heading_mdeg}}`.
- `InteriorDef.bays`, `InteriorDef::bay()`, `InteriorRoomKind::Bay`, `MAX_BAYS`, `MAX_BAY_ITEMS`.
- `UnitClass::Vehicle`; `LogisticsDef.drive: Option<DriveDef>`.

**Port definitions**
- `port::def::{VehiclesDef, STANDARD_REFIT, MAX_VEHICLE_POOL, ...}`.
- `PortDef.vehicles: Option<Box<VehiclesDef>>`.
- `ProgrammeDef.refits: BTreeMap<word, unit>`.
- `PortMap.vehicles: Option<VehiclesRow {shed, rigs: Vec<RigRow {n, mark, tv, rooms, vault}>}>`.
- `readiness::Binding::Vehicles` (ungated).
- `vendor::role_room` (`pub(crate)`).

**Verb:** `+tv` with switches `1`, `2`, `lease` under `LegacyPostImage`. SM.3b moves it to the authoritative class.

**Tests:** `tests/support/vehicles.rs` `strip_vehicles(doc)`, idempotent. SM.5's v11 strip goes beside it.

### Doc 20 amendments owed

- **§13, the SM.3 row.** SM.3 is split by the lead into SM.3a (definitions and assets: goods v3, `UnitClass::Vehicle`, the optional drive, tools, the two packs, the bay, the payload, lane v10, the refit, Occator's lease, the charter-end repossession) and SM.3b (the excursion family, the tick, act starts with charge, launch, recover, dump, the go and selector rules, `TV_OUT`, `HOPPER_LOADED`). The acceptance's "lease refused without a licence" becomes a seam SM.5 fills. **SM.5 row:** "lane v9 … goods v2 (four ore rows, `stow.ore`)" becomes lane **v11** with no goods change. Goods v3 landed in SM.3a.
- **§7.3.** "Goods table version 2 adds four rows" becomes "version 3 (doc 20 cut SM.3a)". The ore notes say "drilled from a Ceres site", which should also cover Rheasilvia (Vesta).
- **§4.1.**
  - Tools are strict typed documents pinned by digest (`tacsim.tools.v1`), not packs, read only by their tests until SM.4/SM.7. A third row, `battery_mk1`, carries the charge figures as data.
  - "read by the surface engine now" is not yet true.
  - The TV is fielded into the catalog (it binds and mints a `DefinitionRef`) but has no burn row. A drive-less unit refuses with `NO_DRIVE`.
- **§4.2.** The hopper vault is laid up by the applier's part (h) in Occator's unanchored shed, not by "the hull recipe" or "`create_vault` as `+coop/enrol` does". Its id comes from the applier's allocator, and it is owned by `System(tacsim.vehicles)` with scope None.
- **§4.3 and D-20-7.**
  - `+coop/refit` re-binds the chartered hull's tactics unit in place (`refit_unit`). It is not a swap "as lane data". The lane carries a `refits` row.
  - The bay room, its space and its `rigs` compartment are laid on **every** programme hull at apply, standard Drays included: hidden, empty, exitless.
  - The egress is definition data only (`EgressDef`); no `EgressSpec` is written until SM.3b.
  - Occator's counter is the Ceres Exchange Hall. The rigs are `OWP-TV-01..08`, in "Occator Vehicle Shed".
  - Refit refusals also cover provisions in the hold, a rig in the bay (or a live lease record) and being away from the desk.
  - The lease mechanics:
    - an economy lease with a 1-minor Manual rent;
    - one public post carrying the term fee and an engine-owned `tacsim.tv.deposit` hold;
    - "re-debited at the next dock" is as built "at the pilot's next `+tv/lease` act" (the at-dock hook is residual R1);
    - a forfeit captures the whole deposit, and the rig waits to be returned;
    - the return is keyed on the lessee's own record, so a Revoked pilot can still hand a rig back.
- **§1 beat 1.** "at Occator's counter" is the Exchange Hall. "+coop/refit … refused under way or with cargo aboard" also refuses with provisions aboard and with a rig in the bay.
- **§11, the Dray-P row.** "2.3 t of headroom" is exactly 2,320 kg (178,680 of 181,000 with two occupants; 178,580 with one).
- **New text owed (§4.3 or §9, plus doc 18 §4.1's recall and buyout paragraphs): a charter's end with rigs aboard.**
  - `+coop/buyout` refuses with `BUYOUT_BAY` and `+coop/release` with `RECALL_BAY`.
  - `@coop/revoke`, the dormancy recall and the return arm repossess every rig first.
  - Deposits follow D16: released within the paid term, captured whole past it, and the engine never debits a fee. Each deposit settles before the lease ends.
  - A non-empty hopper, an unreadable or unlisted record, or a deposit that will not settle holds the hull, and staff are paged with `BAY_PAGE`.
- **Doc 10b §15.5 (or wherever the engine-owned table's rule lives).** D18: `tacsim.tv.deposit` is the first game namespace in mush-economy's frozen `ENGINE_OWNED_RESERVATIONS`, for the reasons the `api.rs` paragraph gives.
- **§18 amendments owed.**
  - Doc 18 §4.1/§4.2: the Dray-P beside the Dray, `+coop/refit`, `RECALL_BAY`/`BUYOUT_BAY`, every hull's bay room.
  - Doc 11 §7: Occator's terrain-vehicle lease, eight rigs `OWP-TV-01..08`, 1,400 a term and 5,000 deposit.

## SM.3b

**The clock** (doc 20 §13's SM.3 row, split by the lead into SM.3a and SM.3b; this is the second
half): the excursion state family `tacsim.excursion` (one `RigRecord` a rig: the lazy charge, the act
in flight with its charge reserved at the start, the open excursion, the last outcome) and the
`tacsim:tv` projection beside the placement, the 1 s Coarse `tacsim.surface` tick that completes every
act as its operator through `Fire::as_principal`, the act framework on one start action
(`tacsim_surface_act`) and one finish action (`tacsim_surface_done`), `+tv/launch`, `+tv/recover`
(the placement half: from the ramp's cell or beside it; the drive home is SM.4b's), `+tv/dump` onto the
site's spoil vault, the `go` rule and the slot selector `/1`..`/4`, and the departure refusals
`TV_OUT`, `TV_BUSY`, `TV_AWAY` and `HOPPER_LOADED` on `+ship/plan`, `+ship/undock`, `+ship/lift` and
the queued depart. No lane, pack or digest moved (`LANE_VERSION` 10 stands; SM.5 is v11). Built
2026-10-20 on main from `96cc7303`. Judged **ship-with-notes**; the lead accepted the six owner-critical
defaults (`RECOVER_S` 300 IC s, `DUMP_S` 0, a past-term Active lease still launches, a charter's end
with a rig out holds and pages, the "control lease" read as the live lessee aboard the settled hull
with no F3 lease on the TV, `tacsim_surface_mine` folded into `tacsim_surface_act` as `act = mine` in
SM.7) and ran a fix round before the commit. One commit, pushed.

### What landed

1. **The excursion record** (`crates/tacsim/src/surface/excursion.rs`, new; re-exported from `surface/mod.rs`):
   - constants:
     - `EXCURSION_FAMILY` "tacsim.excursion", schema 1;
     - `TV_REC_PREFIX` "tacsim:tv:";
     - `LAUNCH_S` 300, `RECOVER_S` 300, `DUMP_S` 0;
     - caps 4 / 32 / 1,024;
     - `MAX_DUMP_GOODS` 8, a 16-line × 160-byte log, a 4,096-byte projection, `MAX_LEG_WAYPOINTS` 32;
     - `PAD_CENTRE_CM` 128,000, `IC_HOUR`;
   - `RigKey(ObjId)`, with strict decimal key text;
   - `RigRecord { schema, tv, ordinal, charge, act?, excursion?, last? }`, strict, with `check`, `fresh` and `key`;
   - `Charge { value_cu, at_ic, in_bay }`, lazy;
   - `ActKind { Launch, Recover, Dump, Probe }`;
   - `ActParams`, tagged `kind`: `None {}`, `Dump { hopper, spoil, goods }`, `Probe { cost_cu }`;
   - `Act`; `ExcursionId { tv, launch_ic }` (Display `tv@launch_ic`); `Excursion`;
   - `Outcome`, carrying `moved` for a dump's line;
   - `ActResult`: `Done` or `Refused { NeverLeft | NotBeside | HopperChanged }`;
   - pure functions:
     - `charge_at`;
     - `reserve_check`: a 5 % floor, exactly the floor fits, a zero spend always passes;
     - `reserved_total`: always 0 at the SM.3b check;
     - `cap_refusal`, checking operator, then site, then world;
     - `ramp_pose`: the pad centre plus the bay egress, so the Dray-P's ramp is (128,000, 124,500, 180,000);
     - `site_cell`, and `cells_apart` (Chebyshev);
   - the bounded projection `TvRec` with `TvState` / `TvPose` / `ActView` / `FrozenLeg` (SM.4's slot, always None) / `TvLogLine`, `push_log` ring and `check`.
2. **The family and the handles** (`crates/tacsim/src/port/families.rs`, `crates/tacsim/src/port_tick.rs`):
   - `tacsim.excursion` is registered unconditionally, and `PortFamilies.excursions` holds it;
   - two outcomes, `tacsim_surface_act_result` and `tacsim_surface_done_result`, and two actions, `tacsim_surface_act` and `tacsim_surface_done`, all under the crate's one validator per kind;
   - the tools table is built once and never raises a `ModError`;
   - `PortHandles` gains `surface_act`, `surface_done` and `surface_tools`;
   - both providers register only when the inventory cell is filled and the table built;
   - `Fire::new` is now `pub(crate)`;
   - `surface::tick::register` is called at the end.
3. **The world plans and the two providers** (`crates/tacsim/src/surface/act.rs`, new):
   - `operator_rigs`, `where_is` (`Bay` / `Ground` / `Elsewhere`) and `site_berth`;
   - `Refusal`: 19 variants, each with a wire word, a prepare message and a line;
   - `launch_plan`, in the fixed order SM.4 prepends: view/held, under way, an unreadable voyage, cleared, not a site, not aboard, Forfeit, ended, not in the bay;
   - `recover_plan`, `dump_plan` and the test-world `probe_plan`;
   - `rig_battery_id` and `battery_of`, so the battery has one source;
   - `world_rigs`;
   - `act_cmd`: the preview, plus `go` as one submission keyed `tacsim.surface/{tv}/{ordinal}/act`;
   - `SurfaceActProvider` and `SurfaceDoneProvider`. The finish records `Done` or `Refused` from the facts at landing, stamps at `due_ic`, settles the charge, and moves a dump's frozen `available` quantities hopper → spoil through `InventoryFamilies::transfer`.
4. **The tick** (`crates/tacsim/src/surface/tick.rs`, new):
   - `tacsim.surface` runs Coarse every 1 s. Each fire scans in pages of 256 up to 1,024 records, takes candidates due or untold oldest-first, visits at most 64 and works at most 16;
   - the tell writes the projection and `notify`;
   - the due step does the idempotent `mush_sim::place` / `place_interior`, then writes the projection, then the finish through `Fire::as_principal`;
   - the attempt rule (`attempt_visit` / `note_step`): 8 attempts, a once-a-boot page, a 60-fire back-off, then a retry;
   - `still_due` over the whole scan;
   - God/Gone is logged once per act;
   - `SurfaceTickState`, `SurfaceFireStats`, `last_surface_fire`.
5. **The projection's store** (`crates/tacsim/src/store.rs`):
   - `tv_recs`, `tv_rec()` and `put_tv_rec`;
   - a `tacsim:tv:` load arm after the `tacsim:tvlease:` arm; an unreadable body is `Unreadable` and bars nothing;
   - a doc-table row.
6. **The verb** (`crates/tacsim/src/commands.rs`, `crates/tacsim/src/port/tv.rs`):
   - `+tv` moves into an `Authoritative` bracket. Its switches are `SLOT_SWITCHES` `1..4`, then `lease`, `launch`, `recover` and `dump`;
   - the grammar is rewritten, and `TV_USAGE` replaces `LEASE_USAGE`;
   - the RAM `RigPointer` slot selector holds at most 4,096 entries;
   - 37 new texts, plus `ACT_FAILED`'s three `WHY_*`;
   - status gains `{where}` / `{charge}` and an act line;
   - a slot past the bay answers `NO_RIG`;
   - the lease return refuses `RIG_BUSY`;
   - `repossess` holds a rig that is out or busy, failing closed;
   - four help lines.
7. **The departure refusals** (`crates/tacsim/src/port/voyage.rs`, `crates/tacsim/src/port/executor.rs`):
   - `TV_OUT`, `TV_BUSY` and `HOPPER_LOADED`, under `DepartRefusalKind::{TvOut, HopperLoaded}`;
   - `bay_gate`, called right after `site_gate` in `prepare_departure`, `plan_block` and `plan_queue`'s dry run;
   - the order fire waits on both kinds.
8. **The spoil** (`crates/tacsim/src/port/map.rs`, `crates/tacsim/src/port/site.rs`):
   - `SiteRow.spoil` and `spoil_vault`, both optional and skipped when absent;
   - `lay_up` lays a DARK "{site} spoil" Thing and a vault owned by `System(tacsim.sites)`, with no scope and `Container`-backed;
   - `preflight` refuses a recorded spoil, or a spoil vault, that is gone or foreign;
   - `spoil_vault()`;
   - the Site line names the spoil, so a missing spoil closes the pad.
9. **The page and the gates:**
   - `crates/tacsim/src/port/recall.rs` `BAY_PAGE` gains the rig-out cause and its remedy;
   - `crates/mush-contract-tests/legacy-handlers.toml`: the `+tv` stanza is now `authoritative`, `family` is dropped, and it carries the SM.3b note;
   - `crates/mush-contract-tests/state-slot-owners.toml`: a `tests/excursions.rs` `AccountOracle` row;
   - `docs/help/ships.md`: a launch / recover / dump paragraph and the lift rule.
10. **Tests:**
    - `crates/tacsim/tests/excursions.rs` (new) holds 17 world tests: the brief's 1–13 and 15–16, plus 17 (the refused finishes) and 18 (the planner gates at the command and the prepare);
    - unit tests in `excursion.rs`, `act.rs`, `tick.rs` (the attempt rule, `still_due`), `store.rs`, `site.rs` and `tv.rs`;
    - pins changed in `tests/vehicles.rs`, `tests/ports.rs` and `tests/sites.rs`.

### Deferred, and why

**Deferred by the brief's §5:**
- **SM.4:** `+tv/scan`, `sound` and `drive`, the frozen leg (`TvRec.leg` stays None), A*, the implied launch (SM.4 prepends `launch_plan`), `+tv/map`, the grid caps.
- **SM.5:** the licence (`licence_seam` stays always-Ok) and lane v11.
- **SM.6:** deposits and claims.
- **SM.7:** mining and the cycle slot. SM.7 must prove `reserved_total` in the world: in SM.3b "two acts cannot both start" holds only through the one act slot and the ordinal.
- **SM.8:** the unload half of recover, `+tv/unload`, `HOPPER_LOADED`'s unload remedy, and `docs/help/mining.md` (it does not exist).
- **SM.9:** screen `q`, the `tv.status` keyframe per act change (doc 20 §10), and the `RUN_ALLOWLIST` entries (left untouched).
- **SM.11:** `+tv/plan`, `await tv`, idle return-and-park, the notice budget.
- **Phase 3:** promotion, `Held`, and an F3 control lease on the TV.

**Lead rulings and builder forks taken:**
- **Two records on two lanes.** The typed family is authoritative; the projection decides nothing.
- **Two actions.** Every act's start and finish share one pair of actions.
- **The site pose.** The pad centre plus the egress offset; "within one cell" is Chebyshev.
- **One act slot per rig.**
- **The finish.** Its facts come from its offer at landing.
- **The spoil.** A Container-backed system vault, laid by part (g).
- **The departure gate.** `bay_gate` sits beside `site_gate`.
- **The caps.** Counted in the start's prepare over `world_rigs`.
- **Lazy charge.**
- **The tick.** It reuses `port_tick::Fire` with an empty owed map each fire. Only a `Submitted` finish that did not land counts toward retry; after 8 the tick pages once a boot and backs off 60 fires.
- **The rig pointer.** It is RAM; after a restart, a two-rig act with no number refuses.
- **The tools table.** One table; a bad table leaves the acts unwired (`TV_UNWIRED`), never a `ModError`.
- **The finish offer.** Only to live-lease pilots, over their own rigs.
- **A launch fails closed** on an unreadable sailing plan.
- **A rig can always come home.** A zero spend is exempt. A recover from beside the ramp needs no open excursion. A recover of a rig in the bay with an excursion open closes it.

**The builder's forks:**
- `Outcome.moved`, so `DUMPED` can name the load.
- `ActParams::None {}`, a struct variant, for strictness.
- The fixed plan wire gains `from_bay` and `goods`.
- A key-mismatch record is paged under the sentinel `(tv, 0)`.
- A probe's tell logs and says nothing.
- `bay_gate` names an unlisted rig `?` and fails closed on an unreadable record or pool.
- `repossess` treats an unreadable record as out.
- `site_line_text` is factored out as a pure function.

**Fixer change:** the dump freezes each pool's `available`, not its `quantity`.

**Owner-critical defaults, standing until the owner rules:**
- **Decision 9:** `RECOVER_S` = 300 IC s. Doc 20 gives no figure for the placement half.
- **Decision 10:** `DUMP_S` = 0.
- **Decision 11:** a past-term Active lease still launches; recover and dump are never lease-gated.
- **Decision 13:** a charter's end with a rig out or mid-act holds and pages the hull; it never strips the rig.
- **Decision 14:** "the operator's control lease" (doc 20 §4.1 line 393, §8 lines 702–704, §19 item 3 line 1172) is read as the live lessee aboard the settled hull. No mush-sim F3 lease is taken on the TV. "Aboard" gates launch, recover and dump alike.
- **Decision 25:**
  - `tacsim_surface_mine` (§6 line 537) folds into `tacsim_surface_act` with `act = mine` in SM.7;
  - §4.3's egress is the tick's `mush_sim::place` at `ramp_pose`, not `egress_parent`;
  - the implied launch is SM.4's.

**Decision 16, corrected (the brief's remedy was false):**
- No lane version moves. A world that latched v10 before SM.3b reads its Site lines Missing and its pads closed.
- Re-applying v10 (`@atlas/pack apply`, or a seed re-apply) plans nothing, because the latch skips unchanged entries.
- The remedy is a store wipe, or a later lane that changes every site entry.
- This is harmless today: dev1 is at 8c648527, before SM.

**Named residuals:**
- The lease return checks only the act and fails open; a staff-move case is described below.
- `bay_gate`'s `TV_OUT` can name a remedy that refuses: a staff move while the hull is docked at a port.
- A zero-cost recover still needs a readable battery id.
- A failed dump transfer wedges the act instead of recording `Refused`.
- Back-off acts can starve the 64-visit window.
- The tell projects `Bay` for a rig on the ground that has no excursion.
- "Within 10 m" vs the cell rule, and "about 5 m" (minutes) — both for the lead.
- The `@coop/revoke` stanza in `legacy-handlers.toml` was not refreshed to mention the new hold.
- `common-sm.md` line 8 is stale (it says SM.2 is lane v8 and SM.5 lane v9); the lead should correct it before SM.4.

### Residual risks the judge named

- The lease return (`plan_return` step 11, `crates/tacsim/src/port/tv.rs` `act_in_flight`) refuses only an act in flight. It does not refuse an open excursion, and it fails open when the record cannot be read; `repossess` and `bay_gate` count an excursion and fail closed. Reaching this takes staff: put a launched rig back in the bay and move the hull to Ceres. The rig then goes back to Occator with `excursion` still open. That stale excursion counts against the old pilot's operator cap and the site cap, and the next lessee meets `TV_OUT` at Ceres with a remedy (`+tv/n recover`) that refuses at a port.
- `bay_gate`'s `TV_OUT` (`crates/tacsim/src/port/voyage.rs`) can name a remedy that refuses. If a staff move leaves a rig elsewhere while the hull is docked at a port, the line names the port and says `+tv/{n} recover`, which answers `LAUNCH_NOT_SITE` there. It is staff-only, but decision 21's "always names a remedy that works" does not hold for it.
- A zero-cost recover or dump still needs a readable battery id (`crates/tacsim/src/surface/act.rs`). Both the offer (every plan `tv_dark`) and the finish's Launch/Recover arms (`battery()?` returns Integrity) depend on it. If tools or pool data change so the battery row stops resolving, a rig on the ground cannot start a recover, an in-flight recover never lands, and the hull stays held by `TV_OUT`/`TV_BUSY`. That is against §4.1's "never stranded by its battery".
- When the dump finish's `inventory.transfer` errors on the spoil side or in the lineage carry, it returns `ProviderRejected` instead of recording a `Refused` outcome. The act then wedges: the rig reads busy and the hull is held. The tick retries every 60 fires and pages once. No such transfer error is reachable today.
- Tick fairness: an act in back-off keeps its old `due_ic`, so it stays at the head of the oldest-due window of 64. If 64 or more finishes cannot land at once, newer acts world-wide starve, and their rigs and hulls stay held. The prune itself is now correct (`still_due` over the whole scan).
- The tell sets the projection's `state` from the excursion: `Bay` when none is open, `Out` otherwise. A rig on the ground with no open excursion is therefore projected `Bay` while `pose` reads the ground (`crates/tacsim/src/surface/tick.rs` `tell`). This is the brief's rule, and the projection decides nothing, but SM.9's screen must read it with this in mind.
- The recover distance text does not match its rule. `RECOVER_FAR`, `WHERE_GROUND` and `docs/help/ships.md` say "within 10 m" and print cells × 10 m, but the rule is Chebyshev ≤ 1 cell from the ramp's cell, so a rig nearly 20 m east is accepted and one 15 m south is told "20 m". The text is the brief's, so this is the lead's call.
- `{real}` and `{when}` print real minutes as "about 5 m" (`voyage::real_span`), and the same verb family uses "m" for metres.
- A world that latched lane v10 before SM.3b reads every `Site` line as Missing and keeps its pads closed (`SITE_CLOSED`). Re-applying v10 by `@atlas/pack apply` or a seed re-apply plans nothing, because the latch skips unchanged entries, so the only remedy is a wipe or a lane version that changes every site entry. dev1 is at 8c648527, before SM, so this is harmless today.
- Thin test coverage:
- Test 12's "a reboot lays nothing twice" leg never calls `lay_up`. The real coverage is SM.2's `sm2_a_site_update_lays_up_nothing_and_a_gone_place_refuses`, whose row and object-count equality now includes the spoil; it does not count vaults.
- Test 5 asserts `TV_BUSY` with a recover in flight, where the brief names a landed launch.
- `TV_UNWIRED`, the God/Gone once-a-boot log and the spoil preflight refusals have no world test.
- Test 16 does not pin the back-off length in the world; the binding unit test does.
- The dump preview reads each pool's balance (`read_pool`), while the start freezes `available`. With a live hold on a hopper pool, the preview would name more than moves. No hopper hold is reachable today.
- A4's cross-world equality (`tests/excursions.rs` test 4) depends on two independently built worlds allocating identical ids. It passes here.

### The lead's fix round (before the commit)

the lease return and the departure gate fail closed on a rig still logged out (one `rig_flight` reader for the return, the repossession and `bay_gate`), a rig comes home and dumps without its battery row, a dump the spoil will not take records `SpoilGone` instead of wedging, acts in back-off never starve the tick's window, the projection reads the ground, the recover text says "about N m … beside it", real minutes print `min` game-wide, the dump preview reads the figure its start freezes, and the thin coverage was filled.

The lead's rulings on the round's review notes, with four hand fixes made after the gate (re-verified with clippy, the cut's suites, the contract tests and the strict rustdoc check; the full sweep ran before them on the round's tree 76f88ed1):

- **F2, ratified.** A refused dump cannot be recorded inside its own failing bundle: `ActionTxn` has no savepoint and a second mutation of one key (`require_fresh_mutation`) poisons the transaction. The refusal is decided before anything is staged — the offer's live-spoil fact, a read-only check of every spoil pool, and a server-authored `failed` list of the act ordinals the tick has already counted a landed-nothing finish for (`tick::finish_retried`, keyed by the act after the review's F1) — so a genuine mid-bundle refusal writes nothing and is recorded `SpoilGone` one fire later. A dump whose first finish failed for any other reason is recorded refused too, with nothing moved, and the pilot dumps again.
- **F3, owed before SM.7 (an inventory finding).** `mush_inventory`'s `add_pool` writes the destination endpoint's census once per new pool, so one bundle that opens two or more new pools on one endpoint always refuses. A hopper holding two ores a spoil has never held can therefore never be dumped onto it (it refuses cleanly now, and `HOPPER_LOADED` then holds the hull). Only staff fill hoppers today; SM.7's multi-ore mining cannot live with it. The fix is inventory-owned (a `transfer_many` writing the census once) or a mush-core way to re-stage a key; test 21(b) then needs another way to make a real mid-bundle refusal.
- **F4, residual (staff-only).** On a site's pad `TV_OUT` still names `+tv/{n} recover` for three staff-made states where recover cannot work (a rig moved `Elsewhere`, or standing on another site's ground, or on this site's ground beyond the ramp with no excursion). Routed to `TV_AWAY` later if it ever matters.
- **F5, fixed by hand.** `TV_AWAY`'s `{where}` with no site to name reads `away from her bay`, not `away` ("logged out away" was not English); test 19 re-pinned.
- **F6, residual: no staff verb closes an excursion.** The remedy `TV_AWAY` and `RIG_LOGGED_OUT` name is two steps today: `@spatial/place/interior` puts the rig back in the bay, and the open `tacsim.excursion` record then needs a finish that lands (a recover from the bay with the excursion open closes it, which the lessee can file once the hull is back on a site's pad). A staff close verb (`@tacsim/tv close <tv>`) is scheduled for SM.12's operator runbook.
- **F7, fixed by hand.** One `projected_state(w, record, hull)` rule for both projection writers (`project_due` and `tell`): `Bay` only with no excursion open and the rig aboard its hull's bay; a recovering rig reads `Out` from the placement step until its finish lands and is told.
- **F8, ruled and the doc softened.** `real_span` is the one spelling of a real wall-clock span; the arena's match durations (`match_::record::duration_words`, `1 h 2 m 3 s`) and the chart's light lag (`chart::lag_text`, `23m 28s light`) are other quantities with their own renders and were not touched.
- **F9, residual.** A launch whose battery row stops resolving between its start and its finish cannot settle (`settle_free` gives nothing when leaving the bay), so it stays busy and holds the hull until the row reads again; a tools table does not change under a running world, so this is a content fault, paged by the attempt rule.
- **F10, fixed by hand.** `read_available` answers `Corrupt` for a book without its scalar, the half pool `InventoryFamilies::pool` refuses, so the dump's preview and its start agree on a corrupt hopper.

1. **1. The lease return fails closed on a rig that is out** — fixed: crates/tacsim/src/port/tv.rs: new pub(crate) `RigFlight { Idle, Busy{kind,due_ic}, Out{site} }` and `rig_flight(ctx, tv) -> Result<RigFlight, String>`. This is now the one reader of `tacsim.excursion`. It re-checks the key; with no PortHandles it reads Idle. `act_in_flight` is deleted. `rig_out_or_busy` (used by repossess) is now `rig_flight` with Err counting as out. `bay_gate` also reads through it. `plan_return` step 11 now answers as follows. Busy gives RIG_BUSY, unchanged. Out gives the new `pub const RIG_LOGGED_OUT` = "Rig {n} is still logged out on {site} and staff must bring it in; nothing changes." Err logs tracing::error with tv, lease and the record, then returns LEASE_FAILED. New world test 19, `sm3b_a_rig_still_logged_out_holds_the_return_and_the_hull_at_a_port` (tests/excursions.rs). It launches and finishes on Ahuna, then staff_bay puts the rig back with its excursion open. The hull is staff-placed on Ceres's domain (berth reads Docked ceres_port), and the pilot runs `+ship/leave` and walks to the counter. `+tv/1 lease return` and `+tv/1 lease return go` each answer exactly RIG_LOGGED_OUT. The lease record, the deposit hold status, the typed record and the rig's bay placement are all unchanged. A record written under the rig's key that names another tv (unreadable) answers LEASE_FAILED and still changes nothing.
2. **2. bay_gate's TV_OUT names a remedy that works where the hull is** — fixed: crates/tacsim/src/port/voyage.rs: new `pub const TV_AWAY` = "Rig {n} is logged out {where}; the {name} lifts only with every rig in her bay: staff must bring it in." `{where}` is "on {site}" (the open excursion's site, else the site whose ground it stands on), or "away". It uses the same `DepartRefusalKind::TvOut`, so +ship/plan, +ship/undock, +ship/lift and the queued depart all refuse the same way. When the hull is Docked at a port whose def.site is Some, TV_OUT keeps its exact old logic and words. Anywhere else, TV_AWAY is used. A missing PortHandles still returns PLAN_DARK, as an explicit check. The doc comment was updated. Test 19 covers the port branch: `+ship/undock ahuna` answers exactly TV_AWAY(on Ahuna Flank), and `+ship/plan ahuna` line 2 shows the same. With the rig staff-placed on Ceres's own ground and no excursion, the line reads `away`. The site-pad branch is still pinned by tests 5 and 15.
3. **3. A rig can always come home without its battery row** — fixed: excursion.rs adds three pure items:
- `needs_battery(kind, cost, home)`: true for Launch, or when cost + home > 0.
- `settle_free(&Charge, at_ic, in_bay) -> Option<Charge>`: the charge stays unchanged when the bay state is unchanged; ground to bay keeps the value with recharge starting at the stamp; bay to ground returns None.
- `RigRecord::fresh_unread(tv)`: value_cu u32::MAX at IC 0 in the bay, which reads full under any battery.
Both helpers are re-exported from surface. Changes in act.rs:
- **Command:** reads the battery only where `needs_battery`. The record falls back to fresh_unread. The reserve check runs only with a battery. The recover preview uses the new `tv::RECOVER_PREVIEW_UNREAD` ("...; its charge cannot be read; hopper {hopper}...").
- **Start offer:** recover and dump plan with or without a battery id; launch and probe get `tv_dark` without one.
- **Start prepare:** refuses "that rig cannot be read" only when `needs_battery`.
- **Finish:** settles through the battery when it resolves; otherwise uses `settle_free` for a zero reserved spend. A launch therefore still fails closed, and a dump never touches charge.
- **tv.rs status:** the ACT_LINE no longer needs the battery; charge still reads `?`.
The launch's cost_cu is 0 in SM.3b, so it fails closed under the "leaves the bay" half of the rule (its bay recharge must be settled), not the spend half. Tests:
- Unit test `excursion::tests::sm3b_a_zero_spend_settles_without_the_battery`.
- World test 20, `sm3b_a_rig_is_never_stranded_by_its_battery`. The test takes every battery out of PortHandles.surface_tools. Status reads `charge ? %`. The recover previews RECOVER_PREVIEW_UNREAD, starts, lands and finishes Done; the charge is {launched value, due, in_bay}. A 2 t dump starts and finishes Done, with the spoil +2. A launch answers TV_DARK at the command, and a built start refuses "that rig cannot be read" at the prepare with nothing written.
4. **4. A failed dump transfer records a refusal, never wedges** — fixed: New `RefusedCode::SpoilGone` (serde `spoil_gone`) and `tv::WHY_SPOIL_GONE` = "the spoil would not take the load; it stays in the hopper and nothing moved". The tick's completion_line handles it. DEVIATION, forced by the engine: `ActionTxn` has no savepoint, and it refuses a second mutation of one key (DuplicateMutation poisons the txn). So a transfer that errors mid-bundle cannot be turned into a Refused record inside its own bundle: the staged debit would commit, or the poisoned txn would refuse everything. The refusal is therefore decided before anything is staged, in three ways:
- (a) The finish offer's `DoneRig` gains `spoil: Option<u64>`, the live `spoil_vault` of the site the hull is settled on. A frozen spoil that differs records SpoilGone.
- (b) Every spoil pool must read (a read-only `inventory.pool`).
- (c) `DoneRig.failed`, from the new `tick::finish_retried`: the attempt rule has counted at least one finish of this rig's act that did not land. A dump whose transfer refused mid-bundle therefore records SpoilGone on the tick's next submission, one fire later.
The refusal is stamped at `due_ic`, releases the act, spends no charge, and pages once with tracing::error naming tv, act, the spoil vault, the site and why. A mid-bundle transfer error is logged as a warn and still refuses the whole bundle, which writes nothing.
No lane bump; this only adds the serde variant.
World test 21, `sm3b_a_dump_the_spoil_will_not_take_is_refused_and_never_wedges`:
- (a) The spoil Thing is flagged GOING between start and finish. The finish lands Refused{SpoilGone} at due with nothing moved. ACT_FAILED/WHY_SPOIL_GONE is heard once. `+ship/lift ceres` reads HOPPER_LOADED, not TV_BUSY. After the spoil is mended, the next dump runs Done.
- (b) A REAL spoil-side transfer error: metal_ore plus ice_ore onto a spoil that has neither pool. The first finish does not land (verified `expect_err`) and writes nothing: the hopper is (2,1), the spoil (0,0), and the act is still in flight. The next fire's finish lands SpoilGone with the hopper exact. The hull then reads HOPPER_LOADED, not TV_BUSY.
NEW FINDING for the lead: (b) shows that mush-inventory's `add_pool` writes the endpoint census once per new pool. A dump of two or more goods the spoil has never held can therefore NEVER land. It now refuses cleanly instead of wedging, but such a hopper can never be dumped onto a fresh spoil, and HOPPER_LOADED then holds the hull for good. Today only staff fill hoppers; SM.7 (multi-ore mining) cannot live with it. Suggested fix: an inventory-owned `transfer_many` that writes the destination census once.
5. **5. The tick's window skips acts in back-off** — fixed: In tick.rs, candidate selection moves into a new pure function, `pub(crate) fn candidates(records, now, told, &state) -> (Vec<Candidate>, u32)`, with a private `backing_off`. An act whose back-off has not expired has its due-ness dropped before the sort and the 64-cap. It stays a candidate only if its outcome is untold, and then only for the tell. The order is still the total `(instant, tv)`. Acts left out are counted into `stats.passed_over`, so test 16's world assertion holds unchanged. Unit test `tick::tests::sm3b_acts_in_their_back_off_never_starve_the_window`:
- 64 backed-off acts plus one fresh due act: only the fresh act is picked, and passed_over is 64.
- One fire before the back-off ends: the same.
- At DONE_BACKOFF_FIRES: the 64 come back oldest first, the window is full and the fresh act waits.
- A backed-off act with an untold outcome stays for the tell, not as due.
6. **6. The projection reads the ground** — fixed: In tick.rs `tell`, `state` is now Bay only when no excursion is open and `where_is` reads Bay{hull} for the tell's hull (any bay when the hull is unknown). Otherwise it is Out. The projection still decides nothing. Test 17(ii), `sm3b_a_finish_the_world_refuses_is_told_and_keeps_what_it_should`, gained assertions: after the NeverLeft refusal the rig stands on the ground 3 cells off with no excursion, and the projection state is `TvState::Out`.
7. **7. The recover distance text matches its rule** — fixed: The rule is unchanged (Chebyshev ≤ 1 cell, cells × 10 m). New words:
- `RECOVER_FAR` = "Rig {n} is about {m} m from the {ramp}; it climbs aboard from beside it."
- `WHERE_GROUND` = "on the ground on {site}, about {m} m from the {ramp}".
- docs/help/ships.md now reads "brings it back up when it stands beside the ramp, on the ramp's own cell or the next (300 IC s); a rig further off is told about how far it is."
Re-pinned: `act::tests::sm3b_every_refusal_word_has_a_message`, OLD "Rig 1 is 20 m from the stern ramp; it climbs aboard from within 10 m of it." → NEW "Rig 1 is about 20 m from the stern ramp; it climbs aboard from beside it." excursions test 3 builds from the const, so it needed no edit. No test pinned WHERE_GROUND.
8. **8. Minutes are `min`, game-wide** — fixed: `voyage::real_span` now prints `{h} h {m} min`, `{m} min` or `under a minute`. No second formatter was added and no distance spelling was touched. Re-pinned (OLD → NEW):
- voyage.rs `ic9_time_renders`: "4 m" → "4 min"; "1 h 2 m" → "1 h 2 min".
- when.rs unit tests: "20:00 UTC today (IC clock stopped; in 1 h 0 m)" → "... in 1 h 0 min)"; `ends_with("; in 30 m)")` → "; in 30 min)".
- tests/orders.rs: ", until 1 h 0 m after." → "1 h 0 min after."; "; in 2 h 0 m)" → "; in 2 h 0 min)"; two "...UTC (in 2 h 0 m)." lines → "(in 2 h 0 min)."; "; departs {} UTC (in 2 h 0 m)." → "(in 2 h 0 min).".
- tests/sites.rs: " UTC (in 2 h 0 m)." → " UTC (in 2 h 0 min).".
- tests/voyage.rs: "Passage 18.5 IC days ≈ 7 m real; arrives about " → "≈ 7 min real; ...".
- tests/excursions.rs: ", about 5 m" → ", about 5 min" in tests 1 and 3.
Re-blessed `web/src/lib/fixtures/chart/sky_chart.json` (37 lines, every "N h M m" now "N h M min") with `TACSIM_BLESS=1 cargo test -p tacsim --test chart`. Reason: the chart lines carry real_span. No docs/help example used real_span's spelling. The match-record and TUI formatters (`4 m 32 s`, `2 m 14 s` in docs/help/simulator.md, mush-bot and mush-net-ssh) are separate formatters and were left alone, as instructed.
9. **9. The dump preview names what moves** — fixed: New `pub fn port_tick::read_available(ctx, endpoint, commodity)`. It is `read_pool`'s lens and rule over the pool scalar and its book: `book.checked_available(scalar)`; absent is 0; a half pool or bad arithmetic is Corrupt. `act::hopper_contents`, the preview and its DUMP_EMPTY check, now reads it, so the preview names exactly the `available` the start freezes. World test 22, `sm3b_the_dump_preview_names_what_the_start_freezes`: 3 t in the hopper, with 1 t held by a live inventory hold placed through `InventoryFamilies::reserve`. The preview names 2 t, the start freezes {metal_ore: 2}, the finish moves 2, and the held tonne stays.
10. **10. Thin coverage, filled** — fixed: (a) Test 12, `sm3b_a_site_lays_up_its_spoil_and_no_player_may_take_from_it`, now censuses all six sites: objects, all vaults, the six spoil vaults owned by System(tacsim.sites), and every site map row. The census runs before and after the reboot. It then runs the REAL lay-up pass again: `EntryApplier::update` of `tacsim::port::apply::PortApplier` over each site entry of the shipped lane (LANE_JSON, with the latch's `placed`), through support::with_ctx. Each update must be Ok and the census unchanged.
(b) Test 5 now asserts TV_BUSY ("going down the stern ramp") with a landed LAUNCH in flight, both in `+ship/lift ceres` (exact, nothing queued, nothing pending) and in `+ship/plan ceres` line 2. The recover case is kept.
(c) New test 23, `sm3b_every_act_answers_tv_unwired_without_the_tools_table`. The tools table is compiled-in content a test cannot break, so the test gives the world exactly the PortHandles such a roster registers: surface_tools, surface_act and surface_done all None. The six act lines (launch, recover and dump, each bare and with go) each answer TV_UNWIRED whole, queue nothing, write no record and no tacsim: document; `+tv` still reads. No test was renamed. teleport's assertion message now includes the command reply.
11. **11. The @coop/revoke stanza, refreshed** — fixed: crates/mush-contract-tests/legacy-handlers.toml: one sentence was appended to the @coop/revoke note: "Doc 20 cut SM.3b: the repossess also holds the hull (and pages) when a rig in the bay has an act in flight or an open tacsim.excursion, or its tacsim.excursion record will not read -- it never strips the rig (decision 13)." No other stanza or baseline count was touched; mush-contract-tests is green (29 binaries, 130 passed).

**The round's review.** I reviewed the delta c9887f27 -> 4562b6b5 (15 files, +1466/-277; computed with a scratch index, and the real index was never touched). All 11 items are done and done correctly. Each has a real test, and no assertion was weakened. The only pins that moved are the chart fixture (verified: all 37 changed lines differ only in the minute spelling) and the @coop/revoke note (one sentence; no other stanza or baseline count changed). Nothing new leaks truth or uses RNG, and the new texts are in the established voice. One should-fix and several notes follow.

Per item:
1. DONE. `rig_flight` (tv.rs) is now the one reader of `tacsim.excursion`. `plan_return` step 11 answers RIG_BUSY, the new RIG_LOGGED_OUT, or LEASE_FAILED with a tracing::error page. `rig_out_or_busy` and `bay_gate` read through the same function. Test 19 proves both the preview and `go` refuse, and that the lease record, deposit hold, typed record and bay placement are unchanged. It also proves that a mis-keyed record fails closed.
2. DONE. TV_AWAY is under the same TvOut kind. The site-pad branch is byte-identical to before (I checked the fallbacks). Test 19 covers the port branch, both the site case and the `away` case.
3. DONE. `needs_battery` and `settle_free`, plus world test 20, show that a recover and a dump start and land with the battery row gone, and that a launch refuses at both the command and the prepare. The launch is held by `needs_battery` (a launch always reads the battery), not by its cost: the lead's premise was wrong, the launch's cost is 0 in SM.3b.
4. DONE, WITH AN ENGINE-FORCED DEVIATION THE FIXER REPORTED. I verified the cause: `ActionTxn::require_fresh_mutation` at mush-core action_state.rs:2085-2089, and `add_pool`'s census write at mush-inventory pool.rs:416-429. The refusal is therefore decided before staging, or one fire later through a RAM `failed` flag. Test 21(a) and 21(b) are real: 21(b) checks that a genuine mid-bundle refusal writes nothing, that the next finish lands SpoilGone at `due_ic`, and that the hull then reads HOPPER_LOADED, not TV_BUSY.
5. DONE. The pure `candidates()` drops acts in back-off before the 64-cap, and its unit test covers the fresh act, the fire before expiry, the expiry itself and an untold act.
6. DONE in `tell`. Test 17 asserts `Out` for a rig on the ground with no excursion. One note below: `project_due` still uses another rule.
7. DONE. The rule is unchanged. RECOVER_FAR, WHERE_GROUND and ships.md now say "beside". There was one re-pin, and test 3 builds its line from the const.
8. DONE. `real_span` prints `min`, and every re-pin is listed. My grep of crates/*/src, crates/*/tests, web/src and docs/help finds no remaining old spelling built from `real_span`. The workspace sweep (mushd, mush-bot) was not run by the fixer and is the gate runner's.
9. DONE. `read_available` uses the same `checked_available` as `InventoryFamilies::pool`. Test 22 places a real inventory hold and shows preview, freeze and move all name 2 t.
10. DONE. (a) The real `PortApplier::update`, which calls `put`, runs again over all six site entries, with a census of objects, vaults, the six spoil vaults and the map rows. (b) TV_BUSY is now asserted with a landed launch in flight. (c) Test 23 covers TV_UNWIRED.
11. DONE. One sentence was added to the @coop/revoke note and nothing else changed.

Recommendation: fix F1 (small) before commit, and have the lead rule on F2 (the item-4 deviation) and F3 (the census defect, owed before SM.7).

- **F1** (should-fix): tick.rs:129-136 `finish_retried(w, tv)` returns true if ANY `attempts` key has `key_tv == tv.0 && ordinal != 0`. It ignores the act's ordinal, so it does not do what its doc says ("a finish of `tv`'s act"). act.rs:1952 fixes it per rig in the finish offer, and act.rs:2130 then records `SpoilGone` for any dump whose rig has a counted attempt. Attempts are pruned only when the scan is complete (tick.rs:452-455; a capped scan over MAX_RIG_RECORDS "prunes nothing"). In that case a stale `(tv, n-1)` attempt, for example from a launch whose finish needed two tries, makes every later dump on that rig refuse SpoilGone on its first finish, with an error page each time, for the rest of the boot. The `ordinal != 0` filter is a no-op for `attempts`: ordinal-0 keys only ever go into `paged`. There is also a forward hazard: SM.7's cycle slot gives one rig two live act keys, and this check would then fire across them.
- **F2** (note): act.rs:2121-2158. As reported, the item-4 design departs from the brief's "inside the same ActionTxn". I confirmed the cause in the engine: mush-core action_state.rs:2085-2089 refuses a second mutation of one key (DuplicateMutation), and mush-inventory pool.rs:60-61 documents the same rule. The `failed` branch fires after ANY first finish of a dump that did not land, not only a transfer refusal. port_tick.rs:1093-1098 documents one such case: the operator's own `@wait` line later in the same slice voids the finish as a fan-out. Another is a store error. In those cases the pilot hears "the spoil would not take the load" and staff get a tracing::error saying the spoil is at fault when it is fine. The outcome is still safe: nothing moves and a second dump works.
- **F3** (note): excursions.rs:3016 (test 21(b)) makes its mid-bundle error by seeding `ice_ore` next to `metal_ore` on a spoil that has neither pool. That relies on the inventory defect the fixer found: `add_pool` writes the destination census once per new pool (pool.rs:416-429). As a result, any hopper holding two or more goods new to a site's spoil can never be dumped. The finish refuses SpoilGone every time, and HOPPER_LOADED (voyage.rs:413), whose remedy is `+tv/{n} dump`, then holds the Dray-P for good, which breaks decision 21. Today this is reachable only through staff-filled hoppers. It cannot stay once SM.7 brings multi-ore mining.
- **F4** (note): voyage.rs:2539-2549. The on-pad branch keeps the old TV_OUT, as the lead ordered, so it still names `+tv/{n} recover` in three staff-made cases where recover cannot work: a rig staff moved `Where::Elsewhere` (the line names the pad, and recover answers RECOVER_ELSEWHERE); a rig on another site's ground (also RECOVER_ELSEWHERE); and a rig on this site's ground more than one cell off (RECOVER_FAR, with no drive in SM.3b). Decision 21's "names a remedy that works" therefore still fails on the pad for these.
- **F5** (note): voyage.rs:408 and 2554. When no site can be named, TV_AWAY renders "Rig 1 is logged out away; the Salt Moth lifts only with every rig in her bay: staff must bring it in." This follows the lead's literal wording ('away' when the site cannot be named), but "logged out away" does not read as English. Test 19 pins it.
- **F6** (note): tv.rs:248 RIG_LOGGED_OUT and voyage.rs:408 TV_AWAY name staff as the remedy. No staff verb writes the `tacsim.excursion` family: only tv.rs, port_tick.rs, surface/act.rs and surface/tick.rs touch `families.excursions`. Staff can re-place the rig with `@spatial/place/interior`, but that does not close an open excursion. The only working path is for staff to `@spatial/place/domain` the hull back onto the site's pad so the pilot can `+tv/{n} recover`, which closes it.
- **F7** (note): tick.rs:705-712 (`tell`) now projects `Bay` only when no excursion is open and the rig is aboard its own hull's bay. tick.rs:618-621 (`project_due`) still projects `Bay` for any `Where::Bay`, ignoring both the excursion and the hull. In the staff-made 'in the bay with an excursion open' state, `tacsim:tv`'s `state` therefore flips Bay -> Out across one fire, depending on which writer ran last.
- **F8** (note): voyage.rs:434-436: the `real_span` doc says it is "the game's one spelling of real time". Other formatters still spell minutes `m`: `match_::record::duration_words` (record.rs:1636-1638, "1 h 2 m 3 s"), mush-net-ssh tui/render.rs:3768, and chart.rs:117-118 `lag_text` ("23m 28s light"). `lag_text` prints in the same `+chart` output as the passages that now read "1 h 0 min". Doc 18's player-text table (18-trade-run-vertical-slice.md rows 9 and 15) still quotes "≈ 19 h 38 m real" and "(in 31 m)". The fixer stayed within the item's instructions, so this is a scope question.
- **F9** (note): act.rs:2047-2063 and 2070-2072. If a launch's battery row stops resolving between its start and its finish, the beside branch's `settle` returns Integrity on every attempt (`settle_free` gives None when leaving the bay). The rig is then placed on the ground by `world_step`, stays busy, and its hull is held by TV_BUSY behind the 8-try page and back-off. The lead's 'launch keeps failing closed' covers this, but it is the one remaining place a battery row can strand a rig.
- **F10** (note): port_tick.rs:1291: `read_available` reads a missing scalar with a book present as 0. `InventoryFamilies::pool`'s `load_pair` (mush-inventory pool.rs:264-271) refuses that same half pool with HalfPool. So in that corrupt state the dump preview silently drops the good, while the start's freeze answers Integrity "the hopper cannot be read". This is reachable only with corrupt data.

**Re-fixed after the review:**

- **F1** — fixed: The retried-finish flag now belongs to the act, not the rig. In crates/tacsim/src/surface/tick.rs, `finish_retried(w, tv)` now returns `Vec<u32>`: the ordinals of the act attempts counted for that rig, ascending. It is built by a pure helper, `retried_ordinals(state, tv)`, which ranges over `attempts` from `(tv,1)` to `(tv,u32::MAX)`, so ordinal 0 is still excluded. The list keeps at most the highest `MAX_RETRIED_ORDINALS` (8, `pub(crate)`) ordinals; a rig's ordinals only grow, so the live act is always kept. In crates/tacsim/src/surface/act.rs, `DoneRig.failed` is now `Vec<u32>` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]` and `deny_unknown_fields` kept. The finish's prepare refuses an offer whose `failed` has more than `MAX_RETRIED_ORDINALS` entries, and the dump branch tests `rig.failed.contains(&act.ordinal)`. A stale `(tv, n-1)` attempt left by a capped scan therefore no longer marks act n, and a second live act on one rig (SM.7's cycle slot) never marks the other. New unit test `surface::tick::tests::sm3b_a_retried_finish_is_keyed_by_the_act_not_the_rig` checks: a stale `(tv,1)` attempt with a fresh dump at ordinal 2 reads not-failed for 2; an attempt at `(tv,2)` reads failed; another rig's attempts and an ordinal-0 key in `paged` never count; and the list is capped at the 8 highest ordinals. The excursions integration test of the mid-transfer refused-then-SpoilGone path (excursions.rs around line 2933) still passes.

**Pins the round moved.** Two pins moved; everything else is verified unmoved.
- **The chart fixture:** `web/src/lib/fixtures/chart/sky_chart.json` was re-blessed with `TACSIM_BLESS=1 cargo test -p tacsim --test chart`. Item 8 changes `real_span`'s minute spelling, and the chart's passage, reach and window lines carry it. 37 lines changed. The chart test then passes un-blessed, and mush-net-ssh `--test chart_screen` passes. The web vitest suite (`web/src/lib/chart.test.ts`) was not run here; it has no minute-spelling assertion.
- **`legacy-handlers.toml`:** the `@coop/revoke` note text changed (item 11). The baseline count did not move.
- **Unmoved:** the lane digest and signatures (LANE_VERSION stays 10, no content touched), the sites table, the geology pins, the goods/TV/Dray-P digests, `state-slot-owners.toml` (the new `module_state_mut::<PortHandles>` reaches in excursions.rs are same-crate, so the scan passes), the panic budget (no new production unwrap/expect/panic), and the module-ns owners.

### Pins re-blessed

- **`crates/mush-contract-tests/legacy-handlers.toml`, the `+tv` stanza.**
  - Category `legacy-post-image` → `authoritative`, and `family = "tacsim.vehicles"` is dropped (`+ship`'s authoritative stanza carries none).
  - The new SM.3b note goes before "Earlier:", with SM.3a's text kept after it.
  - Reason: `+tv/<n> launch|recover|dump go` stages exactly one `tacsim_surface_act`.
  - `PINNED_BASELINE` does not move: `/1../4` and the act switches are switches of the existing verb.
- **`crates/mush-contract-tests/state-slot-owners.toml`.** One new `[[site]]` row for `crates/tacsim/tests/excursions.rs` (`World::module_state_mut`, `AccountOracle`, owner `sim`), in the `vehicles.rs` row's shape. Reason: the new file installs the fixed account map for `+coop/enrol`.
- **The four rig lines in `crates/tacsim/tests/vehicles.rs`.** `RIG_LINE`, `RIG_LEASE_ENDED`, `RIG_FORFEIT` and `RIG_PAST_TERM` each gain `.replace("{where}", WHERE_BAY).replace("{charge}", "100")`. Reason: the constants now carry where the rig is and its charge (lead ruling 9).
- **`sm3a_the_tv_grammar_answers_lease_usage`.**
  - `LEASE_USAGE` → `TV_USAGE` (no second constant is kept).
  - Nine SM.3b cases added, and no existing case changed.
- **`sm3a_tv_three_is_an_unknown_switch` → `sm3a_tv_slots_past_the_bay_answer_no_rig`.**
  - It now runs on a stocked Dray-P: `+tv/3` answers `NO_RIG` (Salt Moth), and `+tv/5` is the unknown switch.
  - Reason: the slot switches are generalised to `MAX_BAY_ITEMS`.
- **`sm3a_a_filed_passage_burns_with_its_rigs`, rewritten.** `HOPPER_LOADED` now refuses a loaded filing.
  - The loaded `+ship/undock` answers exactly `HOPPER_LOADED{1,2,Salt Moth}` and files nothing.
  - Staff then empty the hopper with a new `take_pool` helper, and both hulls file at equal payloads.
  - 2 t put back before the burn make the pilot's burn refuse "the hold changed since the passage was planned". That still proves the burn prepare re-reads and weighs the hoppers.
- **The Site readiness line.** Reason: the spoil is part of a ready site (decision 6).
  - `crates/tacsim/tests/ports.rs` `ic3_the_readiness_report…` now reads "16 block places, the site clerk and the spoil laid up. [ready]".
  - `crates/tacsim/tests/sites.rs`: three pins (Ready, "15 of 16 … the site clerk and the spoil", "16 of 16 … no site clerk and the spoil").
- **Unmoved, and verified unmoved:**
  - `LANE_VERSION` 10, `LANE_DIGEST`, `GOODS_DIGEST`;
  - `TV_DIGEST`, `DRAY_P_DIGEST`, the tool digests and the site-pack pins;
  - `module-ns-owners.toml`, the composition-edge pin, the panic budget (no new production panics), `provider_rng_policy` and `truth_serialization_policy`;
  - `RUN_ALLOWLIST`, the TUI screen counts, and `boot_parity`'s counts (no test pins the family, outcome or action counts).

### How it was verified

**Builder, three reviewers, fixer, final gate**

The cut went through a builder, three reviewers (correctness, conformance, tests), a fixer and a final gate.

Reviewers raised one must-fix:
- **tests/T1:** test 13 never reached `repossess`.

They also raised five should-fix findings:
- **tests/T2–T6**: the depth of the world tests.
- **conformance/F1:** the God/Gone log fired on every fire.
- **conformance/F2:** the prune ran after the visit cut.
- **correctness/F1:** decision 16's re-apply remedy was wrong.

The fixer closed all of them. tests/T4's freeze asymmetry was a real code fix: the dump now freezes `available`. correctness/F1 was corrected in the note, not in code (see Deferred).

The fixer mutation-checked two fixes, then restored the code:
- With `repossess`'s guard disabled, test 13 fails.
- With the zero-spend exemption removed, test 8 fails with `RESERVE_LOW`.

**Final gate, round 1, all green on the first run**

The gate ran at HEAD 96cc7303. `git status` was the same 21 paths before and after the sweep, with no peer edits.
- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets -j 8 -- -D warnings`: Finished, exit 0, no warnings.
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8`: exit 0.
- `cargo test -j 8 -p tacsim --lib surface`: test result: ok. 16 passed; 0 failed; 1277 filtered out.
- `cargo test -j 8 -p tacsim --test excursions`: test result: ok. 17 passed; 0 failed; finished in 7.97s.
- `--test vehicles --test sites --test ports --test voyage --test orders --test landing`: vehicles 41, sites 24, ports 21, voyage 25, orders 33 and landing 7 passed, all ok.
- `cargo test -j 8 -p tacsim --no-fail-fast`: 55 binaries all ok, 2,841 passed, 0 failed, 13 ignored (lib 1,292 passed, 1 ignored).
- `cargo test -j 8 -p mush-contract-tests`: 29 result lines all ok, 130 passed. This covers:
  - legacy_handler_allowlist 4;
  - state_slot_owners 5;
  - help_commands 4;
  - scale_harness 6 (8 ignored).
- `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop`: 21 passed and 6 passed.
- `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8`: 413 result lines all ok, 8,229 passed, 0 failed, 93 ignored.
- `cargo test -p mush-bot -j 8`: 717 passed, 0 failed, 7 ignored. This includes arena_e2e (230 s), arena_mind_isolation and playtesting.
- No load flakes, no LNK1104, and no peer-attributed failures.

**The judge's lenses**

I read the whole diff and all four new files myself. I checked:
- **Each fix against its finding:** tick `still_due` and the paged God/Gone log; act.rs's `available` freeze; tests 1, 4, 6, 8, 9 and 13; new tests 17 and 18; vehicles.rs's grammar additions.
- **No existing assertion weakened** beyond the changes the brief forces.
- **The surface matches the code.** Three small mismatches: `MAX_POINTERS` is `pub`, not `pub(crate)`; the prepare messages "that rig is not yours to work" and "the hopper could not be emptied onto the spoil" are unlisted.
- **No RNG:** no `rand` in the new code, and no `w.now()` in either provider.
- **No applier submission:** `lay_up` issues only `dark_thing`, `create_vault` and `put_port_map`.
- **No truth leak:** `+tv` reads only the actor's own leases and helm hull. Fixed parameters carry the world list and the facts. Choice values name only the actor's rigs.
- **No new production `unwrap`, `expect` or `panic`.**

**Gate notes.** Cut SM.3b passed gate round 1: all 12 commands green on the first run. No reruns were needed, and there were no load flakes, hangs, LNK1104 errors or failures blamed on the peer session.

- **Base and tree:** the base is 96cc7303. The working tree held 21 changed paths (17 modified, 4 untracked: surface/act.rs, surface/excursion.rs, surface/tick.rs, tests/excursions.rs), with a diff stat of 17 files, +1363/-112. Every changed file is named in the cut's brief, so all of it belongs to SM.3b and none to the astronomy session.
- **No peer edits during the run:** I re-checked git status before the workspace sweep and again at the end. It was unchanged both times: same 21 paths, HEAD still 96cc7303.
- **Clippy and rustdoc were cached:** steps 2 and 3 each finished in under a second. Cargo's fingerprints were already current, most likely because the builder ran both after the last edit. With -D warnings, a cached clean pass is still a valid pass. Both logs have 0 warning or error lines, and the strict RUSTDOCFLAGS doc run (CI's Workspace documentation job, verbatim) exited 0.
- **Totals:**
  - tacsim, all 55 binaries: 2841 passed, 0 failed, 13 ignored. The lib alone is 1292 passed, 1 ignored.
  - Workspace excluding mush-bot, 413 binaries: 8229 passed, 0 failed, 93 ignored. The build took 4m54s and the whole sweep about 35 minutes.
  - mush-bot, 25 binaries: 717 passed, 0 failed, 7 ignored. That includes arena_e2e (230s), arena_mind_isolation (131s) and playtesting (124s).
- **Load-sensitive suites:** none of the listed ones failed in the sweep.
- **No changes to the repo:** I did not edit, stage, stash or commit anything. The only file I wrote is the helper script run.sh, in the gate1 log directory.

All logs are in C:/Users/faeni/AppData/Local/Temp/claude/C--Users-faeni-cc-rustmush/0afc2ac2-d687-46e8-b233-de9758bdcbf4/scratchpad/cuts/SM.3b/gate1/ (01-fmt.log through 12-mush-bot.log).

**The lead round's gate, all first run unless noted:**

- pass: `cargo fmt --all -- --check` — exit=0, no output (log 01-fmt.log)
- pass: `cargo clippy --workspace --all-targets -j 8 -- -D warnings` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.71s; exit=0; zero warning/error lines (fresh from cache, replayed diagnostics empty) (log 02-clippy.log)
- pass: `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items -j 8` — Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.67s; Generated C:\Users\faeni\cc\rustmush\target\doc\mush_ansi\index.html and 45 other files; exit=0; zero warning/error lines (log 03-doc.log)
- pass: `cargo test -j 8 -p tacsim --lib surface` — test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 1277 filtered out; finished in 0.15s (log 04-surface.log)
- pass: `cargo test -j 8 -p tacsim --test excursions` — test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.46s (log 05-excursions.log)
- pass: `cargo test -j 8 -p tacsim --test vehicles --test sites --test ports --test voyage --test orders --test landing` — landing: test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 36.43s | orders: test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.56s | ports: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 19.24s | sites: test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.79s | vehicles: test result: ok. 41 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.00s | voyage: test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.95s (log 06-tacsim-integ.log)
- pass: `cargo test -j 8 -p tacsim --no-fail-fast` — 55 test-result lines, all ok; lib: test result: ok. 1295 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 84.06s; last (doc-tests): test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s; summed: 2849 passed, 0 failed, 13 ignored; exit=0 (log 07-tacsim-all.log)
- pass: `cargo test -j 8 -p mush-server --test boot_parity --test ic_slice_loop` — boot_parity: test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.30s | ic_slice_loop: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.30s (log 08-mush-server.log)
- pass: `cargo test -p mush-contract-tests -j 8` — 29 binaries, all ok; e.g. state_slot_owners: test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s; scale_harness: test result: ok. 6 passed; 0 failed; 8 ignored; ...; summed: 130 passed, 0 failed, 8 ignored; exit=0 (log 09-contract.log)
- pass: `cargo test --workspace --exclude mush-bot --no-fail-fast -j 8` — 413 test-result lines, every one 'test result: ok.'; summed: 8237 passed, 0 failed, 93 ignored; final line: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s; exit=0. The 'panicked at crates\mush-server\src\process_fault.rs:197:9' line in the log is an intended panic inside a passing process-fault test. (log 10-workspace.log)
- pass: `cargo test -p mush-bot -j 8` — 25 test-result lines, all ok; summed: 717 passed, 0 failed, 7 ignored; exit=0 (adversarial_reduction_final_rerun_and_blind_packet ran over 60 s and passed) (log 11-mush-bot.log)

All 11 gates passed on the first run. None needed a rerun, there were no load flakes and no LNK1104. Logs are in C:/Users/faeni/AppData/Local/Temp/claude/C--Users-faeni-cc-rustmush/0afc2ac2-d687-46e8-b233-de9758bdcbf4/scratchpad/cuts/SM.3b/leadgate1/ (01-fmt.log through 11-mush-bot.log, plus the run.sh helper).

Tree under test: HEAD 96cc7303 with the working tree at scratch-index tree 76f88ed1e3c53355d4b50726ff75c7143c5e5fc9. I computed it the same way before and after the run and got the same id both times, so the checkout did not change during the gates. git status had 25 entries throughout: the cut's 21 M and 4 ?? files.

This tree is not the judge's tree c9887f27. git diff c9887f27 76f88ed1 shows 15 files, +1523/-277: legacy-handlers.toml, port/tv.rs, port/voyage.rs, port/when.rs, port_tick.rs, surface/act.rs, excursion.rs, surface/mod.rs, surface/tick.rs, tests/excursions.rs, orders.rs, sites.rs, voyage.rs, docs/help/ships.md and web fixture sky_chart.json. I take these to be the lead-round edits. All of them are inside the cut's files. I found no modified file outside this cut, so nothing is peer-attributed.

The sky_chart.json change re-spells the duration text from "1 h 0 m" to "1 h 0 min". That fits a when.rs/real_span change aimed at the judge's residual about "m" also meaning metres.

clippy and cargo doc both ran from fresh cache (0.71 s and 0.67 s) with no diagnostics replayed. Every test binary was already built except mush-server's, which recompiled tacsim and mush-server under that package's feature set.

Totals:
- tacsim full suite: 2849 passed, 13 ignored.
- Workspace without mush-bot: 8237 passed, 93 ignored, 413 binaries.
- mush-bot: 717 passed, 7 ignored.
- Contract tests: 130 passed, 8 ignored.

### Surface for the next cuts

**Record and arithmetic** — `tacsim::surface::excursion`, all re-exported from `tacsim::surface`:
- `RigKey(pub ObjId)`: `key_text()` and a strict `parse`.
- `RigRecord { schema, tv, ordinal: u32, charge: Charge, act: Option<Act>, excursion: Option<Excursion>, last: Option<Outcome> }`: `fresh(tv, &BatteryDef)`, `key()`, `check()`.
- `Charge { value_cu, at_ic, in_bay }`.
- `ActKind { Launch, Recover, Dump, Probe }`: `word()` and `from_word()`. SM.7 adds Mine as `act = mine` on the same action.
- `Act { ordinal, kind, operator, hull, slot, site, started_ic, due_ic, reserved_cu, params }`.
- `ActParams`, tagged `kind`: `None {}`, `Dump { hopper, spoil, goods }`, `Probe { cost_cu }`.
- `ExcursionId { tv, launch_ic }` and `Excursion { id, operator, hull, slot, site }`.
- `Outcome { ordinal, kind, operator, slot, site, at_ic, result, moved }`.
- `ActResult::{Done, Refused { code: RefusedCode::{NeverLeft, NotBeside, HopperChanged} }}`.
- `reserved_total(&RigRecord) -> u32`: **SM.7 extends this one function** with the cycle slot's reservation and proves the sum in the world.
- `charge_at(&Charge, now_ic, &BatteryDef) -> u32`.
- `reserve_check(now, reserved, cost, home, &BatteryDef) -> Result<ChargeAfter{after_act, after_home}, ReserveShort{after_home: i64, floor}>`. A zero spend always passes; SM.4 supplies a real `home_cu`.
- `ExcursionCounts`, `CapRefused`, `cap_refusal`.
- `ramp_pose(&BayDef) -> Result<Pose, String>`, `site_cell`, `cells_apart` (Chebyshev).
- Constants: `LAUNCH_S` 300, `RECOVER_S` 300, `DUMP_S` 0, `MAX_EXCURSIONS_*` 4/32/1,024, `PAD_CENTRE_CM`, `IC_HOUR`, `MAX_LEG_WAYPOINTS` 32.

**Projection:**
- `TvRec { schema, tv, slot, operator?, hull?, excursion?, state: TvState{Bay,Out,Held,Parked}, site?, pose?: TvPose, act?: ActView, leg?: FrozenLeg{started_ic, waypoints: Vec<LegWaypoint{x_cm,y_cm,at_s}>}, applied, told, log }`: `new`, `key`, `push_log`, `check` (≤ 16 lines × 160 B, ≤ 4,096 B, ≤ 32 waypoints).
- **SM.4 fills `leg`.** The tick writes the projection through the private `write_projection`/`project_due`, so SM.4 extends `tick.rs`.
- `tv_rec_key`, `tv_rec_key_owner`, `TvRecState::{Present, Unreadable}`.
- Store: `TacsimStore::tv_rec(tv)` and `pub(crate) store::put_tv_rec(ctx, &TvRec)`.

**Acts** — `tacsim::surface::act`:
- Public:
  - `SURFACE_ACT_ACTION` "tacsim_surface_act", with outcome fields `act, actor, due_ic, ordinal, tv`.
  - `SURFACE_DONE_ACTION` "tacsim_surface_done", with outcome fields `actor, ordinal, result, tv`.
  - `act_key`/`done_key`: `tacsim.surface/{tv}/{ordinal}/act|done`.
  - `MAX_PROBE_CU`, `MAX_PROBE_S`.
  - `RigAt { tv, slot, mark, hull, record: TvLease }`.
  - `Where { Bay{hull}, Ground{domain,x_cm,y_cm,heading_mdeg}, Elsewhere }`.
  - `Refusal`, with `word`/`code`/`line`.
- `pub(crate)`:
  - `operator_rigs`, `where_is`.
  - `SiteBerth { entry, name, def, map, domain, ramp, ramp_label, bay_room, bay_slot }` and `site_berth(w, &HullView)`.
  - `ActPlan { kind, rig, site, secs, cost_cu, home_cu, params, from_bay }`.
  - **`launch_plan(w, actor, &RigAt)`**, which SM.4 prepends for the implied launch. Its refusal order is fixed: dark/held, under way, voyage dark, cleared, not a site, not aboard, Forfeit, ended, not in the bay.
  - `recover_plan`, `dump_plan`, `probe_plan`.
  - `rig_battery_id`, `battery_of`, `world_rigs`, `percent`, `doing_words`, `ramp_label`, `site_name`, `site_of_domain`, `load_words`, `code_of_word`, `act_cmd`.
  - `SurfaceActProvider` and `SurfaceDoneProvider`. Fixed parameters:
    - start: `now`; `rigs{tv:{slot,hull,battery_id?,plans{launch|recover|dump:FixedPlan},probe?}}`; `world`.
    - finish: `now`; `rigs{tv:{battery_id?,facts}}`.
  - The start prepare's refusal messages:
    - "that rig is not yours to work"
    - "that rig's orders moved on" (Conflict)
    - "that rig is busy" (Conflict)
    - "that rig has an excursion open"
    - the three cap lines
    - "that rig is in the bay"
    - "the rig's charge is under the reserve"
    - "the hopper is empty"
    - "that rig cannot be read"
  - The finish prepare's refusal messages:
    - "that act is already finished"
    - "that act is another pilot's"
    - "not due"
    - "that rig is not yours to work"
    - "the hopper could not be emptied onto the spoil"

**Tick** — `tacsim::surface::tick`:
- `SURFACE_TICK` "tacsim.surface", Coarse 1 s.
- `MAX_EXCURSIONS_VISITED_PER_FIRE` 64, `MAX_EXCURSIONS_WORKED_PER_FIRE` 16.
- `MAX_DONE_ATTEMPTS` 8, `DONE_BACKOFF_FIRES` 60, `SCAN_PAGE` 256, `MAX_RIG_RECORDS` 1,024.
- `SurfaceTickState`, `SurfaceFireStats { visited, worked, submitted, deferred, passed_over, told }`, `last_surface_fire(w)`.
- The per-kind world step (`world_step`) and the completion line (`completion_line`) are private. SM.4 and SM.7 add their kinds there.

**Handles** — `port::families`:
- `PortFamilies.excursions: StateFamilyHandle<RigKey, RigRecord>`.
- `PortHandles.surface_act` and `surface_done: Option<DefinitionRef>`.
- `PortHandles.surface_tools: Option<Arc<BTreeMap<CanonicalId, ToolDef>>>`, the one battery source.
- `port_tick::Fire::new` is `pub(crate)`.

**The verb** — `port::tv`:
- `TV_USAGE` replaces `LEASE_USAGE`. `SLOT_SWITCHES` is `["1","2","3","4"]`; `+tv` is Authoritative with switches 1–4, lease, launch, recover and dump.
- The act texts:
  - `WHICH_RIG_ACT`, `TV_UNWIRED`, `TV_NOT_ABOARD`
  - `LAUNCH_NOT_SITE`, `LAUNCH_UNDER_WAY`, `LAUNCH_CLEARED`, `LAUNCH_OUT`, `LAUNCH_OPEN`, `LAUNCH_FORFEIT`, `LAUNCH_ENDED`, `LAUNCH_PREVIEW`, `LAUNCH_FILED`, `LAUNCHED`
  - `RECOVER_IN_BAY`, `RECOVER_ELSEWHERE`, `RECOVER_FAR`, `RECOVER_PREVIEW`, `RECOVER_FILED`, `RECOVERED`
  - `DUMP_NOT_SITE`, `DUMP_NO_SPOIL`, `DUMP_EMPTY`, `DUMP_PREVIEW`, `DUMP_FILED`, `DUMPED`
  - `ACT_FAILED` with `WHY_NEVER_LEFT`, `WHY_NOT_BESIDE` and `WHY_HOPPER_CHANGED`
  - `RIG_BUSY`, `RESERVE_LOW`, `CAP_OPERATOR`, `CAP_SITE`, `CAP_WORLD`
  - `WHERE_BAY`, `WHERE_RAMP`, `WHERE_GROUND`, `WHERE_AWAY`, `ACT_LINE`
- `RIG_LINE`, `RIG_PAST_TERM`, `RIG_FORFEIT` and `RIG_LEASE_ENDED` now carry `{where}` and `{charge}`.
- `pub MAX_POINTERS` 4,096.
- `pub(crate)`: `RigPointer`, `hopper_text`, `hopper_goods_of`, `hopper_load_of`, `counter_words`, `rig_lease_live`, `span_words`, `due_words`, `live_on_hull`.

**Departures and sites:**
- `port::voyage`: `TV_OUT`, `TV_BUSY`, `HOPPER_LOADED`, `DepartRefusalKind::{TvOut, HopperLoaded}`, and `pub(crate) bay_gate(ctx, &HullView, name)`.
- `port::map::SiteRow.spoil` and `spoil_vault`, both `Option`.
- `port::site::spoil_vault(w, &PortMap) -> Option<VaultId>` (`pub(crate)`).
- The Site line reads "{entry} site: 16 block places, the site clerk and the spoil laid up."

**Unchanged:** no lane, pack or digest moved; `LANE_VERSION` is 10, and SM.5's lane is v11.

### Doc 20 amendments owed

- **§4.1, line 393** ("teleoperated from the settled hull's bridge under the operator's control lease"): as built (decision 14, pending the owner), the control lease is read as the live lessee (`TvLease.pilot`) aboard the settled hull. No mush-sim F3 lease is taken on the TV, and "aboard" gates launch, recover and dump alike.
- **§4.1, the reserve rule:** as built, a spend of zero (cost plus home equals 0) always passes, so a recover or a dump is never refused by the rule. The rule is enforced only in `tacsim_surface_act`'s prepare. `reserved_total` reads only the act in flight in SM.3b; SM.7's cycle slot extends it.
- **§4.3, "`+tv/launch` is an egress to `Direct` on the site's passive domain at the ramp pose (mush-sim/src/commands.rs:2060-2159)":** as built, the launch is the surface tick's `mush_sim::place` at `ramp_pose`. That pose is the pad centre (128,000, 128,000 cm, facing north) plus the bay's `EgressDef` offset; the Dray-P's stern ramp sits at (128,000, 124,500, 180,000). mush-sim's `egress_parent` is not used, because it folds the egress pose into a posture at the origin.
- **§4.3, "its slot number (1 or 2) is its name on every verb":** as built, the slot switches are `/1` to `/4` (`MAX_BAY_ITEMS`). A slot past the hull's bay answers `NO_RIG`, and a bare act addresses the rig last named, else the only rig.
- **§4.3, `TV_OUT` and `HOPPER_LOADED`:** as built, there is also a third text, `TV_BUSY` (a rig mid-act), under the same `TvOut` kind. All three refuse `+ship/plan`, `+ship/undock`, `+ship/lift` and the queued depart. `HOPPER_LOADED`'s remedy names only the dump until SM.8 adds the unload.
- **§4.3, "the site's spoil pool, a system vault per site":** as built, each site has a DARK "{site} spoil" Thing in its host room, backing a vault owned by `System(tacsim.sites)` with no scope (`CustodyRef::Container`). Both are laid by part (g) and named on the Site readiness line; a missing spoil closes the pad.
- **§4.3, "An excursion runs from launch to park":** as built in SM.3b, an excursion runs from a launch's start to a recover's `Done`; parking is SM.11's. A refused launch (`NeverLeft`) closes it. A recover from beside the ramp needs no open excursion. A recover of a rig in the bay with one still open closes it.
- **§6, line 537 (`tacsim_surface_mine`):** as built, every act's start is the one action `tacsim_surface_act` (`act` a `Choice`), and every finish is `tacsim_surface_done`, submitted by the tick as the operator. Mining becomes `act = mine` in SM.7, with no second start action.
- **§8, "each fire visits at most 64 excursions whose `due_ic ≤ now`":** as built, a fire also visits records whose last outcome is untold. Candidates are ordered by `(due_ic or last.at_ic, tv)`. The tick reads the whole family (at most 1,024 records) each fire.
- **§8, the state family:** as built, there is one `RigRecord` per terrain vehicle, not per excursion. It holds:
  - a monotonic act ordinal;
  - the lazy charge `{value_cu, at_ic, in_bay}`;
  - the act in flight with `reserved_cu`;
  - the open excursion;
  - the last `Outcome`.
- **§8, "A scan, a sounding, a drive, a mine run, a recover and an unload each begin as an authoritative submission":** the launch and the dump also start this way.
- **§8, "each verified by pull the way the executor verifies an order (a refused completion releases the lock and the charge and writes the public line)":** as built, the finish is an authoritative submission. It records `Done`, or `Refused{NeverLeft|NotBeside|HopperChanged}`, from the world its offer reads at landing, and stamps it at the act's own `due_ic`. A refused finish never spends charge. The next fire tells the operator one line (`ACT_FAILED`). A finish that does not land is retried, with a page after 8 attempts and a back-off of 60 fires.
- **§8, lines 702–704, "need the operator's control lease and the hull settled, nothing more":** read as the lessee aboard; see the §4.1 amendment.
- **§10, line 855, "A small pushed `tv.status` keyframe per act change":** not built in SM.3b (it is SM.9's). Act changes reach the operator only through the completion lines and `+tv` status.
- **§11:** the figures need adding or confirming:
  - **Tick row:** `RECOVER_S` 300 IC s and `DUMP_S` 0 (owner-critical defaults); the 8-attempt and 60-fire retry figures. "≤ 8 writes a completion, ≤ 3 for any other act" is unmeasured.
  - **Caps row:** confirm the caps count open excursions in the start's prepare over every lessor pool's rigs.
- **§13, the SM.3 row:** the lead split it into SM.3a and SM.3b. The implied launch (on the first scan, sound or drive) moves to SM.4, which prepends `launch_plan`.
- **§19, item 3, line 1172:** this item names the control lease too; read it as the lessee aboard, as in §4.1.
