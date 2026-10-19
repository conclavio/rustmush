//! Strict retained economic intent. These values are comparison data, never
//! owner authority, proof of a die's certification, or permission to post.
use super::{CoreIntegrationError as Error, SUBJECT_BYTES};
use crate::{AccountId, Amount, CatalogRef, InstrumentId, VaultId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const RESERVATION_SECONDS: i64 = 7 * 86_400;
pub(crate) const CORE3_ENTITLEMENT: &str = "tacsim.licence.core3_end_user";

/// Physical governed core class. ROM is not a manufactured input or output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreClass {
    I,
    II,
    III,
}
impl CoreClass {
    pub const fn finished_minor(self) -> i64 {
        match self {
            Self::I => 300_000,
            Self::II => 800_000,
            Self::III => 2_500_000,
        }
    }
    pub const fn integration_minor(self) -> i64 {
        self.finished_minor() / 10
    }
    pub const fn die_minor(self) -> i64 {
        self.finished_minor() - self.integration_minor()
    }
    pub const fn catalog_index(self) -> usize {
        match self {
            Self::I => 0,
            Self::II => 1,
            Self::III => 2,
        }
    }
}

/// Exact configured account and actual coherent organization compartment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedCoreAccount {
    pub account: AccountId,
    pub organization: OrgId,
    pub scope: OrgScopeId,
}

/// Frozen by trusted module composition, not accepted as action parameters.
/// Current account/org/scope/issuer/recipe readiness is checked separately.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorePaymentPolicy {
    pub issuer: SystemPrincipalId,
    pub instrument: InstrumentId,
    pub supplier: ScopedCoreAccount,
    pub integrator: ScopedCoreAccount,
    pub supplier_stock: [VaultId; 2],
    pub factory: ObjId,
    pub output_container: ObjId,
    pub recipe: DefinitionRef,
    pub catalogs: [CatalogRef; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogWire {
    ruleset: smol_str::SmolStr,
    code: smol_str::SmolStr,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWire {
    issuer: SystemPrincipalId,
    instrument: InstrumentId,
    supplier: ScopedCoreAccount,
    integrator: ScopedCoreAccount,
    supplier_stock: [VaultId; 2],
    factory: ObjId,
    output_container: ObjId,
    recipe: DefinitionRef,
    catalogs: [CatalogWire; 3],
}
impl<'de> Deserialize<'de> for CorePaymentPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = PolicyWire::deserialize(d)?;
        let policy = Self {
            issuer: w.issuer,
            instrument: w.instrument,
            supplier: w.supplier,
            integrator: w.integrator,
            supplier_stock: w.supplier_stock,
            factory: w.factory,
            output_container: w.output_container,
            recipe: w.recipe,
            catalogs: w.catalogs.map(|c| CatalogRef {
                ruleset: c.ruleset,
                code: c.code,
            }),
        };
        policy.validate().map_err(serde::de::Error::custom)?;
        Ok(policy)
    }
}
impl CorePaymentPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if self.supplier.account == self.integrator.account
            || self.supplier_stock[0] == self.supplier_stock[1]
            || self.factory == self.output_container
            || self.recipe.key().kind.as_str() != "recipe"
            || self.recipe.key().id.as_str() != "core_integrate"
            || self.recipe.version() != 1
            || self.recipe.digest().as_bytes() == &[0; 32]
        {
            return Err(Error::Invalid);
        }
        for (index, catalog) in self.catalogs.iter().enumerate() {
            if catalog.ruleset.is_empty()
                || catalog.ruleset.len() > 64
                || catalog.code.is_empty()
                || catalog.code.len() > 64
                || self
                    .catalogs
                    .iter()
                    .skip(index + 1)
                    .any(|other| other == catalog)
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| Error::Invalid)?;
        let bytes = mush_core::mechanics::canonical_json_bytes_with_limit(&value, SUBJECT_BYTES)
            .map_err(|_| Error::Capacity)?;
        Ok(DefinitionDigest::from_bytes(Sha256::digest(bytes).into()))
    }

    /// Shared exact action recipe. Historical receipts reconstruct this full
    /// reference from their retained policy; a matching locator is insufficient.
    pub fn action_descriptor(
        &self,
        action: &str,
    ) -> Result<mush_core::DefinitionDescriptor, Error> {
        self.validate()?;
        // Cleanup drains the exact acknowledged hold/input even after the current
        // recipe or factory configuration changes. This separately closed action
        // has stable bytes and accepts only the retained order identity.
        if action == "cancel_core_integration" {
            return mush_core::DefinitionDescriptor::new(
                mush_core::DefinitionKind::new("action").map_err(|_| Error::Invalid)?,
                mush_core::DefinitionId::new(action).map_err(|_| Error::Invalid)?,
                1,
                serde_json::json!({"format":1,"protocol":"certified_core_integration_cancel_v1",
                    "authority":"actual_acknowledged_requester","effect":"release_exact_retained_input_slot_and_payment",
                    "current_recipe_required":false}),
            ).map_err(|_| Error::Invalid);
        }
        if !matches!(
            action,
            "initialize_core_production"
                | "offer_core_integration"
                | "accept_core_integration"
                | "reserve_core_output"
                | "advance_core_integration"
                | "complete_core_integration"
                | "cancel_core_integration"
        ) {
            return Err(Error::Invalid);
        }
        mush_core::DefinitionDescriptor::new(
            mush_core::DefinitionKind::new("action").map_err(|_| Error::Invalid)?,
            mush_core::DefinitionId::new(action).map_err(|_| Error::Invalid)?,
            1,
            serde_json::json!({
                "format": 1,
                "protocol": "certified_core_integration_v1",
                "policy": self,
                "reservation_ic_seconds": RESERVATION_SECONDS,
                "supplier_consent": "acknowledged_exact_offer_v1",
                "input": "certified_origin_qty1",
                "title_recipient": "authenticated_personal_requester"
            }),
        )
        .map(|d| d.with_references(vec![self.recipe.clone()]))
        .map_err(|_| Error::Invalid)
    }

    pub fn action_ref(&self, action: &str) -> Result<DefinitionRef, Error> {
        mush_core::DefinitionRegistryBuilder::new()
            .register(
                mush_core::OwnerId::new("industry").map_err(|_| Error::Invalid)?,
                self.action_descriptor(action)?,
            )
            .map_err(|_| Error::Invalid)
    }
}

/// The actual native branch, derived from the present source vault's owner and
/// scope. It is retained only after the Economy owner repeats those checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiePayment {
    SupplierStock,
    RequesterSupplied,
}
impl DiePayment {
    pub(crate) const fn price(self, class: CoreClass) -> Amount {
        Amount::from_minor(match self {
            Self::SupplierStock => class.finished_minor(),
            Self::RequesterSupplied => class.integration_minor(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreFundingSubject {
    pub order: u64,
    pub intent: DefinitionDigest,
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub requester: ObjId,
    pub payer: AccountId,
    pub input_vault: VaultId,
    pub die_lot: u64,
    pub die_certificate: CommitToken,
    pub class: CoreClass,
    pub recipe: DefinitionRef,
    pub policy: DefinitionDigest,
    pub accepted_at: ClockStamp,
    pub expires_at: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectWire {
    order: u64,
    intent: DefinitionDigest,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    requester: ObjId,
    payer: AccountId,
    input_vault: VaultId,
    die_lot: u64,
    die_certificate: CommitToken,
    class: CoreClass,
    recipe: DefinitionRef,
    policy: DefinitionDigest,
    accepted_at: ClockStamp,
    expires_at: ClockStamp,
}
impl<'de> Deserialize<'de> for CoreFundingSubject {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let w = SubjectWire::deserialize(decoder)?;
        let subject = Self {
            order: w.order,
            intent: w.intent,
            offer: w.offer,
            offer_digest: w.offer_digest,
            requester: w.requester,
            payer: w.payer,
            input_vault: w.input_vault,
            die_lot: w.die_lot,
            die_certificate: w.die_certificate,
            class: w.class,
            recipe: w.recipe,
            policy: w.policy,
            accepted_at: w.accepted_at,
            expires_at: w.expires_at,
        };
        subject.validate().map_err(serde::de::Error::custom)?;
        Ok(subject)
    }
}
impl CoreFundingSubject {
    pub fn validate(&self) -> Result<(), Error> {
        if self.intent.as_bytes() == &[0; 32]
            || self.offer_digest.as_bytes() == &[0; 32]
            || self.policy.as_bytes() == &[0; 32]
            || self.recipe.key().kind.as_str() != "recipe"
            || self.recipe.key().id.as_str() != "core_integrate"
            || self.recipe.version() != 1
            || !matches!(self.accepted_at.clock, ClockRef::WorldIc { generation } if generation > 0)
            || self.expires_at.clock != self.accepted_at.clock
            || self.accepted_at.at.checked_add(RESERVATION_SECONDS) != Some(self.expires_at.at)
        {
            return Err(Error::Invalid);
        }
        let value = serde_json::to_value(self).map_err(|_| Error::Invalid)?;
        mush_core::mechanics::canonical_json_bytes_with_limit(&value, SUBJECT_BYTES)
            .map_err(|_| Error::Capacity)?;
        Ok(())
    }

    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| Error::Invalid)?;
        let bytes = mush_core::mechanics::canonical_json_bytes_with_limit(&value, SUBJECT_BYTES)
            .map_err(|_| Error::Capacity)?;
        Ok(DefinitionDigest::from_bytes(Sha256::digest(bytes).into()))
    }
}
