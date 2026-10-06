//! **The station functions** (17b cut M2.3; doc 17 §4 "The outputs"; D-UM-9,
//! D-UM-10, D-UM-11, D-UM-28, D-UM-47; 17b §1's departure (2)).
//!
//! A think acts by calling these, and **exactly** doc 17 §4's list, grouped
//! by the arena channel each needs (D-UM-47: a mind's functions are its hull's
//! stations' verbs; D-UM-10: each is a verb a pilot at that station can type):
//!
//! * **Helm**: `helm(mdeg)` → `Heading`, `speed(cms)` → `Speed`, `startup()`
//!   → `Startup`, `goto([x, y])` → `Goal(Cell)`, `goto("spot")` → `Goal(Cell)`
//!   at the key spot the pilot's `DRIVE <spot>` names
//!   (`crate::match_::cockpit::resolve_spot`, the one resolver), `hold()` →
//!   `Goal(Hold)`, `goto_off()` → `Goal(Off)`;
//! * **Sensors**: `lock(contact)` → `Lock` on a row the view holds,
//!   `emcon(on)` → `Emcon`;
//! * **Fire**: `fire(weapon, slot)` → `Fire { slot: Some(n) }` (the arena
//!   numbers slots; a mind always cites one, 17b §1's departure (2)),
//!   `arm(weapon, target, min_permille, aspects, keeps, min_range_m,
//!   max_range_m)` → `Condition(Arm(FireCondition))` (the seven terms of a
//!   condition — `"lock"`, `"best"` or a contact INT for the target),
//!   `disarm(weapon | ())` → `Condition(Disarm)`;
//! * **any channel**: `trace(text)` → the think's trace (doc 17 §14), never
//!   an intent.
//!
//! Each pushes onto the think's buffer (`ThinkContext::intents`, D-UM-9: at
//! most `MIND_INTENTS_MAX`, a seventeenth push refused `intents` and the
//! context marked `overflowed`) in call order — nothing touches the arena
//! until the landing, and a faulted think lands nothing. There is **no**
//! `order`, `release`, `place`, `board` or exit-the-hull function (D-UM-11: a
//! mind never orders, and the registered-names pin holds it).
//!
//! # The order of checks
//!
//! An argument's TYPE first: every station argument is taken as `Dynamic` and
//! read as the INT, bool, word or cell it must be, within its intent field's
//! type, so a FLOAT or an out-of-range figure is a `type` refusal and never a
//! rhai lookup error a script could catch (D-UM-28: an intent argument is INT
//! only). Then, on the fire station, **the ungranted channel FIRST**: a hull
//! with no weapon rows has no `Fire` channel, and `fire`, `arm` and `disarm`
//! there push their intent and write
//! `fire: this hull has no weapons station (refused at landing)` to the trace
//! and answer — no refusal, no fault: the landing refuses the intent
//! `Channel` through the one door (`intents.rs`' mind arm, D-UM-8), and the
//! refusal reaches the mind's notice buffer (doc 17 §22's italic line: *a
//! refusal in the trace, not a fault*). On an unarmed hull every weapon id is
//! unheld, so the check must come before the next one. Then the unknown ids —
//! a weapon the hull does not carry, a slot outside `1..=slots()`, a contact
//! the view does not hold, a spot the map does not name — refused `unknown`.
//! Then the push.
//!
//! Every refusal sets the context's word and is raised through
//! `mush_mind::system_error`: uncatchable (D-UM-31).

use mush_mind::rhai::{Dynamic, EvalAltResult, INT, ImmutableString};
use smol_str::SmolStr;

use super::Registrar;
use super::api::{bounded, cell_arg, contact_of, int_arg, word_arg};
use super::context::{
    REFUSED_INTENTS, REFUSED_TYPE, REFUSED_UNKNOWN, ThinkContext, refuse, refused, with,
};
use crate::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, STOCK_CONDITION_SLOTS, SlotTarget,
};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::MIND_INTENTS_MAX;
use crate::arena::state::NO_SUCH_CONTACT;
use crate::arena::stock_mind;

/// **The trace line an ungranted fire station writes** (the file head).
pub const UNGRANTED_FIRE: &str = "fire: this hull has no weapons station (refused at landing)";

/// The most a slot's odds term can ask: a certain hit, per mille.
const PERMILLE_MAX: INT = 1_000;

/// The refusal a seventeenth push answers.
const TOO_MANY: &str = "a think emits at most 16 intents";

/// The refusal a weapon id the hull does not carry answers.
const NO_SUCH_WEAPON: &str = "No such weapon on this hull.";

/// The refusal a slot outside the hull's slots answers.
const NO_SUCH_SLOT: &str = "No such slot on this hull.";

/// The refusal a spot the map does not name answers.
const NO_SUCH_SPOT: &str = "No such spot on this map.";

/// **Push one intent** onto the running think's buffer, or refuse the
/// seventeenth `intents` (D-UM-9).
fn push_on(ctx: &mut ThinkContext, kind: IntentKind) -> Result<(), Box<EvalAltResult>> {
    if ctx.intents.len() >= MIND_INTENTS_MAX {
        ctx.overflowed = true;
        return Err(refuse(ctx, REFUSED_INTENTS, TOO_MANY));
    }
    ctx.intents.push(kind);
    Ok(())
}

/// [`push_on`] on the running think.
fn push(kind: IntentKind) -> Result<(), Box<EvalAltResult>> {
    with(|ctx| push_on(ctx, kind))?
}

/// **A fire-station intent**: on a hull with no weapon rows, pushed with
/// [`UNGRANTED_FIRE`] in the trace and no refusal (checked FIRST — the file
/// head); otherwise `check` refuses an unknown id, then the push.
fn fire_station(
    kind: IntentKind,
    check: impl FnOnce(&mut ThinkContext) -> Result<(), Box<EvalAltResult>>,
) -> Result<(), Box<EvalAltResult>> {
    with(|ctx| {
        if ctx.view.weapons.is_empty() {
            push_on(ctx, kind)?;
            mush_mind::trace::push(UNGRANTED_FIRE);
            return Ok(());
        }
        check(ctx)?;
        push_on(ctx, kind)
    })?
}

/// Whether the hull carries `weapon`, else an `unknown` refusal.
fn carried(ctx: &mut ThinkContext, weapon: &str) -> Result<(), Box<EvalAltResult>> {
    if ctx.view.weapons.iter().any(|row| row.id.as_str() == weapon) {
        Ok(())
    } else {
        Err(refuse(ctx, REFUSED_UNKNOWN, NO_SUCH_WEAPON))
    }
}

/// **`helm(mdeg)`** → `Heading { mdeg }`.
fn helm(mdeg: Dynamic) -> Result<(), Box<EvalAltResult>> {
    const TEXT: &str = "helm takes whole millidegrees from 0";
    let mdeg: u32 = bounded(int_arg(&mdeg, TEXT)?, TEXT)?;
    push(IntentKind::Heading { mdeg })
}

/// **`speed(cms)`** → `Speed { cms }`.
fn speed(cms: Dynamic) -> Result<(), Box<EvalAltResult>> {
    const TEXT: &str = "speed takes whole centimetres a second";
    let cms: i32 = bounded(int_arg(&cms, TEXT)?, TEXT)?;
    push(IntentKind::Speed { cms })
}

/// What `goto` takes.
const GOTO_TEXT: &str = "goto takes a cell [x, y] or a spot's name";

/// **`goto([x, y])` or `goto("spot")`** → `Goal(Cell)` (registered as custom
/// syntax: see [`register`]).
fn goto(target: Dynamic) -> Result<(), Box<EvalAltResult>> {
    if target.is_string() {
        let name = word_arg(&target, GOTO_TEXT)?;
        return with(|ctx| {
            let Some((_, spot)) =
                crate::match_::cockpit::resolve_spot(&ctx.map.key_spots, name.as_str())
            else {
                return Err(refuse(ctx, REFUSED_UNKNOWN, NO_SUCH_SPOT));
            };
            let (x, y) = (spot.x, spot.y);
            push_on(ctx, IntentKind::Goal(MotorGoal::Cell { x, y }))
        })?;
    }
    let (x, y) = cell_arg(&target, GOTO_TEXT)?;
    push(IntentKind::Goal(MotorGoal::Cell { x, y }))
}

/// **`lock(contact)`** → `Lock { contact }`, on a row the view holds.
fn lock(contact: Dynamic) -> Result<(), Box<EvalAltResult>> {
    let contact = contact_of(int_arg(&contact, "lock takes a contact's id")?);
    with(|ctx| {
        if stock_mind::contact(&ctx.view, contact).is_none() {
            return Err(refuse(ctx, REFUSED_UNKNOWN, NO_SUCH_CONTACT));
        }
        push_on(ctx, IntentKind::Lock { contact })
    })?
}

/// **`emcon(on)`** → `Emcon { on }`.
fn emcon(on: Dynamic) -> Result<(), Box<EvalAltResult>> {
    let on = on
        .as_bool()
        .map_err(|_| refused(REFUSED_TYPE, "emcon takes true or false"))?;
    push(IntentKind::Emcon { on })
}

/// **`fire(weapon, slot)`** → `Fire { weapon, slot: Some(slot) }`.
fn fire(weapon: Dynamic, slot: Dynamic) -> Result<(), Box<EvalAltResult>> {
    const TEXT: &str = "fire takes a weapon's id and a slot number";
    let weapon = word_arg(&weapon, TEXT)?;
    let slot: u8 = bounded(int_arg(&slot, TEXT)?, TEXT)?;
    let kind = IntentKind::Fire {
        weapon: SmolStr::new(weapon.as_str()),
        slot: Some(slot),
    };
    fire_station(kind, |ctx| {
        carried(ctx, weapon.as_str())?;
        if slot == 0 || usize::from(slot) > STOCK_CONDITION_SLOTS {
            return Err(refuse(ctx, REFUSED_UNKNOWN, NO_SUCH_SLOT));
        }
        Ok(())
    })
}

/// A slot's target: `"lock"`, `"best"` or a contact INT.
fn target_arg(target: &Dynamic, text: &str) -> Result<SlotTarget, Box<EvalAltResult>> {
    if let Ok(id) = target.as_int() {
        return Ok(SlotTarget::Contact { id: contact_of(id) });
    }
    match word_arg(target, text)?.as_str() {
        "lock" => Ok(SlotTarget::Lock),
        "best" => Ok(SlotTarget::BestHostile),
        _ => Err(refused(REFUSED_TYPE, text)),
    }
}

/// **`arm(weapon, target, min_permille, aspects, keeps, min_range_m,
/// max_range_m)`** → `Condition(Arm(FireCondition))`: the seven terms of a
/// slot (`keeps` is `min_rounds_left`; `max_range_m` 4294967295 is no upper
/// bound, `min_range_m` 0 no lower one, as the slot holds them).
///
/// Two terms are bounded tighter than their field's type, to what a slot
/// can mean (D-UM-10: a mind arms no slot a pilot's `FIRE … WHEN` could not
/// write, and D-UM-4: the keyframe prints the mask as its facet words, so a
/// bit past the four facets would read back as a mask the keyframe does not
/// show): `min_permille` at most 1000 (a chance, per mille), and `aspects` a
/// non-empty mask of the four facet bits, `1..=ASPECTS_ANY` — a mask of none
/// is a slot that never fires. Past either, `type`.
fn arm(
    weapon: Dynamic,
    target: Dynamic,
    min_permille: Dynamic,
    aspects: Dynamic,
    keeps: Dynamic,
    min_range_m: Dynamic,
    max_range_m: Dynamic,
) -> Result<(), Box<EvalAltResult>> {
    const TEXT: &str =
        "arm takes a weapon, lock, best or a contact, then five whole numbers in range";
    let weapon = word_arg(&weapon, TEXT)?;
    let min_permille = int_arg(&min_permille, TEXT)?;
    if min_permille > PERMILLE_MAX {
        return Err(refused(REFUSED_TYPE, TEXT));
    }
    let aspects = int_arg(&aspects, TEXT)?;
    if !(1..=INT::from(ASPECTS_ANY)).contains(&aspects) {
        return Err(refused(REFUSED_TYPE, TEXT));
    }
    let cond = FireCondition {
        weapon: SmolStr::new(weapon.as_str()),
        target: target_arg(&target, TEXT)?,
        min_permille: bounded(min_permille, TEXT)?,
        aspects: bounded(aspects, TEXT)?,
        min_rounds_left: bounded(int_arg(&keeps, TEXT)?, TEXT)?,
        min_range_m: bounded(int_arg(&min_range_m, TEXT)?, TEXT)?,
        max_range_m: bounded(int_arg(&max_range_m, TEXT)?, TEXT)?,
    };
    fire_station(IntentKind::Condition(Arming::Arm(cond)), |ctx| {
        carried(ctx, weapon.as_str())
    })
}

/// **`disarm(weapon)` or `disarm(())`** → `Condition(Disarm { weapon })`:
/// one weapon's slot, or every slot.
fn disarm(weapon: Dynamic) -> Result<(), Box<EvalAltResult>> {
    let weapon: Option<ImmutableString> = if weapon.is_unit() {
        None
    } else {
        Some(word_arg(&weapon, "disarm takes a weapon's id or ()")?)
    };
    let kind = IntentKind::Condition(Arming::Disarm {
        weapon: weapon.as_ref().map(|id| SmolStr::new(id.as_str())),
    });
    fire_station(kind, |ctx| match &weapon {
        Some(id) => carried(ctx, id.as_str()),
        None => Ok(()),
    })
}

/// **`trace(text)`** → one line of the think's trace (doc 17 §14), at most
/// eight of 120 characters; never an intent, never journalled.
fn trace(text: Dynamic) -> Result<(), Box<EvalAltResult>> {
    let text = word_arg(&text, "trace takes a string")?;
    mush_mind::trace::push(text.as_str());
    Ok(())
}

/// **Doc 17 §4's functions, and nothing more** (the file head).
pub(crate) fn register(api: &mut Registrar<'_>) {
    function!(api, "helm", helm);
    function!(api, "speed", speed);
    function!(api, "startup", || { push(IntentKind::Startup) });
    // `goto` is one of rhai 1.26's reserved keywords, and a reserved keyword
    // may not name a function call (its tokenizer's table marks `goto` not
    // callable). So `goto(cell | spot)` — doc 17 §4's name, the script's
    // spelling — is registered as custom syntax, `goto ( $expr$ )`: the
    // argument is evaluated as any expression is (its operations counted)
    // and handed to [`goto`]. Recorded only when rhai accepts it, so a
    // refusal would show in the registered-names pin.
    if api
        .engine
        .register_custom_syntax(["goto", "(", "$expr$", ")"], false, |context, inputs| {
            let target = inputs
                .first()
                .ok_or_else(|| refused(REFUSED_TYPE, GOTO_TEXT))?
                .eval_with_context(context)?;
            goto(target)?;
            Ok(Dynamic::UNIT)
        })
        .is_ok()
    {
        api.names.push("goto");
    }
    function!(api, "hold", || { push(IntentKind::Goal(MotorGoal::Hold)) });
    function!(api, "goto_off", || {
        push(IntentKind::Goal(MotorGoal::Off))
    });
    function!(api, "lock", lock);
    function!(api, "emcon", emcon);
    function!(api, "fire", fire);
    function!(api, "arm", arm);
    function!(api, "disarm", disarm);
    function!(api, "trace", trace);
}

#[cfg(test)]
mod tests {
    use mush_mind::Fault;

    use super::super::api::contact_int;
    use super::super::fixture::{FRIEND, card, context_on, think, think_with, unarmed_view};
    use super::*;
    use crate::arena::card::line;
    use crate::arena::stock_mind::arming_for;
    use crate::config::ReflexConfig;

    /// 17b §2's M2.3 test 2: **every station function maps its arguments to
    /// the intent T1 emits**, in call order; the refusals by word; the
    /// ungranted fire station pushing its intent with its trace line and NO
    /// refusal.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_station_function_maps_its_arguments_to_the_intent_t1_emits() {
        let friend = contact_int(FRIEND);
        let source = format!(
            "fn think(v, c) {{
                startup();
                arm(\"cannon\", \"best\", 100, 15, 0, 0, 420);
                goto([6, 4]);
                goto(\"the middle\");
                hold();
                goto_off();
                helm(270000);
                speed(-150);
                lock({friend});
                emcon(true);
                fire(\"cannon\", 1);
                disarm(());
                disarm(\"coax\");
                arm(\"coax\", \"lock\", 550, 6, 20, 50, 4294967295);
                arm(\"coax\", {friend}, 300, 1, 0, 0, 100);
                trace(\"fifteen pushes\");
            }}"
        );
        let ran = think(&source);
        assert!(ran.result.is_ok(), "{:?}", ran.result);
        assert_eq!(ran.ctx.refused, None);
        assert!(!ran.ctx.overflowed);
        // What T1's `arming_for` builds for the cannon on Line at a 420 m
        // ceiling (best hostile, 10%, every aspect, no reserve, no floor),
        // built by T1's own function on the full view with its slots clear.
        let mut clear = super::super::fixture::view();
        for row in &mut clear.weapons {
            row.armed = None;
        }
        let built = arming_for(&clear, &line(&ReflexConfig::default()), Some(42_000));
        let Some(IntentKind::Condition(Arming::Arm(t1))) = built.first().cloned() else {
            panic!("T1 arms the cannon: {built:?}");
        };
        assert_eq!(t1.weapon, "cannon");
        assert_eq!(
            ran.ctx.intents,
            vec![
                IntentKind::Startup,
                IntentKind::Condition(Arming::Arm(t1)),
                IntentKind::Goal(MotorGoal::Cell { x: 6, y: 4 }),
                IntentKind::Goal(MotorGoal::Cell { x: 4, y: 4 }),
                IntentKind::Goal(MotorGoal::Hold),
                IntentKind::Goal(MotorGoal::Off),
                IntentKind::Heading { mdeg: 270_000 },
                IntentKind::Speed { cms: -150 },
                IntentKind::Lock { contact: FRIEND },
                IntentKind::Emcon { on: true },
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: Some(1),
                },
                IntentKind::Condition(Arming::Disarm { weapon: None }),
                IntentKind::Condition(Arming::Disarm {
                    weapon: Some(SmolStr::new("coax")),
                }),
                IntentKind::Condition(Arming::Arm(FireCondition {
                    weapon: SmolStr::new("coax"),
                    target: SlotTarget::Lock,
                    min_permille: 550,
                    aspects: 6,
                    min_rounds_left: 20,
                    min_range_m: 50,
                    max_range_m: u32::MAX,
                })),
                IntentKind::Condition(Arming::Arm(FireCondition {
                    weapon: SmolStr::new("coax"),
                    target: SlotTarget::Contact { id: FRIEND },
                    min_permille: 300,
                    aspects: 1,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: 100,
                })),
            ]
        );
        assert_eq!(ran.trace, vec!["fifteen pushes".to_owned()]);

        // A sixteenth push lands; a seventeenth is refused `intents`.
        let sixteen = "fn think(v, c) { for i in 0..16 { helm(i); } }";
        let ran = think(sixteen);
        assert!(ran.result.is_ok());
        assert_eq!(ran.ctx.intents.len(), MIND_INTENTS_MAX);
        let seventeen = "fn think(v, c) { for i in 0..17 { helm(i); } }";
        let ran = think(seventeen);
        assert!(matches!(ran.result, Err(Fault::Type(_))));
        assert!(ran.ctx.overflowed);
        assert_eq!(ran.ctx.refused, Some(REFUSED_INTENTS));
        assert_eq!(ran.ctx.intents.len(), MIND_INTENTS_MAX);

        // Unknown ids.
        let unknown = [
            "lock(999)",
            "fire(\"cannon\", 3)",
            "fire(\"cannon\", 0)",
            "fire(\"laser\", 1)",
            "goto(\"nowhere\")",
            "arm(\"laser\", \"best\", 100, 15, 0, 0, 420)",
            "disarm(\"laser\")",
        ];
        assert_eq!(STOCK_CONDITION_SLOTS, 2);
        for call in unknown {
            let ran = think(&format!("fn think(v, c) {{ {call}; }}"));
            assert!(matches!(ran.result, Err(Fault::Type(_))), "{call}");
            assert_eq!(ran.ctx.refused, Some(REFUSED_UNKNOWN), "{call}");
            assert!(ran.ctx.intents.is_empty(), "{call}");
        }

        // A wrong type, a FLOAT among them, and a figure past its type.
        let typed = [
            "helm(1.5)",
            "helm(-1)",
            "helm(4294967296)",
            "speed(2147483648)",
            "speed(\"fast\")",
            "goto([1, -1])",
            "goto([1, 2, 3])",
            "goto([1.0, 2])",
            "goto(7)",
            "lock(\"BB\")",
            "lock(1.0)",
            "emcon(1)",
            "fire(\"cannon\", 1.0)",
            "fire(\"cannon\", 256)",
            "fire(1, 1)",
            "arm(\"cannon\", \"nearest\", 100, 15, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 70000, 15, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 256, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 255, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 16, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 0, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 1001, 15, 0, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 15, -1, 0, 420)",
            "arm(\"cannon\", \"best\", 100, 15, 0, 0, 4294967296)",
            "arm(\"cannon\", 1.5, 100, 15, 0, 0, 420)",
            "disarm(1)",
            "trace(5)",
        ];
        for call in typed {
            let ran = think(&format!("fn think(v, c) {{ {call}; }}"));
            assert!(matches!(ran.result, Err(Fault::Type(_))), "{call}");
            assert_eq!(ran.ctx.refused, Some(REFUSED_TYPE), "{call}");
            assert!(ran.ctx.intents.is_empty(), "{call}");
        }

        // The ungranted fire station: pushed, traced, not refused, not a
        // fault — checked before the unknown weapon, which every id is here.
        let ran = think_with(
            "fn think(v, c) { fire(\"cannon\", 1); arm(\"x\", \"best\", 100, 15, 0, 0, 420); disarm(()); helm(0); }",
            context_on(unarmed_view()),
            card(),
            &[],
        );
        assert!(ran.result.is_ok(), "{:?}", ran.result);
        assert_eq!(ran.ctx.refused, None);
        assert_eq!(ran.ctx.intents.len(), 4);
        assert_eq!(
            ran.ctx.intents.first(),
            Some(&IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: Some(1),
            })
        );
        assert_eq!(ran.trace, vec![UNGRANTED_FIRE.to_owned(); 3]);
        // The brief's sentence, literally.
        assert_eq!(
            UNGRANTED_FIRE,
            "fire: this hull has no weapons station (refused at landing)"
        );
        // A type refusal still comes first there.
        let ran = think_with(
            "fn think(v, c) { fire(\"cannon\", 1.5); }",
            context_on(unarmed_view()),
            card(),
            &[],
        );
        assert_eq!(ran.ctx.refused, Some(REFUSED_TYPE));
        assert!(ran.ctx.intents.is_empty());
    }

    /// 17b §2's M2.3 test 3: **every refusal is uncatchable** — a `try`
    /// around a refused `lock` does not catch it: the think ends a `type`
    /// fault (no memory), the context says `unknown`, and the `catch` body
    /// never ran.
    #[test]
    fn every_refusal_is_uncatchable() {
        let ran = think(
            "fn think(v, c) { try { lock(999); } catch { this.caught = true; } this.after = true; }",
        );
        assert!(
            matches!(&ran.result, Err(Fault::Type(text)) if text.contains("No such contact.")),
            "{:?}",
            ran.result
        );
        assert_eq!(ran.ctx.refused, Some(REFUSED_UNKNOWN));
        assert!(ran.ctx.intents.is_empty());
        // Through a helper function too, and for every word.
        for (call, word) in [
            ("lock(999)", REFUSED_UNKNOWN),
            ("helm(1.5)", REFUSED_TYPE),
            ("for i in 0..17 { helm(i); }", REFUSED_INTENTS),
            (
                "line_to(v, v.lock); line_to(v, v.lock); line_to(v, v.lock);",
                super::super::context::REFUSED_LIMIT,
            ),
        ] {
            let source = format!(
                "fn act(v) {{ {call} }} fn think(v, c) {{ try {{ act(v); }} catch (e) {{ this.caught = e; }} }}"
            );
            let ran = think(&source);
            assert!(
                matches!(ran.result, Err(Fault::Type(_))),
                "{call}: {:?}",
                ran.result
            );
            assert_eq!(ran.ctx.refused, Some(word), "{call}");
        }
    }
}
