//! Closed native recipe dispatch. The shared monetary projection contains no
//! seller identity; the actual immutable recipe independently proves its issuer.
use super::*;
use crate::Holder;
use crate::managed_entitlement::{EntitlementPurchase, EntitlementTerminal};
use crate::managed_house_purchase::{HousePurchase, HouseTerminal};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MonetaryPurchase {
    pub quote: CommitToken,
    pub quote_digest: DefinitionDigest,
    pub product_digest: DefinitionDigest,
    pub buyer: Holder,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub instrument: InstrumentId,
    pub price: crate::Amount,
    pub payouts: Vec<PurchasePayout>,
    pub quoted_at_utc: i64,
    pub expires_at_utc: i64,
}
impl From<&OriginalPurchase> for MonetaryPurchase {
    fn from(p: &OriginalPurchase) -> Self {
        Self {
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: Holder::Character(p.buyer),
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            payouts: p.payouts.clone(),
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        }
    }
}
impl MonetaryPurchase {
    fn check_contract_capacity(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        let snapshot = &state.snapshot;
        let buyer = self.buyer.principal();
        if snapshot
            .contracts()
            .filter(|c| c.issuer == buyer && !c.state.is_terminal())
            .count()
            >= crate::contract::MAX_OPEN_CONTRACTS_PER_ISSUER
            || snapshot
                .contracts()
                .filter(|c| !c.state.is_terminal() && c.consent.parties.contains(&buyer))
                .count()
                >= crate::contract::MAX_LIVE_CONTRACTS_PER_PARTY
        {
            return Err(refused());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Recipe {
    Original(OriginalPurchase),
    Entitlement(EntitlementPurchase),
    House(HousePurchase),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PurchaseSubject {
    common: MonetaryPurchase,
    recipe: Recipe,
}
impl From<OriginalPurchase> for PurchaseSubject {
    fn from(p: OriginalPurchase) -> Self {
        Self {
            common: MonetaryPurchase::from(&p),
            recipe: Recipe::Original(p),
        }
    }
}
impl TryFrom<EntitlementPurchase> for PurchaseSubject {
    type Error = ParticipantError;
    fn try_from(p: EntitlementPurchase) -> Result<Self, Self::Error> {
        let common = MonetaryPurchase {
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            payouts: p.payouts()?,
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        };
        Ok(Self {
            common,
            recipe: Recipe::Entitlement(p),
        })
    }
}
impl TryFrom<HousePurchase> for PurchaseSubject {
    type Error = ParticipantError;
    fn try_from(p: HousePurchase) -> Result<Self, Self::Error> {
        let common = MonetaryPurchase {
            quote: p.quote,
            quote_digest: p.quote_digest,
            product_digest: p.product_digest,
            buyer: p.buyer,
            charter: p.charter,
            title_revision: p.title_revision,
            payer: p.payer,
            instrument: p.instrument.clone(),
            price: p.price,
            payouts: p.payouts()?,
            quoted_at_utc: p.quoted_at_utc,
            expires_at_utc: p.expires_at_utc,
        };
        Ok(Self {
            common,
            recipe: Recipe::House(p),
        })
    }
}
impl PurchaseSubject {
    pub(super) fn common(&self) -> &MonetaryPurchase {
        &self.common
    }
    pub(super) fn recipe(&self) -> &Recipe {
        &self.recipe
    }
    pub(crate) fn original(&self) -> Option<&OriginalPurchase> {
        match &self.recipe {
            Recipe::Original(p) => Some(p),
            _ => None,
        }
    }
    pub(crate) fn entitlement(&self) -> Option<&EntitlementPurchase> {
        match &self.recipe {
            Recipe::Entitlement(p) => Some(p),
            _ => None,
        }
    }
    pub(crate) fn house(&self) -> Option<&HousePurchase> {
        match &self.recipe {
            Recipe::House(p) => Some(p),
            _ => None,
        }
    }
    pub(crate) fn namespace(&self) -> &'static str {
        match self.recipe {
            Recipe::Original(_) => NAMESPACE,
            Recipe::Entitlement(_) => crate::managed_entitlement::NAMESPACE,
            Recipe::House(_) => crate::managed_house_purchase::NAMESPACE,
        }
    }
    pub(crate) fn maximum_payouts(&self) -> usize {
        match self.recipe {
            Recipe::Original(_) => MAX_PAYOUTS,
            Recipe::Entitlement(_) => crate::managed_entitlement::MAX_PAYOUTS,
            Recipe::House(_) => crate::managed_house_purchase::MAX_PAYOUTS,
        }
    }
    pub(crate) fn funding_description(&self) -> &'static str {
        match self.recipe {
            Recipe::Original(_) => "Funding original personal mind purchase",
            Recipe::Entitlement(_) => "Funding mind entitlement purchase",
            Recipe::House(_) => "Funding house mind purchase",
        }
    }
    pub(crate) fn contract_name(&self) -> &'static str {
        match self.recipe {
            Recipe::Original(_) => "Original mind purchase",
            Recipe::Entitlement(_) => "Mind entitlement purchase",
            Recipe::House(_) => "House mind purchase",
        }
    }
    pub(crate) fn validate_shape(&self) -> Result<(), ParticipantError> {
        let expected = match &self.recipe {
            Recipe::Original(p) => {
                p.validate_shape()?;
                Self::from(p.clone())
            }
            Recipe::Entitlement(p) => Self::try_from(p.clone())?,
            Recipe::House(p) => Self::try_from(p.clone())?,
        };
        if expected.common != self.common {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn matches_action(&self, action: &DefinitionRef, transition: &str) -> bool {
        match &self.recipe {
            Recipe::Entitlement(p) => p.protocol.matches_action(action, transition),
            Recipe::House(_) => {
                crate::managed_house_purchase::subject::matches_action(action, transition)
            }
            Recipe::Original(_) => {
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
        }
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
        match &self.recipe {
            Recipe::Original(_) => ManagedConsent::original_purchase(
                operation,
                effect,
                p.quote,
                p.quote_digest,
                p.product_digest,
            ),
            Recipe::Entitlement(e) => ManagedConsent::entitlement_purchase(
                operation,
                effect,
                p.quote,
                p.quote_digest,
                p.product_digest,
                e.protocol,
                e.economic_digest()?,
            ),
            Recipe::House(h) => ManagedConsent::house_purchase(
                operation,
                effect,
                p.quote,
                p.quote_digest,
                p.product_digest,
                h.economic_digest()?,
            ),
        }
    }
    pub(crate) fn check_current(
        &self,
        state: &EconomyStore,
        context: &FundingContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.validate_shape()?;
        match &self.recipe {
            Recipe::Original(p) => p.check_current(state, context),
            Recipe::Entitlement(p) => p.check_current(state, context),
            Recipe::House(p) => p.check_current(state, context),
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
        if let Recipe::Entitlement(p) = &self.recipe {
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
        match &self.recipe {
            Recipe::Original(p) => p.payouts.iter().map(|p| p.account).collect(),
            Recipe::Entitlement(p) => p.recipients().into_iter().map(|r| r.account).collect(),
            Recipe::House(p) => p.recipients().into_iter().map(|r| r.account).collect(),
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
impl TryFrom<HouseTerminal> for SubjectTerminal {
    type Error = ParticipantError;
    fn try_from(p: HouseTerminal) -> Result<Self, Self::Error> {
        Ok(Self {
            operation: p.operation,
            purchase: p.purchase.try_into()?,
            funding: p.funding,
        })
    }
}
