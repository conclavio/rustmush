//! Actual historical minds facts. This bridge never grants use or source.
pub(crate) mod audit;
mod authority;
mod history;
pub(crate) mod protocol;
mod provider;
pub(crate) mod publication;
pub(crate) mod reader;
pub(crate) mod records;
mod service;
mod transfer;
use super::records::{PublicationError, Record};
use mush_core::mechanics::{DefinitionRegistry, OwnerId};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ModError, ModuleCtx, Registrar, World};
use mush_economy::{ExternalRef, FactProvider, FactState, ScopedFactContext};
pub(crate) use service::guard_current;
use std::sync::{Arc, OnceLock};
fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Real captured composition, never a caller-set health flag. All reads recheck
/// the shared owner latches and Roll's actual mandatory audit state.
#[derive(Clone, Default)]
pub(crate) struct Readiness(Arc<OnceLock<DefinitionRegistry>>);
impl Readiness {
    pub(crate) fn capture(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| error("minds fact definitions unavailable"))?;
        self.0
            .set(defs.clone())
            .map_err(|_| error("minds facts already captured"))
    }
    fn check_transfer(&self, world: &World) -> Result<(), PublicationError> {
        self.check(world)?;
        self.0
            .get()
            .ok_or(PublicationError::Unavailable)?
            .require_owner_ready(&OwnerId::new("market").map_err(invalid)?)
            .map_err(|_| PublicationError::Unavailable)
    }
    fn check(&self, world: &World) -> Result<(), PublicationError> {
        let defs = self.0.get().ok_or(PublicationError::Unavailable)?;
        for owner in [crate::NS, "economy"] {
            defs.require_owner_ready(&OwnerId::new(owner).map_err(invalid)?)
                .map_err(|_| PublicationError::Unavailable)?;
        }
        if world
            .module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|o| o.as_str() != crate::NS)
            || world
                .module_state_owner(std::any::TypeId::of::<mush_economy::EconomyStore>())
                .is_none_or(|o| o.as_str() != "economy")
            || world
                .module_state::<mush_economy::EconomyStore>()
                .is_none_or(|e| e.is_frozen())
        {
            return Err(PublicationError::Unavailable);
        }
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(|_| PublicationError::Unavailable)
    }
}
/// **The Registry's public findings** (doc 17h cut M8d D6): what only the hull's owner can
/// answer from a world — a seizure, by hull and the hull's inspection sequence. tacsim installs
/// one with [`MindsFactProvider::with_registry_findings`]; without one, the fact never
/// resolves. (`minds.unwarranted` is scoped and resolves for no condition, fix round K6.)
pub trait RegistryFindings: Send + Sync {
    /// The IC second at which the core in `hull` was seized on inspection `seq`, if it was.
    fn seized(&self, world: &World, hull: mush_core::ObjId, seq: u64) -> Option<i64>;
}

struct Shared {
    ready: Readiness,
    economy_reader: OnceLock<OwnerReadHandle<reader::Reader>>,
    findings: OnceLock<Arc<dyn RegistryFindings>>,
}
/// Trusted composition passes the same opaque instance to Roll's explicit
/// Active opt-in and the Economy fact table. Construction alone enables nothing.
#[derive(Clone)]
pub struct MindsFactProvider(Arc<Shared>);
impl Default for MindsFactProvider {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Debug for MindsFactProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MindsFactProvider").finish_non_exhaustive()
    }
}
impl MindsFactProvider {
    pub fn new() -> Self {
        Self(Arc::new(Shared {
            ready: Readiness::default(),
            economy_reader: OnceLock::new(),
            findings: OnceLock::new(),
        }))
    }
    /// Doc 17h cut M8d D6: install the one source of the public Registry findings
    /// (`minds.seized`, `minds.unwarranted`).
    ///
    /// # Errors
    /// A second installation.
    pub fn with_registry_findings(
        &self,
        findings: Arc<dyn RegistryFindings>,
    ) -> Result<(), ModError> {
        self.0
            .findings
            .set(findings)
            .map_err(|_| error("Registry findings already installed"))
    }
    pub(crate) fn capture(&self, ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
        if self.0.economy_reader.get().is_none() {
            return Err(error("minds facts need the same actual Economy provider"));
        }
        self.0.ready.capture(ctx)
    }
    fn scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<reader::Reply> {
        self.0.ready.check(ctx.world()).ok()?;
        ctx.read_owner(
            self.0.economy_reader.get()?,
            reader::Query::Scoped {
                scope: Box::new(ctx.scope().clone()),
                fact: fact.clone(),
            },
        )
        .ok()
    }
}
impl FactProvider for MindsFactProvider {
    fn provider(&self) -> &str {
        protocol::PROVIDER
    }
    fn requires_scope(&self, fact: &ExternalRef) -> bool {
        // All non-public spellings remain private even when malformed or not
        // implemented. No alternate spelling can fall through legacy resolve.
        !protocol::FactKey::parse(fact).is_some_and(protocol::FactKey::is_public)
    }
    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        let fact = ExternalRef {
            provider: protocol::PROVIDER.into(),
            kind: kind.into(),
            id: id.into(),
            revision,
        };
        let key = protocol::FactKey::parse(&fact)?;
        let at = match key {
            protocol::FactKey::Filed(filed) => {
                self.0.ready.check(w).ok()?;
                w.module_state::<crate::RollStore>()?
                    .require_public_filed_fact(filed)
                    .ok()?;
                0
            }
            // Doc 17h cut M8d D6: the Registry's public seizure, from its one source.
            protocol::FactKey::Seized(hull, seq) => {
                self.0.ready.check(w).ok()?;
                self.0.findings.get()?.seized(w, hull, seq)?
            }
            protocol::FactKey::Lapsed(..)
            | protocol::FactKey::Transferred(..)
            | protocol::FactKey::Unattestable(..)
            | protocol::FactKey::Unwarranted(..)
            | protocol::FactKey::Claimed(..) => return None,
        };
        Some(FactState {
            satisfied: true,
            actor: None,
            at,
        })
    }
    fn authorize_scope(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> bool {
        matches!(self.scoped(ctx, fact), Some(reader::Reply::Scoped { .. }))
    }
    fn resolve_scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<FactState> {
        let reader::Reply::Scoped { truth } = self.scoped(ctx, fact)? else {
            return None;
        };
        truth.map(|value| value.state())
    }
    fn register_scoped(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if reg.module() != "economy" {
            return Err(error("minds facts require actual Economy registration"));
        }
        let handle = reader::Reader::request(reg)?;
        self.0
            .economy_reader
            .set(handle)
            .map_err(|_| error("minds facts already registered"))
    }
}

/// Install one actual service/reader/provider union. Both Economy and Roll use
/// this same opaque readiness instance; no public setter can enable it.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: super::families::CommerceFamilies,
    ratings: super::rating::RatingFamilies,
    selected: &MindsFactProvider,
    definitions: [mush_core::DefinitionRef; 2],
) -> Result<MindsFactProvider, ModError> {
    let ready = selected.0.ready.clone();
    reader::register(reg, f.clone(), ratings.clone(), ready.clone())?;
    let handle = reader::Reader::request(reg)?;
    let runtime = service::Runtime::default();
    provider::register(
        reg,
        f.clone(),
        definitions.clone(),
        handle,
        ready.clone(),
        runtime.clone(),
    )?;
    service::register(
        reg,
        runtime,
        ready.clone(),
        f,
        ratings,
        definitions[0].clone(),
    )?;
    Ok(selected.clone())
}

#[cfg(test)]
mod m8d_tests {
    use super::*;

    /// **Doc 17h cut M8d fix round K6**: `minds.unwarranted` is scoped — every contract
    /// condition must name a scope for it, and the provider's public resolve answers nothing for
    /// it, whether the inspection read an ungoverned core or a governed one; `minds.seized` stays
    /// public.
    #[test]
    fn m8d_unwarranted_resolves_for_no_public_condition() {
        let provider = MindsFactProvider::new();
        let world = World::new_minimal();
        let hull = protocol::hull_key(mush_core::ObjId(12));
        for (kind, scoped) in [("unwarranted", true), ("claimed", true), ("seized", false)] {
            let fact = ExternalRef {
                provider: protocol::PROVIDER.into(),
                kind: kind.into(),
                id: hull.clone().into(),
                revision: Some(1),
            };
            assert_eq!(provider.requires_scope(&fact), scoped, "{kind}");
        }
        // The unwarranted and the governed inspection read alike: nothing resolves.
        for seq in [1, 2] {
            assert!(
                provider
                    .resolve(&world, "unwarranted", &hull, Some(seq))
                    .is_none()
            );
        }
    }
}
