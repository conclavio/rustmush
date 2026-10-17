//! Published combat helpers on the ordinary metered Rhai engine.
//!
//! The reference Standard implementation is compiled as private module functions.
//! Six public script wrappers reuse that body or its kernels; `rules` is the
//! metered frozen-profile sampler. All execution stays in the caller's budget,
//! context, memory checks, station authority and intent limit. No native full-think
//! escape or second budget exists. Imported modules remain disabled.

use super::Registrar;
use mush_mind::rhai::{Engine, FuncRegistration, Module, Scope};

/// The entire public helper surface, also carried by the API inventory.
/// `stock::rules() -> bool` samples once/charges once and answers whether the
/// frozen authored program is nonempty; repeated calls reuse that same sample.
pub(crate) const NAMES: [&str; 7] = [
    "stock::think",
    "stock::rules",
    "stock::fire_discipline",
    "stock::best_hostile",
    "stock::hold_band",
    "stock::steer_to",
    "stock::hunt",
];

/// These wrappers have no implicit memory: methods which need it return the
/// map passed by the author, who assigns the result back to `this`.
/// Additional fees are the called kernels' declared fees; all wrapper operations
/// are charged by the same on_progress hook as authored code.
const WRAPPERS: &str = r#"
fn think(view, card, memory) {
    stock::rules();
    memory.stock_body_private(view, card);
    memory
}
fn fire_discipline(view, card, ceiling_cm) {
    arming_for(view, card, ceiling_cm);
}
fn best_hostile(view, card) {
    let w = weights(card);
    best_hostile(view, w[0], w[1], w[2], health_permille(view), (), (), (), ())
}
fn steer_to(view, goal) {
    if view.wreck || view.immobile { return; }
    if goal == () { hold(); } else { goto (goal); }
}
fn hold_band(view, card, target) {
    if view.wreck || view.immobile { return; }
    let row = contact(view, target);
    if row == () { return; }
    let band = ranges(view, card.band_permille, true);
    let range = range_cm(row);
    if band.limit_cm == 0 || range > band.release_cm {
        let want = stand_off_want(band.engage_cm, max(cell_cm(), 1), range);
        let goal = stand_off_cell(view, target, want);
        if goal != () { goto (goal); }
    } else {
        hold();
        helm(row.bearing_deg * 1000);
    }
}
fn hunt(view, card, memory) {
    if !("state" in memory) { memory.fresh(); }
    if view.wreck || view.immobile { return memory; }
    let words = memory.read_notices(view);
    let p = #{here: view.cell, reached: words[0]};
    let goal = memory.hunt_goal(view, card, p);
    if goal != () {
        memory.goal = goal;
        goto (goal);
    }
    memory
}
"#;

/// Build from trusted repository text only. Restoring the caller's parse cap is
/// unconditional; no authored source is compiled while it is raised. There are
/// 31 private reference functions plus six public wrappers, under the fixed 40.
fn build(engine: &mut Engine) -> Result<Module, String> {
    // Rhai strict parsing resolves the qualified sampler name against this
    // namespace; runtime lookup uses the final module installed below.
    engine.register_static_module("stock", Module::new().into());
    const ARMING: &str = "fn arming_for(view, card, ceiling) {";
    if super::STANDARD_RHAI.matches(ARMING).count() != 1 {
        return Err("trusted Standard arming helper changed shape".to_owned());
    }
    // A nonempty frozen authored program owns automatic firing policy. Only
    // this trusted helper copy changes; Standard's reference source/digest stays
    // intact, and explicit author station calls still meet normal admission.
    let private = super::STANDARD_RHAI
        .replace(
            ARMING,
            "fn arming_for(view, card, ceiling) { if stock::rules() { return; }",
        )
        .replace("fn think(", "fn stock_body_private(")
        .replace("\nfn ", "\nprivate fn ");
    let source = format!("{private}\n{WRAPPERS}");
    let cap = engine.max_functions();
    engine.set_max_functions(40);
    let parsed = engine.compile(&source);
    engine.set_max_functions(cap);
    let mut ast = parsed.map_err(|error| error.to_string())?;
    ast.clear_statements();
    // Evaluating this function-only AST executes no operations and does not
    // reset or consume a caller thread's current budget or trace.
    let mut module =
        Module::eval_ast_as_new(Scope::new(), &ast, engine).map_err(|error| error.to_string())?;
    // This sampler reads current context and writes its memoized sidecar; it is
    // neither pure nor stable across thinks and must never be constant-folded.
    FuncRegistration::new("rules")
        .in_internal_namespace()
        .with_purity(false)
        .with_volatility(true)
        .set_into_module(&mut module, super::rule_samples::rules);
    Ok(module)
}

pub(crate) fn register(api: &mut Registrar<'_>) {
    match build(api.engine) {
        Ok(module) => {
            api.engine.register_static_module("stock", module.into());
        }
        Err(error) => {
            // Internal source regression fails closed: the namespace has no
            // executable helpers, so authored callers cannot silently succeed.
            tracing::error!(%error, "cannot register metered stock helpers");
        }
    }
    api.names.extend(NAMES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minds::fixture;
    use mush_mind::{Fault, budget};

    #[test]
    fn module_construction_preserves_budget_and_authored_function_limit() {
        let mut engine = fixture::scratch();
        engine.set_max_functions(32);
        budget::begin(300);
        budget::charge(250);
        let before = budget::spent();
        build(&mut engine).expect("trusted stock module compiles");
        assert_eq!(engine.max_functions(), 32);
        assert_eq!(engine.max_expr_depth(), 48);
        assert_eq!(engine.max_function_expr_depth(), 24);
        assert_eq!(budget::spent(), before);
        let many = (0..33)
            .map(|n| format!("fn f{n}() {{}}\n"))
            .collect::<String>();
        assert!(engine.compile(&many).is_err());
    }

    #[test]
    fn stock_helpers_are_private_except_the_published_surface() {
        for expr in [
            "stock::stock_body_private(v,c)",
            "stock::fresh()",
            "stock::arming_for(v,c,0)",
        ] {
            assert!(fixture::eval(expr).is_err(), "{expr}");
        }
    }

    #[test]
    fn authored_program_owns_automatic_arming_but_empty_program_keeps_standard_policy() {
        use crate::arena::{
            conditions::{Arming, Rule, RulePredicate},
            core::{ClassTable, CoreClass},
            intents::IntentKind,
            profile_rules::BoundProgram,
            schema::AuthoredRule,
            state::Channel,
        };
        let source = "fn think(view,card) { stock::fire_discipline(view,card,()); stock::fire_discipline(view,card,()); }";
        let native = fixture::think(source);
        assert!(native.result.is_ok());
        assert!(
            native
                .ctx
                .intents
                .iter()
                .any(|intent| matches!(intent, IntentKind::Condition(Arming::Disarm { .. })))
        );
        let mut ctx = fixture::context_on(fixture::view());
        ctx.program = BoundProgram::bind(
            &[AuthoredRule {
                rule: Rule::hold_rule("cannon".into(), vec![RulePredicate::Lock { present: true }]),
                think: Vec::new(),
            }],
            &["cannon".into()],
            &Channel::ALL,
            &ClassTable::STOCK.row(CoreClass::I),
            100,
        )
        .unwrap();
        let authored = fixture::think_with(source, ctx, fixture::card(), &[]);
        assert!(authored.result.is_ok());
        assert!(
            authored.ctx.intents.is_empty(),
            "helper must not erase or re-arm sealed authored rows"
        );
        assert!(
            authored.ctx.rule_sample.is_some(),
            "sampling remains present and memoized"
        );
    }

    #[test]
    fn repeated_stock_calls_consume_the_same_think_budget() {
        let ran =
            fixture::think("fn think(view, card) { loop { stock::best_hostile(view, card); } }");
        assert!(matches!(ran.result, Err(Fault::Ops)));
        assert!(ran.ctx.intents.is_empty());
    }
}
