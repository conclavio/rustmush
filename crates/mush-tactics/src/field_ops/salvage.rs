//! `H2-B` & `H2-C` — Recovery entitlement, ground tenure at attach, unlawful salvage
//! evidence, post-close black-box access rules, public salvage title claims, and the
//! 48-available-real-hour contest workflow (design doc 27 §4.2, §4.3, §5, §6 `H2-B`/`H2-C`,
//! `FO-3`, `FO-4`, `Q-S1`–`Q-S4`, `Q-S13`–`Q-S15`, `Q-F3`, `Q-F5`, `Q-F6`, `Q-R1`, `P-3`, `P-4`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::{ObjId, WorldEvent};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::OperationalState;
use crate::field_ops::recovery::RecoveryOpId;
use crate::unit::{ComponentId, SectionId};

/// Contest window duration in available real seconds (`48` hours = `172_800` seconds, `Q-S15`).
pub const CONTEST_WINDOW_AVAILABLE_REAL_SECS: i64 = 48 * 3_600;

/// Maximum authoritative recovery records retained in one [`SalvageClaimBook`].
pub const MAX_AUTHORITATIVE_RECOVERY_RECORDS: usize = 1_024;

/// Maximum salvage claims retained in one [`SalvageClaimBook`].
pub const MAX_SALVAGE_CLAIMS: usize = 1_024;

/// Maximum contests or at-risk repair entries on a single [`SalvageClaim`].
pub const MAX_CLAIM_ENTRIES: usize = 64;

/// Maximum forensic flight/combat recorder entries retained per hull (`P-3`, `Q-H6`, `Q-S4`).
pub const MAX_BLACK_BOX_ENTRIES_PER_HULL: usize = 256;

/// Recognized ground tenure at the casualty's position when physical attachment begins (`Q-S1`, `Q-S3`).
///
/// By `Q-S3`, this tenure is captured when physical securing/attach starts and persists
/// across a continuous recovery operation; it is re-evaluated only if the recovery is
/// interrupted/detached and restarted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tenure", rename_all = "snake_case", deny_unknown_fields)]
pub enum GroundTenure {
    /// Open or unclaimed frontier ground (`Q-S1`).
    Unclaimed,
    /// Ground subject to a recognized land claim or tenure (`Q-S1`, `Q-S3`).
    Owned {
        /// Recognized landholder (character, org, or faction object id).
        landholder: ObjId,
        /// Optional claim/deed identifier.
        claim_id: Option<SmolStr>,
    },
}

impl GroundTenure {
    fn validate(&self) -> Result<(), SalvageError> {
        if let Self::Owned { landholder, .. } = self
            && landholder.0 == 0
        {
            return Err(SalvageError::InvalidObjectId("landholder"));
        }
        Ok(())
    }
}

/// Authority context evaluated when physical recovery/attach begins (`FO-4`, `Q-S1`, `Q-S3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAuthorityContext {
    /// Actor/faction performing the recovery.
    pub recoverer: ObjId,
    /// Current legal title holder of the casualty hull.
    pub unit_title_holder: ObjId,
    /// Whether `recoverer` holds explicit recovery/use authorization from `unit_title_holder`.
    pub has_owner_authorization: bool,
    /// Whether `recoverer` holds explicit recovery authorization from the landholder (when on `Owned` ground).
    pub has_landholder_authorization: bool,
    /// Ground tenure at the casualty's position when physical attachment begins (`Q-S3`).
    pub ground_tenure_at_attach: GroundTenure,
}

/// Derived legal entitlement of a physical recovery operation (`FO-4`, `Q-S1`, `Q-S3`, `Q-S4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entitlement", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryEntitlement {
    /// Recoverer is the current legal unit title holder or their authorized agent (`Q-S1`).
    /// Lawful on both unclaimed and owned ground without needing landholder permission.
    LawfulSelfOrAgent {
        /// Legal unit title holder.
        unit_owner: ObjId,
    },
    /// On owned ground, recoverer is the recognized landholder or their authorized agent (`Q-S1`).
    /// Lawful without needing unit-owner permission.
    LawfulLandholderOrAgent {
        /// Recognized landholder.
        landholder: ObjId,
        /// Prior legal unit title holder.
        unit_owner: ObjId,
    },
    /// On unclaimed/open ground, any capable recoverer (`Q-S1`).
    LawfulOpenGround {
        /// Prior legal unit title holder.
        unit_owner: ObjId,
    },
    /// On owned ground, a third party with neither unit-owner nor landholder authority (`FO-4`, `Q-S4`).
    /// Physically permitted to attach, tow, and load (`Q-S5`), but legally contestable when
    /// proven by acquired evidence (`Q-S4`).
    UnlawfulOnOwnedGround {
        /// Recognized landholder whose tenure was violated.
        landholder: ObjId,
        /// Prior legal unit title holder.
        unit_owner: ObjId,
    },
}

impl RecoveryEntitlement {
    /// Whether this recovery is lawful under `Q-S1`.
    #[must_use]
    pub const fn is_lawful(&self) -> bool {
        !matches!(self, Self::UnlawfulOnOwnedGround { .. })
    }

    /// Whether this recovery is by the existing title holder or their authorized agent (`Q-S1`, `Q-S14`).
    #[must_use]
    pub const fn is_self_or_owner_agent(&self) -> bool {
        matches!(self, Self::LawfulSelfOrAgent { .. })
    }
}

/// Evaluate the [`RecoveryEntitlement`] at the moment physical attachment begins (`FO-4`, `Q-S1`, `Q-S3`).
///
/// # Errors
/// Returns [`SalvageError`] if any object id in `ctx` is `ObjId(0)`.
pub fn evaluate_recovery_entitlement(
    ctx: &RecoveryAuthorityContext,
) -> Result<RecoveryEntitlement, SalvageError> {
    if ctx.recoverer.0 == 0 {
        return Err(SalvageError::InvalidObjectId("recoverer"));
    }
    if ctx.unit_title_holder.0 == 0 {
        return Err(SalvageError::InvalidObjectId("unit_title_holder"));
    }
    ctx.ground_tenure_at_attach.validate()?;

    if ctx.recoverer == ctx.unit_title_holder || ctx.has_owner_authorization {
        return Ok(RecoveryEntitlement::LawfulSelfOrAgent {
            unit_owner: ctx.unit_title_holder,
        });
    }

    match &ctx.ground_tenure_at_attach {
        GroundTenure::Unclaimed => Ok(RecoveryEntitlement::LawfulOpenGround {
            unit_owner: ctx.unit_title_holder,
        }),
        GroundTenure::Owned { landholder, .. } => {
            if ctx.recoverer == *landholder || ctx.has_landholder_authorization {
                Ok(RecoveryEntitlement::LawfulLandholderOrAgent {
                    landholder: *landholder,
                    unit_owner: ctx.unit_title_holder,
                })
            } else {
                Ok(RecoveryEntitlement::UnlawfulOnOwnedGround {
                    landholder: *landholder,
                    unit_owner: ctx.unit_title_holder,
                })
            }
        }
    }
}

/// How an actor acquired actionable evidence of a recovery operation (`Q-S4`, `P-3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SalvageEvidenceKind {
    /// Direct visual/line-of-sight witness of the attach/tow/load operation (`Q-S4`).
    DirectWitness {
        /// Witnessing actor or unit.
        witness: ObjId,
        /// IC timestamp of observation.
        observed_at_ic: i64,
    },
    /// Recorded sensor or telemetry log from a unit on site (`Q-S4`).
    SensorLog {
        /// Sensor source unit.
        sensor_unit: ObjId,
        /// IC timestamp of sensor log.
        recorded_at_ic: i64,
    },
    /// Physical or administrative inspection of the hull in bay/hangar custody (`Q-S4`).
    CustodyInspection {
        /// Inspecting actor.
        inspector: ObjId,
        /// IC timestamp of inspection.
        inspected_at_ic: i64,
    },
    /// Recovered black-box flight/sortie recorder read from the hull (`P-3`, `Q-H6`, `Q-S4`).
    RecoveredBlackBox {
        /// Hull whose black box was read.
        hull: ObjId,
        /// IC timestamp when the black box was read.
        read_at_ic: i64,
    },
}

/// Verified evidence artifact binding an observer/challenger to a specific recovery (`Q-S4`, `P-3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SalvageEvidenceRecord {
    /// Recovery operation id evidenced.
    pub operation_id: RecoveryOpId,
    /// Casualty hull object id.
    pub casualty: ObjId,
    /// Actor holding this evidence.
    pub holder: ObjId,
    /// Evidence acquisition provenance.
    pub kind: SalvageEvidenceKind,
}

impl SalvageEvidenceRecord {
    fn validate_for(
        &self,
        operation_id: RecoveryOpId,
        casualty: ObjId,
    ) -> Result<(), SalvageError> {
        if self.holder.0 == 0 {
            return Err(SalvageError::InvalidObjectId("evidence_holder"));
        }
        if self.operation_id != operation_id || self.casualty != casualty {
            return Err(SalvageError::EvidenceMismatch {
                expected_op: operation_id,
                actual_op: self.operation_id,
                expected_casualty: casualty,
                actual_casualty: self.casualty,
            });
        }
        Ok(())
    }
}

/// Private authoritative record of a physical recovery operation (`Q-S3`, `Q-S4`, `P-3`).
///
/// Never broadcast publicly as a theft alert; only accessible to the recoverer or to an
/// actor who presents a valid [`SalvageEvidenceRecord`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritativeRecoveryRecord {
    /// Physical recovery operation id (`H2-A`).
    pub operation_id: RecoveryOpId,
    /// Casualty unit id.
    pub casualty: ObjId,
    /// Towing unit id.
    pub tug: ObjId,
    /// Recovering actor/faction id.
    pub recoverer: ObjId,
    /// Legal title holder of the casualty at attachment time.
    pub unit_title_holder: ObjId,
    /// Ground tenure pinned when physical attachment began (`Q-S3`).
    pub ground_tenure_at_attach: GroundTenure,
    /// Legal entitlement evaluated at attachment (`Q-S1`, `Q-S3`, `Q-S4`).
    pub entitlement: RecoveryEntitlement,
    /// IC timestamp when physical attachment began.
    pub attached_at_ic: i64,
    /// Carrier or base into which the casualty was loaded, once bay load completes (`P-3`).
    pub loaded_into_base: Option<ObjId>,
    /// IC timestamp when bay load completed and the hull epoch closed as `RecoveredToBase`.
    pub loaded_at_ic: Option<i64>,
}

/// Published deadline adjustment for a pending [`SalvageClaim`] when the world clock
/// resumes after a freeze (`Q-F3`, `P-4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustedSalvageClaimDeadline {
    /// Adjusted salvage claim id.
    pub claim_id: SalvageClaimId,
    /// Claimed hull or field-stripped component.
    pub subject: SalvageSubject,
    /// Recoverer / claimant object id.
    pub claimant: ObjId,
    /// Prior title holder object id.
    pub prior_owner: ObjId,
    /// Deadline in real-time UTC seconds before this outage adjustment.
    pub previous_deadline_real_s: i64,
    /// Adjusted deadline in real-time UTC seconds after adding the unavailable duration (`Q-F3`).
    pub adjusted_deadline_real_s: i64,
    /// Remaining available real seconds in the 48-hour contest window at unfreeze (`Q-F3`).
    pub remaining_available_real_s: i64,
    /// Cumulative real seconds paused across all outages for this claim (`Q-F3`).
    pub total_paused_real_s: i64,
}

/// Strongly typed identifier for a [`SalvageClaim`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct SalvageClaimId(u64);

impl SalvageClaimId {
    /// Construct a non-zero [`SalvageClaimId`].
    ///
    /// # Errors
    /// Returns [`SalvageError::ZeroClaimId`] if `raw == 0`.
    pub const fn new(raw: u64) -> Result<Self, SalvageError> {
        if raw == 0 {
            return Err(SalvageError::ZeroClaimId);
        }
        Ok(Self(raw))
    }

    /// Underlying numeric id (`>= 1`).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for SalvageClaimId {
    type Error = SalvageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SalvageClaimId> for u64 {
    fn from(id: SalvageClaimId) -> Self {
        id.0
    }
}

impl fmt::Display for SalvageClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "salvclaim-{}", self.0)
    }
}

/// Subject of a public salvage claim (`Q-S2`, `Q-F6`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "subject_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SalvageSubject {
    /// Whole recovered hull (`Q-S2`), excluding any components already field-stripped
    /// under separate provenance (`Q-F6`).
    Hull {
        /// Casualty unit object id.
        unit: ObjId,
    },
    /// Individual surviving component stripped in the field (`Q-F6`).
    StrippedComponent {
        /// Detached component record id.
        stripped_record_id: u64,
        /// Parent hull from which the component was stripped.
        parent_hull: ObjId,
        /// Component slot/instance id on the parent hull.
        component: ComponentId,
    },
}

/// Grounds on which a challenger may contest a pending [`SalvageClaim`] (`Q-S13`, `Q-R1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ground", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContestGround {
    /// Challenger asserts only their prior legal title to the unit (`Q-R1`).
    ///
    /// By explicit owner ruling `Q-R1`, bare prior title alone **never** defeats a
    /// lawful salvage claim (whether recovered on unclaimed ground or by a recognized
    /// landholder on owned ground).
    BarePriorTitle,
    /// Challenger proves with acquired [`SalvageEvidenceRecord`] that the recovery was
    /// unlawful (e.g. third party on owned ground without owner or landholder authority, `Q-S4`, `Q-S13`).
    UnlawfulRecovery {
        /// Acquired evidence proving the unlawful recovery.
        evidence: SalvageEvidenceRecord,
    },
    /// Challenger establishes a verified procedural or notice defect in the claim (`Q-S13`).
    DefectiveProcedure {
        /// Canonical defect code.
        defect_code: SmolStr,
    },
    /// Challenger establishes a superior specific right (such as an active recovery/bailee
    /// contract or agency duty binding the claimant, `Q-S13`, `Q-R1`).
    SuperiorSpecificRight {
        /// Contract or obligation identifier.
        contract_id: SmolStr,
        /// Obligor bound by the specific right (must match the claimant).
        obligor: ObjId,
    },
}

/// Reason a contest succeeded in vacating a [`SalvageClaim`] (`Q-S13`, `Q-R1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContestSuccessReason {
    /// Recovery was unlawful on owned ground and proven with acquired evidence (`Q-S4`, `Q-S13`).
    ProvenUnlawfulRecovery,
    /// Claim notice or filing procedure was defective (`Q-S13`).
    DefectiveProcedure,
    /// Challenger held a superior specific contractual/agency right binding the claimant (`Q-S13`, `Q-R1`).
    SuperiorSpecificRight,
}

/// Recorded contest filed against a [`SalvageClaim`] during the 48-hour window (`Q-S13`, `Q-R1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SalvageContest {
    /// Challenging actor (e.g. prior unit owner, landholder, or contract principal).
    pub challenger: ObjId,
    /// Ground asserted for the contest.
    pub ground: ContestGround,
    /// Real-time UTC second when the contest was submitted.
    pub filed_at_real_s: i64,
    /// Whether this contest ground is legally valid to defeat the claim (`Q-S13`, `Q-R1`).
    pub valid_to_vacate: Option<ContestSuccessReason>,
}

/// Documented stabilization or repair work performed by a pending salvage custodian at
/// their own risk and expense (`Q-S14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtRiskRepairEntry {
    /// Summary of the stabilization or repair work performed.
    pub description: SmolStr,
    /// Resource or credit expenditure incurred at risk.
    pub cost_incurred: u64,
    /// IC timestamp when the repair was logged.
    pub performed_at_ic: i64,
}

/// Lifecycle state of a [`SalvageClaim`] (`Q-S2`, `Q-S13`–`Q-S15`, `Q-R1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SalvageClaimState {
    /// Open within the 48-available-real-hour contest window (`Q-S15`).
    Open,
    /// One or more contests have been submitted; awaiting adjudication/settlement.
    Contested,
    /// Claim succeeded (uncontested at window expiry or all contests rejected); clean
    /// legal title transferred to `claimant` (`Q-S2`, `Q-S15`, `Q-R1`).
    AwardedToClaimant {
        /// New legal owner (`claimant`).
        new_owner: ObjId,
        /// Real-time UTC second when title transferred.
        settled_at_real_s: i64,
    },
    /// Claim vacated due to a valid contest; right of custody/title restored to
    /// `restored_to` without reimbursing `documented_repairs_at_risk` (`Q-S13`, `Q-S14`).
    Vacated {
        /// Actor to whom title/custody right is restored.
        restored_to: ObjId,
        /// Reason the contest succeeded.
        reason: ContestSuccessReason,
        /// Real-time UTC second when vacated.
        settled_at_real_s: i64,
    },
}

impl SalvageClaimState {
    /// Whether the claim is still pending (`Open` or `Contested`).
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Open | Self::Contested)
    }
}

/// Public salvage claim with a 48-available-real-hour contest window (`Q-S2`, `Q-S13`–`Q-S15`, `Q-F3`, `Q-R1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SalvageClaim {
    /// Claim id.
    pub id: SalvageClaimId,
    /// Hull or field-stripped component claimed.
    pub subject: SalvageSubject,
    /// Components already stripped/claimed separately and therefore excluded from this hull claim (`Q-F6`).
    pub excluded_components: BTreeSet<ComponentId>,
    /// Recoverer filing the claim.
    pub claimant: ObjId,
    /// Prior legal title holder at the time of recovery.
    pub prior_owner: ObjId,
    /// Authoritative recovery operation id (`H2-A` / `H2-B`).
    pub operation_id: RecoveryOpId,
    /// Entitlement captured at physical attachment (`Q-S1`, `Q-S3`).
    pub entitlement_at_attach: RecoveryEntitlement,
    /// Ground tenure captured at physical attachment (`Q-S3`).
    pub ground_tenure_at_attach: GroundTenure,
    /// Real-time UTC second when the claim was filed.
    pub filed_at_real_s: i64,
    /// Real-time UTC second when the 48-available-real-hour contest window expires (`Q-S15`, `Q-F3`).
    pub deadline_real_s: i64,
    /// If a global world outage freeze is active, the real-time UTC second when the window paused (`Q-F3`).
    pub paused_at_real_s: Option<i64>,
    /// Cumulative real seconds paused across world outages (`Q-F3`).
    pub total_paused_real_s: i64,
    /// Contests submitted against this claim.
    pub contests: Vec<SalvageContest>,
    /// Documented repairs performed at the custodian's own risk while title is pending (`Q-S14`).
    pub documented_repairs_at_risk: Vec<AtRiskRepairEntry>,
    /// Current lifecycle state of the claim.
    pub state: SalvageClaimState,
}

/// Custody and title status of a recovered hull or component (`Q-S1`, `Q-S2`, `Q-S14`, `Q-F5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "custody", rename_all = "snake_case", deny_unknown_fields)]
pub enum CustodyTitleStatus {
    /// Held under resolved legal title (either self-recovered by existing owner or awarded
    /// after a completed salvage claim).
    ResolvedTitle {
        /// Legal title holder.
        owner: ObjId,
    },
    /// Held in physical custody while a [`SalvageClaim`] is pending (`Q-S14`, `Q-F5`).
    PendingSalvageClaim {
        /// Pending salvage claim id.
        claim_id: SalvageClaimId,
        /// Custodian holding the unit.
        custodian: ObjId,
        /// Prior legal owner whose title is being claimed.
        prior_owner: ObjId,
        /// Contest window deadline in real-time UTC seconds.
        deadline_real_s: i64,
    },
}

/// Actions a custodian may attempt on a recovered unit in bay/hangar custody (`Q-S14`, `Q-F5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustodyAction {
    /// Non-destructive stabilization in the bay (`Q-S14`).
    Stabilize,
    /// Documented workshop repair at the custodian's own risk and expense (`Q-S14`).
    DocumentedRepairAtRisk,
    /// Launching/deploying the unit on a new sortie (`Q-S14`).
    DeploySortie,
    /// Selling or transferring legal title (`Q-S14`).
    SellOrTransfer,
    /// Issuing onward use/pilot grants (`Q-S14`).
    IssueUseGrant,
    /// Destructively stripping a component from the hull (`Q-F5`).
    StripComponent,
    /// Destructively scrapping/dismantling the hull (`Q-F5`).
    ScrapHull,
}

/// Enforce the pending-title custody gate (`Q-S14`, `Q-F5`).
///
/// While [`CustodyTitleStatus::PendingSalvageClaim`] is active, `Stabilize` and
/// `DocumentedRepairAtRisk` are allowed, whereas `DeploySortie`, `SellOrTransfer`,
/// `IssueUseGrant`, `StripComponent`, and `ScrapHull` are refused unless the existing
/// title owner has granted explicit permission.
///
/// # Errors
/// Returns [`SalvageError::ActionProhibitedWhileTitlePending`] if the action is
/// barred during pending salvage custody.
pub fn check_custody_action_allowed(
    status: &CustodyTitleStatus,
    action: CustodyAction,
    has_explicit_owner_permission: bool,
) -> Result<(), SalvageError> {
    match status {
        CustodyTitleStatus::ResolvedTitle { .. } => Ok(()),
        CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } => match action {
            CustodyAction::Stabilize | CustodyAction::DocumentedRepairAtRisk => Ok(()),
            CustodyAction::DeploySortie
            | CustodyAction::SellOrTransfer
            | CustodyAction::IssueUseGrant
            | CustodyAction::StripComponent
            | CustodyAction::ScrapHull => {
                if has_explicit_owner_permission {
                    Ok(())
                } else {
                    Err(SalvageError::ActionProhibitedWhileTitlePending {
                        claim_id: *claim_id,
                        action,
                    })
                }
            }
        },
    }
}

/// Structured forensic entry recorded in a hull's flight/combat black box (`P-3`, `Q-H6`, `Q-H9`, `Q-S4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BlackBoxFlightEntry {
    /// Hull epoch opened on dispatch (`D-23-1`, `Q-H9`).
    EpochOpened {
        /// Sortie/epoch identifier.
        sortie_id: SmolStr,
        /// Site or port where the epoch opened.
        host_site: SmolStr,
        /// Operator / warrant principal, if any.
        operator: Option<ObjId>,
        /// Sealed mind label (`Standard Doctrine` or licensed profile).
        mind_label: SmolStr,
        /// Whether the core ran under governor constraints.
        governed: bool,
        /// IC timestamp when the epoch opened.
        at_ic: i64,
    },
    /// Causal section damage or operational disablement recorded during field combat (`Q-H6`–`Q-H9`).
    CombatDamageRecorded {
        /// Attacker unit id, if attributed by onboard sensors/telemetry.
        attacker: Option<ObjId>,
        /// Damaged or destroyed section id.
        section: SectionId,
        /// Resulting operational state of the hull.
        resulting_state: OperationalState,
        /// IC timestamp of the impact.
        at_ic: i64,
    },
    /// Physical tow attachment recorded by hull telemetry (`Q-S1`, `Q-S3`, `Q-S4`).
    TowAttached {
        /// Physical recovery operation id.
        operation_id: RecoveryOpId,
        /// Towing unit id.
        tug: ObjId,
        /// Recoverer actor/faction id.
        recoverer: ObjId,
        /// Ground tenure captured at attachment (`Q-S3`).
        ground_tenure: GroundTenure,
        /// Legal entitlement evaluated at attachment (`Q-S1`).
        entitlement: RecoveryEntitlement,
        /// IC timestamp when tow attachment completed.
        at_ic: i64,
    },
    /// Component detached from the hull in the field (`Q-S11`, `Q-F6`).
    FieldComponentStripped {
        /// Stripped component slot/instance id.
        component: ComponentId,
        /// Actor who detached the component.
        stripped_by: ObjId,
        /// Whether the field strip was lawful at the hull's site position.
        lawful: bool,
        /// IC timestamp when the component was stripped.
        at_ic: i64,
    },
    /// Hull epoch closed upon bay recovery (`RecoveredToBase`, `P-3`, `Q-H9`).
    EpochClosedInBay {
        /// Recovery operation id that delivered the hull, if towed in.
        operation_id: Option<RecoveryOpId>,
        /// Carrier or base object id where the epoch closed.
        base: ObjId,
        /// IC timestamp when bay load closed the epoch.
        at_ic: i64,
    },
}

/// Complete forensic extraction report produced when an actor with physical bay access
/// inspects a recovered hull's closed black box (`P-3`, `Q-H6`, `Q-S4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlackBoxForensicReport {
    /// Casualty hull object id.
    pub hull: ObjId,
    /// Recovery operation id.
    pub operation_id: RecoveryOpId,
    /// Carrier or base where the hull epoch closed (`P-3`).
    pub closed_in_base: ObjId,
    /// Chronological flight/combat/tow/strip entries extracted from the hull's recorder.
    pub entries: Vec<BlackBoxFlightEntry>,
    /// Verifiable [`SalvageEvidenceRecord`] (`SalvageEvidenceKind::RecoveredBlackBox`) minted
    /// for `reader` by this physical extraction (`Q-S4`, `P-3`).
    pub minted_evidence: SalvageEvidenceRecord,
    /// Whether the recorder tape proves that `operation_id` was an `UnlawfulOnOwnedGround` recovery.
    pub reveals_unlawful_recovery: bool,
    /// Components recorded on the tape as unlawfully field-stripped from this hull.
    pub unlawful_stripped_components: Vec<ComponentId>,
}

/// Ledger managing private authoritative recovery records, evidence-gated inspection,
/// post-close black-box access, public salvage claims, outage freeze extensions, and
/// 48-hour contest adjudication (`H2-B`, `H2-C`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SalvageClaimBook {
    next_claim_id: u64,
    recovery_records: BTreeMap<RecoveryOpId, AuthoritativeRecoveryRecord>,
    claims: BTreeMap<SalvageClaimId, SalvageClaim>,
    stripped_components_by_hull: BTreeMap<ObjId, BTreeSet<ComponentId>>,
    #[serde(default)]
    black_box_tapes: BTreeMap<ObjId, Vec<BlackBoxFlightEntry>>,
}

impl SalvageClaimBook {
    /// Construct an empty [`SalvageClaimBook`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_claim_id: 1,
            recovery_records: BTreeMap::new(),
            claims: BTreeMap::new(),
            stripped_components_by_hull: BTreeMap::new(),
            black_box_tapes: BTreeMap::new(),
        }
    }

    /// Append a [`BlackBoxFlightEntry`] to `hull`'s onboard flight/combat recorder (`P-3`, `Q-H6`, `Q-H9`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if `hull.0 == 0` or `hull`'s tape has reached [`MAX_BLACK_BOX_ENTRIES_PER_HULL`].
    pub fn append_black_box_entry(
        &mut self,
        hull: ObjId,
        entry: BlackBoxFlightEntry,
    ) -> Result<(), SalvageError> {
        if hull.0 == 0 {
            return Err(SalvageError::InvalidObjectId("hull"));
        }
        let tape = self.black_box_tapes.entry(hull).or_default();
        if tape.len() >= MAX_BLACK_BOX_ENTRIES_PER_HULL {
            return Err(SalvageError::BlackBoxTapeFull {
                hull,
                max: MAX_BLACK_BOX_ENTRIES_PER_HULL,
            });
        }
        tape.push(entry);
        Ok(())
    }

    /// Recorded [`BlackBoxFlightEntry`] log for `hull`, if any.
    #[must_use]
    pub fn black_box_tape_for_hull(&self, hull: ObjId) -> &[BlackBoxFlightEntry] {
        self.black_box_tapes.get(&hull).map_or(&[], Vec::as_slice)
    }

    /// Record the private authoritative recovery record when physical attachment begins (`Q-S1`, `Q-S3`, `Q-S4`).
    ///
    /// Note that `UnlawfulOnOwnedGround` does **not** block physical attachment (`FO-4`, `Q-S5`);
    /// it is recorded privately here and can be contested only if an actor acquires a
    /// [`SalvageEvidenceRecord`] (`Q-S4`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if IDs are invalid, the operation is already recorded,
    /// or capacity is exceeded.
    pub fn record_physical_attach(
        &mut self,
        operation_id: RecoveryOpId,
        casualty: ObjId,
        tug: ObjId,
        authority_ctx: &RecoveryAuthorityContext,
        attached_at_ic: i64,
    ) -> Result<&AuthoritativeRecoveryRecord, SalvageError> {
        if casualty.0 == 0 {
            return Err(SalvageError::InvalidObjectId("casualty"));
        }
        if tug.0 == 0 {
            return Err(SalvageError::InvalidObjectId("tug"));
        }
        if self.recovery_records.contains_key(&operation_id) {
            return Err(SalvageError::DuplicateRecoveryRecord(operation_id));
        }
        if self.recovery_records.len() >= MAX_AUTHORITATIVE_RECOVERY_RECORDS {
            return Err(SalvageError::MaxRecoveryRecordsReached {
                max: MAX_AUTHORITATIVE_RECOVERY_RECORDS,
            });
        }

        let entitlement = evaluate_recovery_entitlement(authority_ctx)?;
        let record = AuthoritativeRecoveryRecord {
            operation_id,
            casualty,
            tug,
            recoverer: authority_ctx.recoverer,
            unit_title_holder: authority_ctx.unit_title_holder,
            ground_tenure_at_attach: authority_ctx.ground_tenure_at_attach.clone(),
            entitlement: entitlement.clone(),
            attached_at_ic,
            loaded_into_base: None,
            loaded_at_ic: None,
        };
        let tape = self.black_box_tapes.entry(casualty).or_default();
        if tape.len() < MAX_BLACK_BOX_ENTRIES_PER_HULL {
            tape.push(BlackBoxFlightEntry::TowAttached {
                operation_id,
                tug,
                recoverer: authority_ctx.recoverer,
                ground_tenure: authority_ctx.ground_tenure_at_attach.clone(),
                entitlement,
                at_ic: attached_at_ic,
            });
        }
        self.recovery_records.insert(operation_id, record);
        self.recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))
    }

    /// Mark a recorded recovery operation as loaded into a carrier bay or base (`P-3`, `Q-S8`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if `operation_id` is unknown or `base.0 == 0`.
    pub fn record_bay_load(
        &mut self,
        operation_id: RecoveryOpId,
        base: ObjId,
        loaded_at_ic: i64,
    ) -> Result<&AuthoritativeRecoveryRecord, SalvageError> {
        if base.0 == 0 {
            return Err(SalvageError::InvalidObjectId("base"));
        }
        let rec = self
            .recovery_records
            .get_mut(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))?;
        let casualty = rec.casualty;
        rec.loaded_into_base = Some(base);
        rec.loaded_at_ic = Some(loaded_at_ic);
        let tape = self.black_box_tapes.entry(casualty).or_default();
        if tape.len() < MAX_BLACK_BOX_ENTRIES_PER_HULL {
            tape.push(BlackBoxFlightEntry::EpochClosedInBay {
                operation_id: Some(operation_id),
                base,
                at_ic: loaded_at_ic,
            });
        }
        self.recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))
    }

    /// Inspect a private [`AuthoritativeRecoveryRecord`] using acquired evidence (`Q-S4`).
    ///
    /// - The `recoverer` themselves can always read their own operation record.
    /// - Any other actor (including the prior unit owner or landholder) must supply a
    ///   matching [`SalvageEvidenceRecord`] (`DirectWitness`, `SensorLog`, `CustodyInspection`,
    ///   or `RecoveredBlackBox`). Without evidence, access is refused without leaking
    ///   the hidden recoverer or owner (`Q-S4`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the operation is unknown or `actor` lacks evidence.
    pub fn inspect_recovery_record_with_evidence(
        &self,
        operation_id: RecoveryOpId,
        actor: ObjId,
        evidence: Option<&SalvageEvidenceRecord>,
    ) -> Result<&AuthoritativeRecoveryRecord, SalvageError> {
        if actor.0 == 0 {
            return Err(SalvageError::InvalidObjectId("actor"));
        }
        let rec = self
            .recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))?;

        if actor == rec.recoverer {
            return Ok(rec);
        }
        let ev = evidence.ok_or(SalvageError::EvidenceRequiredForUnlawfulAction {
            operation_id,
            casualty: rec.casualty,
        })?;
        ev.validate_for(operation_id, rec.casualty)?;
        if ev.holder != actor {
            return Err(SalvageError::EvidenceHolderMismatch {
                holder: ev.holder,
                actor,
            });
        }
        Ok(rec)
    }

    /// Read the black box of a casualty after carrier bay recovery (`P-3`, `Q-H6`, `Q-S4`).
    ///
    /// By `P-3`, loading a casualty into a non-owner's carrier bay closes the hull epoch
    /// (`RecoveredToBase`), and black-box read access follows `Q-H6`/`Q-S4` evidence rules:
    /// an actor must have physical/custody inspection access to the bay holding the hull
    /// (`has_physical_bay_access == true`); a distant prior owner without physical access
    /// cannot remotely read the black box just because the epoch closed.
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the hull has not been loaded into a bay or `reader`
    /// lacks physical/custody access to the bay.
    pub fn read_recovered_black_box(
        &self,
        operation_id: RecoveryOpId,
        reader: ObjId,
        has_physical_bay_access: bool,
        read_at_ic: i64,
    ) -> Result<SalvageEvidenceRecord, SalvageError> {
        if reader.0 == 0 {
            return Err(SalvageError::InvalidObjectId("reader"));
        }
        let rec = self
            .recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))?;

        if rec.loaded_into_base.is_none() {
            return Err(SalvageError::CasualtyNotYetLoadedInBay(rec.casualty));
        }
        if !has_physical_bay_access {
            return Err(SalvageError::BlackBoxPhysicalAccessRequired {
                casualty: rec.casualty,
                reader,
            });
        }

        Ok(SalvageEvidenceRecord {
            operation_id,
            casualty: rec.casualty,
            holder: reader,
            kind: SalvageEvidenceKind::RecoveredBlackBox {
                hull: rec.casualty,
                read_at_ic,
            },
        })
    }

    /// Extract a full [`BlackBoxForensicReport`] from a recovered hull's closed black box (`P-3`, `Q-H6`, `Q-S4`).
    ///
    /// Requires the hull to be loaded in a bay (`loaded_into_base.is_some()`) and `reader`
    /// to hold physical/custody access to that bay (`has_physical_bay_access == true`).
    /// Returns the chronological recorder tape, flags any unlawful recovery or unlawful
    /// field component stripping recorded on the tape, and mints a verifiable
    /// [`SalvageEvidenceRecord`] (`SalvageEvidenceKind::RecoveredBlackBox`) for `reader`.
    ///
    /// # Errors
    /// Returns [`SalvageError`] if `operation_id` is unknown, the hull has not been loaded
    /// into a bay, or `reader` lacks physical bay access.
    pub fn extract_black_box_forensics(
        &self,
        operation_id: RecoveryOpId,
        reader: ObjId,
        has_physical_bay_access: bool,
        read_at_ic: i64,
    ) -> Result<BlackBoxForensicReport, SalvageError> {
        let minted_evidence = self.read_recovered_black_box(
            operation_id,
            reader,
            has_physical_bay_access,
            read_at_ic,
        )?;
        let rec = self
            .recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))?;
        let closed_in_base = rec
            .loaded_into_base
            .ok_or(SalvageError::CasualtyNotYetLoadedInBay(rec.casualty))?;
        let entries = self.black_box_tape_for_hull(rec.casualty).to_vec();

        let reveals_unlawful_recovery = !rec.entitlement.is_lawful()
            || entries.iter().any(|entry| {
                matches!(
                    entry,
                    BlackBoxFlightEntry::TowAttached {
                        operation_id: op,
                        entitlement,
                        ..
                    } if *op == operation_id && !entitlement.is_lawful()
                )
            });

        let unlawful_stripped_components = entries
            .iter()
            .filter_map(|entry| match entry {
                BlackBoxFlightEntry::FieldComponentStripped {
                    component,
                    lawful: false,
                    ..
                } => Some(component.clone()),
                _ => None,
            })
            .collect();

        Ok(BlackBoxForensicReport {
            hull: rec.casualty,
            operation_id,
            closed_in_base,
            entries,
            minted_evidence,
            reveals_unlawful_recovery,
            unlawful_stripped_components,
        })
    }

    /// Register that a component was stripped from `hull` (`Q-F6`), preventing a subsequent
    /// hull salvage claim from double-claiming the removed component.
    pub fn mark_component_stripped(&mut self, hull: ObjId, component: ComponentId) {
        self.stripped_components_by_hull
            .entry(hull)
            .or_default()
            .insert(component);
    }

    /// Components already stripped from `hull` (`Q-F6`).
    #[must_use]
    pub fn stripped_components_for_hull(&self, hull: ObjId) -> Option<&BTreeSet<ComponentId>> {
        self.stripped_components_by_hull.get(&hull)
    }

    /// Resolve custody and title after a casualty is recovered into a bay (`Q-S1`, `Q-S2`, `Q-S14`, `Q-S15`).
    ///
    /// - **Self-recovery (`LawfulSelfOrAgent`)**: immediately restores [`CustodyTitleStatus::ResolvedTitle`]
    ///   to `unit_title_holder` with **no** salvage claim or 48-hour contest window (`Q-S1`, `Q-S14`).
    /// - **Non-owner recovery**: opens a public [`SalvageClaim`] with a `48`-available-real-hour
    ///   contest window (`CONTEST_WINDOW_AVAILABLE_REAL_SECS`, `Q-S15`) and returns
    ///   [`CustodyTitleStatus::PendingSalvageClaim`].
    ///
    /// # Errors
    /// Returns [`SalvageError`] if `operation_id` is unknown, the hull was not yet loaded
    /// into a bay, or a pending/awarded hull claim already exists for this casualty.
    pub fn resolve_hull_recovery_custody(
        &mut self,
        operation_id: RecoveryOpId,
        filed_at_real_s: i64,
    ) -> Result<CustodyTitleStatus, SalvageError> {
        let rec = self
            .recovery_records
            .get(&operation_id)
            .ok_or(SalvageError::UnknownRecoveryOperation(operation_id))?
            .clone();

        if rec.loaded_into_base.is_none() {
            return Err(SalvageError::CasualtyNotYetLoadedInBay(rec.casualty));
        }

        if rec.entitlement.is_self_or_owner_agent() {
            return Ok(CustodyTitleStatus::ResolvedTitle {
                owner: rec.unit_title_holder,
            });
        }

        let subject = SalvageSubject::Hull { unit: rec.casualty };
        if self
            .claims
            .values()
            .any(|c| c.subject == subject && !matches!(c.state, SalvageClaimState::Vacated { .. }))
        {
            return Err(SalvageError::DuplicateActiveClaimForSubject(subject));
        }
        if self.claims.len() >= MAX_SALVAGE_CLAIMS {
            return Err(SalvageError::MaxClaimsReached {
                max: MAX_SALVAGE_CLAIMS,
            });
        }

        let claim_id = SalvageClaimId::new(self.next_claim_id)?;
        self.next_claim_id = self
            .next_claim_id
            .checked_add(1)
            .ok_or(SalvageError::ClaimIdOverflow)?;

        let deadline_real_s = filed_at_real_s.saturating_add(CONTEST_WINDOW_AVAILABLE_REAL_SECS);
        let excluded_components = self
            .stripped_components_by_hull
            .get(&rec.casualty)
            .cloned()
            .unwrap_or_default();

        let claim = SalvageClaim {
            id: claim_id,
            subject,
            excluded_components,
            claimant: rec.recoverer,
            prior_owner: rec.unit_title_holder,
            operation_id,
            entitlement_at_attach: rec.entitlement,
            ground_tenure_at_attach: rec.ground_tenure_at_attach,
            filed_at_real_s,
            deadline_real_s,
            paused_at_real_s: None,
            total_paused_real_s: 0,
            contests: Vec::new(),
            documented_repairs_at_risk: Vec::new(),
            state: SalvageClaimState::Open,
        };
        self.claims.insert(claim_id, claim);

        Ok(CustodyTitleStatus::PendingSalvageClaim {
            claim_id,
            custodian: rec.recoverer,
            prior_owner: rec.unit_title_holder,
            deadline_real_s,
        })
    }

    /// File a part-scoped salvage claim for an authorized field-stripped component (`Q-F6`).
    ///
    /// Also records the component in `stripped_components_by_hull` so the remaining
    /// chassis cannot double-claim or consume the removed part (`Q-F6`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the field strip was unlawful on owned ground, or if the
    /// component was already claimed.
    pub fn file_part_salvage_claim(
        &mut self,
        stripped_record_id: u64,
        parent_hull: ObjId,
        component: ComponentId,
        operation_id: RecoveryOpId,
        authority_ctx: &RecoveryAuthorityContext,
        filed_at_real_s: i64,
    ) -> Result<CustodyTitleStatus, SalvageError> {
        if stripped_record_id == 0 || parent_hull.0 == 0 {
            return Err(SalvageError::InvalidObjectId("stripped_part"));
        }
        let entitlement = evaluate_recovery_entitlement(authority_ctx)?;
        if !entitlement.is_lawful() {
            return Err(SalvageError::UnlawfulFieldStripCannotClaim {
                hull: parent_hull,
                component,
            });
        }

        self.mark_component_stripped(parent_hull, component.clone());

        if entitlement.is_self_or_owner_agent() {
            return Ok(CustodyTitleStatus::ResolvedTitle {
                owner: authority_ctx.unit_title_holder,
            });
        }

        let subject = SalvageSubject::StrippedComponent {
            stripped_record_id,
            parent_hull,
            component: component.clone(),
        };
        if self.claims.values().any(|c| match &c.subject {
            SalvageSubject::StrippedComponent {
                parent_hull: h,
                component: comp,
                ..
            } => {
                *h == parent_hull
                    && *comp == component
                    && !matches!(c.state, SalvageClaimState::Vacated { .. })
            }
            SalvageSubject::Hull { .. } => false,
        }) {
            return Err(SalvageError::DuplicateActiveClaimForSubject(subject));
        }
        if self.claims.len() >= MAX_SALVAGE_CLAIMS {
            return Err(SalvageError::MaxClaimsReached {
                max: MAX_SALVAGE_CLAIMS,
            });
        }

        let claim_id = SalvageClaimId::new(self.next_claim_id)?;
        self.next_claim_id = self
            .next_claim_id
            .checked_add(1)
            .ok_or(SalvageError::ClaimIdOverflow)?;

        let deadline_real_s = filed_at_real_s.saturating_add(CONTEST_WINDOW_AVAILABLE_REAL_SECS);
        let claim = SalvageClaim {
            id: claim_id,
            subject,
            excluded_components: BTreeSet::new(),
            claimant: authority_ctx.recoverer,
            prior_owner: authority_ctx.unit_title_holder,
            operation_id,
            entitlement_at_attach: entitlement,
            ground_tenure_at_attach: authority_ctx.ground_tenure_at_attach.clone(),
            filed_at_real_s,
            deadline_real_s,
            paused_at_real_s: None,
            total_paused_real_s: 0,
            contests: Vec::new(),
            documented_repairs_at_risk: Vec::new(),
            state: SalvageClaimState::Open,
        };
        self.claims.insert(claim_id, claim);

        Ok(CustodyTitleStatus::PendingSalvageClaim {
            claim_id,
            custodian: authority_ctx.recoverer,
            prior_owner: authority_ctx.unit_title_holder,
            deadline_real_s,
        })
    }

    /// Look up a [`SalvageClaim`] by id.
    #[must_use]
    pub fn claim(&self, id: SalvageClaimId) -> Option<&SalvageClaim> {
        self.claims.get(&id)
    }

    /// Record a documented repair-at-risk on a pending salvage claim (`Q-S14`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the claim is not pending or entry limits are reached.
    pub fn record_repair_at_risk(
        &mut self,
        claim_id: SalvageClaimId,
        description: SmolStr,
        cost_incurred: u64,
        performed_at_ic: i64,
    ) -> Result<&SalvageClaim, SalvageError> {
        let claim = self
            .claims
            .get_mut(&claim_id)
            .ok_or(SalvageError::UnknownClaim(claim_id))?;
        if !claim.state.is_pending() {
            return Err(SalvageError::ClaimAlreadySettled(claim_id));
        }
        if claim.documented_repairs_at_risk.len() >= MAX_CLAIM_ENTRIES {
            return Err(SalvageError::MaxClaimEntriesReached {
                claim_id,
                max: MAX_CLAIM_ENTRIES,
            });
        }
        claim.documented_repairs_at_risk.push(AtRiskRepairEntry {
            description,
            cost_incurred,
            performed_at_ic,
        });
        Ok(claim)
    }

    /// Pause all pending salvage claim contest windows when a global world outage freeze
    /// begins (`Q-F3`, `P-4`).
    pub fn pause_claims_for_outage(&mut self, freeze_at_real_s: i64) {
        for claim in self.claims.values_mut() {
            if claim.state.is_pending() && claim.paused_at_real_s.is_none() {
                claim.paused_at_real_s = Some(freeze_at_real_s);
            }
        }
    }

    /// Resume all paused salvage claim contest windows after a global outage freeze and
    /// 5-minute countdown complete, extending each claim's `deadline_real_s` by the
    /// exact frozen real duration and returning the published deadline adjustments (`Q-F3`, `P-4`).
    pub fn resume_claims_after_outage(
        &mut self,
        unfreeze_at_real_s: i64,
    ) -> Vec<AdjustedSalvageClaimDeadline> {
        let mut adjusted = Vec::new();
        for claim in self.claims.values_mut() {
            if claim.state.is_pending()
                && let Some(paused_at) = claim.paused_at_real_s.take()
            {
                let previous_deadline_real_s = claim.deadline_real_s;
                let delta = unfreeze_at_real_s.saturating_sub(paused_at).max(0);
                claim.total_paused_real_s = claim.total_paused_real_s.saturating_add(delta);
                claim.deadline_real_s = claim.deadline_real_s.saturating_add(delta);
                let remaining_available_real_s = claim
                    .deadline_real_s
                    .saturating_sub(unfreeze_at_real_s)
                    .max(0);
                adjusted.push(AdjustedSalvageClaimDeadline {
                    claim_id: claim.id,
                    subject: claim.subject.clone(),
                    claimant: claim.claimant,
                    prior_owner: claim.prior_owner,
                    previous_deadline_real_s,
                    adjusted_deadline_real_s: claim.deadline_real_s,
                    remaining_available_real_s,
                    total_paused_real_s: claim.total_paused_real_s,
                });
            }
        }
        adjusted
    }

    /// Apply one kernel `W-clock` event to the pending contest windows (`Q-F3`, `P-4`): the
    /// consumer a module holding this book subscribes to `WorldClockFrozen` and
    /// `WorldClockResumed`, the way the economy extends its real-time contract deadlines.
    ///
    /// - [`WorldEvent::WorldClockFrozen`] pauses every pending claim at the event's
    ///   `real_unix` (the pre-outage checkpoint for an outage freeze, the command's second for
    ///   a deliberate one) via [`Self::pause_claims_for_outage`].
    /// - [`WorldEvent::WorldClockResumed`] resumes them at `resumed_real_unix` via
    ///   [`Self::resume_claims_after_outage`], extending each deadline by the time paused. A
    ///   pending claim the freeze event never reached is paused at the event's
    ///   `frozen_real_unix` first, so it is extended by the same unavailable time the economy
    ///   applies to a window it finds open at the freeze.
    /// - Any other event changes nothing.
    ///
    /// Returns the published adjustments (empty for a freeze).
    pub fn apply_world_clock_event(
        &mut self,
        event: &WorldEvent,
    ) -> Vec<AdjustedSalvageClaimDeadline> {
        match event {
            WorldEvent::WorldClockFrozen { real_unix, .. } => {
                self.pause_claims_for_outage(*real_unix);
                Vec::new()
            }
            WorldEvent::WorldClockResumed {
                frozen_real_unix,
                resumed_real_unix,
                ..
            } => {
                self.pause_claims_for_outage(*frozen_real_unix);
                self.resume_claims_after_outage(*resumed_real_unix)
            }
            _ => Vec::new(),
        }
    }

    /// Submit a contest against a pending [`SalvageClaim`] within its 48-available-real-hour
    /// contest window (`Q-S13`, `Q-S15`, `Q-R1`).
    ///
    /// - Evaluates whether the contest ground is legally valid:
    ///   - [`ContestGround::BarePriorTitle`] is **always rejected** (`valid_to_vacate = None`)
    ///     per explicit owner ruling `Q-R1`.
    ///   - [`ContestGround::UnlawfulRecovery`] is validated against the claim's
    ///     `entitlement_at_attach` and the supplied [`SalvageEvidenceRecord`] (`Q-S4`, `Q-S13`).
    ///   - [`ContestGround::DefectiveProcedure`] is valid if `defect_code` is non-empty.
    ///   - [`ContestGround::SuperiorSpecificRight`] is valid if `obligor == claim.claimant`.
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the claim is unknown, already settled, paused for an
    /// outage, past `deadline_real_s`, or if evidence does not match the recovery.
    pub fn submit_contest(
        &mut self,
        claim_id: SalvageClaimId,
        challenger: ObjId,
        ground: ContestGround,
        filed_at_real_s: i64,
    ) -> Result<&SalvageContest, SalvageError> {
        if challenger.0 == 0 {
            return Err(SalvageError::InvalidObjectId("challenger"));
        }
        let claim = self
            .claims
            .get_mut(&claim_id)
            .ok_or(SalvageError::UnknownClaim(claim_id))?;

        if !claim.state.is_pending() {
            return Err(SalvageError::ClaimAlreadySettled(claim_id));
        }
        if claim.paused_at_real_s.is_some() {
            return Err(SalvageError::ClaimPausedForOutage(claim_id));
        }
        if filed_at_real_s > claim.deadline_real_s {
            return Err(SalvageError::ContestWindowExpired {
                claim_id,
                filed_at_real_s,
                deadline_real_s: claim.deadline_real_s,
            });
        }
        if claim.contests.len() >= MAX_CLAIM_ENTRIES {
            return Err(SalvageError::MaxClaimEntriesReached {
                claim_id,
                max: MAX_CLAIM_ENTRIES,
            });
        }

        let casualty_id = match &claim.subject {
            SalvageSubject::Hull { unit } => *unit,
            SalvageSubject::StrippedComponent { parent_hull, .. } => *parent_hull,
        };

        let valid_to_vacate = match &ground {
            ContestGround::BarePriorTitle => {
                // Q-R1: Bare prior unit title alone NEVER defeats a lawful salvage claim.
                None
            }
            ContestGround::UnlawfulRecovery { evidence } => {
                evidence.validate_for(claim.operation_id, casualty_id)?;
                if evidence.holder != challenger {
                    return Err(SalvageError::EvidenceHolderMismatch {
                        holder: evidence.holder,
                        actor: challenger,
                    });
                }
                if !claim.entitlement_at_attach.is_lawful() {
                    Some(ContestSuccessReason::ProvenUnlawfulRecovery)
                } else {
                    None
                }
            }
            ContestGround::DefectiveProcedure { defect_code } => {
                if !defect_code.trim().is_empty() {
                    Some(ContestSuccessReason::DefectiveProcedure)
                } else {
                    None
                }
            }
            ContestGround::SuperiorSpecificRight {
                contract_id,
                obligor,
            } => {
                if !contract_id.trim().is_empty() && *obligor == claim.claimant {
                    Some(ContestSuccessReason::SuperiorSpecificRight)
                } else {
                    None
                }
            }
        };

        claim.contests.push(SalvageContest {
            challenger,
            ground,
            filed_at_real_s,
            valid_to_vacate,
        });
        claim.state = SalvageClaimState::Contested;
        claim
            .contests
            .last()
            .ok_or(SalvageError::UnknownClaim(claim_id))
    }

    /// Adjudicate and settle a [`SalvageClaim`] (`Q-S2`, `Q-S13`–`Q-S15`, `Q-R1`).
    ///
    /// - If any submitted contest has `valid_to_vacate == Some(reason)`, the claim can be
    ///   vacated immediately (even before `deadline_real_s`), restoring title/custody right
    ///   to the winning challenger / `prior_owner` (`SalvageClaimState::Vacated`) without
    ///   reimbursing at-risk repairs (`Q-S14`).
    /// - Otherwise (uncontested, or only invalid contests such as [`ContestGround::BarePriorTitle`]
    ///   were submitted), once `now_real_s >= deadline_real_s` (and not paused for outage),
    ///   the claim deterministically settles as [`SalvageClaimState::AwardedToClaimant`],
    ///   transferring clean legal title to `claimant` (`Q-S2`, `Q-S15`, `Q-R1`).
    ///
    /// # Errors
    /// Returns [`SalvageError`] if the claim is unknown, already settled, paused for an
    /// outage, or has no valid contest and `now_real_s < deadline_real_s`.
    pub fn adjudicate_claim(
        &mut self,
        claim_id: SalvageClaimId,
        now_real_s: i64,
    ) -> Result<(CustodyTitleStatus, &SalvageClaim), SalvageError> {
        let claim = self
            .claims
            .get_mut(&claim_id)
            .ok_or(SalvageError::UnknownClaim(claim_id))?;

        if !claim.state.is_pending() {
            return Err(SalvageError::ClaimAlreadySettled(claim_id));
        }
        if claim.paused_at_real_s.is_some() {
            return Err(SalvageError::ClaimPausedForOutage(claim_id));
        }

        if let Some(winning_contest) = claim.contests.iter().find(|c| c.valid_to_vacate.is_some()) {
            let reason = winning_contest
                .valid_to_vacate
                .clone()
                .unwrap_or(ContestSuccessReason::ProvenUnlawfulRecovery);
            let restored_to = winning_contest.challenger;
            claim.state = SalvageClaimState::Vacated {
                restored_to,
                reason,
                settled_at_real_s: now_real_s,
            };
            return Ok((
                CustodyTitleStatus::ResolvedTitle { owner: restored_to },
                claim,
            ));
        }

        if now_real_s < claim.deadline_real_s {
            return Err(SalvageError::ContestWindowStillOpen {
                claim_id,
                now_real_s,
                deadline_real_s: claim.deadline_real_s,
            });
        }

        let new_owner = claim.claimant;
        claim.state = SalvageClaimState::AwardedToClaimant {
            new_owner,
            settled_at_real_s: now_real_s,
        };
        Ok((
            CustodyTitleStatus::ResolvedTitle { owner: new_owner },
            claim,
        ))
    }
}

/// Errors produced by `H2-B` / `H2-C` recovery entitlement, evidence, and salvage claim workflows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SalvageError {
    /// Object id was `ObjId(0)`.
    InvalidObjectId(&'static str),
    /// Salvage claim id was `0`.
    ZeroClaimId,
    /// Salvage claim id counter overflowed `u64`.
    ClaimIdOverflow,
    /// Duplicate authoritative recovery record for the same `RecoveryOpId`.
    DuplicateRecoveryRecord(RecoveryOpId),
    /// Unknown `RecoveryOpId`.
    UnknownRecoveryOperation(RecoveryOpId),
    /// Maximum recovery records reached.
    MaxRecoveryRecordsReached {
        /// Maximum records.
        max: usize,
    },
    /// Action against an unlawful recovery requires acquired evidence (`Q-S4`).
    EvidenceRequiredForUnlawfulAction {
        /// Recovery operation id.
        operation_id: RecoveryOpId,
        /// Casualty unit id.
        casualty: ObjId,
    },
    /// Supplied evidence did not match the target recovery operation or casualty.
    EvidenceMismatch {
        /// Expected operation id.
        expected_op: RecoveryOpId,
        /// Actual operation id in evidence.
        actual_op: RecoveryOpId,
        /// Expected casualty unit id.
        expected_casualty: ObjId,
        /// Actual casualty unit id in evidence.
        actual_casualty: ObjId,
    },
    /// Evidence holder did not match the actor presenting it.
    EvidenceHolderMismatch {
        /// Holder recorded on the evidence.
        holder: ObjId,
        /// Actor presenting the evidence.
        actor: ObjId,
    },
    /// Casualty has not yet been loaded into a bay.
    CasualtyNotYetLoadedInBay(ObjId),
    /// Reading a recovered hull's black box requires physical/custody access to the bay (`P-3`).
    BlackBoxPhysicalAccessRequired {
        /// Casualty hull id.
        casualty: ObjId,
        /// Actor attempting to read the black box.
        reader: ObjId,
    },
    /// Unlawful field stripping on owned ground cannot establish a valid part salvage claim (`Q-F6`).
    UnlawfulFieldStripCannotClaim {
        /// Parent hull id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// An active or awarded claim already exists for this hull or stripped component (`Q-F6`).
    DuplicateActiveClaimForSubject(SalvageSubject),
    /// Unknown `SalvageClaimId`.
    UnknownClaim(SalvageClaimId),
    /// Claim has already been settled (`AwardedToClaimant` or `Vacated`).
    ClaimAlreadySettled(SalvageClaimId),
    /// Claim contest window is currently paused for a global world outage freeze (`Q-F3`).
    ClaimPausedForOutage(SalvageClaimId),
    /// Contest submitted after the 48-available-real-hour window expired (`Q-S15`).
    ContestWindowExpired {
        /// Claim id.
        claim_id: SalvageClaimId,
        /// Submission timestamp.
        filed_at_real_s: i64,
        /// Deadline timestamp.
        deadline_real_s: i64,
    },
    /// Uncontested claim (or claim with no valid contest) cannot settle before `deadline_real_s`.
    ContestWindowStillOpen {
        /// Claim id.
        claim_id: SalvageClaimId,
        /// Current real timestamp.
        now_real_s: i64,
        /// Window expiry timestamp.
        deadline_real_s: i64,
    },
    /// Action is prohibited while salvage title is pending (`Q-S14`, `Q-F5`).
    ActionProhibitedWhileTitlePending {
        /// Pending claim id.
        claim_id: SalvageClaimId,
        /// Attempted action.
        action: CustodyAction,
    },
    /// Maximum claims reached in [`SalvageClaimBook`].
    MaxClaimsReached {
        /// Maximum claims.
        max: usize,
    },
    /// Maximum contests or repair entries reached on a claim.
    MaxClaimEntriesReached {
        /// Claim id.
        claim_id: SalvageClaimId,
        /// Maximum entries.
        max: usize,
    },
    /// Maximum forensic black-box entries reached for a hull (`MAX_BLACK_BOX_ENTRIES_PER_HULL`).
    BlackBoxTapeFull {
        /// Hull object id.
        hull: ObjId,
        /// Maximum entries per hull.
        max: usize,
    },
}

impl fmt::Display for SalvageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId(field) => write!(f, "{field} object id must be non-zero"),
            Self::ZeroClaimId => f.write_str("salvage claim id must be non-zero"),
            Self::ClaimIdOverflow => f.write_str("salvage claim id counter overflow"),
            Self::DuplicateRecoveryRecord(op) => {
                write!(f, "duplicate authoritative recovery record for {op}")
            }
            Self::UnknownRecoveryOperation(op) => {
                write!(f, "unknown recovery operation {op}")
            }
            Self::MaxRecoveryRecordsReached { max } => {
                write!(f, "authoritative recovery record limit ({max}) reached")
            }
            Self::EvidenceRequiredForUnlawfulAction {
                operation_id,
                casualty,
            } => write!(
                f,
                "acquired evidence is required to inspect or contest recovery {operation_id} of casualty {casualty}"
            ),
            Self::EvidenceMismatch {
                expected_op,
                actual_op,
                expected_casualty,
                actual_casualty,
            } => write!(
                f,
                "evidence mismatch: expected ({expected_op}, {expected_casualty}), got ({actual_op}, {actual_casualty})"
            ),
            Self::EvidenceHolderMismatch { holder, actor } => {
                write!(f, "evidence belongs to {holder}, not {actor}")
            }
            Self::CasualtyNotYetLoadedInBay(u) => {
                write!(f, "casualty {u} has not yet been loaded into a bay")
            }
            Self::BlackBoxPhysicalAccessRequired { casualty, reader } => write!(
                f,
                "reader {reader} lacks physical bay access to read black box of {casualty}"
            ),
            Self::UnlawfulFieldStripCannotClaim { hull, component } => write!(
                f,
                "unlawful field strip of {component} from {hull} on owned ground cannot file a salvage claim"
            ),
            Self::DuplicateActiveClaimForSubject(subj) => {
                write!(f, "duplicate active salvage claim for {subj:?}")
            }
            Self::UnknownClaim(id) => write!(f, "unknown salvage claim {id}"),
            Self::ClaimAlreadySettled(id) => write!(f, "salvage claim {id} is already settled"),
            Self::ClaimPausedForOutage(id) => {
                write!(f, "salvage claim {id} is paused for world outage")
            }
            Self::ContestWindowExpired {
                claim_id,
                filed_at_real_s,
                deadline_real_s,
            } => write!(
                f,
                "contest window for {claim_id} expired at {deadline_real_s} (submitted at {filed_at_real_s})"
            ),
            Self::ContestWindowStillOpen {
                claim_id,
                now_real_s,
                deadline_real_s,
            } => write!(
                f,
                "contest window for {claim_id} remains open until {deadline_real_s} (now={now_real_s})"
            ),
            Self::ActionProhibitedWhileTitlePending { claim_id, action } => write!(
                f,
                "custody action {action:?} is prohibited while salvage claim {claim_id} is pending"
            ),
            Self::MaxClaimsReached { max } => write!(f, "salvage claim limit ({max}) reached"),
            Self::MaxClaimEntriesReached { claim_id, max } => {
                write!(f, "salvage claim {claim_id} entry limit ({max}) reached")
            }
            Self::BlackBoxTapeFull { hull, max } => {
                write!(f, "black box tape for hull {hull} reached limit ({max})")
            }
        }
    }
}

impl std::error::Error for SalvageError {}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn self_recovery_restores_title_immediately_without_contest_window() -> TestResult {
        let mut book = SalvageClaimBook::new();
        let op_id = RecoveryOpId::new(1)?;
        let owner = ObjId(10);
        let casualty = ObjId(100);
        let tug = ObjId(101);
        let carrier = ObjId(900);

        let ctx = RecoveryAuthorityContext {
            recoverer: owner,
            unit_title_holder: owner,
            has_owner_authorization: true,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder: ObjId(99),
                claim_id: Some(SmolStr::new("claim.alpha")),
            },
        };

        let rec = book.record_physical_attach(op_id, casualty, tug, &ctx, 1_000)?;
        check_eq!(
            rec.entitlement,
            RecoveryEntitlement::LawfulSelfOrAgent { unit_owner: owner }
        );
        book.record_bay_load(op_id, carrier, 1_050)?;

        let status = book.resolve_hull_recovery_custody(op_id, 5_000)?;
        check_eq!(status, CustodyTitleStatus::ResolvedTitle { owner });
        check!(check_custody_action_allowed(&status, CustodyAction::DeploySortie, false).is_ok());
        check!(check_custody_action_allowed(&status, CustodyAction::StripComponent, false).is_ok());

        Ok(())
    }

    #[test]
    fn lawful_landholder_recovery_rejects_bare_prior_title_and_transfers_title_after_48h_plus_outage()
    -> TestResult {
        let mut book = SalvageClaimBook::new();
        let op_id = RecoveryOpId::new(2)?;
        let unit_owner = ObjId(10);
        let landholder = ObjId(20);
        let casualty = ObjId(200);
        let tug = ObjId(201);
        let carrier = ObjId(901);

        let ctx = RecoveryAuthorityContext {
            recoverer: landholder,
            unit_title_holder: unit_owner,
            has_owner_authorization: false,
            has_landholder_authorization: true,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder,
                claim_id: Some(SmolStr::new("deed.ceres_north")),
            },
        };

        let rec = book.record_physical_attach(op_id, casualty, tug, &ctx, 2_000)?;
        check_eq!(
            rec.entitlement,
            RecoveryEntitlement::LawfulLandholderOrAgent {
                landholder,
                unit_owner,
            }
        );
        book.record_bay_load(op_id, carrier, 2_100)?;

        let filed_at = 10_000;
        let status = book.resolve_hull_recovery_custody(op_id, filed_at)?;
        let claim_id = match status {
            CustodyTitleStatus::PendingSalvageClaim {
                claim_id,
                custodian,
                prior_owner,
                deadline_real_s,
            } => {
                check_eq!(custodian, landholder);
                check_eq!(prior_owner, unit_owner);
                check_eq!(
                    deadline_real_s,
                    filed_at + CONTEST_WINDOW_AVAILABLE_REAL_SECS
                );
                claim_id
            }
            CustodyTitleStatus::ResolvedTitle { .. } => {
                return Err("expected PendingSalvageClaim".into());
            }
        };

        check!(
            check_custody_action_allowed(&status, CustodyAction::DocumentedRepairAtRisk, false)
                .is_ok()
        );
        check!(check_custody_action_allowed(&status, CustodyAction::DeploySortie, false).is_err());
        check!(
            check_custody_action_allowed(&status, CustodyAction::StripComponent, false).is_err()
        );
        check!(check_custody_action_allowed(&status, CustodyAction::ScrapHull, false).is_err());

        book.record_repair_at_risk(claim_id, SmolStr::new("patch armor"), 1_500, 2_200)?;

        let contest = book.submit_contest(
            claim_id,
            unit_owner,
            ContestGround::BarePriorTitle,
            filed_at + 3_600,
        )?;
        check_eq!(contest.valid_to_vacate, None);

        book.pause_claims_for_outage(filed_at + 10_000);
        check!(matches!(
            book.adjudicate_claim(claim_id, filed_at + CONTEST_WINDOW_AVAILABLE_REAL_SECS + 5_000),
            Err(SalvageError::ClaimPausedForOutage(id)) if id == claim_id
        ));
        book.resume_claims_after_outage(filed_at + 11_000);

        let extended_deadline = filed_at + CONTEST_WINDOW_AVAILABLE_REAL_SECS + 1_000;
        check!(matches!(
            book.adjudicate_claim(claim_id, extended_deadline - 1),
            Err(SalvageError::ContestWindowStillOpen { .. })
        ));

        let (final_status, settled_claim) = book.adjudicate_claim(claim_id, extended_deadline)?;
        check_eq!(
            final_status,
            CustodyTitleStatus::ResolvedTitle { owner: landholder }
        );
        check_eq!(
            settled_claim.state,
            SalvageClaimState::AwardedToClaimant {
                new_owner: landholder,
                settled_at_real_s: extended_deadline,
            }
        );

        Ok(())
    }

    #[test]
    fn unlawful_third_party_recovery_on_owned_ground_requires_evidence_and_vacates_claim()
    -> TestResult {
        let mut book = SalvageClaimBook::new();
        let op_id = RecoveryOpId::new(3)?;
        let unit_owner = ObjId(10);
        let landholder = ObjId(20);
        let pirate = ObjId(30);
        let casualty = ObjId(300);
        let tug = ObjId(301);
        let pirate_carrier = ObjId(903);

        let ctx = RecoveryAuthorityContext {
            recoverer: pirate,
            unit_title_holder: unit_owner,
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder,
                claim_id: Some(SmolStr::new("deed.stickney_basin")),
            },
        };

        let rec = book.record_physical_attach(op_id, casualty, tug, &ctx, 3_000)?;
        check_eq!(
            rec.entitlement,
            RecoveryEntitlement::UnlawfulOnOwnedGround {
                landholder,
                unit_owner,
            }
        );
        book.record_bay_load(op_id, pirate_carrier, 3_100)?;

        check!(matches!(
            book.inspect_recovery_record_with_evidence(op_id, unit_owner, None),
            Err(SalvageError::EvidenceRequiredForUnlawfulAction { .. })
        ));
        check!(matches!(
            book.read_recovered_black_box(op_id, unit_owner, false, 3_150),
            Err(SalvageError::BlackBoxPhysicalAccessRequired { .. })
        ));

        let status = book.resolve_hull_recovery_custody(op_id, 20_000)?;
        let claim_id = match status {
            CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } => claim_id,
            CustodyTitleStatus::ResolvedTitle { .. } => {
                return Err("expected PendingSalvageClaim".into());
            }
        };
        book.record_repair_at_risk(claim_id, SmolStr::new("pirate patch"), 5_000, 3_200)?;

        let evidence = SalvageEvidenceRecord {
            operation_id: op_id,
            casualty,
            holder: unit_owner,
            kind: SalvageEvidenceKind::SensorLog {
                sensor_unit: ObjId(777),
                recorded_at_ic: 3_010,
            },
        };
        check!(
            book.inspect_recovery_record_with_evidence(op_id, unit_owner, Some(&evidence))
                .is_ok()
        );

        let contest = book.submit_contest(
            claim_id,
            unit_owner,
            ContestGround::UnlawfulRecovery { evidence },
            21_000,
        )?;
        check_eq!(
            contest.valid_to_vacate,
            Some(ContestSuccessReason::ProvenUnlawfulRecovery)
        );

        let (restored, vacated_claim) = book.adjudicate_claim(claim_id, 21_500)?;
        check_eq!(
            restored,
            CustodyTitleStatus::ResolvedTitle { owner: unit_owner }
        );
        check_eq!(
            vacated_claim.state,
            SalvageClaimState::Vacated {
                restored_to: unit_owner,
                reason: ContestSuccessReason::ProvenUnlawfulRecovery,
                settled_at_real_s: 21_500,
            }
        );

        Ok(())
    }
}
