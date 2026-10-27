//! The validator (doc 14 §4.3; S3): the eight rules as typed findings over a pack
//! and a captured context, the starter packs validating clean, and `@atlas/pack
//! plan|apply` reading the same findings.

use mush_atlas::{
    AtlasModule, AtlasState, Finding, Severity, ValidateContext, ValidateScope, blocking, validate,
};
use mush_content::{Appliers, ContentModule, ContentState, Pack, parse_pack};
use mush_core::{Audience, GOD, GameModule, ObjKind};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn signed(mut doc: Value) -> Pack {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn region() -> Pack {
    signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000,
                "theme": { "name": "Plain", "banned": ["neon"] } } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } },
            "checkpoint.gate": { "kind": "checkpoint", "def": {
                "region": "region.town", "name": "The Gate", "point": [150, 10, 0] } },
            "connection.road": { "kind": "connection", "def": {
                "region": "region.town", "name": "The Road", "kind": "journey",
                "from": { "neighborhood": "neighborhood.block", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.far", "point": [300, 300, 0] },
                "direction": "both", "modes": ["foot"], "corridor": [[30, 30, 0], [300, 300, 0]] } }
        }
    }))
}

/// A neighborhood: a square, a public room at the access point (the Green), a
/// Parlour on a house site joined to it, and a restricted Vault behind a locked door.
fn block_doc() -> Value {
    json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 5 },
                "hydration": { "enabled": true, "max_player_sites": 1 } } },
            "site.house": { "kind": "site", "def": { "name": "The House", "site_kind": "site.house",
                "footprint": [[60, 10], [120, 10], [120, 70], [60, 70]] } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[10, 10], [60, 60]] }, "desc": "A wide green." } },
            "room.parlour": { "kind": "room", "def": { "name": "Parlour", "surface": "ground",
                "site": "site.house", "footprint": { "rect": [[60, 10], [110, 60]] } } },
            "room.vault": { "kind": "room", "def": { "name": "Vault", "surface": "ground",
                "site": "site.house", "footprint": { "rect": [[110, 10], [120, 60]] },
                "access": "restricted" } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } },
            "exit.green_parlour": { "kind": "exit", "def": { "from": "room.green", "to": "room.parlour",
                "names": ["House <H>;h;house", "Green <G>;g;green"] } },
            "exit.parlour_vault": { "kind": "exit", "def": { "from": "room.parlour", "to": "room.vault",
                "names": ["Vault <V>;v;vault", "Parlour <P>;parlour"], "lock": "staff" } }
        }
    })
}

fn block() -> Pack {
    signed(block_doc())
}

fn modules(shelf: Vec<Pack>, boot: Vec<Pack>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new().with_shelf(shelf)),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(boot),
        ),
    ]
}

/// A world with the region and the block applied.
fn town() -> TestWorld {
    TestWorld::with_modules(modules(Vec::new(), vec![region(), block()]))
}

fn context(tw: &TestWorld, pack: &Pack) -> ValidateContext {
    let w = tw.world();
    let state = w.module_state::<AtlasState>().expect("atlas");
    let content = w.module_state::<ContentState>().expect("content");
    ValidateContext::for_pack(w, state, content, pack)
}

fn codes(findings: &[Finding], severity: Severity) -> Vec<String> {
    findings
        .iter()
        .filter(|f| f.severity == severity)
        .map(|f| f.code.clone())
        .collect()
}

fn errors(findings: &[Finding]) -> Vec<String> {
    codes(findings, Severity::Error)
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)]) -> String {
    tw.seen_by(events, GOD).join("\n")
}

/// The block at version 2 with `entries` edited by `edit`.
fn block_v2(edit: impl FnOnce(&mut Value)) -> Pack {
    let mut doc = block_doc();
    doc["manifest"]["version"] = json!(2);
    edit(&mut doc["entries"]);
    signed(doc)
}

#[test]
fn the_starter_packs_validate_clean() {
    let starter = mush_atlas::starter::packs().expect("starter packs");
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let tw = TestWorld::with_modules(vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter.clone())),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(starter.clone()),
        ),
    ]);
    for pack in &starter {
        let findings = validate(pack, &context(&tw, pack));
        assert_eq!(
            blocking(&findings),
            0,
            "{} blocks: {:#?}",
            pack.manifest().id(),
            findings
                .iter()
                .filter(|f| f.blocks())
                .map(Finding::line)
                .collect::<Vec<_>>()
        );
        assert!(
            codes(&findings, Severity::Warning).is_empty(),
            "{} warns: {:#?}",
            pack.manifest().id(),
            findings
        );
    }
    // The context that was captured is serializable: the agent's off-loop run
    // carries it.
    let lot = starter.get(1).expect("the OOC Area");
    let cx = context(&tw, lot);
    assert!(matches!(cx.scope, Some(ValidateScope::Region { .. })));
    let wire = serde_json::to_string(&cx).expect("serializes");
    let back: ValidateContext = serde_json::from_str(&wire).expect("deserializes");
    assert_eq!(back, cx);
}

#[test]
fn the_block_validates_clean_and_the_schema_rules_refuse_what_does_not_parse() {
    let tw = town();
    let findings = validate(&block(), &context(&tw, &block()));
    assert_eq!(blocking(&findings), 0, "{findings:#?}");
    assert!(
        codes(&findings, Severity::Warning).is_empty(),
        "{findings:#?}"
    );

    // A kind nobody owns, a room that does not decode, a reference to nothing, an
    // exit that joins a room to itself.
    let pack = block_v2(|e| {
        e["thing.odd"] = json!({ "kind": "gizmo", "def": {} });
        e["room.bad"] = json!({ "kind": "room", "def": { "name": "Bad" } });
        e["room.green"]["def"]["site"] = json!("site.nowhere");
        e["exit.loop"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.green",
            "names": ["Loop <X>;x"] } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    let errs = errors(&findings);
    assert!(errs.contains(&"schema.kind".to_string()), "{findings:#?}");
    assert!(errs.contains(&"schema.decode".to_string()), "{findings:#?}");
    assert!(
        findings
            .iter()
            .any(|f| f.code == "schema.reference" && f.entry.as_deref() == Some("room.green")),
        "{findings:#?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.code == "schema.reference" && f.entry.as_deref() == Some("exit.loop")),
        "{findings:#?}"
    );

    // A requirement nothing loaded satisfies.
    let pack = {
        let mut doc = block_doc();
        doc["manifest"]["version"] = json!(2);
        doc["manifest"]["requires"] = json!([{ "id": "region.town", "version": 7 }]);
        signed(doc)
    };
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        errors(&findings).contains(&"schema.requires".to_string()),
        "{findings:#?}"
    );
}

fn visiting_npc_doc() -> Value {
    json!({
        "manifest": { "id": "visitors.town", "version": 1, "kind": "campaign_seed", "digest": "",
                      "requires": [{ "id": "neighborhood.block", "version": 1 }] },
        "entries": {
            "npc.visitor": { "kind": "npc", "def": {
                "name": "Visitor", "room_pack": "neighborhood.block", "room": "room.green"
            } }
        }
    })
}

#[test]
fn npc_room_pack_resolves_declared_external_rooms_outside_the_geometry_scope() {
    let tw = town();
    let pack = signed(visiting_npc_doc());
    let cx = context(&tw, &pack);
    assert!(matches!(cx.scope, Some(ValidateScope::World)));
    assert!(
        cx.rooms.is_empty(),
        "campaign validation carries no regional room geometry"
    );
    let findings = validate(&pack, &cx);
    assert_eq!(blocking(&findings), 0, "{findings:#?}");
}

#[test]
fn npc_room_pack_requires_the_exact_declared_pack_and_applied_room_entry() {
    let tw = town();
    for (name, room_pack, room, requires) in [
        (
            "missing dependency",
            json!("neighborhood.block"),
            "room.green",
            json!([]),
        ),
        (
            "wrong declared pack",
            json!("region.town"),
            "room.green",
            json!([{ "id": "neighborhood.block", "version": 1 }, { "id": "region.town", "version": 1 }]),
        ),
        (
            "missing room",
            json!("neighborhood.block"),
            "room.missing",
            json!([{ "id": "neighborhood.block", "version": 1 }]),
        ),
        (
            "not a room",
            json!("neighborhood.block"),
            "site.house",
            json!([{ "id": "neighborhood.block", "version": 1 }]),
        ),
        (
            "prefixed instead of exact entry",
            json!("neighborhood.block"),
            "neighborhood.block.room.green",
            json!([{ "id": "neighborhood.block", "version": 1 }]),
        ),
        (
            "wrong dependency version",
            json!("neighborhood.block"),
            "room.green",
            json!([{ "id": "neighborhood.block", "version": 2 }]),
        ),
        (
            "not a pack string",
            json!(42),
            "room.green",
            json!([{ "id": "neighborhood.block", "version": 1 }]),
        ),
    ] {
        let mut doc = visiting_npc_doc();
        doc["manifest"]["requires"] = requires;
        doc["entries"]["npc.visitor"]["def"]["room_pack"] = room_pack;
        doc["entries"]["npc.visitor"]["def"]["room"] = json!(room);
        let pack = signed(doc);
        let findings = validate(&pack, &context(&tw, &pack));
        assert!(
            findings.iter().any(|f| f.blocks()
                && f.entry.as_deref() == Some("npc.visitor")
                && matches!(f.code.as_str(), "schema.reference" | "schema.decode")),
            "{name}: {findings:#?}"
        );
    }

    let pack = signed(visiting_npc_doc());
    for retired in [false, true] {
        let mut cx = context(&tw, &pack);
        let latch = cx
            .latches
            .get_mut("neighborhood.block")
            .expect("the applied block");
        if retired {
            latch.retired = true;
        } else {
            latch
                .entries
                .get_mut("room.green")
                .expect("the green")
                .object = None;
        }
        let findings = validate(&pack, &cx);
        assert!(
            findings.iter().any(|f| f.blocks()
                && f.code == "schema.reference"
                && f.entry.as_deref() == Some("npc.visitor")),
            "retired={retired}: {findings:#?}"
        );
    }
}

#[test]
fn room_pack_does_not_bypass_non_npc_definition_schemas() {
    let tw = town();
    let mut doc = visiting_npc_doc();
    doc["entries"] = json!({ "item.bag": { "kind": "item", "def": {
        "name": "Bag", "room_pack": "neighborhood.block", "room": "room.green"
    } } });
    let pack = signed(doc);
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        findings.iter().any(|f| f.blocks()
            && f.code == "schema.decode"
            && f.entry.as_deref() == Some("item.bag")),
        "{findings:#?}"
    );
}

#[test]
fn geometry_refuses_footprints_that_leave_overlap_or_do_not_touch() {
    let tw = town();
    // A room past the neighborhood's edge, a room over another, and an exit
    // between rooms that do not touch with no walk given.
    let pack = block_v2(|e| {
        e["room.beyond"] = json!({ "kind": "room", "def": { "name": "Beyond", "surface": "ground",
            "footprint": { "rect": [[190, 190], [230, 230]] } } });
        e["room.over"] = json!({ "kind": "room", "def": { "name": "Over", "surface": "ground",
            "footprint": { "rect": [[20, 20], [40, 40]] } } });
        e["room.far"] = json!({ "kind": "room", "def": { "name": "Far", "surface": "ground",
            "footprint": { "rect": [[150, 150], [190, 190]] } } });
        e["exit.green_far"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.far",
            "names": ["Far <F>;f", "Back <B>;b"] } });
        e["exit.green_beyond"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.beyond",
            "names": ["Beyond <Y>;y", "Back <K>;k"], "portal": { "distance_m": 200 } } });
        e["exit.green_over"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.over",
            "names": ["Over <Q>;q", "Back <W>;w"], "portal": { "distance_m": 5 } } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    let find = |code: &str, entry: &str| {
        findings
            .iter()
            .any(|f| f.code == code && f.entry.as_deref() == Some(entry) && f.blocks())
    };
    assert!(find("geometry.outside", "room.beyond"), "{findings:#?}");
    assert!(find("geometry.room_overlap", "room.over"), "{findings:#?}");
    assert!(
        find("geometry.portal_gap", "exit.green_far"),
        "{findings:#?}"
    );
    assert!(
        !find("geometry.portal_gap", "exit.green_beyond"),
        "{findings:#?}"
    );
    // The budget: five rooms is the maximum; six is not.
    assert!(
        errors(&findings).contains(&"budget.rooms".to_string()),
        "{findings:#?}"
    );

    // A site over the house, and a structure whose floors run backwards.
    let pack = block_v2(|e| {
        e["site.shed"] = json!({ "kind": "site", "def": { "name": "The Shed", "site_kind": "site.shed",
            "footprint": [[100, 50], [140, 50], [140, 90], [100, 90]] } });
        e["structure.house"] = json!({ "kind": "structure", "def": { "site": "site.house",
            "footprint": [[60, 10], [120, 10], [120, 70], [60, 70]],
            "floors": [{ "index": 1, "z_offset_m": 3.0 }, { "index": 0, "z_offset_m": 0.0 }] } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        findings
            .iter()
            .any(|f| f.code == "geometry.site_overlap" && f.entry.as_deref() == Some("site.shed")),
        "{findings:#?}"
    );
    assert!(
        errors(&findings).contains(&"geometry.floors".to_string()),
        "{findings:#?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.code == "policy.kind" && f.severity == Severity::Warning),
        "{findings:#?}"
    );
}

#[test]
fn topology_wants_every_public_room_reached_and_private_ways_locked() {
    let tw = town();
    let pack = block_v2(|e| {
        // An island: a public room no exit reaches.
        e["room.island"] = json!({ "kind": "room", "def": { "name": "Island", "surface": "ground",
            "footprint": { "rect": [[150, 150], [190, 190]] } } });
        // A cul-de-sac reached only through the locked vault.
        e["room.cellar"] = json!({ "kind": "room", "def": { "name": "Cellar", "surface": "ground",
            "footprint": { "rect": [[110, 60], [120, 70]] } } });
        e["exit.vault_cellar"] = json!({ "kind": "exit", "def": { "from": "room.vault", "to": "room.cellar",
            "names": ["Down <D>;d", "Up <U>;u"] } });
        // The vault's door loses its lock.
        e["exit.parlour_vault"]["def"]
            .as_object_mut()
            .unwrap()
            .remove("lock");
    });
    let findings = validate(&pack, &context(&tw, &pack));
    let find = |code: &str, entry: &str| {
        findings
            .iter()
            .any(|f| f.code == code && f.entry.as_deref() == Some(entry) && f.blocks())
    };
    assert!(find("topology.isolated", "room.island"), "{findings:#?}");
    assert!(
        find("topology.private_ingress", "exit.parlour_vault"),
        "{findings:#?}"
    );
    // With the lock gone the cellar is reachable through the vault; put the lock
    // back and it is not.
    assert!(
        !find("topology.unreachable", "room.cellar"),
        "{findings:#?}"
    );
    let pack = block_v2(|e| {
        e["room.cellar"] = json!({ "kind": "room", "def": { "name": "Cellar", "surface": "ground",
            "footprint": { "rect": [[110, 60], [120, 70]] } } });
        e["exit.vault_cellar"] = json!({ "kind": "exit", "def": { "from": "room.vault", "to": "room.cellar",
            "names": ["Down <D>;d", "Up <U>;u"] } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        findings
            .iter()
            .any(|f| f.code == "topology.unreachable" && f.entry.as_deref() == Some("room.cellar")),
        "{findings:#?}"
    );

    // An anchor (a landing) may stand alone.
    let pack = block_v2(|e| {
        e["room.landing"] = json!({ "kind": "room", "def": { "name": "Landing", "surface": "ground",
            "footprint": { "anchor": { "point": [170, 170], "radius_m": 5 } } } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert_eq!(blocking(&findings), 0, "{findings:#?}");
}

#[test]
fn policy_and_secrets_and_the_theme() {
    let tw = town();
    // A player's site pack in the block: within policy, it validates; a second
    // player site is over the neighborhood's allowance; an exit into the vault is a
    // shortcut through private space.
    let site = |name: &str, x0: f64, into_vault: bool| {
        let mut entries = json!({
            "site.shed": { "kind": "site", "def": { "name": name, "site_kind": "site.garden",
                "footprint": [[x0, 100], [x0 + 40.0, 100], [x0 + 40.0, 140], [x0, 140]] } },
            "room.shed": { "kind": "room", "def": { "name": name, "surface": "ground",
                "site": "site.shed", "footprint": { "rect": [[x0, 100], [x0 + 40.0, 140]] },
                "desc": "A shed under a neon sign." } },
            "exit.green_shed": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
                "to": "room.shed", "names": ["Shed <S>;s", "Green <G>;g"],
                "portal": { "distance_m": 30 } } }
        });
        if into_vault {
            entries["exit.shed_vault"] = json!({ "kind": "exit", "def": { "from": "room.shed",
                "to": "neighborhood.block.room.vault", "names": ["Vault <V>;v", "Shed <S>;s"],
                "portal": { "distance_m": 40 } } });
        }
        signed(json!({
            "manifest": { "id": "site.shed_pack", "version": 1, "kind": "site", "digest": "",
                          "requires": [{ "id": "region.town", "version": 1 }, { "id": "neighborhood.block", "version": 1 }] },
            "entries": entries
        }))
    };
    let pack = site("The Shed", 20.0, false);
    let mut cx = context(&tw, &pack);
    assert!(matches!(cx.scope, Some(ValidateScope::Neighborhood { .. })));
    cx.player_build = true;
    let findings = validate(&pack, &cx);
    assert_eq!(blocking(&findings), 0, "{findings:#?}");
    // The theme bans "neon": a warning, not a refusal.
    assert!(
        findings
            .iter()
            .any(|f| f.code == "policy.banned_word" && f.severity == Severity::Warning),
        "{findings:#?}"
    );

    // Policy: the neighborhood is closed, then full, then the shortcut.
    let mut closed = cx.clone();
    for nb in &mut closed.neighborhoods {
        nb.hydration.enabled = false;
    }
    assert!(errors(&validate(&pack, &closed)).contains(&"policy.hydration".to_string()));
    let mut full = cx.clone();
    for nb in &mut full.neighborhoods {
        nb.player_sites = 1;
    }
    assert!(errors(&validate(&pack, &full)).contains(&"policy.player_sites".to_string()));
    let pack = site("The Shed", 20.0, true);
    let mut cx = context(&tw, &pack);
    cx.player_build = true;
    assert!(errors(&validate(&pack, &cx)).contains(&"policy.shortcut".to_string()));
    // Staff are not bound by the neighborhood's player policy.
    cx.player_build = false;
    let findings = validate(&pack, &cx);
    assert!(
        !errors(&findings).contains(&"policy.shortcut".to_string()),
        "{findings:#?}"
    );

    // Secrets: a truth in an NPC's persona is refused; in the secret entry alone it
    // is a note.
    let pack = block_v2(|e| {
        e["secret.will"] =
            json!({ "kind": "secret", "def": { "truth": "The will names the gardener." } });
        e["npc.aunt"] = json!({ "kind": "npc", "def": { "name": "Aunt May", "room": "room.parlour",
            "profile": { "persona": "Kindly; she knows the will names the gardener." } } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        findings.iter().any(|f| f.code == "secrets.leak"
            && f.entry.as_deref() == Some("npc.aunt")
            && f.blocks()),
        "{findings:#?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.code == "secrets.staff_only" && f.severity == Severity::Info),
        "{findings:#?}"
    );
    let pack = block_v2(|e| {
        e["secret.will"] =
            json!({ "kind": "secret", "def": { "truth": "The will names the gardener." } });
        e["npc.aunt"] = json!({ "kind": "npc", "def": { "name": "Aunt May", "room": "room.parlour",
            "profile": { "persona": "Kindly and vague about the family." } } });
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        !errors(&findings).contains(&"secrets.leak".to_string()),
        "{findings:#?}"
    );
}

#[test]
fn diff_safety_keeps_a_room_someone_stands_in_and_refuses_a_downgrade() {
    let mut tw = town();
    let parlour = tw
        .world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Parlour")
        .expect("the parlour")
        .id;
    let player = tw.spawn_player("Rae");
    tw.world_mut().move_to(player, parlour).expect("moves");
    // Version 2 drops the parlour (and the vault behind it, and their exits).
    let pack = block_v2(|e| {
        let map = e.as_object_mut().unwrap();
        for key in [
            "room.parlour",
            "room.vault",
            "exit.green_parlour",
            "exit.parlour_vault",
        ] {
            map.remove(key);
        }
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        findings.iter().any(|f| f.code == "diff.live_reference"
            && f.entry.as_deref() == Some("room.parlour")
            && f.message.contains("1 character")),
        "{findings:#?}"
    );
    // With version 2 applied, version 1 is a downgrade.
    let tw = TestWorld::with_modules(modules(Vec::new(), vec![region(), block_v2(|_| {})]));
    let pack = block();
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        errors(&findings).contains(&"diff.downgrade".to_string()),
        "{findings:#?}"
    );
}

#[test]
fn the_plan_prints_the_findings_and_apply_refuses_a_blocking_one() {
    let bad = block_v2(|e| {
        e["room.beyond"] = json!({ "kind": "room", "def": { "name": "Beyond", "surface": "ground",
            "footprint": { "rect": [[190, 190], [230, 230]] } } });
        e["exit.green_beyond"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.beyond",
            "names": ["Beyond <Y>;y", "Back <K>;k"], "portal": { "distance_m": 200 } } });
    });
    let mut tw = TestWorld::with_modules(modules(vec![bad], vec![region(), block()]));
    let events = tw.run(GOD, "@atlas/pack plan neighborhood.block");
    let out = text(&tw, &events);
    assert!(out.contains("blocking"), "{out}");
    assert!(
        out.contains(
            "[error] room.beyond: its footprint leaves the neighborhood (geometry.outside)"
        ),
        "{out}"
    );
    assert!(out.contains("Fix the blocking findings"), "{out}");
    assert!(!out.contains("@atlas/pack apply"), "{out}");
    // Apply by any fingerprint refuses on the finding, never on the fingerprint.
    let events = tw.run(GOD, "@atlas/pack apply neighborhood.block 0000000000000000");
    let out = text(&tw, &events);
    assert!(out.contains("blocking finding"), "{out}");
    let rooms = tw
        .world()
        .iter()
        .filter(|o| o.kind == ObjKind::Room && o.name == "Beyond")
        .count();
    assert_eq!(rooms, 0, "nothing was applied");
}

// ---- doc 18 cut IC1: room markers (D-18-19) ------------------------------------

/// A neighborhood east of the block whose two public rooms are joined to each
/// other and to nothing reachable — no Limbo exit, no access point — with
/// `markers` on the Gallery. The shape of Stickney Yards (no foot route).
fn dock_doc(markers: Value) -> Value {
    json!({
        "manifest": { "id": "neighborhood.dock", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Dock",
                "setting": "station", "footprint": [[220, 0], [400, 0], [400, 200], [220, 200]],
                "room_budget": { "target": 2, "maximum": 2 } } },
            "room.gallery": { "kind": "room", "def": { "name": "Gallery", "surface": "ground",
                "kind": "indoor", "footprint": { "rect": [[230, 10], [260, 40]] },
                "markers": markers } },
            "room.hall": { "kind": "room", "def": { "name": "Hall", "surface": "ground",
                "kind": "indoor", "footprint": { "rect": [[260, 10], [290, 40]] } } },
            "exit.gallery_hall": { "kind": "exit", "def": { "from": "room.gallery", "to": "room.hall",
                "names": ["Hall <H>;h;hall", "Gallery <G>;g;gallery"] } }
        }
    })
}

#[test]
fn ic1_a_port_dock_marker_seeds_the_topology() {
    let tw = town();
    // Without the marker: joined to each other, reached from nothing.
    let bare = signed(dock_doc(json!([])));
    let findings = validate(&bare, &context(&tw, &bare));
    let unreachable: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "topology.unreachable")
        .filter_map(|f| f.entry.clone())
        .collect();
    assert_eq!(
        unreachable,
        vec!["room.gallery".to_string(), "room.hall".to_string()],
        "{findings:#?}"
    );
    // With `port.dock` on the Gallery: the Gallery is a seed, the Hall is reached
    // through it, and the pack validates with zero errors.
    let docked = signed(dock_doc(json!(["port.dock"])));
    let findings = validate(&docked, &context(&tw, &docked));
    assert!(errors(&findings).is_empty(), "{findings:#?}");
}

#[test]
fn ic1_an_unknown_room_marker_is_refused() {
    // The validator: `schema.marker`, for a word outside the registry and for a
    // marker named twice.
    let tw = town();
    let typo = signed(dock_doc(json!(["port.dok"])));
    let findings = validate(&typo, &context(&tw, &typo));
    assert!(
        errors(&findings).contains(&"schema.marker".to_string()),
        "{findings:#?}"
    );
    let twice = signed(dock_doc(json!(["port.dock", "port.dock"])));
    let findings = validate(&twice, &context(&tw, &twice));
    assert!(
        errors(&findings).contains(&"schema.marker".to_string()),
        "{findings:#?}"
    );

    // The applier: a boot pack bypasses the validator, so this is the applier's
    // own refusal — the boot aborts naming the marker, and no Gallery was made.
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(
        world,
        registry,
        modules(Vec::new(), vec![region(), block(), typo]),
    )
    .expect("the modules register");
    let error = engine
        .modules_loaded(
            std::collections::HashMap::new(),
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        )
        .expect_err("an unknown marker refuses the apply");
    assert!(
        error
            .to_string()
            .contains("unknown room marker \"port.dok\""),
        "{error}"
    );
    let galleries = engine
        .world
        .iter()
        .filter(|o| o.kind == ObjKind::Room && o.name == "Gallery")
        .count();
    assert_eq!(galleries, 0, "the refused room was never written");
}

// ---- doc 18 cut IC12: the two text bounds the applier enforced alone ----------

/// The `budget.text` findings about `entry`, as `(message, fix_hint)`.
fn text_budget(findings: &[Finding], entry: &str) -> Vec<(String, String)> {
    findings
        .iter()
        .filter(|f| f.code == "budget.text" && f.entry.as_deref() == Some(entry) && f.blocks())
        .map(|f| (f.message.clone(), f.fix_hint.clone()))
        .collect()
}

/// **A room's `short_desc` over 256 bytes fails validation** (doc 18 cut IC12 item
/// 4.4; `def::MAX_SHORT_DESC_BYTES`), not only the apply; 256 bytes passes.
#[test]
fn ic12_a_long_short_desc_is_a_budget_text_error() {
    let tw = town();
    let pack = block_v2(|e| {
        e["room.green"]["def"]["short_desc"] = json!("x".repeat(257));
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert_eq!(
        text_budget(&findings, "room.green"),
        vec![(
            "short_desc is 257 bytes; at most 256".to_owned(),
            "one line".to_owned()
        )],
        "{findings:#?}"
    );
    let pack = block_v2(|e| {
        e["room.green"]["def"]["short_desc"] = json!("x".repeat(256));
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        text_budget(&findings, "room.green").is_empty(),
        "{findings:#?}"
    );
}

/// **Too many ambience lines fail validation** (doc 18 cut IC12 item 4.4;
/// `def::MAX_AMBIENCE_LINES` 24); 24 lines pass.
#[test]
fn ic12_too_many_ambience_lines_is_a_budget_text_error() {
    let tw = town();
    let lines = |n: usize| -> Vec<String> { (1..=n).map(|i| format!("A bell, {i}.")).collect() };
    let pack = block_v2(|e| {
        e["neighborhood"]["def"]["ambience"] = json!(lines(25));
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert_eq!(
        text_budget(&findings, "neighborhood"),
        vec![(
            "ambience has 25 lines; at most 24".to_owned(),
            "fewer ambient lines".to_owned()
        )],
        "{findings:#?}"
    );
    let pack = block_v2(|e| {
        e["neighborhood"]["def"]["ambience"] = json!(lines(24));
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        text_budget(&findings, "neighborhood").is_empty(),
        "{findings:#?}"
    );
}

/// **A blank ambience line fails validation** (doc 18 cut IC12 item 4.4).
#[test]
fn ic12_a_blank_ambience_line_is_a_budget_text_error() {
    let tw = town();
    let pack = block_v2(|e| {
        e["neighborhood"]["def"]["ambience"] = json!(["A bell.", "   "]);
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert_eq!(
        text_budget(&findings, "neighborhood"),
        vec![(
            "ambience line 2 is blank".to_owned(),
            "remove the blank line".to_owned()
        )],
        "{findings:#?}"
    );
}

/// **An ambience line over 512 bytes fails validation** (doc 18 cut IC12 item 4.4;
/// `def::MAX_AMBIENCE_LINE_BYTES`); 512 bytes passes.
#[test]
fn ic12_a_long_ambience_line_is_a_budget_text_error() {
    let tw = town();
    let pack = block_v2(|e| {
        e["neighborhood"]["def"]["ambience"] = json!(["y".repeat(513)]);
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert_eq!(
        text_budget(&findings, "neighborhood"),
        vec![(
            "ambience line 1 is 513 bytes; at most 512".to_owned(),
            "a shorter line".to_owned()
        )],
        "{findings:#?}"
    );
    let pack = block_v2(|e| {
        e["neighborhood"]["def"]["ambience"] = json!(["y".repeat(512)]);
    });
    let findings = validate(&pack, &context(&tw, &pack));
    assert!(
        text_budget(&findings, "neighborhood").is_empty(),
        "{findings:#?}"
    );
}

/// **`topology.unreachable`'s hint names the dock marker** (doc 18 cut IC12 item 4.4):
/// a station reached only by ship validates by marking its gallery `port.dock`, and
/// the hint now says so.
#[test]
fn ic12_the_unreachable_hint_names_the_dock_marker() {
    let tw = town();
    let bare = signed(dock_doc(json!([])));
    let findings = validate(&bare, &context(&tw, &bare));
    let hints: Vec<&str> = findings
        .iter()
        .filter(|f| f.code == "topology.unreachable")
        .map(|f| f.fix_hint.as_str())
        .collect();
    assert!(!hints.is_empty(), "{findings:#?}");
    for hint in hints {
        assert_eq!(
            hint,
            "join it to a reachable room with an unlocked exit, mark it `port.dock` if ships \
             reach it, or make it restricted"
        );
    }
}
