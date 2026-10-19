//! Structural and accounting tests only; not a certificate or host fixture.
use super::super::records::{PublicationError, Record};
use super::super::{
    Capacity, CharterId, FiledRevisionId, LicenceId, LineId, PaidSeats, TermGeneration, TermWindow,
    WorldIcSeconds,
};
use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn token(number: u64) -> TestResult<CommitToken> {
    Ok(CommitToken::from_hex(&format!("{number:032x}"))?)
}
fn digest(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn definition(owner: &str, kind: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            json!({"format":1}),
        )?,
    )?)
}
fn protocol() -> TestResult<IcProtocol> {
    Ok(IcProtocol {
        manifest: definition("tacsim", "registry_manifest", "combat")?,
        participants: IcParticipants {
            retention: definition("roll", "owner_participant", "commercial_roots")?,
            seal: definition("tacsim", "owner_participant", "ic_sortie")?,
            custody: definition("tactics", "owner_participant", "ic_custody")?,
            exclusion: definition("sim", "owner_participant", "ic_exclusion")?,
        },
    })
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn world(at: i64) -> TestResult<WorldIcSeconds> {
    Ok(WorldIcSeconds::from_stamp(&stamp(at))?)
}
fn row(number: u64, licence: u64, capacity: Capacity) -> TestResult<ClaimRecord> {
    let protocol = protocol()?;
    let row = ClaimRecord {
        format: 1,
        closed: None,
        open: OpenClaim {
            operation: token(number)?,
            action: protocol.action(Transition::Open)?,
            protocol,
            unit: ObjId(u32::try_from(number.checked_add(100).ok_or("unit")?)?),
            sortie: SortieNumber::try_from(number)?,
            dispatcher: ObjId(1),
            hull_title_revision: 1,
            licence: LicenceId::new(licence)?,
            term: TermGeneration::new(1)?,
            charter: CharterId::new(1)?,
            line: LineId::new(1)?,
            filed: FiledRevisionId::new(1)?,
            revision: serde_json::from_value(
                json!({"format":1,"revision":{"key":{"character":1,"mind":1,"number":1},"digest":digest(1)},"source":digest(2)}),
            )?,
            rating: serde_json::from_value(
                json!({"job":token(50000)?,"id":digest(3),"schema":1,"receipt_digest":digest(4)}),
            )?,
            window: TermWindow::new(world(0)?, world(0)?, world(100)?, world(120)?)?,
            capacity,
            at_ic: stamp(5),
            seal_digest: digest(5),
            custody_digest: digest(6),
            exclusion_digest: digest(7),
        },
    };
    row.validate()?;
    Ok(row)
}
fn one_seat() -> TestResult<Capacity> {
    Ok(Capacity::PaidSeats {
        seats: PaidSeats::new(1)?,
    })
}
fn close(row: &ClaimRecord, operation: u64) -> TestResult<CloseCause> {
    let protocol = protocol()?;
    Ok(CloseCause {
        operation: token(operation)?,
        action: protocol.action(Transition::Close)?,
        protocol,
        open: row.open.operation,
        open_digest: row.open.digest()?,
        unit: row.open.unit,
        sortie: row.open.sortie,
        transition: row.open.sortie.get().checked_add(1).ok_or("transition")?,
        event_sequence: 1,
        event_digest: digest(8),
        kind: CloseKind::Destroyed {},
        at_ic: stamp(500),
    })
}

#[test]
fn current_paid_seats_count_prior_term_and_holder_claims_while_owned_has_no_seat_count()
-> TestResult {
    let first = row(1, 11, one_seat()?)?;
    let empty = ClaimInventory::reconstruct(Vec::new())?;
    let held = empty.opening(first.clone())?;
    assert_eq!(held.paid_open(first.open.licence), 1);
    let mut later = row(2, 11, one_seat()?)?;
    later.open.dispatcher = ObjId(2);
    later.open.term = TermGeneration::new(2)?;
    assert!(matches!(
        held.opening(later.clone()),
        Err(PublicationError::Capacity)
    ));
    assert_eq!(held.open_len(), 1);
    assert_eq!(held.claim(first.open.operation), Some(&first));
    let drained = held.closing(close(&first, 10001)?)?.ok_or("close")?;
    later.open.sortie = SortieNumber::try_from(drained.next_transition())?;
    let admitted = drained.opening(later)?;
    assert_eq!(admitted.open_len(), 1);
    assert_eq!(admitted.history_len(), 2);
    let owner = empty
        .opening(row(1, 21, Capacity::OwnedDispatch {})?)?
        .opening(row(2, 21, Capacity::OwnedDispatch {})?)?;
    assert_eq!(owner.open_len(), 2);
    assert_eq!(owner.paid_open(LicenceId::new(21)?), 0);
    Ok(())
}

#[test]
fn exact_close_is_idempotent_and_crossed_identity_cannot_release_capacity() -> TestResult {
    let a = row(1, 1, one_seat()?)?;
    let b = row(2, 2, one_seat()?)?;
    let held = ClaimInventory::reconstruct(vec![a.clone(), b.clone()])?;
    let cause = CloseCause {
        transition: held.next_transition(),
        ..close(&a, 10001)?
    };
    for crossed in [
        CloseCause {
            unit: b.open.unit,
            ..cause.clone()
        },
        CloseCause {
            sortie: b.open.sortie,
            ..cause.clone()
        },
        CloseCause {
            open: b.open.operation,
            ..cause.clone()
        },
        CloseCause {
            open_digest: digest(9),
            ..cause.clone()
        },
        CloseCause {
            operation: b.open.operation,
            ..cause.clone()
        },
        CloseCause {
            action: a.open.action.clone(),
            ..cause.clone()
        },
    ] {
        assert!(held.closing(crossed).is_err());
    }
    assert_eq!(held.open_len(), 2);
    let drained = held.closing(cause.clone())?.ok_or("close")?;
    assert_eq!(drained.open_len(), 1);
    assert_eq!(drained.paid_open(a.open.licence), 0);
    assert_eq!(drained.paid_open(b.open.licence), 1);
    let generation = drained.generation();
    assert!(drained.closing(cause.clone())?.is_none());
    // A retransmission with the same authentic event cannot rewrite its first
    // durable receipt even if a new submission has a different commit/stamp.
    let retry = CloseCause {
        operation: token(10002)?,
        at_ic: stamp(900),
        ..cause.clone()
    };
    assert!(drained.closing(retry)?.is_none());
    assert_eq!(drained.generation(), generation);
    let changed = CloseCause {
        event_sequence: 2,
        ..cause.clone()
    };
    assert!(drained.closing(changed).is_err());
    assert_eq!(
        drained
            .claim(a.open.operation)
            .and_then(|v| v.closed.as_ref()),
        Some(&cause)
    );
    Ok(())
}

#[test]
fn reconstruction_keeps_expired_claims_and_rejects_duplicate_unit_sortie_commit() -> TestResult {
    let a = row(1, 1, one_seat()?)?;
    let b = row(2, 2, one_seat()?)?;
    let mut closed = b.clone();
    let mut cause = close(&closed, 10002)?;
    cause.at_ic = ClockStamp {
        clock: ClockRef::WorldIc { generation: 7 },
        at: 5000,
    };
    closed.closed = Some(cause);
    let reboot = ClaimInventory::reconstruct(vec![closed, a.clone()])?;
    assert_eq!(reboot.open_len(), 1);
    assert_eq!(reboot.history_len(), 2);
    assert_eq!(reboot.paid_open(a.open.licence), 1);
    let mut crossed = b.clone();
    crossed.open.unit = a.open.unit;
    assert!(ClaimInventory::reconstruct(vec![a.clone(), crossed]).is_err());
    let mut crossed = b.clone();
    crossed.open.sortie = a.open.sortie;
    assert!(ClaimInventory::reconstruct(vec![a.clone(), crossed]).is_err());
    assert!(ClaimInventory::reconstruct(vec![a.clone(), a]).is_err());
    Ok(())
}

#[test]
fn global_open_bound_refuses_before_history_changes_and_real_close_frees_one_slot() -> TestResult {
    let rows = (1..=MAX_OPEN_IC_CLAIMS)
        .map(|i| row(u64::try_from(i)?, 1, Capacity::OwnedDispatch {}))
        .collect::<TestResult<Vec<_>>>()?;
    let first = rows.first().ok_or("first")?.clone();
    let held = ClaimInventory::reconstruct(rows)?;
    let mut extra = row(
        u64::try_from(MAX_OPEN_IC_CLAIMS + 1)?,
        1,
        Capacity::OwnedDispatch {},
    )?;
    assert!(matches!(
        held.opening(extra.clone()),
        Err(PublicationError::Capacity)
    ));
    assert_eq!(held.history_len(), MAX_OPEN_IC_CLAIMS);
    let cause = CloseCause {
        transition: held.next_transition(),
        ..close(&first, 100000)?
    };
    let drained = held.closing(cause)?.ok_or("close")?;
    assert_eq!(drained.open_len(), MAX_OPEN_IC_CLAIMS - 1);
    extra.open.sortie = SortieNumber::try_from(drained.next_transition())?;
    let reopened = drained.opening(extra)?;
    assert_eq!(reopened.open_len(), MAX_OPEN_IC_CLAIMS);
    assert_eq!(reopened.history_len(), MAX_OPEN_IC_CLAIMS + 1);
    Ok(())
}

#[test]
fn cold_replay_refuses_historical_over_admission_even_when_both_claims_are_closed() -> TestResult {
    let mut first = row(1, 1, one_seat()?)?;
    let mut second = row(2, 1, one_seat()?)?;
    first.closed = Some(CloseCause {
        transition: 3,
        ..close(&first, 10001)?
    });
    second.closed = Some(CloseCause {
        transition: 4,
        ..close(&second, 10002)?
    });
    assert!(matches!(
        ClaimInventory::reconstruct(vec![first.clone(), second.clone()]),
        Err(PublicationError::Capacity)
    ));
    // The same two histories are lawful only if first actually closed before
    // the next admission. Repair the exact owner-assigned event order as well
    // as the digest joined by the second close.
    first.closed = Some(close(&first, 10001)?);
    second.open.sortie = SortieNumber::try_from(3)?;
    second.closed = Some(close(&second, 10002)?);
    let book = ClaimInventory::reconstruct(vec![second.clone(), first.clone()])?;
    assert_eq!(book.open_len(), 0);
    assert_eq!(book.next_transition(), 5);
    second.closed.as_mut().ok_or("close")?.transition = 5;
    assert!(ClaimInventory::reconstruct(vec![first, second]).is_err());
    Ok(())
}

#[test]
fn historical_admission_and_nested_codecs_keep_the_closed_vocabulary() -> TestResult {
    let row = row(1, 1, one_seat()?)?;
    let raw = serde_json::to_vec(&row)?;
    assert_eq!(serde_json::from_slice::<ClaimRecord>(&raw)?, row);
    let mut omitted = serde_json::to_value(&row)?;
    omitted.as_object_mut().ok_or("object")?.remove("closed");
    assert!(serde_json::from_value::<ClaimRecord>(omitted).is_err());
    let mut unknown = serde_json::to_value(&row)?;
    unknown
        .as_object_mut()
        .ok_or("object")?
        .insert("allowed".into(), json!(true));
    assert!(serde_json::from_value::<ClaimRecord>(unknown).is_err());
    for value in [
        json!({"kind":"match_end"}),
        json!({"kind":"field_powerdown"}),
        json!({"kind":"destroyed","base":3}),
    ] {
        assert!(serde_json::from_value::<CloseKind>(value).is_err());
    }
    for at in [-1, 120, 121] {
        let mut stale = row.clone();
        stale.open.at_ic = stamp(at);
        assert!(stale.validate().is_err());
    }
    let mut grace = row.clone();
    grace.open.at_ic = stamp(119);
    grace.validate()?;
    let mut crossed = row.clone();
    crossed.open.protocol.participants.exclusion =
        crossed.open.protocol.participants.custody.clone();
    assert!(crossed.validate().is_err());
    let mut foreign = row.clone();
    foreign.open.action = foreign.open.protocol.action(Transition::Close)?;
    assert!(foreign.validate().is_err());
    Ok(())
}

#[test]
fn full_retained_history_refuses_new_open_but_preserves_the_last_genuine_close() -> TestResult {
    let mut rows = Vec::with_capacity(MAX_IC_CLAIM_HISTORY);
    for i in 1..=MAX_IC_CLAIM_HISTORY {
        let number = u64::try_from(i)?;
        let mut item = row(number, 1, one_seat()?)?;
        item.open.sortie = SortieNumber::try_from(
            number
                .checked_mul(2)
                .and_then(|n| n.checked_sub(1))
                .ok_or("ordinal")?,
        )?;
        if i != MAX_IC_CLAIM_HISTORY {
            item.closed = Some(close(&item, number.checked_add(100000).ok_or("commit")?)?);
        }
        rows.push(item);
    }
    let last = rows.last().ok_or("last")?.clone();
    let held = ClaimInventory::reconstruct(rows)?;
    assert_eq!(held.history_len(), MAX_IC_CLAIM_HISTORY);
    assert_eq!(held.open_len(), 1);
    let drained = held.closing(close(&last, 200000)?)?.ok_or("last close")?;
    assert_eq!(drained.open_len(), 0);
    assert_eq!(drained.history_len(), MAX_IC_CLAIM_HISTORY);
    let mut extra = row(300000, 1, one_seat()?)?;
    extra.open.sortie = SortieNumber::try_from(drained.next_transition())?;
    assert!(matches!(
        drained.opening(extra),
        Err(PublicationError::Capacity)
    ));
    assert_eq!(drained.history_len(), MAX_IC_CLAIM_HISTORY);
    assert!(drained.closing(close(&last, 200000)?)?.is_none());
    Ok(())
}

#[test]
fn close_rejects_same_clock_rollback_but_allows_actual_new_generation() -> TestResult {
    let open = row(1, 1, one_seat()?)?;
    let held = ClaimInventory::reconstruct(vec![open.clone()])?;
    let before = CloseCause {
        at_ic: stamp(open.open.at_ic.at - 1),
        ..close(&open, 10001)?
    };
    assert!(held.closing(before).is_err());
    assert_eq!(held.open_len(), 1);
    assert_eq!(held.claim(open.open.operation), Some(&open));
    let same = CloseCause {
        at_ic: open.open.at_ic.clone(),
        ..close(&open, 10001)?
    };
    assert_eq!(
        held.closing(same)?.ok_or("same instant close")?.open_len(),
        0
    );
    let changed = CloseCause {
        at_ic: ClockStamp {
            clock: ClockRef::WorldIc { generation: 2 },
            at: 0,
        },
        ..close(&open, 10001)?
    };
    assert_eq!(
        held.closing(changed)?
            .ok_or("changed clock close")?
            .open_len(),
        0
    );
    Ok(())
}
