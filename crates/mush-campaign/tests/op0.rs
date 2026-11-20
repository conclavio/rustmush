//! **Doc 50 cut OP.0 — the campaign engine's five additions** (Q-50-57; doc 50a A-2a to A-2h).
//!
//! These run the real campaign tick, the real economy and the real org state over `TestWorld`,
//! on `ap1.rs`'s composition, and drive the compiled owner's call
//! (`mush_campaign::declare_and_activate`) from a probe command, as a compiled arm would from its
//! own tick. One test each:
//!
//! - **A1/A2g** the public declare-and-activate, and the "unfunded" pause before `@op/activate`;
//! - **A2f** an organization as declarer, side and defender, and a defender organization's
//!   notice reaching its members;
//! - **A2a** a close-only objective, never read before the close, with its own polarity, and
//!   waiting (never resolving) when its probe cannot answer;
//! - **A2c** the bond forfeit to the defenders' treasury on the declarer's failure, and released
//!   on their success.
//!
//! The TacSim side of the seam (A2h) is proven in `mush-composition`'s own test, where both
//! crates are in reach.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mush_campaign::{
    AudienceSpec, BondTerms, CampaignStore, CloseProbe, Declaration, DeclareError, DefenderLetter,
    Favors, OPERATION_NOTICE_MIN, ObjectiveKind, ObjectiveSpec, OperationDoc, OperationId,
    OperationState, PauseReason, SideSpec,
};
use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandCompatibility, CommandSpec, Ctx, GOD, ObjId, World,
};
use mush_economy::{AccountId, Amount, EconomicVisibility, EconomyStore};
use mush_org::{
    MembershipStatus, OrgAction, OrgId, OrgMembership, OrgMutationOp, OrgMutationPlan, OrgStore,
    PrincipalId, Visibility,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

/// One staged call, run inside a command so it has a `Ctx`, exactly as a compiled arm's tick
/// would.
type Staged = Box<dyn FnOnce(&mut Ctx<'_>) + Send>;

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Option<Staged>>>);

impl Probe {
    fn stage(&self, f: impl FnOnce(&mut Ctx<'_>) + Send + 'static) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(Box::new(f));
        }
    }
}

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

/// What the fake close probe answers, and how often it was asked.
#[derive(Default)]
struct FakeProbe {
    answer: Mutex<Option<bool>>,
    reads: AtomicUsize,
}

impl FakeProbe {
    fn set(&self, answer: Option<bool>) {
        if let Ok(mut slot) = self.answer.lock() {
            *slot = answer;
        }
    }
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// The campaign's seam, over a shared [`FakeProbe`].
struct ProbeSeam(Arc<FakeProbe>);

impl CloseProbe for ProbeSeam {
    fn read(&self, _w: &World, probe: &str, _operation: OperationId) -> Option<bool> {
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        if probe != "tank_farm" {
            return None;
        }
        self.0.answer.lock().ok().and_then(|slot| *slot)
    }
}

type Log = Arc<Mutex<Vec<(ObjId, DefenderLetter)>>>;

struct Rig {
    tw: TestWorld,
    probe: Probe,
    fake: Arc<FakeProbe>,
    notices: Log,
}

fn boot() -> Rig {
    let probe = Probe::default();
    let fake = Arc::new(FakeProbe::default());
    let notices: Log = Arc::default();
    let log = Arc::clone(&notices);
    let campaign = mush_campaign::CampaignModule::new()
        .with_close_probe(Arc::new(ProbeSeam(Arc::clone(&fake))))
        .with_defender_notice(Arc::new(
            move |_ctx: &mut Ctx<'_>, who: ObjId, letter: &DefenderLetter| {
                if let Ok(mut rows) = log.lock() {
                    rows.push((who, letter.clone()));
                }
            },
        ));
    let tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![]).with_canon_outcome_kinds(vec![
                SmolStr::new_static(mush_campaign::KIND_OPERATION_RESOLVED),
            ]),
        ),
        Box::new(campaign),
        Box::new(ProbeMod(probe.clone())),
    ]);
    Rig {
        tw,
        probe,
        fake,
        notices,
    }
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let events = tw.run(actor, line);
    let text = texts(&events);
    let refused = events.iter().any(|(_, event)| {
        matches!(event, OutputEvent::Sys(text)
            if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    });
    assert!(!refused, "`{line}` was refused: {text}");
    text
}

/// One campaign tick fire.
fn fire(tw: &mut TestWorld) {
    let period = i64::try_from(mush_campaign::TICK_PERIOD_SECS).unwrap_or(15);
    let t = tw.world().now() + period + 1;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

/// Jump the world clock to `t` and fire what is due.
fn jump_to(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

fn doc(tw: &TestWorld, id: OperationId) -> OperationDoc {
    tw.world()
        .module_state::<CampaignStore>()
        .and_then(|store| store.operation(id).cloned())
        .expect("the operation")
}

fn balance(tw: &TestWorld, id: AccountId) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(id).map(|a| a.posted_balance.minor()))
        .expect("the account")
}

fn held(tw: &TestWorld, id: AccountId) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(id).map(|a| a.held_amount.minor()))
        .expect("the account")
}

/// The world IC second now, on the world clock.
fn ic_now(tw: &TestWorld) -> i64 {
    tw.world().world_timeline().stamp_at(tw.world().now()).at
}

/// Move the world clock until the world IC clock reads at least `ic`, and fire.
fn jump_to_ic(tw: &mut TestWorld, ic: i64) {
    let ahead = ic.saturating_sub(ic_now(tw)).max(0);
    // The world IC clock runs at its configured rate; step the wall clock until it is past.
    let mut t = tw.world().now();
    for _ in 0..64 {
        t = t.saturating_add(ahead.max(16));
        tw.set_now(t);
        if ic_now(tw) >= ic {
            break;
        }
    }
    jump_to(tw, t);
}

/// An instrument, a faucet, and an organization with a funded treasury on its root scope.
/// Answers the org and its treasury.
fn org_with_treasury(tw: &mut TestWorld, name: &str, funds: i64) -> (OrgId, AccountId) {
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
    let treasury = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .accounts()
                .find(|a| a.owner == PrincipalId::Organization(org))
                .map(|a| a.id)
        })
        .expect("the treasury");
    if funds > 0 {
        let faucet = tw
            .world()
            .module_state::<EconomyStore>()
            .and_then(|s| {
                s.snapshot()
                    .accounts()
                    .find(|a| matches!(a.owner, PrincipalId::System(_)))
                    .map(|a| a.id)
            })
            .expect("the faucet");
        ok(tw, GOD, &format!("@econ/mint {faucet}={treasury} {funds}"));
    }
    (org, treasury)
}

fn economy(tw: &mut TestWorld) {
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
}

/// Put `member` in `org` with `status`, through the org engine's own mutation path.
fn enroll(rig: &mut Rig, org: OrgId, member: ObjId, status: MembershipStatus) {
    rig.probe.stage(move |ctx| {
        let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutMembership {
            membership: OrgMembership {
                org,
                member,
                status,
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
                org: Some(org),
                detail: serde_json::json!({}),
            },
        )
        .expect("the membership is written");
    });
    ok(&mut rig.tw, GOD, "@probe");
}

/// The Isidis shape, as OP.5's arm will declare it: a declarer organization, a defender
/// organization, a close probe that is the defenders' success, a bond forfeit to the defenders'
/// treasury.
fn isidis(
    declarer: OrgId,
    bond_account: AccountId,
    defender: OrgId,
    defender_treasury: AccountId,
    favors: Favors,
) -> Declaration {
    Declaration {
        declarer: PrincipalId::Organization(declarer),
        objective: ObjectiveSpec {
            kind: ObjectiveKind::ProbeAtClose {
                probe: SmolStr::new_static("tank_farm"),
                met_favors: favors,
            },
            summary: SmolStr::new_static("assume administration of the Concession"),
        },
        legal_targets: vec![SmolStr::new_static("flagged-hull")],
        stakes: Vec::new(),
        bond: Some(BondTerms {
            account: bond_account,
            amount: Amount::from_minor(300),
            forfeit_to: Some((PrincipalId::Organization(defender), defender_treasury)),
        }),
        sides: vec![
            SideSpec {
                name: SmolStr::new_static("republic"),
                principal: PrincipalId::Organization(declarer),
            },
            SideSpec {
                name: SmolStr::new_static("freeport"),
                principal: PrincipalId::Organization(defender),
            },
        ],
        eligible_defenders: AudienceSpec {
            members: vec![PrincipalId::Organization(defender)],
        },
        notice_secs: OPERATION_NOTICE_MIN,
        window_secs: 28 * 86_400,
        visibility: EconomicVisibility::Audience {
            members: vec![PrincipalId::Organization(defender)],
            announce_issuer: false,
        },
    }
}

/// Run `declaration` through the compiled owner's call, answering what it answered.
fn declare(rig: &mut Rig, declaration: Declaration) -> Result<OperationId, DeclareError> {
    let out: Arc<Mutex<Option<Result<OperationId, DeclareError>>>> = Arc::default();
    let sink = Arc::clone(&out);
    rig.probe.stage(move |ctx| {
        let answer = mush_campaign::declare_and_activate(ctx, declaration);
        if let Ok(mut slot) = sink.lock() {
            *slot = Some(answer);
        }
    });
    ok(&mut rig.tw, GOD, "@probe");
    out.lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .expect("the call ran")
}

struct Isidis {
    rig: Rig,
    republic: OrgId,
    republic_treasury: AccountId,
    freeport_treasury: AccountId,
    members: [ObjId; 2],
    lapsed: ObjId,
    id: OperationId,
}

/// The whole Isidis rig, declared: two orgs, two active members and one suspended in the
/// defender org.
fn declared_isidis(favors: Favors) -> Isidis {
    let mut rig = boot();
    economy(&mut rig.tw);
    let (republic, republic_treasury) = org_with_treasury(&mut rig.tw, "Republic", 1_000);
    let (freeport, freeport_treasury) = org_with_treasury(&mut rig.tw, "Freeport", 0);
    let a = rig.tw.spawn_player("Ama");
    let b = rig.tw.spawn_player("Bex");
    let lapsed = rig.tw.spawn_player("Cole");
    enroll(&mut rig, freeport, a, MembershipStatus::Active);
    enroll(&mut rig, freeport, b, MembershipStatus::Active);
    enroll(&mut rig, freeport, lapsed, MembershipStatus::Suspended);
    let id = declare(
        &mut rig,
        isidis(
            republic,
            republic_treasury,
            freeport,
            freeport_treasury,
            favors,
        ),
    )
    .expect("the operation is declared");
    Isidis {
        rig,
        republic,
        republic_treasury,
        freeport_treasury,
        members: [a, b],
        lapsed,
        id,
    }
}

/// Drive the operation through its window to the close, then drain the fan-out.
fn run_to_close(x: &mut Isidis) {
    let close = doc(&x.rig.tw, x.id).terms.window.1.at;
    jump_to_ic(&mut x.rig.tw, close + 1);
    for _ in 0..12 {
        fire(&mut x.rig.tw);
    }
}

// ---- A2g: the unfunded pause before activation ----------------------------

/// **A bonded operation still `Declared` is not paused "unfunded" before `@op/activate`**
/// (doc 50 §4.3; doc 50a A-2g). The bond is placed by activation, so its absence before then is
/// the design, not a fault. Before the fix one tick fire between `@op/terms bond=` and
/// `@op/activate` paused the operation, activation left the pause, and it never ran.
#[test]
fn op0_a2g_a_bonded_declared_operation_runs_after_activation() {
    let mut rig = boot();
    economy(&mut rig.tw);
    let issuer = rig.tw.spawn_player("Issuer");
    ok(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=gold", issuer.0),
    );
    let (account, faucet) = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| {
            let snap = s.snapshot();
            let mine = snap
                .accounts()
                .find(|a| a.owner == PrincipalId::Object(issuer))
                .map(|a| a.id);
            let faucet = snap
                .accounts()
                .find(|a| matches!(a.owner, PrincipalId::System(_)))
                .map(|a| a.id);
            (mine, faucet)
        })
        .expect("the economy");
    let (account, faucet) = (account.expect("mine"), faucet.expect("faucet"));
    ok(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {faucet}={account} 500"),
    );
    ok(&mut rig.tw, issuer, "@op/declare hold the depot");
    let id = OperationId(1);
    ok(
        &mut rig.tw,
        issuer,
        &format!("@op/terms {}/bond={account}:200", id.0),
    );
    // Several fires while it stands declared with a bond and no hold.
    for _ in 0..3 {
        fire(&mut rig.tw);
    }
    assert_eq!(
        doc(&rig.tw, id).runtime.paused,
        None,
        "a declared operation was paused before its bond could exist"
    );
    ok(&mut rig.tw, issuer, &format!("@op/activate {}", id.0));
    assert_eq!(held(&rig.tw, account), 200, "the bond is placed");
    // Through the notice to the window: it runs, unpaused.
    let open = doc(&rig.tw, id).terms.window.0.at;
    jump_to_ic(&mut rig.tw, open + 1);
    fire(&mut rig.tw);
    let after = doc(&rig.tw, id);
    assert_eq!(after.runtime.paused, None);
    assert_eq!(after.runtime.state, OperationState::Running);
}

// ---- A1 / A2f: declare and activate, as an organization -------------------

/// **A compiled owner declares and activates in one step, as an organization** (Q-50-57 items 1
/// and 2). The bond is placed on the declarer organization's own treasury, the terms are frozen
/// and the operation is born `Activated`.
#[test]
fn op0_a1_an_organization_declares_and_activates_in_one_step() {
    let x = declared_isidis(Favors::Defenders);
    let doc = doc(&x.rig.tw, x.id);
    assert_eq!(doc.runtime.state, OperationState::Activated);
    assert!(doc.consent_intact(), "the frozen terms hash");
    assert_eq!(doc.terms.attacker, PrincipalId::Organization(x.republic));
    assert!(doc.terms.bond.as_ref().is_some_and(|b| b.hold.is_some()));
    assert_eq!(held(&x.rig.tw, x.republic_treasury), 300);
    assert_eq!(balance(&x.rig.tw, x.republic_treasury), 1_000);
    // The notice runs from the declaration.
    let lead = doc.terms.window.0.at - doc.terms.notice.at;
    assert_eq!(lead, OPERATION_NOTICE_MIN);
}

/// **A declarer that cannot fund its bond declares nothing** (the never-negative gate, as
/// `@op/activate` keeps it), and a notice outside the band is refused by the terms.
#[test]
fn op0_a1_an_unfunded_or_misframed_declaration_writes_nothing() {
    let mut rig = boot();
    economy(&mut rig.tw);
    let (republic, treasury) = org_with_treasury(&mut rig.tw, "Republic", 100);
    let (freeport, freeport_treasury) = org_with_treasury(&mut rig.tw, "Freeport", 0);
    let poor = declare(
        &mut rig,
        isidis(
            republic,
            treasury,
            freeport,
            freeport_treasury,
            Favors::Defenders,
        ),
    );
    assert!(matches!(poor, Err(DeclareError::Bond(_))), "{poor:?}");
    let mut short = isidis(
        republic,
        treasury,
        freeport,
        freeport_treasury,
        Favors::Defenders,
    );
    short.notice_secs = OPERATION_NOTICE_MIN - 1;
    let short = declare(&mut rig, short);
    assert!(matches!(short, Err(DeclareError::Terms(_))), "{short:?}");
    // A bond on somebody else's account is refused before the economy is asked.
    let theirs = declare(
        &mut rig,
        isidis(
            republic,
            freeport_treasury,
            freeport,
            freeport_treasury,
            Favors::Defenders,
        ),
    );
    assert!(
        matches!(theirs, Err(DeclareError::Refused(_))),
        "{theirs:?}"
    );
    let live = rig
        .tw
        .world()
        .module_state::<CampaignStore>()
        .map_or(0, |s| s.operations().count());
    assert_eq!(live, 0, "a refused declaration wrote an operation");
    assert_eq!(held(&rig.tw, treasury), 0);
}

/// **A defender organization's notice reaches its active members**, once, and they may read the
/// notice (Q-50-57 item 2; Q-45-17). A suspended member and a stranger are told nothing.
#[test]
fn op0_a2f_a_defender_organizations_notice_reaches_its_members() {
    let mut x = declared_isidis(Favors::Defenders);
    let stranger = x.rig.tw.spawn_player("Dray");
    fire(&mut x.rig.tw);
    fire(&mut x.rig.tw);
    let told: Vec<ObjId> = x
        .rig
        .notices
        .lock()
        .map(|rows| rows.iter().map(|(who, _)| *who).collect())
        .unwrap_or_default();
    assert_eq!(told, x.members.to_vec(), "told once each, in id order");
    assert!(doc(&x.rig.tw, x.id).runtime.noticed);
    // A member reads the notice; the plan stays hidden.
    let shown = ok(&mut x.rig.tw, x.members[0], &format!("@op/show {}", x.id.0));
    assert!(shown.contains("You are entitled to defend"), "{shown}");
    assert!(!shown.contains("Concession"), "the plan leaked: {shown}");
    // Neither the suspended member nor a stranger can see it at all.
    for who in [x.lapsed, stranger] {
        let events = x.rig.tw.run(who, &format!("@op/show {}", x.id.0));
        assert!(
            texts(&events).contains(mush_campaign::NO_SUCH_OPERATION),
            "{}",
            texts(&events)
        );
    }
}

// ---- A2a: the close-only objective ----------------------------------------

/// **A close-only objective is never read before the close, and a met probe mid-window decides
/// nothing** (Q-50-57 item 3; doc 50a A-2a). At the close a met probe that favors the defenders
/// resolves the operation as the declarer's `failure`.
#[test]
fn op0_a2a_the_probe_is_read_only_at_the_close_and_its_polarity_decides() {
    let mut x = declared_isidis(Favors::Defenders);
    x.rig.fake.set(Some(true));
    let (open, close) = {
        let d = doc(&x.rig.tw, x.id);
        (d.terms.window.0.at, d.terms.window.1.at)
    };
    jump_to_ic(&mut x.rig.tw, open + 1);
    fire(&mut x.rig.tw);
    // Mid-window, met: nothing resolves, and nobody asked.
    jump_to_ic(&mut x.rig.tw, open + (close - open) / 2);
    for _ in 0..3 {
        fire(&mut x.rig.tw);
    }
    assert_eq!(doc(&x.rig.tw, x.id).runtime.state, OperationState::Running);
    assert_eq!(x.rig.fake.reads(), 0, "the probe was read before the close");
    jump_to_ic(&mut x.rig.tw, close + 1);
    fire(&mut x.rig.tw);
    let after = doc(&x.rig.tw, x.id);
    assert!(after.runtime.state.is_live() || after.runtime.state.is_terminal());
    assert!(
        !matches!(after.runtime.state, OperationState::Running),
        "the close did not resolve it"
    );
    assert_eq!(
        after.runtime.root.as_ref().map(|r| r.outcome.as_str()),
        Some("failure"),
        "a met probe favoring the defenders is the declarer's failure"
    );
}

/// **The polarity is the whole difference**: the same met probe, favoring the declarer, is the
/// declarer's `success`; an unmet one is the other side's.
#[test]
fn op0_a2a_the_four_answers_of_a_close_probe() {
    use mush_campaign::close_outcome;
    assert_eq!(close_outcome(true, Favors::Declarer), "success");
    assert_eq!(close_outcome(false, Favors::Declarer), "failure");
    assert_eq!(close_outcome(true, Favors::Defenders), "failure");
    assert_eq!(close_outcome(false, Favors::Defenders), "success");
    let mut x = declared_isidis(Favors::Declarer);
    x.rig.fake.set(Some(true));
    run_to_close(&mut x);
    assert_eq!(
        doc(&x.rig.tw, x.id)
            .runtime
            .root
            .as_ref()
            .map(|r| r.outcome.to_string()),
        Some("success".to_owned())
    );
}

/// **A probe that cannot answer is a wait, never a verdict.** The operation stays running past
/// its close, and resolves on the first fire the probe answers.
#[test]
fn op0_a2a_an_unanswered_probe_waits_and_resolves_when_it_answers() {
    let mut x = declared_isidis(Favors::Defenders);
    x.rig.fake.set(None);
    run_to_close(&mut x);
    let waiting = doc(&x.rig.tw, x.id);
    assert_eq!(waiting.runtime.state, OperationState::Running);
    assert_eq!(waiting.runtime.paused, None, "a wait is not a pause");
    assert!(x.rig.fake.reads() > 1, "it asks again each fire");
    x.rig.fake.set(Some(false));
    fire(&mut x.rig.tw);
    assert_eq!(
        doc(&x.rig.tw, x.id)
            .runtime
            .root
            .as_ref()
            .map(|r| r.outcome.to_string()),
        Some("success".to_owned()),
        "an unmet probe favoring the defenders is the declarer's success"
    );
}

// ---- A2c: the bond forfeit ------------------------------------------------

/// **The bond is forfeit to the defenders' treasury when the declarer fails** (Q-50-57 item 5,
/// provisional; doc 50a A-2c). Before OP.0 it was released whatever happened.
#[test]
fn op0_a2c_the_declarers_failure_forfeits_the_bond_to_the_defenders() {
    let mut x = declared_isidis(Favors::Defenders);
    x.rig.fake.set(Some(true));
    run_to_close(&mut x);
    let after = doc(&x.rig.tw, x.id);
    assert_eq!(after.runtime.state, OperationState::Resolved);
    assert_eq!(held(&x.rig.tw, x.republic_treasury), 0);
    assert_eq!(balance(&x.rig.tw, x.republic_treasury), 700);
    assert_eq!(
        balance(&x.rig.tw, x.freeport_treasury),
        300,
        "the defenders' treasury took the bond"
    );
}

/// **The declarer's success releases the bond** to its own treasury, untouched.
#[test]
fn op0_a2c_the_declarers_success_releases_the_bond() {
    let mut x = declared_isidis(Favors::Defenders);
    x.rig.fake.set(Some(false));
    run_to_close(&mut x);
    assert_eq!(doc(&x.rig.tw, x.id).runtime.state, OperationState::Resolved);
    assert_eq!(held(&x.rig.tw, x.republic_treasury), 0);
    assert_eq!(balance(&x.rig.tw, x.republic_treasury), 1_000);
    assert_eq!(balance(&x.rig.tw, x.freeport_treasury), 0);
}

/// **Staff can still abandon one, and abandoning releases** (the rule `abort` keeps).
#[test]
fn op0_a2c_an_abandonment_still_releases() {
    let mut x = declared_isidis(Favors::Defenders);
    ok(&mut x.rig.tw, GOD, &format!("@op/resolve {}=abort", x.id.0));
    for _ in 0..8 {
        fire(&mut x.rig.tw);
    }
    assert_eq!(doc(&x.rig.tw, x.id).runtime.state, OperationState::Aborted);
    assert_eq!(balance(&x.rig.tw, x.republic_treasury), 1_000);
    assert_eq!(balance(&x.rig.tw, x.freeport_treasury), 0);
    assert_ne!(
        doc(&x.rig.tw, x.id).runtime.paused,
        Some(PauseReason::Unfunded)
    );
}
