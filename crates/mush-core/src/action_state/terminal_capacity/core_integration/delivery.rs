//! Fixed personal-custody promise. Only actual retained manufacture data
//! selects this profile; no module chooses target vectors or handoff images.
use super::*;

pub(super) const CORE: &str = "core_output_deliveries";
pub(super) const INDUSTRY: &str = "industry_core_deliveries";
pub(super) const FAMILIES: [(&str, &str, usize); 2] =
    [("industry", CORE, 16_384), ("industry", INDUSTRY, 4096)];
pub(super) fn promised(policy: &Value) -> Result<bool, ActionStateError> {
    match policy.get("delivery") {
        None => Ok(false),
        Some(Value::String(s)) if s == "personal_custody_v1" => Ok(true),
        _ => Err(refused("core delivery closed promise")),
    }
}
pub(super) fn completed(r: &Row) -> Result<bool, ActionStateError> {
    Ok(r.terminal()?
        .is_some_and(|t| t.pointer("/result/outcome") == Some(&json!("completed"))))
}
pub(super) fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ActionStateError> {
    let (key_bytes, record_bytes) = match name {
        CORE => (32, 16_384),
        INDUSTRY => (16, 4096),
        _ => return Err(refused("core delivery family")),
    };
    let c = |suffix| CanonicalId::new(format!("industry.{name}.{suffix}.v1")).map_err(bad);
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(bad)?,
        c("key")?,
        c("value")?,
        1,
        c("decode")?,
        c("validate")?,
        c("canonical")?,
    )
    .and_then(|d| d.with_limits(key_bytes, record_bytes))
    .map_err(bad)
}
pub(super) fn action_reference() -> Result<DefinitionRef, ActionStateError> {
    let d = crate::DefinitionDescriptor::new(
        crate::DefinitionKind::new("action").map_err(bad)?,
        crate::DefinitionId::new("deliver_core_output").map_err(bad)?, 1,
        json!({"format":1,"protocol":"certified_core_delivery_v1",
            "authority":"actual_completed_requester","effect":"ordinary_personal_custody",
            "current_recipe_required":false,"core_receipt_bytes":16384,"industry_receipt_bytes":4096}))
        .map_err(bad)?;
    crate::DefinitionRegistryBuilder::new()
        .register(owner_id("industry")?, d)
        .map_err(bad)
}
fn optional_row(
    view: &View<'_>,
    family: &str,
    order: u64,
) -> Result<Option<Value>, ActionStateError> {
    let id = StateFamilyId::new(family).map_err(bad)?;
    if !view
        .registry
        .entries
        .contains_key(&(owner_id("industry")?, id))
    {
        return Ok(None);
    }
    value(
        view,
        &key(
            family,
            if family == CORE {
                order.to_string()
            } else {
                format!("{order:016x}")
            },
        )?,
    )
}
pub(super) fn receipts(view: &View<'_>, r: &Row) -> Result<bool, ActionStateError> {
    let core = optional_row(view, CORE, r.order)?;
    let industry = optional_row(view, INDUSTRY, r.order)?;
    let (core, industry) = match (core, industry) {
        (None, None) => return Ok(false),
        (Some(c), Some(i)) => (c, i),
        _ => return Err(refused("core delivery partial receipt pair")),
    };
    if !promised(&r.policy)? || !completed(r)? {
        return Err(refused("core delivery receipt before paid completion"));
    }
    strict(
        &core,
        &[
            "format",
            "operation",
            "action",
            "output",
            "at_utc",
            "delivered_digest",
        ],
        &[],
    )?;
    strict(
        &industry,
        &[
            "format",
            "sequence",
            "order",
            "accepted",
            "completion",
            "operation",
            "action",
            "object",
            "reservation",
            "at_utc",
            "core_receipt",
        ],
        &[],
    )?;
    let terminal = r
        .terminal()?
        .ok_or_else(|| refused("core delivery terminal"))?;
    let output = required(view, &key(OUTPUT, r.order.to_string())?)?;
    let operation: CommitToken = parsed(field(&core, "operation")?)?;
    let core_digest =
        DefinitionDigest::for_canonical_value("industry.core_delivery.receipt.v1", &core)
            .map_err(bad)?;
    if number(&core, "format")? != 1
        || number(&industry, "format")? != 1
        || !(1..=8192).contains(&number(&industry, "sequence")?)
        || number(&industry, "order")? != r.order
        || field(&core, "action")? != &json!(action_reference()?)
        || field(&core, "output")? != &output
        || operation == r.accepted
        || field(&core, "operation")? == field(terminal, "commit")?
        || field(&core, "operation")? == field(&output, "reservation")?
        || field(&industry, "accepted")? != &json!(r.accepted)
        || field(&industry, "completion")? != field(terminal, "commit")?
        || field(&industry, "object")? != field(&output, "object")?
        || field(&industry, "reservation")? != field(&output, "reservation")?
        || field(&industry, "core_receipt")? != &json!(core_digest)
        || ["operation", "action", "at_utc"]
            .into_iter()
            .any(|n| industry.get(n) != core.get(n))
    {
        return Err(refused("core delivery complete immutable receipt join"));
    }
    parsed::<i64>(field(&core, "at_utc")?)?;
    parsed::<DefinitionDigest>(field(&core, "delivered_digest")?)?;
    Ok(true)
}
pub(super) fn require_future_room(r: &Row, output: &Value) -> Result<(), ActionStateError> {
    if !promised(&r.policy)? {
        return Ok(());
    }
    let action = action_reference()?;
    let token = CommitToken::from_bytes([255; 16]);
    let digest = DefinitionDigest::from_bytes([255; 32]);
    let core = json!({"format":1,"operation":token,"action":action,"output":output,
        "at_utc":i64::MIN,"delivered_digest":digest});
    canonical_json_bytes_with_limit(&envelope(CORE, &r.order.to_string(), core)?, 16_384)
        .map_err(|_| refused("core delivery complete future receipt ceiling"))?;
    let industry = json!({"format":1,"sequence":8192,"order":r.order,"accepted":r.accepted,
        "completion":token,"operation":token,"action":action,"object":u32::MAX,
        "reservation":token,"at_utc":i64::MIN,"core_receipt":digest});
    canonical_json_bytes_with_limit(
        &envelope(INDUSTRY, &format!("{:016x}", r.order), industry)?,
        4096,
    )
    .map_err(|_| refused("Industry delivery complete future receipt ceiling"))?;
    Ok(())
}
/// These two images are derived from one exact retained reservation. This
/// comparison is also repeated at the generic accounting boundary.
pub(in crate::action_state::terminal_capacity) fn require_handoff(
    before: &Reservation,
    after: &Reservation,
) -> Result<(), ActionStateError> {
    if before.format != after.format
        || before.funding != after.funding
        || before.effect != after.effect
        || before.sponsor != after.sponsor
        || before.protocol_fingerprint != after.protocol_fingerprint
        || before.targets.len() != 12
        || after.targets.len() != 3
        || after.targets.iter().any(|t| !before.targets.contains(t))
        || after
            .targets
            .iter()
            .any(|t| ![ORDER, CORE, INDUSTRY].contains(&t.key.family.as_str()))
    {
        return Err(refused(
            "core delivery handoff must strictly shrink exact admission",
        ));
    }
    Ok(())
}
