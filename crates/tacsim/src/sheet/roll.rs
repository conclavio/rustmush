//! `+roll <expr>` — the generic dice-expression roll, and nothing else.
//!
//! 16b R5 keeps [`mush_dice`]'s expression evaluator and deletes `owod20` and
//! `sr5`, because doc 16 §3.5's combat resolution is a keyed-hash draw against a
//! hit curve and never touches a die. So `+roll` is what it says: `2d6`,
//! `3d6+2`, `4d6kh3`, `10`. There is no pool, no difficulty, no specialty, no
//! Willpower, no wound penalty and no contested form — the five switches and
//! nine inline options the WoD verb carried all described a mechanic that is
//! gone.
//!
//! The NPC `check` ability ([`super::agent`]) resolves through
//! [`roll_expression`] here, so a storyteller's `+roll 3d6` and an NPC's
//! `check(gunnery)` draw from the same seeded stream in the same way.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, SceneClass};
use mush_dice::{DiceExpr, RollCtx, RollLimits, RollOutcome};
use mush_proto::{NarrativeKind, OutputEvent};
use mush_ruleset::{RollSide, roll_narration};
use rand::RngCore;

use crate::sheet::{NS, PilotSheetState};

/// Parse and evaluate one dice expression against `rng`.
///
/// Limits are checked **strictly** before evaluation rather than letting
/// [`DiceExpr::eval`] cap silently: a player who types `999d6` should be told
/// the expression is too big, not handed a quietly truncated result.
///
/// # Errors
///
/// A one-line message naming what was wrong with the expression.
pub fn roll_expression(rng: &mut dyn RngCore, expr: &str) -> Result<RollOutcome, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("Usage: +roll <dice expression> (2d6, 3d6+2, 4d6kh3).".to_string());
    }
    let parsed: DiceExpr = mush_dice::parse(expr).map_err(|error| error.to_string())?;
    let limits = RollLimits::default();
    parsed
        .check_limits(&limits)
        .map_err(|error| error.to_string())?;
    let mut ctx = RollCtx::with_limits(rng, limits);
    Ok(parsed.eval(&mut ctx))
}

/// The faces of a roll as display text: `4 3 6`.
#[must_use]
pub fn dice_text(outcome: &RollOutcome) -> String {
    let faces: Vec<String> = outcome
        .dice
        .iter()
        .map(|die| {
            if die.kept {
                die.value.to_string()
            } else {
                format!("({})", die.value)
            }
        })
        .collect();
    if faces.is_empty() {
        "no dice".to_string()
    } else {
        faces.join(" ")
    }
}

/// Register `+roll` under the calling module.
///
/// # Errors
///
/// [`ModError`] if the command name collides.
pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+roll".into(),
            shape: ArgShape::Raw,
            // In a turn-based scene (a timestop), rolling is a mechanical action
            // gated to the roller's turn. Outside a scene, unchanged.
            scene_class: SceneClass::Turn,
            help: "+roll <expression>  --  Roll dice: +roll 2d6, +roll 3d6+2, \
                   +roll 4d6kh3 (keep the highest three), +roll 2d6-1. The room \
                   sees the result."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(roll_cmd),
    )
}

fn roll_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let expr = ctx.args.raw().trim().to_string();
    let outcome = {
        let state = ctx
            .state_mut::<PilotSheetState>()
            .ok_or_else(|| CmdError::msg("The pilot sheet is not loaded."))?;
        roll_expression(state.rng(), &expr).map_err(CmdError::msg)?
    };
    let name = name_of(ctx, actor);
    let dice = dice_text(&outcome);
    let result = outcome.total.to_string();
    let line = format!("{name} rolls {expr}: {dice} = {result}");
    let mut notes: Vec<String> = Vec::new();
    if outcome.capped {
        notes.push("[capped]".to_string());
    }
    let narration = roll_narration(
        &RollSide {
            name: &name,
            expr: &expr,
            dice: &dice,
            result: &result,
        },
        &notes,
    );
    match ctx.w.get(actor).and_then(|o| o.location) {
        Some(room) => {
            let actor_ref = mush_core::obj_ref(ctx.w, actor);
            ctx.out.to_room(
                room,
                Vec::new(),
                OutputEvent::Narrative {
                    kind: NarrativeKind::Emit,
                    actor: Some(actor_ref),
                    text: narration,
                },
            );
        }
        // Nowhere to show it: the roller still gets their own result.
        None => ctx.out.sys_to_actor(actor, NS, line),
    }
    Ok(())
}

fn name_of(ctx: &Ctx<'_>, who: ObjId) -> String {
    ctx.w.get(who).map_or_else(
        || who.to_string(),
        |o| mush_core::output::primary_name(&o.name).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(7)
    }

    #[test]
    fn a_plain_expression_rolls_the_dice_it_names() {
        let mut rng = rng();
        let outcome = roll_expression(&mut rng, "2d6").expect("2d6 parses");
        assert_eq!(outcome.dice.len(), 2);
        assert!(outcome.dice.iter().all(|d| (1..=6).contains(&d.value)));
        assert!((2..=12).contains(&outcome.total));
        assert!(!outcome.capped);
    }

    #[test]
    fn a_modifier_lands_in_the_total_and_not_in_the_dice() {
        let mut rng = rng();
        let outcome = roll_expression(&mut rng, "1d6+10").expect("1d6+10 parses");
        assert_eq!(outcome.dice.len(), 1);
        assert!((11..=16).contains(&outcome.total));
    }

    #[test]
    fn the_same_seed_gives_the_same_roll() {
        let a = roll_expression(&mut rng(), "5d6").expect("parses");
        let b = roll_expression(&mut rng(), "5d6").expect("parses");
        assert_eq!(a, b);
    }

    #[test]
    fn an_oversized_expression_is_refused_rather_than_capped() {
        let mut rng = rng();
        let refused = roll_expression(&mut rng, "9999d6");
        assert!(refused.is_err(), "{refused:?}");
    }

    #[test]
    fn nonsense_and_emptiness_are_refused() {
        let mut rng = rng();
        assert!(roll_expression(&mut rng, "").is_err());
        assert!(roll_expression(&mut rng, "strength+brawl").is_err());
    }

    #[test]
    fn dropped_dice_are_shown_in_parentheses() {
        let mut rng = rng();
        let outcome = roll_expression(&mut rng, "4d6kh3").expect("parses");
        let text = dice_text(&outcome);
        assert_eq!(outcome.dice.len(), 4);
        assert_eq!(text.matches('(').count(), 1, "one die dropped: {text}");
    }
}
