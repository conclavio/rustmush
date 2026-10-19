//! Local history-table accounting, using actual E4 funding and terminal rows.
//! Expanded entries are independently well-formed proofs, not evidence that
//! their complete owner documents or 32,768 transactions were durably posted.

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn counted_row(
    original: &PurchaseHistory,
    ordinal: u64,
) -> Result<PurchaseHistory, Box<dyn std::error::Error>> {
    let mut row = original.clone();
    row.operation = CommitToken::from_bytes(u128::from(ordinal).to_be_bytes());
    row.funding.contract = ContractId(ordinal);
    row.funding.hold = HoldId(ordinal);
    row.funding.transaction = TransactionId(ordinal);
    row.funding.journal_seq = ordinal;
    row.hold.id = row.funding.hold;
    row.hold.holder = IdempotencyKey::new(NAMESPACE, format!("{}/fund", row.operation.to_hex()));
    row.hold.purpose = HoldPurpose::Escrow {
        contract: row.funding.contract,
    };
    row.hold.last_journal_seq = row.funding.journal_seq;
    let purchase = row.purchase.data()?;
    let marker = purchase.marker(row.operation, row.effect.clone())?;
    row.funding.consent_hash = super::super::contract(
        &purchase,
        marker,
        row.funding.contract,
        row.funding.hold,
        row.created_at,
    )?
    .consent_hash;
    row.plan_hash = crate::hash::plan_hash(&row.funding_plan()?);
    row.validate()?;
    Ok(row)
}

#[test]
fn exact_history_ceiling_refuses_funding_but_preserves_both_terminal_slots() -> TestResult {
    let (mut audit, next_funding, terminals) = super::super::tests::receipt_capacity_fixture()?;
    let original = audit.receipts()?.next().ok_or("funded history")?.clone();
    // This local accounting fixture starts with one actual admitted proof, then
    // adds valid, distinct data rows without claiming a boot or live-money run.
    for ordinal in 100..u64::try_from(MAX_HISTORIES)? + 98 {
        let row = counted_row(&original, ordinal)?;
        assert!(audit.histories.insert(row.operation, row).is_none());
    }
    assert_eq!(audit.histories.len(), MAX_HISTORIES - 1);
    let accepted = audit.prepare(&next_funding)?;
    assert!(accepted.matches(&audit));
    assert_eq!(audit.histories.len(), MAX_HISTORIES - 1);
    assert!(accepted.before.is_none());
    assert!(
        serde_json::to_vec(&accepted.after)?.len() <= MAX_HISTORY_BYTES - TERMINAL_RESERVE_BYTES
    );
    accepted.apply(&mut audit);
    assert_eq!(audit.histories.len(), MAX_HISTORIES);

    // A distinct admissible funding transaction fails specifically at the
    // count ceiling, not merely because its operation is already present.
    let mut full = audit.clone();
    let new_operation = CommitToken::from_bytes([5; 16]);
    let removed = full.histories.remove(&new_operation).ok_or("new funding")?;
    let replacement = counted_row(&removed, u64::try_from(MAX_HISTORIES)? + 100)?;
    assert!(
        full.histories
            .insert(replacement.operation, replacement)
            .is_none()
    );
    assert_eq!(full.histories.len(), MAX_HISTORIES);
    assert!(!full.histories.contains_key(&new_operation));
    let before = full.clone();
    assert!(full.prepare(&next_funding).is_err());
    assert_eq!(full, before, "refused preparation must remain inert");

    for terminal in terminals {
        let mut target = full.clone();
        let prepared = target.prepare(&terminal)?;
        assert!(prepared.matches(&target));
        assert_eq!(prepared.before.as_ref(), Some(&original));
        let suffix = prepared.after.terminal.as_ref().ok_or("terminal proof")?;
        assert!(serde_json::to_vec(suffix)?.len() <= TERMINAL_RESERVE_BYTES);
        assert!(serde_json::to_vec(&prepared.after)?.len() <= MAX_HISTORY_BYTES);
        assert_eq!(target, full, "terminal preparation must remain inert");
        prepared.apply(&mut target);
        assert_eq!(target.histories.len(), MAX_HISTORIES);
        assert!(target.get(original.operation)?.terminal().is_some());
        assert!(
            target.prepare(&terminal).is_err(),
            "no second terminal post"
        );
    }
    Ok(())
}

#[test]
fn prepared_history_refuses_changed_count_and_terminal_before_image() -> TestResult {
    let (mut audit, next_funding, terminals) = super::super::tests::receipt_capacity_fixture()?;
    let terminal = terminals.first().ok_or("grant")?;
    let prepared = audit.prepare(terminal)?;
    assert!(prepared.matches(&audit));
    let funding = audit.prepare(&next_funding)?;
    funding.apply(&mut audit);
    assert!(
        !prepared.matches(&audit),
        "another admitted history changes the fence"
    );

    let prepared = audit.prepare(terminal)?;
    let competing = audit.prepare(terminals.last().ok_or("refund")?)?;
    competing.apply(&mut audit);
    assert!(
        !prepared.matches(&audit),
        "terminal phase cannot be replaced"
    );
    Ok(())
}
