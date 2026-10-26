//! Exact optional paid-property transfer actions. These definitions alone grant no authority.
use mush_core::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    ModError, OwnerId,
};
use serde_json::json;
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub const OFFER: &str = "offer_paid_mind_resale";
pub const ACCEPT: &str = "accept_paid_mind_resale";
pub const FUND: &str = "fund_paid_mind_resale";
pub const SETTLE: &str = "settle_paid_mind_resale";
pub const REFUND: &str = "refund_paid_mind_resale";
pub fn definitions() -> Result<Vec<DefinitionDescriptor>, ModError> {
    let retention = super::super::authored_retirement::retention_reference()?;
    let mut rows = Vec::with_capacity(10);
    rows.push(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new("offer_paid_mind_resale").map_err(bad)?,1,json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"current_personal_paid_holder","named_buyer":true,"source_free":true,"existing_term":"unchanged","no_automatic_acceptance":true})).map_err(bad)?.with_references(vec![retention.clone()]));
    rows.push(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new("accept_paid_mind_resale").map_err(bad)?,1,json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"exact_zero","payment":"immediate_exact_gross_split","holder":"same_ack","term":"unchanged"})).map_err(bad)?.with_references(vec![retention.clone()]));
    rows.push(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new("fund_paid_mind_resale").map_err(bad)?,1,json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"named_buyer","claims":"positive","hold":"single_real_gross_escrow","holder":"unchanged","future_terminal_reserved":true})).map_err(bad)?.with_references(vec![retention.clone()]));
    rows.push(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new("settle_paid_mind_resale").map_err(bad)?,1,json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","trigger":"last_paid_claim_close_or_same_clock_deadline","old_claims":"retained","term":"unchanged"})).map_err(bad)?.with_references(vec![retention.clone()]));
    rows.push(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(bad)?,DefinitionId::new("refund_paid_mind_resale").map_err(bad)?,1,json!({"format":1,"protocol":"personal_paid_resale_v1","authority":"registered_private_service_intent","causes":["registry_revoked","clock_changed","recipient_unavailable","monetary_binding_changed"],"source_rights":"not_required"})).map_err(bad)?.with_references(vec![retention.clone()]));
    for id in [
        "paid_mind_resale_offered",
        "paid_mind_resale_accepted",
        "paid_mind_resale_funded",
        "paid_mind_resale_settled",
        "paid_mind_resale_refunded",
    ] {
        rows.push(
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").map_err(bad)?,
                DefinitionId::new(id).map_err(bad)?,
                1,
                json!({"format":1}),
            )
            .map_err(bad)?,
        );
    }
    Ok(rows)
}
pub fn actions() -> Result<[DefinitionRef; 5], ModError> {
    let mut rows = Vec::with_capacity(5);
    for d in definitions()?.into_iter().take(5) {
        rows.push(
            DefinitionRegistryBuilder::new()
                .register(OwnerId::new("market").map_err(bad)?, d)
                .map_err(bad)?,
        );
    }
    rows.try_into()
        .map_err(|_| ModError::msg("paid resale action count"))
}
pub(crate) fn action(
    index: usize,
) -> Result<DefinitionRef, super::super::records::PublicationError> {
    actions()
        .map_err(|_| super::super::records::PublicationError::Invalid)?
        .get(index)
        .cloned()
        .ok_or(super::super::records::PublicationError::Invalid)
}
