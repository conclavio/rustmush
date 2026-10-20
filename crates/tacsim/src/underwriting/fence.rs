//! Additional refusal-only custody/current-issuer check, captured privately by
//! the actual provider and retained inside its real Roll owner preparation.
use super::{
    CoverageProductV1,
    host::{CoverageHost, Intent},
};
use mush_core::{
    CommitToken, DefinitionDigest, DefinitionRef, ObjId, participant::OwnerPrepareContext,
};
use mush_roll::commerce::{coverage::CoveragePrecommitCheck, records::PublicationError};
use std::sync::Arc;

pub(super) struct HostCheck {
    host: CoverageHost,
    action: DefinitionRef,
    operation: CommitToken,
    actor: ObjId,
    index: usize,
    request: DefinitionDigest,
    intent: Intent,
    fresh: Option<(CoverageProductV1, mush_economy::AccountId)>,
    conduct: Option<mush_roll::commerce::conduct::ConductTrialProof>,
}
impl HostCheck {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        host: CoverageHost,
        action: DefinitionRef,
        operation: CommitToken,
        actor: ObjId,
        index: usize,
        request: DefinitionDigest,
        intent: Intent,
        fresh: Option<(CoverageProductV1, mush_economy::AccountId)>,
        conduct: Option<mush_roll::commerce::conduct::ConductTrialProof>,
    ) -> Arc<dyn CoveragePrecommitCheck> {
        Arc::new(Self {
            host,
            action,
            operation,
            actor,
            index,
            request,
            intent,
            fresh,
            conduct,
        })
    }
}
fn same(a: &Intent, b: &Intent) -> bool {
    match (a, b) {
        (
            Intent::Quote {
                departure: a,
                publication: ap,
                payer: ac,
                value: av,
            },
            Intent::Quote {
                departure: b,
                publication: bp,
                payer: bc,
                value: bv,
            },
        ) => Arc::ptr_eq(a, b) && ap == bp && ac == bc && av == bv,
        (
            Intent::Bind {
                departure: a,
                quote: aq,
            },
            Intent::Bind {
                departure: b,
                quote: bq,
            },
        ) => Arc::ptr_eq(a, b) && aq == bq,
        (
            Intent::Terminal {
                close: a,
                policy: ap,
                cancel: ac,
            },
            Intent::Terminal {
                close: b,
                policy: bp,
                cancel: bc,
            },
        ) => Arc::ptr_eq(a, b) && ap == bp && ac == bc,
        _ => false,
    }
}
impl CoveragePrecommitCheck for HostCheck {
    fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        let fail = |_| PublicationError::Stale;
        if ctx.action() != &self.action
            || ctx.commit_token() != self.operation
            || ctx.invocation().actor() != self.actor
        {
            return Err(PublicationError::Stale);
        }
        let reg = self.host.ready(ctx.world(), self.index).map_err(fail)?;
        let actual = self
            .host
            .lookup(self.request, self.actor, self.index)
            .map_err(fail)?;
        if !same(&actual, &self.intent) {
            return Err(PublicationError::Stale);
        }
        match &self.intent {
            Intent::Quote { departure, .. } | Intent::Bind { departure, .. } => departure
                .recheck(ctx.world())
                .map_err(|_| PublicationError::Stale)?,
            Intent::Terminal { close, .. } => close
                .recheck(ctx.world())
                .map_err(|_| PublicationError::Stale)?,
        }
        if let Some((product, payer)) = &self.fresh {
            reg.readiness.issuer(ctx.world(), product).map_err(fail)?;
            reg.readiness
                .references(product.hulls().iter().chain(product.zones()))
                .map_err(fail)?;
            super::provider::accounts(ctx.world(), self.actor, *payer, product).map_err(fail)?;
        }
        if let Some(proof) = &self.conduct {
            proof.check_current(ctx)?;
        }
        Ok(())
    }
}
