#![cfg(test)]

//! Actual registered Economy owner funding/terminal/recovery tests. The typed
//! Roll fixture is inert evidence of the actual opaque owner output. It does not
//! claim production seller offers, claim census, Registry sanction or service intent.
use super::*;
use crate::record::{AccountStatus, CatalogRef, EconomicCause, HoldStatus};
use crate::registry_filing::InitialFilingAuditSubject;
use crate::registry_filing::{
    EconomyRequest, InitialFilingParticipant, InitialFilingPolicy, RegistryInitialFiling,
};
use crate::{AccountId, Amount, EconomyStore, InstrumentId};
use mush_core::action::{ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput};
use mush_core::action_state::StateFamilyHandle;
use mush_core::authoritative::ReplayWrite;
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, DefinitionDescriptor, GOD, GameModule, ModError, ObjKind, PrepareCtx, Registrar,
    RulesValue,
};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_testkit::cold::{Boot, fold_replay_writes_into};
use mush_testkit::fixtures::token;
use mush_testkit::funding::Funding;
use mush_testkit::owner_rig::{self, MockOwner, PreparedAction, Unavailable};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::{Deserialize, Serialize};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    initial: DefinitionRef,
    offer: DefinitionRef,
    immediate: DefinitionRef,
    fund: DefinitionRef,
    settle: DefinitionRef,
    refund: DefinitionRef,
    revoke: DefinitionRef,
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn rules() -> Result<Rules> {
    let mut d = RIG.declare()?;
    let initial = d.add("action", "initial_filing", 1)?;
    let offer = d.add("action", OFFER_ACTION, 2)?;
    let immediate = d.add("action", "accept_mind_charter_transfer", 1)?;
    let fund = d.add("action", FUND_ACTION, 1)?;
    let settle = d.add("action", SETTLE_ACTION, 1)?;
    let refund = d.add("action", REFUND_ACTION, 1)?;
    let revoke = d.add("action", "registry_term_revocation", 1)?;
    let wrong = d.add("action", "wrong_pending_transfer", 1)?;
    let outcome = d.add("outcome", "charter_pending_fixture", 1)?;
    Ok(Rules {
        declarations: d.finish(),
        initial,
        offer,
        immediate,
        fund,
        settle,
        refund,
        revoke,
        wrong,
        outcome,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Initial {
        shell: ObjId,
        payer: AccountId,
    },
    Fund {
        terms: Box<CharterPendingTermsV1>,
        payer: AccountId,
    },
    Terminal {
        funding: CommitToken,
        reason: CharterPendingDecisionKindV1,
    },
}
// Strict test-only typed evidence derived from each actual prepared Economy
// output. It is not a production Roll filing/update record or an authority.
const RECEIPT_FAMILY: &str = "charter_pending_fixture_receipt";
/// This rig's names in the shared owner-rig skeleton (`mush_testkit::owner_rig`).
const RIG: MockOwner = MockOwner {
    owner: "roll",
    label: "native-charter-pending",
    receipts: RECEIPT_FAMILY,
    idempotency: "charter_pending_fixture",
    refusal: "charter pending owner fixture refused",
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "subject",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FixtureEffect {
    Initial(Box<InitialFilingAuditSubject>),
    Pending(Box<CharterPendingHistory>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    effect: FixtureEffect,
}
impl FixtureReceipt {
    fn validate(&self, initial: &DefinitionRef) -> std::result::Result<(), String> {
        if self.format != 1 || self.actor.0 == 0 {
            return Err("fixture receipt shape".into());
        }
        match &self.effect {
            FixtureEffect::Initial(s) if &self.action == initial && s.commit() == self.commit => {
                let data = serde_json::to_value(s).map_err(|_| "initial encoding")?;
                if data.get("issued_owner") != Some(&json!(PrincipalId::Object(self.actor))) {
                    return Err("initial actor differs".into());
                }
            }
            FixtureEffect::Pending(h) => {
                h.validate().map_err(|_| "pending history shape")?;
                let (action, commit, actor) = match h.terminal() {
                    None => (
                        &h.consent().effect,
                        h.consent().operation,
                        h.consent().terms.buyer,
                    ),
                    Some(t) => (
                        &t.decision().action,
                        t.decision().operation,
                        t.decision().actor,
                    ),
                };
                if &self.action != action || self.commit != commit || self.actor != actor {
                    return Err("pending output affinity".into());
                }
            }
            _ => return Err("fixture receipt affinity".into()),
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        match &self.effect {
            FixtureEffect::Initial(subject) => state.filing_receipt_audit().verify(subject)?,
            FixtureEffect::Pending(h) => {
                if state.pending_charter_history(h.consent().operation)? != h.as_ref() {
                    return Err("actual native history differs from prepared output".into());
                }
            }
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
        let r: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let is_initial = matches!(&r, Input::Initial { .. });
        let request = match r {
            Input::Initial { shell, payer } => {
                EconomyRequest::InitialFiling(RegistryInitialFiling {
                    shell,
                    payer,
                    owner: PrincipalId::Object(inv.actor()),
                    scope: None,
                })
            }
            Input::Fund { terms, payer } => {
                EconomyRequest::CharterPendingFunding(CharterPendingFundingRequest {
                    terms: *terms,
                    payer,
                })
            }
            Input::Terminal { funding, reason } => {
                let world = ctx.world().map_err(|_| error())?;
                EconomyRequest::CharterPendingTerminal(CharterPendingTerminalRequest {
                    funding,
                    decision: CharterPendingDecisionV1 {
                        format: 1,
                        operation: ctx.commit_token(),
                        action: self.action.clone(),
                        actor: inv.actor(),
                        at_ic: world.world_ic_now(),
                        posted_at_utc: world.now(),
                        reason,
                    },
                })
            }
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        let effect = if is_initial {
            let output = output.initial_filing().map_err(|_| error())?;
            let title = output.title();
            FixtureEffect::Initial(Box::new(
                InitialFilingAuditSubject::new(
                    output.commit_token(),
                    output.transaction(),
                    output.journal_seq(),
                    title.object,
                    title.title_owner.clone(),
                    title.revision,
                    output.payer(),
                    output.sink(),
                    output.fee(),
                    output.issuer().clone(),
                    title.catalog.clone().ok_or_else(error)?,
                )
                .map_err(|_| error())?,
            ))
        } else {
            FixtureEffect::Pending(Box::new(
                output
                    .charter_pending()
                    .map_err(|_| error())?
                    .history()
                    .clone(),
            ))
        };
        let value = serde_json::to_value(&effect).map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &ctx.commit_token(),
            FixtureReceipt {
                format: 1,
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor: inv.actor(),
                effect,
            },
        )
        .map_err(|_| error())?;
        let result = RulesValue::new(self.outcome.clone(), value).map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(result).with_definitions(vec![self.outcome.clone()]))
    }
}
// The actual Economy pending configuration retains the immediate-transfer
// prerequisite grant. This owner-only fixture never exercises that action;
// register its exact provider while keeping it unavailable to invocations.
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        owner_rig::register_declarations(reg, &self.declarations, &["action", "outcome"])?;
        reg.action_provider(self.immediate.clone(), Unavailable(error()))?;
        let initial = self.initial.clone();
        let receipts = RIG.receipts(reg, 128, 65_536, move |row: &FixtureReceipt| {
            row.validate(&initial)
        })?;
        let fund_handle =
            reg.request_owner_participant::<InitialFilingParticipant>(self.fund.clone())?;
        for action in [
            &self.initial,
            &self.fund,
            &self.settle,
            &self.refund,
            &self.revoke,
        ] {
            let handle = if action == &self.fund {
                fund_handle.clone()
            } else {
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?
            };
            reg.action_provider(
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action: action.clone(),
                    handle,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
        let handle = fund_handle;
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
fn policy(fee: i64) -> InitialFilingPolicy {
    InitialFilingPolicy::new(
        SystemPrincipalId::new("roll.registry").expect("issuer"),
        AccountId(2),
        Amount::from_minor(fee),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("policy")
}
fn modules(r: &Rules, fee: i64) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(r.clone()),
        Box::new(
            crate::EconomyModule::new()
                .with_title_issuers(vec![
                    SystemPrincipalId::new("roll.registry").expect("issuer"),
                ])
                .with_initial_filing(policy(fee), r.initial.clone())
                .with_mind_charter_transfers(r.immediate.clone())
                .with_mind_charter_pending(
                    CharterPendingConfig::new(
                        SystemPrincipalId::new("roll.registry").expect("issuer"),
                        CatalogRef {
                            ruleset: "roll".into(),
                            code: "mind_charter".into(),
                        },
                        GOD,
                        [r.fund.clone(), r.settle.clone(), r.refund.clone()],
                        Some(r.revoke.clone()),
                    )
                    .expect("exact pending config"),
                ),
        ),
    ]
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
    fn empty() -> Result<Box<Self>> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, 100));
        world.world_mut().set_now(100);
        world
            .world_mut()
            .set_game_clock(mush_core::GameClock::default());
        let actor = world.spawn_player("Seller");
        let buyer = world.spawn_player("Buyer");
        let shell =
            world
                .world_mut()
                .create(ObjKind::Thing, "Reserved native charter", GOD, None)?;
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
        let action = f.rules.initial.clone();
        let p = f.prepare(
            &action,
            f.actor,
            1,
            Input::Initial {
                shell: f.shell,
                payer: AccountId(1),
            },
        )?;
        f.publish(action, 1, p)?;
        Ok(f)
    }
    fn state(&self) -> &EconomyStore {
        self.world
            .world()
            .module_state::<EconomyStore>()
            .expect("actual economy")
    }
    fn state_mut(&mut self) -> &mut EconomyStore {
        self.world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("actual economy")
    }
    fn terms(&self) -> Result<CharterPendingTermsV1> {
        let w = self.world.world();
        let row = CharterPendingTermsV1 {
            format: 1,
            offer: token(200),
            offer_action: self.rules.offer.clone(),
            offer_digest: DefinitionDigest::from_bytes([44; 32]),
            charter: self.shell,
            birth: token(1),
            ownership: CharterOwnershipBasisV1 {
                generation: 1,
                operation: token(1),
                digest: DefinitionDigest::from_bytes([45; 32]),
            },
            owner_term: CharterTermBasisV1 {
                licence: 1,
                generation: 1,
                operation: token(1),
                digest: DefinitionDigest::from_bytes([46; 32]),
                term_end: w.world_ic_now().at + 1000,
                grace_end: w.world_ic_now().at + 2000,
            },
            title_revision: self
                .state()
                .snapshot
                .asset(self.shell)
                .ok_or("title")?
                .revision,
            seller: self.actor,
            buyer: self.buyer,
            seller_account: AccountId(1),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(500),
            offered_at_ic: w.world_ic_now(),
            offered_at_utc: w.now(),
            expires_at_utc: w.now() + 3600,
            settle_seconds: 60,
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
    fn fund(&mut self, n: u8) -> Result<PreparedAction> {
        self.prepare(
            &self.rules.fund.clone(),
            self.buyer,
            n,
            Input::Fund {
                terms: Box::new(self.terms()?),
                payer: AccountId(3),
            },
        )
    }
    fn terminal(&mut self, n: u8, reason: CharterPendingDecisionKindV1) -> Result<PreparedAction> {
        let action = if reason.settles() {
            self.rules.settle.clone()
        } else {
            self.rules.refund.clone()
        };
        self.prepare(
            &action,
            GOD,
            n,
            Input::Terminal {
                funding: token(2),
                reason,
            },
        )
    }
    fn cold(&self, boot: Boot) -> Result<Box<Self>> {
        let mut world = self.world.cold_restart(modules(&self.rules, 100), boot)?;
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
        let initial = &self.rules.initial;
        let (writes, receipt) =
            RIG.stage(&mut self.world, action, n, p, |r: &FixtureReceipt| {
                r.validate(initial)
            })?;
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
    let p = f.fund(2)?;
    f.publish(f.rules.fund.clone(), 2, p)?;
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
fn history(f: &Fixture) -> Result<CharterPendingHistory> {
    Ok(f.state().pending_charter_history(token(2))?.clone())
}
fn journal_rows(f: &Fixture) -> Vec<crate::checkpoint::IdemRow> {
    f.state()
        .transactions
        .values()
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
            tx: tx.id,
            posted_at: tx.posted_at,
            seq: tx.journal_seq,
        })
        .collect()
}
#[test]
fn pending_fund_installs_one_real_buyer_acceptance_and_hold_without_title_or_debit() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().snapshot.clone();
    let p = f.fund(2)?;
    check!(f.state().snapshot == before, "preparation must not publish");
    let writes = f.stage(f.rules.fund.clone(), 2, p)?;
    check!(f.state().snapshot == before, "staging must not publish");
    check!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    f.take_install()?;
    let h = history(&f)?;
    let c = f
        .state()
        .snapshot
        .contract(h.contract_id())
        .ok_or("actual contract")?;
    let held = f
        .state()
        .snapshot
        .hold(h.funding().hold())
        .ok_or("actual hold")?;
    h.verify_documents(c, held)?;
    check_eq!(c.consent.parties, vec![PrincipalId::Object(f.buyer)]);
    check_eq!(c.acceptances.len(), 1);
    check_eq!(
        c.acceptances.first().ok_or("acceptance")?.party,
        PrincipalId::Object(f.buyer)
    );
    check!(c.consent.deadline.is_none());
    check_eq!(held.status, HoldStatus::Held);
    check!(held.expires.is_none());
    check!(f.state().snapshot.asset(f.shell) == before.asset(f.shell));
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10000));
    check_eq!(
        f.state()
            .snapshot
            .account(AccountId(3))
            .ok_or("payer")?
            .available(),
        Some(Amount::from_minor(9500))
    );
    check!(h.terminal().is_none());
    // No service, claim or seller-consent authority is inferred from this
    // fixture's inert typed witness. Production Roll must prove those joins.
    Ok(())
}
#[test]
fn pending_deadline_is_positive_and_real_settlement_moves_title_and_money_once() -> Result {
    let mut f = funded()?;
    check!(
        f.terminal(
            3,
            CharterPendingDecisionKindV1::DeadlineReached {
                next_transition: 1,
                open_owned: 2
            }
        )
        .is_err()
    );
    let h = history(&f)?;
    let deadline = h.consent().deadline.at;
    f.world.world_mut().set_now(deadline);
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::DeadlineReached {
            next_transition: 1,
            open_owned: 2,
        },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let h = history(&f)?;
    let t = h.terminal().ok_or("actual terminal")?;
    check!(t.decision().reason.settles());
    check_eq!(
        f.state()
            .snapshot
            .asset(f.shell)
            .ok_or("title")?
            .title_owner,
        PrincipalId::Object(f.buyer)
    );
    check_eq!((posted(&f, 1)?, posted(&f, 3)?), (10400, 9500));
    check_eq!(
        f.state()
            .snapshot
            .account(AccountId(3))
            .ok_or("payer")?
            .held_amount,
        Amount::ZERO
    );
    check!(matches!(
        f.state()
            .snapshot
            .hold(h.funding().hold())
            .ok_or("hold")?
            .status,
        HoldStatus::Consumed { .. }
    ));
    check!(
        f.terminal(
            4,
            CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 }
        )
        .is_err()
    );
    let cold = f.cold(f.boot.clone())?;
    check!(history(&cold)? == h);
    check!(cold.state().snapshot.asset(f.shell) == f.state().snapshot.asset(f.shell));
    check_eq!(
        cold.state()
            .charter_transfer_receipt_audit()
            .receipts()?
            .len(),
        0,
        "immediate v1 receipt namespace stays distinct"
    );
    Ok(())
}
#[test]
fn pending_last_claim_close_and_already_due_term_cap_settle_without_waiting_more() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 8 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    check_eq!(posted(&f, 3)?, 9500);
    let mut f = Fixture::new()?;
    let mut terms = f.terms()?;
    terms.owner_term.term_end = terms.offered_at_ic.at - 1;
    terms.owner_term.grace_end = terms.offered_at_ic.at + 100;
    let p = f.prepare(
        &f.rules.fund.clone(),
        f.buyer,
        2,
        Input::Fund {
            terms: Box::new(terms),
            payer: AccountId(3),
        },
    )?;
    f.publish(f.rules.fund.clone(), 2, p)?;
    check!(
        history(&f)?
            .consent()
            .deadline_reached(&f.world.world().world_ic_now())?
    );
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::DeadlineReached {
            next_transition: 3,
            open_owned: 1,
        },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    check_eq!(posted(&f, 3)?, 9500);
    Ok(())
}
#[test]
fn pending_clock_and_recipient_refunds_release_real_hold_without_transfer() -> Result {
    for changed_clock in [false, true] {
        let mut f = funded()?;
        let title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
        let reason = if changed_clock {
            let epoch = f.world.world().next_world_timeline_epoch(2)?;
            f.world.world_mut().append_world_timeline_epoch(epoch)?;
            CharterPendingDecisionKindV1::ClockChanged
        } else {
            let buyer = f.buyer;
            f.world
                .world_mut()
                .get_mut(buyer)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            // Model an already-published external lifecycle change, so the
            // refund reaches actual owner custody rather than DirtyWorld.
            let _ = f.world.world_mut().take_dirty();
            CharterPendingDecisionKindV1::RecipientUnavailable
        };
        check!(
            f.terminal(
                3,
                CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 }
            )
            .is_err()
        );
        let p = f.terminal(3, reason)?;
        f.publish(f.rules.refund.clone(), 3, p)?;
        check_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10000));
        check!(f.state().snapshot.asset(f.shell) == Some(&title));
        let h = history(&f)?;
        check_eq!(
            f.state()
                .snapshot
                .hold(h.funding().hold())
                .ok_or("hold")?
                .status,
            HoldStatus::Released
        );
        let cold = f.cold(f.boot.clone())?;
        check!(history(&cold)? == h);
    }
    Ok(())
}
#[test]
fn pending_exact_registry_term_refund_is_in_same_actual_owner_commit() -> Result {
    let mut f = funded()?;
    let basis = history(&f)?.consent().terms.owner_term.clone();
    let reason = CharterPendingDecisionKindV1::RegistryRevoked {
        sanction: token(3),
        licence: basis.licence,
        generation: basis.generation,
        term_digest: basis.digest,
    };
    let p = f.prepare(
        &f.rules.revoke.clone(),
        f.actor,
        3,
        Input::Terminal {
            funding: token(2),
            reason,
        },
    )?;
    f.publish(f.rules.revoke.clone(), 3, p)?;
    let terminal = history(&f)?.terminal().ok_or("terminal")?.clone();
    check_eq!(terminal.decision().operation, token(3));
    check_eq!(terminal.decision().action, f.rules.revoke);
    check_eq!(posted(&f, 3)?, 10000);
    // This is the real configured E4 owner leg. The fixture does not fabricate
    // a Registry sanction: production Roll's separate same-union proof is owed.
    Ok(())
}
#[test]
fn pending_funding_rechecks_actual_payer_title_and_clock_at_stage_and_take() -> Result {
    for at_take in [false, true] {
        for change in 0..4 {
            let mut f = Fixture::new()?;
            let p = f.fund(2)?;
            let mut prepared = Some(p);
            if at_take {
                f.stage(f.rules.fund.clone(), 2, prepared.take().ok_or("prepared")?)?;
            }
            let shell = f.shell;
            match change {
                0 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(3))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .revision += 1
                }
                2 => {
                    let epoch = f.world.world().next_world_timeline_epoch(2)?;
                    f.world.world_mut().append_world_timeline_epoch(epoch)?;
                }
                _ => {
                    let now = f.world.world().now();
                    f.world.world_mut().set_now(now - 1);
                }
            }
            let _ = f.world.world_mut().take_dirty();
            if at_take {
                check!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                check!(
                    f.stage(f.rules.fund.clone(), 2, prepared.take().ok_or("prepared")?)
                        .is_err()
                );
            }
            check!(f.state().pending_charter_histories()?.next().is_none());
            check_eq!(
                f.state()
                    .snapshot
                    .account(AccountId(3))
                    .ok_or("payer")?
                    .held_amount,
                Amount::ZERO
            );
        }
    }
    Ok(())
}
#[test]
fn pending_post_ack_native_drift_cannot_publish_any_prepared_money_or_title() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.stage(f.rules.settle.clone(), 3, p)?;
    let held = f.world.engine_mut().take_staged_action()?.ok_or("taken")?;
    let (_, install) = held.into_parts();
    let shell = f.shell;
    f.state_mut()
        .snapshot
        .asset_mut(shell)
        .ok_or("title")?
        .revision += 1;
    let before = f.state().snapshot.clone();
    check!(
        f.world
            .engine_mut()
            .install_composed_action(install)
            .is_err()
    );
    check!(
        f.state().snapshot == before,
        "failed consuming bind must not apply any owner image"
    );
    check!(history(&f)?.terminal().is_none());
    Ok(())
}
#[test]
fn pending_cold_recovery_refuses_missing_crossed_extra_and_disabled_native_history() -> Result {
    let f = funded()?;
    let h = history(&f)?;
    let good = f.cold(f.boot.clone())?;
    check!(history(&good)? == h);
    for change in 0..4 {
        let mut boot = f.boot.clone();
        let data = boot.get_mut("economy").ok_or("economic boot")?;
        let hold_key = crate::api::hold_key(h.funding().hold());
        let contract_key = crate::api::contract_key(h.contract_id());
        match change {
            0 => data.kv.retain(|(k, _)| k != &hold_key),
            1 => data.kv.retain(|(k, _)| k != &contract_key),
            2 => {
                let (_, v) = data
                    .kv
                    .iter_mut()
                    .find(|(k, _)| k == &hold_key)
                    .ok_or("hold doc")?;
                let mut hold: crate::record::Hold = serde_json::from_value(v.clone())?;
                hold.holder.namespace = "foreign".into();
                *v = serde_json::to_value(hold)?;
            }
            _ => {
                let mut hold = f
                    .state()
                    .snapshot
                    .hold(h.funding().hold())
                    .ok_or("hold")?
                    .clone();
                hold.id = crate::HoldId(1000);
                hold.holder.namespace = "foreign".into();
                data.kv
                    .push((crate::api::hold_key(hold.id), serde_json::to_value(hold)?));
            }
        }
        check!(crate::test_checks::refused(f.cold(boot))?);
    }
    let mut disabled = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::new()),
    ]);
    let _ = disabled.take_module_ops();
    check!(disabled.try_modules_loaded(f.boot.clone()).is_err());
    check!(disabled.take_module_ops().is_empty());
    Ok(())
}
#[test]
fn pending_actual_journal_and_checkpoint_inverse_keep_replay_latch_and_full_envelopes() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let actual = history(&f)?;
    let audit = &f.state().fold.charter_pending_receipts;
    let rows = journal_rows(&f);
    check!(audit.checkpoint_complete(&rows));
    let retained = serde_json::to_value(audit)?;
    let mut parsed: history::CharterPendingAudit = serde_json::from_value(retained)?;
    check!(
        parsed.get(token(2)).is_err(),
        "serialized health never grants a live latch"
    );
    parsed.finish_replay()?;
    check!(parsed.get(token(2))? == &actual);
    check!(!audit.checkpoint_complete(&[]));
    let mut missing = rows.clone();
    missing.retain(|r| r.ns.as_str() != NAMESPACE);
    check!(!audit.checkpoint_complete(&missing));
    let fund = rows
        .iter()
        .find(|r| r.ns.as_str() == NAMESPACE)
        .ok_or("journal key")?
        .clone();
    let mut duplicate = rows.clone();
    duplicate.push(fund);
    check!(!audit.checkpoint_complete(&duplicate));
    let mut txs: Vec<_> = f.state().transactions.values().cloned().collect();
    txs.sort_by_key(|t| t.journal_seq);
    let mut replay = history::CharterPendingAudit::default();
    for tx in &txs {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    check!(replay.get(token(2))? == &actual);
    let raw = mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(&actual)?,
        MAX_PENDING_HISTORY_BYTES,
    )?;
    check!(CharterPendingHistory::decode(&raw)? == actual);
    check!(serde_json::to_vec(&actual.contract()?)?.len() <= MAX_PENDING_CONTRACT_BYTES);
    for tx in txs
        .iter()
        .filter(|t| t.idempotency.namespace.as_str() == NAMESPACE)
    {
        let EconomicCause::Ruleset { correlation, .. } = &tx.cause else {
            return Err("native cause".into());
        };
        check!(correlation.len() <= MAX_PENDING_CORRELATION_BYTES);
    }
    Ok(())
}

#[test]
fn pending_funding_requires_the_actual_charter_birth_and_exact_configured_action() -> Result {
    let mut f = Fixture::new()?;
    let before_state = f.state().snapshot.clone();
    for change in 0..3 {
        let mut terms = f.terms()?;
        let mut action = f.rules.fund.clone();
        let mut actor = f.buyer;
        match change {
            0 => {
                // A self-consistent source-free request still cannot replace
                // the actual native filing birth with another lawful token.
                terms.birth = token(90);
                terms.ownership.operation = token(90);
                terms.validate()?;
            }
            1 => action = f.rules.wrong.clone(),
            _ => actor = f.actor,
        }
        check!(
            f.prepare(
                &action,
                actor,
                20 + change,
                Input::Fund {
                    terms: Box::new(terms),
                    payer: AccountId(3),
                }
            )
            .is_err()
        );
        check!(
            f.state().snapshot == before_state,
            "refusal changed native state"
        );
        check!(
            f.world.take_module_ops().is_empty(),
            "refusal published native documents"
        );
    }
    let actual = f.fund(2)?;
    f.publish(f.rules.fund.clone(), 2, actual)?;
    let h = history(&f)?;
    check_eq!(h.consent().terms.birth, token(1));
    check!(f.cold(f.boot.clone()).is_ok());
    Ok(())
}

#[test]
fn pending_namespace_refuses_legacy_replay_and_terminal_hold_survives_real_sweep() -> Result {
    use crate::engine::{NoCustody, NoFacts, ValidateEnv};
    use crate::plan::PlanResult;
    let mut f = funded()?;
    let before = history(&f)?;
    let plan = plans::funding(before.intent())?;
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
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let actual = history(&f)?;
    let at = f
        .world
        .world()
        .now()
        .checked_add(crate::HOLD_GC_AFTER_SECS + 60)
        .ok_or("gc clock")?;
    f.world.set_now(at);
    let _ = f
        .world
        .fire_due(at.checked_mul(1000).ok_or("gc milliseconds")?);
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    check!(f.state().snapshot.hold(actual.funding().hold()).is_some());
    check!(
        f.state()
            .fold
            .journal_holds
            .contains_key(&actual.funding().hold())
    );
    check!(history(&f)? == actual);
    check!(history(f.cold(f.boot.clone())?.as_ref())? == actual);
    Ok(())
}
