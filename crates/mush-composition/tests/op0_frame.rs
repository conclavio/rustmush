//! **Doc 50 cut OP.0 — the operation frame's seam at the composition root** (Q-50-57 item 4;
//! doc 50a A-2h).
//!
//! TacSim has no arm yet (OP.1 to OP.5 build it), so a **fake owner** stands in for doc 50's
//! declaring arm: [`FakeArm`] uses nothing but TacSim's side of the seam
//! (`tacsim::operations::{frame, OperationRequest, OperationStatus, ..}`), finds the frame in
//! world state and declares through it, then reads who won. The frame it finds is the one the
//! composition root seats ([`mush_composition::campaign_operations::CampaignLink`]), over the real
//! campaign, economy and org state.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use mush_campaign::{CampaignModule, CloseProbe, OperationId};
use mush_composition::campaign_operations::{CampaignLink, TacsimCloseProbe};
use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandCompatibility, CommandSpec, Ctx, GOD, ObjId, World,
};
use mush_economy::{AccountId, Amount, EconomyStore};
use mush_org::{
    MembershipStatus, OrgAction, OrgId, OrgMembership, OrgMutationOp, OrgMutationPlan, OrgStore,
    PrincipalId, Visibility,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;
use tacsim::operations::{
    MetFavors, OperationBond, OperationOutcome, OperationPhase, OperationRequest,
};

// ---- the fake owner: TacSim's side of the seam, and nothing else ----------

/// **A stand-in for doc 50's declaring arm** (OP.5). It knows TacSim's vocabulary only.
struct FakeArm;

impl FakeArm {
    /// Declare through whatever frame the composition seated, as the arm's tick would.
    fn declare(ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String> {
        let Some(frame) = tacsim::operations::frame(ctx.w) else {
            return Err("no operation frame is composed".to_owned());
        };
        frame.declare(ctx, request)
    }

    /// Who won, once decided.
    fn outcome(w: &World, operation: u64) -> Option<OperationOutcome> {
        tacsim::operations::frame(w)?.status(w, operation)?.outcome
    }
}

// ---- rig ------------------------------------------------------------------

type Staged = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Option<Staged>>>);

struct ProbeMod(Probe);

impl GameModule for ProbeMod {
    fn name(&self) -> &'static str {
        "probe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::LegacyPostImage);
        let probe = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@probe".into(),
                shape: ArgShape::EqPair,
                help: "test probe: run one staged call with a Ctx".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let staged = probe.0.lock().ok().and_then(|mut slot| slot.take());
                if let Some(staged) = staged {
                    staged(ctx);
                }
                Ok(())
            }),
        )?;
        Ok(())
    }
}

/// The tank farm's answer, which a test sets (OP.4 builds the real one in TacSim).
#[derive(Clone, Default)]
struct TankFarm(Arc<Mutex<Option<bool>>>);

impl CloseProbe for TankFarm {
    fn read(&self, _w: &World, probe: &str, _operation: OperationId) -> Option<bool> {
        if probe != "isidis.tank_farm" {
            return None;
        }
        self.0.lock().ok().and_then(|slot| *slot)
    }
}

struct Rig {
    tw: TestWorld,
    probe: Probe,
}

fn boot(campaign: CampaignModule, linked: bool) -> Rig {
    let probe = Probe::default();
    let campaign: Box<dyn GameModule> = if linked {
        Box::new(CampaignLink::new(campaign))
    } else {
        Box::new(campaign)
    };
    let tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        campaign,
        Box::new(ProbeMod(probe.clone())),
    ]);
    Rig { tw, probe }
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) {
    let events = tw.run(actor, line);
    let refused = events.iter().any(|(_, event)| {
        matches!(event, OutputEvent::Sys(text)
            if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    });
    assert!(!refused, "`{line}` was refused: {}", texts(&events));
}

/// Run `f` inside a command, answering what it answered.
fn with_ctx<T: Send + 'static>(
    rig: &mut Rig,
    f: impl FnOnce(&mut Ctx<'_>) -> T + Send + 'static,
) -> T {
    let out: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&out);
    if let Ok(mut slot) = rig.probe.0.lock() {
        *slot = Some(Box::new(move |ctx: &mut Ctx<'_>| {
            let answer = f(ctx);
            if let Ok(mut slot) = sink.lock() {
                *slot = Some(answer);
            }
        }));
    }
    ok(&mut rig.tw, GOD, "@probe");
    out.lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .expect("the staged call ran")
}

fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

fn balance(tw: &TestWorld, id: AccountId) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(id).map(|a| a.posted_balance.minor()))
        .expect("the account")
}

/// An organization with a treasury on its root scope, funded with `funds`.
fn org(tw: &mut TestWorld, name: &str, funds: i64) -> (OrgId, AccountId) {
    ok(tw, GOD, &format!("@org/create {name}"));
    let (org, root) = tw
        .world()
        .module_state::<OrgStore>()
        .and_then(|s| {
            s.state()
                .orgs()
                .find(|o| o.name.as_str() == name)
                .map(|o| (o.id, o.root_scope))
        })
        .expect("the org");
    ok(
        tw,
        GOD,
        &format!("@account/open org:{name}/{}=gold", root.0),
    );
    let (treasury, faucet) = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| {
            let snap = s.snapshot();
            (
                snap.accounts()
                    .find(|a| a.owner == PrincipalId::Organization(org))
                    .map(|a| a.id),
                snap.accounts()
                    .find(|a| matches!(a.owner, PrincipalId::System(_)))
                    .map(|a| a.id),
            )
        })
        .expect("the economy");
    let treasury = treasury.expect("the treasury");
    if funds > 0 {
        let faucet = faucet.expect("the faucet");
        ok(tw, GOD, &format!("@econ/mint {faucet}={treasury} {funds}"));
    }
    (org, treasury)
}

/// The Isidis shape as OP.5's arm will ask for it.
fn request(
    republic: OrgId,
    treasury: AccountId,
    freeport: OrgId,
    freeport_treasury: AccountId,
) -> OperationRequest {
    OperationRequest {
        declarer: PrincipalId::Organization(republic),
        summary: "assume administration of the Concession".to_owned(),
        probe: "isidis.tank_farm".to_owned(),
        met_favors: MetFavors::Defenders,
        sides: vec![
            ("republic".to_owned(), PrincipalId::Organization(republic)),
            ("freeport".to_owned(), PrincipalId::Organization(freeport)),
        ],
        defenders: vec![PrincipalId::Organization(freeport)],
        legal_targets: vec!["flagged-hull".to_owned()],
        bond: Some(OperationBond {
            account: treasury,
            amount: Amount::from_minor(250),
            forfeit_to: Some((PrincipalId::Organization(freeport), freeport_treasury)),
        }),
        notice_secs: mush_campaign::OPERATION_NOTICE_MIN,
        window_secs: 28 * 86_400,
        public: true,
    }
}

struct Isidis {
    rig: Rig,
    farm: TankFarm,
    treasury: AccountId,
    freeport_treasury: AccountId,
    freeport: OrgId,
    member: ObjId,
}

fn isidis() -> Isidis {
    let farm = TankFarm::default();
    let mut rig = boot(
        CampaignModule::new().with_close_probe(Arc::new(farm.clone())),
        true,
    );
    ok(&mut rig.tw, GOD, "@instrument/register gold=0");
    ok(&mut rig.tw, GOD, "@account/system faucet=gold");
    let (republic, treasury) = org(&mut rig.tw, "Republic", 1_000);
    let (freeport, freeport_treasury) = org(&mut rig.tw, "Freeport", 0);
    let member = rig.tw.spawn_player("Ama");
    with_ctx(&mut rig, move |ctx| {
        let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutMembership {
            membership: OrgMembership {
                org: freeport,
                member,
                status: MembershipStatus::Active,
                titles: Vec::new(),
                roles: BTreeSet::new(),
                visibility: Visibility::Public,
                since: 0,
                revision: 1,
            },
        }]);
        mush_org::submit(
            ctx,
            &plan,
            OrgAction {
                action: SmolStr::new_static("op0.enroll"),
                org: Some(freeport),
                detail: serde_json::json!({}),
            },
        )
        .map_err(|e| e.to_string())
    })
    .expect("the membership is written");
    let _ = (republic, member);
    Isidis {
        rig,
        farm,
        treasury,
        freeport_treasury,
        freeport,
        member,
    }
}

fn declare(x: &mut Isidis) -> u64 {
    let republic = x
        .rig
        .tw
        .world()
        .module_state::<OrgStore>()
        .and_then(|s| s.state().orgs().find(|o| o.name.as_str() == "Republic"))
        .map(|o| o.id)
        .expect("the Republic");
    let ask = request(republic, x.treasury, x.freeport, x.freeport_treasury);
    with_ctx(&mut x.rig, move |ctx| FakeArm::declare(ctx, &ask)).expect("declared")
}

fn jump_past(tw: &mut TestWorld, ic: i64) {
    let now_ic = tw.world().world_timeline().stamp_at(tw.world().now()).at;
    let t = tw.world().now() + (ic - now_ic).max(0) + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

// ---- the seam -------------------------------------------------------------

/// **TacSim declares through the frame and reads who won**, end to end: the fake owner finds the
/// frame the composition root seated, declares the Isidis shape (an organization against an
/// organization, a close probe that is the defenders' success, a bond forfeit to the defenders),
/// and reads the frame's own answer: the declarer failed, and the bond is the defenders'.
#[test]
fn op0_a2h_tacsim_declares_through_the_frame_and_reads_who_won() {
    let mut x = isidis();
    let id = declare(&mut x);
    let status = {
        let w = x.rig.tw.world();
        tacsim::operations::frame(w)
            .and_then(|frame| frame.status(w, id))
            .expect("the frame holds it")
    };
    assert_eq!(status.phase, OperationPhase::Notice);
    assert_eq!(status.outcome, None);
    assert_eq!(
        status.defenders,
        vec![PrincipalId::Organization(x.freeport)]
    );
    assert_eq!(status.sides.len(), 2);
    assert_eq!(status.legal_targets, vec!["flagged-hull".to_owned()]);
    let (open, close) = status.window;
    assert_eq!(close - open, 28 * 86_400);
    // The window opens; a full tank farm mid-window decides nothing.
    *x.farm.0.lock().expect("farm") = Some(true);
    jump_past(&mut x.rig.tw, open);
    fire(&mut x.rig.tw);
    assert_eq!(
        FakeArm::outcome(x.rig.tw.world(), id),
        None,
        "decided mid-window"
    );
    // The close: the farm is full, so the defenders hold.
    jump_past(&mut x.rig.tw, close);
    for _ in 0..10 {
        fire(&mut x.rig.tw);
    }
    let w = x.rig.tw.world();
    let status = tacsim::operations::frame(w)
        .and_then(|frame| frame.status(w, id))
        .expect("still held");
    assert_eq!(status.phase, OperationPhase::Settled);
    assert_eq!(status.outcome, Some(OperationOutcome::DeclarerFailed));
    assert_eq!(balance(&x.rig.tw, x.freeport_treasury), 250);
    assert_eq!(balance(&x.rig.tw, x.treasury), 750);
    let _ = x.member;
}

/// **The other way round: an empty tank farm at the close is the declarer's success**, and the
/// bond goes home.
#[test]
fn op0_a2h_an_unmet_probe_is_the_declarers_success_through_the_frame() {
    let mut x = isidis();
    let id = declare(&mut x);
    *x.farm.0.lock().expect("farm") = Some(false);
    let close = {
        let w = x.rig.tw.world();
        tacsim::operations::frame(w)
            .and_then(|frame| frame.status(w, id))
            .expect("held")
            .window
            .1
    };
    jump_past(&mut x.rig.tw, close);
    for _ in 0..10 {
        fire(&mut x.rig.tw);
    }
    assert_eq!(
        FakeArm::outcome(x.rig.tw.world(), id),
        Some(OperationOutcome::DeclarerSucceeded)
    );
    assert_eq!(balance(&x.rig.tw, x.treasury), 1_000);
    assert_eq!(balance(&x.rig.tw, x.freeport_treasury), 0);
}

/// **Unlinked, TacSim finds no frame and declares nothing** (fail closed), and a refused
/// declaration comes back as words, with nothing written.
#[test]
fn op0_a2h_without_the_link_there_is_no_frame_and_a_refusal_is_words() {
    let mut bare = boot(CampaignModule::new(), false);
    assert!(tacsim::operations::frame(bare.tw.world()).is_none());
    let ask = request(OrgId(0), AccountId(1), OrgId(1), AccountId(2));
    let refused = with_ctx(&mut bare, move |ctx| FakeArm::declare(ctx, &ask));
    assert!(refused.is_err());

    let mut x = isidis();
    let mut poor = request(OrgId(0), x.treasury, x.freeport, x.freeport_treasury);
    poor.bond = Some(OperationBond {
        account: x.treasury,
        amount: Amount::from_minor(5_000),
        forfeit_to: None,
    });
    let answer = with_ctx(&mut x.rig, move |ctx| FakeArm::declare(ctx, &poor));
    assert!(answer.is_err(), "{answer:?}");
    let w = x.rig.tw.world();
    assert!(
        tacsim::operations::frame(w)
            .and_then(|frame| frame.status(w, 1))
            .is_none(),
        "a refused declaration was written"
    );
}

/// **TacSim's own close probe is the production wiring**: it names no probe in OP.0, so it
/// answers "cannot tell", which the frame treats as a wait.
#[test]
fn op0_a2h_tacsims_close_probe_answers_nothing_for_an_unknown_name() {
    let rig = boot(CampaignModule::new(), true);
    assert_eq!(
        TacsimCloseProbe.read(rig.tw.world(), "isidis.tank_farm", OperationId(1)),
        None
    );
    assert!(tacsim::operations::CLOSE_PROBES.is_empty());
}
