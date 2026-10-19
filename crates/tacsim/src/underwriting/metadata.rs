//! A real, publication-only Tacsim owner image retains the scoped publisher
//! fence through stage/take. Monetary Bind still has only Economy and Roll.
use super::{
    CoverageProductV1,
    authority::{PublicationFence, Readiness},
    protocol::Protocol,
    records::{MAX_PUBLICATION_BYTES, Publication},
};
use mush_core::authoritative::{AuthoritativeKey, ReplayWrite};
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use mush_core::{CommitToken, DefinitionDescriptor, ModError, OwnerId};
use std::{collections::BTreeMap, sync::Arc};

pub(super) const PREFIX: &str = "coverage_publication:";
const MAX_PUBLICATIONS: usize = 4096;
const MAX_BYTES: usize = 64 * 1024 * 1024;
fn bad(_: impl std::fmt::Display) -> ParticipantError {
    ParticipantError::refused("coverage publication authority or history differs")
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(super) fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    mush_roll::commerce::coverage::CoverageProtocol::publication_participant_descriptor()
        .map_err(error)
}
fn key(operation: CommitToken) -> String {
    format!("{PREFIX}{operation}")
}

#[derive(Clone, Default)]
pub(super) struct Publications {
    pub(super) loaded: bool,
    generation: u64,
    bytes: usize,
    pub(super) rows: BTreeMap<CommitToken, Arc<Publication>>,
}
impl Publications {
    pub(super) fn recover(
        rows: impl IntoIterator<Item = (String, serde_json::Value)>,
        enabled: bool,
    ) -> Result<Self, ModError> {
        let mut state = Self::default();
        for (k, value) in rows {
            if !k.starts_with(PREFIX) {
                continue;
            }
            if !enabled {
                return Err(error("disabled retained coverage publication"));
            }
            let bytes = mush_core::canonical_json_bytes_with_limit(&value, MAX_PUBLICATION_BYTES)
                .map_err(error)?;
            let row = Publication::decode(&bytes).map_err(error)?;
            if key(row.operation) != k
                || state.rows.len() >= MAX_PUBLICATIONS
                || state.rows.insert(row.operation, Arc::new(row)).is_some()
            {
                return Err(error("coverage publication inventory"));
            }
            state.bytes = state
                .bytes
                .checked_add(bytes.len())
                .ok_or_else(|| error("coverage publication bytes"))?;
            if state.bytes > MAX_BYTES {
                return Err(error("coverage publication capacity"));
            }
        }
        // The typed/native inverse must succeed before actual boot marks loaded.
        Ok(state)
    }
    fn append(&self, row: Publication) -> Result<Self, ParticipantError> {
        if !self.loaded
            || self.rows.len() >= MAX_PUBLICATIONS
            || self.rows.contains_key(&row.operation)
        {
            return Err(bad("publication capacity/identity"));
        }
        row.validate().map_err(bad)?;
        let size = mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(&row).map_err(bad)?,
            MAX_PUBLICATION_BYTES,
        )
        .map_err(bad)?
        .len();
        let mut post = self.clone();
        post.generation = post
            .generation
            .checked_add(1)
            .ok_or_else(|| bad("generation"))?;
        post.bytes = post.bytes.checked_add(size).ok_or_else(|| bad("bytes"))?;
        if post.bytes > MAX_BYTES {
            return Err(bad("publication bytes"));
        }
        post.rows.insert(row.operation, Arc::new(row));
        Ok(post)
    }
}
pub(super) struct Request {
    pub(super) product: CoverageProductV1,
}
pub(super) struct Output {
    row: Publication,
}
impl Output {
    pub(super) fn into_row(self) -> Publication {
        self.row
    }
}
pub(super) struct Participant {
    pub(super) protocol: Protocol,
    pub(super) readiness: Readiness,
}
pub(super) struct Patch {
    generation: u64,
    fence: PublicationFence,
    post: Publications,
}
pub(super) struct Bound<'a> {
    state: &'a mut Publications,
    post: Publications,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        *self.state = self.post;
    }
}
impl Participant {
    fn references(
        &self,
        world: &mush_core::World,
        product: &CoverageProductV1,
    ) -> Result<(), ParticipantError> {
        // Ordinary launch has no role-qualified claim. An arbitrary exact digest
        // is still not an acknowledged ConductTrialProof.
        if !product.approvals().is_empty() {
            return Err(bad("role qualification unavailable"));
        }
        self.readiness
            .references(product.hulls().iter().chain(product.zones()))
            .map_err(bad)?;
        super::authority::player(world, product.adjudicator()).map_err(bad)?;
        if world
            .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
            .is_none_or(|o| o.as_str() != "economy")
        {
            return Err(bad("treasury owner"));
        }
        let economy = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or_else(|| bad("treasury"))?;
        let snap = economy.snapshot();
        if economy.is_frozen()
            || snap
                .instrument(product.terms().instrument())
                .is_none_or(|i| {
                    i.status != mush_economy::InstrumentStatus::Active || !i.transferable
                })
            || snap.account(product.treasury()).is_none_or(|a| {
                a.owner != mush_org::PrincipalId::Organization(product.insurer())
                    || a.scope != Some(product.scope())
                    || a.status != mush_economy::AccountStatus::Active
                    || &a.instrument != product.terms().instrument()
            })
        {
            return Err(bad("actual scoped treasury"));
        }
        Ok(())
    }
}
impl OwnerParticipant for Participant {
    type State = Publications;
    type Request = Request;
    type Output = Output;
    type Patch = Patch;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &Publications,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Output, Patch>, ParticipantError> {
        let fence = PublicationFence::new(
            self.readiness.clone(),
            &self.protocol,
            ctx.world(),
            ctx.commit_token(),
            ctx.invocation().actor(),
            request.product.insurer(),
            request.product.scope(),
        )
        .map_err(bad)?;
        fence.check(ctx).map_err(bad)?;
        self.references(ctx.world(), &request.product)?;
        let row = Publication {
            format: 1,
            operation: ctx.commit_token(),
            action: ctx.action().clone(),
            protocol: self.protocol.clone(),
            publisher: ctx.invocation().actor(),
            authorization: fence.authority().clone(),
            product: request.product,
            posted_at_utc: ctx.world().now(),
        };
        let post = state.append(row.clone())?;
        let write = ReplayWrite::ModuleJsonSet {
            namespace: OwnerId::new(crate::NS).map_err(bad)?,
            key: AuthoritativeKey::parse_canonical(key(row.operation)).map_err(bad)?,
            value: serde_json::to_value(&row).map_err(bad)?,
        };
        if !self.accepts_write(&write) {
            return Err(bad("publication write"));
        }
        Ok(OwnerPreparation::new(
            Output { row },
            Patch {
                generation: state.generation,
                fence,
                post,
            },
            vec![write],
        ))
    }
    fn recheck(
        &self,
        state: &Publications,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Patch,
    ) -> Result<(), ParticipantError> {
        if !state.loaded || state.generation != patch.generation {
            return Err(bad("stale publication owner"));
        }
        patch.fence.check(ctx).map_err(bad)?;
        let row = patch
            .post
            .rows
            .get(&ctx.commit_token())
            .ok_or_else(|| bad("publication postimage"))?;
        self.references(ctx.world(), &row.product)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Publications,
        patch: Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        if !state.loaded || state.generation != patch.generation {
            return Err(bad("stale publication bind"));
        }
        Ok(Bound {
            state,
            post: patch.post,
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key: k,
            value,
        } = write
        else {
            return false;
        };
        if namespace.as_str() != crate::NS {
            return false;
        }
        let Ok(bytes) = mush_core::canonical_json_bytes_with_limit(value, MAX_PUBLICATION_BYTES)
        else {
            return false;
        };
        let Ok(row) = Publication::decode(&bytes) else {
            return false;
        };
        k.as_str() == key(row.operation)
            && row.protocol == self.protocol
            && row.product.approvals().is_empty()
    }
}
