#![cfg(test)]

//! 17g C2f (R2, the lead's option (a)): the closed owner-index admission,
//! row by row, on the protocols' own coordinator fixtures (real typed
//! transactions, installation and cold hydration; business values are inert
//! fixture data, never E4 money). Each `index_<family>_<protocol>_<action>_
//! is_required_and_exact` test proves, for one row of `owner_index::TABLE`:
//! (i) the transition with exactly the expected index mutation commits and
//! reboots; (ii) without it, it refuses (the exact-set message, or "… index
//! row required" where only the terminal branch would see the row); (iii) a
//! second index row, a wrong key, the wrong op or the row at an unlisted
//! action refuses: "… exact typed union differs" in the resale and pending
//! exact sets; in the entitlement, House and v2 terminals "… index row
//! differs" from their validators' own exact check (`owner_index::require`)
//! for a row of a family the table admits at that action, and "unreserved
//! terminal key" from the terminal branch for a row it does not admit there.
//! The index rows here are literal JSON, never `expected`'s.
use super::super::{charter_pending, entitlement, house, owner_index, paid};
use super::*;
use crate::ObjId;
use owner_index::tests::{
    consume_offer, delete_row, holding_row, index_offer, move_holding, put_holding, put_row,
    release_operation,
};
use owner_index::{ACTORS, HOLDINGS, PENDING_OFFERS, RESALE_OFFERS};

type R = Result<(), Box<dyn std::error::Error>>;
type Handles = BTreeMap<String, StateFamilyHandle<String, Value>>;
type Step = Result<PreparedTxn, Box<dyn std::error::Error>>;

const BUYER: u64 = 7;
const OTHER: u64 = 8;
const RESALE: &str = "resale exact typed union differs";
const PENDING: &str = "pending exact typed union differs";
const UNRESERVED: &str = "unreserved terminal key";
const ENTITLEMENT: &str = "entitlement index row differs";
const HOUSE: &str = "house index row differs";
const PURCHASE: &str = "purchase index row differs";

fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
/// The refusal's reason, when the step refused in the typed checks.
fn reason(step: Step) -> Option<&'static str> {
    match step {
        Ok(_) => None,
        Err(error) => match error.downcast_ref::<ActionStateError>() {
            Some(ActionStateError::TerminalCapacity(reason)) => Some(reason),
            _ => Some("other refusal"),
        },
    }
}
fn boxed(step: Result<PreparedTxn, ActionStateError>) -> Step {
    step.map_err(Into::into)
}
fn refuses(step: Step, expected: &str, case: &str) {
    assert_eq!(reason(step), Some(expected), "{case}");
}
fn actors_row(actor: u64, operations: &[CommitToken]) -> Value {
    json!({"format":1,"actor":actor,"quotes":[],"operations":operations})
}
fn resale_key(buyer: u64, licence: u64) -> String {
    format!("{buyer:020}/licence/{licence:016x}")
}
fn resale_head(buyer: u64) -> Value {
    json!({"format":1,"buyer":buyer,"licence":"1"})
}
fn pending_key(buyer: u64) -> String {
    format!("{buyer:020}/charter/{:016x}", 100)
}
fn pending_head(buyer: u64) -> Value {
    json!({"format":1,"buyer":buyer,"charter":"100"})
}
/// Boot a store over `seed`'s registry from `disk`.
fn boots(registry: &StateFamilyRegistry, disk: &BTreeMap<(SmolStr, SmolStr), Value>) -> R {
    let mut store = ActionStateStore::new(registry.clone());
    store.hydrate(disk.iter().map(|((ns, key), value)| {
        ActionStateBootRecord::new(ns.clone(), key.clone(), value.clone())
    }))?;
    Ok(())
}
/// The boot of `disk` refuses with exactly `expected` (the owner-index
/// audit's reason), never an earlier or unrelated refusal.
fn boot_refuses(
    registry: &StateFamilyRegistry,
    disk: &BTreeMap<(SmolStr, SmolStr), Value>,
    expected: &str,
    case: &str,
) {
    let reason =
        boots(registry, disk)
            .err()
            .map(|error| match error.downcast_ref::<ActionStateError>() {
                Some(ActionStateError::TerminalCapacity(reason)) => *reason,
                _ => "other refusal",
            });
    assert_eq!(reason, Some(expected), "{case}");
}
/// The disk path of `family`/`encoded`.
fn path(
    registry: &StateFamilyRegistry,
    owner: &str,
    family: &str,
    encoded: &str,
) -> Result<(SmolStr, SmolStr), Box<dyn std::error::Error>> {
    Ok((
        owner.into(),
        Key {
            owner: OwnerId::new(owner)?,
            family: StateFamilyId::new(family)?,
            encoded: encoded.into(),
        }
        .resolve(registry)?
        .persistence_key(),
    ))
}
/// `disk` with the row at `family`/`encoded` replaced by (or added as) a
/// document of `value`, its envelope copied from `template`.
fn with_row(
    registry: &StateFamilyRegistry,
    disk: &BTreeMap<(SmolStr, SmolStr), Value>,
    template: &(SmolStr, SmolStr),
    (owner, family, encoded): (&str, &str, &str),
    value: Value,
) -> Result<BTreeMap<(SmolStr, SmolStr), Value>, Box<dyn std::error::Error>> {
    let mut document = disk.get(template).ok_or("template row")?.clone();
    document["key"] = json!(encoded);
    document["value"] = value;
    let mut out = disk.clone();
    out.insert(path(registry, owner, family, encoded)?, document);
    Ok(out)
}

// ------------------------------------------------- the entitlement terminal

/// Actors, entitlement Grant (1): the operation leaves its own buyer's row,
/// a Delete when the row would be empty (it held only this operation).
#[test]
fn index_market_mind_entitlement_actors_entitlement_grant_is_required_and_exact() -> R {
    let op = entitlement::tests::original_operation();
    let buyer = ObjId(BUYER as u32);
    let mut f = entitlement::tests::funded_original(false)?;
    let holding = |t: &mut ActionTxn<'_>, h: &Handles| put_holding(t, h, BUYER, 1);
    // (ii) The Grant without its actors row: only the terminal branch would
    // see it, so the validator requires it.
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&holding)),
        "entitlement index row required",
        "grant without the buyer's actors release",
    );
    // (iii) Another buyer's row instead of the buyer's: the buyer's is absent.
    let foreign = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        put_row(t, h, ACTORS, "8", actors_row(OTHER, &[op]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&foreign)),
        "entitlement index row required",
        "another buyer's row instead of the buyer's",
    );
    // A second actors row beside the exact release.
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        release_operation(t, h, BUYER, op)?;
        put_row(t, h, ACTORS, "8", actors_row(OTHER, &[op]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&second)),
        ENTITLEMENT,
        "a second actors row",
    );
    // The wrong op: a Put of the emptied row where the content implies Delete.
    let emptied = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        put_row(t, h, ACTORS, "7", actors_row(BUYER, &[]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&emptied)),
        ENTITLEMENT,
        "a Put of an empty row",
    );
    // The wrong content: the row still holding the operation.
    let kept = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        put_row(t, h, ACTORS, "7", actors_row(BUYER, &[op]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&kept)),
        ENTITLEMENT,
        "the row still holding the operation",
    );
    // (i) Exactly the expected Delete commits and reboots.
    let p = entitlement::tests::original_terminal(&mut f, true, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    // The Put branch: a second open purchase by the same buyer stays in the
    // row, so the content implies a Put of exactly the row minus this one.
    let derived = entitlement::tests::derived_operation();
    let mut f = entitlement::tests::funded_original_and_derived()?;
    let deleted = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        delete_row(t, h, ACTORS, "7")
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&deleted)),
        ENTITLEMENT,
        "a Delete of a row that still holds an open purchase",
    );
    let both = |t: &mut ActionTxn<'_>, h: &Handles| {
        holding(t, h)?;
        let mut operations = vec![op, derived];
        operations.sort();
        put_row(t, h, ACTORS, "7", actors_row(BUYER, &operations))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&both)),
        ENTITLEMENT,
        "a Put that keeps the granted operation",
    );
    let p = entitlement::tests::original_terminal(&mut f, true, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// Actors, entitlement Refund (2): the same release, and nothing else of
/// the two families (the Refund grants no licence, so no holdings row).
#[test]
fn index_market_mind_entitlement_actors_entitlement_refund_is_required_and_exact() -> R {
    let op = entitlement::tests::original_operation();
    let buyer = ObjId(BUYER as u32);
    let mut f = entitlement::tests::funded_original(false)?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        entitlement::tests::original_terminal(&mut f, false, buyer, Some(&nothing)),
        "entitlement index row required",
        "refund without the actors release",
    );
    let with_holding = |t: &mut ActionTxn<'_>, h: &Handles| {
        release_operation(t, h, BUYER, op)?;
        put_holding(t, h, BUYER, 1)
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, false, buyer, Some(&with_holding)),
        UNRESERVED,
        "a holdings row at the Refund (an unlisted action for the family)",
    );
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        release_operation(t, h, BUYER, op)?;
        put_row(t, h, ACTORS, "8", actors_row(OTHER, &[token(40)]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, false, buyer, Some(&second)),
        ENTITLEMENT,
        "a second actors row",
    );
    let emptied =
        |t: &mut ActionTxn<'_>, h: &Handles| put_row(t, h, ACTORS, "7", actors_row(BUYER, &[]));
    refuses(
        entitlement::tests::original_terminal(&mut f, false, buyer, Some(&emptied)),
        ENTITLEMENT,
        "a Put of an empty row",
    );
    let p = entitlement::tests::original_terminal(&mut f, false, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// Holdings, entitlement Grant (1): the holder's row from the pointer
/// post-image (holder 7, licence 1), nothing else.
#[test]
fn index_roll_mind_paid_holdings_entitlement_grant_is_required_and_exact() -> R {
    let op = entitlement::tests::original_operation();
    let buyer = ObjId(BUYER as u32);
    let mut f = entitlement::tests::funded_original(false)?;
    let release = |t: &mut ActionTxn<'_>, h: &Handles| release_operation(t, h, BUYER, op);
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&release)),
        "entitlement index row required",
        "grant without the holder's holdings row",
    );
    let foreign = |t: &mut ActionTxn<'_>, h: &Handles| {
        release(t, h)?;
        put_holding(t, h, OTHER, 1)
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&foreign)),
        "entitlement index row required",
        "another holder's row instead of the holder's",
    );
    for (case, extra) in [
        ("a second holder's row", holding_row(OTHER, 1)),
        ("a wrong licence", holding_row(BUYER, 2)),
    ] {
        let write = |t: &mut ActionTxn<'_>, h: &Handles| {
            release(t, h)?;
            put_holding(t, h, BUYER, 1)?;
            put_row(t, h, HOLDINGS, &extra.0, extra.1.clone())
        };
        refuses(
            entitlement::tests::original_terminal(&mut f, true, buyer, Some(&write)),
            ENTITLEMENT,
            case,
        );
    }
    let crossed = |t: &mut ActionTxn<'_>, h: &Handles| {
        release(t, h)?;
        put_row(
            t,
            h,
            HOLDINGS,
            &holding_row(BUYER, 1).0,
            holding_row(OTHER, 1).1,
        )
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&crossed)),
        ENTITLEMENT,
        "the holder's key carrying another holder's row",
    );
    let p = entitlement::tests::original_terminal(&mut f, true, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// Holdings, House Grant (1): the holder's row from the pointer post-image.
#[test]
fn index_roll_mind_paid_holdings_house_grant_is_required_and_exact() -> R {
    let mut f = house::tests::funded_house()?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        house::tests::house_terminal(&mut f, true, Some(&nothing)),
        "house index row required",
        "House grant without the holdings row",
    );
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        put_holding(t, h, BUYER, 1)?;
        put_holding(t, h, OTHER, 1)
    };
    refuses(
        house::tests::house_terminal(&mut f, true, Some(&second)),
        HOUSE,
        "a second holder's row",
    );
    let crossed = |t: &mut ActionTxn<'_>, h: &Handles| {
        put_row(
            t,
            h,
            HOLDINGS,
            &holding_row(BUYER, 1).0,
            holding_row(BUYER, 2).1,
        )
    };
    refuses(
        house::tests::house_terminal(&mut f, true, Some(&crossed)),
        HOUSE,
        "the holder's key carrying a wrong licence",
    );
    let at_refund = |t: &mut ActionTxn<'_>, h: &Handles| put_holding(t, h, BUYER, 1);
    refuses(
        house::tests::house_terminal(&mut f, false, Some(&at_refund)),
        UNRESERVED,
        "a holdings row at the House Refund (an unlisted action)",
    );
    let p = house::tests::house_terminal(&mut f, true, None)?;
    house::tests::house_commit(&mut f, p)?;
    // The row is action 1 of every House kind: the two-seat HouseV2 pin and
    // core's Follow v3 grant require and admit exactly the same row.
    for follow in [false, true] {
        let mut f = house::tests::funded_two_seat(follow)?;
        refuses(
            house::tests::two_seat_terminal(&mut f, follow, true, Some(&nothing)),
            "house index row required",
            "two-seat grant without the holdings row",
        );
        refuses(
            house::tests::two_seat_terminal(&mut f, follow, true, Some(&second)),
            HOUSE,
            "two-seat grant with a second holder's row",
        );
        refuses(
            house::tests::two_seat_terminal(&mut f, follow, false, Some(&at_refund)),
            UNRESERVED,
            "a holdings row at the two-seat Refund",
        );
        let p = house::tests::two_seat_terminal(&mut f, follow, true, None)?;
        house::tests::house_commit(&mut f, p)?;
    }
    Ok(())
}

/// Holdings, v2 purchase Grant (`settle_original_mind`, 1).
#[test]
fn index_roll_mind_paid_holdings_purchase_grant_is_required_and_exact() -> R {
    let mut f = paid::tests::funded_paid()?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        boxed(paid::tests::paid_terminal(&mut f, true, Some(&nothing))),
        "purchase index row required",
        "v2 grant without the holdings row",
    );
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        put_holding(t, h, BUYER, 1)?;
        put_holding(t, h, OTHER, 1)
    };
    refuses(
        boxed(paid::tests::paid_terminal(&mut f, true, Some(&second))),
        PURCHASE,
        "a second holder's row",
    );
    let at_refund = |t: &mut ActionTxn<'_>, h: &Handles| put_holding(t, h, BUYER, 1);
    refuses(
        boxed(paid::tests::paid_terminal(&mut f, false, Some(&at_refund))),
        UNRESERVED,
        "a holdings row at the v2 Refund (an unlisted action)",
    );
    let p = paid::tests::paid_terminal(&mut f, true, None)?;
    paid::tests::paid_commit(&mut f, p)?;
    Ok(())
}

// ------------------------------------------------------------ paid resale

/// Holdings, resale immediate Accept (1): the buyer's row Put and the
/// seller's Deleted (a non-terminal step: the exact typed set checks it).
#[test]
fn index_roll_mind_paid_holdings_paid_resale_accept_is_required_and_exact() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let consume = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume_offer(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), token(10))
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&consume),
        )),
        RESALE,
        "Accept without the holdings move",
    );
    let buyer_only = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume(t, h)?;
        put_holding(t, h, OTHER, 1)
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&buyer_only),
        )),
        RESALE,
        "the buyer's row without the seller's Delete",
    );
    let kept = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume(t, h)?;
        put_holding(t, h, OTHER, 1)?;
        put_row(
            t,
            h,
            HOLDINGS,
            &holding_row(BUYER, 1).0,
            holding_row(BUYER, 1).1,
        )
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&kept),
        )),
        RESALE,
        "the seller's row Put (the wrong op)",
    );
    let third = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume(t, h)?;
        move_holding(t, h, BUYER, OTHER, 1)?;
        put_holding(t, h, 9, 1)
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&third),
        )),
        RESALE,
        "a third holder's row",
    );
    let p = resale::resale_accept(&mut f, &offer, token(11), false, None)?;
    resale::resale_commit(&mut f, p)?;
    Ok(())
}

/// Holdings, resale service Settle (3): the move the escrowed Fund deferred,
/// admitted both by the terminal branch and by the exact typed set.
#[test]
fn index_roll_mind_paid_holdings_paid_resale_settle_is_required_and_exact() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let p = resale::resale_accept(&mut f, &offer, token(11), true, None)?;
    resale::resale_commit(&mut f, p)?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        boxed(resale::resale_settle(
            &mut f,
            &offer,
            token(11),
            token(12),
            Some(&nothing),
        )),
        RESALE,
        "Settle without the holdings move",
    );
    let seller_only =
        |t: &mut ActionTxn<'_>, h: &Handles| delete_row(t, h, HOLDINGS, &holding_row(BUYER, 1).0);
    refuses(
        boxed(resale::resale_settle(
            &mut f,
            &offer,
            token(11),
            token(12),
            Some(&seller_only),
        )),
        RESALE,
        "the seller's Delete without the buyer's row",
    );
    let wrong_buyer = |t: &mut ActionTxn<'_>, h: &Handles| move_holding(t, h, BUYER, 9, 1);
    refuses(
        boxed(resale::resale_settle(
            &mut f,
            &offer,
            token(11),
            token(12),
            Some(&wrong_buyer),
        )),
        RESALE,
        "the row moved to someone other than the pointer's new holder",
    );
    let p = resale::resale_settle(&mut f, &offer, token(11), token(12), None)?;
    resale::resale_commit(&mut f, p)?;
    Ok(())
}

/// Resale buyer offers, Offer (0): the named buyer's row with its retained
/// tokens plus the new one (pruning is the writer's, against the clock).
#[test]
fn index_market_mind_paid_resale_buyer_offers_paid_resale_offer_is_required_and_exact() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, Some(&nothing));
    refuses(boxed(p), RESALE, "Offer without the buyer's offer row");
    for (case, key, head) in [
        ("another buyer's row", resale_key(9, 1), resale_head(9)),
        (
            "a wrong licence",
            resale_key(OTHER, 2),
            json!({"format":1,"buyer":OTHER,"licence":"2"}),
        ),
    ] {
        let write = |t: &mut ActionTxn<'_>, h: &Handles| {
            index_offer(t, h, RESALE_OFFERS, &key, head.clone(), token(10))
        };
        let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, Some(&write));
        refuses(boxed(p), RESALE, case);
    }
    let foreign = |t: &mut ActionTxn<'_>, h: &Handles| {
        let mut row = resale_head(OTHER);
        let mut offers = vec![token(10).to_hex(), token(99).to_hex()];
        offers.sort();
        row["offers"] = json!(offers);
        put_row(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), row)
    };
    let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, Some(&foreign));
    refuses(boxed(p), RESALE, "a token the row never held");
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        index_offer(
            t,
            h,
            RESALE_OFFERS,
            &resale_key(OTHER, 1),
            resale_head(OTHER),
            token(10),
        )?;
        index_offer(
            t,
            h,
            RESALE_OFFERS,
            &resale_key(9, 1),
            resale_head(9),
            token(10),
        )
    };
    let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, Some(&second));
    refuses(boxed(p), RESALE, "a second offer row");
    let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    // The writer may prune retained tokens (an expired offer) but adds only
    // its own: a row of the new token alone commits.
    let pruned = |t: &mut ActionTxn<'_>, h: &Handles| {
        let mut row = resale_head(OTHER);
        row["offers"] = json!([token(13)]);
        put_row(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), row)
    };
    let (_, p) = resale::resale_offer(&mut f, token(13), OTHER as u32, Some(&pruned));
    resale::resale_commit(&mut f, p?)?;
    Ok(())
}

/// Resale buyer offers, immediate Accept (1): the accepted offer leaves its
/// buyer's row (a Delete when it was the last token, a Put otherwise).
#[test]
fn index_market_mind_paid_resale_buyer_offers_paid_resale_accept_is_required_and_exact() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let (_, p) = resale::resale_offer(&mut f, token(13), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let moved = |t: &mut ActionTxn<'_>, h: &Handles| move_holding(t, h, BUYER, OTHER, 1);
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&moved),
        )),
        RESALE,
        "Accept without the offer leaving the buyer's row",
    );
    let emptied = |t: &mut ActionTxn<'_>, h: &Handles| {
        moved(t, h)?;
        delete_row(t, h, RESALE_OFFERS, &resale_key(OTHER, 1))
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&emptied),
        )),
        RESALE,
        "a Delete of a row that still holds another token",
    );
    let wrong = |t: &mut ActionTxn<'_>, h: &Handles| {
        moved(t, h)?;
        consume_offer(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), token(13))
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            false,
            Some(&wrong),
        )),
        RESALE,
        "the other token taken out instead",
    );
    // (i) the Put branch: token 13 stays.
    let p = resale::resale_accept(&mut f, &offer, token(11), false, None)?;
    resale::resale_commit(&mut f, p)?;
    Ok(())
}

/// Resale buyer offers, escrowed Fund (2): as the Accept, and a holdings
/// move there (an unlisted action for that family) refuses.
#[test]
fn index_market_mind_paid_resale_buyer_offers_paid_resale_fund_is_required_and_exact() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            true,
            Some(&nothing),
        )),
        RESALE,
        "Fund without the offer leaving the buyer's row",
    );
    let consume = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume_offer(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), token(10))
    };
    let kept = |t: &mut ActionTxn<'_>, h: &Handles| {
        let mut row = resale_head(OTHER);
        row["offers"] = json!([token(10)]);
        put_row(t, h, RESALE_OFFERS, &resale_key(OTHER, 1), row)
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            true,
            Some(&kept),
        )),
        RESALE,
        "a Put of the row still holding the token (the wrong op)",
    );
    let with_move = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume(t, h)?;
        move_holding(t, h, BUYER, OTHER, 1)
    };
    refuses(
        boxed(resale::resale_accept(
            &mut f,
            &offer,
            token(11),
            true,
            Some(&with_move),
        )),
        RESALE,
        "a holdings move at the Fund (an unlisted action)",
    );
    let p = resale::resale_accept(&mut f, &offer, token(11), true, None)?;
    resale::resale_commit(&mut f, p)?;
    Ok(())
}

// --------------------------------------------------------- pending charter

/// Pending buyer offers, Offer (0): the named buyer's row for the charter.
#[test]
fn index_roll_mind_charter_pending_buyer_offers_charter_pending_offer_is_required_and_exact() -> R {
    let mut f = charter_pending::tests::pending_rig()?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        charter_pending::tests::pending_offer(&mut f, token(4), Some(&nothing)),
        PENDING,
        "Offer without the buyer's offer row",
    );
    for (case, key, head) in [
        (
            "another buyer's row",
            pending_key(OTHER),
            pending_head(OTHER),
        ),
        (
            "a wrong charter",
            format!("{BUYER:020}/charter/{:016x}", 101),
            json!({"format":1,"buyer":BUYER,"charter":"101"}),
        ),
    ] {
        let write = |t: &mut ActionTxn<'_>, h: &Handles| {
            index_offer(t, h, PENDING_OFFERS, &key, head.clone(), token(4))
        };
        refuses(
            charter_pending::tests::pending_offer(&mut f, token(4), Some(&write)),
            PENDING,
            case,
        );
    }
    let second = |t: &mut ActionTxn<'_>, h: &Handles| {
        index_offer(
            t,
            h,
            PENDING_OFFERS,
            &pending_key(BUYER),
            pending_head(BUYER),
            token(4),
        )?;
        index_offer(
            t,
            h,
            PENDING_OFFERS,
            &pending_key(OTHER),
            pending_head(OTHER),
            token(4),
        )
    };
    refuses(
        charter_pending::tests::pending_offer(&mut f, token(4), Some(&second)),
        PENDING,
        "a second offer row",
    );
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    Ok(())
}

/// Pending buyer offers, Fund (1): the consumed offer leaves its buyer's row.
#[test]
fn index_roll_mind_charter_pending_buyer_offers_charter_pending_fund_is_required_and_exact() -> R {
    let mut f = charter_pending::tests::pending_rig()?;
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let nothing = |_: &mut ActionTxn<'_>, _: &Handles| Ok(());
    refuses(
        charter_pending::tests::pending_fund(&mut f, token(4), token(5), Some(&nothing)),
        PENDING,
        "Fund without the offer leaving the buyer's row",
    );
    let kept = |t: &mut ActionTxn<'_>, h: &Handles| {
        let mut row = pending_head(BUYER);
        row["offers"] = json!([token(4)]);
        put_row(t, h, PENDING_OFFERS, &pending_key(BUYER), row)
    };
    refuses(
        charter_pending::tests::pending_fund(&mut f, token(4), token(5), Some(&kept)),
        PENDING,
        "a Put of the row still holding the token (the wrong op)",
    );
    let other = |t: &mut ActionTxn<'_>, h: &Handles| {
        consume_offer(t, h, PENDING_OFFERS, &pending_key(BUYER), token(4))?;
        index_offer(
            t,
            h,
            PENDING_OFFERS,
            &pending_key(OTHER),
            pending_head(OTHER),
            token(4),
        )
    };
    refuses(
        charter_pending::tests::pending_fund(&mut f, token(4), token(5), Some(&other)),
        PENDING,
        "a second offer row",
    );
    let p = charter_pending::tests::pending_fund(&mut f, token(4), token(5), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    Ok(())
}

// ------------------------------------------------------- cross-cutting rules

/// GOD may refund a funded purchase; the operation still leaves its own
/// buyer's actors row (`quote.buyer`), never the refunding actor's.
#[test]
fn a_god_refund_edits_the_buyers_actors_row_not_gods() -> R {
    let op = entitlement::tests::original_operation();
    let god = crate::GOD;
    let god_key = u64::from(god.0).to_string();
    let mut f = entitlement::tests::funded_original(false)?;
    let gods = |t: &mut ActionTxn<'_>, h: &Handles| {
        put_row(t, h, ACTORS, &god_key, actors_row(u64::from(god.0), &[op]))
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, false, god, Some(&gods)),
        "entitlement index row required",
        "GOD's own row edited instead of the buyer's",
    );
    let both = |t: &mut ActionTxn<'_>, h: &Handles| {
        release_operation(t, h, BUYER, op)?;
        gods(t, h)
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, false, god, Some(&both)),
        ENTITLEMENT,
        "GOD's row beside the buyer's release",
    );
    let p = entitlement::tests::original_terminal(&mut f, false, god, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// The holdings family joins no protocol's own-family filter: with paid
/// resale declared on the same store, an entitlement Grant that puts the
/// holder's holdings row commits (the resale validator, which demands a
/// resale action for any key of its own families, ignores it), while a
/// resale family's row in the same Grant still refuses there, with the
/// resale validator's own reason (an entitlement action is not a resale
/// action), before the terminal branch would see the row.
#[test]
fn an_entitlement_grant_with_a_holdings_row_passes_the_resale_validator() -> R {
    let op = entitlement::tests::original_operation();
    let buyer = ObjId(BUYER as u32);
    let mut f = entitlement::tests::funded_original(true)?;
    let resale_row = |t: &mut ActionTxn<'_>, h: &Handles| {
        release_operation(t, h, BUYER, op)?;
        put_holding(t, h, BUYER, 1)?;
        put_row(
            t,
            h,
            "roll_mind_paid_resale_heads",
            "licence/0000000000000001",
            json!({"fixture":"foreign"}),
        )
    };
    refuses(
        entitlement::tests::original_terminal(&mut f, true, buyer, Some(&resale_row)),
        "resale actual action differs",
        "a resale family row in a grant still meets the resale validator",
    );
    let p = entitlement::tests::original_terminal(&mut f, true, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// A Grant's holdings put is unreserved capacity, charged by the ordinary
/// typed accounting: at a store whose ordinary aggregate has room for the
/// Grant's reserved images but not its holdings row, the Grant refuses
/// (`LiveBytesExceeded`, by exactly the row's bytes) and moves nothing; the
/// Refund of the same purchase at the same aggregate, whose only index write
/// releases the buyer's actors row, commits.
#[test]
fn a_full_store_refuses_a_grant_put_and_never_a_refund() -> R {
    let buyer = ObjId(BUYER as u32);
    let mut measured = entitlement::tests::funded_original(false)?;
    let before = *entitlement::tests::live_bytes(&mut measured);
    let p = entitlement::tests::original_terminal(&mut measured, true, buyer, None)?;
    entitlement::tests::commit(&mut measured, p)?;
    let grant = entitlement::tests::live_bytes(&mut measured)
        .checked_sub(before)
        .ok_or("the grant adds typed bytes")?;
    let holding = entitlement::tests::row_bytes(&measured, HOLDINGS, &holding_row(BUYER, 1).0)?;
    assert!(holding > 0 && holding < grant);
    let mut f = entitlement::tests::funded_original(false)?;
    *entitlement::tests::live_bytes(&mut f) = MAX_ACTION_STATE_BOOT_BYTES - (grant - holding);
    let refused = entitlement::tests::original_terminal(&mut f, true, buyer, None);
    assert!(
        matches!(
            refused.as_ref().err().and_then(|e| e.downcast_ref::<ActionStateError>()),
            Some(ActionStateError::LiveBytesExceeded { attempted, .. })
                if *attempted == MAX_ACTION_STATE_BOOT_BYTES + holding
        ),
        "the grant's unreserved holdings row is what does not fit"
    );
    let p = entitlement::tests::original_terminal(&mut f, false, buyer, None)?;
    entitlement::tests::commit(&mut f, p)?;
    Ok(())
}

/// The clock-free boot audit (`owner_index::audit_boot`), one case per
/// family: a store whose index disagrees with its heads refuses to boot.
#[test]
fn boot_refuses_an_index_that_disagrees_with_its_heads() -> R {
    // Actors: the funded purchase's row omitted, or naming another buyer.
    let f = entitlement::tests::funded_original(false)?;
    let (store, _, disk) = entitlement::tests::into_seed(f);
    let registry = &store.registry;
    boots(registry, &disk)?;
    let actors = path(registry, "market", ACTORS, "7")?;
    let mut omitted = disk.clone();
    omitted.remove(&actors).ok_or("actors row")?;
    boot_refuses(
        registry,
        &omitted,
        "owner index omits a funded purchase",
        "actors: open purchase not indexed",
    );
    let op = entitlement::tests::original_operation();
    let mut moved = with_row(
        registry,
        &disk,
        &actors,
        ("market", ACTORS, "8"),
        actors_row(OTHER, &[op]),
    )?;
    moved.remove(&actors);
    boot_refuses(
        registry,
        &moved,
        "owner index operation is not the buyer's open purchase",
        "actors: indexed under another buyer",
    );

    // Holdings: the granted licence's row omitted, under another holder, or
    // an orphan beside it.
    let mut f = entitlement::tests::funded_original(false)?;
    let p = entitlement::tests::original_terminal(&mut f, true, ObjId(BUYER as u32), None)?;
    entitlement::tests::commit(&mut f, p)?;
    let (store, _, disk) = entitlement::tests::into_seed(f);
    let registry = &store.registry;
    boots(registry, &disk)?;
    let (key, _) = holding_row(BUYER, 1);
    let held = path(registry, "roll", HOLDINGS, &key)?;
    let mut omitted = disk.clone();
    omitted.remove(&held).ok_or("holdings row")?;
    boot_refuses(
        registry,
        &omitted,
        "owner index omits a held licence",
        "holdings: held licence not indexed",
    );
    let (other, row) = holding_row(OTHER, 1);
    let mut crossed = with_row(registry, &disk, &held, ("roll", HOLDINGS, &other), row)?;
    crossed.remove(&held);
    boot_refuses(
        registry,
        &crossed,
        "owner index holding has no current pointer",
        "holdings: under another holder",
    );
    let (orphan, row) = holding_row(BUYER, 2);
    let orphaned = with_row(registry, &disk, &held, ("roll", HOLDINGS, &orphan), row)?;
    boot_refuses(
        registry,
        &orphaned,
        "owner index holding has no current pointer",
        "holdings: orphan row",
    );

    // Resale buyer offers: the token an escrowed Fund consumed, put back.
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let (_, p) = resale::resale_offer(&mut f, token(13), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let p = resale::resale_accept(&mut f, &offer, token(11), true, None)?;
    resale::resale_commit(&mut f, p)?;
    let (store, _, disk) = resale::resale_seed(f);
    let registry = &store.registry;
    boots(registry, &disk)?;
    let offers = path(registry, "market", RESALE_OFFERS, &resale_key(OTHER, 1))?;
    let mut row = resale_head(OTHER);
    let mut tokens = vec![token(10).to_hex(), token(13).to_hex()];
    tokens.sort();
    row["offers"] = json!(tokens);
    let consumed = with_row(
        registry,
        &disk,
        &offers,
        ("market", RESALE_OFFERS, &resale_key(OTHER, 1)),
        row,
    )?;
    boot_refuses(
        registry,
        &consumed,
        "owner index offer differs",
        "resale offers: an accepted offer indexed",
    );
    let mut row = resale_head(9);
    row["offers"] = json!([token(13)]);
    let mut foreign = with_row(
        registry,
        &disk,
        &offers,
        ("market", RESALE_OFFERS, &resale_key(9, 1)),
        row,
    )?;
    foreign.remove(&offers);
    boot_refuses(
        registry,
        &foreign,
        "owner index offer differs",
        "resale offers: under a buyer it does not name",
    );

    // Pending buyer offers: the offer row under another buyer, and the
    // token a Fund consumed put back.
    let mut f = charter_pending::tests::pending_rig()?;
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let (store, _, disk) = charter_pending::tests::pending_seed(f);
    let registry = &store.registry;
    boots(registry, &disk)?;
    let pending = path(registry, "roll", PENDING_OFFERS, &pending_key(BUYER))?;
    let mut row = pending_head(OTHER);
    row["offers"] = json!([token(4)]);
    let mut foreign = with_row(
        registry,
        &disk,
        &pending,
        ("roll", PENDING_OFFERS, &pending_key(OTHER)),
        row,
    )?;
    foreign.remove(&pending);
    boot_refuses(
        registry,
        &foreign,
        "owner index offer differs",
        "pending offers: under a buyer it does not name",
    );
    let mut f = charter_pending::tests::pending_rig()?;
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let template = {
        let (store, _, disk) = charter_pending::tests::pending_seed(f);
        (store, disk)
    };
    let mut f = charter_pending::tests::pending_rig()?;
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let p = charter_pending::tests::pending_fund(&mut f, token(4), token(5), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let (store, _, disk) = charter_pending::tests::pending_seed(f);
    let registry = &store.registry;
    boots(registry, &disk)?;
    let mut put_back = disk.clone();
    put_back.insert(pending.clone(), template.1[&pending].clone());
    boot_refuses(
        registry,
        &put_back,
        "owner index offer differs",
        "pending offers: a funded offer indexed",
    );
    Ok(())
}

// ---------------------------------------- the table's deliberate exclusions

/// A named index write, as a plain function.
type IndexWrite = fn(&mut ActionTxn<'_>, &Handles) -> Result<(), ActionStateError>;
/// The seller's holdings row moved to the resale buyer (`OTHER`).
fn moved(t: &mut ActionTxn<'_>, h: &Handles) -> Result<(), ActionStateError> {
    move_holding(t, h, BUYER, OTHER, 1)
}
/// The resale buyer's offer row holding offer 10.
fn offered(t: &mut ActionTxn<'_>, h: &Handles) -> Result<(), ActionStateError> {
    index_offer(
        t,
        h,
        RESALE_OFFERS,
        &resale_key(OTHER, 1),
        resale_head(OTHER),
        token(10),
    )
}

/// Nothing outside `owner_index::TABLE` is admitted (the brief's "not
/// admitted, deliberately"), resale terminals: a holdings move or an offer
/// row at the resale Refund (4), and an offer row beside the admitted
/// holdings move at the Settle (3), each refuse the exact set; the Refund
/// without them commits.
#[test]
fn the_resale_terminals_admit_no_offer_row_and_the_refund_no_holdings() -> R {
    use paid::tests::resale;
    // The resale Refund of an escrowed Fund: no holder change, no offer row.
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let p = resale::resale_accept(&mut f, &offer, token(11), true, None)?;
    resale::resale_commit(&mut f, p)?;
    for (case, write) in [
        ("a holdings move at the resale Refund", moved as IndexWrite),
        ("an offer row at the resale Refund", offered),
    ] {
        refuses(
            boxed(resale::resale_refund(
                &mut f,
                &offer,
                token(11),
                token(12),
                Some(&write),
            )),
            RESALE,
            case,
        );
    }
    let p = resale::resale_refund(&mut f, &offer, token(11), token(12), None)?;
    resale::resale_commit(&mut f, p)?;
    // The service Settle admits the holdings move and nothing of the offers.
    let mut f = resale::resale_rig()?;
    let (offer, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let p = resale::resale_accept(&mut f, &offer, token(11), true, None)?;
    resale::resale_commit(&mut f, p)?;
    let settle_and_offer = |t: &mut ActionTxn<'_>, h: &Handles| {
        moved(t, h)?;
        offered(t, h)
    };
    refuses(
        boxed(resale::resale_settle(
            &mut f,
            &offer,
            token(11),
            token(12),
            Some(&settle_and_offer),
        )),
        RESALE,
        "an offer row at the resale Settle",
    );
    Ok(())
}

/// Not admitted, the Registry revocation (5): it changes no holder, so a
/// holdings move or a buyer's holdings row refuses the exact set; the
/// revocation without them commits.
#[test]
fn the_registry_revocation_admits_no_holdings_row() -> R {
    use paid::tests::resale;
    let (_, p) = resale::resale_revocation(Some(&moved))?;
    refuses(boxed(p), RESALE, "a holdings move at the revocation");
    let put = |t: &mut ActionTxn<'_>, h: &Handles| put_holding(t, h, OTHER, 1);
    let (_, p) = resale::resale_revocation(Some(&put))?;
    refuses(boxed(p), RESALE, "a buyer's holdings row at the revocation");
    let (mut f, p) = resale::resale_revocation(None)?;
    resale::resale_commit(&mut f, p?)?;
    Ok(())
}

/// Not admitted, the pending Settle (2) and Refund (3): the token left the
/// buyer's row at the Fund, so an offer row refuses the exact set; each
/// terminal without it commits.
#[test]
fn the_pending_terminals_admit_no_offer_row() -> R {
    let pending_row = |t: &mut ActionTxn<'_>, h: &Handles| {
        index_offer(
            t,
            h,
            PENDING_OFFERS,
            &pending_key(BUYER),
            pending_head(BUYER),
            token(4),
        )
    };
    for settle in [true, false] {
        let mut f = charter_pending::tests::pending_rig()?;
        let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
        charter_pending::tests::pending_commit(&mut f, p)?;
        let p = charter_pending::tests::pending_fund(&mut f, token(4), token(5), None)?;
        charter_pending::tests::pending_commit(&mut f, p)?;
        refuses(
            charter_pending::tests::pending_terminal(
                &mut f,
                token(5),
                token(6),
                settle,
                Some(&pending_row),
            ),
            PENDING,
            if settle {
                "an offer row at the pending Settle"
            } else {
                "an offer row at the pending Refund"
            },
        );
        let p = charter_pending::tests::pending_terminal(&mut f, token(5), token(6), settle, None)?;
        charter_pending::tests::pending_commit(&mut f, p)?;
    }
    Ok(())
}

/// Not admitted, a renewal Grant: it keeps the holder, so its holdings row
/// refuses (the renewal's own exact post-image set refuses it before the
/// terminal branch, which has no row for the renewal protocol either); the
/// Grant without it prepares.
#[test]
fn a_renewal_grant_admits_no_holdings_row() -> R {
    let held = |t: &mut ActionTxn<'_>, h: &Handles| {
        let (k, row) = holding_row(BUYER, 1);
        put_row(t, h, HOLDINGS, &k, row)
    };
    refuses(
        super::super::renewal::tests::renewal_grant(Some(&held)),
        "renewal requires exact closed postimages",
        "a holdings row at a renewal Grant",
    );
    drop(super::super::renewal::tests::renewal_grant(None)?);
    Ok(())
}

/// The offer rows' op is decided by their content at every step: a Delete
/// of the buyer's row at an Offer (whose content implies a Put holding the
/// new token) refuses in both exact sets, and the pending Fund that leaves
/// another live offer in the row is the Put branch (a Delete refuses).
#[test]
fn offer_index_ops_follow_the_rows_content() -> R {
    use paid::tests::resale;
    let mut f = resale::resale_rig()?;
    let (_, p) = resale::resale_offer(&mut f, token(10), OTHER as u32, None);
    resale::resale_commit(&mut f, p?)?;
    let deleted =
        |t: &mut ActionTxn<'_>, h: &Handles| delete_row(t, h, RESALE_OFFERS, &resale_key(OTHER, 1));
    let (_, p) = resale::resale_offer(&mut f, token(13), OTHER as u32, Some(&deleted));
    refuses(boxed(p), RESALE, "a Delete of the buyer's row at the Offer");
    let mut f = charter_pending::tests::pending_rig()?;
    let p = charter_pending::tests::pending_offer(&mut f, token(4), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    let deleted =
        |t: &mut ActionTxn<'_>, h: &Handles| delete_row(t, h, PENDING_OFFERS, &pending_key(BUYER));
    refuses(
        charter_pending::tests::pending_offer(&mut f, token(7), Some(&deleted)),
        PENDING,
        "a Delete of the buyer's row at the Offer",
    );
    let p = charter_pending::tests::pending_offer(&mut f, token(7), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    refuses(
        charter_pending::tests::pending_fund(&mut f, token(4), token(5), Some(&deleted)),
        PENDING,
        "a Delete of a row that still holds another live offer",
    );
    let p = charter_pending::tests::pending_fund(&mut f, token(4), token(5), None)?;
    charter_pending::tests::pending_commit(&mut f, p)?;
    Ok(())
}
