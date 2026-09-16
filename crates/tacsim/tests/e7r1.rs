//! **E7 residue pass 1 — R2's data half** (10b §15.1 item 7).
//!
//! `hazard`, `interdiction` and `controller` as **pack-authored** fields, and the
//! `@corridor` read surface over the materialized route projection.
//!
//! The V lens runs through all of it: *a pack change re-prices, re-times and
//! re-resolves nothing already created or contracted* (T1 / Q18A). The three fields
//! are `serde(default)` **and** `skip_serializing_if`, so an unauthored one never
//! enters a pack digest — which is why the shipped `tacsim.corridor@1` digest is the
//! byte-for-byte one cut 1 published, and every route pinned to it is untouched.

use mush_core::{Audience, GOD, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use tacsim::{
    PackSource, RulesPack, TacsimConfig, TacsimRuleset, TacsimStore, fixture, load_pack,
};

// ---- E7r1: the rig (e7c1.rs's, verbatim where it applies) ------------------

fn boot_with(cfg: TacsimConfig) -> TestWorld {
    TestWorld::with_modules(vec![Box::new(TacsimRuleset::with_config(cfg))])
}

fn boot() -> TestWorld {
    boot_with(fixture::config())
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}
fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}
fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}
fn snap(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the tacsim slot always exists")
}

/// A pack body with its declared digest filled in for it — **re-derived, never
/// hand-edited**, which is the only way a pack is ever signed in this tree.
fn signed(body: &str) -> String {
    // S7 (D-P-1): a pack's digest is the content layer's, over the authored bytes.
    let text = body.replace("DIGEST", &"0".repeat(64));
    let parsed = mush_content::parse_pack(&text).expect("the wire parses");
    let digest = parsed.computed_digest().expect("digests").to_hex();
    body.replace("DIGEST", &digest)
}

/// A two-leg corridor seed pack whose FIRST leg is armed, with a controller —
/// **unsigned**, because a body past the hazard cap no longer parses and therefore
/// cannot be signed (independent verify fold, B11).
fn armed_corridor_source(hazard: u8, interdiction: u8) -> String {
    format!(
        r#"{{ "manifest": {{ "id": "tacsim.belt", "version": 1, "kind": "campaign_seed",
          "digest": "DIGEST", "requires": [] }},
          "entries": {{
            "belt_a": {{ "kind": "location", "def": {{ "label": "A", "role": "yard" }} }},
            "belt_node": {{ "kind": "location", "def": {{ "label": "N", "role": "transfer" }} }},
            "belt_b": {{ "kind": "location", "def": {{ "label": "B", "role": "yard" }} }},
            "belt_run": {{ "kind": "route", "def": {{
                "from": "belt_a", "to": "belt_b",
                "legs": [
                  {{ "to": "belt_node", "base_s": 100, "hazard": {hazard},
                     "interdiction": {interdiction} }},
                  {{ "to": "belt_b", "base_s": 200 }}
                ],
                "drive_class": "fusion_torch", "propellant": 0,
                "controller": "belt_authority" }} }}
          }} }}"#
    )
}

/// The signed form of [`armed_corridor_source`], for every band the pack may author.
fn armed_corridor(hazard: u8, interdiction: u8) -> String {
    signed(&armed_corridor_source(hazard, interdiction))
}

/// The fixture's `[tacsim]` block plus one more seed pack.
fn with_seed(body: String) -> TacsimConfig {
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<belt>".to_string(),
        bytes: body,
    });
    cfg
}

fn route(tw: &TestWorld, id: &str) -> mush_sim::RouteDef {
    snap(tw)
        .route(&mush_core::CanonicalId::new(id).expect("route id"))
        .cloned()
        .expect("a materialized route")
}

// ---- E7r1: R2 — the pack-authored corridor fields -------------------------

/// **V. The shipped corridor's digest did not move.**
///
/// The three new fields are `skip_serializing_if`, so an unauthored one contributes
/// no bytes to the canonical encoding the digest is taken over. That is what makes
/// the residue pass safe for a world already seeded from `tacsim.corridor@1`: every
/// v1-pinned route, and every hauling contract's route assumptions with it, is
/// re-priced, re-timed and re-resolved by exactly nothing.
#[test]
fn e7r1_r2_the_shipped_corridors_digest_is_unmoved_by_the_new_fields() {
    let pack = load_pack(fixture::CORRIDOR_JSON).expect("the shipped corridor still loads");
    assert_eq!(
        pack.manifest().digest().to_hex(),
        "6ad21275d8ba59a6b742cf354e2efafe840cacaa34f8e6c84ac057b9997f8d69",
        "cut 1's published digest, byte for byte"
    );
    assert_eq!(
        pack.computed_digest().expect("digest").to_hex(),
        pack.manifest().digest().to_hex(),
        "and it is still the digest the content actually has"
    );
    // The core rules pack likewise.
    let core = load_pack(fixture::CORE_JSON).expect("the shipped core still loads");
    assert_eq!(
        core.computed_digest().expect("digest"),
        core.manifest().digest()
    );
    // The route it authors is not interdicted, and says so.
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    let corridor = route(&tw, fixture::ROUTE);
    assert!(!corridor.is_interdicted());
    assert_eq!(corridor.controller(), None, "the corridor is unclaimed");
    for leg in corridor.legs() {
        assert_eq!((leg.hazard(), leg.interdiction()), (0, 0));
    }
}

/// **The authored fields materialize onto the route**, through
/// `store::materialize_route`, and are readable through `@corridor`.
#[test]
fn e7r1_r2_the_pack_authored_fields_materialize_onto_the_route() {
    let mut tw = boot_with(with_seed(armed_corridor(4, 3)));
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let belt = route(&tw, "belt_run");
    assert_eq!(belt.legs().len(), 2);
    assert_eq!(
        (belt.legs()[0].hazard(), belt.legs()[0].interdiction()),
        (4, 3)
    );
    assert!(belt.legs()[0].is_interdicted(), "4 >= 3");
    assert_eq!(
        (belt.legs()[1].hazard(), belt.legs()[1].interdiction()),
        (0, 0)
    );
    assert!(!belt.legs()[1].is_interdicted());
    assert_eq!(
        belt.controller().map(mush_core::CanonicalId::as_str),
        Some("belt_authority")
    );
    assert!(belt.is_interdicted(), "the corridor as a whole is armed");

    let shown = ok(&mut tw, GOD, "@corridor/show belt_run");
    assert!(shown.contains("belt_authority"), "{shown}");
    assert!(shown.contains("2 legs"), "{shown}");
    let rules = ok(&mut tw, GOD, "@corridor/rules belt_run");
    assert!(rules.contains("interdicted"), "{rules}");
    assert!(rules.contains("hazard 4, interdiction 3"), "{rules}");
    assert!(
        rules.contains("raises an interception"),
        "the armed leg is named: {rules}"
    );
    assert!(rules.contains("hazard 0, interdiction 0"), "{rules}");
}

/// **A hand-edited digest fails `DigestMismatch` naming BOTH, and a re-derived one
/// loads.** The fixture-loading path recomputes and compares, so a stale declaration
/// fails the boot loudly and re-deriving is the only way to publish.
#[test]
fn e7r1_r2_a_hand_edited_corridor_digest_fails_naming_both_and_a_re_derived_one_loads() {
    let good = armed_corridor(4, 3);
    load_pack(&good).expect("a re-derived digest loads");

    // Arm the leg one band higher WITHOUT re-deriving: the classic hand edit.
    let hand_edited = good.replace("\"hazard\": 4", "\"hazard\": 5");
    let error = load_pack(&hand_edited).expect_err("a stale declaration must fail");
    let detail = error.to_string();
    let declared = load_pack(&good).expect("good").manifest().digest().to_hex();
    let recomputed = {
        let parsed: RulesPack = serde_json::from_str(&hand_edited).expect("parses");
        parsed.computed_digest().expect("digest").to_hex()
    };
    assert!(
        detail.contains(&declared) && detail.contains(&recomputed),
        "the refusal names BOTH digests: {detail}"
    );
    // And re-deriving it is what publishes it.
    let re_derived = signed(
        &good
            .replace(&declared, "DIGEST")
            .replace("\"hazard\": 4", "\"hazard\": 5"),
    );
    let pack = load_pack(&re_derived).expect("a re-derived pack loads");
    assert_eq!(pack.computed_digest().expect("d"), pack.manifest().digest());

    // The whole boot refuses too, naming the file — the host-side gate.
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<belt>".to_string(),
        bytes: hand_edited,
    });
    let refused = tacsim::verify_sources(&cfg).expect_err("the boot must refuse");
    assert!(refused.contains("<belt>"), "naming the file: {refused}");
}

/// **A pack past the hazard cap fails the BOOT, naming the file, the route and the
/// band** (independent verify fold, B11 — this test's *before* picture, rewritten in
/// place).
///
/// It used to assert only that the route was not materialized. That was true and it
/// was the defect: `store`'s refresh loop is `Err(detail) => tracing::warn!(..,
/// "tacsim: a pack route could not be materialized; skipped")`, so a pack authoring
/// `hazard = 9` parsed, passed digest verification, seeded **green** — `@tacsim/seed
/// … commit` answered OK — and the route simply disappeared, with `@corridor/show`
/// answering `No such corridor.` and any journey plan naming it failing. No held
/// document, no escalation, no boot refusal; every other refused-on-load record in
/// the tree is louder (cut 6 quarantines a refused journey and pages, cut 9 holds a
/// refused operation, `[tacsim.recovery]` refuses the boot on a typo). §15.1 row 7's
/// *"refused on load rather than clamped"* was true only in the weakest sense.
#[test]
fn vf_b11a_a_pack_past_the_hazard_cap_fails_the_boot_naming_the_file_the_route_and_the_band() {
    assert_eq!(mush_sim::MAX_HAZARD_BAND, 5);
    // The two constants are stated in two crates for the no-edge reason, so they are
    // pinned together here rather than left to drift.
    assert_eq!(
        tacsim::MAX_HAZARD_BAND,
        mush_sim::MAX_HAZARD_BAND,
        "the pack's own band and the sim's are one number"
    );
    // `signed()` cannot be used: the body no longer parses at all, which is the
    // point — so the declared digest is a well-formed placeholder and the refusal
    // still lands on the BAND rather than on the signature.
    let body = armed_corridor_source(9, 3).replace("DIGEST", &"0".repeat(64));
    let error = tacsim::load_pack(&body).expect_err("the pack must not load");
    let rendered = error.to_string();
    assert!(
        rendered.contains("belt_run"),
        "naming the route: {rendered}"
    );
    assert!(rendered.contains('9'), "naming the band: {rendered}");
    assert!(rendered.contains('5'), "naming the maximum: {rendered}");

    // …and the whole boot refuses, naming the FILE, before any module runs.
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<belt>".to_string(),
        bytes: body,
    });
    let refused = tacsim::verify_sources(&cfg).expect_err("the boot must refuse");
    assert!(refused.contains("<belt>"), "naming the file: {refused}");
    assert!(refused.contains("belt_run"), "and the route: {refused}");
}

/// **The negative B11 asks for: no composition reaches a state where a seed commits
/// green while its route is absent.**
///
/// A legal band still materializes, and the seed and the corridor read agree — so
/// "the seed said OK" and "the corridor exists" can no longer disagree.
#[test]
fn vf_b11b_a_seed_that_commits_green_always_leaves_its_corridor_readable() {
    for (hazard, interdiction) in [(0, 0), (1, 1), (5, 5), (4, 3)] {
        let mut tw = boot_with(with_seed(armed_corridor(hazard, interdiction)));
        ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
        let shown = ok(&mut tw, GOD, "@corridor/show belt_run");
        assert!(
            shown.contains("belt_run"),
            "band {hazard}/{interdiction} committed green and reads back: {shown}"
        );
        assert!(
            snap(&tw)
                .route(&mush_core::CanonicalId::new("belt_run").expect("id"))
                .is_some(),
            "band {hazard}/{interdiction} materialized"
        );
    }
}

/// **Disclosure (§4 rule 4): an unobserved corridor and one that was never authored
/// are byte-identical**, so `@corridor` is not a corridor-existence oracle.
#[test]
fn e7r1_r2_an_unobserved_and_a_nonexistent_corridor_are_byte_identical() {
    let mut tw = boot_with(with_seed(armed_corridor(4, 3)));
    // `belt_run` is authored but NOT seeded, so no route is materialized for it.
    let unobserved = err(&mut tw, GOD, "@corridor/show belt_run");
    let nonexistent = err(&mut tw, GOD, "@corridor/show no_such_corridor_at_all");
    let malformed = err(&mut tw, GOD, "@corridor/show ..!!");
    assert_eq!(unobserved, nonexistent);
    assert_eq!(unobserved, malformed);
    assert!(unobserved.contains("No such corridor"), "{unobserved}");

    // Every switch answers the same way, and a non-staff viewer gets the same bytes
    // as staff — the verb discloses nothing either way.
    let player = tw.spawn_player("Pat");
    for switch in ["show", "rules", "domain"] {
        let staff = err(&mut tw, GOD, &format!("@corridor/{switch} belt_run"));
        let ordinary = err(&mut tw, player, &format!("@corridor/{switch} belt_run"));
        assert_eq!(staff, ordinary, "switch {switch}");
        assert_eq!(staff, nonexistent, "switch {switch}");
    }

    // Once seeded, it renders — for both.
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let staff = ok(&mut tw, GOD, "@corridor/show belt_run");
    let ordinary = ok(&mut tw, player, "@corridor/show belt_run");
    assert_eq!(staff, ordinary, "a corridor is public once it exists");
}

/// **`@corridor/domain` renders a pack-derived boolean and never a live domain id**
/// (independent verify fold, C5/B6; owner decision F8(b)) — rewritten in place.
///
/// `mush_sim::commands::cmd_domain` opens with `require_admin` on all eleven of its
/// switches, so the existence, id and status of a simulation domain is staff-only
/// everywhere else in the tree; `@corridor` is one `ReadOnly` group behind
/// `require_enabled` with `..CommandSpec::default()` — no power, no role — and it read
/// `SpatialState::domain_of_host(leg.to())` and printed the live `DomainId` to any
/// player, which after R2 is exactly where an interception has just promoted.
///
/// This test also carried the false pin B6 found: it asserted
/// `!out.contains("domain #0")` as a proxy for *"never a sentinel id"*, but
/// `DomainId(0)` is a legitimate **first** domain, so on a world with a real domain
/// the assertion would have failed for the wrong reason. The phrase is now the pin.
#[test]
fn vf_c5a_corridor_domain_renders_the_pack_derived_answer_and_no_live_domain_id() {
    let mut tw = boot_with(with_seed(armed_corridor(4, 3)));
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let player = tw.spawn_player("Pat");
    let staff = ok(&mut tw, GOD, "@corridor/domain belt_run");
    let ordinary = ok(&mut tw, player, "@corridor/domain belt_run");
    assert_eq!(
        staff, ordinary,
        "and staff learn nothing extra from it either"
    );
    assert!(staff.contains("2 legs"), "{staff}");
    assert!(
        !staff.contains("presents into domain"),
        "the live-domain phrase is gone from the verb entirely: {staff}"
    );
    // **`DomainId(0)` is a legal domain**, so no rendering may use `#0` as a
    // sentinel — the pin the old test had backwards (B6).
    assert!(
        !staff.contains("domain #"),
        "no domain id of any value crosses to a reader: {staff}"
    );
    assert_eq!(
        staff.matches("hosts no domain").count(),
        2,
        "with no domain host bound, both legs say so honestly: {staff}"
    );
}

/// **A leg bound as a domain host says an interception can promote there — and still
/// names no domain** (C5's positive arm; F8(b)).
///
/// The pass's own disclosure test composed **no** `mush-sim` module, so every leg
/// answered *"presents into no domain"* and the leak was never exercised. This one
/// composes both modules and binds a real host.
#[test]
fn vf_c5b_a_bound_host_reads_as_promotable_without_naming_the_domain() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(TacsimRuleset::with_config(with_seed(armed_corridor(4, 3)))),
    ]);
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let node = route(&tw, "belt_run").legs()[0].to();
    tw.world_mut()
        .attach_behavior(
            node,
            mush_core::BehaviorId::new("sim.domain").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the host");
    // A real, live domain at that endpoint — `DomainId(0)`, the world's first.
    ok(
        &mut tw,
        GOD,
        &format!("@domain/create #{}=sim.passive", node.0),
    );
    let player = tw.spawn_player("Pat");

    // The staff-only surface is still staff-only…
    let denied = texts(&tw.run(player, &format!("@domain/status #{}", node.0)));
    assert!(
        denied.contains("Permission denied"),
        "`@domain` is staff's: {denied}"
    );
    // …and the corridor read answers the question its help text asks, without it.
    let leaked = ok(&mut tw, player, "@corridor/domain belt_run");
    assert!(
        leaked.contains("an interception here can promote"),
        "the interdicted, host-bound leg reads as promotable: {leaked}"
    );
    assert!(
        !leaked.contains("domain #"),
        "and no live DomainId — #0 included — crosses to a bystander: {leaked}"
    );
    assert_eq!(
        leaked,
        ok(&mut tw, GOD, "@corridor/domain belt_run"),
        "staff and a bystander read the same bytes, so nothing is hidden either"
    );
}

/// **An uninterdicted leg at a bound host does not read as promotable** (F8(b)'s
/// third arm: the boolean is `is_interdicted() && host-bound`, and both halves are
/// driven).
#[test]
fn vf_c5c_a_bound_host_on_an_uninterdicted_leg_promotes_nothing() {
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(TacsimRuleset::with_config(with_seed(armed_corridor(0, 0)))),
    ]);
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let node = route(&tw, "belt_run").legs()[0].to();
    tw.world_mut()
        .attach_behavior(
            node,
            mush_core::BehaviorId::new("sim.domain").expect("well-formed"),
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .expect("bind the host");
    let out = ok(&mut tw, GOD, "@corridor/domain belt_run");
    assert!(
        out.contains("is a domain host, but this leg is not interdicted"),
        "{out}"
    );
    assert!(!out.contains("can promote"), "{out}");
}

/// **V. A `v2` corridor pack re-prices, re-times and re-resolves nothing already
/// created.**
///
/// Two things carry it, and both are asserted: publishing v2 does not touch v1's
/// **published digest** (versions coexist, Q18A), and the materialized route follows
/// the **seed latch** — what this world actually applied — not the watermark. So a
/// corridor a v1 world is flying keeps v1's legs, v1's times and v1's (absent)
/// hazard, whatever a later pack says.
#[test]
fn e7r1_r2_a_v2_corridor_pack_leaves_every_v1_pinned_route_unchanged() {
    let v1 = armed_corridor(0, 0);
    let v1_digest = load_pack(&v1).expect("v1").manifest().digest().to_hex();
    // v2: the same corridor, re-armed and re-derived under a new version.
    let v2 = signed(
        &armed_corridor(4, 3)
            .replace("\"version\": 1", "\"version\": 2")
            .replace(
                &load_pack(&armed_corridor(4, 3))
                    .expect("armed")
                    .manifest()
                    .digest()
                    .to_hex(),
                "DIGEST",
            ),
    );
    let v2_pack = load_pack(&v2).expect("v2 loads");
    assert_eq!(v2_pack.manifest().version(), 2);

    // v1's own digest is untouched by v2 existing at all.
    assert_eq!(
        load_pack(&v1)
            .expect("v1 still loads")
            .manifest()
            .digest()
            .to_hex(),
        v1_digest,
        "publishing v2 moved nothing about v1"
    );

    // A world that applied v1 keeps v1's route, with v2 published beside it.
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<belt v1>".to_string(),
        bytes: v1,
    });
    let mut tw = boot_with(cfg);
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let before = route(&tw, "belt_run");
    assert!(!before.is_interdicted(), "v1 armed nothing");
    let flown: u32 = before.legs().iter().map(mush_sim::LegDef::base_s).sum();
    assert_eq!(flown, 300);

    // Now publish v2 into the same world's configuration and re-boot the module.
    let mut cfg = fixture::config();
    cfg.seeds.push(PackSource {
        name: "<belt v2>".to_string(),
        bytes: v2,
    });
    let mut later = boot_with(cfg);
    // v2 is published but NOT seeded in this world, so nothing of it materializes —
    // the latch is what decides, never the watermark.
    assert!(
        later
            .world()
            .module_state::<TacsimStore>()
            .expect("slot")
            .route_in(
                &mush_core::CanonicalId::new("tacsim.belt").expect("pack"),
                2,
                &mush_core::CanonicalId::new("belt_run").expect("id"),
            )
            .is_none(),
        "an unseeded version materializes no route"
    );
    let refused = err(&mut later, GOD, "@corridor/show belt_run");
    assert!(refused.contains("No such corridor"), "{refused}");
}

// ---- E7r1: R5(a) — the rate policy's caller -------------------------------

fn timed(rate: u32) -> TacsimConfig {
    let mut cfg = fixture::config();
    cfg.time = tacsim::TacsimTimeConfig {
        rate,
        catch_up_journeys: "restamp_preserving_remaining".to_string(),
        catch_up_production: "pause_timeline_changed".to_string(),
        catch_up_fronts: "no_restamp".to_string(),
        catch_up_domains: "refuse_and_page".to_string(),
        ephemeris: tacsim::EphemerisConfig::default(),
    };
    cfg
}

/// **`@tacsim/time show` renders the live generation and every declared policy, and
/// writes nothing.** `[tacsim.time]` stopped being inert here.
#[test]
fn e7r1_r5_time_show_renders_the_declared_policies_and_writes_nothing() {
    let mut tw = boot_with(timed(4));
    // Drain the boot's own writes first, so what is counted is the command's.
    let _ = tw.take_module_ops();
    let out = ok(&mut tw, GOD, "@tacsim/time show");
    assert!(out.contains("generation 1"), "{out}");
    for word in [
        "restamp_preserving_remaining",
        "pause_timeline_changed",
        "no_restamp",
        "refuse_and_page",
    ] {
        assert!(out.contains(word), "the declared policy is live: {out}");
    }
    assert_eq!(
        tw.take_module_ops().len(),
        0,
        "a read writes nothing at all"
    );
    // A player sees the same thing: the rate is public.
    let player = tw.spawn_player("Pat");
    assert_eq!(ok(&mut tw, player, "@tacsim/time show"), out);
}

/// **Opening a generation is ONE attributed act**, on the kernel's own timeline, and
/// each subsystem's declared policy is announced exactly once for it.
#[test]
fn e7r1_r5_the_rate_verb_opens_one_generation_and_announces_each_policy_once() {
    let mut tw = boot_with(timed(4));
    let out = ok(&mut tw, GOD, "@tacsim/time rate=8");
    assert!(out.contains("generation 2"), "{out}");
    assert!(out.contains("8 IC seconds per real second"), "{out}");
    for word in [
        "restamp_preserving_remaining",
        "pause_timeline_changed",
        "no_restamp",
        "refuse_and_page",
    ] {
        assert_eq!(
            out.matches(word).count(),
            1,
            "each subsystem's policy fires exactly once for the generation: {out}"
        );
    }
    // ONE generation, and every earlier one still stands: nothing recorded is
    // reinterpreted (T1).
    let epochs = tw.world().world_timeline().epochs().to_vec();
    assert_eq!(epochs.len(), 2, "one new generation, not two");
    assert_eq!(epochs[0].generation, 1);
    assert_eq!(epochs[1].generation, 2);
    assert_eq!(epochs[1].rate, 8);
    // …and `show` now reads the live one.
    let shown = ok(&mut tw, GOD, "@tacsim/time show");
    assert!(shown.contains("generation 2"), "{shown}");
}

/// **Re-typing the live rate opens no generation**, so a subsystem is never made to
/// react to a change that is not one.
#[test]
fn e7r1_r5_re_typing_the_live_rate_opens_no_generation() {
    let mut tw = boot_with(timed(4));
    let live = tw
        .world()
        .world_timeline()
        .epochs()
        .last()
        .copied()
        .expect("a generation");
    let refused = err(&mut tw, GOD, &format!("@tacsim/time rate={}", live.rate));
    assert!(refused.contains("already runs at"), "{refused}");
    assert_eq!(
        tw.world().world_timeline().epochs().len(),
        1,
        "nothing was opened"
    );
}

/// **A non-staff actor cannot change the rate**, and the refusal changes nothing.
#[test]
fn e7r1_r5_a_non_staff_actor_cannot_change_the_rate() {
    let mut tw = boot_with(timed(4));
    let pat = tw.spawn_player("Pat");
    let refused = err(&mut tw, pat, "@tacsim/time rate=9");
    assert!(refused.contains("Only staff"), "{refused}");
    assert_eq!(tw.world().world_timeline().epochs().len(), 1);
}

/// **A disabled layer refuses out loud** — `@tacsim/time` is not a second switch that
/// escapes the `enabled` gate.
#[test]
fn e7r1_r5_a_disabled_layer_refuses_the_time_verb() {
    let mut tw = boot_with(TacsimConfig::default());
    for line in ["@tacsim/time show", "@tacsim/time rate=8"] {
        let refused = err(&mut tw, GOD, line);
        assert!(refused.contains("not enabled"), "{refused}");
    }
}

/// **`[tacsim.time]` still reaches `load_or_seed_world_timeline` never.** Generation
/// one comes from `[game]`; this crate constructs no `GameClock` and no
/// `WorldTimelineEpoch`, and opens later generations only through the kernel's own
/// `Ctx::change_world_time_rate`. §3.1's pinned `mush-server` test is untouched, and
/// this is its source-side twin.
#[test]
fn e7r1_r5_the_time_block_still_seeds_no_timeline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut callers = 0usize;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src/").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
            // Comment lines are prose, not reach: this module's own doc comments name
            // the function they deliberately do NOT call.
            let production: String = text[..boundary]
                .lines()
                .filter(|line| {
                    let code = line.trim_start();
                    !code.starts_with("//")
                })
                .collect::<Vec<_>>()
                .join(
                    "
",
                );
            for needle in [
                "load_or_seed_world_timeline",
                "WorldTimelineEpoch {",
                "GameClock {",
            ] {
                assert!(
                    !production.contains(needle),
                    "{} reaches {needle}; [tacsim.time] is rate POLICY and seeds nothing",
                    path.display()
                );
            }
            callers += production.matches("change_world_time_rate(").count();
        }
    }
    assert_eq!(
        callers, 1,
        "exactly one caller of the kernel's own rate change — the verb, and nothing else"
    );
}

// ---- E7 residue pass 1 independent verify fold ----

/// **`rate = 0` is refused by the verb AND by the kernel, and opens no generation**
/// (independent verify fold, C9/B10; owner decision F9(a), band `1..=3_600`).
///
/// `time_rate` parsed a `u32` and validated only that it differed from the live rate;
/// `Ctx::change_world_time_rate` → `World::next_world_timeline_epoch` →
/// `WorldTimeline::next_rate_epoch` checked the anchor, contiguity and
/// `MAX_WORLD_TIMELINE_EPOCHS` and **never the rate**. `WorldTimelineEpoch::ic_at`
/// computes `ic_anchor + delta * rate`, so at zero every `world_ic_now` consumer —
/// recovery cooldowns, production schedules, journey ETAs, escalation cool-offs —
/// stops for ever. Boot was protected (`WorldTimeline::from_game_clock` clamps
/// `ic_ratio.max(1)`); the verb was not.
#[test]
fn vf_c9a_rate_zero_is_refused_by_the_verb_and_by_the_kernel_and_opens_no_generation() {
    assert_eq!(mush_core::MIN_WORLD_TIME_RATE, 1);
    assert_eq!(mush_core::MAX_WORLD_TIME_RATE, 3_600);
    const {
        assert!(mush_core::MIN_WORLD_TIME_RATE >= 1);
        assert!(mush_core::MAX_WORLD_TIME_RATE > mush_core::MIN_WORLD_TIME_RATE);
    }
    let mut tw = boot();
    let before = tw.world().world_timeline().epochs().len();
    let refused = err(&mut tw, GOD, "@tacsim/time rate=0");
    assert!(refused.contains("between 1 and 3600"), "{refused}");
    assert!(
        refused.contains("not a pause"),
        "and it says why a stopped clock is not a legal rate: {refused}"
    );
    assert_eq!(
        tw.world().world_timeline().epochs().len(),
        before,
        "no generation was opened for a refused rate"
    );

    // The KERNEL refuses it too, so no other caller can install one.
    let timeline = tw.world().world_timeline().clone();
    let error = timeline
        .next_rate_epoch(tw.world().now().saturating_add(1), 0)
        .expect_err("the kernel refuses a stopped clock");
    assert!(
        matches!(
            error,
            mush_core::WorldTimelineError::RateOutOfBand { rate: 0, .. }
        ),
        "{error:?}"
    );
}

/// **A rate past the band is refused, naming the band** (C9/B10: `u32::MAX` was
/// accepted, and at that rate one real second advances the IC calendar by 136 years).
#[test]
fn vf_c9b_a_rate_past_the_band_is_refused_naming_the_band() {
    let mut tw = boot();
    let before = tw.world().world_timeline().epochs().len();
    for rate in [mush_core::MAX_WORLD_TIME_RATE.saturating_add(1), u32::MAX] {
        let refused = err(&mut tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert!(refused.contains("between 1 and 3600"), "{rate}: {refused}");
        assert!(refused.contains("nothing was changed"), "{rate}: {refused}");
    }
    assert_eq!(tw.world().world_timeline().epochs().len(), before);
    // The kernel is the backstop for both ends of the band.
    let timeline = tw.world().world_timeline().clone();
    assert!(matches!(
        timeline.next_rate_epoch(tw.world().now().saturating_add(1), u32::MAX),
        Err(mush_core::WorldTimelineError::RateOutOfBand { .. })
    ));
    // …and the edges of the band are legal, so the refusal is a band and not a wall.
    for rate in [
        mush_core::MIN_WORLD_TIME_RATE,
        mush_core::MAX_WORLD_TIME_RATE,
    ] {
        assert!(
            timeline
                .next_rate_epoch(tw.world().now().saturating_add(1), rate)
                .is_ok(),
            "rate {rate} is inside the band"
        );
    }
}

/// **The world clock and the ambient clock advance together across a legal rate
/// change** (C9, narrowed by the fold's own grounding — the invariant the divergence
/// broke, asserted positively).
///
/// The verify's stated mechanism — *"`world_ic_now` freezes while `game_now_unix`
/// keeps ticking"* — is wrong on this tree: `World::game_now_unix()` **is**
/// `self.world_ic_now().at`, so under `rate = 0` both freeze. The real divergence is
/// one seam over: `game_clock_compatibility(epoch)` stores `GameClock { ic_ratio: 0 }`
/// whose only two readers — `mush_core::module`'s `AMBIENT_IC_TIME_KEY` projection and
/// `mush_economy::sweep`'s `ratio` — both apply `.max(1)`, so the IC clock and
/// calendar a player reads in the room would have kept advancing at 1:1 against a
/// world in which nothing keyed on `world_ic_now` would ever come due again.
/// Banding the rate is what makes the two agree by construction.
#[test]
fn vf_c9c_the_world_clock_and_the_ambient_clock_advance_together_across_a_legal_rate_change() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@tacsim/time rate=4");
    let ratio = tw.world().game_clock().ic_ratio;
    assert_eq!(ratio, 4, "the compatibility clock carries the live rate");
    assert_eq!(
        ratio.max(1),
        ratio,
        "so its two `.max(1)` readers see exactly the rate the timeline runs at"
    );
    let start = tw.world().world_ic_now().at;
    let then = tw.world().now() + 10;
    tw.set_now(then);
    let moved = tw.world().world_ic_now().at - start;
    assert_eq!(moved, 40, "ten real seconds at 4:1 is forty IC seconds");
    // `game_now_unix` IS `world_ic_now().at` on this tree, so they cannot diverge.
    assert_eq!(tw.world().game_now_unix(), tw.world().world_ic_now().at);
}

/// **`/time show` names the remaining epoch budget** (F9(a)'s third part).
///
/// `MAX_WORLD_TIMELINE_EPOCHS` is a **lifetime** budget — old generations are never
/// reclaimed, because every old `ClockStamp` keeps its meaning for ever — so a
/// staffer toggling the rate can exhaust it. It is now said out loud rather than
/// discovered at 4 096.
#[test]
fn vf_c9d_time_show_names_the_remaining_generation_budget() {
    let mut tw = boot();
    let shown = ok(&mut tw, GOD, "@tacsim/time rate=2");
    assert!(
        shown.contains(&format!(
            "{} of {} world-time generations remain",
            mush_core::MAX_WORLD_TIMELINE_EPOCHS - 2,
            mush_core::MAX_WORLD_TIMELINE_EPOCHS
        )),
        "the budget is named after the act that spent one: {shown}"
    );
}

/// **No player-visible string this pass shipped carries a run of spaces**
/// (Z1, this fold's own grounding — four sites, all in `945e27a`).
///
/// Rust multi-line string literals keep every leading space of the continuation line
/// unless the line ends in `\`, so `"…and is paused;\n                 `@wo/restamp`…"`
/// reaches a player as ten to eighteen spaces mid-sentence. All four were the first
/// thing a player read after the two verbs this pass introduced: `wo_deliver`'s pause
/// line, `time_rate`'s generation announcement and its policy summary, and
/// `@corridor`'s own help text.
#[test]
fn vf_z1a_no_shipped_tacsim_string_carries_a_double_space() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();
    for file in std::fs::read_dir(&dir).expect("src/").flatten() {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let boundary = text.find("#[cfg(test)]").unwrap_or(text.len());
        for (n, line) in text[..boundary].lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            // The un-continued-literal shape, `mush-industry`'s own `vf_z1a` verbatim:
            // a quoted run that opens AND closes on one physical line, starts with
            // real words, and carries two consecutive spaces inside. A `\`-continued
            // literal's later lines carry no opening quote and are skipped, so the
            // deliberately column-aligned help text is not an offender.
            let Some(open) = line.find('"') else {
                continue;
            };
            let tail = &line[open + 1..];
            if tail.contains("  ")
                && tail.trim_end().ends_with('"')
                && tail
                    .split("  ")
                    .next()
                    .is_some_and(|head| head.chars().any(char::is_alphanumeric))
            {
                offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "player-visible literals with a run of spaces mid-sentence:
{}",
        offenders.join(
            "
"
        )
    );
}
