//! Admission-only size proof for the complete future retained images.
//! Sizing values are never decoded, published, or accepted as owner evidence.
use super::{Error, bad, records::*, runtime::Runtime};
use mush_core::{DefinitionDigest, World, canonical_json_bytes_with_limit};
use serde_json::{Value, json};

pub(crate) fn terminal_room(
    world: &World,
    runtime: &Runtime,
    row: &RefitOrder,
) -> Result<(), Error> {
    row.validate()?;
    if row.terminal.is_some() || row.installation.is_some() {
        return Err(Error::Integrity);
    }
    let definitions = runtime
        .config
        .definitions
        .0
        .get()
        .ok_or(Error::Unavailable)?;
    let (_, journal) =
        super::policies::replacement(world, definitions, &runtime.config, &row.intent)?;
    let replacement = ReplacementEvidence::capture(&journal);
    replacement.digest()?;
    let installation = json!({"replacement":replacement,"core_receipt":DefinitionDigest::from_bytes([255;32]),"at_utc":i64::MIN});
    canonical_json_bytes_with_limit(&installation, INSTALLATION_BYTES).map_err(bad)?;
    let mut projected = serde_json::to_value(row).map_err(bad)?;
    let object = projected.as_object_mut().ok_or(Error::Integrity)?;
    // Progress and native terminal history have independent complete codec bounds.
    // A canonical string of that exact size is a conservative nested-value bound.
    object.insert(
        "progress".into(),
        Value::String("x".repeat(super::MAX_PROGRESS_BYTES - 2)),
    );
    object.insert("terminal".into(), Value::String("x".repeat(16_384 - 2)));
    object.insert("installation".into(), installation);
    canonical_json_bytes_with_limit(&projected, ORDER_BYTES).map_err(bad)?;
    let descriptor = runtime.families.orders.descriptor();
    let document = json!({"family":{"owner":"industry","id":"industry_refit_orders"},"key":format!("{:016x}",row.intent.order.0),"schema":{"id":descriptor.value_schema(),"version":descriptor.schema_version()},"value":projected});
    canonical_json_bytes_with_limit(&document, descriptor.max_record_bytes()).map_err(bad)?;
    Ok(())
}
