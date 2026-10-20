//! Real Economy module recovery of managed purchase history. No test opens the
//! receipt audit: only normal empty boot or complete journal/checkpoint replay
//! can establish it. E4 owner preparation/install is real; the cross-owner
//! Market/Roll transaction and server writer are tested separately.

use super::*;
use std::collections::HashMap;

use mush_core::action::{ActionError, ActionErrorCode, ActionInvocation, ActionProvider};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::BoundParticipant;
use mush_core::{
    Audience, GOD, GameModule, LedgerChunk, ModError, ModuleBootData, ModuleOp, ModuleWrite,
    Registrar,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde_json::{Value, json};
use smol_str::SmolStr;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type BootMap = HashMap<SmolStr, ModuleBootData>;

fn module_error(error: impl std::fmt::Display) -> ModError {
    ModError::msg(error.to_string())
}
fn descriptor(id: &str) -> TestResult<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("action")?,
        DefinitionId::new(id)?,
        1,
        json!({"fixture":"purchase-boot","action":id}),
    )?)
}
fn action(owner: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new(owner)?, descriptor(id)?)?)
}
fn purchase_actions() -> TestResult<[DefinitionRef; 3]> {
    Ok([
        action("market", "purchase_original_mind")?,
        action("market", "settle_original_mind")?,
        action("market", "refund_original_mind")?,
    ])
}
struct Declarations(&'static str, &'static [&'static str]);
struct DeclarationOnly(ActionError);
impl ActionProvider for DeclarationOnly {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Err(self.0.clone())
    }
}
impl GameModule for Declarations {
    fn name(&self) -> &'static str {
        self.0
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = self
            .1
            .iter()
            .map(|id| action(self.0, id))
            .collect::<TestResult<Vec<_>>>()
            .map_err(module_error)?;
        reg.definition_validator(DefinitionKind::new("action").map_err(module_error)?,
            move |context: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                let entry = context.definition();
                if expected.iter().any(|reference| reference == entry.reference())
                    && entry.payload() == &json!({"fixture":"purchase-boot","action":entry.reference().key().id.as_str()})
                    && entry.references().is_empty() { Ok(()) }
                else { Err("closed boot-fixture action differs".into()) }
            })?;
        for id in self.1 {
            let reference = reg.definition(descriptor(id).map_err(module_error)?)?;
            let refusal = ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The E4 boot fixture does not expose commercial actions.",
            )
            .map_err(module_error)?;
            reg.action_provider(reference, DeclarationOnly(refusal))?;
        }
        Ok(())
    }
}
fn world(purchases: bool) -> TestResult<Box<TestWorld>> {
    let issuer = mush_org::SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
    let policy = crate::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(3),
        crate::Amount::from_minor(100),
        crate::CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?;
    let mut economy = crate::EconomyModule::new()
        .with_config(crate::EconomyConfig {
            tx_tail_max: 3,
            checkpoint_every_entries: 4,
            ..Default::default()
        })
        .with_title_issuers(vec![issuer])
        .with_initial_filing(policy, action("roll", "initial_filing")?);
    if purchases {
        economy = economy.with_original_mind_purchases(purchase_actions()?);
    }
    Ok(Box::new(TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(Declarations("roll", &["initial_filing"])),
        Box::new(Declarations(
            "market",
            &[
                "purchase_original_mind",
                "settle_original_mind",
                "refund_original_mind",
            ],
        )),
        Box::new(economy),
    ])))
}
fn store(tw: &TestWorld) -> TestResult<&EconomyStore> {
    tw.world()
        .module_state::<EconomyStore>()
        .ok_or_else(|| "Economy owner".into())
}
fn run(tw: &mut TestWorld, command: &str) {
    let events = tw.run(GOD, command);
    let error = |(_, event): &(Audience, OutputEvent)| {
        matches!(event,
        OutputEvent::Sys(text) if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    };
    assert!(!events.iter().any(error), "{command}: {events:?}");
}
fn objects(tw: &mut TestWorld) -> TestResult<(ObjId, ObjId)> {
    let seller = tw
        .world_mut()
        .create(ObjKind::Player, "Seller", GOD, None)?;
    let charter = tw
        .world_mut()
        .create(ObjKind::Thing, "Acknowledged charter", GOD, None)?;
    Ok((seller, charter))
}
fn fold(tw: &mut TestWorld, disk: &mut BootMap) {
    fold_module_ops_into(disk, tw.take_module_ops());
}
fn persist(disk: &mut BootMap, writes: Vec<ReplayWrite>) -> TestResult {
    let operations = writes
        .into_iter()
        .map(|write| {
            Ok(match write {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value,
                    },
                },
                ReplayWrite::LedgerChunkSet { write } => ModuleOp {
                    ns: write.namespace().as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: write.key().as_str().into(),
                        value: serde_json::to_value(write.chunk())?,
                    },
                },
                _ => {
                    return Err(
                        "purchase E4 only writes module JSON and actual ledger chunks".into(),
                    );
                }
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    fold_module_ops_into(disk, operations);
    Ok(())
}
struct Fixture {
    tw: Box<TestWorld>,
    disk: BootMap,
    purchase: OriginalPurchase,
}
impl Fixture {
    fn new(purchases: bool) -> TestResult<Self> {
        let mut tw = world(purchases)?;
        let (seller, charter) = objects(&mut tw)?;
        tw.set_now(1000);
        run(&mut tw, "@instrument/register cr=2");
        run(&mut tw, "@account/system faucet=cr");
        run(&mut tw, "@account/open #1=cr");
        run(&mut tw, &format!("@account/open #{}=cr", seller.0));
        run(&mut tw, "@account/system roll.registry=cr");
        run(&mut tw, "@econ/mint 0=1 10000");
        run(
            &mut tw,
            &format!("@asset/title #{}=#{}", charter.0, seller.0),
        );
        let title = store(&tw)?
            .snapshot()
            .asset(charter)
            .ok_or("actual title")?;
        assert_eq!(title.title_owner, PrincipalId::Object(seller));
        let purchase = OriginalPurchase {
            quote: CommitToken::from_bytes([2; 16]),
            quote_digest: DefinitionDigest::for_canonical_value(
                "test.purchase.quote",
                &json!({"quote":2}),
            )?,
            product_digest: DefinitionDigest::for_canonical_value(
                "test.purchase.product",
                &json!({"charter":charter}),
            )?,
            buyer: GOD,
            seller,
            charter,
            title_revision: title.revision,
            payer: AccountId(1),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: crate::Amount::from_minor(100),
            payouts: vec![
                PurchasePayout {
                    account: AccountId(2),
                    owner: PrincipalId::Object(seller),
                    amount: crate::Amount::from_minor(90),
                },
                PurchasePayout {
                    account: AccountId(3),
                    owner: PrincipalId::System(
                        mush_org::SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
                    ),
                    amount: crate::Amount::from_minor(10),
                },
            ],
            quoted_at_utc: 999,
            expires_at_utc: 1100,
        };
        let mut disk = BootMap::new();
        fold(&mut tw, &mut disk);
        Ok(Self { tw, disk, purchase })
    }
    fn fund(&mut self) -> TestResult<FundingOutput> {
        let before = store(&self.tw)?.snapshot().clone();
        let action = action("market", "purchase_original_mind")?;
        let prepared = prepare_funding_in(
            store(&self.tw)?,
            &FundingContext {
                world: self.tw.world(),
                actor: GOD,
                principal: &mush_core::PrincipalId::Object { id: GOD },
                action: &action,
                commit: CommitToken::from_bytes([3; 16]),
            },
            self.purchase.clone(),
        )?;
        let (output, patch, writes) = install::finish_images(
            store(&self.tw)?,
            self.tw.world().world_ic_now().clock,
            prepared,
        )?;
        assert_eq!(
            store(&self.tw)?.snapshot(),
            &before,
            "preparing complete owner writes is inert"
        );
        assert_eq!(
            store(&self.tw)?.purchase_receipt_audit().receipts()?.len(),
            0
        );
        patch
            .bind(
                self.tw
                    .world_mut()
                    .module_state_mut::<EconomyStore>()
                    .ok_or("owner")?,
            )?
            .apply();
        persist(&mut self.disk, writes)?;
        check_history(&self.tw, &self.purchase, &output, None)?;
        Ok(output)
    }
    fn terminal(
        &mut self,
        funding: &FundingOutput,
        outcome: PurchaseOutcome,
    ) -> TestResult<TerminalOutput> {
        let action = action(
            "market",
            match outcome {
                PurchaseOutcome::Granted => "settle_original_mind",
                PurchaseOutcome::Refunded => "refund_original_mind",
            },
        )?;
        let before = store(&self.tw)?.snapshot().clone();
        let count = store(&self.tw)?.purchase_receipt_audit().receipts()?.len();
        let prepared = terminal::prepare_in(
            store(&self.tw)?,
            &FundingContext {
                world: self.tw.world(),
                actor: GOD,
                principal: &mush_core::PrincipalId::Object { id: GOD },
                action: &action,
                commit: CommitToken::from_bytes([4; 16]),
            },
            PurchaseTerminal {
                operation: funding.operation(),
                purchase: self.purchase.clone(),
                funding: funding.receipt().clone(),
            },
            outcome,
        )?;
        let (output, patch, writes) = install::finish_terminal_images(
            store(&self.tw)?,
            self.tw.world().world_ic_now().clock,
            prepared,
        )?;
        assert_eq!(store(&self.tw)?.snapshot(), &before);
        assert!(
            store(&self.tw)?
                .purchase_receipt_audit()
                .get(funding.operation())?
                .terminal()
                .is_none()
        );
        patch
            .bind(
                self.tw
                    .world_mut()
                    .module_state_mut::<EconomyStore>()
                    .ok_or("owner")?,
            )?
            .apply();
        persist(&mut self.disk, writes)?;
        assert_eq!(
            store(&self.tw)?.purchase_receipt_audit().receipts()?.len(),
            count,
            "terminal uses its funding proof slot"
        );
        check_history(&self.tw, &self.purchase, funding, Some(outcome))?;
        Ok(output)
    }
    fn checkpoint(&mut self) -> TestResult<(u64, u64, SmolStr)> {
        for _ in 0..120 {
            run(&mut self.tw, "@econ/mint 0=1 1");
        }
        let now = self.tw.world().now() + 1;
        self.tw.set_now(now);
        self.tw
            .fire_due(now.checked_mul(1000).ok_or("timer milliseconds")?);
        let saved = store(&self.tw)?.checkpoint().ok_or("actual checkpoint")?;
        let covered = saved.covered_seq;
        let head = saved.head_key.clone();
        let floor = store(&self.tw)?
            .chunk_of_seq(covered.checked_sub(2).ok_or("checkpoint floor")?)
            .ok_or("chunk")?;
        assert!(floor > 0, "fixture actually excludes old journal chunks");
        fold(&mut self.tw, &mut self.disk);
        Ok((covered, floor, head))
    }
    fn restarted(&self, purchases: bool, disk: BootMap) -> TestResult<Box<TestWorld>> {
        let mut tw = world(purchases)?;
        assert_eq!(
            objects(&mut tw)?,
            (self.purchase.seller, self.purchase.charter)
        );
        tw.set_now(self.tw.world().now());
        tw.try_modules_loaded(disk)?;
        assert!(!store(&tw)?.is_frozen());
        Ok(tw)
    }
}
fn check_history(
    tw: &TestWorld,
    purchase: &OriginalPurchase,
    funding: &FundingOutput,
    outcome: Option<PurchaseOutcome>,
) -> TestResult {
    let owner = store(tw)?;
    let audit = owner.purchase_receipt_audit();
    audit.require_complete()?;
    let history = audit.get(funding.operation())?;
    assert!(history.matches_purchase(purchase, funding.receipt()));
    assert_eq!(
        history.effect(),
        &action("market", "purchase_original_mind")?
    );
    history.verify_documents(
        owner
            .snapshot()
            .contract(funding.receipt().contract())
            .ok_or("contract")?,
        owner
            .snapshot()
            .hold(funding.receipt().hold())
            .ok_or("hold")?,
    )?;
    match (outcome, history.terminal()) {
        (None, None) => {}
        (Some(outcome), Some(terminal)) => {
            assert_eq!(terminal.commit(), CommitToken::from_bytes([4; 16]));
            assert_eq!(terminal.granted(), outcome == PurchaseOutcome::Granted);
            assert_eq!(
                terminal.issued_at_ic().is_some(),
                outcome == PurchaseOutcome::Granted
            );
        }
        _ => return Err("exact funding/terminal phase".into()),
    }
    Ok(())
}
fn narrow(mut disk: BootMap, floor: u64) -> TestResult<BootMap> {
    let data = disk.get_mut("economy").ok_or("economy documents")?;
    data.ledger.retain(|(key, _)| {
        mush_core::parse_chunk_key(key)
            .is_none_or(|(log, chunk)| log != JOURNAL_LOG || chunk >= floor)
    });
    data.omitted.insert(JOURNAL_LOG.into(), floor);
    Ok(disk)
}
fn no_checkpoint(mut disk: BootMap) -> TestResult<BootMap> {
    disk.get_mut("economy")
        .ok_or("economy documents")?
        .kv
        .retain(|(key, _)| !crate::checkpoint::is_checkpoint_key(key));
    Ok(disk)
}
fn older_than_loaded(disk: &BootMap, seq: u64) -> TestResult {
    let data = disk.get("economy").ok_or("economy")?;
    assert!(!data.ledger.is_empty());
    for (key, raw) in &data.ledger {
        if mush_core::parse_chunk_key(key).is_some_and(|(log, _)| log == JOURNAL_LOG) {
            let chunk: LedgerChunk = serde_json::from_str(raw)?;
            assert!(chunk.entries.iter().all(|row| row.seq > seq));
        }
    }
    Ok(())
}
fn same_purchase_projection(
    a: &TestWorld,
    b: &TestWorld,
    p: &OriginalPurchase,
    f: &FundingOutput,
) -> TestResult {
    let a = store(a)?;
    let b = store(b)?;
    assert_eq!(
        a.purchase_receipt_audit().get(f.operation())?,
        b.purchase_receipt_audit().get(f.operation())?
    );
    for account in [AccountId(1), AccountId(2), AccountId(3)] {
        assert_eq!(a.snapshot().account(account), b.snapshot().account(account));
    }
    assert_eq!(a.snapshot().asset(p.charter), b.snapshot().asset(p.charter));
    assert_eq!(
        a.snapshot().hold(f.receipt().hold()),
        b.snapshot().hold(f.receipt().hold())
    );
    assert_eq!(
        a.snapshot().contract(f.receipt().contract()),
        b.snapshot().contract(f.receipt().contract())
    );
    Ok(())
}

#[test]
fn funding_and_each_terminal_survive_cold_checkpoint_and_full_replay() -> TestResult {
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut f = Fixture::new(true)?;
        let funding = f.fund()?;
        let (covered, floor, _) = f.checkpoint()?;
        assert!(
            store(&f.tw)?
                .transaction(funding.receipt().transaction())
                .is_none(),
            "actual hot-tail eviction"
        );
        let narrowed = narrow(f.disk.clone(), floor)?;
        older_than_loaded(&narrowed, funding.receipt().journal_seq())?;
        let cold = f.restarted(true, narrowed)?;
        assert_eq!(
            store(&cold)?
                .boot_replay()
                .ok_or("boot report")?
                .from_checkpoint,
            Some(covered)
        );
        check_history(&cold, &f.purchase, &funding, None)?;
        let full = f.restarted(true, no_checkpoint(f.disk.clone())?)?;
        assert_eq!(
            store(&full)?
                .boot_replay()
                .ok_or("boot report")?
                .from_checkpoint,
            None
        );
        same_purchase_projection(&cold, &full, &f.purchase, &funding)?;
        // Terminal preparation now uses history actually recovered from a
        // checkpoint, with the funding body absent from the hot transaction map.
        f.tw = cold;
        // Any ordinary boot projection writes precede the new terminal image.
        fold(&mut f.tw, &mut f.disk);
        assert!(
            store(&f.tw)?
                .transaction(funding.receipt().transaction())
                .is_none()
        );
        let terminal = f.terminal(&funding, outcome)?;
        let (covered, floor, _) = f.checkpoint()?;
        assert!(store(&f.tw)?.transaction(terminal.transaction()).is_none());
        run(&mut f.tw, "@econ/mint 0=1 1");
        fold(&mut f.tw, &mut f.disk);
        let narrowed = narrow(f.disk.clone(), floor)?;
        older_than_loaded(&narrowed, terminal.journal_seq())?;
        let cold = f.restarted(true, narrowed)?;
        let replay = store(&cold)?.boot_replay().ok_or("boot report")?;
        assert_eq!(replay.from_checkpoint, Some(covered));
        assert_eq!(replay.replayed, 1);
        check_history(&cold, &f.purchase, &funding, Some(outcome))?;
        let full = f.restarted(true, no_checkpoint(f.disk.clone())?)?;
        check_history(&full, &f.purchase, &funding, Some(outcome))?;
        same_purchase_projection(&cold, &full, &f.purchase, &funding)?;
        let payer = store(&full)?
            .snapshot()
            .account(f.purchase.payer)
            .ok_or("payer")?;
        assert_eq!(payer.held_amount, crate::Amount::ZERO);
        assert_eq!(
            payer.posted_balance.minor(),
            10_241
                - if outcome == PurchaseOutcome::Granted {
                    100
                } else {
                    0
                },
            "two real 120-post checkpoint advances plus the single tail post, then exactly one capture or release"
        );
        assert_eq!(
            store(&full)?
                .snapshot()
                .account(AccountId(2))
                .ok_or("seller")?
                .posted_balance
                .minor(),
            if outcome == PurchaseOutcome::Granted {
                90
            } else {
                0
            }
        );
        assert_eq!(
            store(&full)?
                .snapshot()
                .account(AccountId(3))
                .ok_or("registry")?
                .posted_balance
                .minor(),
            if outcome == PurchaseOutcome::Granted {
                10
            } else {
                0
            }
        );
    }
    Ok(())
}

#[test]
fn old_checkpoint_without_purchase_proof_requires_actual_full_fetch() -> TestResult {
    let mut f = Fixture::new(false)?;
    let (_, floor, head) = f.checkpoint()?;
    let raw = doc(&f.disk, &head)?;
    assert!(raw.pointer("/body/fold/purchase_receipts").is_none());
    let legacy = f.restarted(false, narrow(f.disk.clone(), floor)?)?;
    assert!(
        store(&legacy)?
            .boot_replay()
            .ok_or("boot report")?
            .from_checkpoint
            .is_some()
    );
    let mut enabled = world(true)?;
    assert_eq!(
        objects(&mut enabled)?,
        (f.purchase.seller, f.purchase.charter)
    );
    assert!(
        matches!(enabled.try_modules_loaded(narrow(f.disk.clone(),floor)?),Err(ModError::RetryFullFetch{module,..}) if module=="economy")
    );
    enabled.try_modules_loaded(f.disk)?;
    assert_eq!(
        store(&enabled)?
            .boot_replay()
            .ok_or("boot report")?
            .from_checkpoint,
        None
    );
    assert!(!store(&enabled)?.is_frozen());
    assert_eq!(
        store(&enabled)?.purchase_receipt_audit().receipts()?.len(),
        0
    );
    Ok(())
}

fn doc<'a>(disk: &'a BootMap, key: &str) -> TestResult<&'a Value> {
    disk.get("economy")
        .ok_or("economy")?
        .kv
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or_else(|| "document".into())
}
fn doc_mut<'a>(disk: &'a mut BootMap, key: &str) -> TestResult<&'a mut Value> {
    disk.get_mut("economy")
        .ok_or("economy")?
        .kv
        .iter_mut()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or_else(|| "document".into())
}
fn selected(disk: &BootMap) -> TestResult<crate::checkpoint::ValidCheckpoint> {
    let data = disk.get("economy").ok_or("economy")?;
    let history = mush_core::inspect_ledger(&data.ledger, None);
    assert!(history.is_clean());
    let mut max = None;
    for (key, raw) in &data.ledger {
        if mush_core::parse_chunk_key(key).is_some_and(|(log, _)| log == JOURNAL_LOG) {
            let chunk: LedgerChunk = serde_json::from_str(raw)?;
            for entry in chunk.entries {
                max = Some(max.map_or(entry.seq, |old: u64| old.max(entry.seq)));
            }
        }
    }
    let (checkpoint, notes) = crate::checkpoint::select(&data.kv, max, &history);
    assert!(notes.is_empty(), "generic checkpoint integrity: {notes:?}");
    checkpoint.ok_or_else(|| "generic checkpoint must remain valid".into())
}
#[derive(Clone, Copy, Debug)]
enum Damage {
    OmittedProof,
    ChangedProof,
    MissingIdem,
    ExtraIdem,
    ChangedIdem,
}
fn damage(disk: &mut BootMap, head_key: &str, damage: Damage) -> TestResult {
    let mut head: crate::checkpoint::CheckpointHead =
        serde_json::from_value(doc(disk, head_key)?.clone())?;
    let mut proof = serde_json::to_value(&head.body.fold.purchase_receipts)?;
    assert_eq!(proof.get("complete"), Some(&json!(true)));
    match damage {
        Damage::OmittedProof => proof
            .get_mut("histories")
            .and_then(Value::as_object_mut)
            .ok_or("histories")?
            .clear(),
        Damage::ChangedProof => {
            let history = proof
                .get_mut("histories")
                .and_then(Value::as_object_mut)
                .ok_or("histories")?
                .values_mut()
                .next()
                .ok_or("history")?;
            *history
                .pointer_mut("/terminal/commit")
                .ok_or("terminal commit")? =
                serde_json::to_value(CommitToken::from_bytes([77; 16]))?;
        }
        _ => {
            let mut changed = false;
            for reference in &mut head.parts {
                let mut part: crate::checkpoint::IdemPart =
                    serde_json::from_value(doc(disk, &reference.key)?.clone())?;
                for row in &mut part.rows {
                    if changed {
                        break;
                    }
                    match damage {
                        Damage::MissingIdem if row.ns.as_str() == NAMESPACE => {
                            row.ns = "test.not_purchase".into();
                            changed = true;
                        }
                        Damage::ExtraIdem if row.ns.as_str() != NAMESPACE => {
                            row.ns = NAMESPACE.into();
                            row.key = format!("{}/fund", CommitToken::from_bytes([77; 16])).into();
                            changed = true;
                        }
                        Damage::ChangedIdem if row.ns.as_str() == NAMESPACE => {
                            row.posted_at += 1;
                            changed = true;
                        }
                        _ => {}
                    }
                }
                let raw = serde_json::to_value(part)?;
                reference.sha256 = crate::checkpoint::canonical_hash(&raw).ok_or("part hash")?;
                *doc_mut(disk, &reference.key)? = raw;
            }
            assert!(changed, "actual immutable row was altered");
        }
    }
    head.body.fold.purchase_receipts = serde_json::from_value(proof)?;
    let mut parts = head.parts.clone();
    parts.push(head.bloom.clone());
    head.state_hash = crate::checkpoint::state_hash(&serde_json::to_value(&head.body)?, &parts)
        .ok_or("state hash")?;
    *doc_mut(disk, head_key)? = serde_json::to_value(head)?;
    let checked = selected(disk)?;
    assert!(
        !checked
            .head
            .body
            .fold
            .purchase_receipts
            .checkpoint_complete(&checked.rows),
        "{damage:?}"
    );
    Ok(())
}

#[test]
fn complete_flag_cannot_hide_missing_extra_or_crossed_checkpoint_evidence() -> TestResult {
    for outcome in [PurchaseOutcome::Granted, PurchaseOutcome::Refunded] {
        let mut f = Fixture::new(true)?;
        let funding = f.fund()?;
        f.terminal(&funding, outcome)?;
        let (_, floor, head) = f.checkpoint()?;
        let original = selected(&f.disk)?;
        assert!(
            original
                .head
                .body
                .fold
                .purchase_receipts
                .checkpoint_complete(&original.rows)
        );
        for corruption in [
            Damage::OmittedProof,
            Damage::ChangedProof,
            Damage::MissingIdem,
            Damage::ExtraIdem,
            Damage::ChangedIdem,
        ] {
            let mut damaged = f.disk.clone();
            damage(&mut damaged, &head, corruption)?;
            let mut recovery = world(true)?;
            assert_eq!(
                objects(&mut recovery)?,
                (f.purchase.seller, f.purchase.charter)
            );
            let error = recovery.try_modules_loaded(narrow(damaged.clone(), floor)?);
            assert!(
                matches!(error,Err(ModError::RetryFullFetch{module,..}) if module=="economy"),
                "{corruption:?}"
            );
            // The same false checkpoint with the complete real journal must
            // recover the actual proof, never trust the altered complete flag.
            recovery.try_modules_loaded(damaged)?;
            assert_eq!(
                store(&recovery)?
                    .boot_replay()
                    .ok_or("boot report")?
                    .from_checkpoint,
                None
            );
            assert!(!store(&recovery)?.is_frozen());
            check_history(&recovery, &f.purchase, &funding, Some(outcome))?;
            same_purchase_projection(&f.tw, &recovery, &f.purchase, &funding)?;
        }
    }
    Ok(())
}

#[test]
fn closed_purchase_history_cannot_boot_with_purchase_composition_disabled() -> TestResult {
    let mut f = Fixture::new(true)?;
    let funding = f.fund()?;
    for terminal in [false, true] {
        if terminal {
            f.terminal(&funding, PurchaseOutcome::Refunded)?;
        }
        let mut disabled = world(false)?;
        assert_eq!(
            objects(&mut disabled)?,
            (f.purchase.seller, f.purchase.charter)
        );
        assert!(disabled.try_modules_loaded(f.disk.clone()).is_err());
    }
    Ok(())
}

mod entitlement;
