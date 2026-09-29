//! **The line-format contract of the simulator pilot** (cut AJ.1).
//!
//! The bot reads a match only through the text a seated line-mode pilot is sent
//! (`src/arena/`). This file holds that reader to the game twice over:
//!
//! 1. `the_parser_reads_what_the_cockpit_formatters_print` builds arena frames
//!    by hand and feeds every sentence the cockpit's own `pub` formatters print
//!    — the contact and weapon rows, the lock, `RANGE`/`BEARING`, the damage
//!    block, every notice, every refusal, the unseated, reclaim and resume
//!    sentences and the centre's refusals — through the server's own line
//!    renderer (`mush_ansi::render` at 120 columns), then the bot's own
//!    `terminal::Decoder`, `Assembler`, `classify` and `Picture`. A formatter
//!    whose sentence changes breaks this test and not a match. Every
//!    `FrameNotice` and `NoticeKind` variant is reached through a `match` with no
//!    wildcard arm, so a notice added later is a compile error here.
//! 2. `the_bot_reads_a_live_centre_over_its_own_ssh_client` boots the
//!    production composition with the simulator centre **inside the bot's own
//!    test binary** — where `serde_json` carries the bot's `arbitrary_precision`
//!    feature, which no simulator world had run under before this cut — and walks
//!    the whole join over the bot's own SSH session: the walk, the claim, the
//!    start, the boarding, the power, the read verbs, the orders' acks, the
//!    cockpit's `FIRE` pre-check and the abort; then a second match with the
//!    tanks posed inside each other's sight by staff, for `LOCK`, `Locking`,
//!    the `Locked` notice and `Firing` and a fresh per-match tally. It pins the
//!    sentences no `pub` formatter prints (the centre's announcements, the
//!    `STATUS` head and position rows, the room headings, the order acks) and it
//!    is the canary the later sub-cuts' end-to-end lanes stand on.
//!
//! Nothing of the machines' own controller (`tacsim::arena::reflex`) is
//! imported: what the pilot does is written from the player help alone.
use std::time::{Duration, Instant};

use mush_ansi::AnsiTheme;
use mush_bot::{
    arena::{
        Assembler, Line, Picture, Side, classify,
        lines::{Power, ResultWord, Scope},
        picture::{EventKind, Phase},
    },
    terminal::Decoder,
};
use mush_core::{GOD, ObjId, ObjKind, World};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent, RichText};
use tacsim::arena::{
    catchup::{Catchup, MachineWords, SectionWords},
    clock::Tempo,
    damage::MISS_REASONS,
    intents::Refusal,
    motion::BlockedCause,
    perception::{FrameContact, FrameNotice, Named, NoticeKind, Receiver, RefusalDetail},
    state::{Classification, ContactId, Designator, KeySpot, Power as ArenaPower, Team},
    step::{Frame, FrameFacet, FrameHeat, FrameSection},
    weapons::{FrameBand, FrameWeapon},
};
use tacsim::match_::{cockpit, lobby::SimError, resume};

#[path = "support/arena.rs"]
mod arena_rig;

// ---------------------------------------------------------------------------
// The reader: the server's renderer, then the bot's own pipeline
// ---------------------------------------------------------------------------

/// One sentence at a time through `mush_ansi::render` (the line client's own
/// renderer, 120 columns, colour on so the decoder has escapes to strip), then
/// the bot's `Decoder`, `Assembler`, `classify` and `Picture`.
struct Reader {
    caps: ClientCaps,
    theme: AnsiTheme,
    decoder: Decoder,
    assembler: Assembler,
    picture: Picture,
    now: u64,
}

impl Reader {
    fn new() -> Reader {
        Reader {
            caps: ClientCaps {
                kind: ClientKind::Ssh,
                color: ColorDepth::Ansi16,
                width: 120,
                height: 40,
                utf8: true,
                structured: false,
            },
            theme: AnsiTheme::default(),
            decoder: Decoder::default(),
            assembler: Assembler::default(),
            picture: Picture::default(),
            now: 0,
        }
    }

    /// Every line one `Sys` event renders to, classified and applied.
    fn say(&mut self, sentence: &str) -> Vec<Line> {
        self.now += 10;
        let mut bytes = Vec::new();
        mush_ansi::render(
            &OutputEvent::Sys(RichText::tag("arena-notice", sentence.to_owned())),
            &self.caps,
            &self.theme,
            &mut bytes,
        );
        let text = self.decoder.feed(&bytes);
        self.assembler
            .feed(self.now, &text)
            .iter()
            .map(|line| {
                let read = classify(line);
                self.picture.apply(self.now, &read);
                read
            })
            .collect()
    }

    /// What a sentence reads as: its first line. A sentence past 120 columns
    /// wraps, and its tail must read as nothing but `Other`.
    fn one(&mut self, sentence: &str) -> Line {
        let lines = self.say(sentence);
        assert!(
            !lines.is_empty() && lines[1..].iter().all(|line| matches!(line, Line::Other(_))),
            "{sentence:?} rendered as {lines:?}"
        );
        lines
            .into_iter()
            .next()
            .unwrap_or(Line::Other(String::new()))
    }

    /// Let every open reply commit.
    fn settle(&mut self) {
        self.now += 1_000;
        self.picture.settle(self.now);
    }
}

// ---------------------------------------------------------------------------
// The scene, by hand
// ---------------------------------------------------------------------------

const BA_ID: ContactId = ContactId(0x1111_1111_1111_1111);
const X2_ID: ContactId = ContactId(0x2222_2222_2222_2222);
const RB_ID: ContactId = ContactId(0x3333_3333_3333_3333);

fn ba() -> Designator {
    Designator::Slot {
        team: Team::Blue,
        slot: 1,
    }
}

/// The own side's second tank: a friend on a red frame.
fn rb() -> Designator {
    Designator::Slot {
        team: Team::Red,
        slot: 2,
    }
}

/// [`frame`] with a friend `RB` beside the two contacts, and the lock on it: the
/// frame the game's `(FRIENDLY)` mark is printed from.
fn friendly_frame() -> Frame {
    let mut frame = frame();
    frame.contacts.push(FrameContact {
        id: RB_ID,
        designator: rb(),
        alias: Some(2),
        class: Classification::Classified { team: Team::Red },
        bearing_mdeg: 180_000,
        range_cm: 12_000,
        age_ms: 0,
        cell: Some((100, 85)),
        condition: None,
    });
    frame.lock = Some(RB_ID);
    frame
}

fn named(id: ContactId, designator: Designator) -> Named {
    Named { id, designator }
}

/// The opaque posture blob every `Pose` carries, empty.
fn posture() -> mush_sim::RulesValue {
    let reference = mush_core::DefinitionRef::new(
        mush_core::DefinitionKey::new(
            mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
            mush_core::DefinitionKind::new("schema").expect("a canonical kind"),
            mush_core::DefinitionId::new("tacsim.arena.fixture").expect("a canonical id"),
        ),
        1,
        mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero");
    mush_sim::RulesValue::new(reference, serde_json::json!({})).expect("an empty posture blob")
}

fn facet(id: &str, armor: i64, armor_max: i64) -> FrameFacet {
    FrameFacet {
        id: id.into(),
        armor,
        armor_max,
    }
}

/// Red slot 1 at 1000 m east, 975 m north, heading east at 30 kph: a classified
/// hostile `BA` (the lock) and an unidentified `X2`, a cannon ready and a
/// machine gun cycling, four sections with facets.
fn frame() -> Frame {
    Frame {
        unit: ObjId(101),
        callsign: Designator::Slot {
            team: Team::Red,
            slot: 1,
        },
        cell: Some((100, 97)),
        team: Team::Red,
        pose: mush_sim::data::Pose {
            x_cm: 100_000,
            y_cm: 97_500,
            z_cm: 0,
            heading_mdeg: 90_000,
            posture: posture(),
        },
        speed_cms: 833,
        power: ArenaPower::Online,
        held: false,
        wreck: false,
        immobile: false,
        tempo: Tempo::Engaged,
        sight_cm: 60_000,
        contacts: vec![
            FrameContact {
                id: BA_ID,
                designator: ba(),
                alias: Some(1),
                class: Classification::Classified { team: Team::Blue },
                bearing_mdeg: 87_000,
                range_cm: 45_000,
                age_ms: 1_200,
                cell: Some((145, 99)),
                condition: None,
            },
            FrameContact {
                id: X2_ID,
                designator: Designator::Provisional(2),
                alias: None,
                class: Classification::Unknown,
                bearing_mdeg: 315_000,
                range_cm: 25_300,
                age_ms: 3_500,
                cell: Some((82, 115)),
                condition: None,
            },
        ],
        scan_tables: Vec::new(),
        notices: Vec::new(),
        lock: Some(BA_ID),
        weapons: vec![
            FrameWeapon {
                id: "cannon".into(),
                ready: true,
                ready_in_ms: 0,
                rounds: 14,
                rounds_max: 20,
                range_band: Some(FrameBand {
                    index: 1,
                    of: 2,
                    max_range_cm: 150_000,
                    dispersion_mrad: 4,
                }),
                p_permille: Some(624),
                bands_cm: vec![150_000, 300_000],
                heat_per_shot: 0,
            },
            FrameWeapon {
                id: "machine_gun".into(),
                ready: false,
                ready_in_ms: 900,
                rounds: 380,
                rounds_max: 500,
                range_band: None,
                p_permille: Some(210),
                bands_cm: vec![40_000],
                heat_per_shot: 0,
            },
        ],
        sections: vec![
            FrameSection {
                id: "hull".into(),
                condition: "intact".into(),
                internals: Some((60, 60)),
                facets: vec![
                    facet("front", 400, 400),
                    facet("left", 250, 250),
                    facet("right", 240, 250),
                    facet("rear", 150, 150),
                ],
            },
            FrameSection {
                id: "turret".into(),
                condition: "intact".into(),
                internals: Some((30, 30)),
                facets: vec![facet("front", 350, 350), facet("rear", 140, 140)],
            },
            FrameSection {
                id: "left_track".into(),
                condition: "destroyed".into(),
                internals: Some((0, 24)),
                facets: vec![facet("left", 0, 120)],
            },
            FrameSection {
                id: "right_track".into(),
                condition: "intact".into(),
                internals: Some((24, 24)),
                facets: vec![facet("right", 120, 120)],
            },
        ],
        heat: Some(FrameHeat { heat: 40, max: 100 }),
        receiver: None,
        ecm: None,
        catchup: None,
    }
}

/// Every `NoticeKind`, and a compile error here the day one is added.
fn every_notice_kind() -> Vec<NoticeKind> {
    let kinds = vec![
        NoticeKind::Acquired,
        NoticeKind::Classified,
        NoticeKind::Lost,
        NoticeKind::Unidentified,
        NoticeKind::LockLost,
        NoticeKind::Locked,
        NoticeKind::Hit,
        NoticeKind::Miss,
        NoticeKind::Kill,
        NoticeKind::Struck,
        NoticeKind::StruckSpent,
        NoticeKind::SectionLost,
        NoticeKind::TargetSectionLost,
        NoticeKind::Refused,
        NoticeKind::Blocked,
        NoticeKind::PowerOnline,
        NoticeKind::OwnDestroyed,
        NoticeKind::OwnDisabled,
        NoticeKind::ReceiverDegraded,
        NoticeKind::ReceiverClear,
        NoticeKind::EmconOn,
        NoticeKind::EmconOff,
    ];
    for kind in &kinds {
        match kind {
            NoticeKind::Acquired
            | NoticeKind::Classified
            | NoticeKind::Lost
            | NoticeKind::Unidentified
            | NoticeKind::LockLost
            | NoticeKind::Locked
            | NoticeKind::Hit
            | NoticeKind::Miss
            | NoticeKind::Kill
            | NoticeKind::Struck
            | NoticeKind::StruckSpent
            | NoticeKind::SectionLost
            | NoticeKind::TargetSectionLost
            | NoticeKind::Refused
            | NoticeKind::Blocked
            | NoticeKind::PowerOnline
            | NoticeKind::OwnDestroyed
            | NoticeKind::OwnDisabled
            | NoticeKind::ReceiverDegraded
            | NoticeKind::ReceiverClear
            | NoticeKind::EmconOn
            | NoticeKind::EmconOff => {}
        }
    }
    kinds
}

/// Every arena `Refusal`, and a compile error here the day one is added.
fn every_refusal() -> Vec<Refusal> {
    let refusals = vec![
        Refusal::Epoch,
        Refusal::Channel,
        Refusal::PowerNotOnline,
        Refusal::QueueFull,
        Refusal::Wreck,
        Refusal::MotionDisabled,
        Refusal::Linkdead,
        Refusal::Unknown,
        Refusal::NoContact,
        Refusal::NoLock,
        Refusal::Cycling,
        Refusal::Empty,
        Refusal::Overheated,
        Refusal::NoWeapon,
        Refusal::NotReflex,
        Refusal::NoJammer,
    ];
    for refusal in &refusals {
        match refusal {
            Refusal::Epoch
            | Refusal::Channel
            | Refusal::PowerNotOnline
            | Refusal::QueueFull
            | Refusal::Wreck
            | Refusal::MotionDisabled
            | Refusal::Linkdead
            | Refusal::Unknown
            | Refusal::NoContact
            | Refusal::NoLock
            | Refusal::Cycling
            | Refusal::Empty
            | Refusal::Overheated
            | Refusal::NoWeapon
            | Refusal::NotReflex
            | Refusal::NoJammer => {}
        }
    }
    refusals
}

/// One notice of every `FrameNotice` variant (the contact arm once per
/// `NoticeKind`, both halves of each `Option`, every miss reason, every block
/// cause and every refusal).
fn every_notice() -> Vec<FrameNotice> {
    let mut notices = Vec::new();
    for kind in every_notice_kind() {
        // `BA` for the renames and the lock, `X9` (not on the frame, so no
        // bearing) for the plain acquisition, `X2` for the rest.
        let (contact, was) = match kind {
            NoticeKind::Classified => (named(BA_ID, ba()), Some(Designator::Provisional(1))),
            NoticeKind::Unidentified => (named(X2_ID, Designator::Provisional(2)), Some(ba())),
            NoticeKind::LockLost => (named(BA_ID, ba()), None),
            _ => (named(X2_ID, Designator::Provisional(2)), None),
        };
        notices.push(FrameNotice::Contact {
            kind,
            contact,
            class: Classification::Classified { team: Team::Blue },
            was,
        });
    }
    notices.push(FrameNotice::Contact {
        kind: NoticeKind::Acquired,
        contact: named(ContactId(9), Designator::Provisional(9)),
        class: Classification::Unknown,
        was: None,
    });
    notices.push(FrameNotice::Locked {
        contact: named(BA_ID, ba()),
    });
    notices.push(FrameNotice::Locked {
        contact: named(X2_ID, Designator::Provisional(2)),
    });
    for (contact, section, spent) in [
        (Some(named(BA_ID, ba())), Some("turret"), false),
        (Some(named(BA_ID, ba())), Some("turret"), true),
        (Some(named(X2_ID, Designator::Provisional(2))), None, false),
        (None, None, false),
    ] {
        notices.push(FrameNotice::Hit {
            contact,
            section: section.map(Into::into),
            spent,
        });
    }
    for reason in MISS_REASONS {
        notices.push(FrameNotice::Miss {
            contact: Some(named(BA_ID, ba())),
            reason: reason.into(),
        });
        notices.push(FrameNotice::Miss {
            contact: None,
            reason: reason.into(),
        });
    }
    notices.push(FrameNotice::Miss {
        contact: None,
        reason: "".into(),
    });
    notices.push(FrameNotice::Kill {
        contact: Some(named(BA_ID, ba())),
    });
    notices.push(FrameNotice::Kill { contact: None });
    for (section, armor_section, facet, left, max, inside) in [
        ("hull", "hull", "front", 280, 400, 60),
        ("hull", "hull", "front", 0, 400, 45),
        ("engine", "hull", "rear", 90, 150, 40),
    ] {
        notices.push(FrameNotice::Struck {
            section: section.into(),
            armor_section: armor_section.into(),
            facet: facet.into(),
            armor_left: left,
            armor_max: max,
            internals_left: inside,
            internals_max: 60,
        });
    }
    notices.push(FrameNotice::StruckSpent {
        section: "left_track".into(),
    });
    notices.push(FrameNotice::SectionLost {
        section: "left_track".into(),
        consequences: vec!["slow".into(), "no_turn_toward".into()],
    });
    notices.push(FrameNotice::SectionLost {
        section: "turret".into(),
        consequences: Vec::new(),
    });
    notices.push(FrameNotice::TargetSectionLost {
        contact: named(BA_ID, ba()),
        section: "turret".into(),
    });
    for refusal in every_refusal() {
        let detail = match refusal {
            Refusal::Cycling => RefusalDetail::Cycling { secs: 4 },
            Refusal::Overheated => RefusalDetail::Overheated {
                heat: 100,
                max: 100,
            },
            _ => RefusalDetail::None,
        };
        notices.push(FrameNotice::Refused {
            order: "fire".into(),
            refusal,
            weapon: Some("cannon".into()),
            detail,
        });
    }
    notices.push(FrameNotice::Refused {
        order: "heading".into(),
        refusal: Refusal::MotionDisabled,
        weapon: None,
        detail: RefusalDetail::None,
    });
    for cause in [
        BlockedCause::Impassable,
        BlockedCause::Occupied,
        BlockedCause::Edge,
        BlockedCause::TooSteep,
    ] {
        notices.push(FrameNotice::Blocked { cause });
    }
    notices.push(FrameNotice::PowerOnline);
    notices.push(FrameNotice::OwnDestroyed);
    notices.push(FrameNotice::OwnDisabled);
    notices.push(FrameNotice::Receiver { degraded: true });
    notices.push(FrameNotice::Receiver { degraded: false });
    notices.push(FrameNotice::Emcon {
        on: true,
        radius_m: 60,
    });
    notices.push(FrameNotice::Emcon {
        on: false,
        radius_m: 60,
    });
    notices
}

// ---------------------------------------------------------------------------
// (1) The formatters
// ---------------------------------------------------------------------------

#[test]
fn the_parser_reads_what_the_cockpit_formatters_print() {
    let frame = frame();
    let rows = cockpit::frame_contacts(&frame);
    assert_eq!(rows.len(), 2);

    // -- CONTACTS -----------------------------------------------------------
    let mut reader = Reader::new();
    for row in &rows {
        match reader.one(&row.render()) {
            Line::ContactRow(read) => {
                assert_eq!(read.designator, row.designator.to_string());
                assert_eq!(
                    read.side.map(Side::as_str),
                    (row.class != "unknown").then_some(row.class.as_str())
                );
                assert_eq!(read.bearing_deg, row.bearing_deg);
                assert_eq!(read.rel_deg, row.relative_deg);
                assert_eq!(read.band, row.band.as_str());
                assert_eq!(u64::from(read.range_m), row.range_m);
                assert_eq!(i64::from(read.age_s), row.age_ms / 1_000);
            }
            other => panic!("{} read as {other:?}", row.render()),
        }
    }
    reader.settle();
    let picture = &reader.picture;
    assert_eq!(picture.contacts.keys().collect::<Vec<_>>(), ["BA", "X2"]);
    assert_eq!(picture.contacts["BA"].side, Some(Side::Blue));
    assert_eq!(picture.contacts["X2"].side, None);

    // -- WEAPONSTATUS ---------------------------------------------------------
    let mut reader = Reader::new();
    let lock = cockpit::lock_line(&frame).expect("a lock line");
    assert_eq!(
        reader.one(&lock),
        Line::Lock {
            designator: "BA".into(),
            friendly: false,
            percent: Some(62)
        }
    );
    let weapons = cockpit::frame_weapons(&frame);
    for weapon in &weapons {
        let sentence = weapon.render(&weapons);
        match reader.one(&sentence) {
            Line::WeaponRow(read) => {
                assert_eq!(read.id, weapon.id.as_str());
                assert_eq!(read.ready, weapon.ready);
                assert_eq!(i64::from(read.rounds), weapon.rounds);
                assert_eq!(i64::from(read.rounds_max), weapon.rounds_max);
                assert_eq!(read.percent, weapon.p_permille.map(|p| p / 10));
                assert_eq!(read.band, weapon.range_band.map(|b| (b.index, b.of)));
                assert_eq!(
                    read.cycling_s.map(i64::from),
                    (!weapon.ready).then(|| (weapon.ready_in_ms + 999) / 1_000)
                );
            }
            other => panic!("{sentence} read as {other:?}"),
        }
    }
    reader.settle();
    let picture = &reader.picture;
    assert_eq!(picture.lock.as_deref(), Some("BA"));
    assert_eq!(picture.lock_percent, Some(62));
    let ids: Vec<&str> = picture.weapons.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["cannon", "machine_gun"]);
    assert_eq!(picture.weapons[1].short.as_deref(), Some("mg"));
    assert!(!picture.weapons[1].ready);

    // -- a friend: `(FRIENDLY)` as the game's own formatters print it ---------
    // The fratricide guard AJ.2 builds reads `lock_friendly`: the mark is held
    // to the game's `friendly_mark`, on the lock line, the `Locked` notice and
    // the shooter's hit, never to a hand-typed string.
    let friend = friendly_frame();
    let friend_rows = cockpit::frame_contacts(&friend);
    let mut reader = Reader::new();
    for row in &friend_rows {
        reader.one(&row.render());
    }
    reader.settle();
    assert_eq!(reader.picture.contacts["RB"].side, Some(Side::Red));
    let lock = cockpit::lock_line(&friend).expect("a lock line");
    assert_eq!(
        reader.one(&lock),
        Line::Lock {
            designator: "RB".into(),
            friendly: true,
            percent: Some(62)
        },
        "{lock}"
    );
    for weapon in &cockpit::frame_weapons(&friend) {
        reader.one(&weapon.render(&cockpit::frame_weapons(&friend)));
    }
    reader.settle();
    assert_eq!(reader.picture.lock.as_deref(), Some("RB"));
    assert!(reader.picture.lock_friendly);
    let sentence = cockpit::notice_line(
        &friend,
        &FrameNotice::Locked {
            contact: named(RB_ID, rb()),
        },
        &friend_rows,
    );
    assert!(
        matches!(reader.one(&sentence), Line::Lock { ref designator, friendly: true, .. } if designator == "RB"),
        "{sentence}"
    );
    let sentence = cockpit::notice_line(
        &friend,
        &FrameNotice::Hit {
            contact: Some(named(RB_ID, rb())),
            section: Some("hull".into()),
            spent: false,
        },
        &friend_rows,
    );
    assert_eq!(
        reader.one(&sentence),
        Line::HitScored {
            designator: Some("RB".into()),
            friendly: true,
            section: Some("hull".into()),
            already_destroyed: false
        },
        "{sentence}"
    );
    // The same hit on the hostile reads unmarked.
    let sentence = cockpit::notice_line(
        &friend,
        &FrameNotice::Hit {
            contact: Some(named(BA_ID, ba())),
            section: Some("hull".into()),
            spent: false,
        },
        &friend_rows,
    );
    assert!(
        matches!(
            reader.one(&sentence),
            Line::HitScored {
                friendly: false,
                ..
            }
        ),
        "{sentence}"
    );

    // -- RANGE and BEARING, fresh and stale, locked and not -----------------
    let mut reader = Reader::new();
    for row in &rows {
        for pass_ms in [1_000, 10_000] {
            let stale = row.age_ms > pass_ms;
            let sentence = cockpit::range_line(&frame, row, pass_ms);
            match reader.one(&sentence) {
                Line::Range {
                    designator,
                    plotted_s,
                    ..
                } => {
                    assert_eq!(designator, row.designator.to_string());
                    assert_eq!(plotted_s.is_some(), stale, "{sentence}");
                }
                other => panic!("{sentence} read as {other:?}"),
            }
            let sentence = cockpit::bearing_line(row, pass_ms);
            match reader.one(&sentence) {
                Line::Bearing {
                    designator,
                    bearing_deg,
                    rel_deg,
                    plotted_s,
                } => {
                    assert_eq!(designator, row.designator.to_string());
                    assert_eq!((bearing_deg, rel_deg), (row.bearing_deg, row.relative_deg));
                    assert_eq!(plotted_s.is_some(), stale, "{sentence}");
                }
                other => panic!("{sentence} read as {other:?}"),
            }
        }
    }
    // The lock's own range is the finer figure, at the step its plot's age
    // allows; a row that is not the lock reads the scope's 50 m.
    let lock_m = cockpit::range_m_rounded(45_000, cockpit::lock_range_step_m(1_200));
    assert_eq!(
        reader.one(&cockpit::range_line(&frame, &rows[0], 10_000)),
        Line::Range {
            designator: "BA".into(),
            range_m: u32::try_from(lock_m).expect("metres"),
            plotted_s: None
        }
    );
    assert_eq!(
        reader.one(&cockpit::range_line(&frame, &rows[1], 10_000)),
        Line::Range {
            designator: "X2".into(),
            range_m: 250,
            plotted_s: None
        }
    );

    // -- the named ground ------------------------------------------------------
    let spots = vec![
        KeySpot {
            name: "The Ford".into(),
            x: 60,
            y: 97,
        },
        KeySpot {
            name: "The Crossroads".into(),
            x: 129,
            y: 97,
        },
    ];
    assert_eq!(
        reader.one(&cockpit::spot_range_line(&spots[0], 'A', 430)),
        Line::SpotRange {
            name: "The Ford".into(),
            letter: 'A',
            range_m: 430
        }
    );
    assert_eq!(
        reader.one(&cockpit::spot_bearing_line(&spots[1], 'B', 87, 357)),
        Line::SpotBearing {
            name: "The Crossroads".into(),
            letter: 'B',
            bearing_deg: 87,
            rel_deg: 357
        }
    );

    // -- the parts of STATUS a `pub` formatter prints -------------------------
    let mut reader = Reader::new();
    reader.one(&format!("Callsign {}.", frame.callsign));
    let nearest = cockpit::nearest_spot_line(&spots, &frame.pose, 1_000).expect("a nearest spot");
    assert!(matches!(
        reader.one(&nearest),
        Line::StatusNearestSpot { letter: 'B', .. }
    ));
    assert_eq!(
        reader.one(&cockpit::receiver_line(Receiver {
            budget: 15,
            budget_max: 40,
            reach_permille: 600
        })),
        Line::StatusReceiver {
            budget: 15,
            budget_max: 40,
            reach_percent: 60
        }
    );
    for on in [true, false] {
        assert_eq!(
            reader.one(cockpit::emcon_status_line(on)),
            Line::StatusEmcon { on }
        );
    }
    for sentence in cockpit::section_lines(&frame.sections) {
        let read = reader.one(&sentence);
        assert!(
            matches!(read, Line::StatusSection { .. } | Line::StatusFacets { .. }),
            "{sentence:?} read as {read:?}"
        );
    }
    reader.settle();
    let own = &reader.picture.own;
    assert_eq!(own.callsign.as_deref(), Some("RA"));
    assert!(own.jammed);
    assert_eq!(own.emcon, Some(false));
    for section in &frame.sections {
        let read = &own.sections[section.id.as_str()];
        assert_eq!(read.condition, section.condition.as_str());
        assert_eq!(read.internals, section.internals);
        for facet in &section.facets {
            assert_eq!(
                read.facets[facet.id.as_str()],
                (facet.armor, facet.armor_max)
            );
        }
    }
    assert_eq!(reader.picture.spots[&'B'].name, "The Crossroads");

    // -- every notice ---------------------------------------------------------
    let mut reader = Reader::new();
    let mut said = 0_usize;
    for notice in every_notice() {
        let sentence = cockpit::notice_line(&frame, &notice, &rows);
        if sentence.is_empty() {
            continue;
        }
        said += 1;
        let read = reader.one(&sentence);
        let fail = || format!("{notice:?} said {sentence:?} and read as {read:?}");
        match &notice {
            FrameNotice::Contact {
                kind, contact, was, ..
            } => {
                let d = contact.designator.to_string();
                match kind {
                    NoticeKind::Acquired => assert!(
                        matches!(&read, Line::NewContact { designator, .. } if *designator == d),
                        "{}",
                        fail()
                    ),
                    NoticeKind::Classified => assert!(
                        matches!(&read, Line::Identified { designator, was: w, side: Side::Blue }
                            if *designator == d && Some(w.clone()) == was.map(|w| w.to_string())),
                        "{}",
                        fail()
                    ),
                    NoticeKind::Unidentified => assert!(
                        matches!(&read, Line::Unidentified { designator, .. } if *designator == d),
                        "{}",
                        fail()
                    ),
                    NoticeKind::LockLost => assert!(
                        matches!(&read, Line::LockBroken { designator } if *designator == d),
                        "{}",
                        fail()
                    ),
                    // `Lost`, and every other kind the contact arm says as a loss.
                    NoticeKind::Lost
                    | NoticeKind::Locked
                    | NoticeKind::Hit
                    | NoticeKind::Miss
                    | NoticeKind::Kill
                    | NoticeKind::Struck
                    | NoticeKind::StruckSpent
                    | NoticeKind::SectionLost
                    | NoticeKind::TargetSectionLost
                    | NoticeKind::Refused
                    | NoticeKind::Blocked
                    | NoticeKind::PowerOnline
                    | NoticeKind::OwnDestroyed
                    | NoticeKind::OwnDisabled
                    | NoticeKind::ReceiverDegraded
                    | NoticeKind::ReceiverClear
                    | NoticeKind::EmconOn
                    | NoticeKind::EmconOff => assert!(
                        matches!(&read, Line::ContactLost { designator } if *designator == d),
                        "{}",
                        fail()
                    ),
                }
            }
            FrameNotice::Locked { contact } => assert!(
                matches!(&read, Line::Lock { designator, .. } if *designator == contact.designator.to_string()),
                "{}",
                fail()
            ),
            FrameNotice::Hit {
                contact,
                section,
                spent,
            } => assert!(
                matches!(&read, Line::HitScored { designator, section: s, already_destroyed, .. }
                    if *designator == contact.as_ref().map(|c| c.designator.to_string())
                        && s.as_deref() == contact.as_ref().and(section.as_deref())
                        && *already_destroyed == (*spent && section.is_some())),
                "{}",
                fail()
            ),
            FrameNotice::Miss { contact, reason } => assert!(
                matches!(&read, Line::Missed { designator, words }
                    if *designator == contact.as_ref().map(|c| c.designator.to_string())
                        && words.is_some() == (contact.is_some() || !reason.is_empty())),
                "{}",
                fail()
            ),
            FrameNotice::Kill { contact } => assert!(
                matches!(&read, Line::Killed { designator }
                    if *designator == contact.as_ref().map(|c| c.designator.to_string())),
                "{}",
                fail()
            ),
            FrameNotice::Struck {
                section,
                armor_section,
                facet,
                armor_left,
                armor_max,
                internals_left,
                internals_max,
            } => {
                let through = (armor_section != section).then(|| armor_section.to_string());
                let (armour, internals) = if *armor_left > 0 {
                    (Some((i64::from(*armor_left), i64::from(*armor_max))), None)
                } else {
                    (
                        None,
                        Some((i64::from(*internals_left), i64::from(*internals_max))),
                    )
                };
                assert_eq!(
                    read,
                    Line::HitTaken {
                        section: section.to_string(),
                        through,
                        facet: Some(facet.to_string()),
                        armour,
                        internals,
                        already_destroyed: false
                    },
                    "{}",
                    fail()
                );
            }
            FrameNotice::StruckSpent { section } => assert!(
                matches!(&read, Line::HitTaken { section: s, already_destroyed: true, .. } if s == section.as_str()),
                "{}",
                fail()
            ),
            FrameNotice::SectionLost {
                section,
                consequences,
            } => assert!(
                matches!(&read, Line::SectionLost { section: s, cost }
                    if s == section.as_str() && cost.is_some() == !consequences.is_empty()),
                "{}",
                fail()
            ),
            FrameNotice::TargetSectionLost { contact, section } => assert!(
                matches!(&read, Line::TargetSectionLost { designator, section: s }
                    if *designator == contact.designator.to_string() && s == section.as_str()),
                "{}",
                fail()
            ),
            FrameNotice::Refused {
                order,
                refusal,
                weapon,
                detail,
            } => {
                let Line::Refused {
                    order: o,
                    weapon: w,
                    cycling_s,
                    ..
                } = &read
                else {
                    panic!("{}", fail());
                };
                assert_eq!(*o, order.to_uppercase(), "{}", fail());
                assert_eq!(w.as_deref(), weapon.as_deref(), "{}", fail());
                let cycling = match (refusal, detail) {
                    (Refusal::Cycling, RefusalDetail::Cycling { secs }) => {
                        u32::try_from(*secs).ok()
                    }
                    (Refusal::Cycling, _) => Some(0),
                    _ => None,
                };
                assert_eq!(*cycling_s, cycling, "{}", fail());
            }
            FrameNotice::Blocked { cause } => {
                use mush_bot::arena::lines::HeldCause;
                let want = match cause {
                    BlockedCause::Impassable => HeldCause::Impassable,
                    BlockedCause::Occupied => HeldCause::Occupied,
                    BlockedCause::Edge => HeldCause::Edge,
                    BlockedCause::TooSteep => HeldCause::TooSteep,
                };
                assert_eq!(read, Line::Held { cause: want }, "{}", fail());
            }
            FrameNotice::PowerOnline => assert_eq!(read, Line::PowerOnline, "{}", fail()),
            FrameNotice::OwnDestroyed => assert_eq!(read, Line::OwnDestroyed, "{}", fail()),
            FrameNotice::OwnDisabled => assert_eq!(read, Line::OwnDisabled, "{}", fail()),
            FrameNotice::Receiver { degraded } => assert_eq!(
                read,
                if *degraded {
                    Line::ReceiverDegraded
                } else {
                    Line::ReceiverClear
                },
                "{}",
                fail()
            ),
            FrameNotice::Emcon { on, radius_m } => assert_eq!(
                read,
                Line::EmconSwitched {
                    on: *on,
                    radius_m: on.then_some(*radius_m)
                },
                "{}",
                fail()
            ),
        }
    }
    assert!(said > 60, "{said} notices said");
    let tally = &reader.picture.tally;
    assert_eq!(tally.hits_scored, 4);
    assert_eq!(tally.misses, 2 * MISS_REASONS.len() as u32 + 1);
    assert_eq!(tally.kills, 2);
    assert_eq!(tally.hits_taken, 4);
    assert_eq!(tally.sections_lost, ["left_track", "turret"]);
    assert_eq!(tally.shots, 0);
    assert!(
        reader.picture.own.destroyed && reader.picture.own.disabled && reader.picture.own.immobile
    );
    assert_eq!(
        reader.picture.own.sections["hull"].facets["front"],
        (0, 400)
    );
    assert_eq!(
        reader.picture.own.sections["hull"].facets["rear"],
        (90, 150)
    );
    let kinds: Vec<EventKind> = reader.picture.events.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&EventKind::PowerOnline));

    // -- the refusal every FIRE says, whichever half answers --------------------
    let mut reader = Reader::new();
    assert_eq!(
        reader.one(&cockpit::order_refused(
            "fire",
            Some("cannon"),
            cockpit::NO_LOCK
        )),
        Line::Refused {
            order: "FIRE".into(),
            weapon: Some("cannon".into()),
            said: cockpit::NO_LOCK.into(),
            cycling_s: None
        }
    );
    assert_eq!(
        reader.one(&cockpit::order_refused("heading", None, cockpit::IMMOBILE)),
        Line::Refused {
            order: "HEADING".into(),
            weapon: None,
            said: cockpit::IMMOBILE.into(),
            cycling_s: None
        }
    );
    assert_eq!(reader.picture.tally.refused_orders, 2);
    assert!(reader.picture.own.immobile);

    // -- the constants: cockpit, arena and resume ----------------------------------
    let mut reader = Reader::new();
    for (sentence, want) in [
        (cockpit::OWN_DESTROYED, Line::OwnDestroyed),
        (cockpit::OWN_DISABLED, Line::OwnDisabled),
        (cockpit::RECEIVER_DEGRADED, Line::ReceiverDegraded),
        (cockpit::RECEIVER_CLEAR, Line::ReceiverClear),
        (cockpit::NO_LOCK, Line::NoLock),
        (cockpit::NO_LOCK_OR_NAME, Line::NoLockOrName),
        (cockpit::NO_SUCH_CONTACT, Line::NoSuchContact),
        (cockpit::NO_FRAME_YET, Line::NoFrameYet),
        (cockpit::POWER_STARTING, Line::PowerStarting),
        (cockpit::POWER_NOT_ONLINE, Line::PowerNotOnline),
        (cockpit::DESTROYED, Line::DestroyedNoOrders),
        (cockpit::IMMOBILE, Line::Immobile),
        (cockpit::OUTSIDE_COCKPIT, Line::OutsideCockpit),
        (cockpit::NOT_IN_A_POD, Line::NotInAPod),
        (cockpit::OUTSIDE_TEAM_ROOM, Line::OutsideTeamRoom),
        (cockpit::NOT_STARTED_YET, Line::NotStartedYet),
        (resume::RESUME_LINE, Line::Resumed),
    ] {
        assert_eq!(reader.one(sentence), want, "{sentence}");
    }
    for (error, want) in [
        (cockpit::CockpitError::AlreadyAboard, Line::AlreadyAboard),
        (
            cockpit::CockpitError::HandingOver {
                unit: "Red Tank 1".into(),
                secs: 3,
            },
            Line::HandingOver {
                unit: "Red Tank 1".into(),
                secs: 3,
            },
        ),
        (
            cockpit::CockpitError::LeftForGood {
                unit: "Red Tank 1".into(),
            },
            Line::LeftForGood {
                unit: "Red Tank 1".into(),
            },
        ),
        (cockpit::CockpitError::NoLockOrName, Line::NoLockOrName),
        (cockpit::CockpitError::NoWeapon, Line::NoWeapon),
        (cockpit::CockpitError::Destroyed, Line::DestroyedNoOrders),
    ] {
        assert_eq!(reader.one(&error.to_string()), want, "{error}");
    }

    // -- the centre's refusals (`SimError`'s own sentences) ----------------------
    for (error, want) in [
        (
            SimError::AlreadyClaimed {
                team: "red",
                index: 1,
            },
            Line::AlreadyHold {
                side: Side::Red,
                index: 1,
            },
        ),
        (
            SimError::SideFull {
                team: "blue",
                max: 6,
            },
            Line::SideFull {
                side: Side::Blue,
                max: 6,
            },
        ),
        (SimError::NotLead, Line::NotLead),
        (
            SimError::EmptySide("blue"),
            Line::EmptySide { side: Side::Blue },
        ),
        (
            SimError::WrongState { state: "running" },
            Line::WrongCentreState {
                state: "running".into(),
            },
        ),
        (SimError::NoCentre, Line::NoCentre),
        (SimError::LayerOff, Line::LayerOff),
        (SimError::NobodyPlaying, Line::NobodyPlaying),
        (SimError::OutsideCentre, Line::OutsideCentre),
        (SimError::NoSlot, Line::NoSlot),
        (SimError::NothingToAbort, Line::NothingToAbort),
        (
            SimError::ArenaBusy,
            Line::Transient("The arena is busy; try that again in a moment.".into()),
        ),
    ] {
        let sentence = error.to_string();
        let lines = reader.say(&sentence);
        // The layer-off sentence is the one long enough to wrap at 120: its
        // first line carries the whole of what the pilot needs.
        assert_eq!(lines.first(), Some(&want), "{sentence} read as {lines:?}");
    }

    // -- the reclaim catch-up block and the doctrine line ------------------------
    let mut reader = Reader::new();
    let catchup = Catchup {
        away_ms: 134_567,
        cell_then: Some((64, 97)),
        cell_now: Some((70, 101)),
        power_then: ArenaPower::Online,
        power_now: ArenaPower::Online,
        wreck: false,
        contacts_new: vec![ba(), Designator::Provisional(3)],
        contacts_lost: vec![Designator::Provisional(2)],
        contacts_more: 0,
        sections: vec![(
            "hull".into(),
            SectionWords {
                condition: "intact".into(),
                armour: 230,
                internals: 60,
            },
            SectionWords {
                condition: "intact".into(),
                armour: 90,
                internals: 60,
            },
        )],
        sections_more: 0,
        rounds_spent: vec![("cannon".into(), 3)],
        rounds_more: 0,
        machine: Some(MachineWords {
            state: "withdraw".into(),
            goal: None,
            objective: None,
        }),
    };
    let block = cockpit::catchup_lines(
        Designator::Slot {
            team: Team::Red,
            slot: 1,
        },
        &catchup,
    );
    let lines = reader.say(&block.join("\n"));
    assert_eq!(lines.len(), block.len());
    assert_eq!(
        lines[0],
        Line::BackAboard {
            callsign: "RA".into(),
            away_ms: 134_000
        }
    );
    assert!(lines.contains(&Line::CatchupPower {
        power: Power::Online
    }));
    assert!(
        lines[1..]
            .iter()
            .all(|line| matches!(line, Line::Other(_) | Line::CatchupPower { .. }))
    );
    assert!(reader.picture.aboard);
    assert_eq!(reader.picture.reclaims, 1);
    assert_eq!(reader.picture.away_ms, 134_000);
    assert!(
        reader.picture.contacts.is_empty(),
        "the catch-up names contacts it does not add"
    );
    assert_eq!(reader.picture.own.power, Power::Online);

    let mut world = World::new_minimal();
    let tank = world
        .create(ObjKind::Thing, "Red Tank 1", GOD, None)
        .expect("a tank");
    assert_eq!(
        reader.one(&resume::doctrine_line(&world, tank)),
        Line::Doctrine {
            tank: "Red Tank 1".into()
        }
    );
    assert!(!reader.picture.aboard);
}

// ---------------------------------------------------------------------------
// (2) The live centre: the canary
// ---------------------------------------------------------------------------

/// The nonsense word a whole answer is fenced with, and the parser's answer.
const SENTINEL: &str = "zzq-arena-sentinel";
const HUH: &str = "Huh?";

/// The client, the bot's reader on its stream, and what it has read.
struct Live {
    client: arena_rig::Client,
    assembler: Assembler,
    picture: Picture,
    started: Instant,
    lines: Vec<Line>,
    phases: Vec<Phase>,
}

impl Live {
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Everything the client has decoded since the last read, through the
    /// bot's own assembler, classifier and picture.
    fn read(&mut self) {
        let text = self.client.drain();
        let now = self.now();
        for raw in self.assembler.feed(now, &text) {
            self.apply(now, &raw);
        }
    }

    fn apply(&mut self, now: u64, raw: &str) {
        let line = classify(raw);
        self.picture.apply(now, &line);
        if self.phases.last() != Some(&self.picture.phase) {
            self.phases.push(self.picture.phase.clone());
        }
        self.lines.push(line);
    }

    async fn say(&mut self, line: &str, needle: &str) -> String {
        let seen = self.client.say(line, needle).await;
        self.read();
        seen
    }

    /// `line` and then [`SENTINEL`]: the answer runs to the sentinel's `Huh?`,
    /// so all of it is in and none of it is in flight.
    async fn ask_whole(&mut self, line: &str) -> String {
        for _ in 0..arena_rig::RETRY_BUDGET {
            let from = self.client.seen.len();
            self.client.send(line).await;
            self.client.send(SENTINEL).await;
            self.client.expect(HUH, arena_rig::STEP_TIMEOUT).await;
            self.read();
            let seen = self.client.seen.get(from..).unwrap_or("").to_owned();
            if !arena_rig::TRANSIENTS.iter().any(|t| seen.contains(t)) {
                return seen;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("`{line}` never produced a whole answer");
    }

    async fn wait_for(&mut self, needle: &str, timeout: Duration) {
        self.client.expect(needle, timeout).await;
        self.read();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bot_reads_a_live_centre_over_its_own_ssh_client() {
    tokio::time::timeout(Duration::from_secs(90), live_centre())
        .await
        .expect("the live centre finishes inside 90 s");
}

async fn live_centre() {
    let rig = arena_rig::boot(arena_rig::arena_config(), &[("pilot", "Pilot")]).await;
    let client = arena_rig::Client::login(&rig, "pilot", arena_rig::PILOT_PASSWORD, "Pilot").await;
    let mut live = Live {
        client,
        assembler: Assembler::default(),
        picture: Picture::default(),
        started: Instant::now(),
        lines: Vec::new(),
        phases: vec![Phase::Unknown],
    };
    live.read();
    assert_eq!(
        live.picture.me.as_deref(),
        Some("Pilot"),
        "`Connected as` names me"
    );

    // The walk: Limbo, the Welcome Hall, the lobby.
    live.ask_whole("look").await;
    live.say("ooc", "Welcome Hall").await;
    live.say("simulator", "Simulator Lobby").await;
    live.ask_whole("SIM STATUS").await;
    assert!(
        live.lines
            .iter()
            .any(|line| matches!(line, Line::Board { state, .. } if state == "idle")),
        "the idle board"
    );

    // The claim. The lane knows the slot is its own by the name on the line.
    live.say("SIM TEAM red", "takes red slot").await;
    let claim = live
        .lines
        .iter()
        .find_map(|line| match line {
            Line::Claimed { name, side, index } => Some((name.clone(), *side, *index)),
            _ => None,
        })
        .expect("the claim line reads");
    assert_eq!(claim, ("Pilot".to_owned(), Side::Red, 1));
    live.picture.claimed(claim.1, claim.2);
    live.ask_whole("SIM STATUS").await;
    assert!(
        live.lines.iter().any(|line| matches!(
            line,
            Line::BoardSide { side: Side::Red, pilots } if pilots.first().is_some_and(|p| p.name == "Pilot" && p.callsign == "RA")
        )),
        "the board's red row; the transcript:\n{}",
        live.client.seen
    );
    assert!(
        live.lines
            .iter()
            .any(|line| matches!(line, Line::AiFill { each: 1 }))
    );

    // The start, the team room, the pod.
    live.say("SIM START", "is boarding").await;
    let match_id = live
        .lines
        .iter()
        .find_map(|line| match line {
            Line::Boarding { match_id } => Some(*match_id),
            _ => None,
        })
        .expect("the boarding line reads");
    live.say("red", "Red Room").await;
    live.say("BOARD", "You climb into").await;
    live.wait_for("Good hunting", Duration::from_secs(30)).await;
    live.say("STARTUP", "power comes online shortly").await;
    live.wait_for("Power online.", Duration::from_secs(30))
        .await;

    // The read verbs, each answer whole.
    live.ask_whole("STATUS").await;
    live.ask_whole("CONTACTS").await;
    live.ask_whole("WEAPONSTATUS").await;
    live.ask_whole("RANGE B").await;
    live.ask_whole("BEARING B").await;
    live.say("SPEED 999", "kph ahead and").await;

    // The orders' own acks (private strings, pinned only here): the heading,
    // the speed, and a `FIRE` with no lock, which the cockpit's pre-check
    // answers instead of `Firing`. The runner's `sent_fire` mark makes it the
    // pre-check whatever acks are outstanding.
    live.say("HEADING 90", "Steering to").await;
    assert!(live.lines.contains(&Line::Steering { deg: 90 }));
    live.say("SPEED 10", "Making").await;
    assert!(live.lines.contains(&Line::Making { kph: 10 }));
    let now = live.now();
    live.picture.sent_fire(now);
    live.say("FIRE cannon", "refused").await;
    assert!(
        live.lines.iter().any(|line| matches!(
            line,
            Line::Refused { order, weapon, said, .. }
                if order == "FIRE" && weapon.as_deref() == Some("cannon")
                    && said == "No lock: LOCK a contact first."
        )),
        "the pre-check's refusal"
    );
    assert_eq!(
        (
            live.picture.tally.shots,
            live.picture.tally.refused_shots,
            live.picture.tally.refused_orders
        ),
        (0, 0, 1)
    );

    // The abort: the running match is stopped and checked in.
    live.say("SIM ABORT", "aborts match").await;
    live.wait_for("is over (", Duration::from_secs(30)).await;
    live.client.pump(Duration::from_millis(500)).await;
    live.read();
    let now = live.now() + 1_000;
    if let Some(tail) = live.assembler.flush_stale(now) {
        live.apply(now, &tail);
    }
    live.picture.settle(now);

    let picture = &live.picture;
    let transcript = live.client.seen.clone();
    assert_eq!(picture.following, Some(match_id));
    assert_eq!(
        live.phases,
        [
            Phase::Unknown,
            Phase::Walking,
            Phase::InCentre,
            Phase::Claimed {
                side: Side::Red,
                index: 1
            },
            Phase::Boarding { match_id },
            Phase::Seated { match_id },
            Phase::Running { match_id },
            Phase::Over {
                match_id,
                result: ResultWord::Abort
            },
        ],
        "the phases in order; the transcript:\n{transcript}"
    );
    assert!(live.lines.contains(&Line::AbortsMatch {
        name: "Pilot".into(),
        match_id
    }));
    assert!(live.lines.contains(&Line::Over {
        match_id,
        result: ResultWord::Abort
    }));
    for room in ["Welcome Hall", "Simulator Lobby", "Red Room"] {
        assert!(
            live.lines.contains(&Line::Room(room.into())),
            "the heading {room}"
        );
    }
    assert!(live.lines.contains(&Line::Room("Red Pod 1".into())));
    assert!(live.lines.contains(&Line::ClimbIn {
        pod: "Red Pod 1".into()
    }));
    assert!(live.lines.contains(&Line::StartupBegun));
    assert!(live.lines.contains(&Line::Running { match_id }));
    assert!(live.lines.contains(&Line::HatchOpen {
        pod: "Red Pod 1".into()
    }));
    assert!(live.lines.contains(&Line::NoContacts));
    assert!(live.lines.iter().any(|line| matches!(
        line,
        Line::StatusHead(head) if head.name == "Red Tank 1" && head.power == Power::Online
    )));
    assert!(live.lines.contains(&Line::PowerOnline));
    assert!(!picture.aboard, "the hatch opened");

    let own = &picture.own;
    assert_eq!(own.callsign.as_deref(), Some("RA"));
    assert_eq!(own.name.as_deref(), Some("Red Tank 1"));
    assert_eq!(own.power, Power::Online);
    assert!(own.pos_m.is_some(), "a position");
    assert_ne!(own.scope, Scope::Unknown);
    assert_eq!(own.max_ahead_kph, Some(54));
    assert_eq!(own.max_astern_kph, Some(27));
    for section in ["hull", "turret", "left_track", "right_track"] {
        let read = own
            .sections
            .get(section)
            .unwrap_or_else(|| panic!("the {section} in {:?}", own.sections));
        assert!(!read.facets.is_empty(), "the {section}'s facets");
    }
    let weapons: Vec<(&str, Option<&str>)> = picture
        .weapons
        .iter()
        .map(|w| (w.id.as_str(), w.short.as_deref()))
        .collect();
    assert_eq!(weapons, [("cannon", None), ("machine_gun", Some("mg"))]);
    assert!(
        picture
            .weapons
            .iter()
            .all(|w| w.rounds == w.rounds_max && w.rounds > 0)
    );
    let spot = picture.spots.get(&'B').expect("spot B");
    assert_eq!(spot.name, "The Crossroads");
    assert!(spot.range_m.is_some() && spot.bearing_deg.is_some());
    assert_eq!(picture.tally.shots, 0);
    for designator in picture.contacts.keys() {
        assert!(
            transcript.contains(designator.as_str()),
            "{designator} was never named"
        );
    }

    // -- a second match, the tanks posed 400 m apart: LOCK and FIRE ------------
    // Staff re-pose both tanks inside each other's sight for the next match
    // (the probe's `@arena/pose`), so `Locking` and `Firing` — the ack the
    // tally's shots rest on — are read live, off the game's own words.
    let mut god = arena_rig::Client::login(&rig, "boss", arena_rig::GOD_PASSWORD, "God").await;
    god.say("@arena/pose RA=100 97 90", "opens at cell").await;
    god.say("@arena/pose BA=140 97 270", "opens at cell").await;
    live.say("SIM TEAM red", "takes red slot").await;
    live.picture.claimed(Side::Red, 1);
    live.say("SIM START", "is boarding").await;
    let second = live
        .lines
        .iter()
        .rev()
        .find_map(|line| match line {
            Line::Boarding { match_id } => Some(*match_id),
            _ => None,
        })
        .expect("the second boarding line reads");
    assert_ne!(second, match_id);
    assert_eq!(live.picture.following, Some(second));
    assert_eq!(
        live.picture.tally,
        mush_bot::arena::picture::Tally::default(),
        "the tally is per match"
    );
    live.say("BOARD", "You climb into").await;
    live.wait_for("Good hunting", Duration::from_secs(30)).await;
    live.say("STARTUP", "power comes online shortly").await;
    live.wait_for("Power online.", Duration::from_secs(30))
        .await;
    // The hostile, once the platform has told it apart.
    let deadline = Instant::now() + Duration::from_secs(30);
    let target = loop {
        live.ask_whole("CONTACTS").await;
        let now = live.now() + 1_000;
        live.picture.settle(now);
        if let Some(designator) = live
            .picture
            .contacts
            .values()
            .find(|contact| contact.side == Some(Side::Blue))
            .map(|contact| contact.designator.clone())
        {
            break designator;
        }
        assert!(
            Instant::now() < deadline,
            "no identified hostile; the transcript:\n{}",
            live.client.seen
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert!(live.lines.iter().any(|line| matches!(
        line,
        Line::ContactRow(row) if row.designator == target && row.side == Some(Side::Blue)
    )));
    live.say(&format!("LOCK {target}"), "Locking").await;
    live.wait_for("Locked on", Duration::from_secs(30)).await;
    assert!(live.lines.contains(&Line::Locking {
        designator: target.clone(),
        friendly: false
    }));
    assert_eq!(live.picture.lock.as_deref(), Some(target.as_str()));
    assert!(!live.picture.lock_friendly);
    let now = live.now();
    live.picture.sent_fire(now);
    live.say("FIRE cannon", "Firing cannon.").await;
    assert!(live.lines.contains(&Line::Firing {
        weapon: "cannon".into()
    }));
    assert_eq!(live.picture.tally.shots, 1);
    assert_eq!(live.picture.tally.refused_orders, 0);
    assert!(
        live.picture.open.fire_sent.is_empty(),
        "the ack took the mark"
    );
    live.say("SIM ABORT", "aborts match").await;
    live.wait_for("is over (", Duration::from_secs(30)).await;
    live.client.pump(Duration::from_millis(500)).await;
    live.read();
    assert_eq!(
        live.picture.phase,
        Phase::Over {
            match_id: second,
            result: ResultWord::Abort
        }
    );
    let transcript = live.client.seen.clone();
    for designator in live.picture.contacts.keys() {
        assert!(
            transcript.contains(designator.as_str()),
            "{designator} was never named"
        );
    }
    god.close().await;
    live.client.close().await;
    drop(rig);
}
