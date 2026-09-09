//! The starter packs as bytes: they load, they verify, their digests are pinned, and
//! their ids and requires are what the boot order assumes. (Applying them is
//! `tests/lot.rs`.)

use mush_atlas::starter::{OOC_AREA_PACK, REGION_PACK, SOURCES, STAFF_LOBBY_PACK, packs};
use mush_content::{PackKind, parse_pack};

/// The pinned digests, in `SOURCES` order. An edit to a pack changes its digest; the
/// failure message prints the new values so a *reviewed* edit is a two-line change
/// here plus the `digest` field in the pack itself.
const PINNED: [(&str, &str); 3] = [
    (
        REGION_PACK,
        "80cc2032c4f21da8a4335bb896e3f895b9cf1c1a2945a6610d6f1d63f038932b",
    ),
    (
        OOC_AREA_PACK,
        "ea068eb175a6590f904980615e2e8050a15fb34309cdeae3c6fe7fc43c679532",
    ),
    (
        STAFF_LOBBY_PACK,
        "5f91be4eb27bc64a6496a5a789397d271d9289f7392236cea6e0466295fec069",
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
