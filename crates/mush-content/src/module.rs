//! The [`ContentModule`] — the latch slot, the composed appliers, and the first-boot
//! apply of whatever packs the composition root shipped (plan §3.1; doc 14 §3.1).
//!
//! ## Where it sits in the roster
//!
//! **Last.** Its boot pass applies packs whose entries are made by *other* modules'
//! appliers — an NPC entry writes a lore pool into the NPC host's RAM slot, a room
//! entry writes an atlas record into the atlas's — and `on_world_loaded` runs in list
//! order, so every owner must have hydrated its own slot before this pass runs or the
//! owner's boot would overwrite what the pass wrote. Registration order does not
//! matter to it: it registers one state slot and one nothing else.
//!
//! ## What boot does, and does not
//!
//! - A shipped pack with **no latch at all** is applied. That is the first boot of a
//!   fresh world, and only that.
//! - A shipped pack whose latch is **retired** is left retired: staff removed it on
//!   purpose, and a restart is not a staff act.
//! - A shipped pack whose latch holds an **older version** is left alone and logged:
//!   `@atlas/pack plan` shows the diff, and a staffer applies it. A restart never
//!   rewrites a lot underneath its players.
//! - A first-boot pack that will not apply, or stops with an entry anomaly,
//!   **fails boot** with the pack and cause (16c cut L, T-85). The host does not
//!   persist boot output or open listeners after that failure. This supersedes
//!   doc 14 §3.1's minimal-world fallback for automatic first-boot packs; manual
//!   applies still latch their partial progress honestly.
//! - **No call leaves the world task.** Applying a pack is a pure function of the pack
//!   and the world; nothing here can reach a provider (P9).

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};
use mush_core::{CommandCompatibility, Ctx};

use crate::apply::{Appliers, ApplyError, ApplyReport, apply};
use crate::latch::{Applied, Latches, NS};
use crate::pack::Pack;

/// The content module's state slot: the latches this world holds and the appliers the
/// composition root registered.
#[derive(Clone, Debug, Default)]
pub struct ContentState {
    /// The latch per applied pack.
    pub latches: Latches,
    /// Every entry kind's owner.
    pub appliers: Appliers,
}

/// The content [`GameModule`].
#[derive(Debug, Default)]
pub struct ContentModule {
    appliers: Appliers,
    boot_packs: Vec<Pack>,
}

impl ContentModule {
    /// A module with no appliers and nothing to apply at boot.
    #[must_use]
    pub fn new() -> ContentModule {
        ContentModule::default()
    }

    /// The appliers the composition root assembled — every kind's owner.
    #[must_use]
    pub fn with_appliers(mut self, appliers: Appliers) -> ContentModule {
        self.appliers = appliers;
        self
    }

    /// The packs to apply on a first boot, in order (`requires` order: a pack after
    /// the packs it names).
    #[must_use]
    pub fn with_boot_packs(mut self, packs: Vec<Pack>) -> ContentModule {
        self.boot_packs = packs;
        self
    }

    /// The packs this module applies on a first boot.
    #[must_use]
    pub fn boot_packs(&self) -> &[Pack] {
        &self.boot_packs
    }
}

impl GameModule for ContentModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::LegacyPostImage);
        reg.state(ContentState {
            latches: Latches::new(),
            appliers: self.appliers.clone(),
        });
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let latches = Latches::load(&data.kv);
        for (key, reason) in latches.held() {
            tracing::error!(%key, %reason, "content: a latch document is held undecodable; its pack cannot be applied or retired until it is repaired");
        }
        {
            let slot = ctx
                .state_mut::<ContentState>()
                .ok_or_else(|| ModError::msg("content: state slot missing"))?;
            slot.latches = latches;
        }
        for pack in &self.boot_packs {
            let id = pack.manifest().id().clone();
            let already = ctx
                .state::<ContentState>()
                .and_then(|s| s.latches.applied(&id).cloned());
            match already {
                None => match apply_pack(ctx, pack) {
                    Ok((applied, report)) => {
                        if let Some(reason) = report.anomaly {
                            return Err(ModError::msg(format!(
                                "content: first-boot pack {id}@{} stopped: {reason}",
                                pack.manifest().version()
                            )));
                        }
                        tracing::info!(
                            pack = %id,
                            version = applied.version(),
                            created = report.created,
                            "content: first-boot pack applied"
                        );
                    }
                    Err(error) => {
                        return Err(ModError::msg(format!(
                            "content: first-boot pack {id}@{} refused: {error}",
                            pack.manifest().version()
                        )));
                    }
                },
                Some(latch) if latch.is_retired() => {
                    tracing::info!(pack = %id, "content: shipped pack is retired; not re-applied at boot");
                }
                Some(latch) if latch.version() < pack.manifest().version() => {
                    tracing::info!(
                        pack = %id,
                        applied = latch.version(),
                        shipped = pack.manifest().version(),
                        "content: a newer version of a shipped pack is available; `@atlas/pack plan` shows the diff"
                    );
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

/// Apply `pack` against this world's latches with the composed appliers.
///
/// The latches are taken out of the state slot for the duration (the apply needs the
/// world mutably and the latches mutably, and the slot lives in the world) and put
/// back whatever happened, so a refused apply loses nothing.
///
/// # Errors
///
/// [`ApplyError`] when nothing was touched; a missing state slot is reported as
/// [`ApplyError::Held`] naming the pack, since without the slot no latch can be read.
pub fn apply_pack(ctx: &mut Ctx<'_>, pack: &Pack) -> Result<(Applied, ApplyReport), ApplyError> {
    with_state(
        ctx,
        pack.manifest().id().as_str(),
        |ctx, latches, appliers| apply(ctx, latches, appliers, pack),
    )
}

/// Retire `pack` against this world's latches. See [`apply_pack`] for the borrow
/// discipline.
///
/// # Errors
///
/// [`ApplyError`] when nothing was touched.
pub fn retire_pack(
    ctx: &mut Ctx<'_>,
    pack: &mush_core::CanonicalId,
    reason: &str,
) -> Result<(Applied, ApplyReport), ApplyError> {
    with_state(ctx, pack.as_str(), |ctx, latches, appliers| {
        crate::apply::retire_pack(ctx, latches, appliers, pack, reason)
    })
}

/// Run `f` with the latches taken out of the slot and put back afterwards.
///
/// WS03-S14: applying a pack is something an OWNER asks for — `@atlas/pack
/// apply` runs under the atlas's identity, `+build` under worldgen's — so the
/// checked `Ctx::state_mut` would refuse this write of the content module's own
/// slot and the latch would go to the store without ever reaching RAM. Same
/// crate, same module, only the context's name differs, so the unchecked world
/// accessor is the right spelling and `state-slot-owners.toml` wants no row
/// (that file baselines a crate writing ANOTHER module's slot).
fn with_state<T>(
    ctx: &mut Ctx<'_>,
    pack: &str,
    f: impl FnOnce(&mut Ctx<'_>, &mut Latches, &Appliers) -> Result<T, ApplyError>,
) -> Result<T, ApplyError> {
    let Some(slot) = ctx.w.module_state_mut::<ContentState>() else {
        return Err(ApplyError::Held {
            pack: format!("{pack} (no content state slot on this world)"),
        });
    };
    let mut latches = std::mem::take(&mut slot.latches);
    let appliers = slot.appliers.clone();
    let result = f(ctx, &mut latches, &appliers);
    if let Some(slot) = ctx.w.module_state_mut::<ContentState>() {
        slot.latches = latches;
    }
    result
}
