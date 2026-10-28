//! Codec and complete-envelope bounds only. Padded native JSON below is an
//! explicit upper-bound sizing device, never decoded or installed as history.
use super::{protocol, records::*};
use crate::commerce::{families::CommerceFamilies, records::Record};
use mush_core::{
    CommitToken, DefinitionDigest, Engine, GameModule, ModError, Registrar, Registry,
    StateFamilyHandle, World,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn wide_offer() -> Result<PendingOffer> {
    let d = DefinitionDigest::from_bytes([255; 32]);
    let ownership = json!({"generation":65536,"operation":token(252),"digest":d});
    let window = json!({"format":1,"eligibility_start":i64::MAX-200,"issued_at":i64::MAX-200,"term_end":i64::MAX-100,"grace_end":i64::MAX});
    let term = json!({"licence":u64::MAX.to_string(),"generation":u64::MAX.to_string(),"operation":token(251),"digest":d,"window":window});
    let revision = json!({"format":1,"revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":d},"source":d});
    Ok(serde_json::from_value(json!({
        "format":1,"operation":token(254),"action":protocol::offer_action()?,
        "charter":u32::MAX.to_string(),"birth":token(253),"ownership":ownership,
        "seller":u32::MAX,"buyer":u32::MAX-1,"seller_account":u64::MAX,"instrument":"cr",
        "price":i64::MAX,"title_revision":u64::MAX-1,"line":u64::MAX.to_string(),"filed":u64::MAX.to_string(),
        "revision":revision,"owner_term":term,
        "offered_at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX-200},
        "offered_at_utc":i64::MAX-86400,"expires_at_utc":i64::MAX,"settle_seconds":259200
    }))?)
}
type Handles = (
    StateFamilyHandle<String, PendingOffer>,
    StateFamilyHandle<String, PendingRecord>,
    StateFamilyHandle<String, PendingHead>,
    StateFamilyHandle<String, crate::commerce::charter_transfer::records::CharterTransfer>,
    StateFamilyHandle<String, crate::commerce::charter_transfer::records::CharterOwnershipHead>,
);
struct Rows(Arc<Mutex<Option<Handles>>>);
impl GameModule for Rows {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let f = CommerceFamilies::register(reg)?;
        *self.0.lock().map_err(|_| ModError::msg("handles"))? = Some((
            f.charter_pending_offers,
            f.charter_pending,
            f.charter_pending_heads,
            f.charter_transfers,
            f.charter_ownership,
        ));
        Ok(())
    }
}
fn document<R: Record>(h: &StateFamilyHandle<String, R>, key: String, value: Value) -> Value {
    json!({"family":{"owner":h.owner(),"id":h.descriptor().id()},"schema":{"id":h.descriptor().value_schema(),"version":1},"key":key,"value":value})
}
#[test]
fn pending_offer_is_closed_and_uses_exact_new_seller_action() -> Result {
    let offer = wide_offer()?;
    check_eq!(
        serde_json::from_value::<PendingOffer>(serde_json::to_value(&offer)?)?,
        offer
    );
    check_eq!(offer.native()?.offer_digest, offer.digest()?);
    for field in ["source", "valid", "authority", "parent"] {
        let mut value = serde_json::to_value(&offer)?;
        value
            .as_object_mut()
            .ok_or("offer")?
            .insert(field.into(), json!(true));
        check!(serde_json::from_value::<PendingOffer>(value).is_err());
    }
    let mut value = serde_json::to_value(&offer)?;
    value.as_object_mut().ok_or("offer")?.insert(
        "action".into(),
        serde_json::to_value(crate::commerce::charter_transfer::protocol::offer_action()?)?,
    );
    check!(serde_json::from_value::<PendingOffer>(value).is_err());
    for field in ["owner_term", "ownership", "offered_at_ic", "settle_seconds"] {
        let mut value = serde_json::to_value(&offer)?;
        value.as_object_mut().ok_or("offer")?.remove(field);
        check!(serde_json::from_value::<PendingOffer>(value).is_err());
    }
    Ok(())
}
#[test]
fn full_native_history_ceiling_plus_widest_outer_images_fits_four_future_targets() -> Result {
    use crate::commerce::charter_transfer::records::{CharterOwnershipHead, CharterTransfer};
    let shared = Arc::new(Mutex::new(None));
    let _engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Rows(shared.clone()))],
    )?;
    let (offers, rows, heads, transfers, owners) = shared
        .lock()
        .map_err(|_| "handles")?
        .clone()
        .ok_or("handles")?;
    let offer = wide_offer()?;
    let d = DefinitionDigest::from_bytes([255; 32]);
    let transfer = json!({"generation":65537,"operation":token(249),"digest":d});
    let prior = json!({"sequence":65535,"operation":token(248),"digest":d});
    let owner_term = serde_json::to_value(&offer.owner_term)?;
    let stamp = json!({"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX});
    // Any validated native history is at most this many canonical JSON bytes.
    // Substitute an exactly equally long JSON value without assuming that the
    // actual maximum is attained. It is never treated as a valid native proof.
    let native =
        json!("x".repeat(mush_economy::managed_charter_transfer::MAX_PENDING_HISTORY_BYTES - 2));
    check_eq!(
        mush_core::canonical_json_bytes_with_limit(&native, 32768)?.len(),
        32768
    );
    let reasons = [
        json!({"kind":"clock_changed"}),
        json!({"kind":"recipient_unavailable"}),
        json!({"kind":"registry_revoked","sanction":token(249),"licence":u64::MAX,"generation":u64::MAX,"term_digest":d}),
    ];
    let mut states = vec![
        json!({"phase":"funded"}),
        json!({"phase":"settled","terminal":token(249),"transfer":transfer}),
    ];
    states.extend(
        reasons
            .into_iter()
            .map(|reason| json!({"phase":"refunded","terminal":token(249),"reason":reason})),
    );
    for state in states {
        let row = json!({"format":1,"operation":token(250),"action":protocol::fund_action()?,"sequence":65536,"previous":prior,
            "offer":offer.operation,"offer_digest":d,"charter":offer.charter,"ownership":offer.ownership,
            "owner_term":owner_term,"accepted_at_ic":stamp,"deadline":stamp,
            "claim_observation":{"generation":u64::MAX,"next_transition":u64::MAX,"open_owned":1024},"economic":native,"state":state});
        let head = json!({"format":1,"charter":offer.charter,"current":{"sequence":65536,"operation":token(250),"digest":d},"state":state});
        check!(
            mush_core::canonical_json_bytes_with_limit(
                &document(&rows, token(250).to_hex(), row),
                PendingRecord::MAX_BYTES
            )?
            .len()
                < PendingRecord::MAX_BYTES
        );
        check!(
            mush_core::canonical_json_bytes_with_limit(
                &document(&heads, offer.charter.key(), head),
                PendingHead::MAX_BYTES
            )?
            .len()
                < PendingHead::MAX_BYTES
        );
    }
    let ownership = json!({"format":2,"operation":token(249),"action":protocol::settle_action()?,"charter":offer.charter,"generation":65537,"birth":offer.birth,
        "predecessor":offer.ownership,"offer":offer.operation,"offer_digest":d,"accepted_at_ic":stamp,"economic":{"pending":native}});
    let head = json!({"format":1,"charter":offer.charter,"current":transfer});
    check!(
        mush_core::canonical_json_bytes_with_limit(
            &document(
                &transfers,
                CharterTransfer::key_for(offer.charter, 65537),
                ownership
            ),
            CharterTransfer::MAX_BYTES
        )?
        .len()
            < CharterTransfer::MAX_BYTES
    );
    check!(
        mush_core::canonical_json_bytes_with_limit(
            &document(&owners, offer.charter.key(), head),
            CharterOwnershipHead::MAX_BYTES
        )?
        .len()
            < CharterOwnershipHead::MAX_BYTES
    );
    check!(
        mush_core::canonical_json_bytes_with_limit(
            &document(&offers, offer.key(), serde_json::to_value(&offer)?),
            PendingOffer::MAX_BYTES
        )?
        .len()
            < PendingOffer::MAX_BYTES
    );
    check_eq!(
        PendingRecord::MAX_BYTES
            + PendingHead::MAX_BYTES
            + CharterTransfer::MAX_BYTES
            + CharterOwnershipHead::MAX_BYTES,
        90112
    );
    Ok(())
}

/// 17g C2f: the per-(buyer, charter) pending offer index (test 11).
#[path = "buyer_offer_tests.rs"]
pub(crate) mod buyer_offers;
