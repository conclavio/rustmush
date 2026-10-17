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
//! mind, and until 17b cut M4.3 a mind's hull was a lab knob's or a fixture's
//! (no lobby, check-out or config names this runner in 17b cut M2; from M4.3
//! `crate::match_::start` seals every hull on `PRODUCTION_FILL_ENGINE` and
//! the installer gives a scripted seal this runner — `scripted` from the flip
//! to the fill decision of 2026-10-09, `native` since, so a production match
//! checked out since runs none and this runner flies a fixture's scripted
//! seal, or a match checked out between the flip and the decision, resumed —
//! until 17b cut M4.6's format 9, which voids every older snapshot).
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
//! 1. **A dropped job** (D-UM-32: three faults running, or a sealed core
//!    gone, `crate::arena::mind::MindState::dropped`; since 17b cut M3.1 also
//!    a hull whose mind class is `Zero` — no core, or a core at Class 0 — from
//!    its first think) is answered by **the ROM** — the runner's native
//!    stand-in, `NativeStockMind` on the match's doctrine (Standard
//!    Doctrine's native body), at the job's own card and Class 0's row, which
//!    is what the arena hands a dropped job — with [`DROPPED_LINE`] as the
//!    whole trace, and charged no budget: no [`ThinkStat`] is recorded. **The
//!    ROM is native by construction** (17b cut M3.4, the lead's ruling on
//!    17b cut M2's stop): the scripted Standard Doctrine cannot run inside
//!    Class 0's 400 operations, Gate A proved the native and the scripted
//!    stock minds identical, D-UM-32 says the ROM cannot fault and D-UM-33
//!    that a hull with no core runs only the ROM — so D-UM-32's "ceiling at
//!    twice its measured worst case" is met vacuously. The arena journals
//!    the drop (`mind_dropped`); the runner only answers.
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
//!    and a fault: since 17b cut M3.4 (`mush-mind`'s own order, M2's residual
//!    settled) a sandbox [`Fault::Panic`] is read FIRST (`panic`: a native
//!    function that panicked ends the think whatever the context latched
//!    before it), then the context's refusal word (`type`, `unknown`,
//!    `intents`, `limit`: the station's or the kernel's own word), then its
//!    overflow flag (`intents`), then the sandbox's own fault's cause word
//!    (`ops`, `limit`, `type`, `memory`). Every refusal is
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
/// it, one line, in place of anything the script would have printed — since
/// 17b cut M3.4 the ROM's own words, whatever dropped the hull (three faults,
/// a destroyed core, or no core at all).
pub const DROPPED_LINE: &str = "on the ROM: Standard Doctrine, native, at Class 0";

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
    /// The process sandbox (bounded LRU) or an explicitly disposable check sandbox.
    sandbox: Arc<Sandbox>,
    /// The script, compiled once on that sandbox under [`MIND_ABI`].
    compiled: Arc<Compiled>,
    /// **What answers a dropped job — the ROM** (D-UM-32; 17b cut M3.4):
    /// Standard Doctrine's native body on the match's doctrine, thinking at
    /// the job's card and its Class 0 row.
    stand_in: NativeStockMind,
    /// The doctrine's think cadence, ms (`decide_ms()` in a script).
    decide_ms: u32,
    /// The motor's frozen re-plan interval, ms (`repath_ms()` in a script).
    repath_ms: u32,
    /// The figures, when recording ([`RhaiRunner::recording`]).
    stats: Option<Mutex<Vec<ThinkStat>>>,
}

impl RhaiRunner {
    /// A runner for `source` on the process's bounded shared sandbox and the
    /// match's doctrine (`cfg.reflex`): the
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
            repath_ms: cfg.reflex.repath_ms,
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
        context::set(ThinkContext::of(job, self.decide_ms, self.repath_ms));
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
                Dynamic::from(Card(job.card.clone())),
            ],
            memory: job.memory,
            limits: &limits,
            ops_limit: job.profile.ops,
            fee_per_think: FEE_THINK,
        });
        // The context was set above on this thread and nothing between takes
        // it; a context function never removes it. Should it be gone, a fresh
        // one answers no intents and no flag — the think's own result stands.
        let ctx = context::take()
            .unwrap_or_else(|| ThinkContext::of(job, self.decide_ms, self.repath_ms));
        (result, trace, ctx)
    }
}

/// **The label rule** (the file head's step 3; D-UM-31): a sandbox
/// [`Fault::Panic`] first (17b cut M3.4), then the context's refusal word,
/// then its overflow, then the sandbox's own fault's cause word — or `None`
/// for a clean think.
fn label(ctx: &ThinkContext, fault: Option<&Fault>) -> Option<&'static str> {
    // 17b cut M3.4: a panic is the think's end whatever was latched before it
    // — the refusal it latched never ended the think, the panic did.
    if let Some(panicked @ Fault::Panic) = fault {
        return Some(panicked.cause());
    }
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
        if !job.card.is_hydrated()
            || job.profile_token.unit != job.unit
            || job.profile_token.key != job.card.key()
        {
            return ThinkResult {
                fault: Some(ThinkFault {
                    cause: "profile".into(),
                }),
                memory: job.memory.to_vec(),
                ..ThinkResult::default()
            };
        }
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
                diagnostics: Some(crate::arena::author_trace::ThinkDiagnostics::measured(
                    ops, fee,
                )),
                rule_sample: ctx.rule_sample,
                intents: ctx.intents,
                memory: outcome.memory,
                fault: None,
                trace,
            },
            (word, _) => ThinkResult {
                diagnostics: Some(crate::arena::author_trace::ThinkDiagnostics::measured(
                    ops, fee,
                )),
                rule_sample: None,
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
    /// flag `intents`, a sandbox fault its own word — and, since 17b cut
    /// M3.4, a sandbox `panic` before either flag; on every fault no intents
    /// and the job's memory.
    #[test]
    fn the_label_rule_reads_the_flags_first_on_both_ends() {
        let mut ctx = context_on(view());
        assert_eq!(label(&ctx, None), None);
        assert_eq!(label(&ctx, Some(&Fault::Ops)), Some("ops"));
        assert_eq!(label(&ctx, Some(&Fault::Panic)), Some("panic"));
        ctx.overflowed = true;
        assert_eq!(label(&ctx, None), Some("intents"));
        // 17b cut M3.4: the panic first (it was `intents` under M2's order).
        assert_eq!(label(&ctx, Some(&Fault::Panic)), Some("panic"));
        assert_eq!(label(&ctx, Some(&Fault::Ops)), Some("intents"));
        ctx.refused = Some(context::REFUSED_UNKNOWN);
        assert_eq!(label(&ctx, None), Some("unknown"));
        assert_eq!(label(&ctx, Some(&Fault::Limit)), Some("unknown"));
        // 17b cut M3.4: and before a latched refusal.
        assert_eq!(label(&ctx, Some(&Fault::Panic)), Some("panic"));

        // Through the runner: a refusal ends a fault, the memory handed back.
        let source = "fn think(v, c) { helm(5); this.k = 1; lock(999); }";
        let runner = RhaiRunner::new(&ArenaConfig::default(), source, fixture::digest(source))
            .expect("compiles")
            .recording();
        let seen = Arc::new(view());
        let defined = Arc::new(map());
        let held = fixture::runtime_card(card());
        let program = fixture::empty_program();
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
            program: &program,
            profile_token: crate::arena::profile_runtime::ProfileToken {
                unit: fixture::UNIT,
                generation: 0,
                key: held.key(),
            },
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

        // A dropped job: the stand-in, one trace line, no stat — the ROM at
        // Class 0's row, as the arena hands a dropped job (17b cut M3.4).
        let rom = crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::Zero);
        let dropped = ThinkJob {
            dropped: true,
            profile: &rom,
            ..job
        };
        let result = runner.think(&dropped);
        assert_eq!(
            DROPPED_LINE, "on the ROM: Standard Doctrine, native, at Class 0",
            "17b cut M3.4's words"
        );
        assert_eq!(result.trace, vec![DROPPED_LINE.to_owned()]);
        assert_eq!(result.fault, None, "the ROM cannot fault");
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
        let held = fixture::runtime_card(card());
        let program = fixture::empty_program();
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
            program: &program,
            profile_token: crate::arena::profile_runtime::ProfileToken {
                unit: fixture::UNIT,
                generation: 0,
                key: held.key(),
            },
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
                    Dynamic::from(Card(held.clone())),
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

    /// **A panic is labelled before a latched refusal** (17b cut M3.4, the
    /// label order settled as `mush-mind`'s own): a test-only native
    /// `refuse_then_boom()` on a second sandbox latches exactly what
    /// `lock(999)` latches — the `unknown` refusal, through the station's own
    /// `context::refuse` — and then panics. The sandbox answers
    /// `Fault::Panic`; under M2's order the latched `unknown` won, and under
    /// this cut's the think is `panic`: no intents, the job's memory, a
    /// faulted stat. The refusal alone, with no panic (`lock(999)`), is still
    /// `unknown`.
    #[test]
    fn a_panic_is_labelled_before_a_latched_lock_refusal() {
        use super::super::STOCK_OPS_PER_THINK;
        use super::super::register_api;
        use mush_mind::EngineLimits;

        let sandbox = Arc::new(Sandbox::new(
            &EngineLimits {
                max_ops_backstop: STOCK_OPS_PER_THINK,
            },
            |engine| {
                register_api(engine);
                engine.register_fn("refuse_then_boom", || -> mush_mind::rhai::INT {
                    let _ = context::with(|ctx| {
                        let _ = context::refuse(ctx, context::REFUSED_UNKNOWN, "No such contact.");
                    });
                    panic!("boom")
                });
            },
        ));
        let seen = Arc::new(view());
        let defined = Arc::new(map());
        let held = fixture::runtime_card(card());
        let program = fixture::empty_program();
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
            program: &program,
            profile_token: crate::arena::profile_runtime::ProfileToken {
                unit: fixture::UNIT,
                generation: 0,
                key: held.key(),
            },
        };
        let source = "fn think(v, c) { helm(5); this.k = 1; refuse_then_boom(); }";
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
            Some("panic".into()),
            "the panic over the latched refusal"
        );
        assert!(result.intents.is_empty(), "nothing lands");
        assert_eq!(result.memory, b"{\"k\":0}".to_vec(), "the memory kept");
        let stats = runner.take_stats();
        assert_eq!(stats.len(), 1);
        assert!(stats.iter().all(|stat| stat.faulted));

        // The refusal alone is still its own word.
        let refusal = "fn think(v, c) { lock(999); }";
        let alone = RhaiRunner::on(
            sandbox,
            &ArenaConfig::default(),
            refusal,
            fixture::digest(refusal),
        )
        .expect("compiles");
        assert_eq!(
            alone.think(&job).fault.map(|fault| fault.cause),
            Some("unknown".into())
        );
    }
}
