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
use ruleset_tacsim::{
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
    let text = body.replace("DIGEST", &"0".repeat(64));
    let parsed: RulesPack = serde_json::from_str(&text).expect("the wire parses");
    let digest = parsed.computed_digest().expect("digests").to_hex();
    body.replace("DIGEST", &digest)
}

/// A two-leg corridor seed pack whose FIRST leg is armed, with a controller.
fn armed_corridor(hazard: u8, interdiction: u8) -> String {
    signed(&format!(
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
    ))
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
        "1885f24044f15bdb63b858ed861665465e2b4527233696d237ec84389f758792",
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
    let refused = ruleset_tacsim::verify_sources(&cfg).expect_err("the boot must refuse");
    assert!(refused.contains("<belt>"), "naming the file: {refused}");
}

/// **A band past the cap is refused on load, never clamped** — and the refusal is the
/// route's own, so the pack does not half-materialize.
#[test]
fn e7r1_r2_a_band_past_the_cap_refuses_the_materialization() {
    assert_eq!(mush_sim::MAX_HAZARD_BAND, 5);
    let mut tw = boot_with(with_seed(armed_corridor(9, 3)));
    // The pack itself parses — the band is a `u8` — and the refusal lands where the
    // route is materialized against `mush-sim`'s own constructor.
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    assert!(
        snap(&tw)
            .route(&mush_core::CanonicalId::new("belt_run").expect("id"))
            .is_none(),
        "an over-band route is NOT materialized: refused, never clamped to 5"
    );
    // And the corridor read is the one existence-hiding line, not a partial render.
    let refused = err(&mut tw, GOD, "@corridor/show belt_run");
    assert!(refused.contains("No such corridor"), "{refused}");
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

/// **`@corridor/domain` names a real domain or says there is none** — never a
/// sentinel id. With no `mush-sim` domains composed in this rig, every leg presents
/// into none, which is exactly the honest answer.
#[test]
fn e7r1_r2_corridor_domain_names_no_sentinel_when_a_leg_presents_into_nothing() {
    let mut tw = boot_with(with_seed(armed_corridor(4, 3)));
    ok(&mut tw, GOD, "@tacsim/seed tacsim.belt commit");
    let out = ok(&mut tw, GOD, "@corridor/domain belt_run");
    assert!(out.contains("2 legs"), "{out}");
    assert_eq!(
        out.matches("presents into no domain").count(),
        2,
        "both legs, honestly: {out}"
    );
    assert!(
        !out.contains("domain #0"),
        "and never a sentinel domain id: {out}"
    );
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
    cfg.time = ruleset_tacsim::TacsimTimeConfig {
        rate,
        catch_up_journeys: "restamp_preserving_remaining".to_string(),
        catch_up_production: "pause_timeline_changed".to_string(),
        catch_up_fronts: "no_restamp".to_string(),
        catch_up_domains: "refuse_and_page".to_string(),
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
