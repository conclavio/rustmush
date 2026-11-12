//! **A detachment's roster flown into a field, in tacsim's own world harness** (doc 44 unit
//! MW.15; the hook doc 36 cut GC.0 named: `driving_rig::land_composed`).
//!
//! Until this rig no tacsim test ran a DT roster to its field: the field stories stood a hired
//! NPC in a hull by a staff hand (`field_raiders_rig::seat_npc`) and left the delivered orders to
//! the split-boundary harness in `mush-server`. [`fly`] is that harness's `pair_field_fixture`
//! on the composed world of `driving_rig`, every step the production command:
//!
//! 1. at Ceres the organiser hires the shipped crew in the Slack Line (`+sortie/crew`, then the
//!    `+sortie/hire` line it prints), leases one prospector a slot (`+tv/lease go`), takes each
//!    rig's operational loans (`+tv/<n>/permissions go`) and brings the crew aboard
//!    (`+sortie/board`);
//! 2. the hull sails to the site and settles on its pad (`driving_rig::on_pad_with_at`);
//! 3. the organiser lends each NPC its rig (`@unitgrant/issue … operate_crew parent=<loan>`);
//! 4. a player leader boards their own rig in the bay (`+tv/<n> board`, `+focus`);
//! 5. the plan is drafted (`+sortie/draft`), accepted and made ready by its player
//!    (`+sortie/accept`, `@station/handoff`, `+sortie/ready`), and activated
//!    (`+sortie/activate`);
//! 6. each NPC is seated (`+sortie/seat`), the roster launched (`+sortie/launch`) and the field
//!    command refreshed (`+sortie/refresh`) until it names a leader.
//!
//! Every submission is landed on the world task's composed path, whoever owns its action
//! (tacsim's, Org's unit grants, Sim's helm hand-off), and the scheduled `control_handoff_apply`
//! on the composed scheduled path ([`apply_due`]), as production's coordinator lands them.
//!
//! Every item is `pub(crate)`; a domain root declares this module by path beside the other
//! `*_rig` modules.
#![allow(dead_code)]

use mush_core::{AuthoritativeSubmit, CommitToken, ObjId, ObjKind};
use mush_tactics::detachment::DetachmentRoster;
use mush_tactics::detachment::planning::{Draft, PlanId};
use serde_json::json;
use tacsim::detachment::sortie::{Flight, FlightKey, OperationId, OperationRecord, Phase};

use crate::driving_rig::{
    HullRow, Rig, at_counter, fold_module_ops_into, keep, land_composed, next_token,
    on_pad_with_at, run, support, teleport, typed,
};
use crate::grounds_rig::Ground;

/// Who flies one slot of the roster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Billet {
    /// The organiser, a player, in their own rig.
    Leader,
    /// A shipped crew member, by name (`content/crew/ceres.json`).
    Hired(&'static str),
}

/// **The roster a story asks for**: slot `n` is `billets[n - 1]`, each in a leased prospector.
/// At most two: a Dray-P's bay holds two rigs.
pub(crate) struct Plan {
    /// The organiser's name, account seed and hull name (`driving_rig::prospector`).
    pub(crate) who: (&'static str, i64, &'static str),
    /// The activation's duty word (`mining`, `combat`, `recovery`): every hired NPC's contract
    /// must carry it.
    pub(crate) duty: &'static str,
    /// The slots, in order.
    pub(crate) billets: Vec<Billet>,
    /// The plan's `npc_commander`, by name: one of the hired, under a commander's contract.
    pub(crate) commander: Option<&'static str>,
}

/// **A roster standing on a site's ground**, every flight in its field phase.
pub(crate) struct Flown {
    /// The organiser.
    pub(crate) organiser: ObjId,
    /// The carrier, and its row on the port map.
    pub(crate) carrier: ObjId,
    /// The carrier's row.
    pub(crate) row: HullRow,
    /// The unit of each slot: slot `n` is `units[n - 1]`.
    pub(crate) units: Vec<ObjId>,
    /// The operator of each slot: the organiser, or the hired NPC.
    pub(crate) crew: Vec<ObjId>,
    /// The operation: the organiser's draft 1, activation 1.
    pub(crate) operation: OperationId,
}

impl Flown {
    /// The unit of `slot`.
    pub(crate) fn unit(&self, slot: usize) -> ObjId {
        self.units[slot - 1]
    }

    /// The operator of `slot`.
    pub(crate) fn operator(&self, slot: usize) -> ObjId {
        self.crew[slot - 1]
    }

    /// The operation's selector as its organiser types it (`1:1`).
    pub(crate) fn selector(&self) -> String {
        format!("{}:{}", self.operation.draft.0, self.operation.activation)
    }
}

/// The NPC named `name`.
pub(crate) fn npc_named(rig: &Rig, name: &str) -> ObjId {
    rig.tw
        .world()
        .iter()
        .find(|object| object.kind == ObjKind::Npc && object.name.as_str() == name)
        .map(|object| object.id)
        .unwrap_or_else(|| panic!("{name} stands in the world"))
}

/// **Every queued submission**, whoever owns its action (`driving_rig::drain` keeps only the
/// port's lanes: an Org unit grant and Sim's helm hand-off are neither).
pub(crate) fn queued(rig: &mut Rig) -> Vec<AuthoritativeSubmit> {
    let mut out = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        out.push(*submit);
    }
    out
}

/// The owners whose submissions a roster's journey lands: tacsim's lanes and the two it calls.
fn lands(submit: &AuthoritativeSubmit) -> bool {
    matches!(
        submit.action.key().owner.as_str(),
        "tacsim" | "inventory" | "market" | "org" | "sim"
    )
}

/// **`line` as `who`, and everything it queued landed on the composed path**; what `who` heard
/// and how many submissions landed.
///
/// # Panics
///
/// When a submission the line queued does not land.
pub(crate) fn commit_counted(rig: &mut Rig, who: ObjId, line: &str) -> (String, usize) {
    // Whatever an earlier tick left queued is not this line's.
    let _ = queued(rig);
    let out = run(&mut rig.tw, who, line);
    let mut landed = 0;
    for submit in queued(rig) {
        land_composed(rig, &submit).unwrap_or_else(|error| {
            panic!(
                "{line}: {} by #{} did not land: {error}; #{} heard: {out}",
                submit.action.key().id,
                submit.actor.0,
                who.0
            )
        });
        landed += 1;
    }
    (out, landed)
}

/// [`commit_counted`], asserting the line queued exactly one submission; what `who` heard.
pub(crate) fn commit(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, landed) = commit_counted(rig, who, line);
    assert_eq!(landed, 1, "{line}: one submission: {out}");
    out
}

/// **Land every due composed scheduled action named `name`** (Sim's `control_handoff_apply`, the
/// second half of a helm hand-off), as `m8_world`'s `progress` lands industry's: discovered,
/// planned, prepared and installed as the world task does, its writes kept; how many landed.
pub(crate) fn apply_due(rig: &mut Rig, name: &str) -> usize {
    let page = rig
        .tw
        .engine()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 256)
        .expect("the due schedules");
    let candidates: Vec<_> = page
        .candidates()
        .iter()
        .filter(|candidate| candidate.action().key().id.as_str() == name)
        .cloned()
        .collect();
    for candidate in &candidates {
        let ingress =
            mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 1)
                .expect("the schedule's ingress");
        let plan = rig
            .tw
            .engine()
            .plan_scheduled_action(candidate, &ingress)
            .expect("the schedule's plan");
        let (_, invocation, _) = plan.into_parts();
        let (prepared, _) = rig
            .tw
            .engine_mut()
            .prepare_registered_composed_scheduled_authoritative_action(
                candidate,
                &invocation,
                CommitToken::from_bytes(next_token()),
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let (writes, install) = prepared.into_parts();
        rig.tw
            .engine_mut()
            .install_composed_action(install)
            .expect("the schedule installs");
        let earlier = rig.tw.take_module_ops();
        fold_module_ops_into(&mut rig.store, earlier);
        let writes: Vec<_> = writes
            .into_iter()
            .filter(|write| {
                !matches!(
                    write,
                    mush_core::authoritative::ReplayWrite::ObjectSet { .. }
                        | mush_core::authoritative::ReplayWrite::ObjectDelete { .. }
                        | mush_core::authoritative::ReplayWrite::ObjectAllocatorSet { .. }
                )
            })
            .collect();
        let (ops, ledger) = support::composed::split(&writes);
        keep(rig, ops);
        fold_module_ops_into(&mut rig.store, ledger);
    }
    candidates.len()
}

/// **One world second of a roster's journey**: the ticks due, then every submission they queued
/// for tacsim, Org or Sim landed on the composed path, and every due helm hand-off applied; the
/// refusals, as sentences.
pub(crate) fn beat(rig: &mut Rig) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut refused = Vec::new();
    for submit in queued(rig) {
        if !lands(&submit) {
            continue;
        }
        if let Err(error) = land_composed(rig, &submit) {
            refused.push(format!(
                "{} by #{}: {error}",
                submit.action.key().id,
                submit.actor.0
            ));
        }
    }
    let _ = apply_due(rig, HANDOFF);
    refused
}

/// Sim's scheduled second half of a helm hand-off.
const HANDOFF: &str = "control_handoff_apply";

/// **Apply the helm hand-off just asked for**: the scheduled half lands once it is due, within
/// a few world seconds. A hand-off a beat already applied is the same hand-off, and the caller's
/// next step (a ready declaration, a launch) is what says whether the helm is held.
fn handoff(rig: &mut Rig) {
    for _ in 0..8 {
        if apply_due(rig, HANDOFF) > 0 {
            return;
        }
        let _ = beat(rig);
    }
}

/// The organiser's draft `id`, as the planning book holds it.
fn draft(rig: &mut Rig, organiser: ObjId, id: u64) -> Draft {
    let books = rig
        .tw
        .world()
        .module_state::<tacsim::detachment::planning::PlanningHandles>()
        .expect("the planning handles")
        .families
        .books
        .clone();
    typed(&mut rig.tw, &books, &organiser)
        .and_then(|book| book.drafts.get(&PlanId(id)).cloned())
        .expect("the draft is in the organiser's book")
}

/// A consent declaration's line (`+sortie/accept`, `+sortie/ready`) over `draft`.
fn declaration(verb: &str, draft: &Draft) -> String {
    format!(
        "+sortie/{verb} {}:{}:{}:{}",
        draft.organizer.0,
        draft.id.0,
        draft.revision,
        draft.plan_hash.as_str()
    )
}

/// The operation's record, as the family holds it.
pub(crate) fn operation(rig: &mut Rig, id: OperationId) -> Option<OperationRecord> {
    let operations = rig
        .tw
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .expect("the sortie handles")
        .operations
        .clone();
    typed(&mut rig.tw, &operations, &id)
}

/// **The roster as its last refresh confirmed it** (its leader, its term), when one has landed.
pub(crate) fn field_roster(rig: &mut Rig, id: OperationId) -> Option<DetachmentRoster> {
    let saved = operation(rig, id)?.command?;
    DetachmentRoster::restore(&saved).ok()
}

/// `unit`'s flight `ordinal`.
pub(crate) fn flight(rig: &mut Rig, unit: ObjId, ordinal: u64) -> Option<Flight> {
    let flights = rig
        .tw
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .expect("the sortie handles")
        .flights
        .clone();
    typed(&mut rig.tw, &flights, &FlightKey { unit, ordinal })
}

/// **The order fence a leader types** (`DT:<roster>:<revision>:<term>`), read off the prefill
/// line `+sortie/orders` prints for `who`, as a player's client reads it.
pub(crate) fn fence(rig: &mut Rig, who: ObjId, flown: &Flown) -> String {
    let report = run(
        &mut rig.tw,
        who,
        &format!(
            "+sortie/orders {}:{}",
            flown.operation.organizer.0,
            flown.selector()
        ),
    );
    report
        .split_whitespace()
        .find(|word| word.starts_with("DT:"))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("+sortie/orders prints the order prefill: {report}"))
}

/// **What `+sortie/orders` says of the order sent to `slot`** (`WILCO received`, `NEGATIVE
/// received`, `submitted; no reply confirmed`), for its latest sequence; `None` before any.
pub(crate) fn reply(rig: &mut Rig, who: ObjId, flown: &Flown, slot: usize) -> Option<String> {
    let report = run(
        &mut rig.tw,
        who,
        &format!(
            "+sortie/orders {}:{}",
            flown.operation.organizer.0,
            flown.selector()
        ),
    );
    let prefix = format!("Sent slot {slot} order ");
    report
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .next_back()
        .and_then(|rest| rest.split_once(": "))
        .map(|(_, status)| {
            status
                .trim_end_matches(" Execution unconfirmed.")
                .trim_end_matches('.')
                .to_owned()
        })
}

/// **Hire `npc` as `who`** at the room they share, through the offer `+sortie/crew` prints.
fn hire(rig: &mut Rig, who: ObjId, npc: ObjId) {
    let room = rig
        .tw
        .world()
        .get(npc)
        .and_then(|object| object.location)
        .expect("the crew member stands in a room");
    teleport(&mut rig.tw, who, room);
    let offers = run(&mut rig.tw, who, "+sortie/crew");
    let accept = offers
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("Accept: ")
                .filter(|line| line.contains(&format!("#{}=", npc.0)))
        })
        .unwrap_or_else(|| panic!("#{} is on offer: {offers}", npc.0))
        .to_owned();
    let _ = commit(rig, who, &accept);
}

/// **The operational loan `organiser` holds over `unit`'s crew station** (the lessor's
/// `OperateCrew` loan `+tv/<n>/permissions go` recorded), which a sub-loan to an NPC names as
/// its parent.
fn crew_loan(rig: &mut Rig, organiser: ObjId, unit: ObjId) -> u64 {
    use mush_org::unit_authority::{
        UnitCapability, UnitGrantStatus, UnitGrantee, UnitScope, runtime::UnitAuthorityHandles,
    };
    let registry = rig
        .tw
        .world()
        .module_state::<UnitAuthorityHandles>()
        .expect("Org's unit authority")
        .registry
        .clone();
    let grants = typed(&mut rig.tw, &registry, &()).expect("Org's grant registry");
    grants
        .grants
        .values()
        .find(|grant| {
            grant.capability == UnitCapability::OperateCrew
                && grant.status == UnitGrantStatus::Active
                && grant.grantee == UnitGrantee::Character(organiser)
                && matches!(&grant.scope, UnitScope::Units { units } if units.contains(&unit))
        })
        .map(|grant| grant.id.0)
        .unwrap_or_else(|| {
            panic!(
                "#{} holds an operate loan over #{}: {:?}",
                organiser.0, unit.0, grants.grants
            )
        })
}

/// **Fly `plan`'s roster onto `at`'s ground** (this module's head): the roster, every unit
/// standing at the ramp in its field phase and the field command refreshed.
///
/// `at_ceres` runs at Occator's counter after the leases are signed and before the hull sails
/// (a story's own extra lease or purchase).
#[allow(clippy::too_many_lines)] // one journey, in order
pub(crate) fn fly(
    rig: &mut Rig,
    at: &Ground,
    plan: &Plan,
    at_ceres: impl FnOnce(&mut Rig, ObjId),
) -> Flown {
    let hired: Vec<Option<ObjId>> = plan
        .billets
        .iter()
        .map(|billet| match billet {
            Billet::Leader => None,
            Billet::Hired(name) => Some(npc_named(rig, name)),
        })
        .collect();
    let commander = plan.commander.map(|name| npc_named(rig, name));
    assert!(
        commander.is_none_or(|npc| hired.contains(&Some(npc))),
        "the commander flies a slot"
    );
    let slots = plan.billets.len();
    assert!((1..=2).contains(&slots), "a Dray-P's bay holds two rigs");

    // (1) At Ceres: the crew hired, the rigs leased with their loans, the crew aboard.
    let (organiser, row, units) = on_pad_with_at(rig, plan.who, at.site(), |rig, organiser| {
        let _ = rig.tw.connect(organiser);
        for npc in hired.iter().flatten() {
            hire(rig, organiser, *npc);
        }
        at_counter(rig, organiser);
        for n in 1..=slots {
            let leased = run(&mut rig.tw, organiser, "+tv/lease go");
            assert!(
                leased.contains("Occator Water & Propellant leases you "),
                "rig {n} is leased: {leased}"
            );
            let (loan, landed) = commit_counted(rig, organiser, &format!("+tv/{n}/permissions go"));
            assert!(
                landed > 0,
                "rig {n}'s operational loans are recorded: {loan}"
            );
        }
        at_ceres(rig, organiser);
        for npc in hired.iter().flatten() {
            let aboard = run(&mut rig.tw, organiser, &format!("+sortie/board #{}", npc.0));
            assert!(aboard.contains("has come aboard"), "{aboard}");
        }
    });
    assert_eq!(units.len(), slots, "one leased rig a slot");
    let carrier = ObjId(row.hull);
    let crew: Vec<ObjId> = hired.iter().map(|npc| npc.unwrap_or(organiser)).collect();

    // (3) Each NPC is lent its rig under the organiser's own operational loan.
    for (unit, npc) in units.iter().zip(&hired) {
        if let Some(npc) = npc {
            let loan = crew_loan(rig, organiser, *unit);
            let _ = commit(
                rig,
                organiser,
                &format!(
                    "@unitgrant/issue #{}=#{} operate_crew parent={loan}",
                    unit.0, npc.0
                ),
            );
        }
    }

    // (4) A player leader boards their own rig in the bay and takes its console.
    let own = plan
        .billets
        .iter()
        .position(|billet| *billet == Billet::Leader);
    if let Some(index) = own {
        for step in ["inner", "hold"] {
            let _ = run(&mut rig.tw, organiser, step);
        }
        let bay = run(&mut rig.tw, organiser, "+tv/bay in");
        assert!(bay.contains("entered vehicle bay"), "{bay}");
        let boarded = run(&mut rig.tw, organiser, &format!("+tv/{} board", index + 1));
        assert!(boarded.contains("You are at rig"), "{boarded}");
        let focus = run(
            &mut rig.tw,
            organiser,
            &format!("+focus #{}/pilot", units[index].0),
        );
        assert!(!focus.contains("Usage"), "{focus}");
    }

    // (5) The plan: drafted, accepted and made ready by its player, activated.
    let payload = json!({
        "carrier": carrier.0,
        "slots": plan.billets.iter().enumerate().map(|(index, billet)| json!({
            "slot": index + 1,
            "unit": units[index].0,
            "controller": {
                "kind": if *billet == Billet::Leader { "player" } else { "npc" },
                "character": crew[index].0,
            },
            "role": "scout",
            "mind": null,
        })).collect::<Vec<_>>(),
        "npc_commander": commander.map(|npc| npc.0),
        "financial_terms": null,
    });
    let drafted = commit(rig, organiser, &format!("+sortie/draft {payload}"));
    let selected = draft(rig, organiser, 1);
    if let Some(index) = own {
        let _ = commit(rig, organiser, &declaration("accept", &selected));
        let _ = commit(
            rig,
            organiser,
            &format!("@station/handoff #{}/pilot", units[index].0),
        );
        handoff(rig);
        let _ = commit(rig, organiser, &declaration("ready", &selected));
    }
    let activated = commit(
        rig,
        organiser,
        &format!("+sortie/activate 1:1={}", plan.duty),
    );
    let operation = OperationId {
        organizer: organiser,
        draft: PlanId(1),
        activation: 1,
    };

    // (6) Each NPC seated, the roster launched, the field command refreshed.
    for (index, npc) in hired.iter().enumerate() {
        if npc.is_some() {
            let seated = commit(rig, organiser, &format!("+sortie/seat 1:1={}", index + 1));
            assert!(seated.contains("console"), "{seated}");
            handoff(rig);
        }
    }
    let launched = commit(rig, organiser, "+sortie/launch 1:1=all");
    let mut refused = Vec::new();
    // The ramp transit is IC time: the clock is taken to the last flight's due instant, as
    // `driving_rig::finish` takes it to an act's, and the detachment tick lands each finish.
    let due = units
        .iter()
        .filter_map(|unit| flight(rig, *unit, 1).map(|flight| flight.due.at))
        .max()
        .expect("every unit has a departure");
    let rate = i64::from(crate::driving_rig::world_rate(&rig.tw).max(1));
    let ahead = due
        .saturating_sub(crate::driving_rig::ic_now(&rig.tw))
        .max(0);
    let t = rig
        .tw
        .world()
        .now()
        .saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..60 {
        let down = units
            .iter()
            .all(|unit| flight(rig, *unit, 1).is_some_and(|flight| flight.phase == Phase::Field));
        if down {
            break;
        }
        refused.extend(beat(rig));
    }
    for unit in &units {
        let phase = flight(rig, *unit, 1).map(|flight| flight.phase);
        assert_eq!(
            phase,
            Some(Phase::Field),
            "#{} stands on the ground: draft {drafted} / activation {activated} / launch \
             {launched} / refused {refused:?}",
            unit.0
        );
    }
    let flown = Flown {
        organiser,
        carrier,
        row,
        units,
        crew,
        operation,
    };
    // The detachment tick refreshes the command itself once a flight is in the field; the
    // organiser's own refresh is the same act, asked for.
    for _ in 0..60 {
        if field_roster(rig, operation).is_some_and(|roster| roster.leader().is_some()) {
            break;
        }
        let _ = commit_counted(rig, organiser, "+sortie/refresh 1:1");
        refused.extend(beat(rig));
    }
    assert!(
        field_roster(rig, operation).is_some_and(|roster| roster.leader().is_some()),
        "the field command names a leader: {:?}; refused {refused:?}",
        field_roster(rig, operation).map(|roster| roster.checkpoint())
    );
    flown
}
