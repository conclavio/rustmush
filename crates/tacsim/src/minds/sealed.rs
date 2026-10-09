//! **Each hull's runner, by its seal** (17b cut M3.5; doc 17 §13 "The
//! seal", "Records, the seal and replay"; D-UM-1, D-UM-45).
//!
//! A seal records the engine a hull's mind runs on
//! ([`crate::arena::mind::SealEngine`]): `reflex` (the machine), `native` (the
//! native stock mind, the arena-wide fallback `Arena::mind_runner`) or
//! `scripted` (a Rhai mind, the source the seal names). The arena keeps each
//! unit's seal by reference ([`crate::arena::mind::SealRef`] on
//! `UnitState::seal`) and never names the interpreter (D-UM-1); this module
//! is the one place a scripted seal becomes a runner: [`install_sealed_runners`]
//! walks every unit's seal and installs, on `Arena::runners`, a
//! [`RhaiRunner`] over the source a scripted seal names — compiled once per
//! distinct source and the `Arc` shared — and nothing for `native` or
//! `reflex`, which think on the fallback.
//!
//! **Called at both ends of a match's life**: by
//! `crate::arena_task::spawn_arena` over a fresh check-out and by
//! `crate::arena_task::resume_arena` over a decoded snapshot, whose runners
//! are re-derived EMPTY (`crate::arena::snapshot`'s head) — so a resumed
//! scripted hull thinks on the runner its seal names, and a native one on
//! the fallback, with no fixture re-applying anything. No production seal is
//! `scripted` since the fill decision of 2026-10-09
//! (`crate::match_::PRODUCTION_FILL_ENGINE` is `native`: the fill and every
//! seated hull think on the fallback), so on the production path it installs
//! nothing for a match checked out since; from 17b cut M4.3's flip to the
//! decision every production seal was `scripted`, and a match checked out
//! then still resumes on the runner its seal names. A fixture's rig
//! (`tests/support`'s `Muster::scripted`) seals a scripted mind.
//!
//! **What names the source.** The arena holds the seal's DIGEST
//! ([`crate::arena::mind::SealRef::digest`], the SHA-256 of the whole
//! [`crate::arena::mind::MindSeal`]), not the seal: the record does
//! (`crate::match_::MatchRecord::minds`), and the custody the arena was opened
//! from. So the caller hands the installer `sources`, which answers the
//! source TEXT for a seal digest — in production the store's
//! (`crate::store::TacsimStore::mind_source`, content-addressed by the seal's
//! `source_sha256`) for every scripted seal of the custody or the record,
//! keyed by that seal's digest. The installer compiles the text under ITS OWN
//! SHA-256 (the sandbox's `(Digest, abi)` cache key, which a content-addressed
//! store makes the seal's `source_sha256` by construction), so two hulls
//! flying one mind share one compilation.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::ObjId;
use mush_mind::{CompileError, Digest};
use sha2::{Digest as _, Sha256};

use super::RhaiRunner;
use crate::arena::mind::{MindRunner, SealEngine};
use crate::arena::state::Arena;
use crate::config::ArenaConfig;

/// **Why a sealed runner could not be installed** (17b cut M3.5): the match
/// cannot be opened (`crate::arena_bridge::OpenMatchError::Mind`) — a mind the
/// seal names that cannot be run is never silently swapped for another.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// A scripted seal whose digest `sources` answered no source for.
    #[error("{unit}'s sealed mind (seal {digest_text}) has no source on this server", digest_text = crate::arena::digest_text(.digest))]
    NoSource {
        /// The hull.
        unit: ObjId,
        /// The seal's digest (`UnitState::seal.digest`).
        digest: [u8; 32],
    },
    /// The source a scripted seal names does not compile on the standard
    /// sandbox under this server's `MIND_ABI`.
    #[error("{unit}'s sealed mind does not compile: {error}")]
    Compile {
        /// The hull.
        unit: ObjId,
        /// The sandbox's own refusal.
        error: CompileError,
    },
}

/// **Install every scripted seal's runner on `arena`** (17b cut M3.5; this
/// module's head): for every unit whose seal's engine is
/// [`SealEngine::Scripted`], in unit-id order, the source `sources` answers
/// for the seal's digest, compiled on the standard sandbox
/// ([`RhaiRunner::new`] at `cfg`'s doctrine, keyed by the source's own
/// SHA-256) and set on `Arena::runners` for that unit — one compilation per
/// distinct source, its `Arc` cloned. `native` and `reflex` seals install
/// nothing (they think on `Arena::mind_runner`). Answers how many hulls were
/// given a runner.
///
/// All or nothing: on a refusal `arena.runners` is left as it was.
///
/// # Errors
///
/// [`InstallError::NoSource`] for a scripted seal `sources` has no source for,
/// [`InstallError::Compile`] for a source that does not compile — the first,
/// in unit-id order.
pub fn install_sealed_runners(
    arena: &mut Arena,
    cfg: &ArenaConfig,
    sources: &dyn Fn(&[u8; 32]) -> Option<String>,
) -> Result<usize, InstallError> {
    let mut compiled: BTreeMap<[u8; 32], Arc<dyn MindRunner>> = BTreeMap::new();
    let mut installed: BTreeMap<ObjId, Arc<dyn MindRunner>> = BTreeMap::new();
    for unit in &arena.units {
        if unit.seal.engine != SealEngine::Scripted {
            continue;
        }
        let source = sources(&unit.seal.digest).ok_or(InstallError::NoSource {
            unit: unit.id,
            digest: unit.seal.digest,
        })?;
        let key: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let runner = match compiled.get(&key) {
            Some(runner) => Arc::clone(runner),
            None => {
                let runner: Arc<dyn MindRunner> =
                    Arc::new(RhaiRunner::new(cfg, &source, Digest(key)).map_err(|error| {
                        InstallError::Compile {
                            unit: unit.id,
                            error,
                        }
                    })?);
                compiled.insert(key, Arc::clone(&runner));
                runner
            }
        };
        installed.insert(unit.id, runner);
    }
    let count = installed.len();
    arena.runners.extend(installed);
    Ok(count)
}

#[cfg(test)]
mod tests {
    use mush_core::ObjId;

    use super::*;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, pose, tank};
    use crate::minds::{STANDARD_RHAI, standard_digest};

    /// Two hulls on the test window, red #1 and blue #2, with red sealed on
    /// `engine` under the seal digest `[7; 32]`.
    fn field(engine: SealEngine) -> Arena {
        let mut arena = arena_of(vec![
            tank(ObjId(1), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(2), Team::Blue, pose(6_500, 6_500)),
        ]);
        if let Some(unit) = arena.unit_mut(ObjId(1)) {
            unit.seal.engine = engine;
            unit.seal.digest = [7; 32];
        }
        arena
    }

    /// **17b cut M3.5**: a scripted seal whose digest names no source is
    /// refused by name — the hull and the digest — and installs nothing; a
    /// source that does not compile is refused with the sandbox's own words;
    /// the stock source installs one `RhaiRunner` keyed by its own digest;
    /// `native` and `reflex` seals install nothing and ask for no source.
    #[test]
    fn m3_install_sealed_runners_refuses_by_name_and_installs_by_engine() {
        let cfg = ArenaConfig::default();
        let none = |_: &[u8; 32]| None;

        let mut arena = field(SealEngine::Scripted);
        let refused = install_sealed_runners(&mut arena, &cfg, &none);
        assert_eq!(
            refused,
            Err(InstallError::NoSource {
                unit: ObjId(1),
                digest: [7; 32],
            })
        );
        assert_eq!(
            refused.map_err(|error| error.to_string()),
            Err(format!(
                "#1's sealed mind (seal {}) has no source on this server",
                "07".repeat(32)
            ))
        );
        assert!(arena.runners.is_empty(), "nothing installed on a refusal");

        let broken = |_: &[u8; 32]| Some("fn think(view) { }".to_owned());
        let refused = install_sealed_runners(&mut arena, &cfg, &broken);
        assert_eq!(
            refused,
            Err(InstallError::Compile {
                unit: ObjId(1),
                error: CompileError::NoThink,
            })
        );
        assert_eq!(
            refused.map_err(|error| error.to_string()),
            Err(
                "#1's sealed mind does not compile: the mind defines no fn think of two \
                 parameters"
                    .to_owned()
            )
        );
        assert!(arena.runners.is_empty(), "nothing installed on a refusal");

        let asked = std::cell::Cell::new(0_u32);
        let stock = |digest: &[u8; 32]| {
            asked.set(asked.get() + 1);
            (*digest == [7; 32]).then(|| STANDARD_RHAI.to_owned())
        };
        assert_eq!(install_sealed_runners(&mut arena, &cfg, &stock), Ok(1));
        assert_eq!(asked.get(), 1, "one scripted seal, one source asked");
        let shown = format!("{:?}", arena.runner_for(ObjId(1)));
        assert!(shown.starts_with("RhaiRunner"), "{shown:.40}");
        assert!(
            shown.contains(&format!("{:?}", standard_digest())),
            "compiled under the source's own digest: {shown:.200}"
        );
        assert!(
            format!("{:?}", arena.runner_for(ObjId(2))).starts_with("NativeStockMind"),
            "the unsealed hull thinks on the fallback"
        );

        for engine in [SealEngine::Native, SealEngine::Reflex] {
            let mut arena = field(engine);
            let never = |_: &[u8; 32]| -> Option<String> {
                panic!("a {} seal asks for no source", engine.as_str())
            };
            assert_eq!(install_sealed_runners(&mut arena, &cfg, &never), Ok(0));
            assert!(arena.runners.is_empty());
        }
    }

    /// **One compilation per distinct source** (17b cut M3.5): two scripted
    /// hulls naming one source share one runner's `Arc`.
    #[test]
    fn m3_two_seals_of_one_source_share_one_runner() {
        let cfg = ArenaConfig::default();
        let mut arena = field(SealEngine::Scripted);
        if let Some(unit) = arena.unit_mut(ObjId(2)) {
            unit.seal.engine = SealEngine::Scripted;
            unit.seal.digest = [9; 32];
        }
        let stock = |_: &[u8; 32]| Some(STANDARD_RHAI.to_owned());
        assert_eq!(install_sealed_runners(&mut arena, &cfg, &stock), Ok(2));
        let (Some(red), Some(blue)) = (arena.runners.get(&ObjId(1)), arena.runners.get(&ObjId(2)))
        else {
            panic!("both hulls have a runner");
        };
        assert!(Arc::ptr_eq(red, blue), "one compilation, the Arc shared");
    }
}
