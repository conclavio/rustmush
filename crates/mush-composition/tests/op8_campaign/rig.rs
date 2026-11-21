//! The OP.8 rig: `driving`'s stocked world with the campaign seated beside it, and the clock
//! driven the way the stories need (jump to arrivals; a picket's bubble a wall second at a time).

use std::sync::Arc;

use mush_composition::campaign_formations::TacsimFormationAuthority;
use mush_composition::campaign_notice::defender_notice;
use mush_composition::campaign_operations::{CampaignLink, TacsimCloseProbe};
use mush_core::module::GameModule;
use mush_core::{AuthoritativeSubmit, CanonicalId, GOD, ObjId, World};
use tacsim::operations::OperationStatus;
use tacsim::space::SpaceBubbleBook;

use crate::driving_rig::{
    EXTRA_MODULES, ExtraModules, Rig, drain, ic_now, land, land_quietly, run, said, set_rate,
    stocked, sweep, to_ic,
};

/// An IC day.
pub(crate) const DAY: i64 = 86_400;

/// **The campaign as the composition root seats it** (`mush_composition::server_modules_with`):
/// TacSim's operation frame beside it, its close-only objectives answered by TacSim's probe, and
/// its defender notice queued and mailed.
fn campaign() -> Vec<Box<dyn GameModule>> {
    vec![Box::new(CampaignLink::new(
        mush_campaign::CampaignModule::new()
            .with_formation_kinds(Arc::new(TacsimFormationAuthority))
            .with_defender_notice(defender_notice())
            .with_close_probe(Arc::new(TacsimCloseProbe)),
    ))]
}

/// **The composed world at the production rate** (24:1, doc 50's own figure), with what the
/// stock rig lays up opened again: Ceres' hauling house and the Freeport arm, both open on a
/// production world. Staff's two lines are the rig's setup, before the run starts.
pub(crate) fn composed() -> Rig {
    let _ = EXTRA_MODULES.set(campaign as ExtraModules);
    let mut rig = stocked();
    assert!(
        tacsim::operations::frame(rig.tw.world()).is_some(),
        "the campaign seats TacSim's operation frame"
    );
    let said = run(&mut rig.tw, GOD, "@tacsim/port arm on");
    assert!(said.contains("Freeport arm: open"), "{said}");
    let said = run(&mut rig.tw, GOD, "@tacsim/port haulers ceres_port on");
    assert!(said.contains("ceres_port hauling house: open"), "{said}");
    set_rate(&mut rig.tw, 24);
    rig
}

/// Port fires (31 s apart), every submission they stage landed.
pub(crate) fn fires(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
}

/// The interdiction bubbles open now.
pub(crate) fn bubbles(w: &World) -> usize {
    w.module_state::<SpaceBubbleBook>()
        .map_or(0, |book| book.by_domain.len())
}

/// **The longest a picket's bubble is stepped**, wall seconds: heave-to, inspection and the
/// ruling take a few minutes.
const BUBBLE_S: usize = 1_800;

/// **A picket's encounter the run saw**: the carrier, and her inspection's status when last seen
/// with the bubble open.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Encounter {
    pub(crate) hull: ObjId,
    pub(crate) outcome: Option<String>,
}

/// The picket's bubbles open now: each carrier, and her inspection's status.
fn at_the_picket(w: &World) -> Vec<(ObjId, String)> {
    w.module_state::<SpaceBubbleBook>()
        .map(|book| {
            book.by_domain
                .values()
                .filter_map(|b| {
                    let picket = b.picket.as_ref()?;
                    Some((b.hull, format!("{:?}", picket.inspection_status)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// **One step of the run**: the IC clock `secs` on in one jump (an arrival the jump passes is
/// still met: a carrier is promoted at an interdicted leg's boundary and its leg paused there),
/// then any bubble stepped a wall second at a time until it closes, then two fires landed. The
/// picket's encounters it stepped through.
pub(crate) fn step(rig: &mut Rig, secs: i64) -> Vec<Encounter> {
    let now = ic_now(&rig.tw);
    let _ = to_ic(rig, now + secs);
    let mut met: Vec<Encounter> = Vec::new();
    for _ in 0..BUBBLE_S {
        for (hull, status) in at_the_picket(rig.tw.world()) {
            match met.iter_mut().find(|e| e.hull == hull) {
                Some(seen) => seen.outcome = Some(status),
                None => met.push(Encounter {
                    hull,
                    outcome: Some(status),
                }),
            }
        }
        if bubbles(rig.tw.world()) == 0 {
            break;
        }
        sweep(&mut rig.tw, 1);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
    fires(rig, 2);
    met
}

/// **Every queued submission a space story lands** (`voyage.rs`'s `drain_space_and_industry`):
/// the port's lanes, and industry's own where its receipt is a space draw.
pub(crate) fn drain_space(tw: &mut mush_testkit::TestWorld) -> Vec<AuthoritativeSubmit> {
    let mut kept = Vec::new();
    while let Some(submit) = tw.engine_mut().take_authoritative_submit() {
        let owner = submit.action.key().owner.as_str();
        if owner == tacsim::NS || owner == "inventory" || owner == "market" {
            kept.push(*submit);
        } else if owner == mush_industry::NS {
            let receipt = submit
                .parameters
                .get("receipt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if receipt.contains("kirkwood_gap_3_1") || receipt.contains("tacsim.space.") {
                kept.push(*submit);
            }
        }
    }
    kept
}

/// **A customs verb and its ruling** (`voyage.rs`'s `customs_ruling`, doc 35 cut SF.R26c): the
/// submissions queued before it set aside, the ruling landed, the rest queued again, and one
/// space tick to apply it. What `who` heard.
pub(crate) fn customs_ruling(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let mut earlier = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        earlier.push(submit);
    }
    let mut heard = run(&mut rig.tw, who, line);
    let mut filed = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        filed.push(submit);
    }
    for submit in filed {
        if submit.action.key().id.as_str() == tacsim::space::CUSTOMS_ACTION {
            land(rig, &submit).expect("the customs ruling lands");
        } else {
            earlier.push(submit);
        }
    }
    for submit in earlier {
        let _ = rig.tw.engine_mut().queue_authoritative_submit(submit);
    }
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    heard.push('\n');
    heard.push_str(&said(&rig.tw, &events, who));
    heard
}

/// A canonical id.
pub(crate) fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

/// The operation now, through the frame.
pub(crate) fn status(w: &World) -> Option<OperationStatus> {
    tacsim::isidis_op::status(w)
}

/// Ceres' house hulls.
pub(crate) fn house_hulls(rig: &Rig) -> Vec<ObjId> {
    crate::driving_rig::store(&rig.tw)
        .hauler(&cid("ceres_port"))
        .map(|house| {
            house
                .rows
                .iter()
                .filter_map(|row| row.hull.map(ObjId))
                .collect()
        })
        .unwrap_or_default()
}
