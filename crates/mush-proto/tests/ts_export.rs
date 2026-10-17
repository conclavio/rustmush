//! TypeScript binding generation for the web client (`--features ts`).
//!
//! Run with `cargo test -p mush-proto --features ts` (alias: `cargo gen-ts`).
//! Regenerates `web/src/lib/proto/` — one `.ts` file per wire type plus a
//! barrel `index.ts`. Generation is idempotent and the output is COMMITTED,
//! so web builds never need cargo; re-run after any wire-type change.
#![cfg(feature = "ts")]

use std::fs;
use std::path::Path;

use mush_proto::{
    AtlasMapDelta, AtlasMapFrame, ClientCommand, Color, Envelope, InvokeRequest, InvokeResult,
    MindDiagnostic, MindRequest, MindResponse, OutputEvent, RoomView, Span, VitalValue,
    VitalsFrame, WhoEntry,
};
use ts_rs::{Config, TS};

/// `web/src/lib/proto`, located relative to this crate so the test works from
/// any working directory.
fn proto_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/src/lib/proto"
    ))
}

/// The ts-rs config used for every export: large integers as `number` (the
/// wire is JSON — `serde_json` emits plain numbers and `JSON.parse` yields
/// `number`, never `bigint`), output into the web client's proto dir.
fn config() -> Config {
    Config::new()
        .with_out_dir(proto_dir())
        .with_large_int("number")
}

/// Collects `.ts` files under `dir` (recursing into subdirectories — ts-rs
/// puts foreign-crate impls like `serde_json/JsonValue.ts` in subdirs) as
/// `/`-separated module paths relative to `dir`, without the extension.
fn collect_ts_modules(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read proto dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            let name = path.file_name().expect("dir name").to_str().expect("utf-8");
            collect_ts_modules(&path, &format!("{prefix}{name}/"), out);
        } else if path.extension().is_some_and(|e| e == "ts") {
            let stem = path
                .file_stem()
                .expect("file stem")
                .to_str()
                .expect("utf-8 filename");
            out.push(format!("{prefix}{stem}"));
        }
    }
}

/// Removes every `.ts` file under `dir`, recursively — the whole directory is
/// generated, so this drops stale bindings from removed/renamed types.
fn remove_ts_files(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read proto dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            remove_ts_files(&path);
        } else if path.extension().is_some_and(|e| e == "ts") {
            fs::remove_file(&path).expect("remove stale binding");
        }
    }
}

/// Normalize generator-only whitespace so the committed bindings are
/// byte-stable on every host. `ts-rs` currently leaves a space before some
/// inline union newlines; keeping that artifact would make the CI drift check
/// sensitive to generator whitespace and violate the repository whitespace
/// policy.
fn normalize_ts_files(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read proto dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            normalize_ts_files(&path);
        } else if path.extension().is_some_and(|e| e == "ts") {
            let source = fs::read_to_string(&path).expect("read generated binding");
            let mut normalized = String::with_capacity(source.len());
            for line in source.lines() {
                normalized.push_str(line.trim_end());
                normalized.push('\n');
            }
            fs::write(&path, normalized).expect("normalize generated binding");
        }
    }
}

/// Exports every wire type and rebuilds the barrel `index.ts`.
///
/// A single test (not one per type) so file removal, export, verification and
/// barrel generation cannot interleave.
#[test]
fn export_typescript_bindings() {
    let dir = proto_dir();
    fs::create_dir_all(dir).expect("create proto dir");
    remove_ts_files(dir);

    let cfg = config();
    // `export_all` walks dependencies: Envelope covers OutputEvent and every
    // payload type; ClientCommand covers Query and ClientCaps + enums.
    Envelope::export_all(&cfg).expect("export Envelope graph");
    ClientCommand::export_all(&cfg).expect("export ClientCommand graph");
    // The vitals frame rides inside `Panel.data` (opaque to the envelope
    // graph), so it is exported explicitly with its Vital/VitalValue/… deps.
    VitalsFrame::export_all(&cfg).expect("export VitalsFrame graph");
    // The map frame and its delta ride inside `Panel.data` too (S4).
    AtlasMapFrame::export_all(&cfg).expect("export AtlasMapFrame graph");
    AtlasMapDelta::export_all(&cfg).expect("export AtlasMapDelta graph");
    normalize_ts_files(dir);

    // Barrel: web code imports from "$lib/proto" only.
    let mut modules = Vec::new();
    collect_ts_modules(dir, "", &mut modules);
    modules.sort();
    let mut index = String::from(
        "// Generated by `cargo test -p mush-proto --features ts` (crates/mush-proto/tests/ts_export.rs).\n\
         // Do not edit by hand.\n",
    );
    for m in &modules {
        index.push_str(&format!("export type * from \"./{m}\";\n"));
    }
    fs::write(dir.join("index.ts"), index).expect("write index.ts");
}

/// The tricky shapes, asserted against the emitted TypeScript so a ts-rs
/// upgrade that changes representation fails loudly instead of silently
/// drifting the web client's types. Uses `export_to_string` (same content the
/// export test writes) so it never races the file regeneration above.
#[test]
fn generated_shapes_match_wire_format() {
    let cfg = config();

    // Envelope: `#[serde(flatten)]` event — seq/ts intersected with the
    // OutputEvent discriminated union, exactly like the wire.
    let envelope = Envelope::export_to_string(&cfg).expect("Envelope decl");
    assert!(
        envelope.contains("seq: number") && envelope.contains("ts: number"),
        "Envelope must carry seq/ts as numbers:\n{envelope}"
    );
    // ts-rs inlines the flattened enum: `{ seq, ts } & (<OutputEvent union>)`.
    // Structurally identical to `& OutputEvent`, and regenerated in lockstep
    // with OutputEvent.ts, so it cannot drift.
    assert!(
        envelope.contains(r#"} & ({ "t": "room""#),
        "Envelope must flatten the OutputEvent union in as an intersection:\n{envelope}"
    );

    // Adjacent tagging: discriminated unions on "t"/"d".
    let output_event = OutputEvent::export_to_string(&cfg).expect("OutputEvent decl");
    for needle in [
        r#""t": "narrative""#,
        r#""t": "room", "d": RoomView"#,
        r#""t": "sys", "d": RichText"#,
        r#""t": "resumed""#,
        r#""t": "ping""#,
        r#""t": "composing""#,
        r#""t": "request""#,
        r#""t": "notice""#,
        r#""t": "invoke_result""#,
    ] {
        assert!(
            output_event.contains(needle),
            "OutputEvent missing {needle}:\n{output_event}"
        );
    }
    let client_command = ClientCommand::export_to_string(&cfg).expect("ClientCommand decl");
    for needle in [
        r#""t": "line", "d": string"#,
        r#""t": "ack""#,
        "commit: string",
        r#""t": "pong""#,
        r#""t": "composing""#,
        r#""t": "respond""#,
        r#""t": "invoke""#,
    ] {
        assert!(
            client_command.contains(needle),
            "ClientCommand missing {needle}:\n{client_command}"
        );
    }

    // Owner requests and results remain distinct closed discriminated unions.
    let invoke_request = InvokeRequest::export_to_string(&cfg).expect("InvokeRequest");
    assert!(invoke_request.contains(r#""service": "mind", "request": MindRequest"#));
    let invoke_result = InvokeResult::export_to_string(&cfg).expect("InvokeResult");
    assert!(invoke_result.contains(r#""service": "mind", "result": MindResponse"#));
    let mind_request = MindRequest::export_to_string(&cfg).expect("MindRequest");
    for needle in [
        r#""op": "replace""#,
        r#""op": "check""#,
        r#""op": "save""#,
        r#""op": "trace""#,
    ] {
        assert!(
            mind_request.contains(needle),
            "MindRequest missing {needle}: {mind_request}"
        );
    }
    let mind_response = MindResponse::export_to_string(&cfg).expect("MindResponse");
    for needle in [
        r#""status": "read""#,
        r#""status": "queued""#,
        "rules_text",
        "rules_source",
        "diagnostics",
    ] {
        assert!(
            mind_response.contains(needle),
            "MindResponse missing {needle}: {mind_response}"
        );
    }
    let diagnostic = MindDiagnostic::export_to_string(&cfg).expect("MindDiagnostic");
    assert!(
        diagnostic.contains("line: number | null") && diagnostic.contains("column: number | null")
    );

    // RoomView.ambient: an open map typed loosely on purpose (I1, D-I-4).
    let room_view = RoomView::export_to_string(&cfg).expect("RoomView decl");
    assert!(
        room_view.contains("ambient?: Record<string, unknown>"),
        "RoomView.ambient must be an optional open record:\n{room_view}"
    );

    // Vitals: internally tagged values, a track of nullable marks (I1, D-I-2).
    let vital_value = VitalValue::export_to_string(&cfg).expect("VitalValue decl");
    for needle in [
        r#""kind": "bounded""#,
        r#""kind": "track""#,
        r#""kind": "text""#,
        "Array<string | null>",
    ] {
        assert!(
            vital_value.contains(needle),
            "VitalValue missing {needle}:\n{vital_value}"
        );
    }

    // Color: externally tagged, tuple variant as tuple — all three forms.
    let color = Color::export_to_string(&cfg).expect("Color decl");
    for needle in [
        r#"{ "ansi": number }"#,
        r#"{ "idx256": number }"#,
        r#"{ "rgb": [number, number, number] }"#,
    ] {
        assert!(color.contains(needle), "Color missing {needle}:\n{color}");
    }

    // u64/i64 must be JSON numbers, never bigint (config().with_large_int).
    let who = WhoEntry::export_to_string(&cfg).expect("WhoEntry decl");
    for (name, src) in [
        ("Envelope", &envelope),
        ("OutputEvent", &output_event),
        ("ClientCommand", &client_command),
        ("WhoEntry", &who),
    ] {
        assert!(!src.contains("bigint"), "{name} leaked bigint:\n{src}");
    }

    // skip_serializing_if fields are optional (`?`), matching their absence
    // on the wire when unset.
    let span = Span::export_to_string(&cfg).expect("Span decl");
    for needle in ["style?", "tag?", "link?"] {
        assert!(span.contains(needle), "Span missing {needle}:\n{span}");
    }
    assert!(who.contains("here?"), "WhoEntry.here optional:\n{who}");
}
