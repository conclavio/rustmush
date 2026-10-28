//! Strict retained fitting terms. They are data, never authority to charge,
//! withdraw a core, replace a unit, or bypass a title/custody predicate.
use super::{FITTING_MINOR, RESERVATION_SECONDS, RefitError as Error, SUBJECT_BYTES};
use crate::core_integration::{CoreClass, ScopedCoreAccount};
use crate::{AccountId, Amount, InstrumentId};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::SystemPrincipalId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitPaymentPolicy {
    pub issuer: SystemPrincipalId,
    pub instrument: InstrumentId,
    pub provider: ScopedCoreAccount,
    pub factory: ObjId,
    pub recipe: DefinitionRef,
    /// Class I, II, III, in this exact order. Trusted content validates the
    /// finite compatible family through the actual Tactics definitions.
    pub fitted_units: [DefinitionRef; 3],
    /// **Further governed families** (doc 17h cut M8b C1: the surface rig's
    /// roots), each Class I, II, III. Absent when empty, so a line-only policy
    /// digests as it always did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub families: Vec<[DefinitionRef; 3]>,
    /// **The ungoverning refit** (doc 17h cut M8b C2; doc 17 §10, D-UM-42): a
    /// second recipe at its own facility, fitting the ungoverned twins. Absent
    /// when not composed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ungovern: Option<UngovernTerms>,
    /// **Coreless bases** (doc 17h cut M8b G2, the lead's ruling): a chassis with no core
    /// mount that `core_refit` fits a core into, becoming its family's root of the incoming
    /// class. `core_ungovern` never fits into one. Absent when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coreless: Vec<CorelessBase>,
}

/// **A coreless chassis and the governed family it fits into** (doc 17h cut M8b G2):
/// `family` indexes `[fitted_units] + families` (0 is the line family).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorelessBase {
    pub base: DefinitionRef,
    pub family: u8,
}

/// **The ungoverning recipe's own terms** (doc 17h cut M8b C2): the facility
/// holding `core_ungovern` (never also `core_refit`), its provider, its
/// recipe, its fee (`ungovern_minor`, provisional: the fitting fee) and the
/// ungoverned twin of every governed family, in the families' order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UngovernTerms {
    pub provider: ScopedCoreAccount,
    pub factory: ObjId,
    pub recipe: DefinitionRef,
    pub fee_minor: i64,
    /// One `[I, II, III]` per governed family: `fitted_units` first, then
    /// `families` in order.
    pub twins: Vec<[DefinitionRef; 3]>,
}

/// **Which recipe a refit runs** (doc 17h cut M8b C2): the chartered
/// `core_refit` or the ungoverning `core_ungovern`. A subject or offer that
/// carries no kind is a `core_refit`, so every pre-M8b record reads as it did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefitKind {
    #[default]
    CoreRefit,
    CoreUngovern,
}
impl RefitKind {
    #[must_use]
    pub const fn is_refit(&self) -> bool {
        matches!(self, Self::CoreRefit)
    }
    /// The recipe's definition id, which is also its facility capability word.
    #[must_use]
    pub const fn recipe_id(self) -> &'static str {
        match self {
            Self::CoreRefit => "core_refit",
            Self::CoreUngovern => "core_ungovern",
        }
    }
    /// The Org capability an offering operator needs on the site's scope.
    #[must_use]
    pub const fn operator_capability(self) -> &'static str {
        match self {
            Self::CoreRefit => "industry.core.refit",
            Self::CoreUngovern => "industry.core.ungovern",
        }
    }
}

/// One recipe's site: who is paid, where, under what recipe, for how much.
#[derive(Clone, Copy, Debug)]
pub struct RefitSite<'a> {
    pub provider: &'a ScopedCoreAccount,
    pub factory: ObjId,
    pub recipe: &'a DefinitionRef,
    pub fee_minor: i64,
}

fn recipe_ok(recipe: &DefinitionRef, id: &str) -> bool {
    recipe.key().kind.as_str() == "recipe"
        && recipe.key().id.as_str() == id
        && recipe.version() == 1
        && recipe.digest().as_bytes() != &[0; 32]
}

impl RefitPaymentPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if !recipe_ok(&self.recipe, "core_refit") {
            return Err(Error::Invalid);
        }
        let roots = self.roots().collect::<Vec<_>>();
        for (i, root) in roots.iter().enumerate() {
            if root.key().kind.as_str() != "unit"
                || root.key().owner.as_str() != "tacsim"
                || root.digest().as_bytes() == &[0; 32]
                || roots.iter().skip(i + 1).any(|r| r == root)
            {
                return Err(Error::Invalid);
            }
        }
        if let Some(u) = &self.ungovern
            && (!recipe_ok(&u.recipe, "core_ungovern")
                || u.factory == self.factory
                || u.provider.account == self.provider.account
                || !(1..=FITTING_MINOR.saturating_mul(1000)).contains(&u.fee_minor)
                || u.twins.len() != self.families.len().checked_add(1).ok_or(Error::Invalid)?)
        {
            return Err(Error::Invalid);
        }
        if self
            .coreless
            .iter()
            .any(|c| usize::from(c.family) > self.families.len())
        {
            return Err(Error::Invalid);
        }
        bounded(self, SUBJECT_BYTES)?;
        Ok(())
    }
    /// Every root this policy touches: the governed families, then the twins, then the
    /// coreless bases (doc 17h cut M8b G2).
    pub fn roots(&self) -> impl Iterator<Item = &DefinitionRef> {
        self.governed_families()
            .chain(self.ungovern.iter().flat_map(|u| u.twins.iter()))
            .flatten()
            .chain(self.coreless.iter().map(|c| &c.base))
    }
    fn governed_families(&self) -> impl Iterator<Item = &[DefinitionRef; 3]> {
        std::iter::once(&self.fitted_units).chain(self.families.iter())
    }
    /// The governed fitting's fee (`core_refit`).
    pub fn price(&self) -> Amount {
        Amount::from_minor(FITTING_MINOR)
    }
    /// The site `kind` runs at. A `core_ungovern` on a policy composed
    /// without the ungoverning terms is refused.
    pub fn site(&self, kind: RefitKind) -> Result<RefitSite<'_>, Error> {
        match kind {
            RefitKind::CoreRefit => Ok(RefitSite {
                provider: &self.provider,
                factory: self.factory,
                recipe: &self.recipe,
                fee_minor: FITTING_MINOR,
            }),
            RefitKind::CoreUngovern => {
                let u = self.ungovern.as_ref().ok_or(Error::Invalid)?;
                Ok(RefitSite {
                    provider: &u.provider,
                    factory: u.factory,
                    recipe: &u.recipe,
                    fee_minor: u.fee_minor,
                })
            }
        }
    }
    /// The fee `kind` charges.
    pub fn price_for(&self, kind: RefitKind) -> Result<Amount, Error> {
        Ok(Amount::from_minor(self.site(kind)?.fee_minor))
    }
    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest(self, SUBJECT_BYTES)
    }
    /// The governed line root of `class` (the first family's).
    pub fn target(&self, class: CoreClass) -> Result<&DefinitionRef, Error> {
        self.fitted_units
            .get(class.catalog_index())
            .ok_or(Error::Invalid)
    }
    /// **The root `kind` fits to a hull whose current root is `before`** (doc
    /// 17h cut M8b): the family `before` belongs to (as a governed root or a
    /// twin), its governed root of `class` for `core_refit`, its twin for
    /// `core_ungovern`. A root of no family is refused.
    pub fn target_for(
        &self,
        kind: RefitKind,
        before: &DefinitionRef,
        class: CoreClass,
    ) -> Result<&DefinitionRef, Error> {
        // Doc 17h cut M8b G2 (the lead's ruling): `core_refit` fits a core into a coreless
        // base, becoming its family's root of the class; `core_ungovern` rewrites a fitted
        // core only, so it refuses a coreless base.
        if let Some(base) = self.coreless.iter().find(|c| &c.base == before) {
            return match kind {
                RefitKind::CoreRefit => self
                    .governed_families()
                    .nth(usize::from(base.family))
                    .and_then(|f| f.get(class.catalog_index()))
                    .ok_or(Error::Invalid),
                RefitKind::CoreUngovern => Err(Error::Invalid),
            };
        }
        let twins: &[[DefinitionRef; 3]] = self.ungovern.as_ref().map_or(&[], |u| &u.twins);
        let index = self
            .governed_families()
            .position(|f| f.contains(before))
            .or_else(|| twins.iter().position(|f| f.contains(before)))
            .ok_or(Error::Invalid)?;
        let family = match kind {
            RefitKind::CoreRefit => self.governed_families().nth(index),
            RefitKind::CoreUngovern => twins.get(index),
        }
        .ok_or(Error::Invalid)?;
        family.get(class.catalog_index()).ok_or(Error::Invalid)
    }
    /// Whether `target` is a `kind` root of `class` in some family.
    pub fn admits_target(&self, kind: RefitKind, target: &DefinitionRef, class: CoreClass) -> bool {
        let i = class.catalog_index();
        match kind {
            RefitKind::CoreRefit => self.governed_families().any(|f| f.get(i) == Some(target)),
            RefitKind::CoreUngovern => self
                .ungovern
                .iter()
                .flat_map(|u| u.twins.iter())
                .any(|f| f.get(i) == Some(target)),
        }
    }
    /// Whether `root` is any family's root, governed or a twin.
    pub fn fits(&self, root: &DefinitionRef) -> bool {
        self.roots().any(|r| r == root)
    }
    pub fn action_descriptor(
        &self,
        action: &str,
    ) -> Result<mush_core::DefinitionDescriptor, Error> {
        self.validate()?;
        if action == super::INSTALL_ACTION {
            return mush_core::core_exchange::action_descriptor().map_err(|_| Error::Invalid);
        }
        let kind = mush_core::DefinitionKind::new("action").map_err(|_| Error::Invalid)?;
        let id = mush_core::DefinitionId::new(action).map_err(|_| Error::Invalid)?;
        if action == super::CANCEL_ACTION {
            return mush_core::DefinitionDescriptor::new(kind,id,1,serde_json::json!({
                "format":1,"protocol":"certified_core_refit_cancel_v1",
                "authority":"actual_acknowledged_requester","effect":"release_exact_fee_and_slot",
                "current_recipe_required":false
            })).map_err(|_| Error::Invalid);
        }
        if !matches!(action, super::OFFER_ACTION | super::ACCEPT_ACTION) {
            return Err(Error::Invalid);
        }
        let mut refs = vec![self.recipe.clone()];
        refs.extend(self.ungovern.iter().map(|u| u.recipe.clone()));
        refs.extend(self.roots().cloned());
        refs.sort();
        refs.dedup();
        // `fee_minor` is the chartered `core_refit`'s fee; the ungoverning recipe's is the
        // policy's own `ungovern.fee_minor` (doc 17h cut M8b G3), carried in `policy`.
        mush_core::DefinitionDescriptor::new(kind,id,1,serde_json::json!({
            "format":1,"protocol":"certified_core_refit_v1","policy":self,
            "fee_minor":FITTING_MINOR,"reservation_ic_seconds":RESERVATION_SECONDS,
            "incoming":"delivered_certified_core","supplier_consent":"acknowledged_exact_offer_v1"
        })).map(|d|d.with_references(refs)).map_err(|_| Error::Invalid)
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefitSubject {
    pub order: u64,
    pub intent: DefinitionDigest,
    pub offer: CommitToken,
    pub offer_digest: DefinitionDigest,
    pub requester: ObjId,
    pub payer: AccountId,
    /// Actual native manufacture acceptance; not a caller's catalogue label.
    pub core_acceptance: CommitToken,
    pub incoming: ObjId,
    pub hull: ObjId,
    pub unit_revision: u64,
    pub target: DefinitionRef,
    pub class: CoreClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displaced: Option<ObjId>,
    pub policy: DefinitionDigest,
    pub accepted_at: ClockStamp,
    pub expires_at: ClockStamp,
    /// The recipe (doc 17h cut M8b C2); absent for `core_refit`.
    #[serde(default, skip_serializing_if = "RefitKind::is_refit")]
    pub kind: RefitKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    order: u64,
    intent: DefinitionDigest,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    requester: ObjId,
    payer: AccountId,
    core_acceptance: CommitToken,
    incoming: ObjId,
    hull: ObjId,
    unit_revision: u64,
    target: DefinitionRef,
    class: CoreClass,
    #[serde(default, deserialize_with = "present_object")]
    displaced: Option<ObjId>,
    policy: DefinitionDigest,
    accepted_at: ClockStamp,
    expires_at: ClockStamp,
    #[serde(default, deserialize_with = "present_ungovern")]
    kind: RefitKind,
}
/// A present `kind` must be the ungoverning one: `core_refit` is written by
/// absence only, so one subject has one encoding.
pub fn present_ungovern<'de, D: serde::Deserializer<'de>>(d: D) -> Result<RefitKind, D::Error> {
    match RefitKind::deserialize(d)? {
        RefitKind::CoreUngovern => Ok(RefitKind::CoreUngovern),
        RefitKind::CoreRefit => Err(serde::de::Error::custom("core_refit is written by absence")),
    }
}
fn present_object<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<ObjId>, D::Error> {
    ObjId::deserialize(d).map(Some)
}
impl<'de> Deserialize<'de> for RefitSubject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let s = Self {
            order: w.order,
            intent: w.intent,
            offer: w.offer,
            offer_digest: w.offer_digest,
            requester: w.requester,
            payer: w.payer,
            core_acceptance: w.core_acceptance,
            incoming: w.incoming,
            hull: w.hull,
            unit_revision: w.unit_revision,
            target: w.target,
            class: w.class,
            displaced: w.displaced,
            policy: w.policy,
            accepted_at: w.accepted_at,
            expires_at: w.expires_at,
            kind: w.kind,
        };
        s.validate().map_err(serde::de::Error::custom)?;
        Ok(s)
    }
}
impl RefitSubject {
    pub fn validate(&self) -> Result<(), Error> {
        if self.intent.as_bytes() == &[0; 32]
            || self.offer_digest.as_bytes() == &[0; 32]
            || self.policy.as_bytes() == &[0; 32]
            || self.incoming == self.hull
            || [self.incoming, self.hull].contains(&self.requester)
            || self
                .displaced
                .is_some_and(|v| [self.incoming, self.hull, self.requester].contains(&v))
            || self.target.key().kind.as_str() != "unit"
            || self.target.key().owner.as_str() != "tacsim"
            || !matches!(self.accepted_at.clock,ClockRef::WorldIc{generation}if generation>0)
            || self.expires_at.clock != self.accepted_at.clock
            || self.accepted_at.at.checked_add(RESERVATION_SECONDS) != Some(self.expires_at.at)
        {
            return Err(Error::Invalid);
        }
        bounded(self, SUBJECT_BYTES)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        digest(self, SUBJECT_BYTES)
    }
    pub(crate) fn matches_policy(&self, p: &RefitPaymentPolicy) -> Result<(), Error> {
        self.validate()?;
        p.validate()?;
        let site = p.site(self.kind)?;
        if self.policy != p.digest()?
            || !p.admits_target(self.kind, &self.target, self.class)
            || self.payer == site.provider.account
            || [self.incoming, self.hull, self.requester].contains(&site.factory)
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}
pub(super) fn bounded<T: Serialize>(v: &T, n: usize) -> Result<(), Error> {
    bytes(v, n).map(|_| ())
}
pub(super) fn bytes<T: Serialize>(v: &T, n: usize) -> Result<Vec<u8>, Error> {
    mush_core::mechanics::canonical_json_bytes_with_limit(
        &serde_json::to_value(v).map_err(|_| Error::Invalid)?,
        n,
    )
    .map_err(|_| Error::Capacity)
}
pub(super) fn digest<T: Serialize>(v: &T, n: usize) -> Result<DefinitionDigest, Error> {
    Ok(DefinitionDigest::from_bytes(
        Sha256::digest(bytes(v, n)?).into(),
    ))
}

#[cfg(test)]
mod m8b_tests {
    //! Doc 17h cut M8b (G11): the policy's two recipes, its families, twins and coreless
    //! bases, and the one encoding of a subject's kind.
    use super::*;
    use crate::core_integration::CoreClass;
    use mush_core::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    };
    use mush_org::{OrgId, OrgScopeId};

    fn reference(owner: &str, kind: &str, id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(owner).expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new(kind).expect("a kind"),
                    DefinitionId::new(id).expect("an id"),
                    1,
                    serde_json::json!({"fixture": id}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }
    fn family(prefix: &str) -> [DefinitionRef; 3] {
        ["i", "ii", "iii"].map(|c| reference("tacsim", "unit", &format!("{prefix}_{c}")))
    }
    fn account(n: u64) -> ScopedCoreAccount {
        ScopedCoreAccount {
            account: crate::AccountId(n),
            organization: OrgId(n),
            scope: OrgScopeId(n),
        }
    }
    fn line_only() -> RefitPaymentPolicy {
        RefitPaymentPolicy {
            issuer: SystemPrincipalId::new("hardware.factory").expect("an issuer"),
            instrument: crate::InstrumentId::new("tacsim.credit").expect("an instrument"),
            provider: account(1),
            factory: ObjId(10),
            recipe: reference("tacsim", "recipe", "core_refit"),
            fitted_units: family("line"),
            families: Vec::new(),
            ungovern: None,
            coreless: Vec::new(),
        }
    }
    fn composed() -> RefitPaymentPolicy {
        RefitPaymentPolicy {
            families: vec![family("rig")],
            ungovern: Some(UngovernTerms {
                provider: account(2),
                factory: ObjId(20),
                recipe: reference("tacsim", "recipe", "core_ungovern"),
                fee_minor: 25_000,
                twins: vec![family("line_twin"), family("rig_twin")],
            }),
            coreless: vec![CorelessBase {
                base: reference("tacsim", "unit", "rig_base"),
                family: 1,
            }],
            ..line_only()
        }
    }

    #[test]
    fn m8b_a_line_only_policy_writes_none_of_the_new_keys() {
        let value = serde_json::to_value(line_only()).expect("encodes");
        for key in ["families", "ungovern", "coreless"] {
            assert!(value.get(key).is_none(), "{key}");
        }
        assert!(line_only().validate().is_ok() && composed().validate().is_ok());
    }

    #[test]
    fn m8b_a_present_kind_must_be_the_ungoverning_one() {
        assert!(present_ungovern(serde_json::json!("core_refit")).is_err());
        assert_eq!(
            present_ungovern(serde_json::json!("core_ungovern")).expect("reads"),
            RefitKind::CoreUngovern
        );
    }

    #[test]
    fn m8b_the_ungoverning_terms_validate() {
        type Change = Box<dyn Fn(&mut UngovernTerms)>;
        let bad: [Change; 5] = [
            Box::new(|u| u.factory = ObjId(10)),
            Box::new(|u| u.provider.account = crate::AccountId(1)),
            Box::new(|u| u.fee_minor = 0),
            Box::new(|u| {
                u.twins.pop();
            }),
            Box::new(|u| u.recipe = reference("tacsim", "recipe", "core_refit")),
        ];
        for change in bad {
            let mut policy = composed();
            if let Some(u) = policy.ungovern.as_mut() {
                change(u);
            }
            assert!(policy.validate().is_err());
        }
        let mut stray = composed();
        stray.coreless[0].family = 2;
        assert!(stray.validate().is_err(), "a base of no family");
    }

    #[test]
    fn m8b_target_for_reads_the_family_the_recipe_and_the_coreless_base() {
        let p = composed();
        let (line, rig) = (family("line"), family("rig"));
        let (line_twin, rig_twin) = (family("line_twin"), family("rig_twin"));
        let base = reference("tacsim", "unit", "rig_base");
        let refit = RefitKind::CoreRefit;
        let ungovern = RefitKind::CoreUngovern;
        assert_eq!(
            p.target_for(refit, &line[1], CoreClass::III).ok(),
            Some(&line[2])
        );
        assert_eq!(
            p.target_for(ungovern, &rig[0], CoreClass::I).ok(),
            Some(&rig_twin[0])
        );
        // A twin back to governed under the chartered refit (lawful restoration).
        assert_eq!(
            p.target_for(refit, &line_twin[2], CoreClass::II).ok(),
            Some(&line[1])
        );
        // G2: the coreless base takes its family's root under `core_refit` only.
        assert_eq!(
            p.target_for(refit, &base, CoreClass::II).ok(),
            Some(&rig[1])
        );
        assert!(p.target_for(ungovern, &base, CoreClass::II).is_err());
        assert!(p.fits(&base) && p.roots().any(|r| r == &base));
        assert!(p.admits_target(ungovern, &rig_twin[2], CoreClass::III));
        assert!(!p.admits_target(refit, &rig_twin[2], CoreClass::III));
        assert_eq!(
            p.price_for(ungovern).expect("composed").minor(),
            25_000,
            "the ungoverning fee is its own"
        );
        assert!(line_only().site(ungovern).is_err());
    }
}
