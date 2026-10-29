//! **`tacsim_procurement`'s provider** (doc 17h §3 M8f P7; the yard dispatch's shape): the
//! House agent records one purchase in the ledger. The rows are the tick's, fixed into the
//! agent's offer ([`super::tick::ProcurementState::offered`]), so the agent chooses only which
//! operation; the prepare re-checks the row's bounds, that the actor is the agent of the
//! deployment's policy for the row's organisation, and the one transition allowed: a new row is
//! `Funded` at a vacant `(org, period, ordinal)` key, and a `Funded` row may only become
//! `Granted` or `Refunded`, every other field unchanged. One purchase is recorded once because
//! the tick records only an operation no row of the month or the one before names, one step at
//! a time per agent (a pending submission defers it).
use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, DefinitionRef, PerceivedRef, PrepareCtx, ProjectedParameterKind,
    StateFamilyHandle, SubjectKey,
};
use serde_json::{Map, Value, json};

use super::ledger::{ProcurementKey, ProcurementRow, RowPhase};
use crate::port_tick::port_error;

pub(super) struct RecordProvider {
    pub(super) action: DefinitionRef,
    pub(super) outcome: DefinitionRef,
    pub(super) rows: StateFamilyHandle<ProcurementKey, ProcurementRow>,
}

fn invalid() -> ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the procurement record is invalid",
    )
}

/// The chosen row: `rows[operation]`, from the offer's fixed rows.
fn chosen(invocation: &ActionInvocation) -> Result<ProcurementRow, ActionError> {
    if invocation.principal()
        != &(PrincipalId::Object {
            id: invocation.actor(),
        })
        || invocation.subject() != &PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(invalid());
    }
    let parameters: &Map<String, Value> =
        invocation.parameters().as_object().ok_or_else(invalid)?;
    let operation = parameters
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let row = parameters
        .get("rows")
        .and_then(Value::as_object)
        .and_then(|rows| rows.get(operation))
        .cloned()
        .ok_or_else(invalid)?;
    let row: ProcurementRow = serde_json::from_value(row).map_err(|_| invalid())?;
    row.check().map_err(|_| invalid())?;
    if row.operation.to_string() != operation || row.agent != invocation.actor() {
        return Err(invalid());
    }
    Ok(row)
}

impl ActionProvider for RecordProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        chosen(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some(rows) = engine
            .world
            .module_state::<super::tick::ProcurementState>()
            .and_then(|state| state.offered(query.actor()))
        else {
            return Ok(None);
        };
        if rows.is_empty() {
            return Ok(None);
        }
        let mut fixed = Map::new();
        for (operation, row) in rows {
            fixed.insert(
                operation.clone(),
                serde_json::to_value(row).map_err(|_| invalid())?,
            );
        }
        let choices: Vec<String> = fixed.keys().cloned().collect();
        crate::surface::act::offer_draft(
            action,
            engine,
            json!({ "rows": Value::Object(fixed) }),
            vec![crate::surface::act::parameter(
                "operation",
                true,
                ProjectedParameterKind::Choice { choices },
            )?],
            (
                "tacsim-procurement",
                "Record a House purchase",
                "A House's procurement agent records one purchase against the House's monthly \
                 budget.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let row = chosen(invocation)?;
        {
            let world = ctx.world().map_err(|_| invalid())?;
            let agent_of_org = world
                .module_state::<super::tick::ProcurementState>()
                .is_some_and(|state| state.is_agent(row.org, invocation.actor()));
            if !agent_of_org {
                return Err(port_error(
                    ActionErrorCode::Unauthorized,
                    "that is not this House's procurement agent",
                ));
            }
        }
        let key = row.key();
        let prior = txn
            .get(&self.rows, &key)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the procurement ledger is unavailable",
                )
            })?
            .cloned();
        let staged = match prior {
            None if row.phase == RowPhase::Funded => txn.insert(&self.rows, &key, row.clone()),
            Some(prior)
                if prior.phase == RowPhase::Funded
                    && row.phase != RowPhase::Funded
                    && prior.with_phase(row.phase) == row =>
            {
                txn.replace(&self.rows, &key, row.clone())
            }
            _ => {
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "that procurement record is already written",
                ));
            }
        };
        staged.map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the procurement record did not stage",
            )
        })?;
        let outcome = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            json!({
                "org": row.org.0,
                "operation": row.operation,
                "phase": row.phase,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the procurement outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.action.clone(), self.outcome.clone()])
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}
