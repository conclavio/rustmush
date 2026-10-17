//! A separate expression-only engine. No game registration, station functions,
//! module loading, live-engine eval, source lookup or mutable memory enters it.
use super::{BulkError, invalid, model::BulkConfig};
use mush_mind::{
    budget,
    rhai::{
        self, Dynamic, Engine, Scope,
        packages::{ArithmeticPackage, LogicPackage, Package},
    },
};
use serde_json::Value;

pub(crate) const MAX_EXPRESSION: usize = 2_048;
pub(crate) const MAX_PICTURE_BYTES: usize = 128 * 1024;
/// Construct only after the bench session has authorized this exact immutable
/// unit/revision. Opponent memory/source must never be placed in these values.
pub(crate) struct EvalPicture {
    pub view: Value,
    pub card: Value,
    pub memory: Value,
}
pub(crate) struct EvalAnswer {
    pub text: String,
    pub operations: u64,
    pub fees: u64,
}
fn dynamic(value: &Value, depth: usize, nodes: &mut usize) -> Result<Dynamic, BulkError> {
    *nodes += 1;
    if depth > 12 || *nodes > 4096 {
        return Err(invalid("evaluation picture dimensions"));
    }
    Ok(match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(v) => (*v).into(),
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| invalid("evaluation supports integer facts only"))?
            .into(),
        Value::String(s) => {
            if s.len() > 8192 {
                return Err(invalid("evaluation string bound"));
            }
            s.clone().into()
        }
        Value::Array(rows) => {
            if rows.len() > 256 {
                return Err(invalid("evaluation array bound"));
            }
            rows.iter()
                .map(|v| dynamic(v, depth + 1, nodes))
                .collect::<Result<rhai::Array, _>>()?
                .into()
        }
        Value::Object(rows) => {
            if rows.len() > 256 {
                return Err(invalid("evaluation map bound"));
            }
            let mut map = rhai::Map::new();
            for (k, v) in rows {
                if k.len() > 128 {
                    return Err(invalid("evaluation key bound"));
                }
                map.insert(k.clone().into(), dynamic(v, depth + 1, nodes)?);
            }
            map.into()
        }
    })
}
pub(crate) fn evaluate(
    cfg: &BulkConfig,
    expression: &str,
    picture: &EvalPicture,
) -> Result<EvalAnswer, BulkError> {
    cfg.validate()?;
    if expression.is_empty() || expression.len() > MAX_EXPRESSION {
        return Err(invalid("evaluation expression bound"));
    }
    let bytes = serde_json::to_vec(&(&picture.view, &picture.card, &picture.memory))
        .map_err(|e| invalid(e.to_string()))?;
    if bytes.len() > MAX_PICTURE_BYTES {
        return Err(invalid("evaluation picture byte bound"));
    }
    let mut engine = Engine::new_raw();
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
    for symbol in [
        "eval", "import", "export", "Fn", "call", "curry", "this", "fn", "let", "const", "while",
        "loop", "for", "print", "debug",
    ] {
        engine.disable_symbol(symbol);
    }
    engine
        .set_max_modules(0)
        .set_max_call_levels(2)
        .set_max_functions(0)
        .set_max_variables(3)
        .set_max_expr_depths(24, 12)
        .set_max_operations(cfg.eval_ops)
        .set_max_string_size(8192)
        .set_max_array_size(256)
        .set_max_map_size(256)
        .set_strict_variables(true)
        .set_fail_on_invalid_map_property(true)
        .set_allow_anonymous_fn(false)
        .set_allow_statement_expression(false)
        .set_allow_loop_expressions(false)
        .set_allow_looping(false)
        .set_allow_if_expression(false)
        .set_allow_switch_expression(false);
    engine.on_progress(budget::check);
    let mut scope = Scope::new();
    let mut nodes = 0;
    scope.push_constant("pic", dynamic(&picture.view, 0, &mut nodes)?);
    scope.push_constant("card", dynamic(&picture.card, 0, &mut nodes)?);
    scope.push_constant("mem", dynamic(&picture.memory, 0, &mut nodes)?);
    // Simple optimization is allowed to fold constant arithmetic. It calls no
    // custom native function at compile time; this engine has no such helpers.
    let ast = engine
        .compile_expression_with_scope(&scope, expression)
        .map_err(|e| invalid(format!("expression: {e}")))?;
    budget::begin(cfg.eval_ops);
    budget::charge(20);
    let value = engine
        .eval_ast_with_scope::<Dynamic>(&mut scope, &ast)
        .map_err(|_| invalid("evaluation refused or its 500-op allowance was exhausted"))?;
    if budget::tripped() {
        return Err(invalid("evaluation allowance exhausted"));
    }
    let (operations, fees) = budget::spent();
    // Only scalar results are printable. This prevents formatting a permitted
    // nested picture into an unbounded intermediate string.
    let text = if value.is_unit() {
        "()".into()
    } else if value.is::<rhai::INT>() {
        value.cast::<rhai::INT>().to_string()
    } else if value.is::<bool>() {
        value.cast::<bool>().to_string()
    } else if value.is::<rhai::ImmutableString>() {
        value.cast::<rhai::ImmutableString>().to_string()
    } else {
        return Err(invalid(
            "evaluation result must be an integer, bool or string",
        ));
    };
    if text.len() > cfg.eval_result_bytes || text.chars().any(char::is_control) {
        return Err(invalid("evaluation result bound"));
    }
    Ok(EvalAnswer {
        text: text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
        operations,
        fees,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn picture() -> EvalPicture {
        EvalPicture {
            view: serde_json::json!({"hp":7}),
            card: serde_json::json!({"threshold":5}),
            memory: serde_json::json!({"visits":3}),
        }
    }
    #[test]
    fn reads_frozen_facts_under_separate_budget() {
        let answer = evaluate(
            &BulkConfig::default(),
            "pic.hp > card.threshold && mem.visits == 3",
            &picture(),
        )
        .expect("read");
        assert_eq!(answer.text, "true");
        assert!(answer.operations + answer.fees <= 500);
        assert_eq!(answer.fees, 20)
    }
    #[test]
    fn mutation_and_live_capabilities_refuse() {
        for text in [
            "mem.visits = 0",
            "mem.clear()",
            "{ mem.visits = 0; 1 }",
            "eval(\"1\")",
            "helm(9)",
            "while true { }",
            "view",
            "sleep(1)",
        ] {
            assert!(
                evaluate(&BulkConfig::default(), text, &picture()).is_err(),
                "{text}"
            )
        }
    }
    #[test]
    fn configured_low_budget_and_output_bound() {
        let mut cfg = BulkConfig {
            eval_ops: 1,
            ..Default::default()
        };
        assert!(evaluate(&cfg, "pic.hp", &picture()).is_err());
        cfg.eval_ops = 500;
        cfg.eval_result_bytes = 64;
        let mut p = picture();
        p.memory = serde_json::json!({"text":"x".repeat(65)});
        assert!(evaluate(&cfg, "mem.text", &p).is_err())
    }
}
