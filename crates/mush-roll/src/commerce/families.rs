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
    pub(crate) conduct_jobs: StateFamilyHandle<String, super::conduct::records::ConductJob>,
    pub(crate) conduct_executions:
        StateFamilyHandle<String, super::conduct::records::ConductExecution>,
    pub(crate) conduct_results: StateFamilyHandle<String, super::conduct::records::ConductResult>,
    pub(crate) coverage_publications:
        StateFamilyHandle<String, super::coverage::CoveragePublicationRetention>,
    pub(crate) coverages: StateFamilyHandle<String, super::coverage::CoverageRetention>,
    pub(crate) paid_resales: StateFamilyHandle<String, super::paid_resale::PaidResaleRecord>,
    pub(crate) paid_resale_heads: StateFamilyHandle<String, super::paid_resale::PaidResaleHead>,
    pub(crate) paid_holders: StateFamilyHandle<String, super::paid_resale::PaidHolderRecord>,
    pub(crate) measured_recalls:
        StateFamilyHandle<String, super::rating::reconformance::recall::records::MeasuredRecall>,
    pub(crate) reconformance_heads:
        StateFamilyHandle<String, super::rating::reconformance::ReconformanceHead>,
    pub(crate) charter_pending_offers:
        StateFamilyHandle<String, super::charter_pending::records::PendingOffer>,
    /// 17g C2f: the live pending-charter offers naming a buyer for one
    /// charter (`roll_mind_charter_pending_buyer_offers`, the C2b brief §4).
    pub(crate) charter_pending_buyer_offers:
        StateFamilyHandle<String, super::charter_pending::records::PendingBuyerOffers>,
    pub(crate) charter_pending:
        StateFamilyHandle<String, super::charter_pending::records::PendingRecord>,
    pub(crate) charter_pending_heads:
        StateFamilyHandle<String, super::charter_pending::records::PendingHead>,
    pub(crate) charter_transfer_offers:
        StateFamilyHandle<String, super::charter_transfer::records::CharterOffer>,
    pub(crate) charter_transfers:
        StateFamilyHandle<String, super::charter_transfer::records::CharterTransfer>,
    pub(crate) charter_ownership:
        StateFamilyHandle<String, super::charter_transfer::records::CharterOwnershipHead>,
    pub(crate) lapse_occurrences: StateFamilyHandle<String, super::facts::records::LapseOccurrence>,
    pub(crate) ic_claims: StateFamilyHandle<String, super::ic_claims::ClaimRecord>,
    pub(crate) owner_qualification_operations:
        StateFamilyHandle<String, super::owner_qualification::records::QualificationOperation>,
    pub(crate) owner_attestations:
        StateFamilyHandle<String, super::owner_attestation::records::AttestationOperation>,
    pub(crate) owner_qualification_heads:
        StateFamilyHandle<String, super::owner_qualification::records::QualificationHead>,
    pub(crate) line_updates:
        StateFamilyHandle<String, super::line_update::records::UpdateOperation>,
    pub(crate) update_positions:
        StateFamilyHandle<String, super::line_update::records::UpdatePosition>,
    pub(crate) registry_revocations:
        StateFamilyHandle<String, super::registry_revocation::records::RevocationOperation>,
    pub(crate) revoked_terms:
        StateFamilyHandle<String, super::registry_revocation::records::RevokedTerm>,
    pub(crate) registry_recalls:
        StateFamilyHandle<String, super::registry_recall::records::RecallOperation>,
    /// Doc 17h cut M8a: the Registry's hull inspections (`roll_registry_inspections`), keyed
    /// by the inspection's commit; written from tacsim's transaction through the delegation
    /// the inspection writer installs.
    pub(crate) registry_inspections:
        StateFamilyHandle<String, super::registry_inspection::records::InspectionRecord>,
    /// Doc 17h cut M8d D4: principals barred from new licences
    /// (`roll_registry_revoked_principals`), keyed as the holder indexes key; written from
    /// tacsim's transaction through the inspection writer's delegation.
    pub(crate) revoked_principals:
        StateFamilyHandle<String, super::registry_inspection::principal::RevokedPrincipal>,
    /// Doc 17h cut M8d2 S4: the Registry's seizures (`roll_registry_seizures`), keyed by the
    /// inspection that grounded each; written from tacsim's transaction through the
    /// inspection writer's delegation.
    pub(crate) registry_seizures:
        StateFamilyHandle<String, super::registry_inspection::SeizureRecord>,
    pub(crate) withdrawals:
        StateFamilyHandle<String, super::withdrawal::records::WithdrawalOperation>,
    pub(crate) house_renewal_operations:
        StateFamilyHandle<String, super::house_renewal::RenewalOperationV1>,
    pub(crate) house_renewal_heads: StateFamilyHandle<String, super::house_renewal::RenewalHeadV1>,
    pub(crate) renewal_operations: StateFamilyHandle<String, super::renewal::RenewalOperationV1>,
    pub(crate) renewal_heads: StateFamilyHandle<String, super::renewal::RenewalHeadV1>,
    pub(crate) authored_retirements:
        StateFamilyHandle<String, super::authored_retirement::records::RetirementOperation>,
    pub(crate) retired_draft_parents:
        StateFamilyHandle<String, super::authored_retirement::records::RetiredDraftParent>,
    pub(crate) filed_royalties: StateFamilyHandle<String, super::entitlement::FiledRoyaltyTermsV1>,
    pub(crate) term_economics: StateFamilyHandle<String, super::entitlement::PaidTermEconomicsV1>,
    pub(crate) house_allocations:
        StateFamilyHandle<String, super::house_purchase::HouseAllocationV1>,
    pub(crate) house_economics:
        StateFamilyHandle<String, super::house_purchase::HouseTermEconomicsV1>,
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
    /// 17g C2f: one row per held paid licence (`roll_mind_paid_holdings`,
    /// the C2b brief §2), delegated to the market as `paid_licences` is.
    pub(crate) paid_holdings: StateFamilyHandle<String, super::paid_holdings::PaidHolding>,
    /// M8c B5: one ring of seal marks per licence (`roll_licence_seal_marks`).
    pub(crate) seal_marks: StateFamilyHandle<String, super::seal_marks::SealMarkRing>,
}

impl CommerceFamilies {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        Ok(Self {
            conduct_jobs: super::conduct::families::register(reg)?,
            conduct_executions: super::conduct::families::register(reg)?,
            conduct_results: super::conduct::families::register(reg)?,
            coverage_publications: register(reg)?,
            coverages: register(reg)?,
            paid_resales: register(reg)?,
            paid_resale_heads: register(reg)?,
            paid_holders: register(reg)?,
            measured_recalls: register(reg)?,
            reconformance_heads: register(reg)?,
            charter_pending_offers: register(reg)?,
            charter_pending_buyer_offers: register(reg)?,
            charter_pending: register(reg)?,
            charter_pending_heads: register(reg)?,
            charter_transfer_offers: register(reg)?,
            charter_transfers: register(reg)?,
            charter_ownership: register(reg)?,
            lapse_occurrences: register(reg)?,
            ic_claims: register(reg)?,
            owner_qualification_operations: register(reg)?,
            owner_qualification_heads: register(reg)?,
            owner_attestations: register(reg)?,
            line_updates: register(reg)?,
            update_positions: register(reg)?,
            registry_recalls: register(reg)?,
            registry_inspections: register(reg)?,
            revoked_principals: register(reg)?,
            registry_seizures: register(reg)?,
            registry_revocations: register(reg)?,
            revoked_terms: register(reg)?,
            withdrawals: register(reg)?,
            house_renewal_operations: register(reg)?,
            house_renewal_heads: register(reg)?,
            renewal_operations: register(reg)?,
            renewal_heads: register(reg)?,
            authored_retirements: register(reg)?,
            retired_draft_parents: register(reg)?,
            filed_royalties: register(reg)?,
            seal_marks: register(reg)?,
            term_economics: register(reg)?,
            entitlement_allocations: register(reg)?,
            house_allocations: register(reg)?,
            house_economics: register(reg)?,
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
            paid_holdings: register(reg)?,
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

/// A person's id as the owner indexes key it: exactly twenty decimal
/// digits, nonzero and within `u32`.
fn person(text: &str) -> bool {
    text.len() == 20
        && text.bytes().all(|c| c.is_ascii_digit())
        && text
            .parse::<u64>()
            .is_ok_and(|n| n != 0 && u32::try_from(n).is_ok())
}

/// M8c B1: an organisation's id under the `org/` head of the holdings index:
/// exactly twenty decimal digits, nonzero (an `OrgId` is a `u64`).
/// M8c F8: any organisation number, 0 included (mush-org mints from 0).
fn org_number(text: &str) -> bool {
    text.len() == 20 && text.bytes().all(|c| c.is_ascii_digit()) && text.parse::<u64>().is_ok()
}

pub(crate) fn valid_key(family: &str, key: &str) -> bool {
    let mut parts = key.split('/');
    let head = parts.next().unwrap_or("");
    match family {
        "roll_mind_owner_parent" | "roll_mind_owner_qualification_head" => {
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
        "roll_mind_paid_resales"
        | "roll_mind_charter_pending_offers"
        | "roll_mind_charter_pending"
        | "roll_mind_charter_transfer_offers"
        | "roll_mind_ic_claim"
        | "roll_mind_coverage"
        | "roll_coverage_publication"
        | "roll_mind_line_update"
        | "roll_mind_initial_filing"
        | "roll_mind_paid_purchase"
        | "roll_mind_authored_retirement"
        | "roll_mind_author_withdrawal"
        | "roll_registry_revision_recalls"
        | "roll_registry_term_revocations"
        | "roll_registry_measured_recalls"
        | "roll_mind_owner_qualification_operation"
        | "roll_mind_owner_attestation_operation"
        | "roll_mind_entitlement_purchase"
        | "roll_mind_house_purchase"
        | "roll_house_mind_renewal_operation"
        | "roll_mind_renewal_operation" => hex(head, 32, false) && parts.next().is_none(),
        "roll_registry_reconformance_heads" | "roll_mind_filed_identity" => {
            hex(head, 64, false) && parts.next().is_none()
        }
        // M8c B5: one ring per licence, keyed by the licence alone.
        "roll_licence_seal_marks" => {
            head == "licence"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        "roll_registry_revoked_terms"
        | "roll_mind_lapse_occurrences"
        | "roll_mind_owner_term"
        | "roll_mind_paid_term"
        | "roll_mind_paid_term_economics"
        | "roll_mind_house_term_economics" => {
            head == "licence"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next() == Some("term")
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        // Doc 17h cut M8a fix round 1 (F9): a hull's 64-slot inspection ring,
        // `hull/<hull, 8 hex>/slot/<seq mod 64, 2 hex>`.
        "roll_registry_inspections" => {
            head == "hull"
                && parts.next().is_some_and(|v| hex(v, 8, true))
                && parts.next() == Some("slot")
                && parts.next().is_some_and(|v| {
                    hex(v, 2, false) && u8::from_str_radix(v, 16).is_ok_and(|n| n < 64)
                })
                && parts.next().is_none()
        }
        "roll_mind_line_update_position" => {
            head == "line"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16)
                            .is_ok_and(|n| n >= 2 && n <= crate::MAX_RECORDS as u64 + 1)
                })
                && parts.next().is_none()
        }
        "roll_mind_filed_position" => {
            head == "line"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next() == Some("0000000000000001")
                && parts.next().is_none()
        }
        // 17g C2f: the two owner indexes keyed by a zero-padded decimal
        // person first, so one person's rows are contiguous.
        // M8c B1: an organisation's rows are keyed `org/{org:020}/licence/...`
        // (`Holder::key_prefix`); the two heads never collide.
        // Doc 17h cut M8d D4: `{character:020}` or `org/{org:020}`.
        // Doc 17h cut M8d2: `hull/<8 hex>/inspection/<seq>`.
        "roll_registry_seizures" => {
            head == "hull"
                && parts.next().is_some_and(|v| hex(v, 8, true))
                && parts.next() == Some("inspection")
                && parts
                    .next()
                    .is_some_and(|v| v.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == v))
                && parts.next().is_none()
        }
        "roll_registry_revoked_principals" => {
            super::registry_inspection::principal::valid_key(key, person, org_number)
        }
        "roll_mind_paid_holdings" => {
            (person(head) || (head == "org" && parts.next().is_some_and(org_number)))
                && parts.next() == Some("licence")
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_none()
        }
        "roll_mind_charter_pending_buyer_offers" => {
            person(head)
                && parts.next() == Some("charter")
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16).is_ok_and(|n| u32::try_from(n).is_ok())
                })
                && parts.next().is_none()
        }
        "roll_mind_paid_holders" => {
            head == "licence"
                && parts.next().is_some_and(|v| hex(v, 16, true))
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16).is_ok_and(|n| (2..=65_537).contains(&n))
                })
                && parts.next().is_none()
        }
        "roll_mind_charter_transfers" => {
            head == "charter"
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16).is_ok_and(|n| u32::try_from(n).is_ok())
                })
                && parts.next().is_some_and(|v| {
                    hex(v, 16, true)
                        && u64::from_str_radix(v, 16).is_ok_and(|n| (2..=65_537).contains(&n))
                })
                && parts.next().is_none()
        }
        "roll_mind_charter" | "roll_mind_charter_ownership" | "roll_mind_charter_pending_head" => {
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
        | "roll_mind_paid_resale_heads"
        | "roll_house_mind_renewal_head"
        | "roll_mind_renewal_head" => {
            let prefix = match family {
                "roll_mind_line" => "line",
                "roll_mind_owner_licence"
                | "roll_mind_personal_paid_licence"
                | "roll_mind_paid_resale_heads"
                | "roll_house_mind_renewal_head"
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

#[cfg(test)]
mod tests {
    use super::valid_key;

    #[test]
    fn lapse_occurrences_are_keyed_by_licence_term() {
        let key = format!(
            "{}/{}",
            super::super::LicenceId::new(4).expect("licence").key(),
            super::super::TermGeneration::new(1).expect("term").key()
        );
        assert!(valid_key("roll_mind_lapse_occurrences", &key));
        assert!(!valid_key(
            "roll_mind_lapse_occurrences",
            "licence/0000000000000004"
        ));
    }
}
