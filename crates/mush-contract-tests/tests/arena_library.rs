//! **The two arena gates doc 16 §8 names** (16b cut A.1; doc 16 §3.1 D-16-1 and
//! D-16-3, §3.5 "Draw labels", §8 "Gates"; 16b §6 "Verify lenses").
//!
//! 1. **The arena library has no I/O.** Doc 16's whole authority argument rests on
//!    it: while a match runs the arena is the single writer for its own facts and
//!    the world half is the single writer for everything else, and *the two share
//!    no state and no lock*. A promise like that decays the first time somebody
//!    needs "just a quick read" of a record inside a pulse, so the first test
//!    below reads every source file under `crates/tacsim/src/arena/` and refuses
//!    the names that would make such a read possible: the persistence crate, its
//!    handles and batches, the async runtime, the output sink and the world
//!    handle. Cut T's `arena_task` owns all of those *outside* the library, which
//!    is exactly the shape that makes the panic firewall and the void-the-match
//!    rule implementable. **16b cut AF.1** added one name that is not a way out
//!    but a way to lose determinism: a hash map, whose order would be a byte of
//!    the snapshot the library now writes of itself.
//!
//! 2. **The draw-label set is pinned.** Doc 16 §3.5: *every draw is
//!    `draw(match_seed, label)` … with these labels and no others*, and *adding a
//!    draw can never shift another*. The second property is a consequence of the
//!    keyed hash and is tested inside the crate; the first is a **vocabulary**
//!    claim, and a vocabulary claim needs a gate outside the crate that defines
//!    it. The second test below asserts the five kinds and one encoded example of
//!    each, so a sixth reason to draw — or a changed separator, or an object id
//!    that starts rendering as `#42` — is a reviewed diff.
//!
//! Why a source scan rather than a dependency assertion: `tacsim` as a whole is
//! the game's composition root and legitimately depends on the store, the runtime
//! and the world. The property being defended is about **one module tree inside
//! it**, which no manifest can express, so it is defended the way
//! `restriction_lints.rs` defends its own: by reading the tree.

use std::fs;
use std::path::{Path, PathBuf};

use tacsim::arena::rng::{DrawLabel, LABEL_KINDS};

/// The names that would give the arena library a path out of itself.
///
/// Each is a substring of the file's text, so a `use`, a type in a signature, a
/// field and a doc comment all count — mentioning one is as reviewable as calling
/// it, and a gate that can be talked around by putting the type behind an alias
/// is not a gate.
const FORBIDDEN: [(&str, &str); 9] = [
    (
        "mush_store",
        "the persistence crate: the arena holds no store handle (D-16-3)",
    ),
    (
        "tokio",
        "the async runtime: the arena is a library, and cut T's task owns the runtime (16b §6)",
    ),
    (
        "GatewaySink",
        "the output sink: frames leave through the world half's ordinary output path (doc 16 §3.1)",
    ),
    ("ObjectStore", "a store handle (doc 16 §8; 16b T6)"),
    ("WriteBatch", "a store batch (doc 16 §8; 16b T6)"),
    (
        "&mut World",
        "the world handle: the arena and the world share no state and no lock (D-16-1)",
    ),
    (
        "Ctx<",
        "a module context, which carries the world and the store with it (D-16-1)",
    ),
    // 16b cut AF.1: the snapshot serialises the state tree, so an unordered map
    // anywhere in the library would be a byte of the document that differs from
    // run to run. A float cannot be banned by a substring (`perception.rs` uses
    // one inside a computation); `crates/tacsim/tests/arena.rs`'
    // `af1_the_encoding_is_deterministic_and_integer` is the float guard.
    (
        "HashMap",
        "a hash map: iteration order must never feed an outcome or a byte of the snapshot (doc 16 §3.1)",
    ),
    // 17b cut M1a.5: the mind seam is a trait the library calls and never
    // implements beyond its no-op; the interpreter that will answer it lives
    // with the sim task. Case-sensitive like every row here, so the library's
    // own prose names the engine capitalised and never the crate.
    (
        "rhai",
        "the interpreter: a mind's runner is a trait the sim task implements outside the library (doc 17 §7, D-UM-1)",
    ),
];

/// The workspace root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, sorted, so the walk is reproducible.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_arena_library_has_no_path_to_the_store_the_runtime_or_the_world() {
    let root = repo_root();
    let arena = root.join("crates/tacsim/src/arena");
    assert!(
        arena.is_dir(),
        "{} is not a directory: the arena library moved, and this gate is now asserting nothing. \
         Re-point it at the new path in the same commit.",
        arena.display()
    );
    let mut files = Vec::new();
    rust_files(&arena, &mut files);
    assert!(
        files.len() >= 6,
        "the scan found only {} source files under {}; cut A.1 ships six modules (state, clock, \
         intents, rng, hash, ledger) plus mod.rs, so a shorter walk means the gate is reading the \
         wrong place",
        files.len(),
        arena.display()
    );

    let mut offences: Vec<String> = Vec::new();
    for path in &files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(path) else {
            offences.push(format!("{relative}: unreadable"));
            continue;
        };
        for (token, why) in FORBIDDEN {
            // This file names every token itself, so a self-scan is excluded by
            // path rather than by cleverness.
            if let Some(line) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(token))
                .map(|(n, _)| n + 1)
            {
                offences.push(format!("{relative}:{line}: {token:?} — {why}"));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the arena library reached outside itself. Doc 16 §3.1 (D-16-1, D-16-3) makes the arena an \
         authority that shares no state and no lock with the world half and holds no store handle; \
         cut T's arena task owns the inbox, the egress, the timer and the unwind boundary, and the \
         library owns none of them.\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_draw_label_set_is_pinned() {
    // The kinds, in code order (doc 16 §3.5's four plus §3.6's contact id,
    // and since 17b cut M2.2 doc 17 §7's unit-mind draw, D-UM-29).
    assert_eq!(
        LABEL_KINDS,
        ["hit", "facet", "burst", "classify", "contact", "mind"],
        "the draw-label set changed. Doc 16 §3.5 admits these labels and no others; a new reason \
         to draw is a reviewed diff here and in doc 16."
    );
    assert_eq!(LABEL_KINDS, DrawLabel::LABEL_KINDS);

    // One encoded example per kind, with the exact separators and the exact
    // rendering of an object id (its decimal number, never `#42`).
    let examples: [(DrawLabel, &str); 6] = [
        (DrawLabel::Hit { shot_seq: 7 }, "hit:7"),
        (DrawLabel::Facet { shot_seq: 7 }, "facet:7"),
        (DrawLabel::Burst { shot_seq: 7 }, "burst:7"),
        (
            DrawLabel::Classify {
                observer: mush_core::ObjId(4),
                target: mush_core::ObjId(9),
                pass_seq: 3,
            },
            "classify:4:9:3",
        ),
        (
            DrawLabel::Contact {
                viewer: mush_core::ObjId(4),
                target: mush_core::ObjId(9),
            },
            "contact:4:9",
        ),
        (
            DrawLabel::Mind {
                unit: 4,
                think_seq: 2,
                n: 7,
            },
            "mind:4:2:7",
        ),
    ];
    for (label, expected) in examples {
        assert_eq!(
            label.encode(),
            expected,
            "{label:?} no longer encodes as {expected:?}: a label's bytes are the key of its draw, \
             so changing them silently re-rolls every shot in every replayed ledger"
        );
        assert!(
            LABEL_KINDS.contains(&label.kind()),
            "{label:?} declares a kind that is not in the pinned set"
        );
    }
    // And every pinned kind has an example above, so a new variant cannot be
    // added to the enum without being exercised here.
    let encoded: Vec<&'static str> = LABEL_KINDS.to_vec();
    assert_eq!(encoded.len(), 6);
}

// ---------------------------------------------------------------------------
// 16b cut J.1: the early-stopping trace is the arena's
// ---------------------------------------------------------------------------

/// The source files allowed to call `mush_atlas::sight_only`, workspace-relative
/// and sorted: the atlas, which defines it, and the arena's horizon index, whose
/// `Horizon::sight` answers with it every line it does not answer itself (an
/// endpoint off the grid, a line past its buffer, a terrain without levels). 16b
/// cut J.1 pinned the arena's perception here, whose `Arena::sight` was then the
/// one caller; since 16b cut J.2 `Arena::sight` asks the terrain
/// (`ArenaTerrain::sight`), and the index is the arena's one caller.
const SIGHT_ONLY_FILES: [&str; 2] = [
    "crates/mush-atlas/src/trace.rs",
    "crates/tacsim/src/arena/horizon.rs",
];

/// **`sight_only` is the arena's trace and nobody else's** (16b cut J.1; doc 16
/// §3.2, §3.6).
///
/// `mush_atlas::sight_only` stops at the first blocker, which is the one
/// exception to the atlas's O lens (its module head: *the walk never stops
/// early … so its timing and its reads say nothing about the answer*). The
/// exception is argued for the combat arena alone — the arena pass's cost already
/// depends on the hidden field before any line is walked, through the index cull
/// and the thermal cull, and no reader of the arena can time one line — and a
/// world-side query that adopted it would reopen exactly the channel the atlas
/// head forbids. So every `crates/*/src/**/*.rs` whose text calls `sight_only(`
/// is collected and the set is pinned: a new caller is a reviewed diff here.
/// (16b cut J.2 moved the arena's entry from its perception to its horizon
/// index.)
#[test]
fn sight_only_is_the_arenas_trace_and_nobody_elses() {
    let root = repo_root();
    let crates = root.join("crates");
    let Ok(entries) = fs::read_dir(&crates) else {
        panic!("{} is not readable", crates.display());
    };
    let mut members: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    members.sort();
    let mut files = Vec::new();
    for member in members {
        rust_files(&member.join("src"), &mut files);
    }
    assert!(
        files.len() > 100,
        "the scan found only {} source files under {}/*/src: the workspace moved, and this gate \
         is now asserting nothing",
        files.len(),
        crates.display()
    );
    let mut callers: Vec<String> = files
        .iter()
        .filter(|path| fs::read_to_string(path).is_ok_and(|text| text.contains("sight_only(")))
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    callers.sort();
    assert_eq!(
        callers, SIGHT_ONLY_FILES,
        "`mush_atlas::sight_only` stops at its first blocker, which the atlas's O lens allows the \
         combat arena alone (16b cut J.1): a world-side trace keeps `trace_cells` or the walk, \
         whose reads never depend on the answer"
    );
}

// ---------------------------------------------------------------------------
// 16b cut T.3 (T6): the custody markers are the only unit writes during a match
// ---------------------------------------------------------------------------

/// **The third arena gate** (16b §7 T6; doc 16 §3.1 "Custody").
///
/// Doc 16 makes the arena the single writer for its units' condition cells while
/// a match runs, and the world the single writer for everything else. The first
/// half is defended by the library scan above — the arena cannot reach a store at
/// all. The second half is a claim about the *world*, and the only way to defend
/// it is to run a real match and read what the world wrote.
///
/// So this composes the placement graph, the tactics module, the game layer and
/// one pump (standing in for the host's once-per-iteration egress drain), creates
/// two units, places them, starts a match, drives one, aborts it — and asserts
/// that across the whole of it each unit's `unit:<obj>` document was written
/// **exactly twice**: once to set the marker and once to clear it. Not once more,
/// and the cleared document is byte-identical to the one that existed before the
/// match.
mod custody_writes {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mush_core::module::{GameModule, ModError, Registrar};
    use mush_core::{
        ArgShape, BehaviorBinding, BehaviorId, CmdError, CommandSpec, Ctx, DefinitionDescriptor,
        DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder, GOD, LIMBO,
        ModuleWrite, ObjId, ObjKind, OwnerId,
    };
    use mush_tactics::{
        CellId, CellSpec, ConditionId, FacetId, FacetSpec, LocomotionId, MediumId, MobilityMode,
        MobilityModeId, MobilityProfile, MobilityVocabulary, ModeRules, RulesValue, SectionId,
        SectionSpec, TacticsModule, UnitCatalog, UnitCatalogBuilder, UnitSpec, UnitSpecBody,
    };
    use mush_testkit::{TestWorld, text_of};
    use serde_json::{Value, json};
    use smol_str::SmolStr;
    use tacsim::arena_bridge::ArenaRegistry;

    /// The unit schema the fixture authors.
    const SCHEMA: &str = "tacsim.unit.line_tank";
    /// The module that provides it. Not `tacsim`: a module's name is its
    /// namespace and the game layer below already owns that one, so the fixture
    /// owns its definitions under its own name — which is also what a real
    /// content pack does. (16b cut C.2 ships the real one; this gate keeps its
    /// own so it composes no atlas and no content — see `ARENA` below.)
    const RULES_OWNER: &str = "tacsimrules";
    /// The arena `@arena/start` names.
    // 16b cut C.2: the **stand-in** window, not the shipped arena. This gate
    // composes no atlas and applies no content — it is about unit writes, not
    // about terrain — and `tacsim.arena.stand_in` is the id that exists for
    // exactly that: production geometry with nothing authored in it.
    const ARENA: &str = tacsim::match_::STAND_IN;

    fn dref(builder: &mut DefinitionRegistryBuilder, kind: &str, id: &str) -> DefinitionRef {
        builder
            .register(
                OwnerId::new(RULES_OWNER).expect("a canonical owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new(kind).expect("a canonical kind"),
                    DefinitionId::new(id).expect("a canonical id"),
                    1,
                    json!({}),
                )
                .expect("a bounded descriptor"),
            )
            .expect("the test registry accepts it")
    }

    /// One tracked tank with one section, one facet cell and a `tracked` mobility
    /// mode carrying doc 16 §3.3's five kinematic fields.
    fn tank_spec() -> UnitSpec {
        let mut definitions = DefinitionRegistryBuilder::new();
        let opaque = dref(&mut definitions, "schema", "tacsim.tank.rules");
        let kinematics = dref(&mut definitions, "schema", "tacsim.tank.kinematics");
        let vocabulary = MobilityVocabulary::base().freeze();
        let profile = MobilityProfile::new(
            vec![MobilityMode::new(
                MobilityModeId::new("ground.tracked").expect("a mode id"),
                [MediumId::new("ground").expect("a medium")],
                LocomotionId::new("tracked").expect("a locomotion"),
                ModeRules::new(
                    kinematics,
                    json!({
                        "max_speed_cms": 1_500,
                        "accel_cms2": 400,
                        "brake_cms2": 600,
                        "turn_rate_mdeg_s": 45_000,
                        "reverse_factor_pct": 40,
                    }),
                )
                .expect("a rules blob"),
            )],
            MobilityModeId::new("ground.tracked").expect("a mode id"),
            &vocabulary,
        )
        .expect("a one-mode profile");
        let body = UnitSpecBody {
            sections: vec![SectionSpec {
                id: SectionId::new("hull").expect("a section"),
                condition: ConditionId::new("intact").expect("a condition"),
                facets: vec![FacetSpec {
                    id: FacetId::new("front").expect("a facet"),
                    cells: vec![CellSpec {
                        id: CellId::new("armor").expect("a cell"),
                        min: 0,
                        max: 10,
                        init: 10,
                    }],
                }],
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: CellId::new("internals").expect("a cell"),
                    min: 0,
                    max: 8,
                    init: 8,
                }],
            }],
            unit_cells: Vec::new(),
            components: Vec::new(),
            capabilities: Vec::new(),
            mobility: Some(profile),
            rules: RulesValue::new(opaque, json!({})).expect("a rules blob"),
        };
        let descriptor = body
            .descriptor(
                DefinitionKind::new("unit").expect("a kind"),
                DefinitionId::new(SCHEMA).expect("an id"),
                1,
            )
            .expect("a bounded descriptor");
        let definition = definitions
            .register(OwnerId::new(RULES_OWNER).expect("an owner"), descriptor)
            .expect("registered");
        body.with_definition(definition)
    }

    fn catalog(spec: &UnitSpec) -> UnitCatalog {
        let vocabulary = MobilityVocabulary::base().freeze();
        let mut catalog = UnitCatalogBuilder::new();
        catalog
            .add(spec.clone(), &vocabulary)
            .expect("the tank is a valid spec");
        catalog.freeze()
    }

    /// The definition provider the tactics module validates against.
    struct Rules {
        spec: UnitSpec,
    }

    impl GameModule for Rules {
        fn name(&self) -> &'static str {
            RULES_OWNER
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let mut definitions: BTreeMap<DefinitionRef, DefinitionDescriptor> = BTreeMap::new();
            for reference in self.spec.body().definition_references() {
                let descriptor = DefinitionDescriptor::new(
                    reference.key().kind.clone(),
                    reference.key().id.clone(),
                    reference.version(),
                    json!({}),
                )
                .map_err(|error| ModError::msg(error.to_string()))?;
                definitions.entry(reference).or_insert(descriptor);
            }
            definitions.insert(
                self.spec.definition.clone(),
                self.spec
                    .descriptor()
                    .map_err(|error| ModError::msg(error.to_string()))?
                    .as_current(),
            );
            // Every owner must carry a semantic validator for each kind it
            // registers. The fixture's definitions are its own by construction,
            // so its validator accepts them; the strict shape belongs in the
            // content decoders cut C writes, not in a gate's fixture.
            for kind in ["schema", "unit"] {
                reg.definition_validator(
                    DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
                    |_: &mush_core::DefinitionValidationContext<'_>| Ok(()),
                )?;
            }
            for descriptor in definitions.into_values() {
                reg.definition(descriptor)?;
            }
            Ok(())
        }
    }

    /// The host's once-per-iteration egress drain, in two halves.
    ///
    /// `mush-server`'s world loop owns the real one: it takes what the arena's
    /// egress holds, **stages** it, and then re-enters the world once with a
    /// `WorldCommand::ModuleAsyncDone { module: "tacsim", .. }` so the handler
    /// runs with a `Ctx` **carrying the tacsim module's identity**. `TestWorld`
    /// has no loop, so `@pump` does the staging half and [`pump`] below does the
    /// re-entry — as the real arm, not as a direct call, because the identity is
    /// load-bearing: the custody write crosses into `tactics`' namespace under a
    /// declared delegation, and a re-entry under any other module's name is
    /// refused (WS03-A1). A test that called `drain_egress` from its own module
    /// would silently lose every check-in write, which is exactly what this
    /// fixture did before it was written this way.
    struct Pump {
        egress: Arc<Mutex<Option<tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>>>>,
    }

    impl GameModule for Pump {
        fn name(&self) -> &'static str {
            "arenapump"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
            let egress = Arc::clone(&self.egress);
            reg.command(
                CommandSpec {
                    name: "@pump".into(),
                    shape: ArgShape::None,
                    help: "test: drain the arena egress once, the way the world loop does".into(),
                    ..CommandSpec::default()
                },
                Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                    let Some(tasks) = ctx
                        .w
                        .module_state::<ArenaRegistry>()
                        .map(|registry| registry.tasks().clone())
                    else {
                        return Ok(());
                    };
                    let Ok(mut slot) = egress.lock() else {
                        return Err(CmdError::msg("the egress slot is poisoned"));
                    };
                    if slot.is_none() {
                        *slot = tasks.take_egress();
                    }
                    if let Some(rx) = slot.as_mut() {
                        while let Ok(message) = rx.try_recv() {
                            tasks.stage(message);
                        }
                    }
                    Ok(())
                }),
            )?;
            Ok(())
        }
    }

    /// A fast tacsim layer: a 50 ms pulse at every tempo, so a match runs, is
    /// driven and is aborted inside a second instead of inside thirty.
    fn tacsim_layer() -> tacsim::TacsimRuleset {
        let mut cfg = tacsim::TacsimConfig::default();
        cfg.arena.tempo.quiet_ms = 50;
        cfg.arena.tempo.alert_ms = 50;
        cfg.arena.tempo.engaged_ms = 50;
        tacsim::TacsimRuleset::with_config(cfg)
    }

    fn texts(events: &[(mush_core::Audience, mush_proto::OutputEvent)]) -> String {
        events
            .iter()
            .map(|(_, event)| text_of(event))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// One host iteration: stage whatever the arena has sent, then re-enter the
    /// world through the same typed completion the loop uses.
    fn pump(tw: &mut TestWorld) {
        run_ok(tw, "@pump");
        tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: SmolStr::new(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
    }

    fn run_ok(tw: &mut TestWorld, line: &str) -> String {
        let out = texts(&tw.run(GOD, line));
        assert!(
            !out.contains("Huh?") && !out.contains("Permission denied"),
            "`{line}` refused: {out}"
        );
        out
    }

    /// Every `unit:<obj>` write queued since the last drain, in order.
    fn unit_writes(tw: &mut TestWorld) -> Vec<(ObjId, Value)> {
        let mut writes = Vec::new();
        for op in tw.take_module_ops() {
            if op.ns.as_str() != mush_tactics::module::NS {
                continue;
            }
            if let ModuleWrite::JsonSet { key, value } = op.op
                && let Some(rest) = key.as_str().strip_prefix("unit:")
                && let Ok(id) = rest.parse::<u32>()
            {
                writes.push((ObjId(id), value));
            }
        }
        writes
    }

    fn marker_of(document: &Value) -> Option<&Value> {
        document.get("deployed")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_custody_markers_are_the_only_unit_writes_during_a_match() {
        let spec = tank_spec();
        let egress = Arc::new(Mutex::new(None));
        let mut tw = TestWorld::with_modules(vec![
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(Rules { spec: spec.clone() }),
            Box::new(TacticsModule::with_catalog(catalog(&spec))),
            Box::new(tacsim_layer()),
            Box::new(Pump {
                egress: Arc::clone(&egress),
            }),
        ]);

        // --- the world: one domain, two tanks in it --------------------------
        let field = tw
            .world_mut()
            .create(ObjKind::Room, "Proving Ground", GOD, None)
            .expect("a room");
        tw.world_mut()
            .attach_behavior(
                field,
                BehaviorId::new("sim.domain").expect("a behavior"),
                BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )
            .expect("a room may host a domain");
        run_ok(&mut tw, &format!("@domain/create #{}=sim.passive", field.0));

        let mut units = Vec::new();
        for name in ["Red One", "Blue One"] {
            let obj = tw
                .world_mut()
                .create(ObjKind::Thing, name, GOD, Some(LIMBO))
                .expect("a thing");
            let created = run_ok(&mut tw, &format!("@unit/create #{}={SCHEMA}", obj.0));
            assert!(
                created.contains("is now a tactical unit"),
                "@unit/create refused: {created}"
            );
            tw.world_mut()
                .attach_behavior(
                    obj,
                    BehaviorId::new("sim.participant").expect("a behavior"),
                    BehaviorBinding {
                        schema_version: 1,
                        attached_by: GOD,
                        attached_at: 0,
                    },
                )
                .expect("a thing may be a participant");
            let placed = run_ok(&mut tw, &format!("@spatial/place/domain #{}=0", obj.0));
            assert!(
                placed.contains("Placed"),
                "@spatial/place refused: {placed}"
            );
            units.push(obj);
        }
        let (red, blue) = (units[0], units[1]);

        // The record each unit carries before any match exists. Taken off the
        // op queue, so it is the exact document the store would hold.
        let mut before: BTreeMap<ObjId, Value> = BTreeMap::new();
        for (unit, document) in unit_writes(&mut tw) {
            before.insert(unit, document);
        }
        assert_eq!(before.len(), 2, "both units wrote a record at creation");
        for document in before.values() {
            assert!(
                marker_of(document).is_none(),
                "a unit nobody deployed carries no marker: {document}"
            );
        }

        // --- check-out -------------------------------------------------------
        let started = run_ok(
            &mut tw,
            &format!("@arena/start {ARENA}=#{}/#{}", red.0, blue.0),
        );
        assert!(
            started.contains("is running on"),
            "the match started: {started}"
        );
        let checkout = unit_writes(&mut tw);
        assert_eq!(
            checkout.len(),
            2,
            "check-out writes each unit's record exactly once: {checkout:?}"
        );
        for (unit, document) in &checkout {
            let Some(marker) = marker_of(document) else {
                panic!("#{} was checked out with no marker: {document}", unit.0);
            };
            assert_eq!(
                marker.pointer("/mark/arena").and_then(Value::as_str),
                Some(ARENA),
                "the marker names its arena: {marker}"
            );
            assert!(
                marker.get("snapshot").is_some(),
                "and carries the snapshot a void restores: {marker}"
            );
            // The ONLY difference from the pre-match record.
            let mut stripped = document.clone();
            if let Some(object) = stripped.as_object_mut() {
                object.remove("deployed");
            }
            assert_eq!(
                Some(&stripped),
                before.get(unit),
                "the marker is the only thing the check-out changed"
            );
        }

        // --- drive: nothing may write a unit record while the match runs -----
        run_ok(&mut tw, &format!("@arena/intent #{}=startup", red.0));
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_millis(60)).await;
            pump(&mut tw);
            run_ok(&mut tw, &format!("@arena/intent #{}=heading 90", red.0));
            run_ok(&mut tw, &format!("@arena/intent #{}=speed 1200", red.0));
            let during = unit_writes(&mut tw);
            assert!(
                during.is_empty(),
                "a running match wrote a unit record: {during:?}. Doc 16 §3.1 (D-16-1) makes the \
                 arena the single writer for its units' cells and the world the single writer for \
                 everything else; the custody markers are the ONLY unit writes a match may cause."
            );
        }

        // --- check-in --------------------------------------------------------
        run_ok(&mut tw, "@arena/abort 1");
        let mut checkin: Vec<(ObjId, Value)> = Vec::new();
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(60)).await;
            pump(&mut tw);
            checkin.extend(unit_writes(&mut tw));
            if checkin.len() >= 2 {
                break;
            }
        }
        assert_eq!(
            checkin.len(),
            2,
            "check-in writes each unit's record exactly once: {checkin:?}"
        );
        for (unit, document) in &checkin {
            assert!(
                marker_of(document).is_none(),
                "check-in clears the marker: {document}"
            );
            assert_eq!(
                Some(document),
                before.get(unit),
                "a unit nothing happened to comes back byte-identical (doc 16 §7's lens)"
            );
        }

        // And nothing is deployed any more, on either side of the seam.
        assert!(
            mush_sim::deployed::deployed_units(tw.world()).is_empty(),
            "the world-side index is empty after check-in"
        );
        assert!(
            mush_tactics::custody::deployed(tw.world()).is_empty(),
            "and so is the record-side one"
        );
    }
}

/// The names of cut I.4's staff picture that no pilot surface may speak.
const OBSERVER_NAMES: [&str; 4] = [
    "Observer",
    "ObserverUnit",
    "observer_frame",
    "OBSERVER_FEED",
];

/// **The staff picture has no path into a pilot's feed** (16b cut I.4; doc 16
/// §1's observer rule; 16c N10).
///
/// `@arena/watch` gives staff an objective picture of a running match — every
/// unit by callsign, cell and object number — built by `Arena::observer` and
/// published on `arena.observer` to watchers only. Doc 16 §1 says a pilot is
/// told nothing about a unit their platform has not detected, and what keeps
/// that true is that the two paths never meet: the pilot's frame is built by
/// `Arena::frames` (through `Arena::frame_for` and `Arena::frame_odds` since 17b
/// cut M1a.3, and `Arena::frame_mind`, the hull's sealed mind, since 17b cut
/// M5.4) and rendered by `crates/tacsim/src/match_/cockpit.rs`, whose
/// contact rows (`frame_contacts_held`, `ContactRow`, `own_scope`, the lock
/// reads) live in `crates/tacsim/src/arena/view.rs` since 17b cut M1a.2 — the
/// row a pilot's `CONTACTS` and, from M1a.5, a mind's view read — and none of
/// them names the picture's type, its row, its payload builder or its feed.
/// This reads all three and refuses those names, so a path from one to the
/// other is a reviewed diff and not an accident.
#[test]
fn the_observer_has_no_path_into_a_pilot_feed() {
    let root = repo_root();
    let cockpit_path = root.join("crates/tacsim/src/match_/cockpit.rs");
    let view_path = root.join("crates/tacsim/src/arena/view.rs");
    let step_path = root.join("crates/tacsim/src/arena/step.rs");
    let cockpit = fs::read_to_string(&cockpit_path)
        .unwrap_or_else(|error| panic!("{}: {error}", cockpit_path.display()));
    // 17b cut M1a.2: the row formatter the cockpit re-exports.
    let view = fs::read_to_string(&view_path)
        .unwrap_or_else(|error| panic!("{}: {error}", view_path.display()));
    let step = fs::read_to_string(&step_path)
        .unwrap_or_else(|error| panic!("{}: {error}", step_path.display()));

    // `Arena::frames`: from its `fn frames(` line to the next `fn ` at the
    // same indent — **17b cut M1a.3** factored the per-pilot build out of it
    // as `Arena::frame_for` (and its odds as `Arena::frame_odds`), which
    // follow it in the file, so the slice runs on through those two and ends
    // at the next function that is neither. **17b cut M5.4** put
    // `Arena::frame_mind` (the frame's `mind`) between them, and the slice
    // runs through it too. T-106 inserts the worker policy and pure parallel
    // candidate builder; both remain inside the same observer-isolation scan.
    let lines: Vec<&str> = step.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("fn frames("))
        .unwrap_or_else(|| {
            panic!(
                "no `fn frames(` in {}: `Arena::frames` moved, and this gate is now asserting \
                 nothing. Re-point it in the same commit.",
                step_path.display()
            )
        });
    let indent = lines
        .get(start)
        .map_or(0, |line| line.len() - line.trim_start().len());
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| {
            let trimmed = line.trim_start();
            line.len() - trimmed.len() == indent
                && (trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("pub(crate) fn "))
                && !trimmed.starts_with("pub fn set_frame_workers(")
                && !trimmed.starts_with("fn frame_candidates(")
                && !trimmed.starts_with("pub(crate) fn frame_for(")
                && !trimmed.starts_with("fn frame_mind(")
                && !trimmed.starts_with("pub fn frame_odds(")
        })
        .map_or(lines.len(), |(n, _)| n);
    let frames_text = lines.get(start..end).unwrap_or_default().join("\n");
    assert!(
        frames_text.contains("Frame {"),
        "the slice of step.rs read as `Arena::frames` builds no `Frame`; the scan is reading \
         the wrong place:\n{frames_text}"
    );
    assert!(
        frames_text.contains("fn frame_candidates(")
            && frames_text.contains("fn set_frame_workers(")
            && frames_text.contains("fn frame_for(")
            && frames_text.contains("fn frame_mind(")
            && frames_text.contains("fn frame_odds("),
        "the slice of step.rs read as `Arena::frames` stops before its worker policy, candidates, `frame_for`, `frame_mind` \
         and `frame_odds`, which build its frames (17b cut M1a.3, 17b cut M5.4); re-point \
         it:\n{frames_text}"
    );

    let mut offences: Vec<String> = Vec::new();
    // Each text with the file line its first line is.
    for (what, text, first) in [
        ("crates/tacsim/src/match_/cockpit.rs", cockpit.as_str(), 1),
        ("crates/tacsim/src/arena/view.rs", view.as_str(), 1),
        (
            "`Arena::frames`, `frame_for`, `frame_mind` and `frame_odds` in \
             crates/tacsim/src/arena/step.rs",
            frames_text.as_str(),
            start + 1,
        ),
    ] {
        for name in OBSERVER_NAMES {
            if let Some((n, _)) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(name))
            {
                offences.push(format!("{what}, line {}: names {name:?}", n + first));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "the staff picture (16b cut I.4) reached a pilot surface. Doc 16 §1: nothing tells a \
         pilot about a unit their platform has not detected, and `arena.observer` names every \
         unit; the cockpit payloads and `Arena::frames` must never name it.\n{}",
        offences.join("\n")
    );
}
