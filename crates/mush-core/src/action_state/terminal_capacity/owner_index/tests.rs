#![cfg(test)]

//! 17g C2f: the owner-index fixture helpers every protocol fixture shares.
//! The rows are literal JSON, written as the owners' typed writers write
//! them; they never call `expected`, so a fixture that commits is an
//! independent witness of what the table admits.
use super::*;

type Handle = StateFamilyHandle<String, Value>;
type Handles = BTreeMap<String, Handle>;
/// 17g C2f: a fixture transition's index writes, supplied by a test in place
/// of the owners' default writes (the default is `None`).
pub(in crate::action_state::terminal_capacity) type IndexWriter<'w> =
    &'w dyn Fn(&mut ActionTxn<'_>, &Handles) -> Result<(), ActionStateError>;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Register `families` (of the four) exactly as their owners do, with the
/// fixtures' inert codecs, and the holdings family's delegation to market.
pub(in crate::action_state::terminal_capacity) fn register_index_families(
    builder: &mut StateFamilyRegistryBuilder,
    handles: &mut Handles,
    families: &[&str],
) -> TestResult {
    for family in families {
        let (owner, descriptor, delegate) = descriptor(family)?;
        let handle = builder.register(
            owner.clone(),
            descriptor,
            |key: &String| Ok(key.clone()),
            |row: &Value| Ok(row.clone()),
            |_: &Value| Ok(()),
            |row: &Value| Ok(row.clone()),
        )?;
        if let Some(grantee) = delegate {
            builder.delegate_family(owner, &handle, OwnerId::new(grantee)?)?;
        }
        handles.insert((*family).to_owned(), handle);
    }
    Ok(())
}

fn handle<'a>(handles: &'a Handles, family: &str) -> Result<&'a Handle, ActionStateError> {
    handles
        .get(family)
        .ok_or_else(|| refused("fixture index family"))
}
fn strings(v: &Value) -> Result<Vec<String>, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(|_| refused("fixture token set"))
}
type TxnResult = Result<(), ActionStateError>;

/// The literal holdings key and row of `holder` for `licence`.
pub(in crate::action_state::terminal_capacity) fn holding_row(
    holder: u64,
    licence: u64,
) -> (String, Value) {
    (
        format!("{holder:020}/licence/{licence:016x}"),
        json!({"format":1,"holder":holder,"licence":licence.to_string()}),
    )
}

/// A grant's holdings put, as Roll's `paid_holdings::grant` writes it.
pub(in crate::action_state::terminal_capacity) fn put_holding(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    holder: u64,
    licence: u64,
) -> TxnResult {
    let (k, row) = holding_row(holder, licence);
    txn.insert(handle(handles, HOLDINGS)?, &k, row)?;
    Ok(())
}

/// A resale settlement's holdings move, as Roll's `paid_holdings::transfer`
/// writes it.
pub(in crate::action_state::terminal_capacity) fn move_holding(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    seller: u64,
    buyer: u64,
    licence: u64,
) -> TxnResult {
    let h = handle(handles, HOLDINGS)?;
    txn.remove(h, &holding_row(seller, licence).0)?;
    let (k, row) = holding_row(buyer, licence);
    txn.insert(h, &k, row)?;
    Ok(())
}

/// The buyer's actors row gains `operation` (the market's Fund write; an
/// ordinary typed write core never checks).
pub(in crate::action_state::terminal_capacity) fn index_operation(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    buyer: u64,
    operation: CommitToken,
) -> TxnResult {
    let h = handle(handles, ACTORS)?;
    let k = buyer.to_string();
    match txn.get(h, &k)?.cloned() {
        Some(mut row) => {
            let mut operations = strings(&row["operations"])?;
            operations.push(operation.to_hex());
            operations.sort();
            row["operations"] = json!(operations);
            txn.replace(h, &k, row)?;
        }
        None => {
            txn.insert(
                h,
                &k,
                json!({"format":1,"actor":buyer,"quotes":[],"operations":[operation]}),
            )?;
        }
    }
    Ok(())
}

/// The buyer's actors row loses `operation` at a Grant or Refund (the
/// market's terminal write): a Put, or a Delete when both sets are empty.
pub(in crate::action_state::terminal_capacity) fn release_operation(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    buyer: u64,
    operation: CommitToken,
) -> TxnResult {
    let h = handle(handles, ACTORS)?;
    let k = buyer.to_string();
    let mut row = txn
        .get(h, &k)?
        .cloned()
        .ok_or_else(|| refused("fixture actors row"))?;
    let mut operations = strings(&row["operations"])?;
    operations.retain(|t| *t != operation.to_hex());
    let quotes = strings(&row["quotes"])?;
    if operations.is_empty() && quotes.is_empty() {
        txn.remove(h, &k)?;
    } else {
        row["operations"] = json!(operations);
        txn.replace(h, &k, row)?;
    }
    Ok(())
}

/// A named buyer's offer row gains `offer` (both Offers): `head` is the row
/// without its `offers` (`{"format":1,"buyer":..,"licence"|"charter":..}`).
pub(in crate::action_state::terminal_capacity) fn index_offer(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    family: &str,
    key: &str,
    head: Value,
    offer: CommitToken,
) -> TxnResult {
    let h = handle(handles, family)?;
    let k = key.to_owned();
    match txn.get(h, &k)?.cloned() {
        Some(mut row) => {
            let mut offers = strings(&row["offers"])?;
            offers.push(offer.to_hex());
            offers.sort();
            row["offers"] = json!(offers);
            txn.replace(h, &k, row)?;
        }
        None => {
            let mut row = head;
            row["offers"] = json!([offer]);
            txn.insert(h, &k, row)?;
        }
    }
    Ok(())
}

/// A named buyer's offer row loses `offer` (resale Accept or Fund, pending
/// Fund): a Put, or a Delete when no token is left.
pub(in crate::action_state::terminal_capacity) fn consume_offer(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    family: &str,
    key: &str,
    offer: CommitToken,
) -> TxnResult {
    let h = handle(handles, family)?;
    let k = key.to_owned();
    let mut row = txn
        .get(h, &k)?
        .cloned()
        .ok_or_else(|| refused("fixture offer row"))?;
    let mut offers = strings(&row["offers"])?;
    offers.retain(|t| *t != offer.to_hex());
    if offers.is_empty() {
        txn.remove(h, &k)?;
    } else {
        row["offers"] = json!(offers);
        txn.replace(h, &k, row)?;
    }
    Ok(())
}

/// Put `value` at `key` in `family` (an insert, or a replace when present).
pub(in crate::action_state::terminal_capacity) fn put_row(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    family: &str,
    key: &str,
    value: Value,
) -> TxnResult {
    let h = handle(handles, family)?;
    let k = key.to_owned();
    if txn.get(h, &k)?.is_some() {
        txn.replace(h, &k, value)?;
    } else {
        txn.insert(h, &k, value)?;
    }
    Ok(())
}

/// Delete the row at `key` in `family`.
pub(in crate::action_state::terminal_capacity) fn delete_row(
    txn: &mut ActionTxn<'_>,
    handles: &Handles,
    family: &str,
    key: &str,
) -> TxnResult {
    txn.remove(handle(handles, family)?, &key.to_owned())?;
    Ok(())
}
