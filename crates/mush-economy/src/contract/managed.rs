//! Closed managed consent identity and legacy refusal predicates. Decoding is
//! structural only. Every managed kind requires its own actual owner protocol.

use super::EconomicContract;
use crate::ids::ContractId;
use crate::managed_renewal::RenewalMarkerV1;
use crate::plan::{AssetOp, EconomyError, EconomyPlan, HoldOp};
use crate::record::{EconomicCause, EconomicTransaction, Encumbrance, HoldPurpose, IdempotencyKey};
use crate::snapshot::EconomySnapshot;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef};
use serde::{Deserialize, Serialize};

/// Closed internal kind. Adding renewal does not project it as an original or
/// entitlement purchase. Existing wire variants retain their exact fields/order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ManagedWire", into = "ManagedWire")]
pub struct ManagedConsent {
    wire: ManagedWire,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ManagedWire {
    OriginalPersonal {
        operation: CommitToken,
        effect: DefinitionRef,
        quote: CommitToken,
        quote_digest: DefinitionDigest,
        product_digest: DefinitionDigest,
    },
    MindEntitlement {
        operation: CommitToken,
        effect: DefinitionRef,
        quote: CommitToken,
        quote_digest: DefinitionDigest,
        product_digest: DefinitionDigest,
        protocol: crate::managed_entitlement::EntitlementProtocol,
        economic_digest: DefinitionDigest,
    },
    MindRenewalOfferV1 {
        renewal: RenewalMarkerV1,
    },
}
impl TryFrom<ManagedWire> for ManagedConsent {
    type Error = &'static str;
    fn try_from(wire: ManagedWire) -> Result<Self, Self::Error> {
        match &wire {
            ManagedWire::OriginalPersonal {
                operation,
                effect,
                quote,
                ..
            } => {
                if effect.key().owner.as_str() != "market"
                    || effect.key().kind.as_str() != "action"
                    || effect.key().id.as_str() != "purchase_original_mind"
                    || !matches!(effect.version(), 1 | 2)
                    || operation == quote
                {
                    return Err("invalid original personal managed consent");
                }
            }
            ManagedWire::MindEntitlement {
                operation,
                effect,
                quote,
                protocol,
                ..
            } => {
                if !protocol.matches_action(effect, "fund") || operation == quote {
                    return Err("invalid mind entitlement managed consent");
                }
            }
            ManagedWire::MindRenewalOfferV1 { renewal } => {
                renewal
                    .validate()
                    .map_err(|_| "invalid mind renewal offer consent")?;
            }
        }
        Ok(Self { wire })
    }
}
impl From<ManagedConsent> for ManagedWire {
    fn from(value: ManagedConsent) -> Self {
        value.wire
    }
}
impl ManagedConsent {
    pub(crate) fn entitlement_purchase(
        operation: CommitToken,
        effect: DefinitionRef,
        quote: CommitToken,
        quote_digest: DefinitionDigest,
        product_digest: DefinitionDigest,
        protocol: crate::managed_entitlement::EntitlementProtocol,
        economic_digest: DefinitionDigest,
    ) -> Result<Self, mush_core::participant::ParticipantError> {
        Self::try_from(ManagedWire::MindEntitlement {
            operation,
            effect,
            quote,
            quote_digest,
            product_digest,
            protocol,
            economic_digest,
        })
        .map_err(mush_core::participant::ParticipantError::refused)
    }
    pub(crate) fn original_purchase(
        operation: CommitToken,
        effect: DefinitionRef,
        quote: CommitToken,
        quote_digest: DefinitionDigest,
        product_digest: DefinitionDigest,
    ) -> Result<Self, mush_core::participant::ParticipantError> {
        Self::try_from(ManagedWire::OriginalPersonal {
            operation,
            effect,
            quote,
            quote_digest,
            product_digest,
        })
        .map_err(mush_core::participant::ParticipantError::refused)
    }
    pub(crate) fn renewal_offer(
        renewal: RenewalMarkerV1,
    ) -> Result<Self, mush_core::participant::ParticipantError> {
        Self::try_from(ManagedWire::MindRenewalOfferV1 { renewal })
            .map_err(mush_core::participant::ParticipantError::refused)
    }
    pub fn renewal_offer_marker(&self) -> Option<&RenewalMarkerV1> {
        match &self.wire {
            ManagedWire::MindRenewalOfferV1 { renewal } => Some(renewal),
            _ => None,
        }
    }
    pub fn entitlement_protocol(&self) -> Option<crate::managed_entitlement::EntitlementProtocol> {
        match &self.wire {
            ManagedWire::MindEntitlement { protocol, .. } => Some(*protocol),
            _ => None,
        }
    }
    pub fn economic_digest(&self) -> Option<DefinitionDigest> {
        match &self.wire {
            ManagedWire::MindEntitlement {
                economic_digest, ..
            } => Some(*economic_digest),
            _ => None,
        }
    }
    #[must_use]
    pub const fn operation(&self) -> CommitToken {
        match &self.wire {
            ManagedWire::OriginalPersonal { operation, .. }
            | ManagedWire::MindEntitlement { operation, .. } => *operation,
            ManagedWire::MindRenewalOfferV1 { renewal } => renewal.operation(),
        }
    }
    #[must_use]
    pub fn effect(&self) -> &DefinitionRef {
        match &self.wire {
            ManagedWire::OriginalPersonal { effect, .. }
            | ManagedWire::MindEntitlement { effect, .. } => effect,
            ManagedWire::MindRenewalOfferV1 { renewal } => renewal.effect(),
        }
    }
    /// Actual retained quote basis. Renewal returns its accepted prior quote;
    /// only the explicit renewal marker conveys its different protocol shape.
    #[must_use]
    pub const fn quote(&self) -> CommitToken {
        match &self.wire {
            ManagedWire::OriginalPersonal { quote, .. }
            | ManagedWire::MindEntitlement { quote, .. } => *quote,
            ManagedWire::MindRenewalOfferV1 { renewal } => renewal.basis().prior_quote,
        }
    }
    #[must_use]
    pub const fn quote_digest(&self) -> DefinitionDigest {
        match &self.wire {
            ManagedWire::OriginalPersonal { quote_digest, .. }
            | ManagedWire::MindEntitlement { quote_digest, .. } => *quote_digest,
            ManagedWire::MindRenewalOfferV1 { renewal } => renewal.basis().prior_quote_digest,
        }
    }
    #[must_use]
    pub const fn product_digest(&self) -> DefinitionDigest {
        match &self.wire {
            ManagedWire::OriginalPersonal { product_digest, .. }
            | ManagedWire::MindEntitlement { product_digest, .. } => *product_digest,
            ManagedWire::MindRenewalOfferV1 { renewal } => renewal.basis().product_digest,
        }
    }
}

pub(crate) fn at(snapshot: &EconomySnapshot, id: ContractId) -> bool {
    snapshot
        .contract(id)
        .is_some_and(|c| c.consent.managed.is_some())
}

/// The actual retained contract is consulted as well as the caller's copy. A
/// clone with its marker stripped does not regain legacy contract authority.
pub(crate) fn legacy_contract(c: &EconomicContract, snapshot: &EconomySnapshot) -> bool {
    c.consent.managed.is_none() && !at(snapshot, c.id)
}

fn purpose(snapshot: &EconomySnapshot, purpose: &HoldPurpose) -> bool {
    matches!(purpose, HoldPurpose::Escrow { contract } if at(snapshot, *contract))
}
fn encumbrance(snapshot: &EconomySnapshot, value: &Encumbrance) -> bool {
    match value {
        Encumbrance::Lease { contract, .. } | Encumbrance::Collateral { contract } => {
            at(snapshot, *contract)
        }
        Encumbrance::Lien { .. } | Encumbrance::Disputed { .. } => false,
    }
}
fn cause(snapshot: &EconomySnapshot, cause: &EconomicCause) -> bool {
    matches!(cause, EconomicCause::Internal { kind, id }
        if (kind == "contract" || kind.starts_with("contract.")) && at(snapshot, ContractId(*id)))
}
fn key(snapshot: &EconomySnapshot, key: &IdempotencyKey) -> bool {
    // A supplementary reference, never the sole escrow fence. The actual hold
    // and contract references below remain authoritative even with a new key.
    key.namespace == "contract"
        && key
            .key
            .split('/')
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|id| at(snapshot, ContractId(id)))
}

fn closed_money_namespace(namespace: &str) -> bool {
    crate::managed_purchase::closed_namespace(namespace)
        || namespace == crate::managed_renewal::NAMESPACE
}

/// Refuse before idempotency lookup, custom authorization or approval replay.
/// New-key plans cannot release/capture/expire an actual managed escrow hold.
pub(crate) fn require_legacy_plan(
    plan: &EconomyPlan,
    snapshot: &EconomySnapshot,
) -> Result<(), EconomyError> {
    let managed = closed_money_namespace(plan.idempotency.namespace.as_str())
        || matches!(&plan.cause, EconomicCause::Ruleset { namespace, .. }
            if closed_money_namespace(namespace.as_str()))
        || cause(snapshot, &plan.cause)
        || key(snapshot, &plan.idempotency)
        || plan.hold_ops.iter().any(|op| match op {
            HoldOp::Place { purpose: p, .. } => purpose(snapshot, p),
            HoldOp::Capture { hold, .. } | HoldOp::Release { hold } | HoldOp::Expire { hold } => {
                snapshot
                    .hold(*hold)
                    .is_some_and(|h| purpose(snapshot, &h.purpose))
            }
        })
        || plan.asset_ops.iter().any(|op| {
            snapshot
                .asset(op.object())
                .is_some_and(|asset| asset.encumbrances.iter().any(|e| encumbrance(snapshot, e)))
                || snapshot
                    .deposit_lock_of(op.object())
                    .is_some_and(|id| at(snapshot, id))
                || match op {
                    AssetOp::Encumber { encumbrance: e, .. }
                    | AssetOp::Release { encumbrance: e, .. } => encumbrance(snapshot, e),
                    _ => false,
                }
        })
        || plan.custody_ops.iter().any(|op| {
            snapshot
                .deposit_lock_of(op.object())
                .is_some_and(|id| at(snapshot, id))
        });
    if managed {
        Err(EconomyError::ManagedContract)
    } else {
        Ok(())
    }
}

pub(crate) fn require_legacy_transaction(
    tx: &EconomicTransaction,
    snapshot: &EconomySnapshot,
) -> Result<(), EconomyError> {
    if closed_money_namespace(tx.idempotency.namespace.as_str())
        || matches!(&tx.cause, EconomicCause::Ruleset { namespace, .. }
            if closed_money_namespace(namespace.as_str()))
        || cause(snapshot, &tx.cause)
        || key(snapshot, &tx.idempotency)
        || tx
            .hold_effects
            .iter()
            .any(|e| purpose(snapshot, &e.hold.purpose))
    {
        Err(EconomyError::ManagedContract)
    } else {
        Ok(())
    }
}
