#![cfg(test)]

//! Public selector shape is distinct from persistent typed identifier strings.
use super::provider::{Kind, Request, decode};
use serde_json::json;
#[test]
fn terminal_receipt_key_is_the_reserved_acceptance_and_retains_actual_terminal() {
    use super::{families::Row, records::MindPaidResaleReceipt};
    use mush_core::{CommitToken, DefinitionDigest};
    let acceptance = CommitToken::from_bytes([2; 16]);
    let terminal = CommitToken::from_bytes([3; 16]);
    for outcome in ["settled", "refunded"] {
        let value = json!({"format":1,"operation":terminal,"acceptance":acceptance,
            "licence":"1","offer":CommitToken::from_bytes([1;16]),
            "offer_digest":DefinitionDigest::from_bytes([4;32]),
            "roll_digest":DefinitionDigest::from_bytes([5;32]),
            "economic_digest":DefinitionDigest::from_bytes([6;32]),"outcome":outcome});
        let receipt: MindPaidResaleReceipt = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(receipt.key(), acceptance);
        assert_ne!(receipt.key(), terminal);
        assert_eq!(serde_json::to_value(&receipt).unwrap(), value);
    }
}
#[test]
fn resale_offer_uses_public_integer_selectors_and_refuses_unpublished_fields() {
    let value = json!({"licence":1,"buyer":2,"price":100,"seller_account":0});
    assert!(matches!(
        decode(Kind::Offer, &value),
        Ok(Request::Offer { .. })
    ));
    for (field, bad) in [
        ("licence", json!("1")),
        ("licence", json!(0)),
        ("buyer", json!(0)),
        ("buyer", json!(4294967296u64)),
        ("price", json!(0)),
        ("seller_account", json!("0")),
        ("seller_account", json!(u64::MAX)),
    ] {
        let mut crossed = value.clone();
        crossed[field] = bad;
        assert!(decode(Kind::Offer, &crossed).is_err());
    }
    let mut crossed = value;
    crossed["term_end"] = json!(100);
    assert!(decode(Kind::Offer, &crossed).is_err());
}
#[test]
fn resale_acceptance_uses_the_exact_named_offer_and_explicit_payer() {
    let value = json!({"offer":"00000000000000000000000000000001","payer":0});
    for kind in [Kind::Accept, Kind::Fund] {
        assert!(matches!(decode(kind, &value), Ok(Request::Accept { .. })));
        for bad in [
            json!({"offer":1,"payer":0}),
            json!({"offer":value["offer"]}),
            json!({"offer":value["offer"],"payer":"0"}),
            json!({"offer":value["offer"],"payer":0,"reason":"last_claim_closed"}),
        ] {
            assert!(decode(kind, &bad).is_err());
        }
    }
}
