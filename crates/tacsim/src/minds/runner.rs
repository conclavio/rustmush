//! **The Rhai runner** (17b cut M2.4; doc 17 §6 "The think", §7 "The sandbox
//! and the budget", §8 "Faults", §14 "The trace", §19 "Fit with the crates";
//! D-UM-1, D-UM-22, D-UM-31, D-UM-32).
//!
//! [`RhaiRunner`] is a [`MindRunner`]: the arena hands it a [`ThinkJob`] and it
//! answers a [`ThinkResult`], exactly as the native stock mind does. The arena
//! never names it (D-UM-1: `crate::arena::mind` names no Rhai type, and the
//! seam is the whole contract); only a fixture sets it on
//! `Arena::mind_runner`. Nothing in production does: `Arena::new` and a
//! snapshot's decode both install `default_mind_runner`, the native stock
//! mind, and the lab knob (`[tacsim.minds].fill_controller = "mind"`) only
//! checks hulls out as `Controller::Mind`, flown by that native runner (no
//! lobby, check-out or config names this runner in 17b cut M2).
//!
//! **Synchronous, at the think point.** The sim task calls the runner inside
//! the think's due event and waits for the answer, as it calls
//! `NativeStockMind` — no pool, no tokio, no second thread (17b cut M4's
//! pool, D-UM-22, is the later shape). So one think runs on one thread from
//! start to end, which is the rule `mush_mind`'s budget and trace and this
//! crate's think context ([`super::context`]) rest on.
//!
//! # One think
//!
//! 1. **A dropped job** (D-UM-32: three faults running,
//!    `crate::arena::mind::MindState::dropped`; since 17b cut M3.1 also a
//!    hull whose mind class is `Zero` — no core, or a core at Class 0 — from
//!    its first think) is answered by the runner's
//!    native stand-in — `NativeStockMind` on the match's doctrine, Standard
//!    Doctrine's native body — with [`DROPPED_LINE`] as the whole trace. 17b
//!    cut M3 replaces the stand-in with the ROM and a `MindDropped` line.
//! 2. Otherwise the think's context is opened ([`context::set`]) from the job —
//!    the view and the map behind the job's own `Arc`s (cloned, never
//!    copied), the seed, the hull, the sequence and the two clocks, the
//!    doctrine's `decide_ms`; the script is called as `think(view, card)`
//!    with the job's memory bound as `this`, at the job's row's op ceiling and
//!    memory keys (17b cut M3.1: `ThinkJob::profile` — Class I's
//!    [`super::STOCK_OPS_PER_THINK`] and `STOCK_MEMORY_LIMITS` on every shipped
//!    chassis, the shape bounds `STOCK_MEMORY_LIMITS`' for every class)
//!    and the per-think fee ([`FEE_THINK`]); the context is taken back
//!    ([`context::take`]).
//! 3. **The label rule** (D-UM-31), on BOTH ends of the call — a clean return
//!    and a fault: the context's refusal word is read first (`type`,
//!    `unknown`, `intents`, `limit`: the station's or the kernel's own word),
//!    then its overflow flag (`intents`), then the sandbox's own fault's cause
//!    word (`ops`, `limit`, `type`, `memory`, `panic`). Every refusal is
//!    raised through `mush_mind::system_error`, uncatchable by `try` and
//!    latched for the think, so a refused think already ends a sandbox
//!    `type` fault; reading the flags on the `Ok` end too is the second wall
//!    — a refusal that somehow came back clean is still the think's fault,
//!    and still under its own word. On any fault the think answers **no
//!    intents** and the memory it was handed (`job.memory`), so the arena's
//!    atomic fault path keeps the old memory whatever the runner holds; on a
//!    clean end it answers the buffer's intents in call order and the memory
//!    the script left.
//!
//! The trace (doc 17 §14: at most eight lines) rides the result on both ends;
//! the arena drops it (never hashed, never journalled) and a recording
//! wrapper reads it in the fixtures.
//!
//! # The figures
//!
//! [`RhaiRunner::recording`] turns on [`ThinkStat`]s — each Rhai think's
//! operations and fees (the budget's own counters, read off
//! [`mush_mind::budget::spent`] after the call on either end), whether it
//! faulted, and its wall time in microseconds. **The recording timer is the
//! only clock this runner reads**, it is read only when recording is on, and
//! nothing it measures reaches the think's answer: the arena's path never
//! sees a clock (doc 16 §3.1, D-16-2). 17b cut M2.7 prints the figures.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use mush_core::ObjId;
use mush_mind::memory::Limits;
use mush_mind::rhai::Dynamic;
use mush_mind::{CompileError, Compiled, Digest, Fault, Sandbox, Think, budget};
use smol_str::SmolStr;

use super::api::View;
use super::card::Card;
use super::context::{self, REFUSED_INTENTS, ThinkContext};
use super::fees::FEE_THINK;
use super::{MIND_ABI, standard_sandbox};
use crate::arena::mind::{MindRunner, STOCK_MEMORY_LIMITS, ThinkFault, ThinkJob, ThinkResult};
use crate::arena::stock_mind::NativeStockMind;
use crate::config::ArenaConfig;

/// **The trace a dropped think answers** (D-UM-32; doc 17 §14): the whole of
/// it, one line, in place of anything the script would have printed.
pub const DROPPED_LINE: &str = "on doctrine: three faults running; Standard Doctrine's native stand-in thinks (M3 replaces it with the ROM)";

/// **One Rhai think's figures** (17b cut M2.7 reads them): which think, what
/// it spent, whether it faulted and how long it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinkStat {
    /// The hull that thought.
    pub unit: ObjId,
    /// The think's sequence number.
    pub think_seq: u64,
    /// The operations the budget counted (`mush_mind::budget::spent().0`),
    /// up to the end of the think on either end.
    pub ops: u64,
    /// The fees charged, the per-think fee among them
    /// (`mush_mind::budget::spent().1`).
    pub fee: u64,
    /// The think's wall time, microseconds: the context opened, the script
    /// run and the context taken back.
    pub rhai_us: u128,
    /// Whether the think faulted (under the label rule: a refusal or an
    /// overflow on a clean return counts).
    pub faulted: bool,
}

/// **A unit mind written in Rhai** (17b cut M2.4; the file head): one compiled
/// script on one sandbox, the native stand-in a dropped hull falls back to,
/// the doctrine's `decide_ms` the context functions read, and — when
/// recording — the figures.
#[derive(Debug)]
pub struct RhaiRunner {
    /// The sandbox the script runs on — the process's one
    /// ([`standard_sandbox`]) unless a test built another.
    sandbox: Arc<Sandbox>,
    /// The script, compiled once on that sandbox under [`MIND_ABI`].
    compiled: Arc<Compiled>,
    /// **What answers a dropped job** (D-UM-32): Standard Doctrine's native
    /// body on the match's doctrine. 17b cut M3's ROM replaces it.
    stand_in: NativeStockMind,
    /// The doctrine's think cadence, ms (`decide_ms()` in a script).
    decide_ms: u32,
    /// The figures, when recording ([`RhaiRunner::recording`]).
    stats: Option<Mutex<Vec<ThinkStat>>>,
}

impl RhaiRunner {
    /// **A runner for `source`** on the process's one sandbox
    /// ([`standard_sandbox`]) and the match's doctrine (`cfg.reflex`): the
    /// script compiled under [`MIND_ABI`] keyed by `digest` (the seal's
    /// SHA-256 of the source), the stand-in `NativeStockMind` on the same
    /// doctrine, `decide_ms` the doctrine's. A script that does not parse, or
    /// defines no `fn think(view, card)`, is refused.
    ///
    /// # Errors
    ///
    /// [`CompileError`] for a script that does not compile.
    pub fn new(
        cfg: &ArenaConfig,
        source: &str,
        digest: Digest,
    ) -> Result<RhaiRunner, CompileError> {
        RhaiRunner::on(standard_sandbox(), cfg, source, digest)
    }

    /// **[`RhaiRunner::new`] on another sandbox** — a test's second sandbox
    /// carrying a function the standard one does not (a native function that
    /// panics, for the `panic` fault's fixture), or 17b cut M4's per-worker
    /// one. Production calls [`RhaiRunner::new`].
    ///
    /// # Errors
    ///
    /// [`CompileError`] for a script that does not compile on `sandbox`.
    pub fn on(
        sandbox: Arc<Sandbox>,
        cfg: &ArenaConfig,
        source: &str,
        digest: Digest,
    ) -> Result<RhaiRunner, CompileError> {
        let compiled = sandbox.compile(source, digest, MIND_ABI)?;
        Ok(RhaiRunner {
            sandbox,
            compiled,
            stand_in: NativeStockMind::with(&cfg.reflex),
            decide_ms: cfg.reflex.decide_ms,
            stats: None,
        })
    }

    /// **The same runner, recording a [`ThinkStat`] per Rhai think** — the
    /// only path that reads a clock ([`Instant`], off the arena's path: the
    /// file head). A dropped job, answered by the stand-in, records nothing.
    #[must_use]
    pub fn recording(self) -> RhaiRunner {
        RhaiRunner {
            stats: Some(Mutex::new(Vec::new())),
            ..self
        }
    }

    /// **The figures recorded so far**, in think order, emptied — `[]` when
    /// not recording.
    #[must_use]
    pub fn take_stats(&self) -> Vec<ThinkStat> {
        self.stats.as_ref().map_or_else(Vec::new, |stats| {
            std::mem::take(&mut *stats.lock().unwrap_or_else(PoisonError::into_inner))
        })
    }

    /// One Rhai think of `job` (the file head's step 2), with the context it
    /// left: the sandbox's answer, the trace and the context taken back.
    fn run(
        &self,
        job: &ThinkJob<'_>,
    ) -> (Result<mush_mind::Outcome, Fault>, Vec<String>, ThinkContext) {
        context::set(ThinkContext::of(job, self.decide_ms));
        // 17b cut M3.1 (doc 17 §9, D-UM-35): the row the think is held to —
        // its op ceiling and its memory keys (the shape bounds are every
        // class's). Class I's 1,500 and 32 on every shipped chassis.
        let limits = Limits {
            keys: job.profile.memory_keys,
            ..STOCK_MEMORY_LIMITS
        };
        let (result, trace) = self.sandbox.think(Think {
            compiled: &self.compiled,
            args: vec![
                Dynamic::from(View(Arc::clone(job.view))),
                Dynamic::from(Card(Arc::new(job.card.clone()))),
            ],
            memory: job.memory,
            limits: &limits,
            ops_limit: job.profile.ops,
            fee_per_think: FEE_THINK,
        });
        // The context was set above on this thread and nothing between takes
        // it; a context function never removes it. Should it be gone, a fresh
        // one answers no intents and no flag — the think's own result stands.
        let ctx = context::take().unwrap_or_else(|| ThinkContext::of(job, self.decide_ms));
        (result, trace, ctx)
    }
}

/// **The label rule** (the file head's step 3; D-UM-31): the context's
/// refusal word first, then its overflow, then the sandbox's own fault's
/// cause word — or `None` for a clean think.
fn label(ctx: &ThinkContext, fault: Option<&Fault>) -> Option<&'static str> {
    if let Some(word) = ctx.refused {
        // `type`, `unknown`, `intents` or `limit` — each its own cause word.
        return Some(word);
    }
    if ctx.overflowed {
        return Some(REFUSED_INTENTS);
    }
    fault.map(Fault::cause)
}

impl MindRunner for RhaiRunner {
    /// **One think** (the file head).
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        if job.dropped {
            let mut result = self.stand_in.think(job);
            result.trace = vec![DROPPED_LINE.to_owned()];
            return result;
        }
        let started = self.stats.as_ref().map(|_| Instant::now());
        let (result, trace, ctx) = self.run(job);
        let (ops, fee) = budget::spent();
        let rhai_us = started.map_or(0, |started| started.elapsed().as_micros());
        let word = label(&ctx, result.as_ref().err());
        if let Some(stats) = &self.stats {
            stats
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ThinkStat {
                    unit: job.unit,
                    think_seq: job.think_seq,
                    ops,
                    fee,
                    rhai_us,
                    faulted: word.is_some(),
                });
        }
        match (word, result) {
            (None, Ok(outcome)) => ThinkResult {
                intents: ctx.intents,
                memory: outcome.memory,
                fault: None,
                trace,
            },
            (word, _) => ThinkResult {
                intents: Vec::new(),
                memory: job.memory.to_vec(),
                // A fault with no word cannot come out of `label` (an `Err`
                // always carries its cause); `panic` is the fallback so a
                // faulted think never reads clean.
                fault: Some(ThinkFault {
                    cause: SmolStr::new_static(word.unwrap_or("panic")),
                }),
                trace,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{self, card, context_on, map, view};
    use super::*;
    use crate::arena::mind::ThinkJob;

    /// The runner is a `MindRunner` the arena can hold: `Debug + Send + Sync`.
    #[test]
    fn the_runner_is_a_runner_the_arena_can_hold() {
        fn held<T: MindRunner + std::fmt::Debug + Send + Sync + 'static>() {}
        held::<RhaiRunner>();
        let source = "fn think(v, c) { }";
        let runner = RhaiRunner::new(&ArenaConfig::default(), source, fixture::digest(source))
            .expect("compiles");
        let _held: Arc<dyn MindRunner> = Arc::new(runner);
        assert_eq!(
            RhaiRunner::new(
                &ArenaConfig::default(),
                "fn other() {}",
                fixture::digest("fn other() {}")
            )
            .err(),
            Some(CompileError::NoThink)
        );
    }

    /// **The label rule on both ends** — a clean return with the refusal
    /// flag set still answers the flag's word (the second wall), an overflow
    /// flag `intents`, a sandbox fault its own word; on every fault no
    /// intents and the job's memory.
    #[test]
    fn the_label_rule_reads_the_flags_first_on_both_ends() {
        let mut ctx = context_on(view());
        assert_eq!(label(&ctx, None), None);
        assert_eq!(label(&ctx, Some(&Fault::Ops)), Some("ops"));
        ctx.overflowed = true;
        assert_eq!(label(&ctx, None), Some("intents"));
        assert_eq!(label(&ctx, Some(&Fault::Panic)), Some("intents"));
        ctx.refused = Some(context::REFUSED_UNKNOWN);
        assert_eq!(label(&ctx, None), Some("unknown"));
        assert_eq!(label(&ctx, Some(&Fault::Limit)), Some("unknown"));

        // Through the runner: a refusal ends a fault, the memory handed back.
        let source = "fn think(v, c) { helm(5); this.k = 1; lock(999); }";
        let runner = RhaiRunner::new(&ArenaConfig::default(), source, fixture::digest(source))
            .expect("compiles")
            .recording();
        let seen = Arc::new(view());
        let defined = Arc::new(map());
        let held = card();
        let job = ThinkJob {
            unit: fixture::UNIT,
            think_seq: 3,
            t_ms: 5_000,
            lands_ms: 5_100,
            seed: 42,
            dropped: false,
            profile: &crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::I),
            view: &seen,
            memory: b"{\"k\":0}",
            map: &defined,
            card: &held,
        };
        let result = runner.think(&job);
        assert_eq!(
            result.fault.map(|fault| fault.cause),
            Some("unknown".into())
        );
        assert!(result.intents.is_empty());
        assert_eq!(result.memory, b"{\"k\":0}".to_vec());
        let stats = runner.take_stats();
        assert_eq!(stats.len(), 1);
        assert!(stats.iter().all(|stat| stat.faulted && stat.think_seq == 3));
        assert!(runner.take_stats().is_empty(), "taken");

        // A dropped job: the stand-in, one trace line, no stat.
        let dropped = ThinkJob {
            dropped: true,
            ..job
        };
        let result = runner.think(&dropped);
        assert_eq!(result.trace, vec![DROPPED_LINE.to_owned()]);
        let mut native = NativeStockMind::with(&ArenaConfig::default().reflex).think(&dropped);
        native.trace = vec![DROPPED_LINE.to_owned()];
        assert_eq!(result, native);
        assert!(runner.take_stats().is_empty());
    }

    /// **The second wall, end to end** (the file head's step 3): a think
    /// whose script RETURNS cleanly with the context's overflow flag or
    /// refusal word set — which no registered station or kernel can leave,
    /// since each raises an uncatchable system error with its flag — is still
    /// the flag's fault through `MindRunner::think`: no intents, the job's
    /// memory, the flag's word, a faulted stat. A test-only `flag()` on a
    /// second sandbox sets the flag and returns, so the sandbox answers `Ok`.
    #[test]
    fn a_flag_left_on_a_clean_return_is_still_the_thinks_fault() {
        use super::super::STOCK_OPS_PER_THINK;
        use mush_mind::EngineLimits;

        use super::super::register_api;

        /// What a test-only `flag()` does to the running think's context.
        type Flag = fn(&mut ThinkContext);

        fn flagging(set: Flag) -> Arc<Sandbox> {
            Arc::new(Sandbox::new(
                &EngineLimits {
                    max_ops_backstop: STOCK_OPS_PER_THINK,
                },
                move |engine| {
                    register_api(engine);
                    engine.register_fn("flag", move || {
                        let _ = context::with(set);
                    });
                },
            ))
        }
        let seen = Arc::new(view());
        let defined = Arc::new(map());
        let held = card();
        let job = ThinkJob {
            unit: fixture::UNIT,
            think_seq: 3,
            t_ms: 5_000,
            lands_ms: 5_100,
            seed: 42,
            dropped: false,
            profile: &crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::I),
            view: &seen,
            memory: b"{\"k\":0}",
            map: &defined,
            card: &held,
        };
        let source = "fn think(v, c) { helm(5); this.k = 1; flag(); }";
        let cases: [(Flag, &str); 3] = [
            (|ctx| ctx.overflowed = true, "intents"),
            (
                |ctx| ctx.refused = Some(context::REFUSED_UNKNOWN),
                "unknown",
            ),
            (|ctx| ctx.refused = Some(context::REFUSED_LIMIT), "limit"),
        ];
        for (set, word) in cases {
            let sandbox = flagging(set);
            // The sandbox itself answers a clean end: the flag rides only the
            // context, which the runner reads.
            context::set(context_on(view()));
            let compiled = sandbox
                .compile(source, fixture::digest(source), MIND_ABI)
                .expect("compiles");
            let (ended, _) = sandbox.think(Think {
                compiled: &compiled,
                args: vec![
                    Dynamic::from(View(Arc::clone(&seen))),
                    Dynamic::from(Card(Arc::new(held.clone()))),
                ],
                memory: job.memory,
                limits: &STOCK_MEMORY_LIMITS,
                ops_limit: STOCK_OPS_PER_THINK,
                fee_per_think: FEE_THINK,
            });
            assert!(ended.is_ok(), "{word}: the script returned: {ended:?}");
            let left = context::take().expect("the context");
            assert_eq!(left.intents.len(), 1, "{word}: the helm buffered");

            let runner = RhaiRunner::on(
                Arc::clone(&sandbox),
                &ArenaConfig::default(),
                source,
                fixture::digest(source),
            )
            .expect("compiles")
            .recording();
            let result = runner.think(&job);
            assert_eq!(
                result.fault.map(|fault| fault.cause),
                Some(word.into()),
                "{word}"
            );
            assert!(result.intents.is_empty(), "{word}: nothing lands");
            assert_eq!(result.memory, b"{\"k\":0}".to_vec(), "{word}: kept");
            let stats = runner.take_stats();
            assert_eq!(stats.len(), 1);
            assert!(stats.iter().all(|stat| stat.faulted), "{word}");
        }
    }
}
