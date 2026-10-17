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
