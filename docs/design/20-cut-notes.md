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
