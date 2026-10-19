//! One closed source-free reader for contract scope and the actual Roll service.
use super::super::{families::CommerceFamilies, rating::RatingFamilies};
use super::*;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRef, ModError, Registrar};
use mush_economy::{ContractFactScope, ExternalRef};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub(super) enum Query {
    Scoped {
        scope: ContractFactScope,
        fact: ExternalRef,
    },
    Term(protocol::Request),
}
#[derive(Debug, Serialize)]
pub(super) enum Reply {
    Scoped { truth: Option<Truth> },
    Term(history::TermProof),
}
#[derive(Debug, Serialize)]
pub(super) struct Truth {
    at: i64,
    actor: mush_core::ObjId,
}
impl Truth {
    pub(super) fn state(&self) -> mush_economy::FactState {
        mush_economy::FactState {
            satisfied: true,
            actor: Some(self.actor),
            at: self.at,
        }
    }
}
pub(super) struct Reader {
    f: CommerceFamilies,
    ratings: RatingFamilies,
    ready: Readiness,
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(DefinitionKind::new("owner_read").map_err(error)?, DefinitionId::new("minds_contract_facts").map_err(error)?, 1,
        serde_json::json!({"format":1,"protocol":"personal_minds_facts_v1","subjects":["accepted_owner_generation","accepted_paid_generation"],"scope":"actual_contract_consent_and_current_subject","truth":"committed_occurrence_only","source":false})).map_err(error)
}
impl Reader {
    pub(super) fn request(reg: &mut Registrar<'_>) -> Result<OwnerReadHandle<Self>, ModError> {
        let exact = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, descriptor()?)
            .map_err(error)?;
        reg.request_owner_read(exact)
    }
}
pub(super) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    ratings: RatingFamilies,
    ready: Readiness,
) -> Result<(), ModError> {
    let exact = reg.definition(descriptor()?)?;
    reg.owner_read_service(
        exact,
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new(crate::NS).map_err(error)?,
        ]),
        BTreeSet::from([OwnerId::new("economy").map_err(error)?]),
        Reader { f, ratings, ready },
    )
}
impl OwnerReadService for Reader {
    type State = crate::RollStore;
    type Request = Query;
    type Output = Reply;
    fn read(
        &self,
        roll: &Self::State,
        ctx: &OwnerReadContext<'_>,
        query: Query,
    ) -> Result<Reply, OwnerReadError> {
        let refuse = |_| OwnerReadError::Unavailable;
        self.ready.check(ctx.world()).map_err(refuse)?;
        match query {
            Query::Term(request) => {
                if ctx.requester().as_str() != crate::NS || ctx.actor() != mush_core::GOD {
                    return Err(OwnerReadError::Authority);
                }
                let proof = history::resolve(
                    ctx,
                    &self.f,
                    &self.ratings,
                    request.licence,
                    request.generation,
                )
                .map_err(refuse)?;
                Ok(Reply::Term(proof))
            }
            Query::Scoped { scope, fact } => {
                let Some(protocol::FactKey::Lapsed(licence, generation)) =
                    protocol::FactKey::parse(&fact)
                else {
                    return Err(OwnerReadError::Authority);
                };
                let proof = history::resolve(ctx, &self.f, &self.ratings, licence, generation)
                    .map_err(refuse)?;
                authority::authorize(ctx, &scope, &fact, &proof).map_err(refuse)?;
                let occurrence = super::super::entitlement::history::optional(
                    ctx,
                    &self.f.lapse_occurrences,
                    proof.accepted.key(),
                )
                .map_err(refuse)?;
                let truth = occurrence
                    .map(|row| {
                        proof.require_historical_lapse(row)?;
                        roll.check_required_root(&row.root()?).map_err(invalid)?;
                        Ok::<_, PublicationError>(Truth {
                            at: row.at_utc,
                            actor: row.actor,
                        })
                    })
                    .transpose()
                    .map_err(refuse)?;
                Ok(Reply::Scoped { truth })
            }
        }
    }
}
