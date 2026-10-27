//! 17g C2f (the C2b brief §2; the C2e plan §3.2): the per-holder paid-licence
//! index, `roll_mind_paid_holdings`.
//!
//! One row per held paid licence, keyed `{holder:020}/licence/{licence:016x}`
//! (zero-padded decimal holder, so one holder's rows are contiguous and a
//! prefix page never reaches a neighbour's). It **authorizes nothing**, like
//! v2's `ActorOffers`: it only lets `+minds/holdings` find the licences the
//! actor holds with a bounded read (`page`, C2g's `OwnHoldings { section:
//! Paid }`); every fact a row shows is reread from the licence's own rows
//! (`command_reader::own_paid`).
//!
//! It is written wherever a paid-licence pointer's holder is SET, inside the
//! same money transaction as the pointer, and nowhere else:
//! - `grant` beside the three pointer inserts: the entitlement grant
//!   (`entitlement::writer`), the House grant (`house_purchase::writer`) and
//!   the v2 grant (`purchase_writer`);
//! - `transfer` beside the resale's two pointer replaces: the immediate Accept
//!   and the service Settle (`paid_resale::writer`).
//!
//! The renewal writers replace the pointer with a new term and the same
//! holder and write no row; a revocation and a lapse change no holder and no
//! row (a revoked licence keeps its row, which C2g's `; revoked` paid row
//! needs). There is no cap per holder, so no money path gains a refusal.
//!
//! The family is delegated to the market as `paid_licences` is (the market's
//! transactions run all five writes), and mush-core's closed owner-index table
//! admits exactly these rows in those transitions (`terminal_capacity::
//! owner_index`, R2). `audit` (Roll's commerce boot audit) checks the whole
//! family: exactly one row per pointer, under the pointer's current holder.
use super::LicenceId;
use super::command_reader::PaidHoldingView;
use super::families::CommerceFamilies;
use super::records::{PublicationError, Record};
use mush_core::read_service::{OwnerReadContext, OwnerReadError};
use mush_core::{ActionTxn, ModuleCtx, ObjId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Paid rows one `page` answers at most (a provisional default, 17g §1
/// ruling 6): each row rereads its licence's own rows, so a page stays well
/// inside one owner read's budget.
pub const PAID_ROWS_PER_READ: usize = 3;

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

/// One held paid licence. Source-free and authorizes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaidHolding {
    pub(crate) format: u16,
    pub(crate) holder: ObjId,
    pub(crate) licence: LicenceId,
}
impl PaidHolding {
    pub(crate) fn new(holder: ObjId, licence: LicenceId) -> Result<Self, PublicationError> {
        let row = Self {
            format: 1,
            holder,
            licence,
        };
        row.validate()?;
        Ok(row)
    }
    #[must_use]
    pub const fn holder(&self) -> ObjId {
        self.holder
    }
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    /// The row key: `{holder:020}/licence/{licence:016x}`.
    #[must_use]
    pub fn key_for(holder: ObjId, licence: LicenceId) -> String {
        format!("{}/{}", holder_prefix(holder), licence.key())
    }
}
/// The holder's key prefix, `{holder:020}`.
fn holder_prefix(holder: ObjId) -> String {
    format!("{:020}", holder.0)
}
impl Record for PaidHolding {
    const FAMILY: &'static str = "roll_mind_paid_holdings";
    const MAX_BYTES: usize = 1024;
    fn key(&self) -> String {
        Self::key_for(self.holder, self.licence)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || self.holder.0 == 0 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// A grant's holder row, beside its pointer insert. Only this holder's key
/// is checked here (no row may exist under it); that no row for the licence
/// exists under any other holder is the boot audits' check (`audit`, and
/// mush-core's owner-index audit), which allow exactly one row per pointer.
/// The read-back matches the inserted row.
pub(crate) fn grant(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    holder: ObjId,
    licence: LicenceId,
) -> Result<(), PublicationError> {
    let row = PaidHolding::new(holder, licence)?;
    let key = row.key();
    if txn.get(&f.paid_holdings, &key).map_err(bad)?.is_some() {
        return Err(PublicationError::Mismatch);
    }
    txn.insert(&f.paid_holdings, &key, row.clone())
        .map_err(bad)?;
    if txn.get(&f.paid_holdings, &key).map_err(bad)? != Some(&row) {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// A resale settlement's holder move, beside its pointer replace: the
/// seller's exact row is deleted and the buyer's inserted.
pub(crate) fn transfer(
    txn: &mut ActionTxn<'_>,
    f: &CommerceFamilies,
    seller: ObjId,
    buyer: ObjId,
    licence: LicenceId,
) -> Result<(), PublicationError> {
    let old = PaidHolding::new(seller, licence)?;
    let new = PaidHolding::new(buyer, licence)?;
    if seller == buyer
        || txn.get(&f.paid_holdings, &old.key()).map_err(bad)? != Some(&old)
        || txn
            .get(&f.paid_holdings, &new.key())
            .map_err(bad)?
            .is_some()
    {
        return Err(PublicationError::Mismatch);
    }
    txn.remove(&f.paid_holdings, &old.key()).map_err(bad)?;
    txn.insert(&f.paid_holdings, &new.key(), new.clone())
        .map_err(bad)?;
    if txn
        .get(&f.paid_holdings, &old.key())
        .map_err(bad)?
        .is_some()
        || txn.get(&f.paid_holdings, &new.key()).map_err(bad)? != Some(&new)
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// The clock-free reconciliation `audit` runs: `pointers` are every paid
/// licence's (holder, id), `rows` every holdings row with its stored key.
/// Exactly one row per pointer under its current holder; no orphan row.
pub(crate) fn reconcile(
    pointers: impl IntoIterator<Item = (ObjId, LicenceId)>,
    rows: impl IntoIterator<Item = (String, PaidHolding)>,
) -> Result<(), PublicationError> {
    let mut expected = BTreeMap::new();
    for (holder, licence) in pointers {
        let row = PaidHolding::new(holder, licence)?;
        if expected.insert(row.key(), row).is_some() {
            return Err(PublicationError::Mismatch);
        }
    }
    for (key, row) in rows {
        row.validate()?;
        if row.key() != key || expected.remove(&key).as_ref() != Some(&row) {
            return Err(PublicationError::Mismatch);
        }
    }
    if !expected.is_empty() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// Roll's commerce boot audit of the family: every paid-licence pointer
/// against every holdings row (`reconcile`). A world whose index disagrees
/// with its pointers refuses to boot. A store from before C2f, with paid
/// licences and no rows, refuses too: acceptable pre-launch (a fresh store).
pub(crate) fn audit(ctx: &ModuleCtx<'_>, f: &CommerceFamilies) -> Result<(), PublicationError> {
    let mut pointers = Vec::new();
    super::audit::scan(ctx, &f.paid_licences, |p| {
        pointers.push((p.holder(), p.id()));
        Ok(())
    })?;
    let mut rows = Vec::new();
    super::audit::scan(ctx, &f.paid_holdings, |row| {
        rows.push((row.key(), row.clone()));
        Ok(())
    })?;
    reconcile(pointers, rows)
}

/// One page of the actor's own paid licences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidHoldingsPage {
    rows: Vec<PaidHoldingView>,
    next: Option<LicenceId>,
}
impl PaidHoldingsPage {
    #[must_use]
    pub fn rows(&self) -> &[PaidHoldingView] {
        &self.rows
    }
    /// The cursor to continue after, when the holder has more rows.
    #[must_use]
    pub const fn next(&self) -> Option<LicenceId> {
        self.next
    }
}

/// 17g C2f (for C2g's `OwnHoldings { section: Paid }`): one page of the paid
/// licences `holder` holds, in licence order, after `after`: one bounded scan
/// of `PAID_ROWS_PER_READ + 1` keys from `page_start` (the extra one only
/// telling whether more follow), cut by `cut` at the first key outside the
/// holder's prefix, so it never returns a neighbour's row. Each row is built
/// exactly as `command_reader::own_paid` builds a `PaidHoldingView`.
/// Own-only: a `holder` other than the calling actor is `Refused`.
/// Unused until C2g wires the read kind to it (the allowance goes then).
#[allow(dead_code)]
pub(crate) fn page(
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    holder: ObjId,
    after: Option<LicenceId>,
) -> Result<PaidHoldingsPage, OwnerReadError> {
    if holder != ctx.actor() {
        return Err(OwnerReadError::Refused);
    }
    let start = page_start(holder, after);
    let scanned = ctx.scan_action_state(
        &families.paid_holdings,
        Some(start.as_str()),
        PAID_ROWS_PER_READ + 1,
    )?;
    let entries: Vec<(String, PaidHolding)> = scanned
        .entries()
        .iter()
        .map(|entry| (entry.encoded_key().to_owned(), entry.value().clone()))
        .collect();
    let (rows, next) = cut(&entries, holder, |row| {
        super::command_reader::own_paid(ctx, families, row.licence)
    })?;
    Ok(PaidHoldingsPage { rows, next })
}

/// Where a holder's page scan starts (exclusively): the holder's bare prefix
/// `{holder:020}/`, which sorts before every one of its rows, or the row the
/// previous page ended on.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn page_start(holder: ObjId, after: Option<LicenceId>) -> String {
    after.map_or_else(
        || format!("{}/", holder_prefix(holder)),
        |licence| PaidHolding::key_for(holder, licence),
    )
}

/// One page from the scanned `entries` (in key order, from `page_start`):
/// the rows under `holder`'s prefix only, stopping at the first other key;
/// `row_of` each of the first `PAID_ROWS_PER_READ`; the cursor is the last
/// row's licence when one more of the holder's rows follows.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn cut<T>(
    entries: &[(String, PaidHolding)],
    holder: ObjId,
    mut row_of: impl FnMut(&PaidHolding) -> Result<T, OwnerReadError>,
) -> Result<(Vec<T>, Option<LicenceId>), OwnerReadError> {
    let prefix = format!("{}/", holder_prefix(holder));
    let mine: Vec<&PaidHolding> = entries
        .iter()
        .take_while(|(key, _)| key.starts_with(&prefix))
        .map(|(key, row)| {
            if row.validate().is_err() || row.holder != holder || row.key() != *key {
                return Err(OwnerReadError::Integrity);
            }
            Ok(row)
        })
        .collect::<Result<_, _>>()?;
    let mut rows = Vec::new();
    let mut last = None;
    for row in mine.iter().take(PAID_ROWS_PER_READ) {
        rows.push(row_of(row)?);
        last = Some(row.licence);
    }
    let next = if mine.len() > PAID_ROWS_PER_READ {
        last
    } else {
        None
    };
    Ok((rows, next))
}

#[cfg(test)]
mod tests;
