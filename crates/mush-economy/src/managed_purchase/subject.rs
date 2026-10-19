//! Internal closed recipe dispatch. The common monetary projection is never
//! accepted as an OriginalPurchase: its recipe is retained through preparation,
//! history, recheck and installation. No public constructor selects a recipe.

use super::*;
use crate::managed_entitlement::{EntitlementPurchase, EntitlementTerminal};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PurchaseSubject {
    common: OriginalPurchase,
    entitlement: Option<EntitlementPurchase>,
}
impl From<OriginalPurchase> for PurchaseSubject {
    fn from(common: OriginalPurchase) -> Self {
        Self {
            common,
            entitlement: None,
        }
    }
}
impl TryFrom<EntitlementPurchase> for PurchaseSubject {
    type Error = ParticipantError;
    fn try_from(value: EntitlementPurchase) -> Result<Self, Self::Error> {
        let common = OriginalPurchase {
            quote: value.quote,
            quote_digest: value.quote_digest,
            product_digest: value.product_digest,
            buyer: value.buyer,
            seller: value.seller,
            charter: value.charter,
            title_revision: value.title_revision,
            payer: value.payer,
            instrument: value.instrument.clone(),
            price: value.price,
            payouts: value.payouts()?,
            quoted_at_utc: value.quoted_at_utc,
            expires_at_utc: value.expires_at_utc,
        };
        Ok(Self {
            common,
            entitlement: Some(value),
        })
    }
}
impl PurchaseSubject {
    pub(crate) fn common(&self) -> &OriginalPurchase {
        &self.common
    }
    pub(crate) fn entitlement(&self) -> Option<&EntitlementPurchase> {
        self.entitlement.as_ref()
    }
    pub(crate) fn namespace(&self) -> &'static str {
        if self.entitlement.is_some() {
            crate::managed_entitlement::NAMESPACE
        } else {
            NAMESPACE
        }
    }
    pub(crate) fn maximum_payouts(&self) -> usize {
        if self.entitlement.is_some() {
            crate::managed_entitlement::MAX_PAYOUTS
        } else {
            MAX_PAYOUTS
        }
    }
    pub(crate) fn funding_description(&self) -> &'static str {
        if self.entitlement.is_some() {
            "Funding mind entitlement purchase"
        } else {
            "Funding original personal mind purchase"
        }
    }
    pub(crate) fn contract_name(&self) -> &'static str {
        if self.entitlement.is_some() {
            "Mind entitlement purchase"
        } else {
            "Original mind purchase"
        }
    }
    pub(crate) fn validate_shape(&self) -> Result<(), ParticipantError> {
        if let Some(p) = &self.entitlement {
            if Self::try_from(p.clone())?.common != self.common {
                return Err(refused());
            }
            Ok(())
        } else {
            self.common.validate_shape()
        }
    }
    pub(crate) fn matches_action(&self, action: &DefinitionRef, transition: &str) -> bool {
        if let Some(p) = &self.entitlement {
            return p.protocol.matches_action(action, transition);
        }
        let id = match transition {
            "fund" => "purchase_original_mind",
            "grant" => "settle_original_mind",
            "refund" => "refund_original_mind",
            _ => return false,
        };
        action.key().owner.as_str() == "market"
            && action.key().kind.as_str() == "action"
            && action.key().id.as_str() == id
            && matches!(action.version(), 1 | 2)
    }
    pub(crate) fn marker(
        &self,
        operation: CommitToken,
        effect: DefinitionRef,
    ) -> Result<ManagedConsent, ParticipantError> {
        if !self.matches_action(&effect, "fund") {
            return Err(refused());
        }
        let p = &self.common;
        if let Some(e) = &self.entitlement {
            ManagedConsent::entitlement_purchase(
                operation,
                effect,
                p.quote,
                p.quote_digest,
                p.product_digest,
                e.protocol,
                e.economic_digest()?,
            )
        } else {
            ManagedConsent::original_purchase(
                operation,
                effect,
                p.quote,
                p.quote_digest,
                p.product_digest,
            )
        }
    }
    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        context: &FundingContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.validate_shape()?;
        match &self.entitlement {
            Some(p) => p.check_current(state, context),
            None => self.common.check_current(state, context),
        }
    }
    pub(crate) fn check_contract_capacity(
        &self,
        state: &EconomyStore,
    ) -> Result<(), ParticipantError> {
        self.common.check_contract_capacity(state)
    }
    pub(crate) fn title_ids(&self) -> Vec<ObjId> {
        let mut ids = vec![self.common.charter];
        if let Some(p) = &self.entitlement {
            ids.extend(
                p.parents
                    .iter()
                    .filter(|c| c.permille != 0)
                    .map(|c| c.charter),
            );
        }
        ids
    }
    pub(crate) fn account_ids(&self) -> Vec<AccountId> {
        match &self.entitlement {
            Some(p) => p.recipients().into_iter().map(|r| r.account).collect(),
            None => self.common.payouts.iter().map(|p| p.account).collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SubjectTerminal {
    pub operation: CommitToken,
    pub purchase: PurchaseSubject,
    pub funding: FundingReceipt,
}
impl From<PurchaseTerminal> for SubjectTerminal {
    fn from(p: PurchaseTerminal) -> Self {
        Self {
            operation: p.operation,
            purchase: p.purchase.into(),
            funding: p.funding,
        }
    }
}
impl TryFrom<EntitlementTerminal> for SubjectTerminal {
    type Error = ParticipantError;
    fn try_from(p: EntitlementTerminal) -> Result<Self, Self::Error> {
        Ok(Self {
            operation: p.operation,
            purchase: p.purchase.try_into()?,
            funding: p.funding,
        })
    }
}
