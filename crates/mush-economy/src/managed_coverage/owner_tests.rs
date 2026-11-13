#![cfg(test)]

//! Real native Economy owner tests. Captured product/host/role selectors below
//! are inert fixtures: these tests do not certify a role, open a real sortie,
//! publish a product or authorize the production Tacsim producer.
use super::*;
use crate::record::{CatalogRef, EconomicCause, HoldStatus};
use crate::registry_filing::{EconomyRequest, InitialFilingParticipant, InitialFilingPolicy};
use crate::{AccountId, Amount, EconomyStore, InstrumentId};
use mush_core::action::{ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput};
use mush_core::action_state::StateFamilyHandle;
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::OwnerId;
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, DefinitionDescriptor, DefinitionRegistryBuilder, GOD, GameModule, ModError, ObjKind,
    PrepareCtx, Registrar, RulesValue,
};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_testkit::cold::{Boot, fold_replay_writes_into};
use mush_testkit::fixtures::token;
use mush_testkit::funding::Funding;
use mush_testkit::owner_rig::{self, InertOwner, MockOwner, PreparedAction};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::{Deserialize, Serialize};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

use mush_org::{OrgId, OrgScopeId, OrgStore};
const SERVICE: ObjId = ObjId(2);
use mush_core::{ClockRef, ClockStamp};

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    actions: [DefinitionRef; 3],
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn initial_descriptor() -> Result<DefinitionDescriptor> {
    RIG.descriptor("action", "initial_filing", 1)
}
fn initial() -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, initial_descriptor()?)?)
}
fn rules() -> Result<Rules> {
    let mut d = RIG.declare()?;
    let actions = [
        d.add("action", BIND_ACTION, 1)?,
        d.add("action", CLAIM_ACTION, 1)?,
        d.add("action", RELEASE_ACTION, 1)?,
    ];
    let wrong = d.add("action", "wrong_coverage", 1)?;
    let outcome = d.add("outcome", "coverage_fixture", 1)?;
    Ok(Rules {
        declarations: d.finish(),
        actions,
        wrong,
        outcome,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Bind {
        basis: Box<CoverageBasisV1>,
        terms: CoverageQuoteTermsV1,
    },
    Terminal {
        binding: CommitToken,
        reason: CoverageDecisionKindV1,
    },
}
const RECEIPT_FAMILY: &str = "coverage_fixture_receipt";
/// This rig's names in the shared owner-rig skeleton (`mush_testkit::owner_rig`).
const RIG: MockOwner = MockOwner {
    owner: "tacsim",
    label: "native-hull-coverage",
    receipts: RECEIPT_FAMILY,
    idempotency: "coverage_fixture",
    refusal: "hull coverage owner fixture refused",
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    history: CoverageHistory,
}
impl FixtureReceipt {
    fn validate(&self) -> std::result::Result<(), String> {
        self.history.validate().map_err(|_| "history shape")?;
        let (action, commit, actor) = match self.history.terminal() {
            None => (
                &self.history.consent().effect,
                self.history.consent().operation,
                self.history.consent().terms.buyer,
            ),
            Some(t) => (
                &t.decision().action,
                t.decision().operation,
                t.decision().actor,
            ),
        };
        if self.format != 1
            || self.actor.0 == 0
            || &self.action != action
            || self.commit != commit
            || self.actor != actor
        {
            return Err("fixture affinity".into());
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        if state.coverage_history(self.history.consent().operation)? != &self.history {
            return Err("actual native output differs".into());
        }
        Ok(())
    }
}
impl owner_rig::FixtureReceipt for FixtureReceipt {
    fn action(&self) -> &DefinitionRef {
        &self.action
    }
    fn commit(&self) -> CommitToken {
        self.commit
    }
    fn actor(&self) -> ObjId {
        self.actor
    }
}

struct Provider {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    action: DefinitionRef,
    handle: ParticipantHandle<InitialFilingParticipant>,
    outcome: DefinitionRef,
}
fn error() -> ActionError {
    RIG.error()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let input: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let request = match input {
            Input::Bind { basis, terms } => EconomyRequest::CoverageBind(CoverageBindRequest {
                basis: *basis,
                terms,
            }),
            Input::Terminal { binding, reason } => {
                let w = ctx.world().map_err(|_| error())?;
                EconomyRequest::CoverageTerminal(CoverageTerminalRequest {
                    binding,
                    decision: CoverageDecisionV1 {
                        format: 1,
                        operation: ctx.commit_token(),
                        action: self.action.clone(),
                        actor: inv.actor(),
                        at_ic: w.world_ic_now(),
                        posted_at_utc: w.now(),
                        reason,
                    },
                })
            }
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?
            .coverage()
            .map_err(|_| error())?;
        let history = output.history().clone();
        let value = serde_json::to_value(&history).map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &ctx.commit_token(),
            FixtureReceipt {
                format: 1,
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor: inv.actor(),
                history,
            },
        )
        .map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), value).map_err(|_| error())?,
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        owner_rig::register_declarations(reg, &self.declarations, &["action", "outcome"])?;
        let receipts = RIG.receipts(reg, 128, 65_536, FixtureReceipt::validate)?;
        let handle =
            reg.request_owner_participant::<InitialFilingParticipant>(self.actions[0].clone())?;
        for action in &self.actions {
            let h = if action == &self.actions[0] {
                handle.clone()
            } else {
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?
            };
            reg.action_provider(
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action: action.clone(),
                    handle: h,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
        reg.action_provider(
            self.wrong.clone(),
            Provider {
                receipts,
                action: self.wrong.clone(),
                handle,
                outcome: self.outcome.clone(),
            },
        )?;
        Ok(())
    }
}
fn modules(r: &Rules, enabled: bool) -> Vec<Box<dyn GameModule>> {
    let issuer = SystemPrincipalId::new("roll.registry").expect("issuer");
    let policy = InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(2),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("policy");
    let economy = crate::EconomyModule::new()
        .with_title_issuers(vec![issuer.clone()])
        .with_initial_filing(policy, initial().expect("initial"));
    // Initial filing is a real configuration prerequisite, but this fixture never
    // invokes it or pretends its inert paid basis is a filing receipt.
    let roll = InertOwner::new(
        "roll",
        vec![initial_descriptor().expect("initial")],
        error(),
    );
    let mut modules: Vec<Box<dyn GameModule>> =
        vec![Box::new(mush_org::OrgModule::new()), Box::new(roll)];
    if enabled {
        modules.push(Box::new(r.clone()));
        modules.push(Box::new(economy.with_hull_coverage(
            CoverageConfig::new(SERVICE, r.actions.clone()).expect("coverage config"),
        )));
    } else {
        modules.push(Box::new(economy));
    }
    modules
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    actor: ObjId,
    buyer: ObjId,
    shell: ObjId,
    boot: Boot,
    staged: Option<FixtureReceipt>,
}
impl Fixture {
    #[inline(never)]
    fn empty_enabled(enabled: bool) -> Result<Box<Self>> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, enabled));
        world.world_mut().set_now(100);
        world
            .world_mut()
            .set_game_clock(mush_core::GameClock::default());
        let actor = world.spawn_player("Coverage service");
        check_eq!(actor, SERVICE);
        let buyer = world.spawn_player("Coverage buyer");
        let shell = world
            .world_mut()
            .create(ObjKind::Thing, "Native covered title", GOD, None)?;
        Ok(Box::new(Self {
            world,
            rules,
            actor,
            buyer,
            shell,
            boot: Boot::new(),
            staged: None,
        }))
    }
    fn empty() -> Result<Box<Self>> {
        Self::empty_enabled(true)
    }
    fn new() -> Result<Box<Self>> {
        let mut f = Self::empty()?;
        Funding::of("cr")
            .line("@org/create Underwriter")
            .line("@org/scope create Underwriter=coverage")
            .register(0)
            .system("faucet")
            .open(f.buyer)
            .system("roll.registry")
            .open_for("org:Underwriter/0")
            .mint(0, 1, 10000)
            .mint(0, 3, 10000)
            .line(format!("@asset/title #{}=#{}", f.shell.0, f.buyer.0))
            .run_into(&mut f.world, &mut f.boot)?;
        Ok(f)
    }
    fn state(&self) -> &EconomyStore {
        self.world
            .world()
            .module_state::<EconomyStore>()
            .expect("economy")
    }
    fn state_mut(&mut self) -> &mut EconomyStore {
        self.world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy")
    }
    fn basis(&self) -> Result<CoverageBasisV1> {
        Ok(CoverageBasisV1 {
            format: 1,
            product: token(200),
            product_digest: DefinitionDigest::from_bytes([45; 32]),
            quote: token(201),
            quote_digest: DefinitionDigest::from_bytes([46; 32]),
            ic_open: token(202),
            seal_digest: DefinitionDigest::from_bytes([47; 32]),
            admission_digest: DefinitionDigest::from_bytes([48; 32]),
            departure_digest: DefinitionDigest::from_bytes([49; 32]),
            hull: self.shell,
            hull_title_revision: self
                .state()
                .snapshot
                .asset(self.shell)
                .ok_or("actual hull title")?
                .revision,
            hull_revision: 0,
            hull_schema: self.rules.outcome.clone(),
            zone: self.rules.outcome.clone(),
            qualified_role: None,
        })
    }
    fn terms(&self) -> Result<CoverageQuoteTermsV1> {
        let orgs = self
            .world
            .world()
            .module_state::<OrgStore>()
            .ok_or("actual Org owner")?
            .state();
        let org = orgs.org(OrgId(0)).ok_or("actual insurer")?;
        let scope = orgs.scope(OrgScopeId(0)).ok_or("actual scope")?;
        check_eq!(scope.org, org.id);
        Ok(CoverageQuoteTermsV1 {
            format: 1,
            buyer: self.buyer,
            insurer: org.id,
            scope: scope.id,
            treasury: AccountId(3),
            payer: AccountId(1),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            adjudicator: self.actor,
            insured_value: Amount::from_minor(10000),
            base_premium: Amount::from_minor(100),
            discount: Amount::ZERO,
            premium: Amount::from_minor(100),
            liability: Amount::from_minor(8000),
            duration_seconds: 100,
            claim_delay_seconds: 10,
            quoted_at_utc: self.world.world().now(),
            expires_at_utc: self.world.world().now() + 60,
        })
    }
    fn prepare(
        &mut self,
        action: &DefinitionRef,
        actor: ObjId,
        n: u8,
        input: Input,
    ) -> Result<PreparedAction> {
        RIG.prepare(
            &mut self.world,
            action,
            actor,
            n,
            serde_json::to_value(input)?,
        )
    }
    fn bind(&mut self, n: u8) -> Result<PreparedAction> {
        self.prepare(
            &self.rules.actions[0].clone(),
            self.buyer,
            n,
            Input::Bind {
                basis: Box::new(self.basis()?),
                terms: self.terms()?,
            },
        )
    }
    fn terminal(&mut self, n: u8, reason: CoverageDecisionKindV1) -> Result<PreparedAction> {
        let index = if reason.pays() { 1 } else { 2 };
        let actor = if matches!(reason, CoverageDecisionKindV1::Cancelled { .. }) {
            self.buyer
        } else {
            self.actor
        };
        self.prepare(
            &self.rules.actions[index].clone(),
            actor,
            n,
            Input::Terminal {
                binding: token(2),
                reason,
            },
        )
    }
    fn cold(&self, boot: Boot) -> Result<Box<Self>> {
        self.cold_enabled(boot, true)
    }
    fn cold_enabled(&self, boot: Boot, enabled: bool) -> Result<Box<Self>> {
        let mut world = self
            .world
            .cold_restart(modules(&self.rules, enabled), boot)?;
        let _ = world.take_module_ops();
        let _ = world.world_mut().take_dirty();
        Ok(Box::new(Self {
            world,
            rules: self.rules.clone(),
            actor: self.actor,
            buyer: self.buyer,
            shell: self.shell,
            boot: Boot::new(),
            staged: None,
        }))
    }
    fn stage(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let (writes, receipt) =
            RIG.stage(&mut self.world, action, n, p, FixtureReceipt::validate)?;
        self.staged = Some(receipt);
        Ok(writes)
    }
    fn take_install(&mut self) -> Result {
        owner_rig::install_staged(&mut self.world)?;
        self.staged
            .take()
            .ok_or("prepared typed receipt")?
            .verify(self.state())?;
        Ok(())
    }
    fn publish(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let writes = self.stage(action, n, p)?;
        self.take_install()?;
        fold_replay_writes_into(&mut self.boot, &writes)?;
        Ok(writes)
    }
}

fn bound() -> Result<Box<Fixture>> {
    let mut f = Fixture::new()?;
    let pending = f.bind(2)?;
    f.publish(f.rules.actions[0].clone(), 2, pending)?;
    Ok(f)
}
fn history(f: &Fixture) -> Result<CoverageHistory> {
    Ok(f.state().coverage_history(token(2))?.clone())
}
fn posted(f: &Fixture, id: u64) -> Result<i64> {
    Ok(f.state()
        .snapshot
        .account(AccountId(id))
        .ok_or("account")?
        .posted_balance
        .minor())
}
fn clean() -> CoverageDecisionKindV1 {
    CoverageDecisionKindV1::CleanClose {
        close: token(203),
        close_digest: DefinitionDigest::from_bytes([50; 32]),
        transition: 2,
    }
}
fn loss(f: &Fixture) -> Result<CoverageDecisionKindV1> {
    let h = history(f)?;
    Ok(CoverageDecisionKindV1::Destroyed {
        loss: token(203),
        loss_digest: DefinitionDigest::from_bytes([50; 32]),
        occurred_at: ClockStamp {
            clock: h.consent().terms.starts_at.clock.clone(),
            at: h.consent().terms.starts_at.at + 1,
        },
        zone: h.consent().basis.zone.clone(),
        transition: 2,
    })
}

#[test]
fn native_bind_charges_once_reserves_full_liability_and_replays_exact_collateral() -> Result {
    let mut f = Fixture::new()?;
    let now = f.world.world().world_ic_now();
    let pending = f.bind(2)?;
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (10000, 10000));
    check_eq!(f.state().coverage_histories()?.len(), 0);
    f.publish(f.rules.actions[0].clone(), 2, pending)?;
    let h = history(&f)?;
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10100));
    check_eq!(h.consent().terms.starts_at, now);
    check_eq!(h.consent().terms.ends_at.at, now.at + 100);
    check_eq!(
        f.state()
            .snapshot
            .hold(h.binding().hold())
            .ok_or("hold")?
            .status,
        HoldStatus::Held
    );
    check_eq!(
        f.state()
            .snapshot
            .account(AccountId(3))
            .ok_or("treasury")?
            .available()
            .ok_or("available")?
            .minor(),
        2100
    );
    check!(
        h.binding()
            .title()
            .encumbrances
            .contains(&crate::record::Encumbrance::Collateral {
                contract: h.contract_id()
            })
    );
    check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    check!(crate::test_checks::refused(f.bind(4))?);
    Ok(())
}

#[test]
fn delayed_native_claim_moves_exact_wreck_title_and_pays_without_new_spend() -> Result {
    let mut f = bound()?;
    let h = history(&f)?;
    check!(f.terminal(3, loss(&f)?).is_err());
    f.world.world_mut().set_now(120);
    let p = f.terminal(3, loss(&f)?)?;
    f.publish(f.rules.actions[1].clone(), 3, p)?;
    let actual = history(&f)?;
    check_eq!(actual.consent(), h.consent());
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (17900, 2100));
    let title = f.state().snapshot.asset(f.shell).ok_or("actual title")?;
    check_eq!(title.title_owner, PrincipalId::Organization(OrgId(0)));
    check_eq!(title.scope, Some(OrgScopeId(0)));
    check!(title.encumbrances.is_empty());
    check!(matches!(
        f.state()
            .snapshot
            .hold(h.binding().hold())
            .ok_or("hold")?
            .status,
        HoldStatus::Consumed { .. }
    ));
    check!(history(f.cold(f.boot.clone())?.as_ref())? == actual);
    check!(f.terminal(4, loss(&f)?).is_err());
    Ok(())
}

#[test]
fn clean_close_and_buyer_cancellation_release_liability_but_keep_premium() -> Result {
    for cancelled in [false, true] {
        let mut f = bound()?;
        let reason = if cancelled {
            CoverageDecisionKindV1::Cancelled {
                close: token(203),
                close_digest: DefinitionDigest::from_bytes([50; 32]),
                transition: 2,
            }
        } else {
            clean()
        };
        let p = f.terminal(3, reason)?;
        f.publish(f.rules.actions[2].clone(), 3, p)?;
        let h = history(&f)?;
        check_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10100));
        check_eq!(
            f.state()
                .snapshot
                .hold(h.binding().hold())
                .ok_or("hold")?
                .status,
            HoldStatus::Released
        );
        let title = f.state().snapshot.asset(f.shell).ok_or("title")?;
        check_eq!(title.title_owner, PrincipalId::Object(f.buyer));
        check!(title.encumbrances.is_empty());
        check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    }
    Ok(())
}

#[test]
fn expiry_clock_loss_interval_and_claim_delay_refuse_false_terminal_windows() -> Result {
    let mut f = bound()?;
    let expired = CoverageDecisionKindV1::ExpiredClean {
        close: token(203),
        close_digest: DefinitionDigest::from_bytes([50; 32]),
        transition: 2,
    };
    check!(f.terminal(3, expired.clone()).is_err());
    f.world.world_mut().set_now(120);
    let original = loss(&f)?;
    for which in 0..4 {
        let mut bad = original.clone();
        if let CoverageDecisionKindV1::Destroyed {
            occurred_at, zone, ..
        } = &mut bad
        {
            match which {
                0 => occurred_at.at = history(&f)?.consent().terms.starts_at.at - 1,
                1 => occurred_at.at = history(&f)?.consent().terms.ends_at.at,
                2 => occurred_at.clock = ClockRef::WorldIc { generation: 99 },
                _ => *zone = f.rules.wrong.clone(),
            }
        }
        check!(f.terminal(3, bad).is_err());
    }
    f.world.world_mut().set_now(200);
    let p = f.terminal(3, expired)?;
    f.publish(f.rules.actions[2].clone(), 3, p)?;
    check_eq!(posted(&f, 1)?, 9900);
    Ok(())
}

#[test]
fn native_cleanup_selector_can_cross_clock_but_claim_and_expiry_cannot() -> Result {
    let mut f = bound()?;
    let reason = loss(&f)?;
    let epoch = f.world.world().next_world_timeline_epoch(2)?;
    f.world.world_mut().append_world_timeline_epoch(epoch)?;
    check!(f.terminal(3, reason).is_err());
    check!(
        f.terminal(
            3,
            CoverageDecisionKindV1::ExpiredClean {
                close: token(203),
                close_digest: DefinitionDigest::from_bytes([50; 32]),
                transition: 2
            }
        )
        .is_err()
    );
    let p = f.terminal(3, clean())?;
    f.publish(f.rules.actions[2].clone(), 3, p)?;
    check!(history(f.cold(f.boot.clone())?.as_ref())? == history(&f)?);
    Ok(())
}

#[test]
fn registered_action_actor_and_native_title_scope_are_all_required() -> Result {
    for which in 0..6 {
        let mut f = Fixture::new()?;
        let mut basis = f.basis()?;
        let mut terms = f.terms()?;
        let mut action = f.rules.actions[0].clone();
        let mut actor = f.buyer;
        match which {
            0 => action = f.rules.wrong.clone(),
            1 => actor = f.actor,
            2 => basis.hull_title_revision += 1,
            3 => terms.scope = OrgScopeId(99),
            4 => terms.payer = AccountId(3),
            _ => terms.adjudicator = f.buyer,
        }
        check!(
            f.prepare(
                &action,
                actor,
                2,
                Input::Bind {
                    basis: Box::new(basis),
                    terms
                }
            )
            .is_err()
        );
        check_eq!((posted(&f, 1)?, posted(&f, 3)?), (10000, 10000));
        check_eq!(f.state().coverage_histories()?.len(), 0);
    }
    let mut f = bound()?;
    let p = f.prepare(
        &f.rules.actions[2].clone(),
        f.buyer,
        3,
        Input::Terminal {
            binding: token(2),
            reason: clean(),
        },
    );
    check!(p.is_err());
    Ok(())
}

#[test]
fn liability_must_be_prefunded_and_approval_rules_never_become_implicit_consent() -> Result {
    for which in 0..5 {
        let mut f = Fixture::new()?;
        if which == 0 {
            let _ = f.world.run(GOD, "@econ/pay 3=2 2100");
            check_eq!(posted(&f, 3)?, 7900);
        } else {
            let id = AccountId(if which < 3 { 1 } else { 3 });
            let mut a = f
                .state()
                .snapshot
                .account(id)
                .ok_or("approval account")?
                .clone();
            a.approval = Some(crate::ApprovalRule {
                threshold: Amount::from_minor(if which % 2 == 1 { 0 } else { 10000 }),
                policy: mush_org::ApprovalPolicy::NamedSet {
                    approvals: 1,
                    actors: std::collections::BTreeSet::from([GOD]),
                },
                window: (which % 2 == 0)
                    .then(|| crate::ApprovalWindow::new(Amount::ZERO, 3600).expect("window")),
            });
            f.state_mut().snapshot.put_account(a);
        }
        check!(crate::test_checks::refused(f.bind(2))?);
        check_eq!(posted(&f, 1)?, 10000);
        check_eq!(f.state().coverage_histories()?.len(), 0);
    }
    Ok(())
}

#[test]
fn preimages_repeat_at_stage_take_and_consuming_bind_without_partial_money() -> Result {
    for boundary in 0..3 {
        let mut f = Fixture::new()?;
        let pending = f.bind(2)?;
        let action = f.rules.actions[0].clone();
        if boundary == 0 {
            let mut title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
            title.revision += 1;
            f.state_mut().snapshot.put_asset(title);
            check!(f.stage(action, 2, pending).is_err());
        } else {
            f.stage(action, 2, pending)?;
            let held = if boundary == 2 {
                Some(f.world.engine_mut().take_staged_action()?.ok_or("held")?)
            } else {
                None
            };
            f.state_mut().next_transaction += 1;
            if let Some(held) = held {
                let (_, install) = held.into_parts();
                check!(
                    f.world
                        .engine_mut()
                        .install_composed_action(install)
                        .is_err()
                );
            } else {
                check!(f.world.engine_mut().take_staged_action().is_err());
            }
        }
        check_eq!((posted(&f, 1)?, posted(&f, 3)?), (10000, 10000));
        check_eq!(f.state().coverage_histories()?.len(), 0);
    }
    Ok(())
}

#[test]
fn raw_native_inventory_refuses_missing_or_crossed_contract_hold_and_title() -> Result {
    let f = bound()?;
    let h = history(&f)?;
    for which in 0..6 {
        let mut boot = f.boot.clone();
        let native = boot.get_mut("economy").ok_or("native boot")?;
        let keys = [
            crate::api::contract_key(h.contract_id()),
            crate::api::hold_key(h.binding().hold()),
            crate::api::asset_key(f.shell),
        ];
        if which < 3 {
            native.kv.retain(|(key, _)| key != &keys[which]);
        } else {
            let (_, v) = native
                .kv
                .iter_mut()
                .find(|(key, _)| key == &keys[which - 3])
                .ok_or("document")?;
            match which {
                3 => v["name"] = json!("crossed"),
                4 => v["amount"] = serde_json::to_value(Amount::from_minor(7999))?,
                _ => v["revision"] = json!(999),
            }
        }
        check!(crate::test_checks::refused(f.cold(boot))?);
    }
    check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    Ok(())
}

#[test]
fn disabled_owner_cannot_recover_retained_coverage_even_without_fixture_rows() -> Result {
    let f = bound()?;
    let mut boot = f.boot.clone();
    boot.remove("tacsim");
    check!(crate::test_checks::refused(f.cold_enabled(boot, false))?);
    let empty = Fixture::new()?;
    check!(empty.cold_enabled(empty.boot.clone(), false).is_ok());
    Ok(())
}

#[test]
fn sparse_bind_and_terminal_match_journal_fold_and_checkpoint_exact_inventory() -> Result {
    let mut f = Fixture::new()?;
    for terminal in [false, true] {
        let mut full = f.state().fold.clone();
        let pending = if terminal {
            f.terminal(3, clean())?
        } else {
            f.bind(2)?
        };
        f.publish(
            f.rules.actions[if terminal { 2 } else { 0 }].clone(),
            if terminal { 3 } else { 2 },
            pending,
        )?;
        let h = history(&f)?;
        let id = if terminal {
            h.terminal().ok_or("terminal")?.transaction()
        } else {
            h.binding().transaction()
        };
        let tx = f.state().transactions.get(&id).ok_or("actual journal")?;
        full.fold_entry(tx.journal_seq, tx);
        check!(serde_json::to_value(&full)? == serde_json::to_value(&f.state().fold)?);
        let index: Vec<_> = f
            .state()
            .transactions
            .values()
            .map(|t| crate::checkpoint::IdemRow {
                ns: t.idempotency.namespace.clone(),
                key: t.idempotency.key.clone(),
                tx: t.id,
                seq: t.journal_seq,
                posted_at: t.posted_at,
                plan_hash: t.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
            })
            .collect();
        check!(f.state().fold.coverage_receipts.checkpoint_complete(&index));
        let mut missing = index.clone();
        missing.retain(|r| r.tx != id);
        check!(
            !f.state()
                .fold
                .coverage_receipts
                .checkpoint_complete(&missing)
        );
        let mut duplicate = index;
        duplicate.push(duplicate.iter().find(|r| r.tx == id).ok_or("row")?.clone());
        check!(
            !f.state()
                .fold
                .coverage_receipts
                .checkpoint_complete(&duplicate)
        );
    }
    Ok(())
}

#[test]
fn legacy_engine_cannot_release_managed_collateral_or_hold_and_sweep_retains_terminal() -> Result {
    use crate::engine::{NoCustody, NoFacts, ValidateEnv};
    let mut f = bound()?;
    let h = history(&f)?;
    let mut plan = crate::EconomyPlan::new(
        crate::IdempotencyKey::new("ordinary", "coverage-release"),
        PrincipalId::Object(f.buyer),
        EconomicCause::Staff {
            reason: "fixture".into(),
        },
        vec![],
    );
    plan.hold_ops.push(crate::HoldOp::Release {
        hold: h.binding().hold(),
    });
    let gate = f.state().key_gate();
    let env = ValidateEnv {
        auth: &crate::AllowAll,
        custody: &NoCustody,
        facts: &NoFacts,
        now: 100,
        ic_now: 100,
        next_hold_id: f.state().next_hold,
        spent: Default::default(),
        keys: &gate,
    };
    check!(matches!(
        crate::engine::validate(&plan, &f.state().snapshot, &env),
        crate::PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    plan.hold_ops.clear();
    plan.asset_ops.push(crate::AssetOp::Release {
        object: f.shell,
        encumbrance: crate::Encumbrance::Collateral {
            contract: h.contract_id(),
        },
    });
    check!(matches!(
        crate::engine::validate(&plan, &f.state().snapshot, &env),
        crate::PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    let p = f.terminal(3, clean())?;
    f.publish(f.rules.actions[2].clone(), 3, p)?;
    let terminal = history(&f)?;
    let at = f.world.world().now() + crate::HOLD_GC_AFTER_SECS + 60;
    f.world.set_now(at);
    let _ = f.world.fire_due(at * 1000);
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    check!(f.state().snapshot.hold(h.binding().hold()).is_some());
    check!(history(f.cold(f.boot.clone())?.as_ref())? == terminal);
    Ok(())
}

#[test]
fn persisted_consent_requires_explicit_null_and_refuses_unknown_duplicate_and_invalid_terms()
-> Result {
    let f = bound()?;
    let h = history(&f)?;
    let consent = h.consent();
    let raw = serde_json::to_vec(consent)?;
    check!(CoverageConsentV1::decode(&raw)? == *consent);
    for which in 0..8 {
        let mut value = serde_json::to_value(consent)?;
        match which {
            0 => {
                value["basis"]
                    .as_object_mut()
                    .ok_or("basis")?
                    .remove("qualified_role");
            }
            1 => {
                value["basis"]["authority"] = json!(true);
            }
            2 => {
                value["format"] = json!(2);
            }
            3 => {
                value["terms"]["discount"] = serde_json::to_value(Amount::from_minor(1))?;
            }
            4 => {
                value["terms"]["ends_at"] = value["terms"]["starts_at"].clone();
            }
            5 => {
                value["terms"]["expires_at_utc"] = value["terms"]["quoted_at_utc"].clone();
            }
            6 => {
                value["terms"]["payer"] = value["terms"]["treasury"].clone();
            }
            _ => {
                value["basis"]["quote"] = value["basis"]["product"].clone();
            }
        }
        check!(CoverageConsentV1::decode(&serde_json::to_vec(&value)?).is_err());
    }
    let duplicate =
        String::from_utf8(raw)?.replacen("\"format\":1", "\"format\":1,\"format\":1", 1);
    check!(CoverageConsentV1::decode(duplicate.as_bytes()).is_err());
    check!(CoverageConsentV1::decode(&vec![b' '; MAX_SUBJECT_BYTES + 1]).is_err());
    check_eq!(consent.terms.insurer, OrgId(0));
    check_eq!(consent.terms.scope, OrgScopeId(0));
    Ok(())
}

#[test]
fn history_decode_is_bounded_canonical_and_does_not_accept_missing_terminal_or_changed_receipts()
-> Result {
    let f = bound()?;
    let h = history(&f)?;
    let value = serde_json::to_value(&h)?;
    let raw = mush_core::canonical_json_bytes_with_limit(&value, MAX_HISTORY_BYTES)?;
    check!(CoverageHistory::decode(&raw)? == h);
    check!(CoverageHistory::decode(&serde_json::to_vec_pretty(&value)?).is_err());
    for which in 0..4 {
        let mut bad = value.clone();
        match which {
            0 => {
                bad.as_object_mut().ok_or("history")?.remove("terminal");
            }
            1 => {
                bad["binding"]["journal_seq"] = json!(u64::MAX);
            }
            2 => {
                bad["binding"]["title"]["revision"] = json!(u64::MAX);
            }
            _ => {
                bad["binding"]["trusted"] = json!(true);
            }
        }
        let bytes = mush_core::canonical_json_bytes_with_limit(&bad, MAX_HISTORY_BYTES)?;
        check!(CoverageHistory::decode(&bytes).is_err());
    }
    check!(CoverageHistory::decode(&vec![b' '; MAX_HISTORY_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn checkpoint_complete_flag_never_supplies_verified_health_and_duplicate_history_keys_refuse()
-> Result {
    use super::book::CoverageBook;
    let f = bound()?;
    let h = history(&f)?;
    let encoded = serde_json::to_value(&f.state().fold.coverage_receipts)?;
    let mut decoded: CoverageBook = serde_json::from_value(encoded.clone())?;
    check!(decoded.histories().is_err());
    check!(
        decoded
            .finish_replay(
                &Default::default(),
                &Default::default(),
                &Default::default()
            )
            .is_err()
    );
    check!(decoded.histories().is_err());
    let mut forged = encoded;
    forged["verified"] = json!(true);
    check!(serde_json::from_value::<CoverageBook>(forged).is_err());
    let key = serde_json::to_string(&h.consent().operation)?;
    let row = serde_json::to_string(&h)?;
    let duplicate = format!(
        "{{\"histories\":{{{key}:{row},{key}:{row}}},\"complete\":true,\"invalid\":false}}"
    );
    check!(serde_json::from_str::<CoverageBook>(&duplicate).is_err());
    let mut empty: CoverageBook =
        serde_json::from_value(json!({"histories":{},"complete":true,"invalid":false}))?;
    check!(empty.histories().is_err());
    empty.finish_replay(
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )?;
    check_eq!(empty.histories()?.len(), 0);
    Ok(())
}

#[test]
fn bind_reserves_future_title_width_before_charging_at_the_admitted_size_boundary() -> Result {
    let mut f = Fixture::new()?;
    let _ = f.bind(2)?;
    let mut title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
    title.catalog = Some(CatalogRef {
        ruleset: "fixture".into(),
        code: "".into(),
    });
    let fixed = mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(&title)?,
        MAX_TITLE_BYTES,
    )?
    .len();
    title.catalog.as_mut().ok_or("catalog")?.code = "x".repeat(MAX_TITLE_BYTES - fixed).into();
    check_eq!(
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(&title)?,
            MAX_TITLE_BYTES
        )?
        .len(),
        MAX_TITLE_BYTES
    );
    let hull = f.shell;
    f.state_mut()
        .fold
        .journal_assets
        .insert(hull, title.clone());
    f.state_mut().snapshot.put_asset(title);
    check!(crate::test_checks::refused(f.bind(2))?);
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (10000, 10000));
    check_eq!(f.state().coverage_histories()?.len(), 0);
    Ok(())
}

mod ct0_snapshot;
