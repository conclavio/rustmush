//! **The unit design registry** (doc 31 cut UD.3, D-31-9): a design published
//! over the `unit_design` invoke is a registry row; the same version is never
//! published twice; `@design` lists, shows and withdraws; and a restart that
//! hands the rows' packs in beside `[tacsim].pack_paths`
//! (`tacsim::design::design_sources`, what the host does) loads the design into
//! the catalogue the boot freezes.

use std::collections::{BTreeMap, HashMap};

use mush_core::engine::{Effects, Inbound};
use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, Outbox};
use mush_proto::{InvokeRequest, InvokeResult, OutputEvent, UnitDesignRequest, UnitDesignResponse};
use mush_testkit::{TestWorld, text_of};
use serde_json::Value;
use smol_str::SmolStr;
use tacsim::{TacsimConfig, TacsimRuleset, TacsimStore};

const CONN: u64 = 931;
const ID: &str = "tacsim.unit.design.laser_tank";

fn boot(cfg: TacsimConfig) -> TestWorld {
    let mut tw = TestWorld::with_modules(vec![Box::new(TacsimRuleset::with_config(cfg))]);
    let mut out = Outbox::new();
    tw.engine_mut().step(
        Inbound::ConnUp {
            conn: CONN,
            actor: GOD,
            caps_structured: true,
            from_linkdead: false,
        },
        &mut out,
        &mut Effects::new(),
    );
    tw
}

fn enabled() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

/// The shipped laser tank's rules and look, for a design to start from.
fn laser() -> (Value, Value) {
    let pack: Value =
        serde_json::from_str(include_str!("../../content/units/line_tank_laser.json"))
            .expect("JSON");
    let rules = pack["entries"]["tacsim.unit.line_tank_laser"]["def"].clone();
    let mut look: Value =
        serde_json::from_str(include_str!("../../../mush-pixel/looks/tracked_tank.json"))
            .expect("JSON");
    look["units"] = serde_json::json!([ID]);
    (rules, look)
}

fn publish(tw: &mut TestWorld, version: u32) -> UnitDesignResponse {
    let (rules, look) = laser();
    let mut out = Outbox::new();
    tw.engine_mut().step(
        Inbound::Invoke {
            conn: CONN,
            actor: GOD,
            id: 77,
            request: InvokeRequest::UnitDesign(UnitDesignRequest::Publish {
                id: ID.into(),
                version,
                rules,
                look,
            }),
        },
        &mut out,
        &mut Effects::new(),
    );
    out.take()
        .into_iter()
        .find_map(|(_, event)| match event {
            OutputEvent::InvokeResult {
                id: 77,
                result: InvokeResult::UnitDesign(response),
            } => Some(response),
            _ => None,
        })
        .expect("the designer is answered")
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn snap(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the tacsim slot")
}

/// Every TacSim document written since the world was built.
fn rows(tw: &mut TestWorld) -> Vec<(SmolStr, Value)> {
    let mut kv: BTreeMap<SmolStr, Value> = BTreeMap::new();
    for op in tw.take_module_ops() {
        if op.ns != tacsim::NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.insert(key, value);
            }
            ModuleWrite::Del { key } => {
                kv.remove(&key);
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv.into_iter().collect()
}

#[test]
fn ud3_a_published_design_is_kept_never_republished_and_loads_after_a_restart() {
    let mut tw = boot(enabled());
    match publish(&mut tw, 1) {
        UnitDesignResponse::Published { refusals, .. } => {
            assert_eq!(refusals, Vec::<String>::new());
        }
        other => panic!("not published: {other:?}"),
    }
    assert_eq!(snap(&tw).designs().count(), 1);
    match publish(&mut tw, 1) {
        UnitDesignResponse::Published { refusals, .. } => {
            assert!(
                refusals[0].contains("already published"),
                "a version never changes: {refusals:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    let listed = texts(&tw.run(GOD, "@design/list"));
    assert!(listed.contains(&format!("{ID}@1")), "{listed}");
    let shown = texts(&tw.run(GOD, &format!("@design/show {ID}")));
    assert!(shown.contains("tacsim.pack.design.laser_tank"), "{shown}");
    assert!(
        shown.contains("tacsim.pack.laser_mk1@1"),
        "its requires: {shown}"
    );

    // The restart: the host reads the rows and hands their packs in.
    let docs = rows(&mut tw);
    let plain: Vec<(String, Value)> = docs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let sources = tacsim::design::design_sources(&plain);
    assert_eq!(sources.len(), 1);
    let cfg = TacsimConfig {
        packs: sources,
        ..enabled()
    };
    let boot_data: HashMap<SmolStr, ModuleBootData> = HashMap::from([(
        SmolStr::new(tacsim::NS),
        ModuleBootData {
            kv: docs,
            ..Default::default()
        },
    )]);
    let mut tw = tw
        .reboot_into(vec![Box::new(TacsimRuleset::with_config(cfg))], boot_data)
        .expect("the restart loads the design");
    assert!(
        snap(&tw)
            .definitions()
            .specs
            .keys()
            .any(|id| id.as_str() == ID),
        "the design is in the catalogue the boot froze"
    );
    assert_eq!(snap(&tw).designs().count(), 1, "the registry row hydrated");
    assert!(!snap(&tw).design_withdrawn(ID));

    // Withdrawn: still loaded, no longer offered.
    let withdrawn = texts(&tw.run(GOD, &format!("@design/withdraw {ID}")));
    assert!(withdrawn.contains("Withdrawn"), "{withdrawn}");
    assert!(snap(&tw).design_withdrawn(ID));
    assert!(
        snap(&tw)
            .definitions()
            .specs
            .keys()
            .any(|id| id.as_str() == ID)
    );
}

#[test]
fn ud3_a_published_design_takes_the_design_namespace() {
    let mut tw = boot(enabled());
    let (rules, look) = laser();
    let mut out = Outbox::new();
    tw.engine_mut().step(
        Inbound::Invoke {
            conn: CONN,
            actor: GOD,
            id: 78,
            request: InvokeRequest::UnitDesign(UnitDesignRequest::Publish {
                id: "tacsim.unit.laser_tank_mk2".into(),
                version: 1,
                rules,
                look,
            }),
        },
        &mut out,
        &mut Effects::new(),
    );
    let refused = out.take().into_iter().any(|(_, e)| {
        matches!(e, OutputEvent::InvokeResult {
            result: InvokeResult::UnitDesign(UnitDesignResponse::Published { refusals, .. }), ..
        } if refusals.iter().any(|r| r.contains("tacsim.unit.design.")))
    });
    assert!(refused, "an id outside the namespace is refused");
    assert_eq!(snap(&tw).designs().count(), 0);
}

#[test]
fn ud3_the_registry_is_staff_only() {
    let mut tw = boot(enabled());
    let player = tw.spawn_player("Rigger");
    let refused = texts(&tw.run(player, "@design/list"));
    assert!(refused.contains("Permission denied."), "{refused}");
}
