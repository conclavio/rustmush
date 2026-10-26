//! E6b (B5/B7): **the seal is canon, and only the producer gates it** — the real
//! three-crate loop.
//!
//! `mush-scenes` seals an outcome under a **staff-conferred sealer**, `mush-canon`
//! records it, and `mush-economy`'s attestation observer settles the run it
//! discharges. It lives here because the economy must not depend on canon
//! (08c §6.1 deviation 1) and this is the composition root that depends on all three.
//!
//! What these tests hold to:
//!
//! - **kind + source + party intersection, and nothing else.** `CanonRecorded`
//!   carries no `detail`, so the economy cannot check `sealed_by`; who may seal is
//!   the **producer's** gate. A seal naming another run attests nothing.
//! - **`scene.concluded` is never an outcome kind.** A payee who ends a scene
//!   manufactures no settlement.
//! - **DL-09.** A seal committed while the economy was frozen is still found after
//!   `@econ/unfreeze`, and one committed before a restart is found across it —
//!   through the durable read seam, carrying the record's own `entities`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_canon::{CanonEvent, CanonState};
use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, ModuleBootData, ModuleWrite,
    ObjId, ObjKind,
};
use mush_economy::{AccountId, ContractId, ContractState, EconomySnapshot, EconomyStore};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_server::econ_canon::CanonEvidenceReader;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

/// The one thing no shipped verb does: **set** the whole-economy latch. Boot's
/// recovery pass sets it in production and `@econ/unfreeze` clears it, so a test that
/// wants a frozen economy has to reach for it the way `e4c.rs` does.
struct FreezeStandIn;

impl GameModule for FreezeStandIn {
    fn name(&self) -> &'static str {
        "e6bfreeze"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            CommandSpec {
                name: "@econfreeze".into(),
                shape: ArgShape::None,
                help: "@econfreeze   test: set the whole-economy latch".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                mush_economy::set_economy_frozen(ctx, true);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

/// The production shape of the seal loop: canon, org, scenes, economy — with
/// `scene.outcome` registered as the one producer-declared outcome kind and DL-09's
/// read seam wired, exactly as `modules.rs` wires them.
fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        // E6b independent verify fold (M4): the composition root's own test wires
        // the **sealer gate** the way `modules.rs` wires it, so the seal is held to
        // rule 22 here as well — the authority that ends a run never stands in it.
        Box::new(mush_scenes::ScenesModule::new().with_sealer_gate(Arc::new(
            |w: &mush_core::World, run: u64, who: ObjId| {
                mush_economy::stands_in_contract(w, ContractId(run), who)
            },
        ))),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_canon_outcome_kinds(vec![SmolStr::new_static(
                    mush_scenes::KIND_SCENE_OUTCOME,
                )])
                .with_canon_evidence(Arc::new(CanonEvidenceReader)),
        ),
        Box::new(FreezeStandIn),
    ])
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
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn canon(tw: &TestWorld) -> Vec<CanonEvent> {
    tw.world()
        .module_state::<CanonState>()
        .expect("canon state")
        .recent()
}
fn account_of(tw: &TestWorld, owner: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(owner))
        .expect("account of owner")
        .id
}
fn only_contract(tw: &TestWorld) -> ContractId {
    let ids: Vec<ContractId> = snap(tw).contracts().map(|c| c.id).collect();
    assert_eq!(ids.len(), 1, "expected exactly one contract, got {ids:?}");
    ids[0]
}
/// The newest contract — the one a `@contract/draft` just minted.
fn newest_contract(tw: &TestWorld) -> ContractId {
    snap(tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("a contract")
}
fn scene_of(tw: &TestWorld, room: ObjId) -> mush_scenes::SceneUid {
    tw.world()
        .module_state::<mush_scenes::ScenesState>()
        .and_then(|st| st.latest_scene_at(room).map(|s| s.id))
        .expect("a scene")
}

/// Every module's persisted documents, last write wins — a reboot exactly as the
/// store would replay it.
type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = BTreeMap::new();
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

struct Rig {
    johnson: ObjId,
    runner: ObjId,
    /// A non-party in the room, and the one the seal is conferred on (E6b
    /// independent verify fold, M4). The shipped rig conferred it on the **runner**,
    /// who is a payee of the very run they sealed.
    facilitator: ObjId,
    room: ObjId,
    j_acct: AccountId,
    r_acct: AccountId,
}

fn rig(tw: &mut TestWorld) -> Rig {
    let johnson = tw.spawn_player("Johnson");
    let runner = tw.spawn_player("Runner");
    let facilitator = tw.spawn_player("Fixerless");
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "The Meet", GOD, None)
        .expect("a room");
    tw.world_mut().move_to(runner, room).expect("the move");
    tw.world_mut().move_to(facilitator, room).expect("the move");
    tw.world_mut().move_to(GOD, room).expect("the move");
    for who in [runner, facilitator] {
        tw.world_mut().set_connected(who, true);
        tw.world_mut().set_live_conns(who, 1);
    }
    ok(tw, GOD, "@instrument/register nuyen=0");
    ok(tw, GOD, "@account/system faucet=nuyen");
    for who in [johnson, runner] {
        ok(tw, GOD, &format!("@account/open #{}=nuyen", who.0));
    }
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let j_acct = account_of(tw, johnson);
    ok(tw, GOD, &format!("@econ/mint {faucet}={j_acct} 9999"));
    Rig {
        johnson,
        runner,
        facilitator,
        room,
        j_acct,
        r_acct: account_of(tw, runner),
    }
}

/// A fully-accepted run gated on a sealed scene outcome.
///
/// `source` is computed from the contract's own id — a `scene:{uid}` is unknowable
/// when the run is offered, which is exactly why the seal names `run:{contract}`.
fn accepted_run(tw: &mut TestWorld, r: &Rig, source: impl Fn(ContractId) -> String) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = newest_contract(tw);
    ok(
        tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000", r.r_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!(
            "@contract/condition {id}=attest canon:scene.outcome:{}",
            source(id)
        ),
    );
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.runner, &format!("@contract/accept {id}"));
    assert_eq!(state(tw, id), ContractState::Active);
    id
}

/// Start a scene in the meet room, confer the seal on the runner, and **bind it to
/// `contract`** — staff say which run a scene may settle (E6b verify fold, M2/R12).
fn scene_with_sealer(tw: &mut TestWorld, r: &Rig, contract: ContractId) -> mush_scenes::SceneUid {
    ok(tw, r.runner, "+timestop");
    let scene = scene_of(tw, r.room);
    ok(
        tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, r.facilitator.0, contract.0
        ),
    );
    scene
}

// ======================================================================

/// **The whole path**, and the authority at each step: staff confers the seal
/// (`@scene/facilitate`), the conferred sealer seals (`+scene/outcome`), canon
/// records it and emits `CanonRecorded`, the economy's observer matches it with
/// `canon_set_accepts` — kind, source, party intersection — and `ContractAuth`
/// settles the row.
#[test]
fn e6b_b7_a_conferred_seal_settles_the_run_it_names_and_pays_the_row() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    let before = bal(&tw, r.r_acct);
    scene_with_sealer(&mut tw, &r, id);
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sealed outcome did not settle the run: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
    let ev = canon(&tw)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("the seal reached canon");
    assert_eq!(ev.source, format!("run:{}#success", id.0));
    assert!(ev.entities.contains(&r.runner));
}

/// A seal naming **another** run attests nothing: `source` is exact, and it is the
/// load-bearing half of the match.
#[test]
fn e6b_b7_a_seal_naming_another_run_attests_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |_| "run:9999#success".to_string());
    let before = bal(&tw, r.r_acct);
    scene_with_sealer(&mut tw, &r, id);
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(bal(&tw, r.r_acct), before);
}

/// A seal that names **nobody the contract binds** attests nothing either — the
/// third gate, the party intersection (E4c verify fold, E1).
#[test]
fn e6b_b7_a_seal_naming_nobody_the_contract_binds_attests_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // The bystander is in the room and in the scene, and is the conferred sealer —
    // but is no party to the run and acts for nobody who is.
    let bystander = tw.spawn_player("Bystander");
    tw.world_mut().move_to(bystander, r.room).expect("the move");
    tw.world_mut().set_connected(bystander, true);
    tw.world_mut().set_live_conns(bystander, 1);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    ok(&mut tw, r.runner, "+timestop");
    let scene = scene_of(&tw, r.room);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, bystander.0, id.0
        ),
    );
    // Right kind, right source, wrong people: `entities` is the bystander alone.
    ok(
        &mut tw,
        bystander,
        &format!("+scene/outcome success=Bystander run={}", id.0),
    );
    let ev = canon(&tw)
        .into_iter()
        .find(|e| e.kind == "scene.outcome")
        .expect("the seal reached canon");
    assert_eq!(ev.entities, vec![bystander]);
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "a seal about somebody else's business settled this run"
    );
}

/// **`scene.concluded` is never an outcome kind.** A payee who ends the scene
/// produces a lifecycle record and settles nothing — the whole reason the kind table
/// is producer-declared (E4c verify fold, E1), and the hazard 08f §4.5 names.
#[test]
fn e6b_b7_scene_concluded_from_the_same_scene_attests_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    ok(&mut tw, r.runner, "+timestop");
    ok(&mut tw, r.runner, "+timestop/end");
    assert!(
        canon(&tw).iter().any(|e| e.kind == "scene.concluded"),
        "the lifecycle record was not produced"
    );
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "a scene the payee ended settled their own run"
    );
}

/// A `canon:` set with **no source** is refused at offer (`check_canon_sets`), and so
/// is one naming a kind nobody registered.
#[test]
fn e6b_b8_a_sourceless_or_unregistered_canon_set_is_refused_at_offer() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // A source is not even spellable: `@contract/condition` refuses the grammar
    // before an offer could, which is strictly stronger than `check_canon_sets`.
    ok(&mut tw, r.johnson, "@contract/draft Sourceless");
    let bare = newest_contract(&tw);
    err(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {bare}=attest canon:scene.outcome"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/cancel {bare}"));
    // And a kind nobody registered is refused at offer, by `check_canon_sets`.
    {
        let condition = "canon:scene.concluded:run:1";
        ok(&mut tw, r.johnson, "@contract/draft Extraction");
        let id = newest_contract(&tw);
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/pay {id}={} 10", r.j_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/split {id}={} 10", r.r_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/remainder {id}={}", r.j_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=attest {condition}"),
        );
        err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        assert_eq!(state(&tw, id), ContractState::Draft);
        ok(&mut tw, r.johnson, &format!("@contract/cancel {id}"));
    }
}

/// **DL-09.** A seal committed while the economy is **frozen** records no
/// attestation at the time — a contract must never claim evidence the world had
/// stopped acting on — and is still found afterwards, because `@econ/unfreeze`
/// re-scans the tail (now carrying the record's own `entities`).
#[test]
fn e6b_b5_a_seal_committed_while_frozen_is_found_after_unfreeze() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    scene_with_sealer(&mut tw, &r, id);
    let before = bal(&tw, r.r_acct);

    ok(&mut tw, GOD, "@econfreeze");
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert_eq!(state(&tw, id), ContractState::Active);

    ok(&mut tw, GOD, "@econ/unfreeze");
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the seal was lost to the freeze: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

/// **DL-09 across a restart.** The economy's canon tail is RAM-only and dies with
/// the process, so the only thing that can find a seal committed before a reboot is
/// the durable read seam — asked once by the first sweep after boot, keyed on the
/// source the contract's own condition names, and carrying the record's `entities`
/// so the party-intersection gate is no weaker than the live path's.
#[test]
fn e6b_b5_a_seal_committed_before_a_restart_is_found_across_it() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    scene_with_sealer(&mut tw, &r, id);
    let before = bal(&tw, r.r_acct);

    // Freeze so the live edge records nothing at all, then seal.
    ok(&mut tw, GOD, "@econfreeze");
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert_eq!(state(&tw, id), ContractState::Active);

    // The restart: every module reloads from its own documents, so the economy's
    // RAM tail is gone and canon's open tail is what survives.
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the contract did not survive the reboot"
    );
    // Boot re-derives the freeze latch from the meta document, so clear it the way a
    // staffer would and let the sweep run its once-per-boot source re-scan.
    ok(&mut tw, GOD, "@econ/unfreeze");
    tw.set_now(100_000);
    tw.fire_due(100_000_000);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the seal was lost across the restart: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

// ---- E6b independent verify fold ----------------------------------------

/// **M4.** `@scene/facilitate` performed no party check at all, so staff could
/// confer the seal of a run on one of that run's own payees — the authority that
/// decides which outcome row pays them. The gate is asked at conferral **and** at
/// seal, because a contract's roster moves in between; and a non-party seals and
/// pays exactly as before, which is the other half of the claim.
#[test]
fn vf2_m4_a_runs_own_payee_cannot_be_conferred_the_seal_that_pays_them() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    ok(&mut tw, r.runner, "+timestop");
    let scene = scene_of(&tw, r.room);
    // The runner is a party; the Johnson is a party and the payer.
    for who in [r.runner, r.johnson] {
        let refusal = err(
            &mut tw,
            GOD,
            &format!("@scene/facilitate {}=#{} run={}", scene.0, who.0, id.0),
        );
        assert!(
            refusal.contains("never held by someone standing in it"),
            "{refusal}"
        );
    }
    assert!(
        tw.world()
            .module_state::<mush_scenes::ScenesState>()
            .and_then(|st| st.scenes.get(&scene).map(|s| s.sealer.is_none()))
            .unwrap_or_default(),
        "a party was conferred the seal anyway"
    );
    // The non-party is conferred, seals, and the run pays.
    let before = bal(&tw, r.r_acct);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, r.facilitator.0, id.0
        ),
    );
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the non-party seal did not settle: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

/// **M4, at seal.** Conferral passes while the sealer is a bystander; the sealer
/// then joins the contract as a party, and the seal is refused — the roster moved,
/// and the gate is asked again at the moment the authority is used.
#[test]
fn vf2_m4_the_gate_is_asked_again_at_seal_because_the_roster_moves() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // A draft the facilitator is not yet a party to.
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000", r.r_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/condition {id}=attest canon:scene.outcome:run:{}#success",
            id.0
        ),
    );
    ok(&mut tw, r.runner, "+timestop");
    let scene = scene_of(&tw, r.room);
    // Conferral stands: the facilitator is nobody in this contract yet.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, r.facilitator.0, id.0
        ),
    );
    // …and then they join it.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.facilitator.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    let refusal = err(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert!(
        refusal.contains("never held by someone standing in it"),
        "{refusal}"
    );
}

/// **R1.** `rescan_canon_sources` took its 256-contract budget **before** filtering
/// live, and `contracts` is ascending by id with terminal records never collected —
/// so on a world with 256 dead contracts the budget was spent entirely on them and
/// the live run at the end was never visited. DL-09's recovery stopped silently.
///
/// The probe: 256 cancelled contracts, then the run; freeze, seal, reboot, unfreeze.
#[test]
fn vf2_r1_the_source_rescan_filters_live_before_it_takes_its_budget() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // 256 terminal contracts ahead of the live one, exactly as a long-lived world
    // accumulates them.
    for _ in 0..mush_economy::MAX_CANON_CONTRACTS_VISITED {
        ok(&mut tw, r.johnson, "@contract/draft Old");
        let dead = newest_contract(&tw);
        ok(&mut tw, r.johnson, &format!("@contract/cancel {dead}"));
    }
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    scene_with_sealer(&mut tw, &r, id);
    let before = bal(&tw, r.r_acct);

    ok(&mut tw, GOD, "@econfreeze");
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    ok(&mut tw, GOD, "@econ/unfreeze");
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the live run was past the budget the dead ones spent: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

/// **R3(a).** The durable read seam answered from the ledger's **open tail**, and
/// `Ledger::restore_from_kv` restores a *sealed* tail chunk as an empty open chunk —
/// so a seal committed just before a 16 KB chunk boundary was unrecoverable across
/// the very reboot DL-09 exists for. The seam now also reads the last sealed chunks.
///
/// The probe marks every persisted `log:` chunk sealed before the reboot, which is
/// precisely the state a boundary that fell between the seal and the bounce leaves.
#[test]
fn vf2_r3_a_seal_in_a_sealed_chunk_is_still_found_across_the_reboot() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    scene_with_sealer(&mut tw, &r, id);
    let before = bal(&tw, r.r_acct);

    ok(&mut tw, GOD, "@econfreeze");
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    let mut kv = fold_all(&mut tw);
    let mut sealed_any = false;
    for (key, value) in kv.entry(SmolStr::new_static("canon")).or_default() {
        if key.starts_with("log:")
            && let Some(map) = value.as_object_mut()
        {
            map.insert("sealed".to_owned(), serde_json::Value::Bool(true));
            sealed_any = true;
        }
    }
    assert!(sealed_any, "the rig wrote no canon chunk to seal");
    reboot(&mut tw, &kv);
    assert!(
        tw.world()
            .module_state::<CanonState>()
            .is_some_and(|st| st.recent().is_empty()),
        "the open tail is not empty, so this probe is not testing the sealed read"
    );
    ok(&mut tw, GOD, "@econ/unfreeze");
    tw.set_now(100_000);
    tw.fire_due(100_000_000);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sealed-chunk seal was lost: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

/// **R3(b).** And a seal that is genuinely gone — no record anywhere, in the open
/// tail or the sealed window — is **paged** on the contract's own subject rather
/// than returning silently, with the level-2 command that resolves it.
#[test]
fn vf2_r3_a_source_no_record_answers_pages_the_ladder() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    // Nothing is ever sealed. The recovery re-scan asks the seam, gets nothing, and
    // says so.
    ok(&mut tw, GOD, "@econfreeze");
    ok(&mut tw, GOD, "@econ/unfreeze");
    let paged = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .escalations()
        .find(|e| e.subject == mush_economy::EscalationSubject::Contract(id))
        .cloned()
        .expect("a page about the missing evidence");
    // H4 (E4c-D42): evidence past recovery wears its own kind; `SettlementRefused`
    // now means only a settlement refusal.
    assert_eq!(
        paged.kind,
        mush_economy::EscalationKind::EvidenceUnavailable
    );
    assert_eq!(paged.kind.label(), "evidence unavailable");
    assert!(
        paged
            .recommended
            .iter()
            .any(|c| c.contains("@contract/resolve")),
        "the page names no command: {:?}",
        paged.recommended
    );
}

/// **R4.** An amendment clears `satisfied`, but a canon source names the run and the
/// row and no generation — so the next re-scan found the old record and put the old
/// attestation straight back. A seal made before the crew joined would have paid the
/// crew. Evidence older than the current generation is ignored; a seal made after it
/// still settles.
#[test]
fn vf2_r4_a_pre_amendment_seal_does_not_re_attest_the_amended_surface() {
    let mut tw = boot();
    let r = rig(&mut tw);
    // The run is **offered** but not yet active, so the seal below records nothing
    // live (the observer only ever writes onto a live contract) and the re-scan at
    // activation is what would resurrect it.
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = newest_contract(&tw);
    let source = format!("run:{}#success", id.0);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000", r.r_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest canon:scene.outcome:{source}"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Offered);

    // The seal, against the surface as it stands.
    ok(&mut tw, r.runner, "+timestop");
    let scene = scene_of(&tw, r.room);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene.0, r.facilitator.0, id.0
        ),
    );
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert!(
        canon(&tw).iter().any(|e| e.source == source),
        "the seal did not reach canon"
    );

    // The amendment re-opens the surface. The generation moves with it.
    let now = tw.world().now();
    tw.set_now(now + 60);
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);
    // A′: the author's own acceptance survives the amendment, so only the runner
    // re-binds.
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    // Activation re-scans the tail and the durable seam, finds the old record with
    // exactly the right kind, source and people — and must not use it.
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "a seal made against the previous consent surface settled the amended one"
    );
    let c = snap(&tw).contract(id).expect("contract").clone();
    assert!(
        c.satisfied.iter().all(Option::is_none),
        "the pre-amendment attestation came back: {:?}",
        c.satisfied
    );

    // …and a seal made **after** the amendment settles it, which is what makes the
    // rule a generation check rather than a refusal to use canon at all.
    ok(&mut tw, GOD, "+timestop/end");
    ok(&mut tw, r.runner, "+timestop");
    let scene2 = scene_of(&tw, r.room);
    assert_ne!(scene2, scene, "the second seal must be a second scene");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@scene/facilitate {}=#{} run={}",
            scene2.0, r.facilitator.0, id.0
        ),
    );
    let before = bal(&tw, r.r_acct);
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the post-amendment seal did not settle: {:?}",
        state(&tw, id)
    );
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}

/// **R5.** The 64-deep RAM tail was walked **before** the re-entrancy latch was
/// taken — the latch lived one level down, in the source re-scan — so a settlement's
/// own observers could in principle re-enter the tail walk. The latch is at the
/// entry point now: one walk, one settlement, one payout.
#[test]
fn vf2_r5_the_rescan_latch_covers_the_tail_walk_too() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = accepted_run(&mut tw, &r, |c| format!("run:{}#success", c.0));
    scene_with_sealer(&mut tw, &r, id);
    ok(&mut tw, GOD, "@econfreeze");
    ok(
        &mut tw,
        r.facilitator,
        &format!("+scene/outcome success=Runner run={}", id.0),
    );
    let before = bal(&tw, r.r_acct);
    ok(&mut tw, GOD, "@econ/unfreeze");
    assert_eq!(
        bal(&tw, r.r_acct),
        before + 1000,
        "the row paid more than once"
    );
    // And a second re-scan finds the filled slot and changes nothing.
    tw.set_now(100_000);
    tw.fire_due(100_000_000);
    assert_eq!(bal(&tw, r.r_acct), before + 1000);
}
