//! **E7 residue pass 2 — P5: the Mars–Phobos–Ceres corridor as content**
//! (10b §15.1 item 9's content half, 9b; doc 10 §11).
//!
//! The **V lens** is the whole point of this file: *a pack change re-prices, re-times
//! and re-resolves nothing already created or contracted* (T1/Q18A). P5 does not edit
//! the published `tacsim.corridor@1` — it publishes `tacsim.corridor@2` beside it, and
//! the first test below is the proof that v1 is untouched. The v2 digest is
//! **re-derived, never hand-edited**, and a hand-edited one fails `DigestMismatch`
//! naming both.
//!
//! The rig is `crates/ruleset-tacsim/tests/e7r1.rs`'s.

use mush_core::{Audience, GOD, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_tacsim::{
    MAX_PACK_ENTRIES, MAX_SEED_OBJECTS, PackError, TacsimConfig, TacsimRuleset, TacsimStore,
    fixture, load_pack,
};

// ---- E7r2: the rig ---------------------------------------------------------

fn boot_with(cfg: TacsimConfig) -> TestWorld {
    TestWorld::with_modules(vec![Box::new(TacsimRuleset::with_config(cfg))])
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
fn snap(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the tacsim slot always exists")
}
/// Whether the seed instantiated a world object for one pack-local place id.
fn seeded(tw: &TestWorld, id: &str) -> bool {
    snap(tw)
        .seeded_objects()
        .any(|(name, _)| name.as_str() == id)
}

fn route(tw: &TestWorld, id: &str) -> mush_sim::RouteDef {
    snap(tw)
        .route(&mush_core::CanonicalId::new(id).expect("route id"))
        .cloned()
        .expect("a materialized route")
}

// ---- E7r2: P5 — the V lens first -------------------------------------------

/// **The corridor's digest is RE-DERIVED and `tacsim.corridor@1` is untouched.**
///
/// The pass publishes a **version**, not an edit: v1's bytes, and therefore its
/// digest, are exactly cut 1's, so every world seeded from it and every hauling
/// contract's route assumptions with it are re-priced, re-timed and re-resolved by
/// **nothing** (T1/Q18A — versions coexist, and a consumer stays pinned until an
/// explicit migration). `e7r1.rs`'s own digest pin is green and unmodified beside
/// this, which is the same claim from the other end.
#[test]
fn e7r2_p5_v2_is_published_beside_v1_and_re_prices_nothing_already_pinned() {
    let v1 = load_pack(fixture::CORRIDOR_JSON).expect("the shipped v1 still loads");
    assert_eq!(v1.manifest().version(), 1);
    assert_eq!(
        v1.manifest().digest().to_hex(),
        "6ad21275d8ba59a6b742cf354e2efafe840cacaa34f8e6c84ac057b9997f8d69",
        "cut 1's published digest, byte for byte — P5 edits no published pack"
    );

    let v2 = load_pack(fixture::CORRIDOR_V2_JSON).expect("v2 loads, so its digest is real");
    assert_eq!(v2.manifest().id().as_str(), fixture::CORRIDOR_PACK);
    assert_eq!(v2.manifest().version(), 2, "a NEW version, not an edit");
    assert_ne!(
        v2.manifest().digest(),
        v1.manifest().digest(),
        "different content, different digest"
    );
    assert_eq!(
        v2.computed_digest().expect("digest"),
        v2.manifest().digest(),
        "and the declared digest is the one the content actually has: re-derived, \
         never hand-edited"
    );
    // Its one dependency is still the unmoved core rules pack.
    let core = load_pack(fixture::CORE_JSON).expect("the shipped core still loads");
    assert_eq!(
        core.computed_digest().expect("digest"),
        core.manifest().digest(),
        "P5 authored nothing new in `tacsim.core`: every commodity, recipe and \
         formation the loop names was already there"
    );
    assert_eq!(v2.manifest().requires().len(), 1);
    assert_eq!(v2.manifest().requires()[0].version, 1);
}

/// **A hand-edited digest fails `DigestMismatch` naming both.**
///
/// The one way a pack is ever signed in this tree is by re-derivation, and this is the
/// refusal that makes that true rather than customary.
#[test]
fn e7r2_p5_a_hand_edited_v2_digest_fails_naming_both() {
    let tampered = fixture::CORRIDOR_V2_JSON.replacen(
        "f89b7310b1d30e7b38a5a6cf9fe3e22330065c797dcf750e9cb8dc8554177041",
        &"0".repeat(64),
        1,
    );
    let error = load_pack(&tampered).expect_err("a hand-edited digest is refused");
    assert!(matches!(error, PackError::DigestMismatch { .. }));
    let text = error.to_string();
    assert!(text.contains(&"0".repeat(64)), "the declared one: {text}");
    assert!(
        text.contains("f89b7310b1d30e7b38a5a6cf9fe3e22330065c797dcf750e9cb8dc8554177041"),
        "and the real one: {text}"
    );
    // …and an edited *entry* moves the digest too, so content cannot drift silently.
    let edited = fixture::CORRIDOR_V2_JSON.replacen("\"base_s\": 43200", "\"base_s\": 43201", 1);
    assert!(matches!(
        load_pack(&edited).expect_err("edited content"),
        PackError::DigestMismatch { .. }
    ));
}

// ---- E7r2: P5 — the corridor doc 10 §11 actually names ---------------------

/// **The whole §11 corridor is authored**: a Ceres mining region *and* a freeport with
/// a refinery, a Phobos yard with a forge, a transfer node, a Mars orbital approach, a
/// Tharsis ground theater and a ground objective in it — joined by **two** routes.
///
/// Asserted through the seeded world rather than by reading the file, because the
/// claim is that the corridor *exists in a world*, not that a JSON file mentions it.
#[test]
fn e7r2_p5_the_seeded_v2_corridor_carries_the_whole_of_doc10_11() {
    let mut tw = boot_with(fixture::config_v2());
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");

    // The seven places, by the pack-local ids the fixture pins.
    assert_eq!(fixture::PLACES_V2.len(), 7);
    for place in fixture::PLACES_V2 {
        assert!(seeded(&tw, place), "the seed instantiates `{place}`");
    }
    // Both routes materialized, and the Mars leg reaches the ground theater.
    let ceres = route(&tw, fixture::ROUTE);
    let mars = route(&tw, fixture::ROUTE_MARS);
    assert_eq!(ceres.legs().len(), 2);
    assert_eq!(
        mars.legs().len(),
        2,
        "orbital approach, then the ground leg"
    );
    assert_ne!(
        mars.endpoints().0,
        mars.endpoints().1,
        "a route that starts where it ends is degenerate and refuses at load"
    );
    // The interception step of §11's proving story is authorable at all only because
    // E7 residue pass 1's R2 landed the fields — so v2 authors them.
    assert!(
        ceres.is_interdicted(),
        "the haul can be intercepted: §11's 'fight or evade an interception'"
    );
    assert!(mars.is_interdicted(), "and so can the deployment run");
    assert_eq!(
        ceres.controller().map(|id| id.as_str().to_owned()),
        Some("ceres_combine".to_owned()),
        "the corridor is held by somebody, which is what makes taking it mean something"
    );
    assert_eq!(
        mars.controller().map(|id| id.as_str().to_owned()),
        Some("mars_directorate".to_owned())
    );
}

/// **The whole fixture stays inside every bound it declares**, and the v2 seed inside
/// `MAX_SEED_OBJECTS`.
#[test]
fn e7r2_p5_the_v2_pack_is_inside_every_cap() {
    let v2 = load_pack(fixture::CORRIDOR_V2_JSON).expect("v2 loads");
    let entries = v2.entries().len();
    assert_eq!(
        entries, 15,
        "v1's six, plus P5's six, plus S7's three bodies"
    );
    assert!(entries <= fixture::MAX_FIXTURE_ENTRIES);
    assert!(entries <= MAX_PACK_ENTRIES);
    assert_eq!(MAX_PACK_ENTRIES, 512);
    let seeded = v2
        .entries()
        .values()
        .filter(|entry| entry.seeds_an_object())
        .count();
    assert_eq!(seeded, 7, "seven places");
    assert!(seeded <= MAX_SEED_OBJECTS);
    assert_eq!(MAX_SEED_OBJECTS, 256);
    // The whole fixture — core plus both corridor versions — is still under the
    // fixture's own declared ceiling.
    let core = load_pack(fixture::CORE_JSON).expect("core");
    let v1 = load_pack(fixture::CORRIDOR_JSON).expect("v1");
    assert!(
        core.entries().len() + v1.entries().len() + entries <= 64,
        "the fixture as a whole stays small enough to read"
    );
}

/// **The default composition is byte-identical to what it was**: `fixture::config()`
/// still seeds v1 and only v1, so a host that upgrades this binary and changes no
/// configuration seeds exactly what it seeded before.
///
/// This is the operational half of the versioning claim. A pass that quietly swapped
/// the built-in seed would re-resolve every place id in every running world on the
/// next boot, whatever the digest said.
#[test]
fn e7r2_p5_the_default_fixture_still_seeds_v1_and_only_v1() {
    let default = fixture::config();
    assert_eq!(default.seeds.len(), 1);
    assert_eq!(
        default.seeds[0].bytes,
        fixture::CORRIDOR_JSON,
        "the default seed is still v1, byte for byte"
    );
    let v2 = fixture::config_v2();
    assert_eq!(v2.seeds.len(), 1);
    assert_eq!(v2.seeds[0].bytes, fixture::CORRIDOR_V2_JSON);
    assert_eq!(
        v2.packs[0].bytes, default.packs[0].bytes,
        "and both ride the same unmoved `tacsim.core@1`"
    );
    // A world booted on the default still has exactly the three v1 places and one
    // route — nothing P5 authored leaks into it.
    let mut tw = boot_with(default);
    ok(&mut tw, GOD, "@tacsim/seed tacsim.corridor commit");
    for place in fixture::PLACES {
        assert!(seeded(&tw, place));
    }
    assert!(!seeded(&tw, "mars_theater"), "v1 knows nothing about Mars");
    assert!(
        snap(&tw)
            .route(&mush_core::CanonicalId::new(fixture::ROUTE_MARS).expect("route id"))
            .is_none()
    );
}
