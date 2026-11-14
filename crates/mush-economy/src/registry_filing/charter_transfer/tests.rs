#![cfg(test)]

//! Native payment/title owner contract, driven through actual Engine composition.
//! This test provider is not the production signed Roll resale offer protocol.
use super::*;
use mush_core::action::{ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput};
use mush_core::action_state::StateFamilyHandle;
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, DefinitionDescriptor, GOD, GameModule, ModError, ObjKind, PrepareCtx, Registrar,
    RulesValue,
};
use mush_testkit::TestWorld;
use mush_testkit::cold::{Boot, fold_replay_writes_into};
use mush_testkit::fixtures::token;
use mush_testkit::funding::Funding;
use mush_testkit::owner_rig::{self, MockOwner, PreparedAction};
use serde::{Deserialize, Serialize};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    initial: DefinitionRef,
    update: DefinitionRef,
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn rules() -> Result<Rules> {
    let mut d = RIG.declare()?;
    let initial = d.add("action", "initial_filing", 1)?;
    let update = d.add("action", ACTION_ID, 1)?;
    let wrong = d.add("action", "wrong_transfer", 1)?;
    let outcome = d.add("outcome", "charter_sold", 1)?;
    Ok(Rules {
        declarations: d.finish(),
        initial,
        update,
        wrong,
        outcome,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    charter: ObjId,
    payer: AccountId,
    title_revision: u64,
    seller: ObjId,
    seller_account: AccountId,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    price: Amount,
}
// Strict test-only typed evidence derived from each actual prepared Economy
// output. It is not a production Roll filing/update record or an authority.
const RECEIPT_FAMILY: &str = "charter_sale_fixture_receipt";
/// This rig's names in the shared owner-rig skeleton (`mush_testkit::owner_rig`).
const RIG: MockOwner = MockOwner {
    owner: "roll",
    label: "native-charter-sale",
    receipts: RECEIPT_FAMILY,
    idempotency: "charter_transfer_fixture",
    refusal: "charter sale fixture refused",
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "subject",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FixtureEffect {
    Initial(Box<super::super::receipt::InitialFilingAuditSubject>),
    Transfer(Box<CharterTransferAuditSubject>),
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
    fn validate(
        &self,
        initial: &DefinitionRef,
        update: &DefinitionRef,
    ) -> std::result::Result<(), String> {
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
            FixtureEffect::Transfer(s)
                if &self.action == update && s.commit == self.commit && s.buyer == self.actor =>
            {
                s.validate().map_err(|_| "update shape")?;
            }
            _ => return Err("fixture receipt affinity".into()),
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        match &self.effect {
            FixtureEffect::Initial(subject) => state.filing_receipt_audit().verify(subject)?,
            FixtureEffect::Transfer(subject) => {
                state.charter_transfer_receipt_audit().verify(subject)?
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
    initial: bool,
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
        let request = if self.initial {
            EconomyRequest::InitialFiling(RegistryInitialFiling {
                shell: r.charter,
                payer: r.payer,
                owner: PrincipalId::Object(inv.actor()),
                scope: None,
            })
        } else {
            EconomyRequest::CharterTransfer(RegistryCharterTransfer {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
                seller: r.seller,
                buyer: inv.actor(),
                seller_account: r.seller_account,
                offer: r.offer,
                offer_digest: r.offer_digest,
                price: r.price,
            })
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        let effect = if self.initial {
            let output = output.initial_filing().map_err(|_| error())?;
            let title = output.title();
            FixtureEffect::Initial(Box::new(
                super::super::receipt::InitialFilingAuditSubject::new(
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
            FixtureEffect::Transfer(Box::new(
                output
                    .charter_transfer()
                    .map_err(|_| error())?
                    .subject()
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
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        owner_rig::register_declarations(reg, &self.declarations, &["action", "outcome"])?;
        let (initial, update) = (self.initial.clone(), self.update.clone());
        let receipts = RIG.receipts(reg, 32, 16_384, move |row: &FixtureReceipt| {
            row.validate(&initial, &update)
        })?;
        let initial =
            reg.request_owner_participant::<InitialFilingParticipant>(self.initial.clone())?;
        let update =
            reg.request_owner_participant::<InitialFilingParticipant>(self.update.clone())?;
        for (action, handle, is_initial) in [
            (self.initial.clone(), initial, true),
            (self.update.clone(), update.clone(), false),
            (self.wrong.clone(), update, false),
        ] {
            reg.action_provider(
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action,
                    handle,
                    initial: is_initial,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
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
                .with_mind_charter_transfers(r.update.clone()),
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
    fn empty() -> Result<Self> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, 100));
        let actor = world.spawn_player("Seller");
        let buyer = world.spawn_player("Buyer");
        let shell =
            world
                .world_mut()
                .create(ObjKind::Thing, "Reserved native charter", GOD, None)?;
        Ok(Self {
            world,
            rules,
            actor,
            buyer,
            shell,
            boot: Boot::new(),
            staged: None,
        })
    }
    fn new() -> Result<Self> {
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
        let p = f.prepare(&action, f.actor, 1)?;
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
    fn prepare(&mut self, action: &DefinitionRef, actor: ObjId, n: u8) -> Result<PreparedAction> {
        let parameters = json!({"charter":self.shell,"payer":if action==&self.rules.initial {AccountId(1)} else {AccountId(3)},"title_revision":self.state().snapshot.asset(self.shell).map_or(0,|a|a.revision),
            "seller":self.actor,"seller_account":AccountId(1),"offer":token(200),"offer_digest":DefinitionDigest::from_bytes([44;32]),"price":Amount::from_minor(500)});
        RIG.prepare(&mut self.world, action, actor, n, parameters)
    }
    fn update(&mut self, n: u8) -> Result<PreparedAction> {
        let action = self.rules.update.clone();
        self.prepare(&action, self.buyer, n)
    }
    fn stage(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let (initial, update) = (&self.rules.initial, &self.rules.update);
        let (writes, receipt) =
            RIG.stage(&mut self.world, action, n, p, |r: &FixtureReceipt| {
                r.validate(initial, update)
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

#[test]
fn native_sale_commits_buyer_money_and_title_together_preserving_birth() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().snapshot.clone();
    let before_birth = serde_json::to_value(f.state().filing_receipt_audit())?;
    let title = before.asset(f.shell).ok_or("title")?.clone();
    let p = f.update(2)?;
    check_eq!(f.state().snapshot, before);
    let writes = f.stage(f.rules.update.clone(), 2, p)?;
    check_eq!(f.state().snapshot, before);
    f.take_install()?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("actual native receipt")?
        .clone();
    f.state()
        .charter_transfer_receipt_audit()
        .verify(&subject)?;
    check_eq!(subject.seller, f.actor);
    check_eq!(subject.buyer, f.buyer);
    check_eq!(subject.title_revision, title.revision);
    check_eq!(subject.title.revision, title.revision + 1);
    check_eq!(subject.title.catalog, title.catalog);
    check_eq!(subject.title.title_owner, PrincipalId::Object(f.buyer));
    check_eq!(f.state().snapshot.asset(f.shell), Some(&subject.title));
    check_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        before_birth
    );
    check_eq!(f.state().snapshot.managed_titles, before.managed_titles);
    for (id, value) in [(1, 10_400), (2, 100), (3, 9500)] {
        check_eq!(
            f.state()
                .snapshot
                .account(AccountId(id))
                .ok_or("account")?
                .posted_balance,
            Amount::from_minor(value)
        );
    }
    check_eq!(
        writes.len(),
        7,
        "native journal, two accounts, title, key, meta and typed witness"
    );
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("native transaction")?;
    check_eq!(receipt::subject_of(tx)?, subject);
    let mut replay = CharterTransferReceiptAudit::default();
    let mut history: Vec<_> = f.state().transactions.values().collect();
    history.sort_by_key(|t| t.journal_seq);
    for entry in history {
        replay.observe(entry);
    }
    check!(replay.verify(&subject).is_err());
    replay.finish_replay()?;
    replay.verify(&subject)?;
    check!(
        f.update(3).is_err(),
        "old seller never retains transfer authority"
    );
    Ok(())
}

#[test]
fn sale_rechecks_title_money_and_birth_at_both_custody_boundaries() -> Result {
    for at_take in [false, true] {
        for change in 0..4 {
            let mut f = Fixture::new()?;
            let p = f.update(2)?;
            let action = f.rules.update.clone();
            let mut prepared = Some(p);
            if at_take {
                f.stage(action.clone(), 2, prepared.take().ok_or("preparation")?)?;
            }
            let shell = f.shell;
            match change {
                0 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .revision += 1
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(3))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                2 => {
                    f.state_mut().snapshot.managed_titles =
                        crate::managed_title::ManagedTitleIndex::Unavailable
                }
                _ => f
                    .world
                    .world_mut()
                    .get_mut(f.actor)
                    .ok_or("seller")?
                    .flags
                    .insert(mush_core::Flags::GOING),
            }
            let before = f.state().snapshot.clone();
            let next = f.state().next_transaction;
            if at_take {
                check!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                check!(
                    f.stage(action, 2, prepared.take().ok_or("preparation")?)
                        .is_err()
                );
            }
            check_eq!(f.state().snapshot, before);
            check_eq!(f.state().next_transaction, next);
            check_eq!(
                f.state().charter_transfer_receipt_audit().receipts()?.len(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn sale_requires_actual_buyer_personal_payer_and_exact_registered_action() -> Result {
    let mut f = Fixture::new()?;
    let action = f.rules.update.clone();
    let seller = f.actor;
    let buyer = f.buyer;
    let other = f.world.spawn_player("Unrelated buyer");
    check!(f.prepare(&action, seller, 2).is_err());
    check!(f.prepare(&action, other, 2).is_err());
    let wrong = f.rules.wrong.clone();
    check!(f.prepare(&wrong, buyer, 2).is_err());
    for id in [1, 3] {
        let original = f
            .state()
            .snapshot
            .account(AccountId(id))
            .ok_or("account")?
            .clone();
        f.state_mut()
            .snapshot
            .account_mut(AccountId(id))
            .ok_or("account")?
            .owner = PrincipalId::Object(other);
        check!(f.update(2).is_err());
        *f.state_mut()
            .snapshot
            .account_mut(AccountId(id))
            .ok_or("account")? = original;
    }
    f.state_mut()
        .snapshot
        .account_mut(AccountId(3))
        .ok_or("payer")?
        .approval = Some(crate::record::ApprovalRule {
        threshold: Amount::from_minor(499),
        policy: mush_org::ApprovalPolicy::NamedSet {
            approvals: 1,
            actors: std::collections::BTreeSet::from([GOD]),
        },
        window: None,
    });
    let before = f.state().snapshot.clone();
    check!(f.update(2).is_err());
    check_eq!(f.state().snapshot, before);
    check_eq!(
        f.state().charter_transfer_receipt_audit().receipts()?.len(),
        0
    );
    Ok(())
}

#[test]
fn sale_cold_replay_preserves_exact_title_and_requires_paired_recovery() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("receipt")?
        .clone();
    let mut cold = Fixture::empty()?;
    check_eq!(cold.shell, f.shell);
    check_eq!(cold.buyer, f.buyer);
    cold.world.try_cold_loaded(f.boot.clone())?;
    cold.state()
        .charter_transfer_receipt_audit()
        .verify(&subject)?;
    check_eq!(
        cold.state().snapshot.asset(f.shell),
        f.state().snapshot.asset(f.shell)
    );
    check_eq!(
        cold.state().snapshot.managed_titles,
        f.state().snapshot.managed_titles
    );
    check_eq!(cold.state().filing_receipt_audit().receipts()?.len(), 1);
    let mut disabled = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::new().with_title_issuers(vec![
            SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        ])),
    ]);
    let _ = disabled.take_module_ops();
    let error = disabled
        .try_modules_loaded(f.boot)
        .err()
        .ok_or("disabled sale history must refuse")?;
    check!(
        error
            .to_string()
            .contains("charter-transfer history requires paired recovery")
    );
    check!(disabled.take_module_ops().is_empty());
    Ok(())
}

#[test]
fn sale_history_checks_complete_inverse_and_rejects_crossed_native_title() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let audit = f.state().charter_transfer_receipt_audit();
    let subject = audit.receipts()?.next().ok_or("receipt")?;
    let row = crate::checkpoint::IdemRow {
        ns: receipt::NAMESPACE.into(),
        key: subject.commit.to_hex().into(),
        plan_hash: subject
            .plan_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        tx: subject.transaction,
        posted_at: subject.posted_at_utc,
        seq: subject.journal_seq,
    };
    let mut rows = journal_rows(&f);
    check!(audit.checkpoint_complete(&rows));
    check!(
        !audit.checkpoint_complete(std::slice::from_ref(&row)),
        "filing inverse missing"
    );
    check!(!audit.checkpoint_complete(&[]));
    check!(!audit.checkpoint_complete(&[row.clone(), row.clone()]));
    check!(!CharterTransferReceiptAudit::default().checkpoint_complete(&[row]));
    check!(CharterTransferReceiptAudit::default().checkpoint_complete(&[]));
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?;
    rows.retain(|r| r.ns.as_str() != receipt::NAMESPACE);
    check!(!audit.checkpoint_complete(&rows));
    for change in 0..7 {
        let mut bad = tx.clone();
        match change {
            0 => bad.idempotency.namespace = "foreign".into(),
            1 => bad.actor = PrincipalId::Object(f.actor),
            2 => bad.lines.get_mut(0).ok_or("line")?.delta = Amount::ZERO,
            3 => {
                bad.asset_effects
                    .get_mut(0)
                    .ok_or("effect")?
                    .post
                    .title_owner = PrincipalId::Object(f.actor)
            }
            4 => bad.asset_effects.get_mut(0).ok_or("effect")?.post.revision += 1,
            5 => {
                bad.asset_effects
                    .get_mut(0)
                    .ok_or("effect")?
                    .post
                    .acquisition = None
            }
            _ => bad.asset_effects.get_mut(0).ok_or("effect")?.post.catalog = None,
        }
        let mut replay = CharterTransferReceiptAudit::default();
        let mut history: Vec<_> = f
            .state()
            .transactions
            .values()
            .filter(|t| t.journal_seq < tx.journal_seq)
            .collect();
        history.sort_by_key(|t| t.journal_seq);
        for entry in history {
            replay.observe(entry);
        }
        replay.observe(&bad);
        check!(replay.finish_replay().is_err());
        replay.observe(tx);
        check!(replay.finish_replay().is_err());
    }
    Ok(())
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
fn legacy_managed_title_mutation_cannot_disappear_from_native_namespace_census() -> Result {
    let mut f = Fixture::new()?;
    let before_sale = f.state().charter_transfer_receipt_audit().clone();
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("sale")?;
    let mut old_generic = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?
        .clone();
    old_generic.idempotency = IdempotencyKey::new("ordinary", "old-sale");
    old_generic.cause = EconomicCause::Ruleset {
        namespace: "ordinary".into(),
        correlation: "old-sale".into(),
    };
    let mut audit = before_sale.clone();
    audit.observe(&old_generic);
    check!(
        audit.finish_replay().is_err(),
        "old generic transfer must poison managed lineage"
    );
    for kind in [
        crate::AssetEffectKind::CatalogSet,
        crate::AssetEffectKind::WrittenOff,
        crate::AssetEffectKind::Abandoned,
        crate::AssetEffectKind::Encumbered,
    ] {
        let mut mutation = old_generic.clone();
        mutation.asset_effects.get_mut(0).ok_or("effect")?.kind = kind;
        let mut audit = before_sale.clone();
        audit.observe(&mutation);
        check!(audit.finish_replay().is_err());
    }
    let rows = journal_rows(&f);
    let mut missing = serde_json::to_value(f.state().charter_transfer_receipt_audit())?;
    missing.as_object_mut().ok_or("audit")?.remove("lineages");
    let missing: CharterTransferReceiptAudit = serde_json::from_value(missing)?;
    check!(!missing.checkpoint_complete(&rows));
    check!(!CharterTransferReceiptAudit::default().checkpoint_complete(&rows));
    Ok(())
}

#[test]
fn public_post_and_exact_key_retries_never_gain_private_title_authority() -> Result {
    let mut f = Fixture::new()?;
    for posted in [false, true] {
        if posted {
            let p = f.update(2)?;
            f.publish(f.rules.update.clone(), 2, p)?;
        }
        let request = RegistryCharterTransfer {
            offer: token(200),
            offer_digest: DefinitionDigest::from_bytes([44; 32]),
            charter: f.shell,
            title_revision: 1,
            seller: f.actor,
            buyer: f.buyer,
            payer: AccountId(3),
            seller_account: AccountId(1),
            price: Amount::from_minor(500),
        };
        let plan = plan(&policy(100), f.buyer, token(2), &request);
        let gate = f.state().key_gate();
        let env = ValidateEnv {
            auth: &crate::AllowAll,
            custody: &NoCustody,
            facts: &NoFacts,
            now: f.world.world().now(),
            ic_now: f.world.world().game_now_unix(),
            next_hold_id: f.state().next_hold,
            spent: BTreeMap::new(),
            keys: &gate,
        };
        check!(matches!(
            crate::engine::validate(&plan, &f.state().snapshot, &env),
            PlanResult::Refused(crate::EconomyError::ManagedContract)
        ));
    }
    Ok(())
}
