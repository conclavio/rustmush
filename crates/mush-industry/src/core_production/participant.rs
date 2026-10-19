use super::*;
use super::{
    authority::{Definitions, LiveFacts},
    records::{DieCertificate, OriginIndex},
};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::read_service::{OwnerReadContext, OwnerReadError, OwnerReadService};
use mush_core::{CommitToken, DefinitionRef, GOD, ObjId, ReplayWrite};
use std::sync::Arc;
#[derive(Clone, Default)]
pub(crate) struct CoreProductionState {
    pub(crate) ready: bool,
    pub(crate) generation: u64,
    pub(crate) index: Option<OriginIndex>,
    pub(crate) integration: Option<super::integration::state::Projection>,
}
pub(crate) struct Readiness;
impl OwnerReadService for Readiness {
    type State = CoreProductionState;
    type Request = ();
    type Output = u64;
    fn read(
        &self,
        s: &Self::State,
        ctx: &OwnerReadContext<'_>,
        _: (),
    ) -> Result<u64, OwnerReadError> {
        if !s.ready || ctx.requester().as_str() != crate::NS {
            return Err(OwnerReadError::Unavailable);
        }
        Ok(s.generation)
    }
}
pub(crate) enum Request {
    Initialize,
    Issue(DieCertificate),
    Integration(super::integration::owner::Request),
}
pub(crate) struct OriginParticipant {
    pub(crate) config: CoreOriginConfig,
    pub(crate) definitions: Definitions,
    pub(crate) actions: [DefinitionRef; 2],
    pub(crate) integration: Option<(
        super::integration::CoreFactoryConfig,
        mush_inventory::core_input::RegisteredCoreInputs,
    )>,
}
pub(crate) struct OriginPatch {
    before: Option<OriginIndex>,
    after: OriginIndex,
    generation: u64,
    next_generation: u64,
    actor: ObjId,
    action: DefinitionRef,
    commit: CommitToken,
    live: Option<LiveFacts>,
}
pub(crate) struct OriginBound<'a> {
    state: &'a mut CoreProductionState,
    index: OriginIndex,
    generation: u64,
}
impl BoundParticipant for OriginBound<'_> {
    fn apply(self) {
        self.state.index = Some(self.index);
        self.state.generation = self.generation;
    }
}
fn refused() -> ParticipantError {
    ParticipantError::refused("certified Origin stock authority or history changed")
}
fn identity(ctx: &OwnerPrepareContext<'_>) -> Result<(), ParticipantError> {
    let inv = ctx.invocation();
    if inv.principal() != &(mush_core::action::PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &mush_core::PerceivedRef::SelfActor
        || !inv.targets().is_empty()
    {
        return Err(refused());
    }
    Ok(())
}
impl OriginParticipant {
    fn prepare_origin(
        &self,
        state: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<(OriginIndex, OriginPatch, Vec<ReplayWrite>), ParticipantError> {
        identity(ctx)?;
        if !state.ready {
            return Err(refused());
        }
        self.definitions
            .content(&self.config.policy)
            .map_err(|_| refused())?;
        let (after, live) = match request {
            Request::Integration(_) => return Err(refused()),
            Request::Initialize => {
                if self.config.mode != CoreOriginMode::Initialize
                    || ctx.invocation().actor() != GOD
                    || ctx.action() != &self.actions[0]
                    || state.index.is_some()
                {
                    return Err(refused());
                }
                (
                    OriginIndex::new(&self.config.policy, ctx.commit_token())
                        .map_err(|_| refused())?,
                    None,
                )
            }
            Request::Issue(certificate) => {
                certificate.validate().map_err(|_| refused())?;
                if self.config.mode != CoreOriginMode::Active
                    || certificate.policy != self.config.policy
                    || certificate.operation != ctx.commit_token()
                    || certificate.action != *ctx.action()
                    || ctx.action() != &self.actions[1]
                    || certificate.agent != ctx.invocation().actor()
                {
                    return Err(refused());
                }
                let live = authority::check(
                    ctx.world(),
                    &self.definitions,
                    &self.config.policy,
                    ctx.invocation().actor(),
                )
                .map_err(|_| refused())?;
                if live.stamp != certificate.observed_ic || live.utc != certificate.observed_utc {
                    return Err(refused());
                }
                (
                    state
                        .index
                        .as_ref()
                        .ok_or_else(refused)?
                        .issue(&certificate)
                        .map_err(|_| refused())?,
                    Some(live),
                )
            }
        };
        let next_generation = state.generation.checked_add(1).ok_or_else(refused)?;
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: mush_core::mechanics::OwnerId::new(crate::NS).map_err(|_| refused())?,
            key: mush_core::AuthoritativeKey::parse_canonical(INDEX_KEY).map_err(|_| refused())?,
            value: encode(&after).map_err(|_| refused())?,
        }];
        let patch = OriginPatch {
            before: state.index.clone(),
            after: after.clone(),
            generation: state.generation,
            next_generation,
            actor: ctx.invocation().actor(),
            action: ctx.action().clone(),
            commit: ctx.commit_token(),
            live,
        };
        Ok((after, patch, writes))
    }
    fn recheck_origin(
        &self,
        state: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        p: &OriginPatch,
    ) -> Result<(), ParticipantError> {
        identity(ctx)?;
        if !state.ready
            || state.generation != p.generation
            || state.index != p.before
            || ctx.action() != &p.action
            || ctx.commit_token() != p.commit
            || ctx.invocation().actor() != p.actor
        {
            return Err(refused());
        }
        self.definitions
            .content(&self.config.policy)
            .map_err(|_| refused())?;
        if let Some(live) = &p.live {
            if self.config.mode != CoreOriginMode::Active {
                return Err(refused());
            }
            authority::recheck(
                ctx.world(),
                &self.definitions,
                &self.config.policy,
                p.actor,
                live,
            )
            .map_err(|_| refused())?;
        } else if self.config.mode != CoreOriginMode::Initialize
            || p.actor != GOD
            || p.action != self.actions[0]
            || state.index.is_some()
        {
            return Err(refused());
        }
        Ok(())
    }
    fn bind_origin<'a>(
        self: Arc<Self>,
        state: &'a mut CoreProductionState,
        p: OriginPatch,
    ) -> Result<OriginBound<'a>, ParticipantError> {
        if !state.ready || state.generation != p.generation || state.index != p.before {
            return Err(refused());
        }
        Ok(OriginBound {
            state,
            index: p.after,
            generation: p.next_generation,
        })
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        match w {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == crate::NS && key.as_str() == INDEX_KEY => {
                decode::<OriginIndex>(value).is_ok_and(|r| r.validate().is_ok())
            }
            _ => false,
        }
    }
}

pub(crate) enum Output {
    Origin(OriginIndex),
    Integration(super::integration::owner::Output),
}
impl Output {
    pub(crate) fn origin(self) -> Result<OriginIndex, ParticipantError> {
        match self {
            Self::Origin(v) => Ok(v),
            _ => Err(refused()),
        }
    }
    pub(crate) fn integration(self) -> Result<super::integration::owner::Output, ParticipantError> {
        match self {
            Self::Integration(v) => Ok(v),
            _ => Err(refused()),
        }
    }
}
pub(crate) enum Patch {
    Origin(OriginPatch),
    Integration(super::integration::owner::Patch),
}
pub(crate) enum Bound<'a> {
    Origin(OriginBound<'a>),
    Integration {
        state: &'a mut CoreProductionState,
        patch: super::integration::owner::Patch,
    },
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        match self {
            Self::Origin(v) => v.apply(),
            Self::Integration { state, patch } => {
                state.integration = Some(patch.after);
                state.generation = patch.next_generation;
            }
        }
    }
}
impl OwnerParticipant for OriginParticipant {
    type State = CoreProductionState;
    type Request = Request;
    type Output = Output;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Output, Patch>, ParticipantError> {
        match request {
            Request::Integration(request) => {
                let (config, inputs) = self.integration.as_ref().ok_or_else(refused)?;
                let (output, patch, writes) = super::integration::owner::prepare(
                    state,
                    ctx,
                    config,
                    &self.definitions,
                    &self.config.policy.content,
                    inputs,
                    request,
                )?;
                Ok(OwnerPreparation::new(
                    Output::Integration(output),
                    Patch::Integration(patch),
                    writes,
                ))
            }
            request => {
                let (output, patch, writes) = self.prepare_origin(state, ctx, request)?;
                Ok(OwnerPreparation::new(
                    Output::Origin(output),
                    Patch::Origin(patch),
                    writes,
                ))
            }
        }
    }
    fn recheck(
        &self,
        state: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        match patch {
            Patch::Origin(p) => self.recheck_origin(state, ctx, p),
            Patch::Integration(p) => {
                let (config, inputs) = self.integration.as_ref().ok_or_else(refused)?;
                p.recheck(
                    state,
                    ctx,
                    config,
                    &self.definitions,
                    &self.config.policy.content,
                    inputs,
                )
            }
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut CoreProductionState,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        match patch {
            Patch::Origin(p) => self.bind_origin(state, p).map(Bound::Origin),
            Patch::Integration(p) => {
                p.check_local(state)?;
                Ok(Bound::Integration { state, patch: p })
            }
        }
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        if OriginParticipant::accepts_write(self, w) {
            return true;
        }
        match w {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if self.integration.is_some() && namespace.as_str() == crate::NS => {
                if key.as_str() == super::integration::records::INDEX_KEY {
                    super::integration::records::decode::<
                        super::integration::records::IntegrationIndex,
                    >(value, super::integration::records::INDEX_BYTES)
                    .is_ok_and(|r| r.validate().is_ok())
                } else if key.as_str() == super::integration::delivery::INDEX_KEY
                    && self
                        .integration
                        .as_ref()
                        .is_some_and(|(c, _)| c.payment().delivery.is_some())
                {
                    super::integration::records::decode::<super::integration::delivery::Index>(
                        value,
                        super::integration::delivery::INDEX_BYTES,
                    )
                    .is_ok_and(|r| r.validate().is_ok())
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
