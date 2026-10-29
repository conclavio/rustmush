//! Same-slot dispatch and complete native replay. No Industry readiness cycle.
use super::{
    ACCEPT_ACTION, CANCEL_ACTION, CoreRefitConfig, HOLD_PURPOSE, INSTALL_ACTION, NAMESPACE,
    RefitError as Error, RefitPaymentHistory,
};
use crate::{EconomyStore, Hold, HoldId};
use mush_core::{DefinitionRef, ModError, Registrar};
use std::collections::{BTreeMap, BTreeSet};
impl std::fmt::Debug for CoreRefitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreRefitConfig")
            .field("policy", &self.policy)
            .field("industry_owner", &self.industry_owner)
            .finish_non_exhaustive()
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &CoreRefitConfig,
) -> Result<[DefinitionRef; 3], ModError> {
    config
        .policy
        .validate()
        .map_err(|e| ModError::msg(e.to_string()))?;
    let actions = [
        config.policy.action_ref(ACCEPT_ACTION),
        config.policy.action_ref(INSTALL_ACTION),
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
    references.extend(config.policy.ungovern.iter().map(|u| u.recipe.clone()));
    references.extend(config.policy.roots().cloned());
    references.sort();
    let descriptor = mush_core::DefinitionDescriptor::new(
        mush_core::DefinitionKind::new("core_refit_policy")
            .map_err(|e| ModError::msg(e.to_string()))?,
        mush_core::DefinitionId::new("core_refit").map_err(|e| ModError::msg(e.to_string()))?,
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
                Err("Core refit composition differs".into())
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
    enabled: bool,
) -> Result<(), ModError> {
    let bad = || ModError::msg("certified Core refit history disagrees");
    fold.refit_receipts.finish_replay().map_err(|_| bad())?;
    if !enabled {
        fold.refit_receipts
            .require_absent()
            .map_err(|_| ModError::msg("Core refit history requires configured paired recovery"))?;
    }
    let mut seen = BTreeSet::new();
    for history in fold.refit_receipts.histories().map_err(|_| bad())? {
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
        let origin = fold
            .core_receipts
            .get(history.subject().core_acceptance)
            .map_err(|_| bad())?;
        let native = origin.terminal().ok_or_else(bad)?;
        let crate::core_integration::CoreTerminalResult::Completed { output, .. } = native.result()
        else {
            return Err(bad());
        };
        if output.object != history.subject().incoming
            || origin.subject().requester != history.subject().requester
            || origin.subject().class != history.subject().class
        {
            return Err(bad());
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
    pub fn core_refit_history(
        &self,
        acceptance: mush_core::CommitToken,
    ) -> Result<&RefitPaymentHistory, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.refit_receipts.get(acceptance)
    }
    pub fn core_refit_histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &RefitPaymentHistory>, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.refit_receipts.histories()
    }
    pub fn require_no_core_refit_history(&self) -> Result<(), Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.refit_receipts.require_absent()
    }
}
