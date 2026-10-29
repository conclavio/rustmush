#![cfg(test)]

//! 17g C2f (the C2b brief §2, tests 9 and 10): the per-holder paid index over
//! real typed transactions in an engine that registers Roll's commerce
//! families exactly as the module does (no purchase protocol composed, so
//! the structural pointer rows below are inert data, never paid authority),
//! with the family's real boot audit (`audit`, the step Roll's commerce boot
//! audit runs) as the rig's boot: each grant's exact post-images are
//! hydrated into a fresh engine and booted (a resold, format-2 pointer
//! hydrates only with the resale protocol composed, so after a settlement
//! the audit runs on the live store), and the live store's audit refuses a
//! missing, crossed or orphan row.
//!
//! What this rig does NOT run: the five holder-setting writers themselves.
//! The entitlement, House and v2 grant writers call `grant`, and the
//! resale's immediate Accept and service Settle call `transfer`, beside
//! their pointer writes (`entitlement::writer`, `house_purchase::writer`,
//! `purchase_writer`, `paid_resale::writer`); those writers need their whole
//! purchase or resale protocol composed, so they run only in the composed
//! launches and the M7 stories. Here `granted` and `settled` stand in for
//! them with the same pointer write and the same call; mush-core's
//! `index_roll_mind_paid_holdings_*` tests prove those transitions admit and
//! require exactly these rows.
use super::super::families::CommerceFamilies;
use super::super::paid::PersonalPaidLicence;
use super::super::{CharterId, LicenceId, LineId, TermGeneration};
use super::*;
use mush_core::{
    ActionStateBootRecord, CommitToken, DefinitionDigest, Engine, GameModule, ModError,
    ModuleBootData, ModuleWrite, ObjId, Outbox, Registrar, Registry, World,
};
use mush_economy::managed_paid_resale::PaidHolderRefV1;
use std::sync::{Arc, Mutex};

/// The character holder `n`.
fn h(n: u32) -> Holder {
    Holder::Character(ObjId(n))
}

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
/// The exact persisted documents, by namespace and key.
type Disk = BTreeMap<(String, String), serde_json::Value>;

struct Capture(Arc<Mutex<Option<CommerceFamilies>>>);
impl GameModule for Capture {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let f = CommerceFamilies::register(reg)?;
        reg.state(f.clone());
        *self.0.lock().map_err(|_| ModError::msg("families"))? = Some(f);
        Ok(())
    }
    /// The family's own boot audit, as Roll's commerce boot audit runs it.
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _: ModuleBootData,
    ) -> Result<(), ModError> {
        let f = ctx
            .w
            .module_state::<CommerceFamilies>()
            .ok_or_else(|| ModError::msg("families"))?
            .clone();
        audit(ctx, &f).map_err(|e| ModError::msg(format!("paid holdings audit: {e:?}")))
    }
}
fn rig() -> TestResult<(Engine, CommerceFamilies)> {
    let shared = Arc::new(Mutex::new(None));
    let e = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Capture(shared.clone()))],
    )?;
    let f = shared
        .lock()
        .map_err(|_| "families")?
        .clone()
        .ok_or("families")?;
    Ok((e, f))
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn licence(n: u64) -> TestResult<LicenceId> {
    Ok(LicenceId::new(n)?)
}
/// A first-term personal paid-licence pointer held by `holder`.
fn pointer(id: u64, holder: u32) -> TestResult<PersonalPaidLicence> {
    Ok(PersonalPaidLicence::original(
        licence(id)?,
        CharterId::new(9)?,
        LineId::new(2)?,
        h(holder),
        TermGeneration::new(1)?,
        token(u8::try_from(id)?),
    )?)
}
/// One typed transaction over the rig's Roll families; its exact writes are
/// folded into `disk`.
fn change(
    e: &mut Engine,
    f: &CommerceFamilies,
    disk: &mut Disk,
    run: impl FnOnce(&mut ActionTxn<'_>) -> Result<(), PublicationError>,
) -> TestResult {
    let mut txn = e.action_transaction(&f.paid_holdings)?;
    run(&mut txn)?;
    let (ops, install) = txn.prepare()?.into_parts();
    e.install_prepared_action(install)?;
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                disk.insert((op.ns.to_string(), key.to_string()), value);
            }
            ModuleWrite::Del { key } => {
                disk.remove(&(op.ns.to_string(), key.to_string()));
            }
            _ => return Err("typed fixture emitted a non-document write".into()),
        }
    }
    Ok(())
}
/// Hydrate `disk` into a fresh rig and run its boot (mush-core's typed
/// audits, then the family's own `audit`).
fn boot(disk: &Disk) -> TestResult {
    let (mut e, _) = rig()?;
    e.hydrate_action_state(disk.iter().map(|((ns, key), value)| {
        ActionStateBootRecord::new(ns.as_str(), key.as_str(), value.clone())
    }))?;
    live_boot(&mut e)
}
/// The rig's boot on its live store: the family's own `audit` alone.
fn live_boot(e: &mut Engine) -> TestResult {
    e.module_loaded(
        "roll",
        ModuleBootData::default(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    Ok(())
}
/// A grant: the pointer inserted and `grant` beside it, as the three grant
/// writers do.
fn granted(
    e: &mut Engine,
    f: &CommerceFamilies,
    disk: &mut Disk,
    id: u64,
    holder: u32,
) -> TestResult {
    let p = pointer(id, holder)?;
    change(e, f, disk, |t| {
        t.insert(&f.paid_licences, &p.key(), p.clone())
            .map_err(bad)?;
        grant(t, f, p.holder(), p.id())
    })
}
/// A resale settlement: the pointer replaced with the buyer and `transfer`
/// beside it, as the immediate Accept and the service Settle do.
fn settled(
    e: &mut Engine,
    f: &CommerceFamilies,
    disk: &mut Disk,
    id: u64,
    buyer: u32,
) -> TestResult {
    let before = e
        .action_state()
        .get(&f.paid_licences, &licence(id)?.key())?
        .cloned()
        .ok_or("pointer")?;
    let mut moved = serde_json::to_value(&before)?;
    moved["format"] = serde_json::json!(2);
    moved["holder"] = serde_json::json!(buyer);
    moved["ownership"] = serde_json::json!(PaidHolderRefV1 {
        generation: 2,
        operation: token(90),
        digest: DefinitionDigest::from_bytes([9; 32]),
    });
    let after: PersonalPaidLicence = serde_json::from_value(moved)?;
    change(e, f, disk, |t| {
        t.replace(&f.paid_licences, &after.key(), after.clone())
            .map_err(bad)?;
        transfer(t, f, before.holder(), after.holder(), after.id())
    })
}
/// Insert (`true`) or delete (`false`) holdings rows with an ordinary typed
/// write, outside any writer: the damage the audit must refuse.
fn set_rows(
    e: &mut Engine,
    f: &CommerceFamilies,
    disk: &mut Disk,
    rows: &[(PaidHolding, bool)],
) -> TestResult {
    change(e, f, disk, |t| {
        for (row, present) in rows {
            if *present {
                t.insert(&f.paid_holdings, &row.key(), row.clone())
                    .map_err(bad)?;
            } else {
                t.remove(&f.paid_holdings, &row.key()).map_err(bad)?;
            }
        }
        Ok(())
    })
}
fn held(e: &Engine, f: &CommerceFamilies, holder: u32, id: u64) -> TestResult<bool> {
    Ok(e.action_state()
        .get(
            &f.paid_holdings,
            &PaidHolding::key_for(h(holder), licence(id)?),
        )?
        .is_some())
}
/// One `page` of `holder`'s licences after `after`, over the store's own
/// scan from `page_start` (`cut`, with each row's licence as its view).
fn page_of(
    e: &Engine,
    f: &CommerceFamilies,
    holder: u32,
    after: Option<u64>,
) -> TestResult<(Vec<u64>, Option<u64>)> {
    let after = after.map(licence).transpose()?;
    let start = page_start(h(holder), after);
    let scanned = e.action_state().scan_family(
        &f.paid_holdings,
        Some(start.as_str()),
        PAID_ROWS_PER_READ + 1,
    )?;
    let entries = scanned
        .entries()
        .iter()
        .map(|entry| (entry.encoded_key().to_owned(), entry.value().clone()))
        .collect::<Vec<_>>();
    let (rows, next) =
        cut(&entries, h(holder), |row| Ok(row.licence().get())).map_err(|e| format!("{e:?}"))?;
    Ok((rows, next.map(LicenceId::get)))
}

/// 17g C2f (the C2b brief test 9): one row after each grant, under the
/// pointer's holder; each resale settlement (the immediate Accept's and the
/// service Settle's shared `transfer`) moves the row from seller to buyer; a
/// holder-preserving pointer write (a renewal's next term; a revocation or a
/// lapse writes no pointer at all) moves nothing; the exact post-images
/// after every grant hydrate and boot through the family's audit, which
/// also passes on the live store after each settlement and the renewal (a
/// resold pointer hydrates only with the resale protocol composed); and
/// `page` returns only the holder's rows, in licence order, with a cursor,
/// never a neighbour's. (The grant and settlement steps stand in for the
/// five writers, which need their protocols composed: see the module doc.)
#[test]
fn paid_holdings_follow_every_holder_write() -> TestResult {
    let (mut e, f) = rig()?;
    let mut disk = Disk::new();
    // The holders 7, 8 and 70: zero-padded, 8's rows sort between 7's and
    // 70's, the neighbours on either side of its prefix.
    for (id, holder) in [(1, 7), (2, 7), (3, 8), (4, 70), (5, 8), (6, 8), (7, 8)] {
        granted(&mut e, &f, &mut disk, id, holder)?;
        check!(held(&e, &f, holder, id)?, "row after grant of {id}");
        boot(&disk)?;
    }
    let three = licence(3)?;
    check!(
        change(&mut e, &f, &mut disk, |t| grant(t, &f, h(8), three)).is_err(),
        "a grant over an existing row refuses"
    );
    // A resold (format-2) pointer cannot be hydrated here: mush-core refuses
    // it without the resale protocol composed ("resale pointer without
    // declaration"). From here the family's audit runs on the live store.
    settled(&mut e, &f, &mut disk, 1, 8)?;
    check!(
        !held(&e, &f, 7, 1)? && held(&e, &f, 8, 1)?,
        "immediate Accept moved the row"
    );
    live_boot(&mut e)?;
    settled(&mut e, &f, &mut disk, 2, 70)?;
    check!(
        !held(&e, &f, 7, 2)? && held(&e, &f, 70, 2)?,
        "Settle moved the row"
    );
    live_boot(&mut e)?;
    // A renewal replaces the pointer with its next term and the same holder.
    let current = e
        .action_state()
        .get(&f.paid_licences, &licence(3)?.key())?
        .cloned()
        .ok_or("pointer")?;
    let renewed = current.with_next_term(TermGeneration::new(2)?)?;
    change(&mut e, &f, &mut disk, |t| {
        t.replace(&f.paid_licences, &renewed.key(), renewed.clone())
            .map_err(bad)
    })?;
    check!(held(&e, &f, 8, 3)?, "a renewal moves nothing");
    live_boot(&mut e)?;
    // A transfer from a seller who holds no row refuses.
    let revision = e.action_state().revision();
    let six = licence(6)?;
    check!(change(&mut e, &f, &mut disk, |t| transfer(t, &f, h(7), h(8), six)).is_err());
    check_eq!(e.action_state().revision(), revision);
    // Holder 8 holds licences 1, 3, 5, 6 and 7: two pages of three.
    check_eq!(page_of(&e, &f, 8, None)?, (vec![1, 3, 5], Some(5)));
    check_eq!(page_of(&e, &f, 8, Some(5))?, (vec![6, 7], None));
    check_eq!(
        page_of(&e, &f, 7, None)?,
        (vec![], None),
        "7 holds nothing now"
    );
    check_eq!(page_of(&e, &f, 70, None)?, (vec![2, 4], None));
    check_eq!(page_of(&e, &f, 9, None)?, (vec![], None));
    Ok(())
}

/// 17g C2f (the C2b brief test 10): the audit requires exactly one row per
/// pointer, under the pointer's current holder. On the live store, the
/// family's real `audit` (its scans and `reconcile`) refuses a missing row,
/// a row moved under another holder and an orphan row, and passes again
/// once each is repaired; a hydrated boot of a missing row refuses too
/// (mush-core's own owner-index audit, before Roll's). `reconcile` alone
/// refuses a row under a key it does not name and two pointers for one
/// licence. (The post-images after each step boot in the test above.)
#[test]
fn paid_holdings_audit_requires_exactly_one_row_per_pointer() -> TestResult {
    let (mut e, f) = rig()?;
    let mut disk = Disk::new();
    granted(&mut e, &f, &mut disk, 1, 7)?;
    granted(&mut e, &f, &mut disk, 2, 8)?;
    live_boot(&mut e)?;
    let one = licence(1)?;
    let mine = PaidHolding::new(h(7), one)?;
    let theirs = PaidHolding::new(h(8), one)?;
    let orphan = PaidHolding::new(h(8), licence(3)?)?;
    let mut scratch = disk.clone();
    let audit_reason = "paid holdings audit: Mismatch";
    for (case, broken, repaired) in [
        (
            "missing",
            vec![(mine.clone(), false)],
            vec![(mine.clone(), true)],
        ),
        (
            "wrong holder",
            vec![(mine.clone(), false), (theirs.clone(), true)],
            vec![(theirs.clone(), false), (mine.clone(), true)],
        ),
        (
            "orphan",
            vec![(orphan.clone(), true)],
            vec![(orphan, false)],
        ),
    ] {
        set_rows(&mut e, &f, &mut scratch, &broken)?;
        check!(
            live_boot(&mut e)
                .err()
                .is_some_and(|e| e.to_string().contains(audit_reason)),
            "{case}"
        );
        set_rows(&mut e, &f, &mut scratch, &repaired)?;
        live_boot(&mut e)?;
    }
    let mut omitted = disk.clone();
    omitted.retain(|(_, key), _| !key.contains(PaidHolding::FAMILY));
    check!(boot(&omitted).is_err(), "a hydrated missing row refuses");
    boot(&disk)?;
    let row = |holder: u32, id: u64| -> TestResult<(String, PaidHolding)> {
        let r = PaidHolding::new(h(holder), licence(id)?)?;
        Ok((r.key(), r))
    };
    let pointers = vec![(h(7), licence(1)?), (h(8), licence(2)?)];
    reconcile(pointers.clone(), vec![row(7, 1)?, row(8, 2)?])?;
    let crossed = {
        let (_, r) = row(7, 2)?;
        (PaidHolding::key_for(h(8), licence(2)?), r)
    };
    for (case, rows) in [
        ("missing", vec![row(7, 1)?]),
        ("wrong holder", vec![row(7, 1)?, row(7, 2)?]),
        ("orphan", vec![row(7, 1)?, row(8, 2)?, row(8, 3)?]),
        ("crossed key", vec![row(7, 1)?, crossed]),
    ] {
        check!(reconcile(pointers.clone(), rows).is_err(), "{case}");
    }
    check!(
        reconcile(
            vec![(h(7), licence(1)?), (h(7), licence(1)?)],
            vec![row(7, 1)?]
        )
        .is_err(),
        "one pointer per licence"
    );
    check_eq!(
        PaidHolding::key_for(h(8), licence(2)?),
        "00000000000000000008/licence/0000000000000002"
    );
    check!(super::super::families::valid_key(
        PaidHolding::FAMILY,
        "00000000000000000008/licence/0000000000000002"
    ));
    for bad_key in [
        "8/licence/0000000000000002",
        "00000000000000000000/licence/0000000000000002",
        "00000000000000000008/licence/0000000000000000",
        "00000000000000000008/charter/0000000000000002",
    ] {
        check!(
            !super::super::families::valid_key(PaidHolding::FAMILY, bad_key),
            "{bad_key}"
        );
    }
    Ok(())
}
