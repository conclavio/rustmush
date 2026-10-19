//! Source-free capture of request coordinates, never a mutation capability.
use crate::{EditBase, RollError, RollStore};
use mush_core::action::IdempotencyKeyText;
use mush_core::{CanonicalHash, DefinitionRef, ObjId};
use serde_json::{Value, json};

/// Exact owned metadata captured for the closed v2 retirement request. The real
/// provider still rechecks identity, current state and all acknowledged joins.
/// This carries neither source bytes nor permission to read or execute them.
pub struct AuthoredRetirementSelection {
    actor: ObjId,
    mind: u64,
    name: String,
    base: EditBase,
}
impl AuthoredRetirementSelection {
    /// Select only an actual owned marked draft. `None` means a validated
    /// original; errors must never be retried through a legacy mutation path.
    /// The adapter supplies its authenticated actor, not a request-body owner.
    pub fn capture(
        roll: &RollStore,
        actor: ObjId,
        name: &str,
        expected: Option<EditBase>,
    ) -> Result<Option<Self>, RollError> {
        let mind = roll.mind(actor, name)?;
        // Roll's owned lookup already refuses quarantined records. Capture only
        // their identity/version metadata; do not traverse working source merely
        // to request cleanup. The actual provider independently validates the
        // retained selector and owner publication before custody.
        if mind.character != actor
            || mind.id() == 0
            || !matches!(
                (mind.format, mind.provenance.as_ref()),
                (crate::model::FORMAT, None) | (crate::model::DERIVATIVE_FORMAT, Some(_))
            )
        {
            return Err(RollError::Missing);
        }
        if expected.is_some_and(|base| base != mind.base()) {
            return Err(RollError::Missing);
        }
        if mind.provenance.is_none() {
            return Ok(None);
        }
        roll.commerce_ready()?;
        Ok(Some(Self {
            actor,
            mind: mind.id(),
            name: mind.name().into(),
            base: mind.base(),
        }))
    }
    /// The actual canonical library label; no source material is returned.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Plain bounded selector data for the normal authenticated submission
    /// driver. Neither the key nor a decoded copy grants execution authority.
    pub fn into_request(self) -> Result<(DefinitionRef, Value, IdempotencyKeyText), RollError> {
        let action = super::captured_action_reference().map_err(|_| RollError::Missing)?;
        let parameters = json!({
            "mind": self.mind.to_string(), "name": self.name,
            "edit": self.base.edit.to_string(),
            "revision": self.base.revision.map(|n|n.to_string()).unwrap_or_else(||"none".into())
        });
        let hash = CanonicalHash::for_canonical_value(
            "roll.authored_retirement_selector.v2",
            2,
            &json!({"actor":self.actor,"action":action,"parameters":parameters}),
            4096,
        )
        .map_err(|_| RollError::Missing)?;
        let key = IdempotencyKeyText::parse_canonical(format!("mind.retire.{}", hash.to_hex()))
            .map_err(|_| RollError::Missing)?;
        Ok((action, parameters, key))
    }
}
