#![cfg(test)]

//! Real registered Economy monetary owner and cold-recovery tests. The Market
//! receipt fixture is inert: it does not prove a paid licence, seller offer,
//! claim census, Registry sanction or a production service intent.
use super::*;
use crate::record::{AccountStatus, CatalogRef, HoldStatus};
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
use mush_testkit::owner_rig::{self, InertOwner, MockOwner, PreparedAction, Unavailable};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::{Deserialize, Serialize};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

use crate::managed_entitlement::{GrossRecipientCut, Recipient};

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    actions: [DefinitionRef; 5],
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
        d.add("action", OFFER, 1)?,
        d.add("action", ACCEPT, 1)?,
        d.add("action", FUND, 1)?,
        d.add("action", SETTLE, 1)?,
        d.add("action", REFUND, 1)?,
    ];
    let wrong = d.add("action", "wrong_paid_resale", 1)?;
    let outcome = d.add("outcome", "paid_resale_fixture", 1)?;
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
    Accept {
        terms: Box<PaidResaleTermsV1>,
        payer: AccountId,
        claims: PaidResaleClaimsV1,
        mode: PaidResaleModeV1,
    },
    Terminal {
        acceptance: CommitToken,
        reason: PaidResaleDecisionKindV1,
    },
}
const RECEIPT_FAMILY: &str = "paid_resale_fixture_receipt";
/// This rig's names in the shared owner-rig skeleton (`mush_testkit::owner_rig`).
const RIG: MockOwner = MockOwner {
    owner: "market",
    label: "native-paid-resale",
    receipts: RECEIPT_FAMILY,
    idempotency: "paid_resale_fixture",
    refusal: "paid resale owner fixture refused",
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    history: PaidResaleHistoryV1,
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
        if state.paid_resale_history(self.history.consent().operation)? != &self.history {
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
            Input::Accept {
                terms,
                payer,
                claims,
                mode,
            } => EconomyRequest::PaidResaleAcceptance(Box::new(PaidResaleAcceptanceRequest {
                terms: *terms,
                payer,
                claims,
                mode,
                offer_digest: DefinitionDigest::from_bytes([44; 32]),
            })),
            Input::Terminal { acceptance, reason } => {
                let w = ctx.world().map_err(|_| error())?;
                EconomyRequest::PaidResaleTerminal(PaidResaleTerminalRequest {
                    acceptance,
                    decision: PaidResaleDecisionV1 {
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
            .paid_resale()
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
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        owner_rig::register_declarations(reg, &self.declarations, &["action", "outcome"])?;
        reg.action_provider(self.actions[0].clone(), Unavailable(error()))?;
        let receipts = RIG.receipts(reg, 128, 65_536, FixtureReceipt::validate)?;
        let handle =
            reg.request_owner_participant::<InitialFilingParticipant>(self.actions[2].clone())?;
        for action in self.actions.iter().skip(1) {
            let h = if action == &self.actions[2] {
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
        modules.push(Box::new(economy.with_paid_mind_resale(
            PaidResaleConfig::new(issuer, GOD, r.actions.clone(), None).expect("resale config"),
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
        let actor = world.spawn_player("Resale seller");
        let buyer = world.spawn_player("Resale buyer");
        let shell = world
            .world_mut()
            .create(ObjKind::Thing, "Inert paid basis", GOD, None)?;
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
        Funding::instrument("cr", 0)
            .system("faucet")
            .open(f.actor)
            .system("roll.registry")
            .mint(0, 1, 10000)
            .open(f.buyer)
            .mint(0, 3, 10000)
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
    fn terms(&self) -> Result<PaidResaleTermsV1> {
        let w = self.world.world();
        let digest = DefinitionDigest::from_bytes([45; 32]);
        let term = PaidResaleTermBasisV1 {
            generation: 1,
            operation: token(1),
            digest,
            clock: w.world_ic_now().clock,
            window: PaidResaleWindowV1 {
                format: 1,
                eligibility_start: 0,
                issued_at: 0,
                term_end: 1000,
                grace_end: 1100,
            },
            terms_digest: digest,
            policy_digest: digest,
        };
        let row = PaidResaleTermsV1 {
            format: 1,
            offer: token(200),
            offer_action: self.rules.actions[0].clone(),
            basis: PaidResaleBasisV1 {
                licence: 1,
                charter: self.shell,
                line: 1,
                birth: PaidBirthRefV1 {
                    route: PaidBirthRouteV1::Legacy,
                    operation: token(1),
                    digest,
                },
                holder: PaidHolderRefV1 {
                    generation: 1,
                    operation: token(1),
                    digest,
                },
                term,
                economics_digest: digest,
                royalty_chain_digest: digest,
            },
            seller: self.actor,
            buyer: self.buyer,
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(503),
            registry: GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: AccountId(2),
                    owner: PrincipalId::System(
                        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
                    ),
                }),
            },
            venue: None,
            parents: vec![],
            remainder: Recipient {
                account: AccountId(1),
                owner: PrincipalId::Object(self.actor),
            },
            offered_at_ic: w.world_ic_now(),
            offered_at_utc: w.now(),
            expires_at_utc: w.now() + 3600,
            settle_seconds: SETTLEMENT_IC_SECONDS,
        };
        row.validate()?;
        Ok(row)
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
    fn accept(&mut self, n: u8, mode: PaidResaleModeV1) -> Result<PreparedAction> {
        let action = self.rules.actions[if mode == PaidResaleModeV1::Immediate {
            1
        } else {
            2
        }]
        .clone();
        let terms = self.terms()?;
        self.prepare(
            &action,
            self.buyer,
            n,
            Input::Accept {
                terms: Box::new(terms),
                payer: AccountId(3),
                claims: PaidResaleClaimsV1 {
                    next_transition: 4,
                    open_paid: if mode == PaidResaleModeV1::Immediate {
                        0
                    } else {
                        1
                    },
                },
                mode,
            },
        )
    }
    fn terminal(&mut self, n: u8, reason: PaidResaleDecisionKindV1) -> Result<PreparedAction> {
        let action = self.rules.actions[if reason.settles() { 3 } else { 4 }].clone();
        self.prepare(
            &action,
            GOD,
            n,
            Input::Terminal {
                acceptance: token(2),
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

fn funded() -> Result<Box<Fixture>> {
    let mut f = Fixture::new()?;
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
    Ok(f)
}
fn posted(f: &Fixture, id: u64) -> Result<i64> {
    Ok(f.state()
        .snapshot
        .account(AccountId(id))
        .ok_or("account")?
        .posted_balance
        .minor())
}
fn history(f: &Fixture) -> Result<PaidResaleHistoryV1> {
    Ok(f.state().paid_resale_history(token(2))?.clone())
}

#[test]
fn resale_immediate_debits_gross_and_credits_exact_split_without_title_or_hold() -> Result {
    let mut f = Fixture::new()?;
    let p = f.accept(2, PaidResaleModeV1::Immediate)?;
    check_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10000, 0, 10000)
    );
    let writes = f.publish(f.rules.actions[1].clone(), 2, p)?;
    check!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    check_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10478, 25, 9497)
    );
    let h = history(&f)?;
    check_eq!(h.outcome(), PaidResaleOutcome::Settled);
    check!(h.acceptance().hold().is_none() && h.terminal().is_none());
    check!(f.state().snapshot.asset(f.shell).is_none());
    let tx = f
        .state()
        .transactions
        .get(&h.acceptance().transaction())
        .ok_or("actual transaction")?;
    check!(
        tx.asset_effects.is_empty() && tx.hold_effects.is_empty() && tx.custody_effects.is_empty()
    );
    check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    Ok(())
}

#[test]
fn resale_pending_holds_gross_then_native_last_close_pays_and_replays() -> Result {
    let mut f = funded()?;
    let before = history(&f)?;
    let hold = before.acceptance().hold().ok_or("held id")?;
    check_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10000, 0, 10000)
    );
    check_eq!(
        f.state().snapshot.hold(hold).ok_or("held")?.status,
        HoldStatus::Held
    );
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    let after = history(&f)?;
    check_eq!(after.consent(), before.consent());
    check_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10478, 25, 9497)
    );
    check!(matches!(
        f.state().snapshot.hold(hold).ok_or("terminal hold")?.status,
        HoldStatus::Consumed { .. }
    ));
    check!(history(f.cold(f.boot.clone())?.as_ref())? == after);
    check!(
        f.terminal(
            4,
            PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 6 }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn resale_refund_uses_retained_money_when_buyer_unavailable_or_clock_changed() -> Result {
    for changed_clock in [false, true] {
        let mut f = funded()?;
        let reason = if changed_clock {
            let epoch = f.world.world().next_world_timeline_epoch(2)?;
            f.world.world_mut().append_world_timeline_epoch(epoch)?;
            PaidResaleDecisionKindV1::ClockChanged {}
        } else {
            let buyer = f.buyer;
            f.world
                .world_mut()
                .get_mut(buyer)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            let _ = f.world.world_mut().take_dirty();
            PaidResaleDecisionKindV1::RecipientUnavailable {}
        };
        check!(
            f.terminal(
                3,
                PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 }
            )
            .is_err()
        );
        let p = f.terminal(3, reason)?;
        f.publish(f.rules.actions[4].clone(), 3, p)?;
        let h = history(&f)?;
        check_eq!(h.outcome(), PaidResaleOutcome::Refunded);
        check_eq!(
            (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
            (10000, 0, 10000)
        );
        check_eq!(
            f.state()
                .snapshot
                .hold(h.acceptance().hold().ok_or("id")?)
                .ok_or("hold")?
                .status,
            HoldStatus::Released
        );
        check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    }
    Ok(())
}

#[test]
fn resale_deadline_is_actual_and_lapsed_property_keeps_immutable_dates() -> Result {
    let mut f = funded()?;
    let reason = PaidResaleDecisionKindV1::DeadlineReached {
        next_transition: 4,
        open_paid: 1,
    };
    check!(f.terminal(3, reason.clone()).is_err());
    f.world.world_mut().set_now(1000);
    let p = f.terminal(3, reason)?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    check_eq!(history(&f)?.outcome(), PaidResaleOutcome::Settled);
    let mut f = Fixture::new()?;
    f.world.world_mut().set_now(1200);
    let terms = f.terms()?;
    let original = terms.basis.term.clone();
    let action = f.rules.actions[1].clone();
    let p = f.prepare(
        &action,
        f.buyer,
        2,
        Input::Accept {
            terms: Box::new(terms),
            payer: AccountId(3),
            claims: PaidResaleClaimsV1 {
                next_transition: 4,
                open_paid: 0,
            },
            mode: PaidResaleModeV1::Immediate,
        },
    )?;
    f.publish(action, 2, p)?;
    let h = history(&f)?;
    check_eq!(h.consent().terms.basis.term, original);
    check!(h.consent().accepted_at_ic.at >= original.window.grace_end);
    check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    Ok(())
}

#[test]
fn resale_actor_action_offer_reuse_and_false_refund_reasons_refuse() -> Result {
    let mut f = Fixture::new()?;
    let input = || Input::Accept {
        terms: Box::new(f.terms().expect("terms")),
        payer: AccountId(3),
        claims: PaidResaleClaimsV1 {
            next_transition: 4,
            open_paid: 1,
        },
        mode: PaidResaleModeV1::Pending,
    };
    let wrong_actor = input();
    let wrong_action = input();
    check!(
        f.prepare(&f.rules.actions[2].clone(), f.actor, 2, wrong_actor)
            .is_err()
    );
    check!(
        f.prepare(&f.rules.wrong.clone(), f.buyer, 2, wrong_action)
            .is_err()
    );
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
    for reason in [
        PaidResaleDecisionKindV1::ClockChanged {},
        PaidResaleDecisionKindV1::RecipientUnavailable {},
        PaidResaleDecisionKindV1::MonetaryBindingChanged {},
    ] {
        check!(f.terminal(3, reason).is_err());
    }
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    check!(
        f.accept(4, PaidResaleModeV1::Immediate).is_err(),
        "used offer remains consumed"
    );
    Ok(())
}

#[test]
fn resale_money_preimages_and_live_actor_repeat_at_stage_take_and_bind() -> Result {
    for boundary in 0..3 {
        for drift in 0..2 {
            let mut f = Fixture::new()?;
            let p = f.accept(2, PaidResaleModeV1::Immediate)?;
            let action = f.rules.actions[1].clone();
            let held = if boundary == 2 {
                f.stage(action.clone(), 2, p)?;
                Some(f.world.engine_mut().take_staged_action()?.ok_or("held")?)
            } else {
                if boundary == 1 {
                    f.stage(action.clone(), 2, p)?;
                } else {
                    if drift == 0 {
                        let mut a = f
                            .state()
                            .snapshot
                            .account(AccountId(1))
                            .ok_or("seller account")?
                            .clone();
                        a.status = AccountStatus::Frozen;
                        f.state_mut().snapshot.put_account(a);
                    } else {
                        let buyer = f.buyer;
                        f.world
                            .world_mut()
                            .get_mut(buyer)
                            .ok_or("buyer")?
                            .flags
                            .insert(mush_core::Flags::GOING);
                        let _ = f.world.world_mut().take_dirty();
                    }
                    check!(f.stage(action, 2, p).is_err());
                    check_eq!(posted(&f, 3)?, 10000);
                    continue;
                }
                None
            };
            if drift == 0 {
                let mut a = f
                    .state()
                    .snapshot
                    .account(AccountId(1))
                    .ok_or("seller account")?
                    .clone();
                a.status = AccountStatus::Frozen;
                f.state_mut().snapshot.put_account(a);
            } else if boundary < 2 {
                let buyer = f.buyer;
                f.world
                    .world_mut()
                    .get_mut(buyer)
                    .ok_or("buyer")?
                    .flags
                    .insert(mush_core::Flags::GOING);
                let _ = f.world.world_mut().take_dirty();
            } else {
                // After custody, only local mutable owner images may invalidate
                // bind; authority has already been taken by the real engine.
                f.state_mut().next_transaction += 1;
            }
            if let Some(held) = held {
                let (_, installation) = held.into_parts();
                check!(
                    f.world
                        .engine_mut()
                        .install_composed_action(installation)
                        .is_err()
                );
            } else {
                check!(f.world.engine_mut().take_staged_action().is_err());
            }
            check_eq!(posted(&f, 3)?, 10000);
            check_eq!(f.state().paid_resale_histories()?.len(), 0);
        }
    }
    Ok(())
}

#[test]
fn resale_cold_inventory_refuses_missing_or_crossed_native_documents() -> Result {
    let f = funded()?;
    let h = history(&f)?;
    for which in 0..5 {
        let mut boot = f.boot.clone();
        let economy = boot.get_mut("economy").ok_or("economy boot")?;
        let contract_key = crate::api::contract_key(h.contract_id());
        let hold_key = crate::api::hold_key(h.acceptance().hold().ok_or("hold")?);
        match which {
            0 => economy.kv.retain(|(key, _)| key != &contract_key),
            1 => economy.kv.retain(|(key, _)| key != &hold_key),
            2 => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &contract_key)
                    .ok_or("contract")?;
                v["name"] = json!("crossed");
            }
            3 => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &hold_key)
                    .ok_or("hold")?;
                v["amount"] = serde_json::to_value(Amount::from_minor(502))?;
            }
            _ => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &hold_key)
                    .ok_or("hold")?;
                v["holder"]["namespace"] = json!("foreign.owner");
            }
        }
        check!(
            crate::test_checks::refused(f.cold(boot))?,
            "native inverse audit must refuse corrupted document"
        );
    }
    check!(history(f.cold(f.boot.clone())?.as_ref())? == h);
    Ok(())
}

#[test]
fn resale_sparse_install_matches_full_fold_and_checkpoint_requires_exact_history() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().fold.clone();
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
    let a = history(&f)?.acceptance().transaction();
    let tx = f
        .state()
        .transactions
        .get(&a)
        .ok_or("actual funding transaction")?
        .clone();
    let mut full = before;
    full.fold_entry(tx.journal_seq, &tx);
    check!(
        serde_json::to_value(&full)? == serde_json::to_value(&f.state().fold)?,
        "sparse and replay folds differ"
    );
    let index: Vec<_> = f
        .state()
        .transactions
        .values()
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            tx: tx.id,
            seq: tx.journal_seq,
            posted_at: tx.posted_at,
            plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
        })
        .collect();
    check!(
        f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&index)
    );
    let mut missing = index.clone();
    missing.retain(|r| r.ns.as_str() != NAMESPACE);
    check!(
        !f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&missing)
    );
    let mut duplicate = index;
    let row = duplicate
        .iter()
        .find(|r| r.ns.as_str() == NAMESPACE)
        .ok_or("row")?
        .clone();
    duplicate.push(row);
    check!(
        !f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&duplicate)
    );
    Ok(())
}

#[test]
fn resale_namespace_blocks_legacy_posts_and_real_sweep_retains_terminal_hold() -> Result {
    use crate::engine::{NoCustody, NoFacts, ValidateEnv};
    use crate::plan::PlanResult;
    let mut f = funded()?;
    let h = history(&f)?;
    {
        let plan = plans::acceptance(h.intent())?;
        let gate = f.state().key_gate();
        let env = ValidateEnv {
            auth: &crate::AllowAll,
            custody: &NoCustody,
            facts: &NoFacts,
            now: f.world.world().now(),
            ic_now: f.world.world().game_now_unix(),
            next_hold_id: f.state().next_hold,
            spent: Default::default(),
            keys: &gate,
        };
        check!(matches!(
            crate::engine::validate(&plan, &f.state().snapshot, &env),
            PlanResult::Refused(crate::EconomyError::ManagedContract)
        ));
    }
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    let actual = history(&f)?;
    let hold = actual.acceptance().hold().ok_or("hold")?;
    let at = f
        .world
        .world()
        .now()
        .checked_add(crate::HOLD_GC_AFTER_SECS + 60)
        .ok_or("gc clock")?;
    f.world.set_now(at);
    let _ = f.world.fire_due(at.checked_mul(1000).ok_or("gc millis")?);
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    check!(f.state().snapshot.hold(hold).is_some());
    check!(f.state().fold.journal_holds.contains_key(&hold));
    check!(history(f.cold(f.boot.clone())?.as_ref())? == actual);
    Ok(())
}

#[test]
fn resale_disabled_boot_refuses_retained_native_history_without_fixture_rows() -> Result {
    let f = funded()?;
    let mut boot = f.boot.clone();
    // Remove the inert fixture family's namespace so refusal must come from
    // native retained history, not an unavailable typed fixture registration.
    boot.remove("market");
    check!(crate::test_checks::refused(f.cold_enabled(boot, false))?);
    let empty = Fixture::new()?;
    check!(empty.cold_enabled(empty.boot.clone(), false).is_ok());
    Ok(())
}

#[test]
fn resale_six_real_payees_include_separate_gross_buyer_royalty_and_parent_title_fences() -> Result {
    use crate::managed_entitlement::{RoyaltyRecipientCut, RoyaltyTitleRecipient};
    for invalidate_parent in [false, true] {
        let mut f = Fixture::new()?;
        let first = f.world.spawn_player("First royalty recipient");
        let second = f.world.spawn_player("Second royalty recipient");
        Funding::of("cr")
            .system("resale.venue")
            .open(first)
            .open(second)
            .run(&mut f.world)?;
        let mut parents = Vec::new();
        for (i, (owner, account, rate)) in [(first, 5, 30), (second, 6, 30), (f.buyer, 3, 40)]
            .into_iter()
            .enumerate()
        {
            let object = f.world.world_mut().create(
                ObjKind::Thing,
                &format!("Native royalty title {i}"),
                GOD,
                None,
            )?;
            Funding::of("cr")
                .line(format!("@asset/title #{}=#{}", object.0, owner.0))
                .run(&mut f.world)?;
            let title = f.state().snapshot.asset(object).ok_or("actual title")?;
            parents.push(RoyaltyRecipientCut {
                charter: object,
                permille: rate,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: title.revision,
                    recipient: Recipient {
                        account: AccountId(account),
                        owner: PrincipalId::Object(owner),
                    },
                }),
            });
        }
        fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
        let _ = f.world.world_mut().take_dirty();
        let mut terms = f.terms()?;
        terms.venue = Some(GrossRecipientCut {
            permille: 50,
            recipient: Some(Recipient {
                account: AccountId(4),
                owner: PrincipalId::System(SystemPrincipalId::new("resale.venue").ok_or("venue")?),
            }),
        });
        terms.parents = parents;
        let title_id = terms.parents[0].charter;
        let p = f.prepare(
            &f.rules.actions[2].clone(),
            f.buyer,
            2,
            Input::Accept {
                terms: Box::new(terms),
                payer: AccountId(3),
                claims: PaidResaleClaimsV1 {
                    next_transition: 4,
                    open_paid: 1,
                },
                mode: PaidResaleModeV1::Pending,
            },
        )?;
        f.publish(f.rules.actions[2].clone(), 2, p)?;
        let h = history(&f)?;
        check_eq!(
            f.state()
                .snapshot
                .hold(h.acceptance().hold().ok_or("id")?)
                .ok_or("hold")?
                .amount
                .minor(),
            503,
            "buyer royalty must not net the gross hold"
        );
        if invalidate_parent {
            let mut title = f.state().snapshot.asset(title_id).ok_or("title")?.clone();
            title.revision += 1;
            f.state_mut().snapshot.put_asset(title);
            check!(
                f.terminal(
                    3,
                    PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 }
                )
                .is_err()
            );
            let p = f.terminal(3, PaidResaleDecisionKindV1::MonetaryBindingChanged {})?;
            f.publish(f.rules.actions[4].clone(), 3, p)?;
            check_eq!(posted(&f, 3)?, 10000);
        } else {
            let p = f.terminal(
                3,
                PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
            )?;
            f.publish(f.rules.actions[3].clone(), 3, p)?;
            check_eq!(
                (
                    posted(&f, 1)?,
                    posted(&f, 2)?,
                    posted(&f, 3)?,
                    posted(&f, 4)?,
                    posted(&f, 5)?,
                    posted(&f, 6)?
                ),
                (10403, 25, 9517, 25, 15, 15)
            );
            let after = history(&f)?;
            let tx = f
                .state()
                .transactions
                .get(&after.terminal().ok_or("terminal")?.transaction())
                .ok_or("transaction")?;
            check_eq!(tx.lines.len(), 7);
            let buyer_lines: Vec<_> = tx
                .lines
                .iter()
                .filter(|l| l.account == AccountId(3))
                .map(|l| l.delta.minor())
                .collect();
            check_eq!(buyer_lines.len(), 2);
            check!(buyer_lines.contains(&-503) && buyer_lines.contains(&20));
        }
    }
    Ok(())
}

mod holder_renewal;
