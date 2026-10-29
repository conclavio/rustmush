//! Exact wire/cap/replay accounting. Wide and counted rows are structural proof
//! boundary fixtures, never a claim of 32,768 live funded owner documents.
use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn idem_rows(a: &PurchaseReceiptAudit) -> Vec<crate::checkpoint::IdemRow> {
    let mut rows = Vec::new();
    for h in a.histories.values() {
        rows.push(crate::checkpoint::IdemRow {
            ns: h.purchase.namespace().into(),
            key: format!("{}/fund", h.operation.to_hex()).into(),
            tx: h.funding.transaction,
            seq: h.funding.journal_seq,
            posted_at: h.created_at,
            plan_hash: h.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
        });
        if let Some(t) = &h.terminal {
            rows.push(crate::checkpoint::IdemRow {
                ns: h.purchase.namespace().into(),
                key: format!(
                    "{}/{}",
                    h.operation.to_hex(),
                    if t.granted { "grant" } else { "refund" }
                )
                .into(),
                tx: t.transaction,
                seq: t.journal_seq,
                posted_at: t.posted_at,
                plan_hash: t.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
            });
        }
    }
    rows
}

#[test]
fn shared_history_binds_new_namespace_without_changing_legacy_subject_bytes() -> TestResult {
    let (old, _, _) = super::super::tests::receipt_capacity_fixture()?;
    let original = old.histories.values().next().ok_or("old")?;
    let SubjectWire::Original(p) = &original.purchase else {
        return Err("original arm".into());
    };
    check_eq!(
        serde_json::to_vec(&original.purchase)?,
        serde_json::to_vec(p)?
    );
    let retained_original = original.original()?.ok_or("actual original recipe")?;
    check!(original.entitlement()?.is_none());
    check!(original.matches_purchase(&retained_original, original.funding()));
    let (mut new, tx, terminals) = super::super::tests::entitlement::history_fixture()?;
    let retained_new = new.histories.values().next().ok_or("new history")?;
    check!(retained_new.original()?.is_none());
    let new_recipe = retained_new
        .entitlement()?
        .ok_or("actual entitlement recipe")?;
    check!(retained_new.matches_entitlement(&new_recipe, retained_new.funding()));
    let before = new.clone();
    let rows = idem_rows(&new);
    check!(new.checkpoint_complete(&rows));
    let mut wrong = rows.clone();
    wrong.first_mut().ok_or("row")?.ns = NAMESPACE.into();
    check!(!new.checkpoint_complete(&wrong));
    let mut duplicate = rows.clone();
    duplicate.push(rows.first().ok_or("row")?.clone());
    check!(!new.checkpoint_complete(&duplicate));
    let mut crossed = tx.clone();
    crossed.idempotency.namespace = NAMESPACE.into();
    let mut replay = PurchaseReceiptAudit::default();
    replay.observe(&crossed);
    check!(replay.finish_replay().is_err());
    for terminal in terminals {
        let mut replay = PurchaseReceiptAudit::default();
        replay.observe(&tx);
        replay.observe(&terminal);
        replay.finish_replay()?;
        let mut checkpoint: PurchaseReceiptAudit =
            serde_json::from_slice(&serde_json::to_vec(&replay)?)?;
        check!(checkpoint.require_complete().is_err());
        check!(checkpoint.checkpoint_complete(&idem_rows(&replay)));
        checkpoint.finish_replay()?;
        check_eq!(checkpoint, replay);
        let mut wrong = terminal.clone();
        wrong.idempotency.namespace = NAMESPACE.into();
        if let EconomicCause::Ruleset { namespace, .. } = &mut wrong.cause {
            *namespace = NAMESPACE.into();
        }
        check!(before.prepare(&wrong).is_err());
        replay.observe(&terminal);
        check!(replay.finish_replay().is_err());
    }
    // Actual old and new proofs cannot alias the same global operation even in
    // distinct namespaces; all current producers use the actual commit token.
    check_eq!(
        original.operation,
        new.histories.values().next().ok_or("new")?.operation
    );
    new.observe(&tx);
    check!(new.finish_replay().is_err());
    Ok(())
}

fn widen(
    original: &PurchaseHistory,
    instrument_bytes: usize,
) -> Result<PurchaseHistory, Box<dyn std::error::Error>> {
    let mut row = original.clone();
    let mut p = row
        .purchase
        .data()?
        .entitlement()
        .ok_or("new subject")?
        .clone();
    p.instrument = InstrumentId::new("x".repeat(instrument_bytes)).ok_or("instrument")?;
    p.buyer = ObjId(u32::MAX).into();
    p.seller = ObjId(u32::MAX - 1);
    p.charter = ObjId(u32::MAX - 2);
    p.title_revision = u64::MAX;
    p.payer = AccountId(u64::MAX);
    p.price = crate::Amount::from_minor(i64::MAX);
    p.remainder.account = AccountId(u64::MAX - 1);
    p.remainder.owner = PrincipalId::Object(p.seller);
    p.registry.recipient.as_mut().ok_or("registry")?.account = AccountId(u64::MAX - 2);
    p.venue
        .as_mut()
        .ok_or("venue")?
        .recipient
        .as_mut()
        .ok_or("venue recipient")?
        .account = AccountId(u64::MAX - 3);
    for (n, parent) in p.parents.iter_mut().enumerate() {
        let n = u32::try_from(n)?;
        parent.charter = ObjId(u32::MAX - 3 - n);
        let title = parent.title.as_mut().ok_or("parent")?;
        title.title_revision = u64::MAX;
        title.recipient.account = AccountId(u64::MAX - 4 - u64::from(n));
        title.recipient.owner = PrincipalId::Object(ObjId(u32::MAX - 6 - n));
    }
    p.quoted_at_utc = i64::MAX - 600;
    p.expires_at_utc = i64::MAX;
    row.operation = CommitToken::from_bytes([0xff; 16]);
    row.created_at = i64::MAX - 599;
    row.funding.contract = ContractId(u64::MAX);
    row.funding.hold = HoldId(u64::MAX);
    row.funding.transaction = TransactionId(u64::MAX - 1);
    row.funding.journal_seq = u64::MAX - 1;
    row.funding.at_ic = ClockStamp {
        clock: ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MAX - 599,
    };
    row.hold.id = row.funding.hold;
    row.hold.account = p.payer;
    row.hold.amount = p.price;
    row.hold.placed_by = p.buyer.principal();
    row.hold.holder = IdempotencyKey::new(
        crate::managed_entitlement::NAMESPACE,
        format!("{}/fund", row.operation.to_hex()),
    );
    row.hold.purpose = HoldPurpose::Escrow {
        contract: row.funding.contract,
    };
    row.hold.last_journal_seq = row.funding.journal_seq;
    let subject = PurchaseSubject::try_from(p)?;
    let marker = subject.marker(row.operation, row.effect.clone())?;
    row.funding.consent_hash = super::super::contract(
        &subject,
        marker,
        row.funding.contract,
        row.funding.hold,
        row.created_at,
    )?
    .consent_hash;
    row.purchase = SubjectWire::capture(&subject);
    // Compute raw hash even beyond encoding admission limits to test refusal at
    // the exact correlation boundary independently from retained-history size.
    if let Ok(plan) = row.funding_plan() {
        row.plan_hash = crate::hash::plan_hash(&plan);
    }
    Ok(row)
}

#[test]
fn maximum_width_real_encodings_charge_correlation_and_terminal_history_headroom() -> TestResult {
    let (audit, _, _) = super::super::tests::entitlement::history_fixture()?;
    let original = audit.histories.values().next().ok_or("history")?;
    let mut accepted = None;
    let mut correlation_crossing = false;
    let mut history_crossing = false;
    for n in 1..=MAX_HISTORY_BYTES {
        let row = widen(original, n)?;
        let proof = Proof::Fund {
            format: 1,
            operation: row.operation,
            effect: row.effect.clone(),
            purchase: Box::new(row.purchase.clone()),
            at_ic: row.funding.at_ic.clone(),
        };
        let correlation_len = serde_json::to_vec(&proof)?.len();
        let history_len = serde_json::to_vec(&row)?.len();
        check_eq!(
            encode(&proof).is_ok(),
            correlation_len <= MAX_CORRELATION_BYTES
        );
        check_eq!(
            row.bounded(),
            history_len <= MAX_HISTORY_BYTES - TERMINAL_RESERVE_BYTES
        );
        if correlation_len > MAX_CORRELATION_BYTES {
            correlation_crossing = true;
        }
        if history_len > MAX_HISTORY_BYTES - TERMINAL_RESERVE_BYTES {
            history_crossing = true;
        }
        if row.validate().is_ok() {
            accepted = Some((n, row));
        }
        if correlation_crossing && history_crossing {
            break;
        }
    }
    check!(correlation_crossing && history_crossing);
    let (n, row) = accepted.ok_or("a fully wide six-payee proof must fit")?;
    check!(widen(original, n + 1)?.validate().is_err());
    let mut terminal_hold = row.hold.clone();
    terminal_hold.status = HoldStatus::Consumed {
        transaction: TransactionId(u64::MAX),
        amount: row.hold.amount,
    };
    terminal_hold.revision = 2;
    terminal_hold.last_journal_seq = u64::MAX;
    let mut terminal = PurchaseTerminalHistory {
        commit: CommitToken::from_bytes([0xfe; 16]),
        granted: true,
        transaction: TransactionId(u64::MAX),
        journal_seq: u64::MAX,
        issued_at_ic: Some(ClockStamp {
            clock: row.funding.at_ic.clock.clone(),
            at: i64::MAX,
        }),
        hold: terminal_hold,
        posted_at: i64::MAX,
        plan_hash: [0xff; 32],
    };
    let mut full = row.clone();
    terminal.plan_hash = crate::hash::plan_hash(&full.terminal_plan(&terminal)?);
    full.terminal = Some(terminal.clone());
    check!(serde_json::to_vec(&terminal)?.len() <= TERMINAL_RESERVE_BYTES);
    check!(serde_json::to_vec(&row)?.len() + TERMINAL_RESERVE_BYTES <= MAX_HISTORY_BYTES);
    check!(serde_json::to_vec(&full)?.len() <= MAX_HISTORY_BYTES);
    full.validate()?;
    Ok(())
}

#[test]
fn combined_old_and_new_history_share_one_admission_cap_and_reserved_terminal_slot() -> TestResult {
    let (mut old, _, _) = super::super::tests::receipt_capacity_fixture()?;
    let original = old.histories.values().next().ok_or("old")?.clone();
    let (_, new_funding, new_terminals) = super::super::tests::entitlement::history_fixture()?;
    // Avoid the actual fixture's shared operation token while preserving all
    // fields in the new actual transaction unchanged.
    old.histories.clear();
    for ordinal in 100..100 + u64::try_from(MAX_HISTORIES - 1)? {
        let row = super::capacity_tests::counted_row(&original, ordinal)?;
        old.histories.insert(row.operation, row);
    }
    let prepared = old.prepare(&new_funding)?;
    prepared.apply(&mut old);
    check_eq!(old.histories.len(), MAX_HISTORIES);
    for tx in new_terminals {
        let mut a = old.clone();
        let next = a.prepare(&tx)?;
        next.apply(&mut a);
        check_eq!(a.histories.len(), MAX_HISTORIES);
    }
    // Replace one old row with another valid old row to keep the count full,
    // then attempt the same absent new funding: refusal is count, not replay.
    let operation = CommitToken::from_bytes([3; 16]);
    old.histories.remove(&operation).ok_or("new")?;
    let row = super::capacity_tests::counted_row(&original, u64::MAX - 1)?;
    old.histories.insert(row.operation, row);
    let before = old.clone();
    check!(old.prepare(&new_funding).is_err());
    check_eq!(old, before);
    Ok(())
}
