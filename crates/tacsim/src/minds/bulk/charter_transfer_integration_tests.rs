#![cfg(test)]

//! Real signed offers and native consideration over the parent's genuinely
//! certified original filing and paid Grant. No additional Registry run or
//! structural receipt is substituted for owner evidence.
use super::charter_transfer_fixture::{ACCEPT, OFFER, consideration, offer_params, signed};
use super::*;
const START: u128 = 48_000_000;
const OFFERS: &str = "roll_mind_charter_transfer_offers";
const TRANSFERS: &str = "roll_mind_charter_transfers";
const HEADS: &str = "roll_mind_charter_ownership";

fn immutable(f: &PurchaseFixture) -> Vec<(String, Vec<Value>)> {
    [
        "roll_mind_initial_filing",
        "roll_mind_owner_licence",
        "roll_mind_owner_term",
        "roll_mind_charter",
        "roll_mind_line",
        "roll_mind_filed",
        "roll_mind_status",
        "roll_mind_personal_paid_licence",
        "roll_mind_paid_term",
        "roll_mind_paid_purchase",
    ]
    .into_iter()
    .map(|family| (family.to_owned(), f.rows("roll", family)))
    .collect()
}
fn current_generation(f: &PurchaseFixture) -> TestResult<u64> {
    Ok(one(f.rows("roll", HEADS))?
        .pointer("/current/generation")
        .and_then(Value::as_u64)
        .ok_or("actual current ownership generation")?)
}
fn assert_title(f: &PurchaseFixture, charter: ObjId, owner: ObjId) -> TestResult {
    check_eq!(
        f.economy()?
            .snapshot()
            .asset(charter)
            .ok_or("actual title")?
            .title_owner,
        EconomicOwner::Object(owner)
    );
    Ok(())
}
pub(super) fn exercise(base: &PurchaseFixture, granted: &Durable) -> TestResult {
    let mut f = base.reboot(granted)?;
    let original = paid_holdings::own(&mut f, GOD, 1)?;
    let paid = paid_holdings::own(&mut f, BUYER, 2)?;
    let charter = original.charter().object();
    let immutable_before = immutable(&f);
    let roots_before = f.roots()?;
    let money_before = f.balances()?;
    let (seller, payout, payer) = consideration(&f.engine, charter, BUYER)?;
    check_eq!(seller, GOD);
    let parameters = offer_params(charter, BUYER, payout.id);
    for (field, value) in [
        ("source", json!("not authority")),
        ("charter", json!(format!("0{}", charter.0))),
        ("price_minor", json!("0")),
    ] {
        let mut bad = parameters.clone();
        let object = bad.as_object_mut().ok_or("offer params")?;
        object.insert(field.into(), value);
        check!(crate::test_checks::refused(signed(
            &mut f.engine,
            OFFER,
            GOD,
            START,
            bad
        ))?);
    }
    let prepared = signed(&mut f.engine, OFFER, GOD, START, parameters.clone())?;
    check!(f.rows("roll", OFFERS).is_empty());
    check_eq!(f.balances()?, money_before);
    assert_title(&f, charter, GOD)?;
    f.install(prepared)?;
    check_eq!(f.rows("roll", OFFERS).len(), 1);
    check!(f.rows("roll", TRANSFERS).is_empty());
    check_eq!(f.balances()?, money_before);
    let offered = f.snapshot();
    let accept = json!({"offer":token(START),"payer":payer.id.0.to_string()});
    check!(crate::test_checks::refused(signed(
        &mut f.engine,
        ACCEPT,
        GOD,
        START + 1,
        accept.clone()
    ))?);
    for (field, value) in [
        ("payer", json!(payer.id.0)),
        ("offer", json!(token(START + 999))),
    ] {
        let mut bad = accept.clone();
        *bad.get_mut(field).ok_or("accept field")? = value;
        check!(crate::test_checks::refused(signed(
            &mut f.engine,
            ACCEPT,
            BUYER,
            START + 1,
            bad
        ))?);
    }
    let prepared = signed(&mut f.engine, ACCEPT, BUYER, START + 1, accept.clone())?;
    assert_title(&f, charter, GOD)?;
    check!(f.rows("roll", TRANSFERS).is_empty());
    check_eq!(f.balances()?, money_before);
    let (writes, install) = prepared.into_parts();
    let mut acknowledged = offered.clone();
    retain_writes(&mut acknowledged.disk, &writes)?;
    // The actual complete acknowledged images recover before the old RAM owners
    // receive installation; no synthetic native receipt is added to the image.
    let mut cold = f.reboot(&acknowledged)?;
    assert_title(&cold, charter, BUYER)?;
    check_eq!(current_generation(&cold)?, 2);
    check!(paid_holdings::observe(&mut cold, GOD, 1, None)?.is_err());
    let acquired = paid_holdings::own(&mut cold, BUYER, 1)?;
    check_eq!(acquired.charter(), original.charter());
    check_eq!(paid_holdings::own(&mut cold, BUYER, 2)?, paid);
    check!(
        immutable(&cold) == immutable_before,
        "title sale changed immutable mind history"
    );
    retain_writes(&mut f.disk, &writes)?;
    f.engine.install_composed_action(install)?;
    check_eq!(f.balances()?, cold.balances()?);
    assert_title(&f, charter, BUYER)?;
    let actual = one(f.rows("roll", TRANSFERS))?;
    let native = actual
        .get("economic")
        .cloned()
        .ok_or("actual native receipt")?;
    let native: mush_economy::registry_filing::charter_transfer::CharterTransferAuditSubject =
        serde_json::from_value(native)?;
    f.economy()?
        .charter_transfer_receipt_audit()
        .verify(&native)?;
    check_eq!(native.offer, token(START));
    check_eq!(native.commit, token(START + 1));
    check_eq!(native.price, Amount::from_minor(1));
    check_eq!(
        f.balances()?.first().ok_or("seller balance")?.0.minor(),
        money_before.first().ok_or("seller balance")?.0.minor() + 1
    );
    check_eq!(
        f.balances()?.get(2).ok_or("buyer balance")?.0.minor(),
        money_before.get(2).ok_or("buyer balance")?.0.minor() - 1
    );
    check!(crate::test_checks::refused(signed(
        &mut f.engine,
        ACCEPT,
        BUYER,
        START + 2,
        accept
    ))?);
    let after_sale = f.snapshot();
    let now = f.engine.world.now();
    f.engine.world.set_now(
        native
            .posted_at_utc
            .checked_sub(1)
            .ok_or("UTC lower bound")?,
    );
    let (_, next_payout, _) = consideration(&f.engine, charter, GOD)?;
    check!(
        crate::test_checks::refused(signed(
            &mut f.engine,
            OFFER,
            BUYER,
            START + 3,
            offer_params(charter, GOD, next_payout.id)
        ))?,
        "backward UTC published an offer before the actual acquisition receipt"
    );
    check!(f.disk == after_sale.disk);
    f.engine.world.set_now(now);
    f.transfer_charter(charter, GOD, START + 4)?;
    check_eq!(current_generation(&f)?, 3);
    assert_title(&f, charter, GOD)?;
    let mut cold = f.reboot(&f.snapshot())?;
    check_eq!(current_generation(&cold)?, 3);
    check_eq!(
        paid_holdings::own(&mut cold, GOD, 1)?.charter(),
        original.charter()
    );
    check_eq!(paid_holdings::own(&mut cold, BUYER, 2)?, paid);
    check!(immutable(&cold) == immutable_before);
    let roots_after = cold.roots()?;
    for (key, value) in roots_before {
        check_eq!(roots_after.get(&key), Some(&value));
    }
    for mode in 0..5 {
        let mut corrupt = cold.snapshot();
        if mode == 0 {
            corrupt.disk.retain(|_, v| {
                ![OFFERS, TRANSFERS, HEADS]
                    .iter()
                    .any(|f| v.pointer("/family/id") == Some(&json!(f)))
            });
        } else if mode == 1 {
            change_one_family(&mut corrupt, "roll", HEADS, |v| {
                *v.pointer_mut("/current/digest").ok_or("head digest")? =
                    json!(mush_core::DefinitionDigest::from_bytes([17; 32]));
                Ok(())
            })?;
        } else if mode == 2 || mode == 3 {
            // Two transfers retain two transfer and two offer rows; corrupt one
            // whose change is real (an offer not already naming GOD).
            let family = if mode == 2 { TRANSFERS } else { OFFERS };
            let (_, row) = corrupt
                .disk
                .iter_mut()
                .find(|((ns, _), v)| {
                    ns == "roll"
                        && v.pointer("/family/id") == Some(&json!(family))
                        && (mode == 2 || v.pointer("/value/buyer") != Some(&json!(GOD)))
                })
                .ok_or("actual corruption target")?;
            let v = row.get_mut("value").ok_or("typed value")?;
            if mode == 2 {
                *v.pointer_mut("/economic/posted_at_utc")
                    .ok_or("native UTC")? = json!(now - 1);
            } else {
                *v.get_mut("buyer").ok_or("named buyer")? = json!(GOD);
            }
        } else {
            let key = corrupt
                .disk
                .iter()
                .find(|(_, v)| v.pointer("/family/id") == Some(&json!(TRANSFERS)))
                .map(|(k, _)| k.clone())
                .ok_or("actual transfer")?;
            check!(corrupt.disk.remove(&key).is_some());
        }
        check!(
            crate::test_checks::refused(base.reboot(&corrupt))?,
            "cold charter ownership lost its paired actual cause"
        );
    }
    Ok(())
}
