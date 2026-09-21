//! The starter packs as bytes: they load, they verify, their digests are pinned, and
//! their ids and requires are what the boot order assumes. (Applying them is
//! `tests/lot.rs`.)

use mush_atlas::AtlasModule;
use mush_atlas::def::ExitDef;
use mush_atlas::starter::{OOC_AREA_PACK, REGION_PACK, SOURCES, STAFF_LOBBY_PACK, packs};
use mush_content::{Appliers, ContentModule, ContentState, PackKind, parse_pack};
use mush_core::{CanonicalId, GameModule};
use mush_testkit::TestWorld;

/// The pinned digests, in `SOURCES` order. An edit to a pack changes its digest; the
/// failure message prints the new values so a *reviewed* edit is a two-line change
/// here plus the `digest` field in the pack itself.
const PINNED: [(&str, &str); 3] = [
    (
        REGION_PACK,
        "f127f7d7cc79b6d1f8f2e0b11f49f57497e101af0fe19cdd8cfaad0631501ef0",
    ),
    (
        OOC_AREA_PACK,
        "64f6cf88104b80c4bf9af2b013ec2cba089cf2e664f827a72f29184a9cc54691",
    ),
    (
        STAFF_LOBBY_PACK,
        "8d791c31a7c81c794cb78529b2735e0c1ab360aea92aa8d03c3d571388056796",
    ),
];

#[test]
fn the_starter_packs_load_and_their_digests_are_pinned() {
    // Parsed without the digest compare, so one run reports every moved digest at once
    // — the content digest is what is pinned; `load_pack` is what boot uses.
    let mut report = Vec::new();
    for ((name, json), (id, pinned)) in SOURCES.iter().zip(PINNED) {
        let pack = parse_pack(json).expect(name);
        assert_eq!(pack.manifest().id().as_str(), id);
        let actual = pack.computed_digest().expect("digests").to_hex();
        if actual != pinned {
            report.push(format!("    ({id:?}, {actual:?}),"));
        }
    }
    assert!(
        report.is_empty(),
        "starter pack digests moved; if the edit was reviewed, pin them here and in each \
         pack's `digest` field:\n{}",
        report.join("\n")
    );
    let packs = packs().expect("the shipped packs verify");
    assert_eq!(packs.len(), 3);
}

#[test]
fn the_declared_digest_is_the_content_digest() {
    for (name, json) in SOURCES {
        let pack = parse_pack(json).expect(name);
        assert_eq!(
            pack.computed_digest().expect("digests").to_hex(),
            pack.manifest().digest().to_hex(),
            "{name}: the digest field must be re-derived, never hand-edited"
        );
    }
}

#[test]
fn kinds_and_requires_match_the_boot_order() {
    let packs = packs().expect("load");
    let region = &packs[0];
    let ooc = &packs[1];
    let lobby = &packs[2];
    assert_eq!(region.manifest().kind(), PackKind::Region);
    assert!(region.manifest().requires().is_empty());
    assert_eq!(ooc.manifest().kind(), PackKind::Neighborhood);
    assert_eq!(
        ooc.manifest()
            .requires()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        [REGION_PACK]
    );
    assert_eq!(
        lobby
            .manifest()
            .requires()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        [REGION_PACK, OOC_AREA_PACK],
        "the lobby's bus exits name the OOC area's bus room, so it applies after it"
    );
    let rooms = |p: &mush_content::Pack| {
        p.entries()
            .values()
            .filter(|e| e.kind().as_str() == "room")
            .count()
    };
    assert_eq!(
        rooms(ooc) + rooms(lobby),
        mush_atlas::starter::LOT_ROOMS + 1,
        "fourteen lot rooms and the bus"
    );
    let npcs = |p: &mush_content::Pack| {
        p.entries()
            .values()
            .filter(|e| e.kind().as_str() == "npc")
            .count()
    };
    assert_eq!(npcs(ooc) + npcs(lobby), 10, "the cast of ten");
    for p in &packs {
        assert!(
            p.brief().is_some(),
            "{}: the brief is kept for reconceptualizing",
            p.manifest().id()
        );
    }
}

/// **16b cut D.5 (16c `B-11`): no shipped exit takes a word the kernel owns**,
/// and the apply check therefore passes over all three packs in a world that
/// composed them.
///
/// The check runs at every apply, including the first boot of a fresh world, so
/// a shipped pack that tripped it would leave the minimal world and no lot. The
/// composition here is the lot's own — the atlas, the NPC host and the content
/// module with the three packs as boot packs — and its registry is what the
/// check reads.
#[test]
fn no_shipped_exit_segment_is_a_word_the_kernel_owns() {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(packs().expect("the shipped packs load")),
        ),
    ];
    let tw = TestWorld::with_modules(modules);

    // Every pack landed whole: the check refused nothing.
    let content = tw
        .world()
        .module_state::<ContentState>()
        .expect("the content module");
    for (id, _) in SOURCES {
        let latch = content
            .latches
            .applied(&CanonicalId::new(id).expect("id"))
            .unwrap_or_else(|| panic!("{id} is latched"));
        assert_eq!(latch.anomaly(), None, "{id}");
    }

    // And the same question asked of the bytes, so the reason a pack is fine is
    // readable here and not only in a boot that did not fail.
    let mut collisions = Vec::new();
    for pack in packs().expect("load") {
        for (id, entry) in pack.entries() {
            if entry.kind().as_str() != "exit" {
                continue;
            }
            let def: ExitDef = entry.decode().expect("a shipped exit decodes");
            for name in &def.names {
                for segment in name.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some(command) = tw.registry().kernel_command_named(segment) {
                        collisions.push(format!("{id}: {segment:?} is the command {command:?}"));
                    }
                }
            }
        }
    }
    assert!(
        collisions.is_empty(),
        "a shipped exit takes a kernel word, which refuses the pack at apply:\n{}",
        collisions.join("\n")
    );
}
