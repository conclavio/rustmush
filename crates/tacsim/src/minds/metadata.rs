//! Optional author metadata, read without executing source top-level statements.
//!
//! The revision's external envelope stays authoritative. Metadata is a checked
//! declaration, never a licence or authority grant. One isolated, disposable
//! engine can evaluate `card()` using only bounded pure data operations.

use mush_mind::rhai::{
    Array, Dynamic, Engine, INT, ImmutableString, Map, Module, ParseErrorType, Scope,
    module_resolvers::DummyModuleResolver,
    packages::{ArithmeticPackage, BasicArrayPackage, BasicMapPackage, LogicPackage, Package},
};
use mush_roll::{Envelope, RollError};
use serde_json::Value;

const MAX_METADATA_OPS: u64 = 2_000;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_METADATA_NODES: usize = 4_096;

/// Validate optional `const MIND` and `fn card()` against the exact saved envelope.
/// No library name, world state, station function or live engine is accessible.
/// # Errors
/// Malformed, oversized, executable or conflicting metadata.
pub(crate) fn validate(source: &str, envelope: &Envelope) -> Result<(), RollError> {
    validate_for_api(source, envelope, super::API_VERSION)
}

/// Only the closed retained-subject path may validate an earlier declaration.
/// The literal API still belongs to its exact immutable envelope. This does not
/// grant source authority or run an old engine: authoring compiles the source in
/// the current sandbox after this bounded metadata check.
pub(crate) fn validate_retained_registry(
    source: &str,
    envelope: &Envelope,
) -> Result<(), RollError> {
    let api = match envelope.mind_abi.as_str() {
        abi if abi == super::MIND_ABI => super::API_VERSION,
        "rhai=1.26.0;api=4;stock=2" => 4,
        "rhai=1.26.0;api=5;stock=2" => 5,
        _ => return Err(invalid("mind_abi", "unsupported retained Registry ABI")),
    };
    validate_for_api(source, envelope, api)
}

/// Retained archival validity keeps the original two-argument entry point,
/// without installing the current station API or granting executability.
pub(super) fn require_retained_think(source: &str) -> Result<(), RollError> {
    let ast = metadata_engine()
        .compile(source)
        .map_err(|error| invalid("source", error.to_string()))?;
    if !ast
        .iter_functions()
        .any(|function| function.name == "think" && function.params.len() == 2)
    {
        return Err(invalid("source", "expected fn think(view, card)"));
    }
    Ok(())
}

fn validate_for_api(source: &str, envelope: &Envelope, api: u32) -> Result<(), RollError> {
    if source.len() > mush_roll::MAX_SOURCE_BYTES || source.contains('\0') {
        return Err(invalid("source", "maximum 32 KiB; NUL is forbidden"));
    }
    let engine = metadata_engine();
    let mut ast = engine
        .compile(source)
        .map_err(|error| invalid("source", error.to_string()))?;
    // Ask the pinned parser whether MIND is a declared global, without running
    // it. An undefined MIND is optional absence. A declared nonliteral or mutable
    // MIND is refused, rather than silently treated as absent. This uses Rhai's
    // own identifier/scoping grammar, including comments, raw strings and Unicode.
    // A top-level expression sees mutable globals too. Its AST is discarded.
    let probe = format!("{source}\n; MIND;");
    let declared = match engine.compile(&probe) {
        Ok(_) => true,
        Err(error) if matches!(error.err_type(), ParseErrorType::VariableUndefined(name) if name == "MIND") => {
            false
        }
        Err(error) => return Err(invalid("MIND", error.to_string())),
    };
    if declared {
        let mut declarations = ast
            .iter_literal_variables(true, true)
            .filter(|(name, _, _)| *name == "MIND");
        let Some((_, true, value)) = declarations.next() else {
            return Err(invalid("MIND", "expected a literal const MIND map"));
        };
        if declarations.next().is_some() {
            return Err(invalid("MIND", "declare the header exactly once"));
        }
        check_manifest(&bounded_json(value)?, envelope, api)?;
    }

    let card_functions = ast
        .iter_functions()
        .filter(|function| function.name == "card")
        .map(|function| function.params.len())
        .collect::<Vec<_>>();
    if !card_functions.is_empty() {
        if card_functions.as_slice() != [0] {
            return Err(invalid("card", "expected exactly one fn card()"));
        }
        let mut scope: Scope<'_> = ast.iter_literal_variables(true, false).collect();
        // call_fn normally evaluates AST statements first. Removing them is
        // essential: `while true {}` or a top-level station call must not run.
        ast.clear_statements();
        let schema: Dynamic = engine
            .call_fn(&mut scope, &ast, "card", ())
            .map_err(|error| invalid("card", error.to_string()))?;
        if bounded_json(schema)? != envelope.schema {
            return Err(invalid(
                "card",
                "schema differs from the draft envelope; update the draft schema before saving",
            ));
        }
    }
    Ok(())
}

fn metadata_engine() -> Engine {
    let mut engine = Engine::new_raw();
    // No LanguageCore/Time package: neither sleep nor clocks are registered.
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
    engine.register_global_module(BasicArrayPackage::new().as_shared_module());
    engine.register_global_module(BasicMapPackage::new().as_shared_module());
    // Compile unrelated think bodies containing stock:: helpers, but do not
    // expose their implementations to metadata execution.
    engine.register_static_module("stock", Module::new().into());
    // `goto` is Rhai's reserved word. Match the production parser so ordinary
    // think bodies compile, but metadata evaluation cannot issue a station call.
    let _ = engine.register_custom_syntax(["goto", "(", "$expr$", ")"], false, |_, _| {
        Err(mush_mind::system_error(
            "station calls are unavailable in metadata",
        ))
    });
    engine.set_module_resolver(DummyModuleResolver::new());
    for symbol in ["eval", "import", "export", "Fn", "call", "curry"] {
        engine.disable_symbol(symbol);
    }
    engine.set_strict_variables(true);
    engine.set_fail_on_invalid_map_property(true);
    engine.set_allow_anonymous_fn(false);
    engine.set_max_operations(MAX_METADATA_OPS);
    engine.set_max_call_levels(4);
    engine.set_max_expr_depths(48, 24);
    engine.set_max_variables(64);
    engine.set_max_functions(32);
    engine.set_max_modules(0);
    engine.set_max_map_size(256);
    engine.set_max_array_size(256);
    engine.set_max_string_size(8 * 1024);
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine
}

fn check_manifest(value: &Value, envelope: &Envelope, api: u32) -> Result<(), RollError> {
    let map = value
        .as_object()
        .ok_or_else(|| invalid("MIND", "expected a literal map"))?;
    if map.len() != 5
        || map
            .keys()
            .any(|key| !matches!(key.as_str(), "name" | "api" | "class" | "role" | "core"))
    {
        return Err(invalid("MIND", "expected name, api, class, role and core"));
    }
    let name = map.get("name").and_then(Value::as_str).unwrap_or_default();
    if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(invalid("MIND.name", "expected 1..128 printable bytes"));
    }
    if map.get("api").and_then(Value::as_u64) != Some(u64::from(api)) {
        return Err(invalid("MIND.api", "differs from the envelope mind API"));
    }
    if map.get("class").and_then(Value::as_str) != Some("discretion") {
        return Err(invalid(
            "MIND.class",
            "authored drafts declare discretion; this grants no licence",
        ));
    }
    if map.get("role").and_then(Value::as_str) != Some("combat") {
        return Err(invalid("MIND.role", "only the combat role is available"));
    }
    if map.get("core").and_then(Value::as_str) != Some(envelope.declared_class.as_str()) {
        return Err(invalid(
            "MIND.core",
            "differs from the draft's declared core class",
        ));
    }
    Ok(())
}

fn bounded_json(value: Dynamic) -> Result<Value, RollError> {
    let mut remaining = MAX_METADATA_NODES;
    let value = json_value(value, 0, &mut remaining)?;
    let encoded =
        serde_json::to_vec(&value).map_err(|_| invalid("metadata", "cannot encode value"))?;
    if encoded.len() > MAX_METADATA_BYTES {
        return Err(invalid("metadata", "maximum 64 KiB"));
    }
    Ok(value)
}

fn json_value(value: Dynamic, depth: usize, remaining: &mut usize) -> Result<Value, RollError> {
    if depth > 16 || *remaining == 0 {
        return Err(invalid("metadata", "maximum depth 16 and 4096 values"));
    }
    *remaining -= 1;
    if value.is_unit() {
        Ok(Value::Null)
    } else if value.is::<bool>() {
        Ok(Value::Bool(value.cast::<bool>()))
    } else if value.is::<INT>() {
        Ok(Value::from(value.cast::<INT>()))
    } else if value.is::<ImmutableString>() {
        Ok(Value::String(value.cast::<ImmutableString>().to_string()))
    } else if value.is::<Array>() {
        value
            .cast::<Array>()
            .into_iter()
            .map(|item| json_value(item, depth + 1, remaining))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    } else if value.is::<Map>() {
        value
            .cast::<Map>()
            .into_iter()
            .map(|(key, item)| {
                json_value(item, depth + 1, remaining).map(|value| (key.to_string(), value))
            })
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Value::Object)
    } else {
        Err(invalid("metadata", "only integer JSON values are allowed"))
    }
}

fn invalid(field: &str, message: impl Into<String>) -> RollError {
    RollError::Invalid {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope() -> Envelope {
        Envelope {
            schema: json!({"sections": []}),
            presets: json!({}),
            profiles: json!({"Default": {}}),
            rules: json!({}),
            declared_class: "II".into(),
            mind_abi: super::super::MIND_ABI.into(),
        }
    }

    fn header() -> String {
        format!(
            "const MIND = #{{ name: \"Sentinel\", api: {}, class: \"discretion\", role: \"combat\", core: \"II\" }};",
            super::super::API_VERSION
        )
    }

    #[test]
    fn optional_metadata_never_executes_top_level_code() {
        let answer = validate("while true {} fn think(v, c) {}", &envelope());
        assert!(answer.is_ok(), "{answer:?}");
        assert!(validate("fn think(v,c) { goto ([1, 2]); }", &envelope()).is_ok());
        assert!(validate(&format!("{} while true {{}} fn card() {{ #{{sections: []}} }} fn think(v,c) {{ this = stock::think(v,c,this); }}", header()), &envelope()).is_ok());
        assert!(
            validate(
                "// const MIND = explode();\nfn think(v,c) { let s = `const MIND = false;`; }",
                &envelope()
            )
            .is_ok()
        );
    }

    #[test]
    fn declared_metadata_is_literal_and_matches_the_envelope() {
        for source in [
            "let MIND = #{}; fn think(v,c) {}".to_owned(),
            "const MIND = foo(); fn think(v,c) {}".to_owned(),
            format!(
                "{} fn think(v,c) {{}}",
                header().replace("\"II\"", "\"III\"")
            ),
            format!(
                "{} fn think(v,c) {{}}",
                header().replace("discretion", "doctrine")
            ),
            "fn card() { #{sections: [1]} } fn think(v,c) {}".to_owned(),
        ] {
            assert!(validate(&source, &envelope()).is_err(), "{source}");
        }
    }

    #[test]
    fn card_has_no_game_io_dynamic_eval_or_unbounded_work() {
        for body in [
            "while true {}",
            "sleep(1)",
            "eval(\"1\")",
            "stock::think((),(),#{})",
            "helm(90)",
            "goto ([1,2]); #{sections: []}",
            "#{sections: [], x: 1.5}",
        ] {
            let source = format!("fn card() {{ {body} }} fn think(v,c) {{}}");
            assert!(validate(&source, &envelope()).is_err(), "{body}");
        }
    }
}
