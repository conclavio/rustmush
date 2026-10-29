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

/// **Where a die came from** (doc 17h cut M8b-2, H1): the funding subject's and the title
/// birth's source word. `Certified` is the importer's `certified_core_die.v1` stock and is
/// written by absence; `Heirloom` (`enclave_heirloom.v1`, an ungoverned Class III) and `Grey`
/// (no origin recorded, of a declared class) integrate only at the ungoverning site
/// ([`CorePaymentPolicy::site`]). Industry re-exports it beside its pinned `DieClass`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DieSource {
    #[default]
    Certified,
    Heirloom,
    Grey,
}
impl DieSource {
    /// `true` for the importer's certified stock: the word written by absence.
    #[must_use]
    pub const fn is_certified(&self) -> bool {
        matches!(self, Self::Certified)
    }
}
/// A present `source` must be an uncertified one: `certified` is written by absence.
fn present_uncertified<'de, D: serde::Deserializer<'de>>(d: D) -> Result<DieSource, D::Error> {
    match DieSource::deserialize(d)? {
        DieSource::Certified => Err(serde::de::Error::custom(
            "a certified die is written by absence",
        )),
        source => Ok(source),
    }
}

/// **The ungoverning integration site** (doc 17h cut M8b-2, H2; doc 17 §10, D-UM-42): the
/// facility holding `core_ungovern` (and neither `core_integration` nor `core_refit`) where a
/// heirloom or grey die is integrated with no certificate, the organization account its
/// integration fee is paid to, and its own completed-work container (fix round 1, L2: a
/// back-room core never passes through the chartered factory's). Absent when the composition
/// names no such shop.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UngovernedIntegration {
    pub integrator: ScopedCoreAccount,
    pub factory: ObjId,
    pub output_container: ObjId,
}

/// The site one source integrates at: the chartered factory for a certified die, the
/// ungoverning one for a heirloom or grey die.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrationSite<'a> {
    pub integrator: &'a ScopedCoreAccount,
    pub factory: ObjId,
    /// Where the site's completed cores are born.
    pub output_container: ObjId,
    /// The facility capability the site's facility holds.
    pub capability: &'static str,
    /// The Org capability the site's operator holds on the integrator's scope.
    pub operator_capability: &'static str,
}

/// Exact configured account and actual coherent organization compartment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedCoreAccount {
    pub account: AccountId,
    pub organization: OrgId,
    pub scope: OrgScopeId,
}

/// Closed paid promise selected only by trusted factory composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreDeliveryPromise {
    PersonalCustodyV1,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<CoreDeliveryPromise>,
    /// The ungoverning site (doc 17h cut M8b-2, H2). Absent when not composed, so a policy
    /// without it keeps its bytes and digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ungoverned: Option<UngovernedIntegration>,
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
    #[serde(default, deserialize_with = "present_delivery")]
    delivery: Option<CoreDeliveryPromise>,
    #[serde(default, deserialize_with = "present_ungoverned")]
    ungoverned: Option<UngovernedIntegration>,
}
fn present_ungoverned<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<UngovernedIntegration>, D::Error> {
    UngovernedIntegration::deserialize(d).map(Some)
}
fn present_delivery<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<CoreDeliveryPromise>, D::Error> {
    CoreDeliveryPromise::deserialize(d).map(Some)
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
            delivery: w.delivery,
            ungoverned: w.ungoverned,
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
        // The shop is its own organization, factory and container (fix round 1, L2 and L4):
        // nothing of the chartered site is the back room's.
        if let Some(u) = &self.ungoverned
            && ([self.supplier.account, self.integrator.account].contains(&u.integrator.account)
                || u.integrator.organization == self.integrator.organization
                || u.factory == u.output_container
                || [self.factory, self.output_container].contains(&u.factory)
                || [self.factory, self.output_container].contains(&u.output_container))
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

    /// **The site a die of `certified`-ness integrates at** (doc 17h cut M8b-2, H2): a
    /// certified die at the chartered factory (`core_integration`, `industry.core.integrate`),
    /// a heirloom or grey die only at the ungoverning one (`core_ungovern`,
    /// `industry.core.ungovern`). An uncertified die on a policy composed without the
    /// ungoverning site is refused.
    pub fn site(&self, certified: bool) -> Result<IntegrationSite<'_>, Error> {
        if certified {
            return Ok(IntegrationSite {
                integrator: &self.integrator,
                factory: self.factory,
                output_container: self.output_container,
                capability: "core_integration",
                operator_capability: "industry.core.integrate",
            });
        }
        let u = self.ungoverned.as_ref().ok_or(Error::Invalid)?;
        Ok(IntegrationSite {
            integrator: &u.integrator,
            factory: u.factory,
            output_container: u.output_container,
            capability: "core_ungovern",
            operator_capability: "industry.core.ungovern",
        })
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
        if action == "deliver_core_output" {
            if self.delivery.is_none() {
                return Err(Error::Invalid);
            }
            return mush_core::DefinitionDescriptor::new(
                mush_core::DefinitionKind::new("action").map_err(|_| Error::Invalid)?,
                mush_core::DefinitionId::new(action).map_err(|_| Error::Invalid)?, 1,
                serde_json::json!({"format":1,"protocol":"certified_core_delivery_v1","authority":"actual_completed_requester","effect":"ordinary_personal_custody","current_recipe_required":false,"core_receipt_bytes":16384,"industry_receipt_bytes":4096}),
            ).map_err(|_| Error::Invalid);
        }
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
                "protocol": if self.delivery.is_some() { "certified_core_integration_delivery_v2" } else { "certified_core_integration_v1" },
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
    /// A heirloom or grey die the requester brings to the ungoverning site (doc 17h cut
    /// M8b-2, H2): the integration fee only, paid to that site's integrator.
    Uncertified,
}
impl DiePayment {
    pub(crate) const fn price(self, class: CoreClass) -> Amount {
        Amount::from_minor(match self {
            Self::SupplierStock => class.finished_minor(),
            Self::RequesterSupplied | Self::Uncertified => class.integration_minor(),
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
    /// The die's issuing operation: the certificate's for a certified die, the staff issue's
    /// for a heirloom or grey one (doc 17h cut M8b-2).
    pub die_certificate: CommitToken,
    pub class: CoreClass,
    pub recipe: DefinitionRef,
    pub policy: DefinitionDigest,
    pub accepted_at: ClockStamp,
    pub expires_at: ClockStamp,
    /// Where the die came from (doc 17h cut M8b-2, H1). Absent for a certified die.
    #[serde(default, skip_serializing_if = "DieSource::is_certified")]
    pub source: DieSource,
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
    #[serde(default, deserialize_with = "present_uncertified")]
    source: DieSource,
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
            source: w.source,
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
            || (self.source == DieSource::Heirloom && self.class != CoreClass::III)
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

#[cfg(test)]
mod m8b2_tests {
    //! Doc 17h cut M8b-2 (H1, H2): the die's source word, written by absence for a certified
    //! die, and the two integration sites.
    use super::*;
    use mush_core::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };

    fn recipe() -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("tacsim").expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new("recipe").expect("a kind"),
                    DefinitionId::new("core_integrate").expect("an id"),
                    1,
                    serde_json::json!({"fixture": "core_integrate"}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }
    fn account(n: u64) -> ScopedCoreAccount {
        ScopedCoreAccount {
            account: AccountId(n),
            organization: OrgId(n),
            scope: OrgScopeId(n),
        }
    }
    fn chartered() -> CorePaymentPolicy {
        CorePaymentPolicy {
            issuer: SystemPrincipalId::new("hardware.factory").expect("an issuer"),
            instrument: InstrumentId::new("tacsim.credit").expect("an instrument"),
            supplier: account(1),
            integrator: account(2),
            supplier_stock: [VaultId(1), VaultId(2)],
            factory: ObjId(10),
            output_container: ObjId(11),
            recipe: recipe(),
            catalogs: ["a", "b", "c"].map(|code| CatalogRef {
                ruleset: "tacsim".into(),
                code: code.into(),
            }),
            delivery: None,
            ungoverned: None,
        }
    }
    fn composed() -> CorePaymentPolicy {
        CorePaymentPolicy {
            ungoverned: Some(UngovernedIntegration {
                integrator: account(3),
                factory: ObjId(20),
                output_container: ObjId(21),
            }),
            ..chartered()
        }
    }

    #[test]
    fn m8b2_a_certified_source_and_an_absent_site_are_written_by_absence() {
        let value = serde_json::to_value(chartered()).expect("encodes");
        assert!(value.get("ungoverned").is_none());
        assert!(present_uncertified(serde_json::json!("certified")).is_err());
        for (word, source) in [("heirloom", DieSource::Heirloom), ("grey", DieSource::Grey)] {
            assert_eq!(
                present_uncertified(serde_json::json!(word)).expect("reads"),
                source
            );
        }
        let decoded: CorePaymentPolicy =
            serde_json::from_value(serde_json::to_value(composed()).expect("encodes"))
                .expect("decodes");
        assert_eq!(decoded, composed());
    }

    #[test]
    fn m8b2_each_source_integrates_at_its_own_site() {
        let policy = composed();
        let governed = policy.site(true).expect("the chartered site");
        assert_eq!(
            (governed.factory, governed.integrator, governed.capability),
            (ObjId(10), &account(2), "core_integration")
        );
        assert_eq!(governed.operator_capability, "industry.core.integrate");
        let back_room = policy.site(false).expect("the ungoverning site");
        assert_eq!(
            (
                back_room.factory,
                back_room.integrator,
                back_room.capability
            ),
            (ObjId(20), &account(3), "core_ungovern")
        );
        assert_eq!(back_room.operator_capability, "industry.core.ungovern");
        assert!(
            chartered().site(false).is_err(),
            "no shop, no uncertified die"
        );
        assert_eq!(
            DiePayment::Uncertified.price(CoreClass::III),
            Amount::from_minor(CoreClass::III.integration_minor())
        );
        assert_eq!(
            (governed.output_container, back_room.output_container),
            (ObjId(11), ObjId(21)),
            "a back-room core is born in the shop's own container (fix round 1, L2)"
        );
        // The shop is none of the chartered site: not its factory or container, not paid as
        // its supplier or integrator, not its integrator's organization; and its own factory
        // is not its container.
        let site =
            |integrator: ScopedCoreAccount, factory: u32, output: u32| UngovernedIntegration {
                integrator,
                factory: ObjId(factory),
                output_container: ObjId(output),
            };
        let same_org = ScopedCoreAccount {
            account: AccountId(9),
            organization: OrgId(2),
            scope: OrgScopeId(9),
        };
        for bad in [
            site(account(3), 10, 21),
            site(account(3), 11, 21),
            site(account(3), 20, 10),
            site(account(3), 20, 11),
            site(account(3), 20, 20),
            site(account(2), 20, 21),
            site(account(1), 20, 21),
            site(same_org, 20, 21),
        ] {
            let policy = CorePaymentPolicy {
                ungoverned: Some(bad),
                ..chartered()
            };
            assert!(policy.validate().is_err());
        }
    }
}
