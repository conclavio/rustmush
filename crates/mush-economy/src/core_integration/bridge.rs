//! Same-slot dispatch and complete native replay. No Industry readiness cycle.
use super::{
    ACCEPT_ACTION, CANCEL_ACTION, COMPLETE_ACTION, CoreIntegrationConfig,
    CoreIntegrationError as Error, CorePaymentHistory, HOLD_PURPOSE, NAMESPACE,
};
use crate::{EconomicAsset, EconomyStore, Hold, HoldId};
use mush_core::{DefinitionRef, ModError, ObjId, Registrar};
use std::collections::{BTreeMap, BTreeSet};
impl std::fmt::Debug for CoreIntegrationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreIntegrationConfig")
            .field("policy", &self.policy)
            .field("industry_owner", &self.industry_owner)
            .finish_non_exhaustive()
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &CoreIntegrationConfig,
) -> Result<[DefinitionRef; 3], ModError> {
    config
        .policy
        .validate()
        .map_err(|e| ModError::msg(e.to_string()))?;
    let actions = [
        config.policy.action_ref(ACCEPT_ACTION),
        config.policy.action_ref(COMPLETE_ACTION),
        config.policy.action_ref(CANCEL_ACTION),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| ModError::msg(e.to_string()))?;
    let actions: [DefinitionRef; 3] = actions
        .try_into()
        .map_err(|_| ModError::msg("Core action count"))?;
    let mut references = vec![config.industry_owner.clone(), config.policy.recipe.clone()];
    references.extend(actions.iter().cloned());
    references.sort();
    let descriptor = mush_core::DefinitionDescriptor::new(
        mush_core::DefinitionKind::new("core_payment_policy")
            .map_err(|e| ModError::msg(e.to_string()))?,
        mush_core::DefinitionId::new("core_integration")
            .map_err(|e| ModError::msg(e.to_string()))?,
        1,
        serde_json::to_value(&config.policy).map_err(|e| ModError::msg(e.to_string()))?,
    )
    .map_err(|e| ModError::msg(e.to_string()))?
    .with_references(references);
    let expected = descriptor.clone();
    reg.definition_validator(
        descriptor.kind().clone(),
        move |context: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let d = context.definition();
            if d.reference().key().id == *expected.id()
                && d.reference().version() == expected.version()
                && d.payload() == expected.payload()
                && d.references() == expected.references()
            {
                Ok(())
            } else {
                Err("Core payment composition differs".into())
            }
        },
    )?;
    reg.definition(descriptor)?;
    Ok(actions)
}
pub(crate) fn retained_hold(hold: &Hold) -> bool {
    hold.holder.namespace == NAMESPACE
        || matches!(&hold.purpose,crate::HoldPurpose::Reservation{kind,..}if kind==HOLD_PURPOSE)
}
pub(crate) fn recover(
    fold: &mut crate::fold::FoldState,
    holds: &BTreeMap<HoldId, Hold>,
    assets: &BTreeMap<ObjId, EconomicAsset>,
    enabled: bool,
) -> Result<(), ModError> {
    let bad = || ModError::msg("certified Core payment history disagrees");
    fold.core_receipts.finish_replay().map_err(|_| bad())?;
    if !enabled {
        fold.core_receipts.require_absent().map_err(|_| {
            ModError::msg("Core payment history requires configured paired recovery")
        })?;
    }
    let mut seen = BTreeSet::new();
    for history in fold.core_receipts.histories().map_err(|_| bad())? {
        let h = history
            .terminal
            .as_ref()
            .map_or(&history.funding.hold, |t| &t.hold);
        if holds.get(&h.id) != Some(h)
            || fold.journal_holds.get(&h.id) != Some(h)
            || !seen.insert(h.id)
        {
            return Err(bad());
        }
        if let Some(birth) = history.title_birth() {
            let mut actual = assets.get(&birth.object).cloned().ok_or_else(bad)?;
            actual.frozen = false;
            actual.frozen_reason = None;
            if fold.journal_assets.get(&birth.object) != Some(&actual)
                || fold.created_count.get(&birth.object) != Some(&1)
            {
                return Err(bad());
            }
        }
    }
    if holds
        .values()
        .any(|h| retained_hold(h) && !seen.contains(&h.id))
        || fold
            .journal_holds
            .values()
            .any(|h| retained_hold(h) && !seen.contains(&h.id))
    {
        return Err(bad());
    }
    Ok(())
}
impl EconomyStore {
    /// Actual complete native journal/checkpoint provenance, never a decoded row latch.
    pub fn core_payment_history(
        &self,
        acceptance: mush_core::CommitToken,
    ) -> Result<&CorePaymentHistory, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.core_receipts.get(acceptance)
    }
    pub fn core_payment_histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &CorePaymentHistory>, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.core_receipts.histories()
    }
    pub fn require_no_core_payment_history(&self) -> Result<(), Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.core_receipts.require_absent()
    }
    /// **The managed birth of a core's title** (doc 17h cut M8a, A6): the acknowledged
    /// complete history whose output `object` titles, joined with the managed title index,
    /// so a core whose title the index does not know as its birth reads `None`. The lot
    /// and certificate references are the funding subject's own words — data for a
    /// provenance read, never custody, consumption or delivery authority
    /// ([`require_core_delivery`](super::require_core_delivery) is that).
    ///
    /// # Errors
    /// `Unavailable` while the store is frozen or the histories cannot be walked.
    pub fn core_title_birth(&self, object: ObjId) -> Result<Option<CoreTitleBirth>, Error> {
        let snapshot = self.snapshot();
        for history in self.core_payment_histories()? {
            let Some((born, birth)) = history.managed_title_birth() else {
                continue;
            };
            if born != object || snapshot.managed_titles.core(object) != Some(&birth) {
                continue;
            }
            let terminal = history.terminal().ok_or(Error::Integrity)?;
            let subject = history.subject();
            return Ok(Some(CoreTitleBirth {
                object,
                acceptance: history.funding().commit(),
                completion: terminal.commit(),
                requester: subject.requester,
                die_lot: subject.die_lot,
                die_certificate: subject.die_certificate,
                class: subject.class,
                // Fix round 1 (L4): no supplier stocked a heirloom or grey die.
                supplier: subject
                    .source
                    .is_certified()
                    .then_some(history.policy().supplier.organization),
                source: subject.source,
            }));
        }
        Ok(None)
    }
}

/// **A core's title birth, in the words a provenance read needs** (doc 17h cut M8a, A6):
/// the acceptance and completion commits, the die lot and certificate the funding named,
/// the class ordered and the supplier organization that stocked the die (the importer in
/// the one shipped composition, `tacsim::hardware::composition`), and the die's source (doc
/// 17h cut M8b-2: for a heirloom or grey die the "certificate" is the staff issue's
/// operation and `supplier` is `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTitleBirth {
    /// The titled output.
    pub object: ObjId,
    /// The acceptance's commit (`accept_core_integration`), the history's key.
    pub acceptance: mush_core::CommitToken,
    /// The completion's commit (`complete_core_integration`), the title's birth operation.
    pub completion: mush_core::CommitToken,
    /// Who ordered the core.
    pub requester: ObjId,
    /// The die lot the funding consumed (`mush_inventory::record::LotId`'s inner word).
    pub die_lot: u64,
    /// The die certificate's operation (`industry_core_die_certificates`' key).
    pub die_certificate: mush_core::CommitToken,
    /// The class ordered.
    pub class: super::CoreClass,
    /// The supplier organization of the payment policy, for a certified die; `None` for a
    /// heirloom or grey die, which no supplier stocked (doc 17h cut M8b-2 fix round 1, L4).
    pub supplier: Option<mush_org::OrgId>,
    /// **Where the die came from** (doc 17h cut M8b-2, H2): the funding subject's source,
    /// `Certified` for the importer's stock, `Heirloom` or `Grey` for a die integrated at the
    /// ungoverning site. Replayed from the history, so it survives a restart.
    pub source: super::DieSource,
}
