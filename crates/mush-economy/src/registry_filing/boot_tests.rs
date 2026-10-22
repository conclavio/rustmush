#![cfg(test)]

//! Actual module boot/checkpoint recovery of filing history. These tests never
//! open the receipt audit directly: only the real complete boot may do so.

use super::*;
use std::collections::HashMap;

use mush_core::action::{ActionError, ActionErrorCode, ActionInvocation, ActionProvider};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
};
use mush_core::{
    Audience, GOD, GameModule, LedgerChunk, ModError, ModuleBootData, ModuleOp, ModuleWrite,
    ObjKind, Registrar,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops_into};

type BootMap = HashMap<SmolStr, ModuleBootData>;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn action_descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").unwrap(),
        DefinitionId::new("initial_filing").unwrap(),
        1,
        serde_json::json!({"fixture":"economy-history"}),
    )
    .unwrap()
}
fn action() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("roll").unwrap(), action_descriptor())
        .unwrap()
}
struct FilingDeclaration;
struct DeclarationOnly;
impl ActionProvider for DeclarationOnly {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "This history fixture does not expose a filing action.",
        )
        .unwrap())
    }
}
impl GameModule for FilingDeclaration {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = action();
        reg.definition_validator(
            DefinitionKind::new("action").unwrap(),
            move |context: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                let entry = context.definition();
                if entry.reference() == &expected
                    && entry.payload() == &serde_json::json!({"fixture":"economy-history"})
                    && entry.references().is_empty()
                {
                    Ok(())
                } else {
                    Err("history fixture action differs from its closed declaration".into())
                }
            },
        )?;
        let action = reg.definition(action_descriptor())?;
        reg.action_provider(action, DeclarationOnly)
    }
}
fn policy() -> InitialFilingPolicy {
    InitialFilingPolicy::new(
        SystemPrincipalId::new("roll.registry").unwrap(),
        AccountId(2),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .unwrap()
}
fn world(composed: bool) -> TestWorld {
    let mut economy = crate::EconomyModule::new()
        .with_config(crate::EconomyConfig {
            tx_tail_max: 3,
            checkpoint_every_entries: 4,
            ..Default::default()
        })
        .with_title_issuers(vec![SystemPrincipalId::new("roll.registry").unwrap()]);
    if composed {
        economy = economy.with_initial_filing(policy(), action());
    }
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(FilingDeclaration),
        Box::new(economy),
    ])
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().unwrap()
}
fn run(tw: &mut TestWorld, command: &str) {
    let events = tw.run(GOD, command);
    let is_error = |(_, event): &(Audience, OutputEvent)| matches!(event, OutputEvent::Sys(text) if text.spans().iter().any(|s| s.tag.as_deref() == Some("error")));
    assert!(!events.iter().any(is_error), "{command}: {events:?}");
}
fn fund(tw: &mut TestWorld) {
    run(tw, "@instrument/register cr=2");
    run(tw, "@account/system faucet=cr");
    run(tw, "@account/open #1=cr");
    run(tw, "@account/system roll.registry=cr");
    run(tw, "@econ/mint 0=1 10000");
    assert_eq!(
        store(tw)
            .snapshot()
            .account(AccountId(1))
            .unwrap()
            .posted_balance,
        Amount::from_minor(10_000)
    );
}
fn fold(tw: &mut TestWorld, boot: &mut BootMap) {
    fold_module_ops_into(boot, tw.take_module_ops());
}
fn checkpoint(tw: &mut TestWorld, boot: &mut BootMap) -> u64 {
    for _ in 0..120 {
        run(tw, "@econ/mint 0=1 1");
    }
    tw.set_now(100);
    tw.fire_due(100_000);
    let saved = store(tw).checkpoint().unwrap();
    let floor = store(tw).chunk_of_seq(saved.covered_seq - 2).unwrap();
    assert!(floor > 0, "fixture must actually omit historical chunks");
    fold(tw, boot);
    floor
}
fn narrow(mut boot: BootMap, floor: u64) -> BootMap {
    let data = boot.get_mut("economy").unwrap();
    data.ledger.retain(|(key, _)| {
        mush_core::parse_chunk_key(key)
            .is_none_or(|(log, chunk)| log != JOURNAL_LOG || chunk >= floor)
    });
    data.omitted.insert(JOURNAL_LOG.into(), floor);
    boot
}
fn reserve_object(tw: &mut TestWorld) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, "Reserved mind charter", GOD, None)
        .unwrap()
}
fn file(tw: &mut TestWorld, boot: &mut BootMap) -> (InitialFilingAuditSubject, u64, ObjId) {
    let shell = reserve_object(tw);
    let adapter = InitialFilingParticipant::new(policy());
    let (output, patch, writes) = adapter
        .prepare_images(
            store(tw),
            tw.world(),
            GOD,
            CommitToken::from_bytes([7; 16]),
            RegistryInitialFiling {
                shell,
                payer: AccountId(1),
                owner: PrincipalId::Object(GOD),
                scope: None,
            },
        )
        .unwrap();
    // Exercise the real owner image installation, then retain exactly its
    // durable writes. Core cross-owner atomicity has its own Engine tests.
    Arc::new(adapter)
        .bind(
            tw.world_mut().module_state_mut::<EconomyStore>().unwrap(),
            patch.into(),
        )
        .unwrap()
        .apply();
    let ops = writes
        .into_iter()
        .map(|write| match write {
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
                    value: serde_json::to_value(write.chunk()).unwrap(),
                },
            },
            _ => panic!("E4 filing only writes module JSON and journal chunks"),
        })
        .collect();
    fold_module_ops_into(boot, ops);
    (
        receipt::subject_of(store(tw).transaction(output.transaction()).unwrap()).unwrap(),
        output.journal_seq(),
        shell,
    )
}

#[test]
fn legacy_checkpoint_requires_full_fetch_before_empty_filing_history_is_trusted() -> TestResult {
    let mut old = world(false);
    fund(&mut old);
    let mut saved = BootMap::new();
    let floor = checkpoint(&mut old, &mut saved);
    let head = &store(&old).checkpoint().unwrap().head_key;
    let (_, document) = saved["economy"].kv.iter().find(|(k, _)| k == head).unwrap();
    check!(document["body"]["fold"].get("filing_receipts").is_none());

    // Prove this is a valid old checkpoint, not merely a bad hash fixture.
    let mut legacy = world(false);
    legacy.try_modules_loaded(narrow(saved.clone(), floor))?;
    check!(
        store(&legacy)
            .boot_replay()
            .unwrap()
            .from_checkpoint
            .is_some()
    );
    check!(!store(&legacy).is_frozen());

    let mut composed = world(true);
    let error = composed.try_modules_loaded(narrow(saved.clone(), floor));
    check!(matches!(error, Err(ModError::RetryFullFetch { module, .. }) if module == "economy"));
    composed.try_modules_loaded(saved)?;
    check_eq!(
        store(&composed).boot_replay().unwrap().from_checkpoint,
        None
    );
    check!(!store(&composed).is_frozen());
    check!(store(&composed).filing_receipt_audit().is_empty()?);
    Ok(())
}

#[test]
fn checkpoint_and_tail_recover_a_receipt_older_than_every_loaded_journal_chunk() -> TestResult {
    let mut live = world(true);
    fund(&mut live);
    let mut saved = BootMap::new();
    fold(&mut live, &mut saved);
    let (subject, receipt_seq, title) = file(&mut live, &mut saved);
    assert_generic_title_commands_refuse(&mut live, title);
    let floor = checkpoint(&mut live, &mut saved);
    let covered = store(&live).checkpoint().unwrap().covered_seq;
    run(&mut live, "@econ/mint 0=1 1");
    fold(&mut live, &mut saved);
    let narrowed = narrow(saved, floor);
    for (_, raw) in &narrowed["economy"].ledger {
        let chunk: LedgerChunk = serde_json::from_str(raw)?;
        check!(chunk.entries.iter().all(|entry| entry.seq > receipt_seq));
    }
    let mut restarted = world(true);
    check_eq!(reserve_object(&mut restarted), title);
    restarted.try_modules_loaded(narrowed)?;
    let replay = store(&restarted).boot_replay().unwrap();
    check_eq!(replay.from_checkpoint, Some(covered));
    check_eq!(replay.replayed, 1);
    check!(!store(&restarted).is_frozen());
    store(&restarted).filing_receipt_audit().verify(&subject)?;
    check_eq!(
        store(&restarted).filing_receipt_audit().receipts()?.len(),
        1
    );
    assert_generic_title_commands_refuse(&mut restarted, title);
    Ok(())
}

fn assert_generic_title_commands_refuse(tw: &mut TestWorld, title: ObjId) {
    assert!(store(tw).snapshot.managed_titles.contains(title));
    let before = store(tw).snapshot.asset(title).unwrap().clone();
    let next = store(tw).next_transaction;
    for command in [
        format!("@asset/transfer #{}=#1", title.0),
        format!("@asset/catalog #{}=none", title.0),
        format!("@asset/writeoff #{}=fixture", title.0),
    ] {
        let events = tw.run(GOD, &command);
        assert!(
            events
                .iter()
                .any(|(_, event)| matches!(event, OutputEvent::Sys(text)
            if text.spans().iter().any(|span| span.tag.as_deref() == Some("error"))))
        );
        assert_eq!(store(tw).snapshot.asset(title), Some(&before));
        assert_eq!(store(tw).next_transaction, next);
    }
}

#[test]
fn malformed_closed_namespace_journal_cannot_be_silently_treated_as_no_filings() -> TestResult {
    let mut live = world(true);
    fund(&mut live);
    let mut saved = BootMap::new();
    fold(&mut live, &mut saved);
    let (_, receipt_seq, _) = file(&mut live, &mut saved);
    let mut changed = false;
    for (_, raw) in &mut saved.get_mut("economy").unwrap().ledger {
        let mut chunk: LedgerChunk = serde_json::from_str(raw)?;
        for entry in &mut chunk.entries {
            if entry.seq == receipt_seq {
                let mut transaction: EconomicTransaction =
                    serde_json::from_value(entry.value.clone())?;
                transaction.cause = EconomicCause::Ruleset {
                    namespace: receipt::FILING_NAMESPACE.into(),
                    correlation: "malformed".into(),
                };
                entry.value = serde_json::to_value(transaction)?;
                changed = true;
            }
        }
        *raw = serde_json::to_string(&chunk)?;
    }
    check!(changed, "actual filing transaction must be present");
    let mut restarted = world(true);
    reserve_object(&mut restarted);
    check!(restarted.try_modules_loaded(saved).is_err());
    Ok(())
}
