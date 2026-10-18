//! Metered bridge from immutable bound profiles to typed landing sidecars.
//! The registered helper calls this at most once per think; subsequent calls
//! return the context's saved result. It emits no station intents itself.

use crate::arena::mind::MindView;
use crate::arena::profile_rules::{BoundProgram, RuleSample};
use crate::arena::profile_runtime::ProfileToken;
use crate::arena::schema::SchemaError;

/// Charge and sample only the current think's public view/program. The wrapper
/// obtains every argument from ThinkContext, never from untrusted script ids.
/// # Errors
/// Invalid clock interval; the wrapper turns this into a whole-think type fault.
pub(crate) fn sample(
    program: &BoundProgram,
    view: &MindView,
    own_last_hit_ms: Option<i64>,
    token: ProfileToken,
    lands_ms: i64,
    next_lands_ms: i64,
) -> Result<RuleSample, SchemaError> {
    if view.t_ms < 0 || view.t_ms > lands_ms || lands_ms >= next_lands_ms {
        return Err(SchemaError {
            field: "rule sample".into(),
            message: "invalid frozen sampling interval".into(),
        });
    }
    super::fees::charge(program.sample_fee(view.contacts.len()));
    Ok(RuleSample {
        program: program.digest,
        generation: token.generation,
        sampled_ms: view.t_ms,
        lands_ms,
        expires_ms: next_lands_ms,
        truth: program.sample(view, own_last_hit_ms),
    })
}

/// Published `stock::rules()` implementation. Scripts supply no program, clock,
/// unit, generation or view; the successful runner's sidecar is the only output.
pub(crate) fn rules() -> Result<bool, Box<mush_mind::rhai::EvalAltResult>> {
    super::context::with(|ctx| {
        if ctx.rule_sample.is_some() {
            return Ok(!ctx.program.rows.is_empty());
        }
        if ctx.profile_token.unit != ctx.unit {
            return Err(super::context::refuse(
                ctx,
                super::context::REFUSED_TYPE,
                "profile owner does not match the current think",
            ));
        }
        match sample(
            &ctx.program,
            &ctx.view,
            ctx.view.own_last_hit_ms,
            ctx.profile_token,
            ctx.lands_ms,
            ctx.next_lands_ms,
        ) {
            Ok(sample) => {
                ctx.rule_sample = Some(sample);
                Ok(!ctx.program.rows.is_empty())
            }
            Err(_) => Err(super::context::refuse(
                ctx,
                super::context::REFUSED_TYPE,
                "invalid profile sampling interval",
            )),
        }
    })?
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mush_core::ObjId;

    use crate::arena::author_trace::Outcome;
    use crate::arena::intents::{IntentKind, MotorGoal};
    use crate::arena::mind::CollectedThink;
    use crate::arena::motor::MotorCounters;
    use crate::arena::state::{Arena, Controller, Power, Team};
    use crate::arena::test_support::{arena_of, line_tank, pose};

    const OWN: ObjId = ObjId(4);
    const SOURCE: &str = "fn think(v,c) { stock::rules(); goto([2,4]); this.landed = t_ms(); }";

    fn arena() -> Arena {
        let mut own = line_tank(OWN, Team::Red, pose(1500, 4500));
        own.controller = Controller::Mind;
        own.reseal(&crate::match_::SealTerms::fixture())
            .expect("fixture seal");
        let mut arena = arena_of(vec![own, line_tank(ObjId(9), Team::Blue, pose(6500, 4500))]);
        arena.unit_mut(OWN).expect("own hull").power = Power::Online;
        arena.mind_runner = Arc::new(
            crate::minds::RhaiRunner::on(
                crate::minds::standard_sandbox(),
                &crate::config::ArenaConfig::default(),
                SOURCE,
                crate::minds::fixture::digest(SOURCE),
            )
            .expect("real sampling helper and goto compile"),
        );
        arena
    }

    fn collect_off_grid(arena: &mut Arena, scheduled_ms: i64) -> CollectedThink {
        // The real due loop advances motion/now_ms to the boundary before
        // collecting events by their unchanged scheduled millisecond.
        arena.now_ms = scheduled_ms + 249;
        let sequence = arena.minds.get(&OWN).expect("mind").think_seq;
        arena.collect_think(OWN, sequence, scheduled_ms, &mut MotorCounters::default());
        let think = arena.collected.pop().expect("actual collection");
        assert_eq!(think.t_ms, scheduled_ms);
        assert_eq!(think.view.t_ms, scheduled_ms + 249);
        assert!(think.view.t_ms < think.lands_ms);
        think
    }

    #[test]
    fn off_grid_samples_bind_the_captured_picture_and_land_without_strikes() {
        let mut arena = arena();
        for scheduled_ms in [1001, 2001, 3001] {
            let think = collect_off_grid(&mut arena, scheduled_ms);
            let answer = think.think();
            assert!(answer.result.fault.is_none());
            assert_eq!(
                answer
                    .result
                    .rule_sample
                    .as_ref()
                    .expect("real sample")
                    .sampled_ms,
                think.view.t_ms
            );
            assert_eq!(
                answer.result.intents,
                vec![IntentKind::Goal(MotorGoal::Cell { x: 2, y: 4 })]
            );
            let settled = arena.settle_collected(&think, answer.result);
            assert_eq!(settled.written, 1);
            assert_eq!(settled.faults, 0);
            arena.now_ms = think.lands_ms;
            arena.mind_land(
                OWN,
                think.think_seq,
                think.lands_ms,
                &mut MotorCounters::default(),
            );
            let mind = arena.minds.get(&OWN).expect("mind");
            assert_eq!(mind.strikes, 0);
            assert!(!mind.dropped());
            assert_eq!(
                mush_mind::memory::decode(&mind.memory)
                    .expect("landed memory")
                    .get("landed"),
                Some(&mush_mind::memory::Value::Int(scheduled_ms))
            );
            let trace = arena.traces.recent(OWN, None).pop().expect("exact trace");
            assert_eq!(trace.think_seq, think.think_seq);
            assert_eq!(trace.outcome, Outcome::Applied);
            assert_eq!(trace.fault, None);
            assert_eq!(trace.rom_drop, None);
        }
    }

    #[test]
    fn a_sample_stamped_with_the_timer_instead_of_its_picture_is_refused() {
        let mut arena = arena();
        let think = collect_off_grid(&mut arena, 1001);
        let mut answer = think.think();
        assert!(answer.result.fault.is_none());
        answer
            .result
            .rule_sample
            .as_mut()
            .expect("real sample")
            .sampled_ms = think.t_ms;
        let settled = arena.settle_collected(&think, answer.result);
        assert_eq!(settled.written, 0);
        assert_eq!(settled.faults, 1);
        let mind = arena.minds.get(&OWN).expect("mind");
        assert_eq!(mind.strikes, 1);
        assert!(mind.memory.is_empty());
        assert!(mind.pending.is_none());
    }
}
