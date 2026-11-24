//! Optional author metadata, read without executing source top-level statements.
//!
//! The revision's external envelope stays authoritative. Metadata is a checked
//! declaration, never a licence or authority grant. One isolated, disposable
//! engine can evaluate `card()` using only bounded pure data operations.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use mush_mind::rhai::{
    Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Module, ParseErrorType, Position,
    Scope,
    module_resolvers::DummyModuleResolver,
    packages::{ArithmeticPackage, LogicPackage, Package},
};
use mush_roll::{Envelope, RollError};
use serde_json::Value;

const MAX_METADATA_OPS: u64 = 2_000;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_METADATA_NODES: usize = 4_096;
const MAX_METADATA_DEPTH: usize = mush_mind::MAX_CONTAINER_DEPTH;
const MAX_METADATA_ERR_BYTES: usize = 256;

/// Validate optional `const MIND` and `fn card()` against the exact saved envelope.
/// No library name, world state, station function or live engine is accessible.
/// # Errors
/// Malformed, oversized, executable or conflicting metadata.
pub(crate) fn validate(source: &str, envelope: &Envelope) -> Result<(), RollError> {
    validate_for_api(source, envelope, super::API_VERSION, false)
}

/// Only the closed retained-subject path may validate an earlier declaration.
/// The literal API still belongs to its exact immutable envelope. This does not
/// grant source authority or run an old engine: authoring compiles the source in
/// the current sandbox after this bounded metadata check.
#[cfg(test)]
pub(crate) fn validate_retained_registry(
    source: &str,
    envelope: &Envelope,
) -> Result<(), RollError> {
    validate_retained_registry_checked(source, envelope, false)
}

/// Single-pass retained metadata validation, optionally also requiring the original
/// two-argument `fn think(view, card)` entry point on the already-parsed AST so
/// archival source reads avoid constructing a second engine or recompiling `source`.
pub(crate) fn validate_retained_registry_checked(
    source: &str,
    envelope: &Envelope,
    require_think: bool,
) -> Result<(), RollError> {
    // Doc 45 unit AP.3: every ABI since the Registry opened, one table
    // (`super::RETAINED_ABIS`). Until that cut the list stopped at API 5, so a
    // revision saved under API 6, 7 or 8 was refused here for ever.
    let api = if envelope.mind_abi == super::MIND_ABI {
        super::API_VERSION
    } else {
        super::RETAINED_ABIS
            .iter()
            .find(|(abi, _)| *abi == envelope.mind_abi)
            .map(|(_, api)| *api)
            .ok_or_else(|| invalid("mind_abi", "unsupported retained Registry ABI"))?
    };
    validate_for_api(source, envelope, api, require_think)
}

fn validate_for_api(
    source: &str,
    envelope: &Envelope,
    api: u32,
    require_think: bool,
) -> Result<(), RollError> {
    if source.len() > mush_roll::MAX_SOURCE_BYTES || source.contains('\0') {
        return Err(invalid("source", "maximum 32 KiB; NUL is forbidden"));
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        validate_for_api_inner(source, envelope, api, require_think)
    }))
    .unwrap_or_else(|_| Err(invalid("metadata", "metadata validation failed")))
}

fn validate_for_api_inner(
    source: &str,
    envelope: &Envelope,
    api: u32,
    require_think: bool,
) -> Result<(), RollError> {
    let (engine, limit_tripped) = metadata_engine();
    let mut ast = engine
        .compile(source)
        .map_err(|error| invalid("source", error.to_string()))?;
    if require_think
        && !ast
            .iter_functions()
            .any(|function| function.name == "think" && function.params.len() == 2)
    {
        return Err(invalid("source", "expected fn think(view, card)"));
    }
    // Ask the pinned parser whether MIND is a declared global, without running
    // it. An undefined MIND is optional absence. A declared nonliteral or mutable
    // MIND is refused, rather than silently treated as absent. This uses Rhai's
    // own identifier/scoping grammar, including comments, raw strings and Unicode.
    // A top-level expression sees mutable globals too. Its AST is discarded.
    // Fast path: if the literal bytes "MIND" never appear in `source`, no `MIND`
    // identifier can be declared and we avoid a second full Rhai compile.
    let declared = if source.contains("MIND") {
        let probe = format!("{source}\n; MIND;");
        match engine.compile(&probe) {
            Ok(_) => true,
            Err(error) if matches!(error.err_type(), ParseErrorType::VariableUndefined(name) if name == "MIND") => {
                false
            }
            Err(error) => return Err(invalid("MIND", error.to_string())),
        }
    } else {
        false
    };
    if declared {
        let mut declarations = ast
            .iter_literal_variables(true, true)
            .map(|(name, is_const, value)| (name, is_const, SafeDynamic(value)))
            .filter(|(name, _, _)| *name == "MIND");
        let Some((_, true, value)) = declarations.next() else {
            return Err(invalid("MIND", "expected a literal const MIND map"));
        };
        if declarations.next().is_some() {
            return Err(invalid("MIND", "declare the header exactly once"));
        }
        check_manifest(&bounded_json(&value.0)?, envelope, api)?;
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
        let mut scope = SafeScope(Scope::new());
        for (name, _, value) in ast.iter_literal_variables(true, false) {
            scope.0.push_constant_dynamic(name, value);
        }
        // call_fn normally evaluates AST statements first. Removing them is
        // essential: `while true {}` or a top-level station call must not run.
        // Strip every script function other than `fn card()` so metadata cannot
        // invoke helper functions or bind `this` via method-call syntax.
        ast.clear_statements();
        ast.retain_functions(|_, _, name, params| name == "card" && params == 0);
        let schema = match engine.call_fn::<Dynamic>(&mut scope.0, &ast, "card", ()) {
            Ok(value) => SafeDynamic(value),
            Err(error) => return Err(invalid("card", sanitize_eval_error(error))),
        };
        if limit_tripped.load(Ordering::Relaxed) {
            return Err(invalid("card", "metadata container limit exceeded"));
        }
        if bounded_json(&schema.0)? != envelope.schema {
            return Err(invalid(
                "card",
                "schema differs from the draft envelope; update the draft schema before saving",
            ));
        }
    }
    Ok(())
}

fn metadata_engine() -> (Engine, Arc<AtomicBool>) {
    let mut engine = Engine::new_raw();
    // Only pure scalar arithmetic and logic packages are registered.
    // No LanguageCore/Time package (no sleep/clocks) and no BasicArrayPackage or
    // BasicMapPackage (blocking Array::pad unchecked resize wraparound,
    // deprecated string-name callback methods, and recursive container helpers;
    // array/map literals and indexing remain native Rhai syntax).
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
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
    engine.set_fast_operators(false);
    engine.set_max_operations(MAX_METADATA_OPS);
    engine.set_max_call_levels(1);
    engine.set_max_expr_depths(24, 24);
    engine.set_max_variables(64);
    engine.set_max_functions(32);
    engine.set_max_modules(0);
    engine.set_max_map_size(256);
    engine.set_max_array_size(256);
    engine.set_max_string_size(8 * 1024);
    let ops_spent = Arc::new(AtomicU64::new(0));
    let limit_tripped = Arc::new(AtomicBool::new(false));
    let progress_ops = Arc::clone(&ops_spent);
    let progress_limit = Arc::clone(&limit_tripped);
    engine.on_progress(move |_ops| {
        if progress_limit.load(Ordering::Relaxed) {
            return Some(Dynamic::from("limit"));
        }
        if progress_ops.fetch_add(1, Ordering::Relaxed) >= MAX_METADATA_OPS {
            Some(Dynamic::from("ops"))
        } else {
            None
        }
    });
    let var_limit = Arc::clone(&limit_tripped);
    #[allow(deprecated)]
    engine.on_var(move |name, _index, ctx| {
        if let Some(val) = ctx.scope().get(name)
            && (val.is_array() || val.is_map())
            && let Err(err) = check_depth(val, MAX_METADATA_DEPTH)
        {
            var_limit.store(true, Ordering::Relaxed);
            return Err(err);
        }
        Ok(None)
    });
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    (engine, limit_tripped)
}

struct SafeDynamic(Dynamic);

impl Drop for SafeDynamic {
    fn drop(&mut self) {
        drop_dynamic_iteratively(std::mem::take(&mut self.0));
    }
}

struct SafeScope<'a>(Scope<'a>);

impl Drop for SafeScope<'_> {
    fn drop(&mut self) {
        for (_, value, _) in std::mem::take(&mut self.0) {
            drop_dynamic_iteratively(value);
        }
    }
}

fn drop_dynamic_iteratively(value: Dynamic) {
    if !value.is_array() && !value.is_map() {
        return;
    }
    let mut stack = vec![value];
    while let Some(current) = stack.pop() {
        if current.is_array() {
            if let Some(array) = current.try_cast::<Array>() {
                for item in array {
                    if item.is_array() || item.is_map() {
                        stack.push(item);
                    }
                }
            }
        } else if current.is_map()
            && let Some(map) = current.try_cast::<Map>()
        {
            for (_, item) in map {
                if item.is_array() || item.is_map() {
                    stack.push(item);
                }
            }
        }
    }
}

fn check_depth(value: &Dynamic, max_depth: usize) -> Result<(), Box<EvalAltResult>> {
    if let Some(array) = value.read_lock::<Array>() {
        if max_depth == 0 {
            return Err(Box::new(EvalAltResult::ErrorDataTooLarge(
                "Container nesting depth".to_owned(),
                Position::NONE,
            )));
        }
        for item in array.iter() {
            check_depth(item, max_depth - 1)?;
        }
    } else if let Some(map) = value.read_lock::<Map>() {
        if max_depth == 0 {
            return Err(Box::new(EvalAltResult::ErrorDataTooLarge(
                "Container nesting depth".to_owned(),
                Position::NONE,
            )));
        }
        for item in map.values() {
            check_depth(item, max_depth - 1)?;
        }
    }
    Ok(())
}

fn sanitize_eval_error(mut error: Box<EvalAltResult>) -> String {
    let mut runtime_payload = false;
    let mut current = &mut *error;
    loop {
        match current {
            EvalAltResult::ErrorRuntime(payload, _) => {
                drop_dynamic_iteratively(std::mem::take(payload));
                runtime_payload = true;
                break;
            }
            EvalAltResult::ErrorTerminated(payload, _) | EvalAltResult::Return(payload, _) => {
                drop_dynamic_iteratively(std::mem::take(payload));
                break;
            }
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
            | EvalAltResult::ErrorInModule(_, inner, _) => {
                current = &mut **inner;
            }
            _ => break,
        }
    }
    if runtime_payload {
        return "card evaluation raised a runtime error".to_owned();
    }
    let mut text = error.to_string();
    if text.len() > MAX_METADATA_ERR_BYTES {
        let mut cut = MAX_METADATA_ERR_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
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
            "authored drafts declare discretion; this grants no license",
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

fn bounded_json(value: &Dynamic) -> Result<Value, RollError> {
    let mut remaining = MAX_METADATA_NODES;
    let value = json_value(value, 0, &mut remaining)?;
    let encoded =
        serde_json::to_vec(&value).map_err(|_| invalid("metadata", "cannot encode value"))?;
    if encoded.len() > MAX_METADATA_BYTES {
        return Err(invalid("metadata", "maximum 64 KiB"));
    }
    Ok(value)
}

fn json_value(value: &Dynamic, depth: usize, remaining: &mut usize) -> Result<Value, RollError> {
    if *remaining == 0 {
        return Err(invalid("metadata", "maximum depth 8 and 4096 values"));
    }
    *remaining -= 1;
    if value.is_unit() {
        Ok(Value::Null)
    } else if let Ok(flag) = value.as_bool() {
        Ok(Value::Bool(flag))
    } else if let Ok(int) = value.as_int() {
        Ok(Value::from(int))
    } else if let Some(text) = value.read_lock::<ImmutableString>() {
        Ok(Value::String(text.to_string()))
    } else if let Some(array) = value.read_lock::<Array>() {
        if depth >= MAX_METADATA_DEPTH {
            return Err(invalid("metadata", "maximum depth 8 and 4096 values"));
        }
        array
            .iter()
            .map(|item| json_value(item, depth + 1, remaining))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    } else if let Some(map) = value.read_lock::<Map>() {
        if depth >= MAX_METADATA_DEPTH {
            return Err(invalid("metadata", "maximum depth 8 and 4096 values"));
        }
        map.iter()
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

    #[test]
    fn p2_tsc_1_card_blocks_array_pad_string_callbacks_and_deep_nesting() {
        let cases = [
            // CRIT-6: Array::pad unchecked resize wraparound.
            "fn card() { let a = [1, 2]; a.pad(9223372036854775807, [1, 2]); #{sections: []} } fn think(v,c) {}",
            // HIGH-16: deprecated string-name callback methods bypassing Fn/budget guards.
            "fn f(x) { let i = 0; while i < 500 { i += 1; } x } fn card() { let a = [1, 2, 3, 4]; a.map(\"f\"); #{sections: []} } fn think(v,c) {}",
            "fn f(x) { true } fn card() { let a = [1, 2, 3]; a.filter(\"f\"); #{sections: []} } fn think(v,c) {}",
            "fn f(a, b) { a + b } fn card() { let a = [1, 2, 3]; a.reduce(\"f\"); #{sections: []} } fn think(v,c) {}",
            "fn cmp(a, b) { let i = 0; while i < 500 { i += 1; } 0 } fn card() { let a = [3, 2, 1]; a.sort(\"cmp\"); #{sections: []} } fn think(v,c) {}",
            "fn cmp(a, b) { true } fn card() { let a = [1, 1, 2]; a.dedup(\"cmp\"); #{sections: []} } fn think(v,c) {}",
            "fn f(x) { true } fn card() { let a = [1, 2]; a.drain(\"f\"); #{sections: []} } fn think(v,c) {}",
            "fn f(x) { true } fn card() { let a = [1, 2]; a.retain(\"f\"); #{sections: []} } fn think(v,c) {}",
            // CRIT-7: alternating Array/Map nesting via while loop, straight-line, fast-op +=, or method `this`.
            "fn card() { let x = #{}; let i = 0; while i < 250 { i += 1; x = #{a: [x]}; } x } fn think(v,c) {}",
            "fn card() { let x = #{}; x = [#{a: x}]; x = [#{a: x}]; x = [#{a: x}]; x = [#{a: x}]; x = [#{a: x}]; x } fn think(v,c) {}",
            "fn card() { let a = []; let i = 0; while i < 50 { i += 1; let b = []; b += a; a = b; } a } fn think(v,c) {}",
            "fn wrap() { let i = 0; while i < 250 { i += 1; this = #{a: [this]}; } } fn card() { let x = #{}; x.wrap(); x } fn think(v,c) {}",
            "fn card() { if this != () { let i = 0; while i < 250 { i += 1; this = #{a: [this]}; } return; } let x = #{}; x.card(); x } fn think(v,c) {}",
            // CRIT-7: static container literals past MAX_METADATA_DEPTH (8) and past expr depth (24).
            "fn card() { [[[[[[[[[1]]]]]]]]] } fn think(v,c) {}",
            "fn card() { [[[[[[[[[[[[[[[[[[[[[[[[[1]]]]]]]]]]]]]]]]]]]]]]]]] } fn think(v,c) {}",
            // Trying to catch depth/ops failure inside card() must not recover.
            "fn card() { try { let x = #{}; let i = 0; while i < 20 { i += 1; x = #{a: [x]}; } } catch (e) {} #{sections: []} } fn think(v,c) {}",
            "fn card() { try { while true {} } catch (e) {} #{sections: []} } fn think(v,c) {}",
            // Deep literal constant referenced from card().
            "const DEEP = [[[[[[[[[1]]]]]]]]]; fn card() { let x = DEEP; #{sections: []} } fn think(v,c) {}",
            // ErrorRuntime container payload dropped iteratively and formatted safely.
            "fn card() { let x = [[[[[[[[1]]]]]]]]; throw x; } fn think(v,c) {}",
        ];
        let env = envelope();
        for source in cases {
            let err = validate(source, &env).expect_err(source);
            assert!(
                matches!(err, RollError::Invalid { .. }),
                "expected RollError::Invalid for {source}: {err:?}"
            );
            assert!(
                validate_retained_registry(source, &env).is_err(),
                "retained registry validation must also refuse: {source}"
            );
        }
    }

    #[test]
    fn p2_tsc_1_valid_full_depth_card_schema_still_passes() {
        let mut env = envelope();
        env.schema = json!({
            "sections": [
                {
                    "id": "fire",
                    "label": "Fire",
                    "station": "fire",
                    "fields": [
                        {
                            "id": "mode",
                            "label": "Mode",
                            "widget": {
                                "kind": "choice",
                                "choices": ["alpha", "bravo"],
                                "default": "alpha",
                                "help": "Select fire posture"
                            }
                        }
                    ]
                }
            ]
        });
        let source = format!(
            "{} fn card() {{ let widget = #{{ kind: \"choice\", choices: [\"alpha\", \"bravo\"], \"default\": \"alpha\", help: \"Select fire posture\" }}; let field = #{{ id: \"mode\", label: \"Mode\", widget: widget }}; #{{ sections: [ #{{ id: \"fire\", label: \"Fire\", station: \"fire\", fields: [field] }} ] }} }} fn think(v, c) {{}}",
            header()
        );
        let result = validate(&source, &env);
        assert!(result.is_ok(), "{result:?}");
    }
}
