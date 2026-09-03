//! MobilityProfile acceptance (M31): the pure-library mobility facet — the atom
//! vocabulary/registry, mode validation, the clock-resolved transition state
//! machine, capability queries, invariant enforcement, and deterministic serde.
//! All pure: methods take `now` (unix seconds); no Engine.

use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_tactics::{
    LocomotionId, MediumId, MobilityError, MobilityMode, MobilityModeId, MobilityProfile,
    MobilityRegistry, MobilityTag, MobilityVocabulary, ModeRules, ModeState,
};
use serde_json::json;

fn med(s: &str) -> MediumId {
    MediumId::new(s).unwrap()
}
fn loco(s: &str) -> LocomotionId {
    LocomotionId::new(s).unwrap()
}
fn tag(s: &str) -> MobilityTag {
    MobilityTag::new(s).unwrap()
}
fn mid(s: &str) -> MobilityModeId {
    MobilityModeId::new(s).unwrap()
}
fn base() -> MobilityRegistry {
    MobilityVocabulary::base().freeze()
}
fn mode_rules(id: &str) -> ModeRules {
    let mut definitions = DefinitionRegistryBuilder::new();
    let schema = definitions
        .register(
            OwnerId::new("tacsim").unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema").unwrap(),
                DefinitionId::new(id).unwrap(),
                1,
                json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    ModeRules::new(schema, json!({})).unwrap()
}
/// A mode over one medium with a `tacsim.kin` placeholder rules blob.
fn mode(id: &str, medium: &str, locomotion: &str) -> MobilityMode {
    MobilityMode::new(
        mid(id),
        [med(medium)],
        loco(locomotion),
        mode_rules("tacsim.kin"),
    )
}

// ----------------------------------------------------------------------

#[test]
fn base_vocabulary_declares_generic_atoms_only() {
    let reg = base();
    assert!(
        reg.has_medium(&med("ground"))
            && reg.has_medium(&med("water"))
            && reg.has_medium(&med("space"))
    );
    assert!(
        reg.has_locomotion(&loco("tracked"))
            && reg.has_locomotion(&loco("rotor"))
            && reg.has_locomotion(&loco("fixed_wing"))
    );
    assert!(reg.has_tag(&tag("amphibious")) && reg.has_tag(&tag("jump")));
    // Game content is NOT in the base vocabulary — the ruleset declares it.
    assert!(!reg.has_locomotion(&loco("battlemech")));
    assert!(!reg.has_tag(&tag("myomer_boost")));
}

#[test]
fn validate_mode_requires_declared_atoms_and_a_medium() {
    let reg = base();
    assert!(
        reg.validate_mode(&mode("ground.tracked", "ground", "tracked"))
            .is_ok()
    );

    // Undeclared atoms are refused, naming the offender.
    let bad_med = MobilityMode::new(mid("m"), [med("lava")], loco("tracked"), mode_rules("x"));
    assert_eq!(
        reg.validate_mode(&bad_med),
        Err(MobilityError::UndeclaredMedium(med("lava")))
    );
    let bad_loco = mode("m", "ground", "battlemech");
    assert_eq!(
        reg.validate_mode(&bad_loco),
        Err(MobilityError::UndeclaredLocomotion(loco("battlemech")))
    );
    let bad_tag = mode("m", "ground", "tracked").with_tags([tag("cloak")]);
    assert_eq!(
        reg.validate_mode(&bad_tag),
        Err(MobilityError::UndeclaredTag(tag("cloak")))
    );

    // A mode must name at least one medium.
    let no_med = MobilityMode::new(mid("m"), [], loco("tracked"), mode_rules("x"));
    assert_eq!(
        reg.validate_mode(&no_med),
        Err(MobilityError::NoMediums(mid("m")))
    );

    // A ruleset extends the vocabulary at construction, then the mode validates.
    let mut v = MobilityVocabulary::base();
    v.declare_locomotion(loco("battlemech"))
        .declare_tag(tag("myomer_boost"));
    let reg2 = v.freeze();
    let walker =
        mode("ground.biped", "ground", "battlemech").with_tags([tag("myomer_boost"), tag("jump")]);
    assert!(reg2.validate_mode(&walker).is_ok());
}

#[test]
fn profile_construction_enforces_intrinsic_invariants() {
    let reg = base();
    // Empty.
    assert_eq!(
        MobilityProfile::new(vec![], mid("x"), &reg).unwrap_err(),
        MobilityError::EmptyProfile
    );
    // Duplicate mode ids.
    let dup = MobilityProfile::new(
        vec![
            mode("g", "ground", "tracked"),
            mode("g", "ground", "wheeled"),
        ],
        mid("g"),
        &reg,
    );
    assert_eq!(dup.unwrap_err(), MobilityError::DuplicateMode(mid("g")));
    // Active mode not among the modes.
    let bad_active = MobilityProfile::new(vec![mode("g", "ground", "tracked")], mid("air"), &reg);
    assert_eq!(
        bad_active.unwrap_err(),
        MobilityError::UnknownMode(mid("air"))
    );
    // An invalid mode fails at construction (extrinsic).
    assert!(MobilityProfile::new(vec![mode("g", "ground", "nope")], mid("g"), &reg).is_err());

    // A valid single-mode profile starts Settled in its active mode.
    let p = MobilityProfile::new(vec![mode("g", "ground", "tracked")], mid("g"), &reg).unwrap();
    assert_eq!(p.effective_mode(0), Some(&mid("g")));
    assert!(matches!(p.mode_state(), ModeState::Settled(m) if m == &mid("g")));
}

fn transformer() -> MobilityProfile {
    // A wheeled-drone-that-briefly-flies: two modes, starts on the ground.
    let reg = base();
    MobilityProfile::new(
        vec![
            mode("ground.wheeled", "ground", "wheeled"),
            mode("air.rotor", "air", "rotor").with_tags([tag("vtol")]),
        ],
        mid("ground.wheeled"),
        &reg,
    )
    .unwrap()
}

#[test]
fn a_switch_resolves_virtually_off_the_clock() {
    let mut p = transformer();
    // Begin a 10s switch at t=100.
    p.begin_switch(&mid("air.rotor"), 100, 10).unwrap();
    // Mid-switch: effective mode is None (between modes); the switch is visible.
    assert!(p.is_switching(105));
    assert_eq!(p.effective_mode(105), None);
    assert_eq!(
        p.switch_parts(),
        Some((&mid("ground.wheeled"), &mid("air.rotor"), 110))
    );
    // At/after ready_at the target is effective — WITHOUT any settle call.
    assert_eq!(p.effective_mode(110), Some(&mid("air.rotor")));
    assert!(!p.is_switching(110));
    // A read never mutated the stored state (still Switching until settled).
    assert!(matches!(p.mode_state(), ModeState::Switching { .. }));

    // settle folds a DUE switch; it is idempotent and safe if the clock rewinds.
    p.settle(110);
    assert!(matches!(p.mode_state(), ModeState::Settled(m) if m == &mid("air.rotor")));
    p.settle(110);
    p.settle(0); // backward clock cannot un-settle
    assert_eq!(p.effective_mode(0), Some(&mid("air.rotor")));

    // settle on a not-yet-due switch is a no-op.
    let mut q = transformer();
    q.begin_switch(&mid("air.rotor"), 100, 10).unwrap();
    q.settle(105);
    assert!(matches!(q.mode_state(), ModeState::Switching { .. }));
}

#[test]
fn begin_switch_supersede_abort_noop_and_instant() {
    // Unknown target.
    let mut p = transformer();
    assert_eq!(
        p.begin_switch(&mid("orbit"), 0, 5).unwrap_err(),
        MobilityError::ModeNotAvailable(mid("orbit"))
    );
    // Switching to the already-settled mode is a no-op error.
    assert_eq!(
        p.begin_switch(&mid("ground.wheeled"), 0, 5).unwrap_err(),
        MobilityError::AlreadyInMode(mid("ground.wheeled"))
    );
    // dur 0 is an instant switch.
    p.begin_switch(&mid("air.rotor"), 0, 0).unwrap();
    assert!(matches!(p.mode_state(), ModeState::Settled(m) if m == &mid("air.rotor")));

    // Supersede: A->B mid-switch, then begin C -> A->C with a reset ready_at
    // (from stays the last COMMITTED mode, never the abandoned target).
    let mut q = MobilityProfile::new(
        vec![
            mode("a", "ground", "tracked"),
            mode("b", "air", "rotor"),
            mode("c", "water", "displacement"),
        ],
        mid("a"),
        &base(),
    )
    .unwrap();
    q.begin_switch(&mid("b"), 100, 10).unwrap();
    q.begin_switch(&mid("c"), 103, 10).unwrap();
    assert_eq!(q.switch_parts(), Some((&mid("a"), &mid("c"), 113)));

    // Abort: mid-switch back to `from` settles instantly.
    let mut r = transformer();
    r.begin_switch(&mid("air.rotor"), 100, 10).unwrap();
    r.begin_switch(&mid("ground.wheeled"), 105, 10).unwrap();
    assert!(matches!(r.mode_state(), ModeState::Settled(m) if m == &mid("ground.wheeled")));
}

#[test]
fn capability_queries_follow_the_effective_mode() {
    let reg = base();
    // An amphibian: ONE mode spanning ground AND water (multi-medium).
    let apc = MobilityMode::new(
        mid("amphibious"),
        [med("ground"), med("water")],
        loco("tracked"),
        mode_rules("tacsim.kin"),
    )
    .with_tags([tag("amphibious")]);
    let air = mode("air.rotor", "air", "rotor").with_tags([tag("vtol")]);
    let mut p = MobilityProfile::new(vec![apc, air], mid("amphibious"), &reg).unwrap();

    // Settled amphibian crosses both media in one mode (NOT derivable from a
    // single medium field).
    assert!(p.in_medium(&med("ground"), 0) && p.in_medium(&med("water"), 0));
    assert!(p.can_tag(&tag("amphibious"), 0));
    assert!(p.is_locomotion(&loco("tracked"), 0));
    assert!(!p.in_medium(&med("air"), 0));

    // Mid-switch, EVERY effective-mode query is false (the unit is between modes).
    p.begin_switch(&mid("air.rotor"), 100, 10).unwrap();
    assert!(
        !p.in_medium(&med("ground"), 105)
            && !p.can_tag(&tag("amphibious"), 105)
            && !p.can_tag(&tag("vtol"), 105)
    );
    // ...but the mode-EXPLICIT predicates ignore the transition, so a ruleset can
    // implement its own mid-switch policy.
    assert!(p.mode_in_medium(&mid("amphibious"), &med("water")));
    assert!(p.mode_has_tag(&mid("air.rotor"), &tag("vtol")));
    // After the switch completes the air mode's capabilities are in effect.
    assert!(p.can_tag(&tag("vtol"), 110) && p.in_medium(&med("air"), 110));
    assert!(!p.in_medium(&med("water"), 110));
}

#[test]
fn serde_round_trips_and_rejects_poisoned_documents() {
    let mut p = transformer();
    p.begin_switch(&mid("air.rotor"), 100, 10).unwrap();

    // A due-but-unsettled switch must be normalized before it crosses the wire.
    p.settle(200);
    let json = serde_json::to_value(&p).unwrap();
    let back: MobilityProfile = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, p, "profile round-trips field-for-field");

    // Intrinsic invariants are re-checked on deserialize: structurally broken
    // documents fail to load.
    let mut empty = json.clone();
    empty["modes"] = serde_json::json!([]);
    assert!(
        serde_json::from_value::<MobilityProfile>(empty).is_err(),
        "empty modes rejected"
    );
    let mut unknown = json.clone();
    unknown["mode"] = serde_json::json!({ "Settled": "nonesuch" });
    assert!(
        serde_json::from_value::<MobilityProfile>(unknown).is_err(),
        "active-not-in-modes rejected"
    );
    // A mode naming no medium is structurally broken and must not load.
    let mut no_med = json.clone();
    no_med["modes"][0]["mediums"] = serde_json::json!([]);
    assert!(
        serde_json::from_value::<MobilityProfile>(no_med).is_err(),
        "empty-mediums mode rejected"
    );
    // A Switching state naming a mode not in `modes` must not load — BOTH arms
    // (from and to), the only path that reaches those checks.
    let mut bad_from = json.clone();
    bad_from["mode"] = serde_json::json!({ "Switching": { "from": "nonesuch", "to": "air.rotor", "ready_at": 0 } });
    assert!(
        serde_json::from_value::<MobilityProfile>(bad_from).is_err(),
        "Switching.from not in modes rejected"
    );
    let mut bad_to = json;
    bad_to["mode"] = serde_json::json!({ "Switching": { "from": "ground.wheeled", "to": "nonesuch", "ready_at": 0 } });
    assert!(
        serde_json::from_value::<MobilityProfile>(bad_to).is_err(),
        "Switching.to not in modes rejected"
    );
}

#[test]
fn an_in_progress_switch_round_trips_over_serde() {
    // A profile serialized mid-flight (now < ready_at) stays Switching — the
    // Switching wire shape must survive a round-trip, not just Settled.
    let mut p = transformer();
    p.begin_switch(&mid("air.rotor"), 100, 10).unwrap();
    let json = serde_json::to_value(&p).unwrap();
    let back: MobilityProfile = serde_json::from_value(json).unwrap();
    assert_eq!(back, p, "in-progress switch round-trips field-for-field");
    assert_eq!(
        back.switch_parts(),
        Some((&mid("ground.wheeled"), &mid("air.rotor"), 110))
    );
    assert_eq!(back.effective_mode(105), None);
    assert_eq!(back.effective_mode(110), Some(&mid("air.rotor")));
}

#[test]
fn id_length_is_bounded_after_trim() {
    // 64 bytes is the limit (inclusive); 65 is rejected.
    assert!(MediumId::new("a".repeat(64)).is_ok());
    assert!(matches!(
        MediumId::new("a".repeat(65)).unwrap_err(),
        MobilityError::IdTooLong(_)
    ));
    // The bound applies AFTER trimming — surrounding whitespace does not count.
    assert!(MediumId::new(format!("  {}  ", "a".repeat(64))).is_ok());
}

#[test]
fn with_speeds_sets_fields_in_order_and_survives_serde() {
    let reg = base();
    let m = mode("g", "ground", "tracked").with_speeds(1500, 8);
    assert_eq!(
        (m.nominal_speed_cms, m.transition_s),
        (1500, 8),
        "builder args are not swapped"
    );
    let p = MobilityProfile::new(vec![m], mid("g"), &reg).unwrap();
    let back: MobilityProfile = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(
        (
            back.modes()[0].nominal_speed_cms,
            back.modes()[0].transition_s
        ),
        (1500, 8),
        "speeds survive serde"
    );
}

#[test]
fn ids_canonicalize_and_reject_malformed() {
    // Trim + lowercase → identical bytes → equal ids (stable hashing).
    assert_eq!(med("  Ground "), med("ground"));
    assert_eq!(mid("Air.Rotor"), mid("air.rotor"));
    // Both bare and namespaced forms are accepted.
    assert!(LocomotionId::new("legged").is_ok() && MobilityModeId::new("tacsim.walk").is_ok());
    // Malformed forms are rejected.
    assert_eq!(MediumId::new("   ").unwrap_err(), MobilityError::EmptyId);
    assert!(matches!(
        MediumId::new("bad char").unwrap_err(),
        MobilityError::MalformedId(_)
    ));
    assert!(matches!(
        MobilityModeId::new(".leading").unwrap_err(),
        MobilityError::MalformedId(_)
    ));
    assert!(matches!(
        MobilityModeId::new("double..dot").unwrap_err(),
        MobilityError::MalformedId(_)
    ));
    // Persisted/wire identities must already use their canonical bytes; serde
    // never silently rewrites an authoritative identifier.
    assert!(serde_json::from_value::<MediumId>(serde_json::json!("  WATER ")).is_err());
    assert_eq!(
        serde_json::from_value::<MediumId>(serde_json::json!("water")).unwrap(),
        med("water")
    );
    assert!(serde_json::from_value::<MediumId>(serde_json::json!("no spaces!")).is_err());
}

#[test]
fn transition_time_is_i64_and_does_not_overflow() {
    let mut p = transformer();
    let now = 4_000_000_000_i64; // beyond 2^31 — proves no u32 time truncation
    p.begin_switch(&mid("air.rotor"), now, 30).unwrap();
    assert_eq!(p.switch_parts().map(|(_, _, r)| r), Some(now + 30));
    assert_eq!(p.effective_mode(now + 29), None);
    assert_eq!(p.effective_mode(now + 30), Some(&mid("air.rotor")));
    // A pathological duration saturates rather than wrapping.
    let mut q = transformer();
    q.begin_switch(&mid("air.rotor"), i64::MAX - 5, u32::MAX)
        .unwrap();
    assert_eq!(q.switch_parts().map(|(_, _, r)| r), Some(i64::MAX));
}
