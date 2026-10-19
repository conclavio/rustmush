//! Exact registered typed families; no legacy owner map mirrors these rows.

use mush_core::mechanics::CanonicalId;
use mush_core::{ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use serde_json::Value;

use super::paid::{PaidTermRecord, PersonalPaidLicence};
use super::purchase::PaidAllocation;
use super::records::*;

/// Handles are minted only by Roll registration in this exact composition.
/// No public constructor, family-name lookup or blanket delegation is added.
#[derive(Clone)]
pub struct CommerceFamilies {
    pub(crate) renewal_operations: StateFamilyHandle<String, super::renewal::RenewalOperationV1>,
    pub(crate) renewal_heads: StateFamilyHandle<String, super::renewal::RenewalHeadV1>,
    pub(crate) authored_retirements:
        StateFamilyHandle<String, super::authored_retirement::records::RetirementOperation>,
    pub(crate) retired_draft_parents:
        StateFamilyHandle<String, super::authored_retirement::records::RetiredDraftParent>,
    pub(crate) filed_royalties: StateFamilyHandle<String, super::entitlement::FiledRoyaltyTermsV1>,
    pub(crate) term_economics: StateFamilyHandle<String, super::entitlement::PaidTermEconomicsV1>,
    pub(crate) entitlement_allocations:
        StateFamilyHandle<String, super::entitlement::EntitlementAllocationV1>,
    pub(crate) owner_parents: StateFamilyHandle<String, super::dependent_owner::OwnerParentBinding>,
    pub(crate) draft_parents:
        StateFamilyHandle<String, super::parent_selector::DraftParentSelection>,
    pub(crate) deployment: StateFamilyHandle<String, CommerceDeployment>,
    pub(crate) charters: StateFamilyHandle<String, CharterRecord>,
    pub(crate) lines: StateFamilyHandle<String, LineRecord>,
    pub(crate) filed: StateFamilyHandle<String, FiledRecord>,
    pub(crate) status: StateFamilyHandle<String, RevisionStatus>,
    pub(crate) licences: StateFamilyHandle<String, OwnerLicence>,
    pub(crate) terms: StateFamilyHandle<String, OwnerTerm>,
    pub(crate) operations: StateFamilyHandle<String, InitialFilingOperation>,
    pub(crate) identities: StateFamilyHandle<String, FiledIdentityIndex>,
    pub(crate) positions: StateFamilyHandle<String, FiledPositionIndex>,
    pub(crate) paid_allocations: StateFamilyHandle<String, PaidAllocation>,
    pub(crate) paid_licences: StateFamilyHandle<String, PersonalPaidLicence>,
    pub(crate) paid_terms: StateFamilyHandle<String, PaidTermRecord>,
}

impl CommerceFamilies {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        Ok(Self {
            renewal_operations: register(reg)?,
            renewal_heads: register(reg)?,
            authored_retirements: register(reg)?,
            retired_draft_parents: register(reg)?,
            filed_royalties: register(reg)?,
            term_economics: register(reg)?,
            entitlement_allocations: register(reg)?,
            owner_parents: register(reg)?,
            draft_parents: register(reg)?,
            deployment: register(reg)?,
            charters: register(reg)?,
            lines: register(reg)?,
            filed: register(reg)?,
            status: register(reg)?,
            licences: register(reg)?,
            terms: register(reg)?,
            operations: register(reg)?,
            identities: register(reg)?,
            positions: register(reg)?,
            paid_allocations: register(reg)?,
            paid_licences: register(reg)?,
            paid_terms: register(reg)?,
        })
    }
}

fn contract<R: Record>(suffix: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix))
        .map_err(|e| ModError::msg(e.to_string()))
}

fn register<R: Record>(reg: &mut Registrar<'_>) -> Result<StateFamilyHandle<String, R>, ModError> {
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(|e| ModError::msg(e.to_string()))?,
        contract::<R>("key")?,
        contract::<R>("value")?,
        1,
        contract::<R>("strict_decode")?,
        contract::<R>("invariants")?,
        contract::<R>("canonical")?,
    )
    .and_then(|d| d.with_limits(192, R::MAX_BYTES))
    .map_err(|e| ModError::msg(e.to_string()))?;
    reg.state_family(
        descriptor,
        |key: &String| {
            if valid_key(R::FAMILY, key) {
                Ok(key.clone())
            } else {
                Err("invalid canonical commercial key".into())
            }
        },
        |value: &Value| {
            // The core bounds the complete family envelope before callback.
            let row: R = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            row.validate().map_err(|e| e.to_string())?;
            Ok(row)
        },
        |row: &R| row.validate().map_err(|e| e.to_string()),
        |row: &R| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}

fn hex(text: &str, width: usize, nonzero: bool) -> bool {
    text.len() == width
        && text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && (!nonzero || text.bytes().any(|c| c != b'0'))
}

pub(crate) fn valid_key(family: &str, key: &str) -> bool {
    let mut parts = key.split('/');
    let head = parts.next().unwrap_or("");
    match family {
        "roll_mind_owner_parent" => {
            head == "licence"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next() == Some("owner")
                && parts.next().is_some_and(|v| hex(v, 8, true))
                && parts.next().is_none()
        }
        "roll_mind_draft_parent" | "roll_mind_retired_draft_parent" => {
            head == "draft"
                && parts.next().is_some_and(|v| hex(v, 8, false))
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        "roll_mind_commerce" => head == "registry" && parts.next().is_none(),
        "roll_mind_initial_filing"
        | "roll_mind_paid_purchase"
        | "roll_mind_authored_retirement"
        | "roll_mind_entitlement_purchase"
        | "roll_mind_renewal_operation" => hex(head, 32, false) && parts.next().is_none(),
        "roll_mind_filed_identity" => hex(head, 64, false) && parts.next().is_none(),
        "roll_mind_owner_term" | "roll_mind_paid_term" | "roll_mind_paid_term_economics" => {
            head == "licence"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next() == Some("term")
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        "roll_mind_filed_position" => {
            head == "line"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next() == Some("0000000000000001")
                && parts.next().is_none()
        }
        "roll_mind_charter" => {
            head == "charter"
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16).is_ok_and(|id| u32::try_from(id).is_ok())
                })
                && parts.next().is_none()
        }
        "roll_mind_line"
        | "roll_mind_filed"
        | "roll_mind_filed_royalty_terms"
        | "roll_mind_status"
        | "roll_mind_owner_licence"
        | "roll_mind_personal_paid_licence"
        | "roll_mind_renewal_head" => {
            let prefix = match family {
                "roll_mind_line" => "line",
                "roll_mind_owner_licence"
                | "roll_mind_personal_paid_licence"
                | "roll_mind_renewal_head" => "licence",
                _ => "filed",
            };
            head == prefix
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        _ => false,
    }
}
