//! Same-slot dispatch and complete native replay. No Industry readiness cycle.
use super::{
    ACCEPT_ACTION, BayPaymentHistory, BayRefitConfig, BayRefitError as Error, CANCEL_ACTION,
    HOLD_PURPOSE, INSTALL_ACTION, NAMESPACE,
};
use crate::{EconomyStore, Hold, HoldId};
use mush_core::{DefinitionRef, ModError, Registrar};
use std::collections::{BTreeMap, BTreeSet};
/// Register the policy's definition and return the three paid actions the
/// Economy participant is granted: Accept, Install and Cancel.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &BayRefitConfig,
) -> Result<[DefinitionRef; 3], ModError> {
    let err = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    config.policy.validate().map_err(|e| err(&e))?;
    let actions: [DefinitionRef; 3] = [
        config.policy.action_ref(ACCEPT_ACTION),
        config.policy.action_ref(INSTALL_ACTION),
        config.policy.action_ref(CANCEL_ACTION),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| err(&e))?
    .try_into()
    .map_err(|_| ModError::msg("bay action count"))?;
    let mut references = vec![config.industry_owner.clone(), config.policy.recipe.clone()];
    references.extend(actions.iter().cloned());
    references.sort();
    let descriptor = mush_core::DefinitionDescriptor::new(
        mush_core::DefinitionKind::new("bay_refit_policy").map_err(|e| err(&e))?,
        mush_core::DefinitionId::new("bay_refit").map_err(|e| err(&e))?,
        1,
        serde_json::to_value(&config.policy).map_err(|e| err(&e))?,
    )
    .map_err(|e| err(&e))?
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
                Err("bay refit composition differs".into())
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
/// Close the replayed bay histories against the stored and journal holds:
/// every history's hold is exactly its last image, and no bay hold exists
/// without a history. Without a composed bay leg the history must be empty.
pub(crate) fn recover(
    fold: &mut crate::fold::FoldState,
    holds: &BTreeMap<HoldId, Hold>,
    enabled: bool,
) -> Result<(), ModError> {
    let bad = || ModError::msg("bay refit history disagrees");
    fold.bay_refit_receipts.finish_replay().map_err(|_| bad())?;
    if !enabled {
        fold.bay_refit_receipts
            .require_absent()
            .map_err(|_| ModError::msg("bay refit history requires configured paired recovery"))?;
    }
    let mut seen = BTreeSet::new();
    for history in fold.bay_refit_receipts.histories().map_err(|_| bad())? {
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
    pub fn bay_refit_history(
        &self,
        acceptance: mush_core::CommitToken,
    ) -> Result<&BayPaymentHistory, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.bay_refit_receipts.get(acceptance)
    }
    pub fn bay_refit_histories(
        &self,
    ) -> Result<impl ExactSizeIterator<Item = &BayPaymentHistory>, Error> {
        if self.is_frozen() {
            return Err(Error::Unavailable);
        }
        self.fold.bay_refit_receipts.histories()
    }
}
