//! **The scripted mind** (lifted out of `tests/minds.rs` by 17b cut M4.4 so the
//! motor family in `tests/motor.rs` can drive a hull by a script's goals at the
//! reflex's own milliseconds): a `MindRunner` that reads its answers off a
//! per-hull script and records every job it is handed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::ObjId;
use smol_str::SmolStr;
use tacsim::arena::intents::IntentKind;
use tacsim::arena::mind::{
    MindRunner, MindView, STOCK_THINK_MS, ThinkFault, ThinkJob, ThinkResult,
};
use tacsim::arena::rng::stagger_ms;
use tacsim::arena::state::Arena;

use super::rig::SEED;

/// The lattice millisecond of `unit`'s think `n` on the fixtures' seed.
#[must_use]
pub fn think_at(unit: ObjId, n: i64) -> i64 {
    stagger_ms(SEED, unit, STOCK_THINK_MS) + n * STOCK_THINK_MS
}

/// One scripted think: what it answers, and the memory it hands back (the
/// memory it was handed, echoed, when `None`) — and, since 17b cut M2.2, the
/// fault it answers (D-UM-31: a cause word, or `None` for a clean think).
#[derive(Clone, Debug)]
pub struct Says {
    pub think_seq: u64,
    pub intents: Vec<IntentKind>,
    pub memory: Option<Vec<u8>>,
    pub fault: Option<&'static str>,
}

pub fn says(think_seq: u64, intents: Vec<IntentKind>) -> Says {
    Says {
        think_seq,
        intents,
        memory: None,
        fault: None,
    }
}

/// What a think was handed — and, since 17b cut M2.2, the match seed the job
/// carried (`ThinkJob::seed`, D-UM-29) and whether it said the hull is
/// dropped (`ThinkJob::dropped`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub unit: ObjId,
    pub think_seq: u64,
    pub t_ms: i64,
    pub view: MindView,
    pub memory: Vec<u8>,
    pub seed: u64,
    pub dropped: bool,
}

/// **A [`MindRunner`] that reads its answers off a script** — per hull, the
/// intents and the memory of think `n` — and records every job it is handed.
/// A pure function of the job and the script, as a runner must be.
#[derive(Debug, Default)]
pub struct ScriptedMind {
    pub script: BTreeMap<ObjId, Vec<Says>>,
    pub seen: Mutex<Vec<Seen>>,
}

impl ScriptedMind {
    pub fn new(script: &[(ObjId, Vec<Says>)]) -> Arc<ScriptedMind> {
        Arc::new(ScriptedMind {
            script: script.iter().cloned().collect(),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for ScriptedMind {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(Seen {
                unit: job.unit,
                think_seq: job.think_seq,
                t_ms: job.t_ms,
                view: (**job.view).clone(),
                memory: job.memory.to_vec(),
                seed: job.seed,
                dropped: job.dropped,
            });
        }
        let line = self
            .script
            .get(&job.unit)
            .and_then(|lines| lines.iter().find(|line| line.think_seq == job.think_seq));
        ThinkResult {
            diagnostics: None,
            rule_sample: None,
            intents: line.map(|line| line.intents.clone()).unwrap_or_default(),
            memory: line
                .and_then(|line| line.memory.clone())
                .unwrap_or_else(|| job.memory.to_vec()),
            fault: line.and_then(|line| line.fault).map(|cause| ThinkFault {
                cause: SmolStr::new_static(cause),
            }),
            trace: Vec::new(),
        }
    }
}

/// What a stock think was handed and answered: the hull, the lattice
/// millisecond, the view and the result.
pub type Kept = (ObjId, i64, MindView, ThinkResult);

/// **The production stock mind, its answers kept** — `NativeStockMind` on the
/// arena's own doctrine, answering every think untouched and recording each
/// view and answer (an input recorder, not a stand-in: 16b §0.4). **17b cut
/// M2.5**: over any runner ([`Stock::over`]) — the twin of Gate A
/// (`support::TwinRunner`), whose answer IS the native one.
#[derive(Debug)]
pub struct Stock {
    pub inner: Arc<dyn MindRunner>,
    pub kept: Mutex<Vec<Kept>>,
}

impl Stock {
    /// The recorder over the production stock mind the arena flies: the
    /// runner its field was opened with (`default_mind_runner`, T1 —
    /// `NativeStockMind` on the arena's own doctrine). **17b cut M2.6**: it
    /// reads the arena's runner, so a field a fixture builds under the twin
    /// (`support::under_the_twin`, whose field builders call
    /// `support::twin_if_asked` and thereby `support::with_twin` BEFORE this
    /// recorder reads the runner) is recorded over that field's own asserting
    /// twin, whose answer is T1's. Anything else on the arena is a fixture's
    /// own runner and is [`Stock::over`]'s to record, never this one's.
    /// **17b cut M2.7**: or, inside `support::under_the_rhai_runner`, over the
    /// field's Rhai Standard Doctrine alone (`support::with_rhai`), whose
    /// answer is T2's own. A `RhaiRunner` is admitted ONLY there
    /// (`support::rhai_asked`) and only over the shipped script (its compiled
    /// digest is `standard_digest()`): every `RhaiRunner` prints the same
    /// prefix, so a fixture's own scripted runner is never recorded as the
    /// stock mind. **17b cut M3.5**: this reads the arena-wide FALLBACK; a
    /// hull's own runner, installed by its seal, is [`Stock::of`]'s.
    pub fn on(arena: &Arena) -> Arc<Stock> {
        let runner = Arc::clone(&arena.mind_runner);
        let name = format!("{runner:?}");
        let standard = format!("{:?}", tacsim::minds::standard_digest());
        assert!(
            name.starts_with("NativeStockMind")
                || name.starts_with("TwinRunner")
                || (name.starts_with("RhaiRunner")
                    && super::rhai_asked()
                    && name.contains(&standard)),
            "Stock::on records the production stock mind (or the twin over it, or the Rhai \
             Standard Doctrine alone under super::under_the_rhai_runner), not {name:.40}"
        );
        Stock::over(runner)
    }

    /// **The recorder over `unit`'s own runner, installed by its seal** (17b
    /// cut M3.5; doc 17 §13): `Arena::runner_for(unit)` — admitted as the
    /// stock mind exactly as [`Stock::on`] admits `with_rhai`'s, a
    /// `RhaiRunner` over the shipped script (its compiled digest
    /// `standard_digest()`), and only where the PRODUCTION installer put it
    /// on `Arena::runners` for that hull (`tacsim::minds::install_sealed_runners`
    /// over a `scripted` seal) — never a fixture's own scripted runner.
    pub fn of(arena: &Arena, unit: ObjId) -> Arc<Stock> {
        let installed = arena.runners.contains_key(&unit)
            && arena
                .unit(unit)
                .is_some_and(|state| state.seal.engine == tacsim::arena::SealEngine::Scripted);
        let runner = Arc::clone(arena.runner_for(unit));
        let name = format!("{runner:?}");
        let standard = format!("{:?}", tacsim::minds::standard_digest());
        assert!(
            installed && name.starts_with("RhaiRunner") && name.contains(&standard),
            "Stock::of records a Standard Doctrine RhaiRunner the seal installed, not \
             {name:.40}"
        );
        Stock::over(runner)
    }

    /// The same recorder over `inner` (17b cut M2.5).
    pub fn over(inner: Arc<dyn MindRunner>) -> Arc<Stock> {
        Arc::new(Stock {
            inner,
            kept: Mutex::new(Vec::new()),
        })
    }

    pub fn kept(&self) -> Vec<Kept> {
        self.kept
            .lock()
            .map(|kept| kept.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for Stock {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        // 17b cut M2.7: held to the native think under
        // `support::under_the_rhai_runner` (a no-op outside it).
        super::note_think(job.unit, job.t_ms, &result);
        if let Ok(mut kept) = self.kept.lock() {
            kept.push((job.unit, job.t_ms, (**job.view).clone(), result.clone()));
        }
        result
    }
}
