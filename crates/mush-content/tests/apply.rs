//! The I lens on the frame: can any sequence of applies create a second object, lose
//! the latch, or leave the latch describing a world that is not there?

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_content::{
    Appliers, ApplyError, EntryApplier, Latches, Pack, RawEntry, Step, apply, load_pack,
    parse_pack, plan, retire_pack,
};
use mush_core::{CanonicalId, Ctx, ObjId, ObjKind, Outbox, World};
use mush_testkit::boot_world;

fn id(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("id")
}

/// A stub owner: `thing` entries become Things in Limbo; a def with `"refuse": true`
/// is refused at create; a thing whose def has `"pinned": true` refuses retire.
struct Things;

impl EntryApplier for Things {
    fn kinds(&self) -> Vec<CanonicalId> {
        vec![id("thing")]
    }

    fn phase(&self) -> u8 {
        1
    }

    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        _pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        if entry
            .def()
            .get("refuse")
            .is_some_and(|v| v == &serde_json::Value::Bool(true))
        {
            return Err(format!("{id} is marked refuse"));
        }
        let name = entry
            .def()
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("thing");
        let obj = ctx
            .w
            .create(ObjKind::Thing, name, mush_core::GOD, Some(mush_core::LIMBO))
            .map_err(|e| e.to_string())?;
        Ok(Some(obj))
    }

    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        _pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        let pinned = existing
            .and_then(|o| ctx.w.get(o))
            .is_some_and(|o| o.name.ends_with("(pinned)"));
        if pinned {
            return Err(format!("{id} is pinned"));
        }
        Ok(())
    }

    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        _pack: &CanonicalId,
        _id: &CanonicalId,
        existing: Option<ObjId>,
    ) -> Result<(), String> {
        if let Some(o) = existing {
            ctx.w.destroy(o).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn signed(id_str: &str, version: u32, entries: &str) -> String {
    let draft = format!(
        r#"{{"manifest":{{"id":"{id_str}","version":{version},"kind":"neighborhood","digest":"{}","requires":[]}},"entries":{entries}}}"#,
        "0".repeat(64)
    );
    let digest = parse_pack(&draft)
        .expect("parses")
        .computed_digest()
        .expect("digests");
    draft.replace(&"0".repeat(64), &digest.to_hex())
}

struct Harness {
    world: World,
    registry: mush_core::Registry,
    latches: Latches,
    appliers: Appliers,
}

impl Harness {
    fn new() -> Harness {
        let (world, registry) = boot_world();
        let mut appliers = Appliers::new();
        appliers
            .register("test", Arc::new(Things))
            .expect("registered");
        Harness {
            world,
            registry,
            latches: Latches::new(),
            appliers,
        }
    }

    fn with_ctx<T>(
        &mut self,
        f: impl FnOnce(&mut Ctx<'_>, &mut Latches, &Appliers) -> T,
    ) -> (T, Outbox) {
        let mut out = Outbox::new();
        let result = {
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: mush_core::GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &self.registry,
                module: Some(smol_str::SmolStr::new(mush_content::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            f(&mut ctx, &mut self.latches, &self.appliers)
        };
        (result, out)
    }

    fn things(&self) -> usize {
        self.world
            .get(mush_core::LIMBO)
            .map(|l| {
                l.contents
                    .iter()
                    .filter(|o| {
                        self.world
                            .get(**o)
                            .is_some_and(|o| o.kind == ObjKind::Thing)
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

const TWO: &str = r#"{"thing.a":{"kind":"thing","def":{"name":"A"}},"thing.b":{"kind":"thing","def":{"name":"B"}}}"#;

#[test]
fn an_unchanged_pack_applied_twice_creates_nothing_and_plans_nothing() {
    let mut h = Harness::new();
    let pack = load_pack(&signed("lot", 1, TWO)).expect("loads");
    let ((first, report), out) = h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("applies"));
    assert_eq!(report.created, 2);
    assert_eq!(first.placed().len(), 2);
    assert_eq!(h.things(), 2);
    assert!(
        out.deferred()
            .iter()
            .any(|d| matches!(d, mush_core::Deferred::Persist(op) if op.ns == "content")),
        "the latch was persisted in the same batch"
    );
    let ((second, report), _) = h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("applies"));
    assert_eq!(
        (
            report.created,
            report.updated,
            report.retired,
            report.unchanged
        ),
        (0, 0, 0, 2)
    );
    assert_eq!(second.placed(), first.placed(), "the same objects");
    assert_eq!(h.things(), 2, "no second object");
    let p = plan(&pack, h.latches.applied(&id("lot"))).expect("plan");
    assert!(p.is_noop());
}

#[test]
fn an_edited_entry_updates_and_a_dropped_entry_retires() {
    let mut h = Harness::new();
    let v1 = load_pack(&signed("lot", 1, TWO)).expect("v1");
    h.with_ctx(|ctx, l, a| apply(ctx, l, a, &v1).expect("applies"));
    let edited = TWO
        .replace(r#""name":"A""#, r#""name":"A prime""#)
        .replace(r#","thing.b":{"kind":"thing","def":{"name":"B"}}"#, "");
    let v2 = load_pack(&signed("lot", 2, &edited)).expect("v2");
    let p = plan(&v2, h.latches.applied(&id("lot"))).expect("plan");
    assert_eq!(p.count(Step::Update), 1);
    assert_eq!(p.count(Step::Retire), 1);
    let ((applied, report), _) = h.with_ctx(|ctx, l, a| apply(ctx, l, a, &v2).expect("applies"));
    assert_eq!((report.created, report.updated, report.retired), (0, 1, 1));
    assert_eq!(applied.version(), 2);
    assert_eq!(applied.entries().len(), 1);
    assert_eq!(h.things(), 1);
    let name = applied
        .object_of(&id("thing.a"))
        .and_then(|o| h.world.get(o))
        .map(|o| o.name.to_string());
    assert_eq!(name.as_deref(), Some("A prime"));
    let err = h
        .with_ctx(|ctx, l, a| apply(ctx, l, a, &v1))
        .0
        .expect_err("a downgrade is refused");
    assert!(matches!(err, ApplyError::Downgrade { .. }), "{err}");
}

#[test]
fn a_refused_entry_stops_the_apply_and_the_latch_describes_the_world() {
    let mut h = Harness::new();
    let entries = r#"{"thing.a":{"kind":"thing","def":{"name":"A"}},"thing.b":{"kind":"thing","def":{"name":"B","refuse":true}},"thing.c":{"kind":"thing","def":{"name":"C"}}}"#;
    let pack = load_pack(&signed("lot", 1, entries)).expect("loads");
    let ((applied, report), _) =
        h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("latched honestly"));
    assert_eq!(report.created, 1, "a was made, b refused, c never reached");
    assert!(
        report
            .anomaly
            .as_deref()
            .is_some_and(|a| a.contains("thing.b"))
    );
    assert_eq!(applied.anomaly(), report.anomaly.as_deref());
    assert_eq!(applied.entries().len(), 1);
    assert_eq!(h.things(), 1);
    // The next apply continues from the latch: a is unchanged, b refuses again, c waits.
    let ((again, report), _) =
        h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("latched honestly"));
    assert_eq!((report.created, report.unchanged), (0, 1));
    assert_eq!(again.entries().len(), 1);
    assert_eq!(h.things(), 1, "no duplicate of a");
}

#[test]
fn a_retire_that_would_orphan_a_reference_is_refused_whole() {
    let mut h = Harness::new();
    let entries = r#"{"thing.a":{"kind":"thing","def":{"name":"A (pinned)"}},"thing.b":{"kind":"thing","def":{"name":"B"}}}"#;
    let pack = load_pack(&signed("lot", 1, entries)).expect("loads");
    h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("applies"));
    let only_b = r#"{"thing.b":{"kind":"thing","def":{"name":"B"}}}"#;
    let v2 = load_pack(&signed("lot", 2, only_b)).expect("v2");
    let err = h
        .with_ctx(|ctx, l, a| apply(ctx, l, a, &v2))
        .0
        .expect_err("refused");
    assert!(
        matches!(err, ApplyError::RetireRefused { ref entry, .. } if entry == "thing.a"),
        "{err}"
    );
    assert_eq!(h.things(), 2, "nothing was touched");
    assert_eq!(
        h.latches
            .applied(&id("lot"))
            .map(mush_content::Applied::version),
        Some(1)
    );
    let err = h
        .with_ctx(|ctx, l, a| retire_pack(ctx, l, a, &id("lot"), "test"))
        .0
        .expect_err("the pack retire is refused too");
    assert!(matches!(err, ApplyError::RetireRefused { .. }));
}

#[test]
fn a_retired_pack_keeps_its_latch_and_does_not_satisfy_requires() {
    let mut h = Harness::new();
    let pack = load_pack(&signed("lot", 1, TWO)).expect("loads");
    h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("applies"));
    let ((retired, report), _) =
        h.with_ctx(|ctx, l, a| retire_pack(ctx, l, a, &id("lot"), "test").expect("retires"));
    assert_eq!(report.retired, 2);
    assert!(retired.is_retired());
    assert_eq!(h.things(), 0);
    assert!(h.latches.loaded().is_empty());
    let dependent = format!(
        r#"{{"manifest":{{"id":"annex","version":1,"kind":"site","digest":"{}","requires":[{{"id":"lot","version":1}}]}},"entries":{{}}}}"#,
        "0".repeat(64)
    );
    let digest = parse_pack(&dependent)
        .expect("p")
        .computed_digest()
        .expect("d");
    let dependent =
        load_pack(&dependent.replace(&"0".repeat(64), &digest.to_hex())).expect("loads");
    let err = h
        .with_ctx(|ctx, l, a| apply(ctx, l, a, &dependent))
        .0
        .expect_err("requires fail closed");
    assert!(
        matches!(
            err,
            ApplyError::Pack(mush_content::PackError::MissingRequire { .. })
        ),
        "{err}"
    );
    // Re-applying the retired pack is an explicit act that works again.
    let ((back, report), _) = h.with_ctx(|ctx, l, a| apply(ctx, l, a, &pack).expect("re-applies"));
    assert!(!back.is_retired());
    assert_eq!(report.created, 2);
    assert_eq!(h.things(), 2);
}

#[test]
fn an_unknown_kind_is_refused_naming_the_entry() {
    let mut h = Harness::new();
    let entries = r#"{"thing.a":{"kind":"thing","def":{}},"wall.x":{"kind":"feature","def":{}}}"#;
    let pack = load_pack(&signed("lot", 1, entries)).expect("loads");
    let err = h
        .with_ctx(|ctx, l, a| apply(ctx, l, a, &pack))
        .0
        .expect_err("refused");
    assert!(
        matches!(err, ApplyError::Pack(mush_content::PackError::UnknownEntryKind { ref entry, .. }) if entry == "wall.x"),
        "{err}"
    );
    assert_eq!(h.things(), 0);
}
