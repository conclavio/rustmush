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
//! Nothing of the machines' own controller (`tacsim::arena::reflex` until 17b
//! cut M4.6 deleted it; the unit minds since) is imported: what the pilot does
//! is written from the player help alone.
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
    conditions::HeldWhy,
    damage::MISS_REASONS,
    intents::Refusal,
    motion::BlockedCause,
    perception::{
        Aspect, DriveOutcome, FrameContact, FrameNotice, FrameOdds, Named, NoticeKind, Receiver,
        RefusalDetail,
    },
    state::{Classification, ContactId, Designator, KeySpot, Power as ArenaPower, Team},
    step::{Frame, FrameFacet, FrameHeat, FrameSection},
    weapons::{FrameBand, FrameWeapon},
};
use tacsim::match_::{cockpit, lobby::SimError, resume};

#[path = "support/arena.rs"]
mod arena_rig;

#[path = "arena_contract/lapse_producer.rs"]
mod lapse_producer;

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
        self.event(&OutputEvent::Sys(RichText::tag(
            "arena-notice",
            sentence.to_owned(),
        )))
    }

    /// Render the actual game event, retaining its tag and notice wrapper.
    fn event(&mut self, event: &OutputEvent) -> Vec<Line> {
        self.now += 10;
        let mut bytes = Vec::new();
        mush_ansi::render(event, &self.caps, &self.theme, &mut bytes);
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
        moving: false,
        condition: None,
        // 17b cut M1a.3: a classified friend, nearer than BA. **17b cut
        // M1b.3** (Q2): the odds rows never carry one of the hull's own side,
        // so RB carries none — its cell prints the dash — and the lock on it
        // takes no row either (the frame the arena now builds).
        aspect: Aspect::Front,
        reads_dead: Some(false),
        fatal: None,
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
        mode: tacsim::arena::state::MatchMode::Skirmish,
        objective: None,
        motor: None,
        own_last_hit_ms: None,
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
                moving: false,
                condition: None,
                // 17b cut M1a.3: broadside, and the lock's own figures below.
                aspect: Aspect::Left,
                reads_dead: Some(false),
                fatal: None,
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
                moving: false,
                condition: None,
                // 17b cut M1a.3: an aspect is told, odds never are.
                aspect: Aspect::Rear,
                reads_dead: Some(false),
                fatal: None,
            },
        ],
        scan_tables: Vec::new(),
        notices: Vec::new(),
        rules: Vec::new(),
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
                // 17b cut M1b.3: no slot armed.
                armed: None,
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
                armed: None,
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
        // 17b cut M1a.3: BA, the one classified contact, led by the cannon.
        odds: vec![
            FrameOdds {
                contact: BA_ID,
                weapon: "cannon".into(),
                p_permille: 624,
            },
            FrameOdds {
                contact: BA_ID,
                weapon: "machine_gun".into(),
                p_permille: 210,
            },
        ],
        order: None,
        t_ms: 61_250,
        // 17b cut M1b.3: the shipped Compact floor.
        odds_floor_permille: 100,
        // 17b cut M5.4: the bot reads nothing of the hull's sealed mind.
        mind: None,
    }
}

/// [`frame`] on a hull with no weapons station (17b cut M1a.3; doc 17 §22's
/// italic line): no guns, no odds, no `reads_dead`.
fn unarmed_frame() -> Frame {
    let mut frame = frame();
    frame.weapons.clear();
    frame.odds.clear();
    for row in &mut frame.contacts {
        row.reads_dead = None;
    }
    frame
}

/// What the bot should read off a row's odds cell: the main gun's id and whole
/// per cent, or `None` for the dash and for an unarmed hull's row.
fn odds_read(row: &cockpit::ContactRow) -> Option<(String, u32)> {
    match &row.odds {
        cockpit::OddsCell::Odds(weapon, permille) => Some((weapon.to_string(), permille / 10)),
        cockpit::OddsCell::Dash | cockpit::OddsCell::Unarmed => None,
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
        NoticeKind::DriveArrived,
        NoticeKind::DriveNoRoute,
        // 17b cut M1b.1: a slot's two answers.
        NoticeKind::SlotFired,
        NoticeKind::SlotHeld,
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
            | NoticeKind::EmconOff
            | NoticeKind::DriveArrived
            | NoticeKind::DriveNoRoute
            | NoticeKind::SlotFired
            | NoticeKind::SlotHeld => {}
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
        Refusal::NotMachine,
        Refusal::NoCommander,
        Refusal::NoJammer,
        // 17b cut M1b.1: the law's two words and the count's.
        Refusal::Unwarranted,
        Refusal::NoSlot,
        Refusal::Unlawful,
        // 17b cut M5.2: a profile switch the arena cannot take now.
        Refusal::Profile,
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
            | Refusal::NotMachine
            | Refusal::NoCommander
            | Refusal::NoJammer
            | Refusal::Unwarranted
            | Refusal::NoSlot
            | Refusal::Unlawful
            | Refusal::Profile => {}
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
            launch_cell: None,
            contact: Some(named(BA_ID, ba())),
            reason: reason.into(),
        });
        notices.push(FrameNotice::Miss {
            launch_cell: None,
            contact: None,
            reason: reason.into(),
        });
    }
    notices.push(FrameNotice::Miss {
        launch_cell: None,
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
    // The turret's own loss (cut AL.1): the words that hold every gun until
    // `WEAPONSTATUS` answers. Last of the lost sections, so the doubt it sets
    // is its own.
    notices.push(FrameNotice::SectionLost {
        section: "turret".into(),
        consequences: vec!["remove_weapons".into()],
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
            // 17b cut M1b.1: the floor an `unlawful` refusal names.
            Refusal::Unlawful => RefusalDetail::Floor { permille: 100 },
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
    // 17b cut M5.2: a `MIND PROFILE` the arena refused, as it stages one.
    notices.push(FrameNotice::Refused {
        order: "mind".into(),
        refusal: Refusal::Profile,
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
    // 17b cut M1a.4: the motor's two answers to a `DRIVE`.
    notices.push(FrameNotice::Drive {
        outcome: DriveOutcome::Arrived,
    });
    notices.push(FrameNotice::Drive {
        outcome: DriveOutcome::NoRoute,
    });
    // 17b cut M1b.1: a slot's two notices, with and without a contact, and one
    // held notice per reason.
    notices.push(FrameNotice::SlotFired {
        slot: 1,
        weapon: "cannon".into(),
        contact: Some(named(BA_ID, ba())),
        p_permille: 620,
        aspect: Aspect::Left,
    });
    notices.push(FrameNotice::SlotFired {
        slot: 2,
        weapon: "machine_gun".into(),
        contact: None,
        p_permille: 950,
        aspect: Aspect::Rear,
    });
    for why in [
        HeldWhy::Odds {
            permille: 540,
            floor: 600,
        },
        HeldWhy::Aspect {
            contact: named(BA_ID, ba()),
            aspect: Aspect::Front,
        },
        HeldWhy::Range {
            contact: named(BA_ID, ba()),
            range_m: 1_200,
            min_m: 200,
            max_m: 600,
        },
        HeldWhy::Rounds {
            rounds: 8,
            keeps: 8,
        },
    ] {
        notices.push(FrameNotice::SlotHeld {
            slot: 1,
            weapon: "cannon".into(),
            why,
        });
    }
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
                // 17b cut M1a.3: the tail, read back as printed.
                assert_eq!(read.aspect.as_deref(), Some(row.aspect.as_str()));
                assert_eq!(read.odds, Some(odds_read(row)));
            }
            other => panic!("{} read as {other:?}", row.render()),
        }
    }
    reader.settle();
    let picture = &reader.picture;
    assert_eq!(picture.contacts.keys().collect::<Vec<_>>(), ["BA", "X2"]);
    assert_eq!(picture.contacts["BA"].side, Some(Side::Blue));
    assert_eq!(picture.contacts["X2"].side, None);
    // 17b cut M1a.3: the picture keeps the tail, and reads nothing off it yet.
    assert_eq!(picture.contacts["BA"].aspect.as_deref(), Some("side"));
    assert_eq!(
        picture.contacts["BA"].odds_percent,
        Some(("cannon".to_owned(), 62))
    );
    assert_eq!(picture.contacts["X2"].aspect.as_deref(), Some("rear"));
    assert_eq!(picture.contacts["X2"].odds_percent, None, "the dash");

    // -- CONTACTS on an unarmed hull: every row ends at the side word ---------
    let unarmed = unarmed_frame();
    let mut reader = Reader::new();
    for row in &cockpit::frame_contacts(&unarmed) {
        let printed = row.render();
        assert!(printed.ends_with(&format!("  {}", row.aspect)), "{printed}");
        match reader.one(&printed) {
            Line::ContactRow(read) => {
                assert_eq!(read.aspect.as_deref(), Some(row.aspect.as_str()));
                assert_eq!(read.odds, Some(None), "{printed}");
            }
            other => panic!("{printed} read as {other:?}"),
        }
    }

    // -- a row WITHOUT the tail (an older server) still parses ---------------
    let mut reader = Reader::new();
    match reader.one("BA  blue  bearing 087° (rel 032°)  far  450 m  2 s") {
        Line::ContactRow(read) => {
            assert_eq!(read.designator, "BA");
            assert_eq!(read.aspect, None);
            assert_eq!(read.odds, None);
        }
        other => panic!("a tail-less row read as {other:?}"),
    }

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
    // 17b cut M1b.3 (Q2): a friend's row prints the dash, which reads as no
    // odds — the reader's code unchanged (its tail is optional).
    assert_eq!(
        friend_rows
            .iter()
            .find(|row| row.designator.to_string() == "RB")
            .and_then(odds_read),
        None,
        "a friend's row carries no odds"
    );
    assert_eq!(reader.picture.contacts["RB"].odds_percent, None, "the dash");
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
                    | NoticeKind::EmconOff
                    | NoticeKind::DriveArrived
                    | NoticeKind::DriveNoRoute
                    | NoticeKind::SlotFired
                    | NoticeKind::SlotHeld => assert!(
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
            FrameNotice::Miss {
                contact, reason, ..
            } => assert!(
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
            } => {
                assert!(
                    matches!(&read, Line::SectionLost { section: s, cost }
                        if s == section.as_str() && cost.is_some() == !consequences.is_empty()),
                    "{}",
                    fail()
                );
                // The bot holds every gun on exactly the words the game says
                // for a section whose loss takes its weapons (cut AL.1).
                assert_eq!(
                    reader.picture.weapons_doubt.is_some(),
                    consequences.iter().any(|c| c == "remove_weapons"),
                    "{}",
                    fail()
                );
            }
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
            // 17b cut M1b.1 defined a slot's two notices and 17b cut M1b.4
            // stages them; the bot reads neither and opens no reply on them
            // (every shape: `m1b4_the_arming_its_line_and_its_sentences_…`).
            FrameNotice::SlotFired { .. } | FrameNotice::SlotHeld { .. } => {
                assert_eq!(read, Line::Other(sentence.clone()), "{}", fail());
            }
            // 17b cut M1a.4: the bot reads neither of the motor's answers.
            FrameNotice::Drive { .. } => {
                assert_eq!(read, Line::Other(sentence.clone()), "{}", fail());
            }
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
        // Cut AL.1: a gun the tank does not carry (every `FIRE` of a gun lost
        // with its turret, in the bot's own transcripts).
        (
            cockpit::CockpitError::NoSuchWeapon("cannon".into()),
            Line::NoSuchWeapon {
                weapon: "cannon".into(),
            },
        ),
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

    // -- the reclaim catch-up block and the reconnect line ----------------------
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
    // The game's 17b cut M5.3: `resume::mind_line` (`doctrine_line`, `… is
    // flying on doctrine; …`, until then), read as `Line::OnItsMind`.
    assert_eq!(
        reader.one(&resume::mind_line(&world, tank)),
        Line::OnItsMind {
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
    live.picture.sent_fire(now, "cannon");
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
    live.picture.sent_fire(now, "cannon");
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

// ---------------------------------------------------------------------------
// Native ordered rules through the player-facing SSH station
// ---------------------------------------------------------------------------

/// Native rules remain player-facing terminal text. Their acknowledgements and
/// complete readbacks are neutral to the independent pilot; a Sensors refusal
/// names LOCK without inventing a weapon or a successful lock. Exercise the
/// actual station over SSH, including its own wrapping and asynchronous frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5rules_the_bot_reads_native_rules_over_its_own_ssh_client() {
    tokio::time::timeout(Duration::from_secs(90), native_rule_lines())
        .await
        .expect("the native-rule contract finishes inside 90 s");
}

async fn native_rule_lines() {
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
    live.say("ooc", "Welcome Hall").await;
    live.say("simulator", "Simulator Lobby").await;
    live.say("SIM TEAM red", "takes red slot").await;
    live.say("SIM START", "is boarding").await;
    live.say("red", "Red Room").await;
    live.say("BOARD", "You climb into").await;
    live.wait_for("Good hunting", Duration::from_secs(30)).await;
    live.say("STARTUP", "power comes online shortly").await;
    live.wait_for("Power online.", Duration::from_secs(30))
        .await;

    // Heat guards keep all three rules inert; this contract observes their
    // public words without teaching the bot a controller or issuing a shot.
    for (command, echo) in [
        (
            "MIND RULE 1 FIRE cannon WHEN odds>=60 AND range<700 AND target=moving AND heat>99%",
            "Queued: cannon fires at your identified lock",
        ),
        (
            "MIND RULE 2 FIRE cannon HOLD WHEN heat>80%",
            "Queued: cannon holds fire",
        ),
        (
            "MIND RULE 3 LOCK nearest WHEN heat>99%",
            "Queued: lock the nearest identified hostile",
        ),
    ] {
        let from = live.lines.len();
        live.say(command, echo).await;
        assert!(
            live.lines[from..]
                .iter()
                .any(|line| matches!(line, Line::Other(text) if text.starts_with(echo))),
            "the queued rule is neutral: {command}"
        );
    }
    let readback = native_rules_until(
        &mut live,
        &[
            "S1: cannon fires",
            "S2: cannon holds fire",
            "S3: lock the nearest",
        ],
        &[],
    )
    .await;
    let whole = readback.split_whitespace().collect::<Vec<_>>().join(" ");
    for term in [
        "Current station rules (manual changes are not saved):",
        "its chance to hit is at least 60%",
        "its rounded range is less than 700 m",
        "the target is moving",
        "your heat is above 99% of its maximum",
        "S2: cannon holds fire when your heat is above 80% of its maximum",
        "S3: lock the nearest identified hostile when your heat is above 99% of its maximum",
    ] {
        assert!(whole.contains(term), "missing {term:?}: {readback}");
    }
    for line in readback.lines().filter(|line| {
        line.starts_with("Current station rules (")
            || line.starts_with("S1:")
            || line.starts_with("S2:")
            || line.starts_with("S3:")
    }) {
        assert!(matches!(classify(line), Line::Other(_)), "{line}");
    }

    let single = live.ask_whole("MIND RULES 2").await;
    assert!(single.contains("S2: cannon holds fire"), "{single}");
    assert!(
        !single.contains("S1:") && !single.contains("S3:"),
        "{single}"
    );
    let weapons = live.ask_whole("WEAPONSTATUS").await;
    assert!(weapons.contains("S1: cannon fires"), "{weapons}");
    assert!(weapons.contains("S2: cannon holds fire"), "{weapons}");
    assert!(weapons.contains("S3: lock the nearest"), "{weapons}");
    let settled_ms = live.now() + 1_000;
    live.picture.settle(settled_ms);
    let ids: Vec<&str> = live
        .picture
        .weapons
        .iter()
        .map(|weapon| weapon.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["cannon", "machine_gun"],
        "rules must not replace weapon rows"
    );
    assert!(live.picture.open.contacts.is_none());
    assert!(live.picture.open.status.is_none());
    assert!(!live.picture.open.lock_line && !live.picture.open.brief);
    assert_eq!(live.picture.tally.shots, 0);

    let from = live.lines.len();
    live.say("LOCK nearest WHEN nope", "LOCK refused.").await;
    assert!(
        live.lines[from..].iter().any(|line| matches!(line,
            Line::Refused { order, weapon: None, cycling_s: None, said }
                if order == "LOCK" && said == "That rule does not fit this core's native grammar."
        )),
        "a Sensors rule refusal names the ordering verb"
    );
    assert_eq!(live.picture.tally.shots, 0);
    assert_eq!(live.picture.tally.refused_shots, 0);
    assert_eq!(live.picture.tally.refused_orders, 1);
    assert!(
        live.picture.lock.is_none(),
        "a rule echo must not read as a lock"
    );

    live.say("MIND RULE 2 OFF", "Clearing rule S2.").await;
    native_rules_until(&mut live, &["S1:", "S3:"], &["S2:"]).await;
    live.say("FIRE ALL OFF", cockpit::CLEARED_ALL).await;
    native_rules_until(&mut live, &["S3:"], &["S1:", "S2:"]).await;
    live.say("MIND RULE 3 OFF", "Clearing rule S3.").await;
    native_rules_until(&mut live, &["None armed."], &["S1:", "S2:", "S3:"]).await;
    live.say("SIM ABORT", "aborts match").await;
    live.client.close().await;
    drop(rig);
}

/// Wait for the station's public mirror after a queued mutation. The bot never
/// inspects the arena's rules, scheduler, memory or authority tables.
async fn native_rules_until(live: &mut Live, present: &[&str], absent: &[&str]) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let answer = live.ask_whole("MIND RULES").await;
        if present.iter().all(|word| answer.contains(word))
            && absent.iter().all(|word| !answer.contains(word))
        {
            return answer;
        }
        assert!(
            Instant::now() < deadline,
            "the rules never settled: {answer}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A landing can reject a rule whose named contact is no longer classified.
/// Only explicit floor detail may claim an odds-floor violation; Sensors
/// refusals stay weaponless in the independent reader.
#[test]
fn m5rules_a_landing_refusal_does_not_invent_an_odds_floor() {
    let frame = frame();
    let rows = cockpit::frame_contacts(&frame);
    let mut reader = Reader::new();
    for (order, weapon) in [("fire", Some("cannon")), ("lock", None)] {
        for (detail, why) in [
            (
                RefusalDetail::None,
                "That rule is not lawful for this unit.",
            ),
            (
                RefusalDetail::Floor { permille: 100 },
                "The Compact floor is 10%; a condition may not ask for less.",
            ),
        ] {
            let line = cockpit::notice_line(
                &frame,
                &FrameNotice::Refused {
                    order: order.into(),
                    refusal: Refusal::Unlawful,
                    weapon: weapon.map(Into::into),
                    detail,
                },
                &rows,
            );
            assert_eq!(
                reader.one(&line),
                Line::Refused {
                    order: order.to_ascii_uppercase(),
                    weapon: weapon.map(str::to_owned),
                    said: why.to_owned(),
                    cycling_s: None,
                },
                "{line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cut AN.2: `VIEW BRIEF`'s header
// ---------------------------------------------------------------------------

/// **`VIEW BRIEF`'s header reads as the game prints it** (cut AN.2): the
/// cockpit's own `brief_header` over a hand-built window payload, through the
/// server's line renderer and the bot's own pipeline, reads as the header with
/// the arena's size in cells, the metres a cell and the own cell, and starts a
/// ground of that size. The bearing lines' grammar is held by the end-to-end
/// test from the shipped muster (`arena_e2e`): their formatter needs the
/// atlas's terrain classes, which are no dependency of this crate's.
#[test]
fn the_view_brief_header_reads_as_the_game_prints_it() {
    let header = cockpit::brief_header(
        "RA",
        90_000,
        &serde_json::json!({"origin_cell": [0, 88], "width": 39, "height": 20,
            "extent": [256, 256], "you": [6, 9], "cell_cm": 1000}),
    );
    let mut reader = Reader::new();
    assert_eq!(
        reader.one(&header),
        Line::ViewHeader {
            extent: (256, 256),
            cell_m: 10,
            you: (6, 97),
        },
        "{header}"
    );
    let ground = reader.picture.ground.clone().expect("a ground");
    assert_eq!(ground.side_m, Some((2_560.0, 2_560.0)));
    assert_eq!(ground.cell_m, Some(10));
    assert!(ground.rays.is_empty());
    assert!(reader.picture.open.brief);
    // A blue tank's at the far muster, three-digit cells and all.
    let header = cockpit::brief_header(
        "BA",
        270_000,
        &serde_json::json!({"origin_cell": [217, 88], "width": 39, "height": 20,
            "extent": [256, 256], "you": [32, 9], "cell_cm": 1000}),
    );
    assert_eq!(
        Reader::new().one(&header),
        Line::ViewHeader {
            extent: (256, 256),
            cell_m: 10,
            you: (249, 97),
        },
        "{header}"
    );
}

// ---------------------------------------------------------------------------
// The game's cut AP.4: the brief's own empty sentence and its end line
// ---------------------------------------------------------------------------

/// The seventeen lines `VIEW BRIEF` says of the canonical scene, as the game's
/// `tests/cockpit.rs` pins them against `cockpit::brief_lines` (16b cut AG.1,
/// the end line since cut AP.4). Read here as text: the brief's formatter
/// needs the atlas's terrain classes, which are no dependency of this crate's.
const BRIEF_GOLDEN: &str = include_str!("../../../web/src/lib/fixtures/cockpit/view_brief.txt");

/// **The brief's two new sentences read as the game prints them** (the
/// game's cut AP.4, 16c `N20`, a declared reader change under doc 16 §11):
/// `cockpit::NO_CONTACTS_IN_WINDOW` reads as `Line::NoContactsInWindow` and
/// `cockpit::BRIEF_END_LINE` as `Line::BriefEnd`, through the server's line
/// renderer and the bot's own pipeline. A whole brief — the golden's
/// seventeen lines — read line by line opens the brief at its header, keeps
/// it open through its contact sentences and spot lines, and closes on the
/// end line with the ground whole; the same brief over an empty frame (its
/// empty sentence in the contacts' place) does too, and its empty sentence
/// and end line open nothing in the lane's ledger.
#[test]
fn the_briefs_empty_sentence_and_end_line_read_as_the_game_prints_them() {
    assert_eq!(cockpit::NO_CONTACTS_IN_WINDOW, "No contacts in the window.");
    assert_eq!(cockpit::BRIEF_END_LINE, "End of brief.");
    // Read alone (outside a brief) each is its own line and changes nothing.
    let mut reader = Reader::new();
    assert_eq!(
        reader.one(cockpit::NO_CONTACTS_IN_WINDOW),
        Line::NoContactsInWindow
    );
    assert_eq!(reader.one(cockpit::BRIEF_END_LINE), Line::BriefEnd);
    assert!(!reader.picture.open.brief);
    assert!(reader.picture.ground.is_none());

    // The golden, whole.
    let golden: Vec<&str> = BRIEF_GOLDEN.lines().collect();
    assert_eq!(golden.len(), 17, "{BRIEF_GOLDEN}");
    assert_eq!(golden.last(), Some(&cockpit::BRIEF_END_LINE));
    let mut reader = Reader::new();
    let last = golden.len() - 1;
    for (n, line) in golden.iter().enumerate() {
        let read = reader.one(line);
        if n == 0 {
            assert!(matches!(read, Line::ViewHeader { .. }), "{line}");
        }
        if (1..=8).contains(&n) {
            assert!(matches!(read, Line::GroundRay { .. }), "{line}");
        }
        if n == last {
            assert_eq!(read, Line::BriefEnd, "{line}");
            assert!(!reader.picture.open.brief, "closed by its end line");
        } else {
            assert!(reader.picture.open.brief, "open at line {n}: {line}");
        }
    }
    let ground = reader.picture.ground.clone().expect("a ground");
    assert!(ground.complete);
    assert_eq!(ground.rays.len(), 8);
    assert_eq!(ground.side_m, Some((960.0, 640.0)));
    assert!(
        reader.picture.contacts.is_empty(),
        "a brief's contact sentences add no contact"
    );

    // The same window over an empty frame: header, rays, the empty sentence,
    // the end line. Through the renderer, then through the picture alone, where
    // what each line opened is read.
    let mut quiet: Vec<&str> = golden[..9].to_vec();
    quiet.push(cockpit::NO_CONTACTS_IN_WINDOW);
    quiet.push(cockpit::BRIEF_END_LINE);
    let mut reader = Reader::new();
    let read: Vec<Line> = quiet.iter().map(|line| reader.one(line)).collect();
    assert_eq!(read[9], Line::NoContactsInWindow);
    assert_eq!(read[10], Line::BriefEnd);
    assert!(!reader.picture.open.brief);
    assert!(
        reader
            .picture
            .ground
            .as_ref()
            .is_some_and(|ground| ground.complete)
    );
    let mut picture = Picture::default();
    for (n, line) in quiet.iter().enumerate() {
        let applied = picture.apply(10 + n as u64, &classify(line));
        if n >= 9 {
            assert_eq!(applied.opened, None, "{line} opens nothing");
        }
    }
    assert!(!picture.open.brief);
    // And a `CONTACTS` empty reply after the brief opens its own.
    let applied = picture.apply(100, &classify("No contacts."));
    assert_eq!(
        applied.opened,
        Some(mush_bot::arena::lines::AskKind::Contacts)
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1a.4: the motor's two answers to a `DRIVE`
// ---------------------------------------------------------------------------

/// **The `DRIVE` notices read as nothing the reader parses** (17b cut M1a.4;
/// doc 17 §4 D-UM-10): `DRIVE: arrived.` and `DRIVE: no route from here;
/// holding.`, each through the cockpit's own `notice_line` and the bot's own
/// pipeline, read `Line::Other` byte for byte and open no `CONTACTS`,
/// `WEAPONSTATUS` or `STATUS` reply, no lock line and no brief. The bot does
/// not drive (yet): it must not mistake the motor's sentences for a reply.
#[test]
fn the_drive_notices_read_as_neutral_lines_and_open_no_reply() {
    let frame = frame();
    let mut reader = Reader::new();
    let lines: Vec<String> = [DriveOutcome::Arrived, DriveOutcome::NoRoute]
        .into_iter()
        .map(|outcome| cockpit::notice_line(&frame, &FrameNotice::Drive { outcome }, &[]))
        .collect();
    assert_eq!(
        lines,
        ["DRIVE: arrived.", "DRIVE: no route from here; holding."]
    );
    for line in &lines {
        assert_eq!(reader.one(line), Line::Other(line.clone()), "{line}");
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// ---------------------------------------------------------------------------
// Cut AP.3: the `WATCH` digest
// ---------------------------------------------------------------------------

/// **The `WATCH` digest reads as nothing the reader parses** (16b cut AP.3;
/// 16c N19, a declared reader change under doc 16 §11): the cockpit's own
/// `watch_lines` over the hand-built scene (locked on `BA`, two contacts, a
/// cycling gun: three lines) and over a bare frame (no gun, no contact: one
/// line), each through the server's line renderer and the bot's own pipeline,
/// reads `Line::Other` byte for byte — line by line, and as the one block the
/// bridge pushes — and opens no `CONTACTS`, `WEAPONSTATUS` or `STATUS` reply
/// and no brief. The prefixes were chosen so (the own line says `online`,
/// never `power online`, which is what `STATUS`'s head is split on); this
/// pins that they stay so.
#[test]
fn the_watch_digest_reads_as_neutral_lines_and_opens_no_reply() {
    let scene = frame();
    let mut bare = frame();
    bare.weapons.clear();
    bare.contacts.clear();
    bare.lock = None;
    let mut reader = Reader::new();
    for (frame, count) in [(&scene, 3), (&bare, 1)] {
        let lines = cockpit::watch_lines(
            frame,
            &std::collections::BTreeMap::new(),
            &cockpit::frame_weapons(frame),
            1_000,
        );
        assert_eq!(lines.len(), count, "{lines:?}");
        for line in &lines {
            assert_eq!(reader.one(line), Line::Other(line.clone()), "{line}");
        }
        let block = lines.join("\n");
        assert_eq!(
            reader.say(&block),
            lines
                .iter()
                .map(|line| Line::Other(line.clone()))
                .collect::<Vec<Line>>(),
            "{block}"
        );
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// ---------------------------------------------------------------------------
// 17b cut L.2: the lapse sentence in the leader's name
// ---------------------------------------------------------------------------

/// **The order-lapse sentences read as nothing the reader parses** (17b cut
/// L.2; D-UM-32 as amended by R-C, D-UM-53): the world's line when an order
/// runs out unrenewed — the pilot's own (`your`) and a lance-leader's (its
/// persona's name), byte for byte as `arena_bridge`'s own pin prints them —
/// read `Line::Other` and open no `CONTACTS`, `WEAPONSTATUS` or `STATUS`
/// reply, no lock line and no brief. The bot does not act on them.
///
/// The game's 17b cut M5.6 moved both tails, `back on doctrine.` → `back on
/// their minds.` (the orchestrator's D9 ruling at the game's M5.3 close: one
/// word for every mind) — a declared reader change, the bot and the server
/// deploying together. The old tails stay neutral too, so a bot reading an
/// older server parses nothing of them either.
#[test]
fn the_order_lapse_sentences_read_as_neutral_lines_and_open_no_reply() {
    let mut reader = Reader::new();
    for line in [
        "Squad red-1: your hold order has run out; back on their minds.",
        "Squad red-1: red lance leader 1's hold order has run out; back on their minds.",
        "Squad red-1: your hold order has run out; back on doctrine.",
        "Squad red-1: red lance leader 1's hold order has run out; back on doctrine.",
    ] {
        assert_eq!(reader.one(line), Line::Other(line.to_owned()), "{line}");
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// ---------------------------------------------------------------------------
// 17b cut M3.4: the tripped sentence, a hull dropped to the ROM
// ---------------------------------------------------------------------------

/// **The core-dropped sentences read as nothing the reader parses** (the
/// game's 17b cut M3.4; doc 17 §8, D-UM-32): the eight player shapes the
/// world says when a hull drops to the ROM — `Your` and the fill's `RB's`,
/// each on an `ops` strike (the budget), any other strike (the last word), a
/// destroyed core and an engine that cannot run the mind — byte for byte as
/// `tacsim::arena_bridge::dropped_line` prints them (and as its own pin
/// does), through the server's line renderer and the bot's own pipeline,
/// read `Line::Other` and open no `CONTACTS`, `WEAPONSTATUS` or `STATUS`
/// reply, no lock line and no brief. Every one fits the 120-column line
/// whole. The bot does not act on them; its reader needed no change.
#[test]
fn m3_the_core_dropped_sentences_read_as_neutral_lines_and_open_no_reply() {
    use tacsim::arena::mind::DropCause;
    use tacsim::arena_bridge::dropped_line;
    // `(whose, cause, fault, ops, sentence)`.
    type Shape<'a> = (&'a str, DropCause, Option<&'a str>, Option<u64>, &'a str);
    let shapes: [Shape<'_>; 8] = [
        (
            "Your",
            DropCause::Strikes,
            Some("ops"),
            Some(1_500),
            "Your core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "Your",
            DropCause::Strikes,
            Some("limit"),
            Some(1_500),
            "Your core tripped: three faults running, the last limit. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "Your",
            DropCause::Core,
            None,
            None,
            "Your core is destroyed. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "Your",
            DropCause::Abi,
            None,
            None,
            "Your core cannot run its mind on this engine. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "RB's",
            DropCause::Strikes,
            Some("ops"),
            Some(1_500),
            "RB's core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "RB's",
            DropCause::Strikes,
            Some("limit"),
            Some(1_500),
            "RB's core tripped: three faults running, the last limit. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "RB's",
            DropCause::Core,
            None,
            None,
            "RB's core is destroyed. The hull is on the ROM; MIND is off for the match.",
        ),
        (
            "RB's",
            DropCause::Abi,
            None,
            None,
            "RB's core cannot run its mind on this engine. The hull is on the ROM; MIND is off for the match.",
        ),
    ];
    let mut reader = Reader::new();
    for (whose, cause, fault, ops, line) in shapes {
        assert_eq!(
            dropped_line(whose, cause, fault, ops),
            line,
            "the game's words"
        );
        assert_eq!(reader.one(line), Line::Other(line.to_owned()), "{line}");
    }
    // The widest budget a class carries (III's 18,000) still fits one line.
    let widest = dropped_line("RB's", DropCause::Strikes, Some("ops"), Some(18_000));
    assert_eq!(reader.say(&widest), vec![Line::Other(widest.clone())]);
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

/// **The turret's loss with its core words reads as the bot reads it** (the
/// game's 17b cut M3.6; doc 17 §9, D-UM-34): the shipped line tanks' and the
/// scout's turret now lose `[remove_weapons, destroy_core]` and the heavy's
/// `[remove_weapons, degrade_core]`, so the pilot's `SectionLost` line reads
/// `The turret is destroyed: its weapons are gone; its core is gone.` (or
/// `…; its core is degraded.`) through the cockpit's own `notice_line`. Each
/// reads `Line::SectionLost` for the turret with its cost, raises the weapons
/// doubt as the bare turret line does (the reader's `contains("its weapons
/// are gone")` still matches), and the core clause is read as nothing: no
/// immobility, no reply opened. The bot's reader needed no change.
#[test]
fn m3_the_turret_lost_with_its_core_words_raises_the_weapons_doubt_and_nothing_else() {
    let frame = frame();
    let rows = cockpit::frame_contacts(&frame);
    for (word, sentence) in [
        (
            "destroy_core",
            "The turret is destroyed: its weapons are gone; its core is gone.",
        ),
        (
            "degrade_core",
            "The turret is destroyed: its weapons are gone; its core is degraded.",
        ),
    ] {
        let notice = FrameNotice::SectionLost {
            section: "turret".into(),
            consequences: vec!["remove_weapons".into(), word.into()],
        };
        let said = cockpit::notice_line(&frame, &notice, &rows);
        assert_eq!(said, sentence, "the game's words for {word}");
        let mut reader = Reader::new();
        let read = reader.one(&said);
        assert!(
            matches!(&read, Line::SectionLost { section, cost: Some(cost) }
                if section == "turret" && cost.contains("its core")),
            "{said:?} read as {read:?}"
        );
        assert!(
            reader.picture.weapons_doubt.is_some(),
            "{said:?}: the weapons doubt is raised as for the bare turret line"
        );
        assert!(!reader.picture.own.immobile, "{said:?}: no immobility read");
        let open = &reader.picture.open;
        assert!(open.contacts.is_none(), "a CONTACTS reply opened");
        assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
        assert!(open.status.is_none(), "a STATUS reply opened");
    }
}

// ---------------------------------------------------------------------------
// 17b cut M1b.4: the arming, its line and its two sentences
// ---------------------------------------------------------------------------

/// **The game's 17b cut M5.1: the selection fog, read as the game prints
/// it** (a declared reader change). A board row whose slot chose a mind with
/// `SIM MIND` ends `, a mind` for every reader but its own pilot and staff
/// (`lobby::board_mind_clause`), after the chassis bracket: the reader strips
/// it into `BoardPilot::mind` and keeps the name, the bracket and the
/// callsign; a slot that chose nothing prints nothing. A side's fill choice
/// is a board row of its own (`lobby::board_fill_row`), which is no side row
/// and reads as a neutral `Line::Other` whoever may read it. The check-out's
/// new call-off — a chosen mind no longer held, naming the slot — reads as
/// the call-off it is. The bot never types `SIM MIND` (`arena_e2e.rs`'
/// `forbidden`), so its own row never carries the named form.
#[test]
fn m5_1_the_selection_fog_reads_as_the_bot_reads_it() {
    use mush_bot::arena::lines::BoardPilot;
    use tacsim::match_::lobby::{self, MindChoice};
    let choice = MindChoice {
        filed: None,
        saved: None,
        mind: "Standard Doctrine".into(),
        profiles: vec!["Sniper".into(), "Line".into()],
    };
    let row = format!(
        "  red: 1 (RA): Ada [biped_mk1]{}; 2 (RB): Bo{}; 3 (RC): Cy{}",
        lobby::board_mind_clause(Some(&choice), false),
        lobby::board_mind_clause(Some(&choice), false),
        lobby::board_mind_clause(None, false),
    );
    let pilot =
        |index: u8, callsign: &str, name: &str, unit_type: Option<&str>, mind: bool| BoardPilot {
            index,
            callsign: callsign.into(),
            name: name.into(),
            unit_type: unit_type.map(str::to_owned),
            mind,
            pod: None,
            standing: None,
        };
    assert_eq!(
        classify(&row),
        Line::BoardSide {
            side: Side::Red,
            pilots: vec![
                pilot(1, "RA", "Ada", Some("biped_mk1"), true),
                pilot(2, "RB", "Bo", None, true),
                pilot(3, "RC", "Cy", None, false),
            ]
        },
        "{row}"
    );
    for reads in [false, true] {
        let fill = lobby::board_fill_row(Team::Blue, &choice, reads);
        assert_eq!(
            classify(&fill),
            Line::Other(fill.trim().to_owned()),
            "the fill row is no side row: {fill}"
        );
    }
    let why = SimError::MindNotHeld {
        team: "red",
        index: Some(1),
    }
    .to_string();
    let call_off = format!(
        "Match 4 could not start: {}. The pods are free again.",
        why.trim_end_matches('.')
    );
    assert_eq!(
        classify(&call_off),
        Line::CouldNotStart {
            match_id: 4,
            why: "Slot red 1's chosen mind is no longer held by its pilot. The pods are free \
                  again."
                .into()
        }
    );
}

/// **The pilot's arming surface reads as the bot reads it** (the game's 17b
/// cut M1b.4; doc 17 §5, a declared reader change under doc 16 §11):
///
/// * the slot's two sentences — `Cannon fires at BA by S1 (62%, side).` with
///   and without a designator, and `S1 held the cannon: …` under every why and
///   the three range shapes — through the cockpit's own `notice_line`, read
///   `Line::Other` byte for byte and open no reply;
/// * the arming's reply and the two `OFF` replies read `Line::Other`;
/// * a `WEAPONSTATUS` answer whose cannon is armed reads the lock line, the
///   cannon's row, `Line::WeaponArming { slot: 1, terms }` and the machine
///   gun's row — and the arming line ends no reply, so both guns land on the
///   picture;
/// * `FIRE cannon refused. The Compact floor is 10%; a condition may not ask
///   for less.` — the arena's `unlawful` refusal with its floor, through
///   `notice_line` — reads `Line::Refused { order: "FIRE", weapon: "cannon" }`
///   and opens no reply; so do the count's and the outside refusal's
///   sentences, through `order_refused` with the cockpit's own errors.
#[test]
fn m1b4_the_arming_its_line_and_its_sentences_read_as_the_bot_reads_them() {
    use tacsim::arena::{ASPECTS_ANY, FrameArming, SlotTarget, aspect_mask};
    let frame = frame();
    let rows = cockpit::frame_contacts(&frame);
    let ba = rows
        .iter()
        .find(|row| row.designator.to_string() == "BA")
        .map(|row| Named {
            id: row.id,
            designator: row.designator,
        })
        .expect("the scene's BA");
    let held = |why: HeldWhy| FrameNotice::SlotHeld {
        slot: 1,
        weapon: "cannon".into(),
        why,
    };
    let sentences: Vec<(FrameNotice, &str)> = vec![
        (
            FrameNotice::SlotFired {
                slot: 1,
                weapon: "cannon".into(),
                contact: Some(ba),
                p_permille: 624,
                aspect: Aspect::Left,
            },
            "Cannon fires at BA by S1 (62%, side).",
        ),
        (
            FrameNotice::SlotFired {
                slot: 2,
                weapon: "machine_gun".into(),
                contact: None,
                p_permille: 950,
                aspect: Aspect::Rear,
            },
            "Machine_gun fires by S2 (95%, rear).",
        ),
        (
            held(HeldWhy::Odds {
                permille: 540,
                floor: 600,
            }),
            "S1 held the cannon: 54% under your 60%.",
        ),
        (
            held(HeldWhy::Aspect {
                contact: ba,
                aspect: Aspect::Front,
            }),
            "S1 held the cannon: BA shows its front.",
        ),
        (
            held(HeldWhy::Range {
                contact: ba,
                range_m: 650,
                min_m: 200,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 650 m, outside 200 to 600.",
        ),
        (
            held(HeldWhy::Range {
                contact: ba,
                range_m: 650,
                min_m: 0,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 650 m, outside under 600.",
        ),
        (
            held(HeldWhy::Range {
                contact: ba,
                range_m: 150,
                min_m: 200,
                max_m: u32::MAX,
            }),
            "S1 held the cannon: BA at 150 m, outside past 200.",
        ),
        (
            held(HeldWhy::Rounds {
                rounds: 7,
                keeps: 8,
            }),
            "S1 held the cannon: 7 rounds, keeps 8.",
        ),
    ];
    let mut reader = Reader::new();
    for (notice, said) in &sentences {
        let line = cockpit::notice_line(&frame, notice, &rows);
        assert_eq!(line, *said);
        assert_eq!(reader.one(&line), Line::Other(line.clone()), "{line}");
    }
    for line in [
        "Arming cannon: fires your lock when your chance to hit is 60% or better and it shows \
         its side or rear.",
        "Cleared: cannon fires only when you say.",
        cockpit::CLEARED_ALL,
    ] {
        assert_eq!(reader.one(line), Line::Other(line.to_owned()), "{line}");
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
    assert_eq!(
        reader.picture.tally.shots, 0,
        "a slot's shot is not the bot's"
    );

    // -- WEAPONSTATUS with the cannon armed -----------------------------------
    let mut armed = frame.clone();
    let slot = FrameArming {
        slot: 1,
        target: SlotTarget::Lock,
        min_permille: 600,
        aspects: aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]),
        keeps: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    };
    assert_ne!(slot.aspects, ASPECTS_ANY);
    armed.weapons[0].armed = Some(slot);
    let weapons = cockpit::frame_weapons(&armed);
    let mut answer = vec![cockpit::lock_line(&armed).expect("a lock line")];
    for weapon in &weapons {
        answer.push(weapon.render(&weapons));
        if let Some(armed) = &weapon.armed {
            answer.push(format!("  {}", cockpit::arming_words(armed)));
        }
    }
    assert_eq!(answer[2], "  S1: >=60% side/rear");
    let mut reader = Reader::new();
    let read: Vec<Line> = answer.iter().map(|line| reader.one(line)).collect();
    assert!(matches!(read[0], Line::Lock { .. }), "{read:?}");
    assert!(
        matches!(&read[1], Line::WeaponRow(row) if row.id == "cannon"),
        "{read:?}"
    );
    assert_eq!(
        read[2],
        Line::WeaponArming {
            slot: 1,
            terms: ">=60% side/rear".into()
        }
    );
    assert!(
        matches!(&read[3], Line::WeaponRow(row) if row.id == "machine_gun"),
        "{read:?}"
    );
    assert!(
        reader
            .picture
            .open
            .weapons
            .as_ref()
            .is_some_and(|open| open.rows.len() == 2),
        "the arming line ended the WEAPONSTATUS reply"
    );
    reader.settle();
    let ids: Vec<&str> = reader
        .picture
        .weapons
        .iter()
        .map(|w| w.id.as_str())
        .collect();
    assert_eq!(ids, ["cannon", "machine_gun"]);
    // And the same answer with nothing armed parses as it always did.
    let mut plain = Reader::new();
    let bare = cockpit::frame_weapons(&frame);
    for weapon in &bare {
        assert!(matches!(
            plain.one(&weapon.render(&bare)),
            Line::WeaponRow(_)
        ));
    }

    // -- the three refusals of an arming --------------------------------------
    let unlawful = cockpit::notice_line(
        &frame,
        &FrameNotice::Refused {
            order: "fire".into(),
            refusal: Refusal::Unlawful,
            weapon: Some("cannon".into()),
            detail: RefusalDetail::Floor { permille: 100 },
        },
        &rows,
    );
    assert_eq!(
        unlawful,
        "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
    );
    let count = cockpit::order_refused(
        "fire",
        Some("cannon"),
        &cockpit::CockpitError::NoSlot.to_string(),
    );
    let outside = cockpit::order_refused(
        "fire",
        Some("cannon"),
        &cockpit::CockpitError::NotThisCore.to_string(),
    );
    assert_eq!(
        outside,
        "FIRE cannon refused. That condition is not one this core takes; help cockpit lists the \
         supported native rules."
    );
    for line in [&unlawful, &count, &outside] {
        let mut picture = Picture::default();
        let read = classify(line);
        assert!(
            matches!(&read, Line::Refused { order, weapon: Some(w), .. }
                if order == "FIRE" && w == "cannon"),
            "{line} read as {read:?}"
        );
        let applied = picture.apply(10, &read);
        assert_eq!(applied.opened, None, "{line} opened a reply");
        assert!(picture.open.weapons.is_none() && picture.open.contacts.is_none());
        let mut reader = Reader::new();
        assert_eq!(reader.one(line), read, "{line} through the renderer");
    }
}

// ---------------------------------------------------------------------------
// 17b cut M5.2: the profile refusal, the seal line and Q-UM-15's sentence
// ---------------------------------------------------------------------------

/// **The game's 17b cut M5.2, read as the game prints it** (a declared
/// reader change; the bot never types `MIND`, so none of these is an answer
/// to it):
///
/// * the arena's `profile` refusal of a `MIND PROFILE` it cannot take,
///   through the cockpit's own `notice_line` —
///   `MIND refused. That profile cannot be switched to now.` — reads
///   `Line::Refused { order: "MIND", weapon: None }` and opens no reply;
/// * the seal line a choosing pilot alone reads before `Good hunting.`
///   (`lobby::seal_line`), in its three shapes — a chosen Doctrine set at
///   the widest preset pair, a substituted choice, a non-Doctrine mind —
///   and Q-UM-15's `SIM START` sentence read `Line::Other` byte for byte,
///   fit the 120-column line, and open no reply;
/// * the check-out's call-off by hull — `Match 12 could not start: RA's mind
///   is over its core's class. The pods are free again.` and (the cut's
///   review) `… RA's card could not be sealed. …` — reads
///   `Line::CouldNotStart`, under 120.
#[test]
fn m5_2_the_profile_refusal_and_the_seal_lines_read_as_the_bot_reads_them() {
    use tacsim::match_::lobby;
    let frame = frame();
    let rows = cockpit::frame_contacts(&frame);
    let refused = cockpit::notice_line(
        &frame,
        &FrameNotice::Refused {
            order: "mind".into(),
            refusal: Refusal::Profile,
            weapon: None,
            detail: RefusalDetail::None,
        },
        &rows,
    );
    assert_eq!(
        refused,
        "MIND refused. That profile cannot be switched to now."
    );
    let mut reader = Reader::new();
    let read = reader.one(&refused);
    assert!(
        matches!(&read, Line::Refused { order, weapon: None, cycling_s: None, .. }
            if order == "MIND"),
        "{refused} read as {read:?}"
    );
    let seal = |mind: &str, profiles: &[&str]| tacsim::arena::MindSeal {
        mind: mind.into(),
        class: tacsim::arena::core::CoreClass::I,
        cards: m6_cards(profiles),
        ..tacsim::arena::MindSeal::placeholder()
    };
    let lines = [
        (
            lobby::seal_line(
                &seal("Standard Doctrine", &["Skirmisher", "Rearguard"]),
                false,
            ),
            "Sealed for this sortie: Standard Doctrine (Skirmisher, Rearguard), Class I; MIND \
             reads it.",
        ),
        (
            lobby::seal_line(&seal("Standard Doctrine", &[]), true),
            "Sealed for this sortie: Standard Doctrine (Line), Class I, for humans on the other \
             side; MIND reads it.",
        ),
        (
            lobby::seal_line(&seal("Fixture I", &[]), false),
            "Sealed for this sortie: Fixture I (Line), Class I; MIND reads it.",
        ),
        (
            Some(lobby::HUMANS_ON_THE_OTHER_SIDE.to_owned()),
            "Humans on the other side: your fill and any released hull fly Standard Doctrine.",
        ),
    ];
    for (said, line) in lines {
        assert_eq!(said.as_deref(), Some(line), "the game's words");
        assert!(line.chars().count() < 120, "{line}");
        assert_eq!(reader.one(line), Line::Other(line.to_owned()), "{line}");
    }
    // The check-out's two new call-offs by hull (the over-class backstop,
    // and since the cut's review a card the seal refuses), as the lobby
    // composes them off `StartError::call_off_words`: the bot's
    // `CouldNotStart`, naming no mind, class or profile.
    let world = World::new_minimal();
    for (refused, why) in [
        (
            tacsim::match_::StartError::MindOverClass {
                unit: ObjId(9),
                callsign: "RA".into(),
                rated: tacsim::arena::core::CoreClass::III,
                core: tacsim::arena::core::CoreClass::II,
            },
            "RA's mind is over its core's class",
        ),
        (
            tacsim::match_::StartError::CardRefused {
                unit: ObjId(9),
                callsign: "RA".into(),
                refusal: tacsim::arena::CardError::Pair {
                    slider: "Rearguard",
                    fight_home: "75".to_owned(),
                    break_off: "70".to_owned(),
                },
            },
            "RA's card could not be sealed",
        ),
    ] {
        let words = refused.call_off_words(&world, &[]);
        assert_eq!(words, why, "the game's words");
        let line = format!("Match 12 could not start: {words}. The pods are free again.");
        assert!(line.chars().count() < 120, "{line}");
        assert_eq!(
            reader.one(&line),
            Line::CouldNotStart {
                match_id: 12,
                why: format!("{why}. The pods are free again."),
            },
            "{line}"
        );
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// ---------------------------------------------------------------------------
// The game's 17b cut M5.3: release to the sealed mind
// ---------------------------------------------------------------------------

/// **The game's 17b cut M5.3: the release's words, read as the game prints
/// them** (a declared reader change; the design note's C-11, D11 and D14):
///
/// * the board's drop clause, the lobby's own `DROP_CLAUSE` (what
///   `board_standing_clause` returns for a drop) — `, flown by its mind`
///   (`, flown by doctrine` until that cut, and exactly as wide) — reads
///   `Standing::Mind`, the resume standing the bot follows a running match
///   on; an `EXIT`'s `, left` still reads `Standing::Left` and a seated pilot
///   `Standing::Seated`, each with its pod and its name whole;
/// * the reconnect line, through `resume::mind_line` — `Red Tank 1 is flying
///   on its mind; BOARD takes it back.` — reads `Line::OnItsMind` and puts the
///   pilot out of the tank;
/// * the release acknowledgement, singular and plural, through the cockpit's
///   own `order_ack` — `Released red-1 to its mind: …` — reads `Line::Other`
///   byte for byte (the bot never types `ORDER`, so it is never an answer to
///   it);
/// * D14's catch-up line, through `catchup_lines` — `  The machine was flying
///   on its own mind under no order.` — reads `Line::Other`, and the block
///   around it reads as every catch-up block does;
///
/// the reconnect line, both release forms and every line of D14's block each
/// under 120 columns, and none opening a reply.
#[test]
fn m5_3_the_release_words_read_as_the_bot_reads_them() {
    use mush_bot::arena::lines::{BoardPilot, Standing};
    use tacsim::match_::{cockpit::ObjectiveWords, lobby};
    // The board: a dropped, an exited and a seated slot, in the game's words.
    let dropped = lobby::DROP_CLAUSE;
    assert_eq!(dropped, ", flown by its mind", "the game's word");
    // `board_standing_clause(dropped, left, seated)` as `status_lines` asks
    // it: a drop (left by a drop, the slot still seated on its unit), an
    // `EXIT` (left, no drop) and a pilot in the cockpit.
    assert_eq!(lobby::board_standing_clause(true, true, true), dropped);
    let exited = lobby::board_standing_clause(false, true, true);
    let seated = lobby::board_standing_clause(false, false, true);
    let row = format!(
        "  red: 1 (RA): Pilot, pod Red Pod 1{dropped}; 2 (RB): Bo, pod Red Pod 2{exited}; 3 (RC): \
         Cy, pod Red Pod 3{seated}"
    );
    // The clause is as wide as the one it replaced (`, flown by doctrine`,
    // 19 columns): no board row grows.
    assert_eq!(
        dropped.chars().count(),
        ", flown by doctrine".chars().count()
    );
    let pilot = |index: u8, callsign: &str, name: &str, standing: Standing| BoardPilot {
        index,
        callsign: callsign.into(),
        name: name.into(),
        unit_type: None,
        mind: false,
        pod: Some(format!("Red Pod {index}")),
        standing: Some(standing),
    };
    assert_eq!(
        classify(&row),
        Line::BoardSide {
            side: Side::Red,
            pilots: vec![
                pilot(1, "RA", "Pilot", Standing::Mind),
                pilot(2, "RB", "Bo", Standing::Left),
                pilot(3, "RC", "Cy", Standing::Seated),
            ]
        },
        "{row}"
    );
    // The reconnect line.
    let mut reader = Reader::new();
    let mut world = World::new_minimal();
    let tank = world
        .create(ObjKind::Thing, "Red Tank 1", GOD, None)
        .expect("a tank");
    let reconnect = resume::mind_line(&world, tank);
    assert_eq!(
        reconnect,
        "Red Tank 1 is flying on its mind; BOARD takes it back."
    );
    assert!(reconnect.chars().count() < 120, "{reconnect}");
    reader.picture.aboard = true;
    assert_eq!(
        reader.one(&reconnect),
        Line::OnItsMind {
            tank: "Red Tank 1".into()
        }
    );
    assert!(!reader.picture.aboard, "the reconnect puts the pilot out");
    // The release acknowledgement, in both forms.
    for (who, want) in [
        (
            vec!["red-1".to_owned()],
            "Released red-1 to its mind: it fights on its own until it is ordered again.",
        ),
        (
            vec!["red-1".to_owned(), "red-2".to_owned()],
            "Released red-1 and red-2 to their minds: they fight on their own until they are \
             ordered again.",
        ),
    ] {
        let said = cockpit::order_ack(&who, &ObjectiveWords::Release, None, None, false, 0);
        assert_eq!(said, want, "the game's words");
        assert!(said.chars().count() < 120, "{said}");
        assert_eq!(reader.one(&said), Line::Other(said.clone()), "{said}");
    }
    // D14: the catch-up of a hull a mind that is not Doctrine flew.
    let catchup = Catchup {
        away_ms: 95_000,
        cell_then: Some((20, 30)),
        cell_now: Some((24, 31)),
        power_then: ArenaPower::Online,
        power_now: ArenaPower::Online,
        wreck: false,
        contacts_new: vec![Designator::Provisional(4)],
        contacts_lost: vec![],
        contacts_more: 0,
        sections: vec![],
        sections_more: 0,
        rounds_spent: vec![("cannon".into(), 2)],
        rounds_more: 0,
        machine: Some(MachineWords {
            state: "unknown".into(),
            goal: Some((26, 31)),
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
    let d14 = "  The machine was flying on its own mind under no order.";
    assert_eq!(block.get(1).map(String::as_str), Some(d14), "{block:?}");
    for line in &block {
        assert!(line.chars().count() < 120, "{line}");
    }
    assert_eq!(
        classify(d14),
        Line::Other(d14.trim_start().to_owned()),
        "the D14 line"
    );
    let lines = reader.say(&block.join("\n"));
    assert_eq!(lines.len(), block.len());
    assert_eq!(
        lines[0],
        Line::BackAboard {
            callsign: "RA".into(),
            away_ms: 95_000
        }
    );
    assert!(
        lines[1..]
            .iter()
            .all(|line| matches!(line, Line::Other(_) | Line::CatchupPower { .. })),
        "{lines:?}"
    );
    assert!(reader.picture.aboard, "the catch-up is a reclaim");
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

/// **The game's 17b cut M5.6: the reworded acknowledgements read as nothing
/// the reader parses** — a declared reader change (the bot and the server
/// deploy together):
///
/// * the two `SUPPORT` acknowledgements, through the cockpit's own `order_ack`
///   — `Ordered red-1 to support RA: it keeps within 150 m of RA and fights
///   what it finds there. …` and its `holding fire` form, which said `fires
///   first on what RA is fighting` / `locks what RA is fighting` until that
///   cut (the owner's OQ-6, the text made true) — read `Line::Other`, the
///   head and the wrapped tail alike (the bot never types `ORDER`), each
///   byte for byte the game's and narrower than the sentence it
///   replaced (145 and 170 columns with the ten-minute tail, against 153 and
///   176; the tail has carried every order line past 120 since 16b cut Y.1);
/// * the sixth first-time hint, `tacsim::match_::tutorial::HINT_LINES[5]`,
///   reads `Line::Hint` as every hint does;
///
/// none opening a reply. (The order lapse's two tails are
/// `the_order_lapse_sentences_read_as_neutral_lines_and_open_no_reply`'s.)
#[test]
fn m5_6_the_reworded_acknowledgements_and_the_mind_hint_read_as_the_bot_reads_them() {
    use tacsim::match_::cockpit::ObjectiveWords;
    let mut reader = Reader::new();
    let one = vec!["red-1".to_owned()];
    let support = ObjectiveWords::Support {
        callsign: "ra".to_owned(),
    };
    for (hold_fire, width, want) in [
        (
            false,
            145,
            "Ordered red-1 to support RA: it keeps within 150 m of RA and fights what it finds \
             there. It stands for 10 minutes or until your next order to it.",
        ),
        (
            true,
            170,
            "Ordered red-1 to support RA, holding fire: it keeps within 150 m of RA and locks \
             what it finds but fires nothing. It stands for 10 minutes or until your next order \
             to it.",
        ),
    ] {
        let said = cockpit::order_ack(&one, &support, Some("RA"), None, hold_fire, 150);
        assert_eq!(said, want, "the game's words");
        assert_eq!(said.chars().count(), width, "{said}");
        // Past 120 columns the sentence wraps: its head and its tail each
        // read `Other` (`Reader::one` asserts the tail).
        assert!(
            matches!(reader.one(&said), Line::Other(head) if said.starts_with(&head)),
            "{said}"
        );
    }
    let hint = tacsim::match_::tutorial::HINT_LINES[5];
    assert!(hint.chars().count() < 120, "{hint}");
    assert!(
        matches!(reader.one(hint), Line::Hint(_)),
        "the mind hint reads as a hint: {hint}"
    );
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

/// **The game's 17b cut M5.6: `SIM REPORT`'s `Your mind` block and its
/// reworded release line read as nothing the reader parses** — a declared
/// reader change (17b cut M5.6's review, R10; conventions §6.6), though the
/// bot never types `SIM REPORT` and so never reads either today: every shape
/// of the block's lines, byte for byte as the game's own pins print them
/// (`match_::report`'s `m5_*` tests), and the timeline's `released red-1 to
/// their minds.` (`to doctrine.` until that cut, the M5.3 close's D9), each
/// read `Line::Other` and open no `CONTACTS`, `WEAPONSTATUS` or `STATUS`
/// reply, no lock line and no brief.
#[test]
fn m5_6_the_your_mind_block_and_the_release_line_read_as_neutral_lines() {
    let mut reader = Reader::new();
    for line in [
        "Your mind: Standard Doctrine (Line).",
        "Your mind: Standard Doctrine (Sniper, Line); switched to Line at 1:05.",
        "Your mind: Standard Doctrine (Sniper, Line); switched to Sniper, edited (min_odds 70) at \
         1:05.",
        "Your mind: Standard Doctrine (Sniper, Line); switched 2 times.",
        "  Its last switch was to Line at 1:30.",
        "  Sealed in place of your choice, for humans on the other side.",
        "  It never flew: you flew the tank the whole match.",
        "  It flew your tank from 0:42, when you climbed out, to the end.",
        "  It flew your tank from 0:42, when you climbed out, to 3:12.",
        "  It flew your tank from 0:42 to 1:10, while your line was down.",
        "  It flew your tank from 0:42 to the end, while your line was down.",
        "  It flew your tank from 0:42 to 1:10 and from 2:05 to the end, while your line was down.",
        "  It flew your tank from 0:42 to 1:10 and from 2:05 to the end.",
        "  It flew your tank 5 times: first from 0:42 to 1:10, last from 12:05 to the end.",
        "  Its shots: cannon 4, 2 of them hits; it destroyed nothing.",
        "  Its shots: cannon 1 and machine_gun 1, 2 of them hits; it destroyed 1 unit.",
        "  It fired nothing.",
        "  It dropped to the ROM at 1:30: three faults running.",
        "  It dropped to the ROM at 1:30: its core was destroyed.",
        "  It dropped to the ROM at 1:30: its core could not run its mind on this engine.",
        "  It dropped to the ROM at 1:30.",
        "  1:02  released red-1 to their minds.",
        "  1:02  released red-1 to doctrine.",
    ] {
        assert!(line.chars().count() < 120, "{line}");
        assert!(
            matches!(reader.one(line), Line::Other(_)),
            "{line} read as something"
        );
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// ---------------------------------------------------------------------------
// The game's 17b cut M5.4: the `MIND` verb in line mode
// ---------------------------------------------------------------------------

/// **The game's 17b cut M5.4: every `MIND` line, read as the game prints it**
/// (a declared reader change; the bot never types `MIND` — it joins
/// `arena_e2e.rs`' `forbidden` — so none of these is ever an answer to it):
///
/// * bare `MIND`'s block, through the cockpit's own `mind_lines` — the head
///   in its five shapes (a Doctrine seal at its live classes, a mind of the
///   pilot's own, a row dropped to the ROM, a hull with no core above Class
///   0, a hull sealed at Class 0 on a real core), the profile line in its three (a pair at the widest presets, the set
///   of one, a countdown on the mirror's clock), both channel lines and the
///   rules line — `MIND RULES`' line, `This hull carries no seal.`, the
///   switch's acknowledgement and the verb's usage line each read
///   `Line::Other` byte for byte and open no reply;
/// * every `MIND PROFILE` pre-check — a preset not sealed, a word that is no
///   preset, the profile already flown, the ten seconds, no seal, no frame,
///   the wreck — composed through `order_refused` as the arena's own
///   `profile` refusal is, reads `Line::Refused { order: "MIND", weapon:
///   None }` as that refusal does (17b cut M5.2), its countdown never read as
///   a gun's `cycling` seconds — reader cases, not neutral lines (declared:
///   the design note's item 5 gives every one the `MIND refused.` prefix,
///   which the reader reads as a refusal of the verb, as it reads the
///   arena's own);
/// * `MIND TRACE refused. No mind trace is available.` — the shared unavailable
///   or unauthorized trace refusal introduced in M6 — a
///   form refused, two words before `refused` and the second no weapon id —
///   reads `Line::Other`;
///
/// each under 120 columns.
#[test]
fn m5_4_the_mind_lines_read_as_the_bot_reads_them() {
    use tacsim::arena::core::CoreClass;
    use tacsim::arena::step::FrameMind;
    let sniper_line = m6_frame(&["Sniper", "Line"], 0);
    let widest = FrameMind {
        switch_at_ms: Some(40_000),
        ..m6_frame(&["Skirmisher", "Rearguard"], 1)
    };
    let mut neutral: Vec<String> = Vec::new();
    for (mind, armed, now_ms) in [
        (sniper_line.clone(), true, 0),
        (widest.clone(), false, 34_000),
        (widest.clone(), true, 40_000),
        (
            FrameMind {
                doctrine: false,
                name: "Field Pilot".into(),
                ..m6_frame(&["Line"], 0)
            },
            false,
            0,
        ),
        (
            FrameMind {
                off: true,
                class: CoreClass::Zero,
                ..sniper_line.clone()
            },
            true,
            0,
        ),
        (
            FrameMind {
                class: CoreClass::Zero,
                core_class: CoreClass::Zero,
                ..sniper_line.clone()
            },
            false,
            0,
        ),
        (
            FrameMind {
                class: CoreClass::Zero,
                ..sniper_line.clone()
            },
            true,
            0,
        ),
    ] {
        neutral.extend(cockpit::mind_lines(&mind, armed, now_ms));
        neutral.push(cockpit::mind_rules_line(&mind));
    }
    neutral.push(cockpit::NO_SEAL.to_owned());
    neutral.push(cockpit::profile_ack("Sniper"));
    neutral.push(cockpit::MIND_USAGE.to_owned());
    neutral.push(cockpit::order_refused(
        "mind trace",
        None,
        cockpit::MIND_NO_TRACE,
    ));
    // The game's words, in each shape.
    for line in [
        "Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II core.",
        "Mind: Field Pilot, sealed for this sortie; it thinks at Class I on a Class II core.",
        "Mind: Standard Doctrine, off for the match: the hull is on the ROM.",
        "Mind: Standard Doctrine, sealed for this sortie; with no core above Class 0, the hull is \
         on the ROM.",
        "Mind: Standard Doctrine, sealed for this sortie at Class 0 on a Class II core: the hull \
         is on the ROM.",
        "Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds.",
        "Profile: Rearguard, 2 of 2 (also Skirmisher); one switch in ten seconds, the next in 6 s.",
        "Profile: Line, 1 of 1; no other is sealed to switch to.",
        "Channels: you hold helm, sensors and fire; the mind is dormant while you fly.",
        "Channels: you hold helm and sensors (no weapons station on this hull); the mind is \
         dormant while you fly.",
        "Rules: none sealed; your own armings are under WEAPONSTATUS.",
        "No rules are sealed on Sniper or Line; your own armings are under WEAPONSTATUS.",
        "This hull carries no seal.",
        "Profile Sniper: the mind will fly it when you leave the seat.",
        "MIND TRACE refused. No mind trace is available.",
    ] {
        assert!(
            neutral.iter().any(|said| said == line),
            "the game no longer says {line:?}: {neutral:#?}"
        );
    }
    let mut reader = Reader::new();
    for line in &neutral {
        assert!(line.chars().count() < 120, "{line}");
        assert_eq!(reader.one(line), Line::Other(line.clone()), "{line}");
    }

    // Every pre-check of a switch, in the arena's own refusal's shape.
    let refused = |said: &str| cockpit::order_refused("mind", None, said);
    let mut checks: Vec<String> = ["Hunter", "berserker", "Sniper"]
        .into_iter()
        .filter_map(|word| cockpit::profile_switch(&sniper_line, word, 0).err())
        .map(|said| refused(&said))
        .collect();
    checks.extend(
        cockpit::profile_switch(&widest, "Skirmisher", 34_000)
            .err()
            .map(|said| refused(&said)),
    );
    checks.push(refused(cockpit::NO_SEAL));
    checks.push(refused(cockpit::NO_FRAME_YET));
    checks.push(refused(cockpit::DESTROYED));
    assert_eq!(
        checks,
        [
            "MIND refused. Hunter is not sealed on this mind; it carries Sniper and Line.",
            "MIND refused. No profile of that name is sealed on this mind; it carries Sniper and \
             Line.",
            "MIND refused. Sniper is already the profile.",
            "MIND refused. One switch in ten seconds; the next in 6 s.",
            "MIND refused. This hull carries no seal.",
            "MIND refused. The panel has told you nothing yet.",
            "MIND refused. Your tank is destroyed and takes no orders. EXIT leaves the pod.",
        ]
    );
    for line in &checks {
        assert!(line.chars().count() < 120, "{line}");
        let read = reader.one(line);
        assert!(
            matches!(&read, Line::Refused { order, weapon: None, cycling_s: None, said }
                if order == "MIND" && line.ends_with(said.as_str())),
            "{line} read as {read:?}"
        );
    }
    let open = &reader.picture.open;
    assert!(open.contacts.is_none(), "a CONTACTS reply opened");
    assert!(open.weapons.is_none(), "a WEAPONSTATUS reply opened");
    assert!(open.status.is_none(), "a STATUS reply opened");
    assert!(!open.lock_line, "a lock line was read");
    assert!(!open.brief, "a VIEW BRIEF opened");
}

// Renderer fixtures carry actual canonical profiles and bound programs, so the
// line contract follows the same complete card shape as a checked-out hull.
fn m6_cards(names: &[&str]) -> tacsim::arena::profile_seal::SealedCards {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let names = if names.is_empty() {
        &["Line"][..]
    } else {
        names
    };
    let cfg = tacsim::config::ReflexConfig::default();
    let selected = names
        .iter()
        .map(|name| {
            (
                (*name).into(),
                tacsim::arena::preset(name, &cfg).expect("fixture preset"),
            )
        })
        .collect::<Vec<_>>();
    let definition = tacsim::arena::runtime_card::stock_definition(&selected, CoreClass::III)
        .expect("full cards");
    let names = selected
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    tacsim::arena::profile_seal::SealedCards::prepare(
        definition,
        &names,
        &ClassTable::STOCK.row(CoreClass::III),
        &[],
        &[
            tacsim::arena::Channel::Helm,
            tacsim::arena::Channel::Sensors,
        ],
        100,
    )
    .expect("sealed fixture profiles")
}

fn m6_frame(names: &[&str], active: u8) -> tacsim::arena::step::FrameMind {
    let cards = m6_cards(names);
    let mut catalog = tacsim::arena::runtime_card::CardCatalog::default();
    let (profiles, _) = cards.admit(&mut catalog).expect("catalog admission");
    let card = catalog
        .resolve(profiles.key(active).expect("active selected key"))
        .expect("active card");
    tacsim::arena::step::FrameMind {
        name: "Standard Doctrine".into(),
        doctrine: true,
        profiles,
        card,
        program: cards.programs[usize::from(active)].clone(),
        active,
        class: tacsim::arena::core::CoreClass::I,
        core_class: tacsim::arena::core::CoreClass::II,
        switch_at_ms: None,
        off: false,
    }
}
