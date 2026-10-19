#![cfg(test)]

use super::*;
use crate::record::{
    AccountStatus, CreditPolicy, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_core::{GOD, ObjKind};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixture(
    fee: Amount,
) -> Result<(World, EconomyStore, InitialFilingParticipant, ObjId, ObjId), Box<dyn std::error::Error>>
{
    let mut world = World::new_minimal();
    let actor = world.create(ObjKind::Player, "Filer", GOD, None)?;
    let shell = world.create(ObjKind::Thing, "Reserved mind charter", GOD, None)?;
    let issuer = SystemPrincipalId::new("roll.registry").ok_or("fixture issuer")?;
    let instrument = crate::InstrumentId::new("cr").ok_or("fixture instrument")?;
    let mut state = EconomyStore::default();
    state.title_issuers = Arc::new(std::collections::BTreeSet::from([issuer.clone()]));
    state.snapshot.put_instrument(InstrumentDef {
        id: instrument.clone(),
        ruleset: "roll".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 2,
        transferable: true,
        negative_policy: NegativePolicy::Forbid,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    });
    for (id, owner, balance) in [
        (AccountId(0), PrincipalId::Object(actor), 10_000),
        (AccountId(1), PrincipalId::System(issuer.clone()), 0),
    ] {
        state.snapshot.put_account(Account {
            id,
            owner,
            instrument: instrument.clone(),
            scope: None,
            name: "fixture".into(),
            status: AccountStatus::Active,
            posted_balance: Amount::from_minor(balance),
            held_amount: Amount::ZERO,
            credit: CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
            last_hold_seq: 0,
        });
        state.fold.running.insert(id, i128::from(balance));
    }
    state.next_account = 2;
    state.fold.filing_receipts.finish_replay()?;
    let policy = InitialFilingPolicy::new(
        issuer,
        AccountId(1),
        fee,
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?;
    Ok((
        world,
        state,
        InitialFilingParticipant::new(policy),
        actor,
        shell,
    ))
}
fn request(actor: ObjId, shell: ObjId) -> RegistryInitialFiling {
    RegistryInitialFiling {
        shell,
        payer: AccountId(0),
        owner: PrincipalId::Object(actor),
        scope: None,
    }
}
fn commit() -> CommitToken {
    CommitToken::from_bytes([7; 16])
}

#[test]
fn exact_images_are_inert_until_bound_apply_and_match_actual_state() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let before = state.snapshot.clone();
    let before_fold = state.fold.clone();
    let before_filter = state.key_filter.clone();
    let (output, patch, writes) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    assert_eq!(state.snapshot, before);
    assert_eq!(state.journal.next_seq(JOURNAL_LOG), 0);
    assert_eq!(
        writes.len(),
        6,
        "journal, two accounts, title, key and allocator travel together"
    );
    assert!(writes.iter().all(|write| adapter.accepts_write(write)));
    drop(Arc::new(adapter.clone()).bind(&mut state, patch.into())?);
    assert_eq!(state.snapshot, before);
    assert_eq!(state.fold, before_fold);
    assert_eq!(state.key_filter, before_filter);
    let (_, patch, writes) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    Arc::new(adapter.clone())
        .bind(&mut state, patch.into())?
        .apply();
    assert_eq!(
        state
            .snapshot
            .account(AccountId(0))
            .map(|a| a.posted_balance),
        Some(Amount::from_minor(9_900))
    );
    assert_eq!(
        state
            .snapshot
            .account(AccountId(1))
            .map(|a| a.posted_balance),
        Some(Amount::from_minor(100))
    );
    assert_eq!(state.snapshot.asset(shell), Some(output.title()));
    assert_eq!(state.journal.next_seq(JOURNAL_LOG), 1);
    assert_eq!(state.next_transaction, 1);
    for write in writes {
        if let ReplayWrite::ModuleJsonSet { key, value, .. } = write {
            if key.as_str() == asset_key(shell) {
                assert_eq!(value, serde_json::to_value(output.title())?);
            }
            if key.as_str() == crate::api::META_KEY {
                assert_eq!(value, state.meta_doc());
            }
        }
    }
    let subject = receipt::subject_of(
        state
            .transaction(output.transaction())
            .ok_or("actual transaction")?,
    )?;
    state.filing_receipt_audit().verify(&subject)?;
    assert!(
        adapter
            .prepare_images(&state, &world, actor, commit(), request(actor, shell))
            .is_err(),
        "repeat cannot allocate or charge twice"
    );
    Ok(())
}

#[test]
fn account_doc_authority_change_without_journal_movement_refuses_before_any_apply() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (_, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    let payer = state.snapshot.account_mut(AccountId(0)).ok_or("payer")?;
    payer.status = AccountStatus::Frozen; // Deliberately preserve its economic revision.
    let before = state.snapshot.clone();
    assert!(Arc::new(adapter).bind(&mut state, patch.into()).is_err());
    assert_eq!(state.snapshot, before);
    assert!(state.snapshot.asset(shell).is_none());
    assert_eq!(state.journal.next_seq(JOURNAL_LOG), 0);
    assert_eq!(state.next_transaction, 0);
    Ok(())
}

#[test]
fn foreign_payer_authority_and_nonlive_shell_are_not_inherited_from_the_issuer() -> TestResult {
    let (mut world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let other = world.create(ObjKind::Player, "Other", GOD, None)?;
    state
        .snapshot
        .account_mut(AccountId(0))
        .ok_or("payer")?
        .owner = PrincipalId::Object(other);
    assert!(
        adapter
            .prepare_images(&state, &world, actor, commit(), request(actor, shell))
            .is_err()
    );
    assert!(state.snapshot.asset(shell).is_none());
    assert_eq!(state.journal.next_seq(JOURNAL_LOG), 0);
    state
        .snapshot
        .account_mut(AccountId(0))
        .ok_or("payer")?
        .owner = PrincipalId::Object(actor);
    world
        .get_mut(shell)
        .ok_or("shell")?
        .flags
        .insert(mush_core::Flags::GOING);
    assert!(
        adapter
            .prepare_images(&state, &world, actor, commit(), request(actor, shell))
            .is_err()
    );
    assert!(state.snapshot.asset(shell).is_none());
    assert_eq!(state.journal.next_seq(JOURNAL_LOG), 0);
    Ok(())
}

#[test]
fn a_lawful_zero_fee_still_commits_title_and_auditable_payer_product_identity() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::ZERO)?;
    let (output, patch, writes) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    assert_eq!(
        writes.len(),
        4,
        "zero omits account writes, never the journal/title/key/meta"
    );
    Arc::new(adapter).bind(&mut state, patch.into())?.apply();
    let tx = state
        .transaction(output.transaction())
        .ok_or("actual transaction")?;
    assert!(tx.lines.is_empty());
    assert_eq!(tx.asset_effects.len(), 1);
    let subject = receipt::subject_of(tx)?;
    state.filing_receipt_audit().verify(&subject)?;
    assert_eq!(
        state
            .snapshot
            .account(AccountId(0))
            .map(|a| a.posted_balance),
        Some(Amount::from_minor(10_000))
    );
    Ok(())
}

#[test]
fn deserialized_receipt_state_is_not_verified_until_actual_replay_completion() -> TestResult {
    let (world, mut state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (output, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    Arc::new(adapter).bind(&mut state, patch.into())?.apply();
    let tx = state
        .transaction(output.transaction())
        .ok_or("actual transaction")?;
    let subject = receipt::subject_of(tx)?;
    let mut replay = ReceiptAudit::default();
    assert_eq!(
        replay.require_complete(),
        Err(ReceiptAuditError::Incomplete)
    );
    replay.observe(tx);
    replay.finish_replay()?;
    replay.verify(&subject)?;
    let encoded = serde_json::to_vec(&replay)?;
    let mut checkpoint: ReceiptAudit = serde_json::from_slice(&encoded)?;
    assert!(checkpoint.checkpoint_complete());
    assert_eq!(
        checkpoint.verify(&subject),
        Err(ReceiptAuditError::Incomplete)
    );
    checkpoint.finish_replay()?;
    checkpoint.verify(&subject)?;
    let mixed = InitialFilingAuditSubject::new(
        output.commit_token(),
        output.transaction(),
        output.journal_seq(),
        shell,
        PrincipalId::Object(actor),
        1,
        output.payer(),
        output.sink(),
        Amount::from_minor(101),
        output.issuer().clone(),
        output
            .title()
            .catalog
            .clone()
            .ok_or("fixture title catalog absent")?,
    )?;
    assert_eq!(checkpoint.verify(&mixed), Err(ReceiptAuditError::Refused));
    Ok(())
}

#[test]
fn malformed_or_duplicate_closed_journal_entry_cannot_complete_an_audit() -> TestResult {
    let (world, state, adapter, actor, shell) = fixture(Amount::from_minor(100))?;
    let (_, patch, _) =
        adapter.prepare_images(&state, &world, actor, commit(), request(actor, shell))?;
    let mut duplicate = ReceiptAudit::default();
    duplicate.observe(&patch.transaction);
    duplicate.observe(&patch.transaction);
    assert_eq!(duplicate.finish_replay(), Err(ReceiptAuditError::Refused));
    let mut forged = patch.transaction;
    forged.asset_effects.clear();
    let mut audit = ReceiptAudit::default();
    audit.observe(&forged);
    assert_eq!(audit.finish_replay(), Err(ReceiptAuditError::Refused));
    Ok(())
}

#[test]
fn prepared_bloom_bits_match_legacy_insertion_and_dropped_binding_is_inert() -> TestResult {
    let key = IdempotencyKey::new("roll.filing.v1", commit().to_hex());
    let mut actual = crate::bloom::ScalableBloom::new();
    let before = actual.clone();
    let mut expected = actual.clone();
    expected.insert(&key);
    let patch = actual.prepare_insert(&key).ok_or("prepare bloom")?;
    drop(actual.bind_insert(patch).ok_or("bind bloom")?);
    assert_eq!(actual, before);
    let patch = actual.prepare_insert(&key).ok_or("prepare bloom")?;
    actual.bind_insert(patch).ok_or("bind bloom")?.apply();
    assert_eq!(actual, expected);
    Ok(())
}
