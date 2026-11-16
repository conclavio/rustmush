//! Strict retained economic intent. These values are comparison data, never
//! owner authority, proof of a die's certification, or permission to post.
use super::{CoreIntegrationError as Error, SUBJECT_BYTES};
use crate::{AccountId, Amount, CatalogRef, InstrumentId, VaultId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The reservation, accepted to expiry: 28 IC days (7 until the owner's rule of 2026-11-04 that a
/// timer waiting on an absent player's response allows at least 28 IC days), the kernel audit's
/// [`mush_core::player_wait::REFIT_RESERVATION_SECONDS`].
pub(crate) const RESERVATION_SECONDS: i64 = mush_core::player_wait::REFIT_RESERVATION_SECONDS;
/// **The Class III end-user entitlement** (doc 17 §16; 17f M7.11): the instrument a principal
/// holds an account in to have a Class III core integrated and delivered. Issued by
/// [`super::register_end_user`] (doc 44 unit MW.23).
pub const CORE3_ENTITLEMENT: &str = "tacsim.licence.core3_end_user";

/// Physical governed core class. ROM is not a manufactured input or output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreClass {
    I,
    II,
    III,
}
impl CoreClass {
    /// **The v1 table's finished price** ([`CorePrices::V1`]): the prices the shipped hardware
    /// catalogue carries, kept as the test reference and the policy's serialization default. The
    /// native protocol never reads it: an integration charges its policy's
    /// [`CorePaymentPolicy::prices`] (doc 48 unit PL.7, Q-48-27).
    pub const fn finished_minor(self) -> i64 {
        CorePrices::V1.finished_minor(self)
    }
    /// The v1 table's integration fee ([`Self::finished_minor`]'s rule).
    pub const fn integration_minor(self) -> i64 {
        CorePrices::V1.integration_minor(self)
    }
    /// The v1 table's die price ([`Self::finished_minor`]'s rule).
    pub const fn die_minor(self) -> i64 {
        CorePrices::V1.die_minor(self)
    }
    pub const fn catalog_index(self) -> usize {
        match self {
            Self::I => 0,
            Self::II => 1,
            Self::III => 2,
        }
    }
}

/// **The governed cores' prices** (doc 48 unit PL.7, Q-48-27: "`CoreClass` prices become
/// content"): each class's finished price in minor units, and the integration's share of it,
/// per mille. The rest is the die's, paid to the supplier when the die is its stock. Trusted
/// composition takes them from the hardware catalogue and freezes them in the
/// [`CorePaymentPolicy`], so every intent retains the prices it was accepted at.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorePrices {
    pub finished_minor: [i64; 3],
    pub integration_permille: u16,
}
impl CorePrices {
    /// **The v1 table** (doc 17f M7): 3,000, 8,000 and 25,000 credits, a tenth of each to the
    /// integration. The shipped catalogue's prices, and what a policy that names none carries.
    pub const V1: Self = Self {
        finished_minor: [300_000, 800_000, 2_500_000],
        integration_permille: 100,
    };
    /// The highest finished price a class may carry, minor units (a hundred million credits).
    pub const MAX_FINISHED_MINOR: i64 = 10_000_000_000;
    #[must_use]
    pub const fn finished_minor(&self, class: CoreClass) -> i64 {
        self.finished_minor[class.catalog_index()]
    }
    /// The integration fee: the finished price's per-mille share, rounded down.
    #[must_use]
    pub const fn integration_minor(&self, class: CoreClass) -> i64 {
        self.finished_minor(class)
            .saturating_mul(self.integration_permille as i64)
            / 1000
    }
    /// The die's price: the finished price less the integration fee.
    #[must_use]
    pub const fn die_minor(&self, class: CoreClass) -> i64 {
        self.finished_minor(class)
            .saturating_sub(self.integration_minor(class))
    }
    /// Every class priced in `1..=MAX_FINISHED_MINOR`, the share in `1..=999` per mille, and both
    /// halves of every price at least one minor unit. The classes need not ascend.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming nothing more.
    pub fn validate(&self) -> Result<(), Error> {
        if !(1..=999).contains(&self.integration_permille)
            || self
                .finished_minor
                .iter()
                .any(|n| !(1..=Self::MAX_FINISHED_MINOR).contains(n))
        {
            return Err(Error::Invalid);
        }
        for class in [CoreClass::I, CoreClass::II, CoreClass::III] {
            if self.integration_minor(class) < 1 || self.die_minor(class) < 1 {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    fn v1() -> Self {
        Self::V1
    }
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` passes a reference
    fn is_v1(&self) -> bool {
        *self == Self::V1
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
    /// **The prices an integration charges** (doc 48 unit PL.7): the hardware catalogue's.
    /// Written only when they differ from [`CorePrices::V1`], so a policy at the v1 prices keeps
    /// its bytes and digest.
    #[serde(skip_serializing_if = "CorePrices::is_v1")]
    pub prices: CorePrices,
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
    #[serde(default = "CorePrices::v1", deserialize_with = "present_prices")]
    prices: CorePrices,
}
/// A present `prices` must differ from the v1 table, which is written by absence.
fn present_prices<'de, D: serde::Deserializer<'de>>(d: D) -> Result<CorePrices, D::Error> {
    let prices = CorePrices::deserialize(d)?;
    if prices.is_v1() {
        return Err(serde::de::Error::custom(
            "the v1 prices are written by absence",
        ));
    }
    Ok(prices)
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
            prices: w.prices,
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
        self.prices.validate()?;
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
    /// What the requester pays at `prices` (doc 48 unit PL.7: the policy's, never the v1 table).
    pub(crate) const fn price(self, prices: &CorePrices, class: CoreClass) -> Amount {
        Amount::from_minor(match self {
            Self::SupplierStock => prices.finished_minor(class),
            Self::RequesterSupplied | Self::Uncertified => prices.integration_minor(class),
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
            prices: CorePrices::V1,
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
            DiePayment::Uncertified.price(&CorePrices::V1, CoreClass::III),
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

    /// **Doc 48 unit PL.7: the policy's prices.** The v1 table validates and is written by
    /// absence, so a policy at it keeps its bytes; other prices are written, round-trip, and are
    /// what the requester pays. Out-of-bounds prices refuse the policy, and a present v1 table is
    /// refused on decode.
    #[test]
    fn pl7_the_policy_s_prices_are_bounded_and_written_by_absence() {
        CorePrices::V1.validate().expect("the v1 table");
        let value = serde_json::to_value(chartered()).expect("encodes");
        assert!(
            value.get("prices").is_none(),
            "the v1 table is written by absence"
        );
        let priced = CorePaymentPolicy {
            prices: CorePrices {
                finished_minor: [450_000, 900_000, 3_000_000],
                integration_permille: 150,
            },
            ..chartered()
        };
        priced.validate().expect("other prices validate");
        let value = serde_json::to_value(&priced).expect("encodes");
        assert!(value.get("prices").is_some(), "other prices are written");
        let back: CorePaymentPolicy = serde_json::from_value(value).expect("decodes");
        assert_eq!(back, priced);
        assert_ne!(
            priced.digest().expect("a digest"),
            chartered().digest().expect("a digest")
        );
        assert_eq!(
            DiePayment::SupplierStock.price(&priced.prices, CoreClass::II),
            Amount::from_minor(900_000)
        );
        assert_eq!(
            DiePayment::RequesterSupplied.price(&priced.prices, CoreClass::II),
            Amount::from_minor(135_000)
        );
        for bad in [
            CorePrices {
                finished_minor: [0, 800_000, 2_500_000],
                ..CorePrices::V1
            },
            CorePrices {
                finished_minor: [300_000, 800_000, CorePrices::MAX_FINISHED_MINOR + 1],
                ..CorePrices::V1
            },
            CorePrices {
                integration_permille: 0,
                ..CorePrices::V1
            },
            CorePrices {
                integration_permille: 1_000,
                ..CorePrices::V1
            },
            // A price whose integration share rounds to nothing.
            CorePrices {
                finished_minor: [5, 800_000, 2_500_000],
                integration_permille: 100,
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
            let policy = CorePaymentPolicy {
                prices: bad,
                ..chartered()
            };
            assert!(policy.validate().is_err(), "{bad:?}");
        }
        let mut written = serde_json::to_value(chartered()).expect("encodes");
        written["prices"] = serde_json::to_value(CorePrices::V1).expect("encodes");
        assert!(serde_json::from_value::<CorePaymentPolicy>(written).is_err());
    }
}
