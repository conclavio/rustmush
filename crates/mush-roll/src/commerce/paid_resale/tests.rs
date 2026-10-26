//! Structural compatibility only; decoded data is never publication authority.
use super::*;
use crate::commerce::{
    TermGeneration, paid::PersonalPaidLicence, records::Record, roots::RootAnchor,
};
use mush_core::{CommitToken, DefinitionDigest};
use mush_economy::managed_paid_resale::PaidBirthRefV1;
use serde_json::{Value, json};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn hash(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn original() -> Value {
    json!({"format":1,"id":"1","charter":"2","line":"3","holder":4,"term":"1","operation":token(1)})
}
fn holder() -> Result<(PaidBirthRefV1, PaidHolderRecord)> {
    let birth = json!({"route":"entitlement","operation":token(1),"digest":hash(1)});
    let row = json!({
        "format":1,"licence":"1","generation":2,"holder":5,"birth":birth,
        "predecessor":{"generation":1,"operation":token(1),"digest":hash(1)},
        "operation":token(3),"action":protocol::actions()?[1],"acceptance":token(3),
        "offer":token(2),"offer_digest":hash(2),
        "term":{"generation":"1","operation":token(1),"digest":hash(3)},
        "accepted_at_ic":{"clock":{"kind":"world_ic","generation":1},"at":100},
        "economic_digest":hash(4),
    });
    Ok((serde_json::from_value(birth)?, serde_json::from_value(row)?))
}
#[test]
fn old_pointer_bytes_and_strict_transferred_pointer_are_disjoint() -> Result {
    let old = original();
    let p: PersonalPaidLicence = serde_json::from_value(old.clone())?;
    check_eq!(serde_json::to_value(&p)?, old);
    let (birth, holder) = holder()?;
    let next = p.transferred(&birth, &holder)?;
    let valid = serde_json::to_value(&next)?;
    check_eq!(valid["format"], 2);
    check_eq!(
        serde_json::from_value::<PersonalPaidLicence>(valid.clone())?,
        next
    );
    let mut cases = Vec::new();
    let mut x = old.clone();
    x["ownership"] = Value::Null;
    cases.push(x);
    let mut x = old;
    x["ownership"] = valid["ownership"].clone();
    cases.push(x);
    let mut x = valid.clone();
    x.as_object_mut().ok_or("object")?.remove("ownership");
    cases.push(x);
    let mut x = valid.clone();
    x["ownership"] = Value::Null;
    cases.push(x);
    for generation in [1, 65538] {
        let mut x = valid.clone();
        x["ownership"]["generation"] = json!(generation);
        cases.push(x);
    }
    let mut x = valid.clone();
    x["ownership"]["unknown"] = json!(1);
    cases.push(x);
    let mut x = valid;
    x["ownership"]["operation"] = json!(token(1));
    cases.push(x);
    for value in cases {
        check!(serde_json::from_value::<PersonalPaidLicence>(value).is_err());
    }
    Ok(())
}
#[test]
fn structural_transfer_keeps_birth_and_term_then_next_term_keeps_ownership() -> Result {
    let old: PersonalPaidLicence = serde_json::from_value(original())?;
    let (birth, h) = holder()?;
    let next = old.transferred(&birth, &h)?;
    check_eq!(next.id(), old.id());
    check_eq!(next.charter(), old.charter());
    check_eq!(next.line(), old.line());
    check_eq!(next.operation(), old.operation());
    check_eq!(next.term(), old.term());
    check_eq!(next.holder(), h.holder());
    let renewed = next.with_next_term(TermGeneration::new(2)?)?;
    check_eq!(renewed.ownership(), next.ownership());
    check_eq!(renewed.holder(), next.holder());
    check_eq!(renewed.operation(), next.operation());
    check!(next.with_next_term(TermGeneration::new(3)?).is_err());
    check!(
        next.transferred(&birth, &h).is_err(),
        "same captured ownership cannot transfer twice"
    );
    for (key, value) in [("licence", json!("9")), ("holder", json!(4))] {
        let mut row = serde_json::to_value(&h)?;
        row[key] = value;
        let crossed: PaidHolderRecord = serde_json::from_value(row)?;
        check!(old.transferred(&birth, &crossed).is_err());
    }
    let mut row = serde_json::to_value(&h)?;
    row["term"]["generation"] = json!("2");
    check!(
        old.transferred(&birth, &serde_json::from_value(row)?)
            .is_err()
    );
    Ok(())
}
#[test]
fn exact_new_family_keys_and_private_root_spellings_are_bounded() -> Result {
    use crate::commerce::families::valid_key;
    let (_, row) = holder()?;
    check!(valid_key(PaidHolderRecord::FAMILY, &row.key()));
    check!(valid_key(
        PaidHolderRecord::FAMILY,
        &PaidHolderRecord::key_for(row.licence(), 65537)
    ));
    for generation in [0, 1, 65538, u64::MAX] {
        check!(!valid_key(
            PaidHolderRecord::FAMILY,
            &PaidHolderRecord::key_for(row.licence(), generation)
        ));
    }
    check!(valid_key(PaidResaleHead::FAMILY, &row.licence().key()));
    check!(valid_key(PaidResaleRecord::FAMILY, &token(2).to_hex()));
    for root in [
        RootAnchor::PaidResaleFunding(token(3), hash(1)),
        RootAnchor::PaidResaleTerminal(token(3), hash(2)),
    ] {
        check_eq!(RootAnchor::parse(&root.record())?, root);
        check!(RootAnchor::parse(&format!("{}/extra", root.record())).is_err());
    }
    Ok(())
}
