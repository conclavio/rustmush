#![cfg(test)]

//! Doc 46 CT.0: codec pins of the paid-resale records (`records.rs`' `data!`)
//! and what `families.rs`' `row!` names (each family's id, ceiling and the
//! key its row is stored under), with the registered family descriptors and
//! the offer digest. No walk: mush-core refuses a paid-resale write unless the
//! whole protocol is composed (`buyer_offer_tests`' note), so the rows here
//! are the existing offer fixture and its paired operation and receipt rows.
use super::super::families::{Families, Row};
use super::super::records::{MindPaidResaleOffer, MindPaidResaleOperation, MindPaidResaleReceipt};
use super::buyer_offers::offer;
use mush_core::{
    Engine, GameModule, ModError, ObjId, Registrar, Registry, StateFamilyDescriptor, World,
};
use mush_roll::commerce::paid_resale::protocol;
use mush_testkit::fixtures::{digest, token};
use mush_testkit::snapshot::Snapshot;
use serde_json::json;
use std::sync::{Arc, Mutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Ct0Module(Arc<Mutex<Option<Families>>>);
impl GameModule for Ct0Module {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let families = Families::register(reg)?;
        *self
            .0
            .lock()
            .map_err(|_| ModError::msg("ct0 family lock"))? = Some(families);
        Ok(())
    }
}

#[inline(never)]
fn families() -> TestResult<Families> {
    let slot = Arc::new(Mutex::new(None));
    let _engine = Box::new(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Ct0Module(slot.clone()))],
    )?);
    let families = slot
        .lock()
        .map_err(|_| "ct0 family lock")?
        .take()
        .ok_or("ct0 families")?;
    Ok(families)
}

fn descriptor(snap: &mut Snapshot, name: &str, owner: &str, d: &StateFamilyDescriptor) {
    snap.line(&format!("{name} family owner"), owner);
    snap.line(&format!("{name} family id"), d.id().as_str());
    snap.line(&format!("{name} key codec"), d.key_codec().as_str());
    snap.line(&format!("{name} value schema"), d.value_schema().as_str());
    snap.line(
        &format!("{name} schema version"),
        &d.schema_version().to_string(),
    );
    snap.line(&format!("{name} decoder"), d.decoder().as_str());
    snap.line(&format!("{name} validator"), d.validator().as_str());
    snap.line(&format!("{name} serializer"), d.serializer().as_str());
    snap.line(
        &format!("{name} max key bytes"),
        &d.max_key_bytes().to_string(),
    );
    snap.line(
        &format!("{name} max record bytes"),
        &d.max_record_bytes().to_string(),
    );
}

/// One row through `row!`'s impl: its family, ceiling and stored key, and
/// its canonical bytes decoding back.
fn row<R: Row + PartialEq + std::fmt::Debug>(
    snap: &mut Snapshot,
    label: &str,
    value: &R,
    decode: fn(&[u8]) -> Result<R, String>,
) -> TestResult {
    snap.line(&format!("{label} family"), R::FAMILY);
    snap.line(&format!("{label} max bytes"), &R::MAX_BYTES.to_string());
    snap.line(&format!("{label} key"), &value.key().to_hex());
    value.validate()?;
    let bytes =
        mush_core::canonical_json_bytes_with_limit(&serde_json::to_value(value)?, R::MAX_BYTES)?;
    snap.bytes(&format!("{label} canonical bytes"), &bytes);
    check_eq!(&decode(&bytes)?, value);
    Ok(())
}

#[inline(never)]
fn operation(
    offer: &MindPaidResaleOffer,
    state: serde_json::Value,
    action: usize,
) -> TestResult<Box<MindPaidResaleOperation>> {
    let actions = protocol::actions()?;
    Ok(Box::new(serde_json::from_value(json!({
        "format":1,"operation":token(20),"action":actions.get(action).ok_or("action")?,
        "licence":"1","sequence":1,"offer":offer.operation(),"offer_digest":offer.digest()?,
        "roll_digest":digest(5),"state":state
    }))?))
}

fn receipt(offer: &MindPaidResaleOffer, outcome: &str) -> TestResult<MindPaidResaleReceipt> {
    Ok(serde_json::from_value(json!({
        "format":1,"operation":token(21),"acceptance":token(20),"licence":"1",
        "offer":offer.operation(),"offer_digest":offer.digest()?,"roll_digest":digest(5),
        "economic_digest":digest(6),"outcome":outcome
    }))?)
}

#[test]
fn ct0_paid_resale_wire_snapshot() -> TestResult {
    let mut snap = Snapshot::new();
    snap.section("paid resale: families");
    let f = families()?;
    descriptor(
        &mut snap,
        "offers",
        f.offers.owner().as_str(),
        f.offers.descriptor(),
    );
    descriptor(
        &mut snap,
        "operations",
        f.operations.owner().as_str(),
        f.operations.descriptor(),
    );
    descriptor(
        &mut snap,
        "receipts",
        f.receipts.owner().as_str(),
        f.receipts.descriptor(),
    );
    descriptor(
        &mut snap,
        "buyer offers",
        f.buyer_offers.owner().as_str(),
        f.buyer_offers.descriptor(),
    );
    for (index, action) in protocol::actions()?.iter().enumerate() {
        snap.wire(&format!("protocol action {index}"), action)?;
    }

    let offer = offer(3, ObjId(8), 1, 100)?;
    snap.section("paid resale: rows");
    snap.line("offer digest", &offer.digest()?.to_hex());
    row(
        &mut snap,
        "offer",
        offer.as_ref(),
        MindPaidResaleOffer::decode,
    )?;
    let funded = operation(&offer, json!({"phase":"funded"}), 2)?;
    row(
        &mut snap,
        "funded operation",
        funded.as_ref(),
        MindPaidResaleOperation::decode,
    )?;
    let settled = operation(
        &offer,
        json!({"phase":"settled","terminal":token(21),
            "ownership":{"generation":2,"operation":token(21),"digest":digest(7)}}),
        2,
    )?;
    row(
        &mut snap,
        "settled operation",
        settled.as_ref(),
        MindPaidResaleOperation::decode,
    )?;
    let refunded = operation(
        &offer,
        json!({"phase":"refunded","terminal":token(21),"reason":{"kind":"clock_changed"}}),
        2,
    )?;
    row(
        &mut snap,
        "refunded operation",
        refunded.as_ref(),
        MindPaidResaleOperation::decode,
    )?;
    let settled_receipt = receipt(&offer, "settled")?;
    row(
        &mut snap,
        "settled receipt",
        &settled_receipt,
        MindPaidResaleReceipt::decode,
    )?;
    let refunded_receipt = receipt(&offer, "refunded")?;
    row(
        &mut snap,
        "refunded receipt",
        &refunded_receipt,
        MindPaidResaleReceipt::decode,
    )?;

    let format = |json: &mut serde_json::Value| json["format"] = 9.into();
    snap.codec("MindPaidResaleOffer", offer.as_ref(), format)?;
    snap.codec("MindPaidResaleOperation (funded)", funded.as_ref(), format)?;
    snap.codec(
        "MindPaidResaleOperation (settled)",
        settled.as_ref(),
        format,
    )?;
    snap.codec(
        "MindPaidResaleOperation (refunded)",
        refunded.as_ref(),
        format,
    )?;
    snap.codec("MindPaidResaleReceipt (settled)", &settled_receipt, format)?;
    snap.codec(
        "MindPaidResaleReceipt (refunded)",
        &refunded_receipt,
        format,
    )?;
    snap.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_paid_resale.snap",
    )
    .map_err(Into::into)
}
