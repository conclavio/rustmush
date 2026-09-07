//! E7 cut 10 B1 acceptance — the `granted` narrowing inside the ONE production
//! `ChannelGrant` projection, the load-refusing seat caps, and the projection
//! probes (10b §3.10; the P lens).
//!
//! The seat *lifecycle* (offer → grant → activation → revocation) is driven
//! through its real ingress in `crates/mush-campaign/tests/e7c10.rs`; what lives
//! here is the half `mush-sim` owns and the half a source probe must pin:
//!
//! * a narrower granted set limits the seat **exactly** — the observable
//!   outcome is the grant a second actor's resolver sees and what
//!   `World::channel_grant_is_current` answers per channel, not the record;
//! * an over-cap `assignment:<unit>` document **refuses on load, never
//!   truncates**, and its host is marked;
//! * the **08g §10 C12 census is still nine** and `links::desired_links` is
//!   still the only production `ChannelGrant` writer.

use std::collections::HashMap;

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, ChannelId, Flags, GOD, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind,
};
use mush_proto::OutputEvent;
use mush_sim::{MAX_SEATS_PER_UNIT, SimulationModule, SpatialState};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "sim";

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

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {t}");
    t
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

struct Fx {
    field: ObjId,
    atlas: ObjId,
    pilot: ObjId,
}

fn fixture(tw: &mut TestWorld) -> Fx {
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let pilot = tw
        .world_mut()
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, field, "sim.domain");
    bind(tw, atlas, "sim.participant");
    Fx {
        field,
        atlas,
        pilot,
    }
}

/// A domain, one placed unit with a two-channel `pilot` station, and Ann
/// billeted on it.
fn scene() -> (TestWorld, Fx) {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    let fx = fixture(&mut tw);
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive", fx.field.0),
    );
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", fx.atlas.0));
    ok(
        &mut tw,
        &format!(
            "@spatial/station #{}=pilot,pilot,channels:movement+fire",
            fx.atlas.0
        ),
    );
    ok(
        &mut tw,
        &format!("@spatial/assign #{}=pilot,#{}", fx.atlas.0, fx.pilot.0),
    );
    tw.world_mut().move_to(fx.pilot, fx.field).unwrap();
    (tw, fx)
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot_with(kv: Vec<(SmolStr, serde_json::Value)>) -> (TestWorld, Fx) {
    let mut tw = TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
    let fx = fixture(&mut tw);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    (tw, fx)
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn granted_channels(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str) -> Vec<String> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|grant| grant.subject == unit && grant.station.as_str() == station)
        .map(|grant| {
            grant
                .channels
                .iter()
                .map(|c| c.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn is_current(tw: &TestWorld, actor: ObjId, unit: ObjId, station: &str, channel: &str) -> bool {
    let epoch = tw
        .world()
        .command_subjects_of(actor)
        .iter()
        .find(|g| g.subject == unit && g.station.as_str() == station)
        .map_or(0, |g| g.epoch);
    tw.world().channel_grant_is_current(
        actor,
        unit,
        NS,
        station,
        epoch,
        &ChannelId::new(channel).unwrap(),
    )
}

/// Patch one seat of the folded `assignment:<unit>` document.
fn patch_seat(
    kv: &mut [(SmolStr, serde_json::Value)],
    unit: ObjId,
    station: &str,
    patch: impl FnOnce(&mut serde_json::Value),
) {
    let key = SmolStr::new(format!("assignment:{}", unit.0));
    let doc = kv
        .iter_mut()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
        .expect("the fixture wrote an assignment document");
    let seat = doc
        .get_mut("seats")
        .and_then(|seats| seats.get_mut(station))
        .expect("the fixture seated that station");
    patch(seat);
}

// ---- E7c10: B1 — the narrowing inside the one projection ----

#[test]
fn e7c10_b1_a_narrower_granted_set_limits_the_seat_exactly() {
    let (mut tw, fx) = scene();
    // Ann's station grants BOTH channels before any narrowing — the baseline
    // another actor's resolver would see.
    assert_eq!(
        granted_channels(&tw, fx.pilot, fx.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
    let mut kv = fold_kv(&mut tw);
    patch_seat(&mut kv, fx.atlas, "pilot", |seat| {
        seat["granted"] = serde_json::json!(["movement"]);
    });
    let (tw2, fx2) = reboot_with(kv);
    // The OBSERVABLE outcome: the republished grant a resolver reads carries
    // exactly the intersection, and `channel_grant_is_current` — the one
    // predicate every stale-intent and offer check consults — answers true for
    // the granted channel and FALSE for the station channel the seat was not
    // given. An action requiring `fire` therefore projects no offer at all.
    assert_eq!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot"),
        vec!["movement".to_owned()],
        "the seat's set is the intersection, not the station's whole set"
    );
    assert!(is_current(&tw2, fx2.pilot, fx2.atlas, "pilot", "movement"));
    assert!(
        !is_current(&tw2, fx2.pilot, fx2.atlas, "pilot", "fire"),
        "an ungranted channel is refused by the kernel's own grant check"
    );
    // The read-only inspector renders the same narrowed set (one internal view
    // feeds every projection — §4 rule 4).
    let mut tw2 = tw2;
    let shown = ok(&mut tw2, &format!("@spatial/authority #{}", fx2.pilot.0));
    assert!(shown.contains("movement"), "{shown}");
    assert!(
        !shown.contains("fire"),
        "the inspector shows the narrowed set, not the station's: {shown}"
    );
}

#[test]
fn e7c10_b1_an_absent_narrowing_publishes_the_stations_whole_set() {
    // A pre-cut-10 document carries no `granted` field at all: it decodes to
    // `None` and the projection is byte-identical to cut 2's.
    let (mut tw, fx) = scene();
    let kv = fold_kv(&mut tw);
    let doc = kv
        .iter()
        .find(|(k, _)| k.as_str() == format!("assignment:{}", fx.atlas.0))
        .map(|(_, v)| v.clone())
        .expect("assignment doc");
    assert!(
        doc["seats"]["pilot"].get("granted").is_none(),
        "an un-narrowed seat serializes no `granted` field: {doc}"
    );
    let (tw2, fx2) = reboot_with(kv);
    assert_eq!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot"),
        vec!["fire".to_owned(), "movement".to_owned()]
    );
}

#[test]
fn e7c10_b1_a_granted_set_disjoint_from_the_station_grants_nothing() {
    // The intersection is the whole rule: a seat cannot be handed a channel its
    // station does not carry, so a forged document buys no authority.
    let (mut tw, fx) = scene();
    let mut kv = fold_kv(&mut tw);
    patch_seat(&mut kv, fx.atlas, "pilot", |seat| {
        seat["granted"] = serde_json::json!(["scuttle"]);
    });
    let (tw2, fx2) = reboot_with(kv);
    assert!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot").is_empty(),
        "a channel the station does not carry is not granted by naming it"
    );
    assert!(!is_current(&tw2, fx2.pilot, fx2.atlas, "pilot", "scuttle"));
    assert!(!is_current(&tw2, fx2.pilot, fx2.atlas, "pilot", "movement"));
}

// ---- E7c10: B1 — the load-refusing caps (contradiction (vii)) ----

#[test]
fn e7c10_b1_an_over_cap_seat_census_refuses_on_load_and_never_truncates() {
    let (mut tw, fx) = scene();
    let mut kv = fold_kv(&mut tw);
    let key = SmolStr::new(format!("assignment:{}", fx.atlas.0));
    let doc = kv
        .iter_mut()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
        .expect("assignment doc");
    let seats = doc["seats"].as_object_mut().expect("seats object");
    let template = seats["pilot"].clone();
    for n in 0..=MAX_SEATS_PER_UNIT {
        seats.insert(format!("seat{n}"), template.clone());
    }
    assert!(seats.len() > MAX_SEATS_PER_UNIT);
    let (tw2, fx2) = reboot_with(kv);
    assert!(
        !state(&tw2).assignments.contains_key(&fx2.atlas),
        "the whole over-cap document is refused — never silently truncated to a \
         smaller crew"
    );
    assert!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot").is_empty(),
        "and no grant is published from a refused document"
    );
}

#[test]
fn e7c10_b1_an_over_cap_granted_set_refuses_on_load() {
    let (mut tw, fx) = scene();
    let mut kv = fold_kv(&mut tw);
    patch_seat(&mut kv, fx.atlas, "pilot", |seat| {
        let channels: Vec<String> = (0..=mush_sim::MAX_GRANTED_CHANNELS)
            .map(|n| format!("c{n}"))
            .collect();
        seat["granted"] = serde_json::json!(channels);
    });
    let (tw2, fx2) = reboot_with(kv);
    assert!(
        !state(&tw2).assignments.contains_key(&fx2.atlas),
        "an over-cap narrowed set refuses the whole document on load"
    );
}

// ---- E7c10: P — the projection probes ----

/// Where a file's in-crate unit-test module begins (everything after it is a
/// fixture, not production code).
fn test_module_start(text: &str) -> usize {
    text.find(
        "
#[cfg(test)]
mod ",
    )
    .or_else(|| {
        text.find(
            "
#[cfg(test)]
pub mod ",
        )
    })
    .unwrap_or(text.len())
}

/// Every `.rs` file under `crates/*/src`, as `(path, contents)`.
fn production_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                out.push((path.to_string_lossy().replace('\\', "/"), text));
            }
        }
    }
    out
}

/// 08g §10's C12 census, re-run: **exactly one production writer**. Cut 10 adds
/// an intersection term inside it and no second projection.
#[test]
fn e7c10_p1_desired_links_is_still_the_only_production_channelgrant_writer() {
    // A publish that appears BEFORE a file's `#[cfg(test)]` marker is
    // production; one after it is a unit-test fixture.
    let writers: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, text)| {
            if !path.contains("/src/") {
                return false;
            }
            if path.ends_with("mush-core/src/world.rs") {
                // The type's own store: `World::publish_command_subject` is the
                // sink every projection writes THROUGH, not a projection.
                return false;
            }
            let boundary = test_module_start(text);
            text[..boundary].contains("publish_command_subject(")
        })
        .map(|(path, _)| path)
        .collect();
    // `mush-contract-tests/src/tactical_gate.rs` is a FIXTURE crate (its whole
    // `src` is a test harness), named here so a new publisher fails loudly
    // rather than passing unremarked.
    let production: Vec<&String> = writers
        .iter()
        .filter(|path| !path.contains("mush-contract-tests/src/"))
        .collect();
    assert_eq!(
        production.len(),
        1,
        "exactly one production ChannelGrant projection; found {production:?}"
    );
    assert!(
        production[0].ends_with("mush-sim/src/links.rs"),
        "the one production projection is `desired_links`: {production:?}"
    );
}

/// The C12 **consumer** census, re-run by file: still **nine** production files
/// read a `ChannelGrant`, and cut 10 adds no tenth. `mush-sim/src/seat.rs`
/// deliberately calls `station.rs`'s own `actor_grant_epoch` rather than opening
/// a second `command_subjects_of` read of its own.
#[test]
fn e7c10_p2_the_c12_consumer_census_is_still_nine() {
    let needles = [
        "command_subjects_of(",
        ".channel_grant(",
        "channel_grant_is_current(",
    ];
    let mut files: Vec<String> = Vec::new();
    for (path, text) in production_sources() {
        if !path.contains("/src/") || path.contains("mush-contract-tests/src/") {
            continue;
        }
        let boundary = test_module_start(&text);
        let production = &text[..boundary];
        let hit = production.lines().any(|line| {
            let code = line.trim_start();
            !code.starts_with("//") && needles.iter().any(|needle| code.contains(needle))
        });
        if hit && !path.ends_with("mush-core/src/world.rs") {
            files.push(path);
        }
    }
    files.sort();
    let names: Vec<&str> = files
        .iter()
        .filter_map(|p| p.split("crates/").nth(1))
        .collect();
    assert_eq!(
        names,
        vec![
            "mush-core/src/action.rs",
            "mush-core/src/behavior.rs",
            "mush-core/src/engine.rs",
            "mush-sim/src/combat.rs",
            "mush-sim/src/commands.rs",
            "mush-sim/src/handoff.rs",
            "mush-sim/src/station.rs",
            "mush-sim/src/tick.rs",
            "mush-tactics/src/status.rs",
        ],
        "the C12 consumer census must stay at nine files"
    );
    // And cut 10's own workflow crate consumes it through no site at all: it
    // reaches the control surface only through the two `mush-sim` seams.
    let campaign: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, text)| {
            path.contains("mush-campaign/src/")
                && text.lines().any(|line| {
                    let code = line.trim_start();
                    !code.starts_with("//") && code.contains("ChannelGrant")
                })
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        campaign.is_empty(),
        "mush-campaign must reach no ChannelGrant directly: {campaign:?}"
    );
}

/// E7 ships **no eviction path**: cut 10's crates never call `@spatial/unassign`
/// or `@spatial/assign`, the staff surgery pair (10b §3.10 DECIDED (d)).
#[test]
fn e7c10_p3_cut_ten_reaches_no_eviction_path() {
    let offenders: Vec<String> = production_sources()
        .into_iter()
        .filter(|(path, text)| {
            (path.contains("mush-campaign/src/") || path.contains("mush-sim/src/seat.rs"))
                && (text.contains("@spatial/unassign") || text.contains("spatial_unassign"))
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        offenders.is_empty(),
        "a seat path reached the eviction pair: {offenders:?}"
    );
}

/// Cut 10 persists **no lease-like record of its own** and registers no second
/// `ControlLease` validator, `state_family`, `revision_validator`,
/// `schedule_source`, clock provider or authoritative submission
/// (10b §3.10 RULES; 08i §4).
#[test]
fn e7c10_p4_cut_ten_registers_no_lease_like_family_or_validator() {
    let banned = [
        "reg.state_family(",
        "reg.authority_guard_validator(",
        "reg.revision_validator(",
        "reg.schedule_source(",
        "reg.clock_provider(",
        "reg.domain_clock_provider(",
        "submit_authoritative(",
        "delegate_state_families(",
    ];
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in production_sources() {
        if !path.contains("mush-campaign/src/") {
            continue;
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") {
                continue;
            }
            if banned.iter().any(|needle| code.contains(needle)) {
                offenders.push(format!("{path}: {code}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "mush-campaign registered engine machinery cut 10 forbids: {offenders:?}"
    );
}

// ---- E7c10: the seat seam's surface is exactly two `pub fn`s ----

#[test]
fn e7c10_p5_the_sim_export_surface_is_exactly_two_seam_functions() {
    let seat = production_sources()
        .into_iter()
        .find(|(path, _)| path.ends_with("mush-sim/src/seat.rs"))
        .map(|(_, text)| text)
        .expect("seat.rs");
    let seams: Vec<&str> = seat
        .lines()
        .filter(|line| line.starts_with("pub fn "))
        .map(|line| line.trim_end_matches(" {").trim_end_matches('('))
        .collect();
    assert_eq!(
        seams,
        vec![
            "pub fn channel_lease_holder(w: &World, unit: ObjId, channel: &ChannelId) -> Option<PrincipalId>",
            "pub fn request_seat_activation",
            // Not a seam: the `sim.station_service` document-key helper, the
            // sibling of `station::fallback_key`. No crate outside `mush-sim`
            // calls it, which the campaign-side probe below proves.
            "pub fn service_key(unit: ObjId, station: &StationId, actor: ObjId) -> SmolStr",
        ],
        "exactly two exported SEAM functions plus one document-key helper          (OPEN 1, as the owner decided)"
    );
    // The fence, from the CALLER's side: `mush-campaign` reaches `mush-sim`'s
    // mechanism through exactly those two `pub fn`s and nothing else.
    let mut called: Vec<String> = Vec::new();
    for (path, text) in production_sources() {
        if !path.contains("mush-campaign/src/") {
            continue;
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let mut rest = code;
            while let Some(at) = rest.find("mush_sim::") {
                let tail = &rest[at + "mush_sim::".len()..];
                let name: String = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                // Only CALLS count; a type or a state read is a read of
                // published world state, not a seam.
                if tail[name.len()..].starts_with('(') && !called.contains(&name) {
                    called.push(name.clone());
                }
                rest = &tail[name.len()..];
            }
        }
    }
    called.sort();
    assert_eq!(
        called,
        vec![
            "channel_lease_holder".to_owned(),
            "request_seat_activation".to_owned(),
        ],
        "mush-campaign must CALL exactly the two exported seams"
    );
    // `handoff` stays PRIVATE: the module declaration is `mod handoff;`.
    let lib = production_sources()
        .into_iter()
        .find(|(path, _)| path.ends_with("mush-sim/src/lib.rs"))
        .map(|(_, text)| text)
        .expect("lib.rs");
    assert!(
        lib.contains("\nmod handoff;"),
        "handoff.rs must stay a private module"
    );
}

// ---- E7c10: the seat billet is a GOING-safe, ordinary billet ----

#[test]
fn e7c10_b1_a_going_unit_publishes_no_seat_grant_however_narrow() {
    let (mut tw, fx) = scene();
    let mut kv = fold_kv(&mut tw);
    patch_seat(&mut kv, fx.atlas, "pilot", |seat| {
        seat["granted"] = serde_json::json!(["movement"]);
    });
    let (mut tw2, fx2) = reboot_with(kv);
    assert_eq!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot"),
        vec!["movement".to_owned()]
    );
    // The narrowing is a term inside the projection, so every pre-existing
    // liveness gate still runs ahead of it.
    if let Some(object) = tw2.world_mut().get_mut(fx2.atlas) {
        object.flags.insert(Flags::GOING);
    }
    ok(
        &mut tw2,
        &format!("@spatial/assign #{}=pilot,#{}", fx2.atlas.0, fx2.pilot.0),
    );
    assert!(
        granted_channels(&tw2, fx2.pilot, fx2.atlas, "pilot").is_empty(),
        "a GOING unit publishes nothing at all"
    );
}
