//! **Doc 23 cut S.4b-T — the testimony producer** (§17.8, §17.10; D-23-19, Q-23-16, Q-23-17),
//! over the composed world (`driving`'s keyed rig), the arena's egress pumped by hand as
//! `field_raiders.rs` pumps it, at both site sizes: Ahuna Flank (256) and Urvara Reach (512).
//!
//! The content's raider is seeded and launched beside a prospector's rig and marks it Hostile;
//! the field opens, and the rig's own sensors tell the raider's hulls apart. Each chunk of the
//! field's own log feeds the earned-contacts index as it lands: the rig's operator (its party's
//! leader, the seal's warrant principal, since nobody sits in a rig) earns every raider hull the
//! rig classified by its own pass, and nothing it only detected; a bystander earns nothing.
//! The ring is written in the chunk's bundle, and a restart mid-field reads it back.
//!
//! The negative half that a live field cannot stage on demand — a party-mate holding the contact
//! only over the mesh (a field's parties are each one unit, or one raider), a rig that only
//! detected — is `tacsim::field::testimony`'s own world test over the same producer.

use std::collections::{BTreeMap, BTreeSet};

use crate::driving_rig::{crash, pilot};
use crate::field_raiders_rig::*;

/// Every (viewer, target) the log shows: told apart by the viewer's own pass, held over the
/// mesh (a classification with no draw), and only detected.
#[derive(Debug, Default)]
struct Seen {
    own: BTreeSet<(ObjId, ObjId)>,
    mesh: BTreeSet<(ObjId, ObjId)>,
    detected: BTreeSet<(ObjId, ObjId)>,
}

fn seen(lines: &[LedgerEvent]) -> Seen {
    let mut seen = Seen::default();
    for line in lines {
        match &line.kind {
            EventKind::Classified { viewer, target, .. } => {
                let label = format!("classify:{}:{}:", viewer.0, target.0);
                if line
                    .draws
                    .iter()
                    .any(|(drawn, _)| drawn.starts_with(&label))
                {
                    seen.own.insert((*viewer, *target));
                } else {
                    seen.mesh.insert((*viewer, *target));
                }
            }
            EventKind::Detected { viewer, target, .. } => {
                seen.detected.insert((*viewer, *target));
            }
            _ => {}
        }
    }
    seen
}

/// Each admitted unit's author: its admission pilot, else its party's leader.
fn authors(live: &tacsim::field::FieldLive) -> BTreeMap<ObjId, ObjId> {
    live.units
        .iter()
        .filter_map(|row| {
            let author = row.pilot.as_ref().map(|(pilot, _)| *pilot).or_else(|| {
                live.parties
                    .iter()
                    .find(|party| party.party == row.party)
                    .and_then(|party| party.leader)
            })?;
            Some((row.unit, author))
        })
        .collect()
}

/// `field_raiders`' fast field inside the layer's checked bounds: the mind commerce's
/// preparation validates the config as production does (`substep_max_ms <= engaged_ms`, a
/// snapshot every 1 to 60 s), which the plain rig's boot never asks.
fn claim_tune(cfg: &mut TacsimConfig) {
    tune(cfg);
    cfg.arena.tempo.substep_max_ms = cfg.arena.tempo.engaged_ms;
    cfg.arena.snapshot_ms = 1_000;
}

fn earned(rig: &Rig, author: ObjId, hull: ObjId) -> bool {
    tacsim::minds::earned_contacts::has_earned(rig.tw.world(), author, hull)
}

/// **`@mind/claim` through its command path**: the reply and how many claims it queued.
fn claim(rig: &mut Rig, who: ObjId, hull: ObjId) -> (String, usize) {
    let (out, queued) = command(
        rig,
        who,
        &format!("@mind/claim #{} Its gait was odd.", hull.0),
    );
    let claims = queued
        .iter()
        .filter(|submit| {
            submit.action.key().id.as_str()
                == tacsim::minds::registry_inspection::claims::CLAIM_ACTION
        })
        .count();
    (out, claims)
}

/// **The story at one size**: the Registry's claim channel composed, the field opens over
/// `operator`'s rig `tv` and the raider, runs until the rig has told a raider hull apart, and
/// then the index is held against the log and `@mind/claim` is asked; a restart mid-field keeps
/// both.
async fn story(rig: Rig, site: &str, operator: ObjId, tv: ObjId, raiders: &[ObjId]) {
    let registry = {
        let mut rig = rig;
        // Each on a gateway account of their own, as every player is (`@mind` asks it).
        let registry = pilot(&mut rig, "Registry operator", 41);
        let bystander = pilot(&mut rig, "Bystander", 42);
        (
            crate::claim_rig::with_claims(rig, registry, &[operator, registry, bystander]),
            bystander,
        )
    };
    let (mut rig, bystander) = registry;
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    // The control: the claim channel answers, and every hull is the observer rule's sentence.
    let (out, queued) = claim(&mut rig, operator, raiders[0]);
    assert_eq!(
        (out.as_str(), queued),
        (tacsim::surface::hull::NO_BOX, 0),
        "before any field"
    );
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        second(&mut rig);
    }
    // Before any field, nobody earned anything.
    for hull in raiders {
        assert!(!earned(&rig, operator, *hull), "no field, no testimony");
    }
    let (out, queued) = command(&mut rig, leader, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens on {site}"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(site)).expect("the live record");
    let by = authors(&live);
    assert_eq!(
        by.get(&tv),
        Some(&operator),
        "nobody sits in a rig: its operator"
    );
    let classified = until(&mut rig, &mut rx, &mut lines, 60, |_, lines| {
        seen(lines)
            .own
            .iter()
            .any(|(viewer, target)| *viewer == tv && raiders.contains(target))
    })
    .await;
    let log = seen(&lines);
    eprintln!(
        "S4BT {site}: own {:?} mesh {:?} detected {:?}",
        log.own, log.mesh, log.detected
    );
    assert!(classified, "the rig tells a raider hull apart on {site}");

    // The index is the log's own-sensor lines, by author, and nothing else.
    let check = |rig: &Rig, when: &str| {
        let mut held = 0;
        for (unit, author) in &by {
            for target in live.units.iter().map(|row| row.unit) {
                let own = log
                    .own
                    .iter()
                    .any(|(viewer, seen)| *seen == target && by.get(viewer) == Some(author));
                let has = earned(rig, *author, target);
                assert!(
                    !own || has,
                    "{when}: #{}'s author #{} told #{} apart and did not earn it",
                    unit.0,
                    author.0,
                    target.0
                );
                if has {
                    assert!(
                        log.own
                            .iter()
                            .any(|(viewer, seen)| *seen == target && by.get(viewer) == Some(author)),
                        "{when}: #{} earned #{} with no own-sensor line (mesh {}, detected {})",
                        author.0,
                        target.0,
                        log.mesh.iter().any(|(_, seen)| *seen == target),
                        log.detected.iter().any(|(_, seen)| *seen == target)
                    );
                    held += 1;
                }
            }
        }
        for hull in raiders {
            assert!(
                !earned(rig, bystander, *hull),
                "{when}: the bystander saw nothing"
            );
        }
        assert!(held > 0, "{when}: something was earned");
    };
    // The claim's observer rule over the index: the hull the rig told apart passes it (the
    // operator holds no charter title, so the claim stops at the driver's one sentence, past
    // the observer rule); a hull the operator's units never told apart, and every hull the
    // bystander names, are refused with the observer rule's sentence. Nothing is queued.
    let told: ObjId = *raiders
        .iter()
        .find(|hull| log.own.contains(&(tv, **hull)))
        .expect("a raider hull the rig told apart");
    let unseen: ObjId = live
        .units
        .iter()
        .map(|row| row.unit)
        .find(|unit| {
            !log.own
                .iter()
                .any(|(viewer, seen)| seen == unit && by.get(viewer) == Some(&operator))
        })
        .expect("a hull the operator's units never told apart");
    let asks = |rig: &mut Rig, when: &str| {
        let (out, queued) = claim(rig, operator, told);
        assert_ne!(
            out,
            tacsim::surface::hull::NO_BOX,
            "{when}: the rig told #{} apart",
            told.0
        );
        assert_eq!(
            (out.as_str(), queued),
            (mush_core::action::SUBMISSION_REFUSED_LINE, 0),
            "{when}: past the observer rule, the charter's refusal"
        );
        let (out, queued) = claim(rig, operator, unseen);
        assert_eq!(
            (out.as_str(), queued),
            (tacsim::surface::hull::NO_BOX, 0),
            "{when}: #{} was never told apart by the operator's units",
            unseen.0
        );
        for hull in raiders {
            let (out, queued) = claim(rig, bystander, *hull);
            assert_eq!(
                (out.as_str(), queued),
                (tacsim::surface::hull::NO_BOX, 0),
                "{when}: the bystander saw nothing"
            );
        }
    };
    check(&rig, "in the field");
    asks(&mut rig, "in the field");
    // A restart mid-field: the ring was written in the chunk's bundle and reads back.
    let mut rig = crash(rig, &mut rx, &[operator, bystander]).await;
    check(&rig, "after a restart");
    asks(&mut rig, "after a restart");
}

/// **S.4b-T at 256** (Ahuna Flank): the prospector's rig works east of the pad and the content's
/// raider launches from its own carrier and marks it Hostile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s4bt_a_rig_earns_what_it_told_apart_in_a_field_at_256() {
    let mut rig = stocked_tuned(claim_tune);
    let (a, _row, tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, a, tvs[0]);
    to_cell(&mut rig, tvs[0], (190, 126));
    warm(&mut rig);
    let (raiders, _carrier) = seed_and_launch(&mut rig);
    story(rig, AHUNA, a, tvs[0], &raiders).await;
}

/// **S.4b-T at 512** (Urvara Reach): the same, with the raider rebased on the 512 site
/// (`field_raiders.rs`'s S.1 setup).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s4bt_a_rig_earns_what_it_told_apart_in_a_field_at_512() {
    const URVARA: &str = "urvara_port";
    let mut rig = stocked_tuned(claim_tune);
    let (a, _row, tvs) = on_pad_at(&mut rig, "Pilot", 1, "Salt Moth", "urvara", URVARA);
    let tv = tvs[0];
    let _ = act_go(&mut rig, a, "+tv/1 launch go");
    let _ = finish(&mut rig, tv, a);
    to_cell_on(&mut rig, URVARA, tv, (300, 257));
    let mut def = tacsim::field::raiders::raiders()
        .expect("the fixture raiders")
        .into_iter()
        .find(|def| def.id.as_str() == SKULKS)
        .expect("the content's raider");
    def.id = cid("tacsim.raider.urvara_s4bt");
    def.site = cid(URVARA);
    let seeded = staff_hand(&mut rig, |ctx| tacsim::field::raiders::seed(ctx, &def));
    assert!(
        seeded.as_deref().is_ok_and(|s| s.contains("seeded")),
        "{seeded:?}"
    );
    let launched = staff_hand(&mut rig, |ctx| tacsim::field::raiders::launch(ctx, &def));
    assert!(launched.is_ok(), "{launched:?}");
    let raiders = raider_hulls(&rig);
    assert_eq!(raiders.len(), 2, "two hulls launched");
    story(rig, URVARA, a, tv, &raiders).await;
}
