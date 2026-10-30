#![cfg(test)]

use super::*;

/// Build actual immutable cards for renderer fixtures; names never reconstruct runtime data.
fn m6_frame(names: &[&str]) -> FrameMind {
    use crate::arena::core::{ClassTable, CoreClass};
    use crate::arena::profile_seal::SealedCards;
    use crate::arena::runtime_card::{CardCatalog, stock_definition};
    let cfg = crate::config::ReflexConfig::default();
    let selected = names
        .iter()
        .map(|name| {
            (
                SmolStr::new(*name),
                crate::arena::preset(name, &cfg).expect("fixture preset"),
            )
        })
        .collect::<Vec<_>>();
    let definition = stock_definition(&selected, CoreClass::III).expect("full cards");
    let cards = SealedCards::prepare(
        definition,
        &[],
        &ClassTable::STOCK.row(CoreClass::III),
        &[],
        &[Channel::Helm, Channel::Sensors],
        100,
    )
    .expect("sealed cards");
    let mut catalog = CardCatalog::default();
    let (_, card) = cards.admit(&mut catalog).expect("catalog admission");
    // Retain the complete selected order for profile renderer tests.
    let names = names
        .iter()
        .map(|name| SmolStr::new(*name))
        .collect::<Vec<_>>();
    let profiles =
        crate::arena::runtime_card::RuntimeProfiles::bind(&catalog, card.key().definition, &names)
            .expect("selected profiles");
    FrameMind {
        name: SmolStr::new_static("Standard Doctrine"),
        doctrine: true,
        profiles,
        card,
        program: cards.programs[0].clone(),
        active: 0,
        class: CoreClass::I,
        core_class: CoreClass::II,
        switch_at_ms: None,
        off: false,
    }
}

use crate::arena::intents::Refusal;
// 17b cut M1a.2: the class word moved to `arena::view`, and only the
// tests here still build a `Classification`.
use crate::arena::state::Classification;

/// The wire's `catchup` object, as `catchup_lines.json` writes it (16b cut
/// Y.4) — a serde mirror, so the fixture is read as data and not as a
/// second copy of the builder.
#[derive(serde::Deserialize)]
struct WireWords {
    condition: String,
    armour: i64,
    internals: i64,
}

#[derive(serde::Deserialize)]
struct WireSection {
    id: String,
    then: WireWords,
    now: WireWords,
}

#[derive(serde::Deserialize)]
struct WireRounds {
    weapon: String,
    rounds: u32,
}

#[derive(serde::Deserialize)]
struct WireMachine {
    state: String,
    goal: Option<(u16, u16)>,
    objective: Option<String>,
}

#[derive(serde::Deserialize)]
struct WireCatchup {
    away_s: i64,
    cell_then: Option<(u16, u16)>,
    cell_now: Option<(u16, u16)>,
    power_then: Power,
    power_now: Power,
    wreck: bool,
    contacts_new: Vec<String>,
    contacts_lost: Vec<String>,
    contacts_more: u32,
    sections: Vec<WireSection>,
    sections_more: u32,
    rounds_spent: Vec<WireRounds>,
    rounds_more: u32,
    machine: Option<WireMachine>,
}

#[derive(serde::Deserialize)]
struct WireCase {
    callsign: String,
    catchup: WireCatchup,
    lines: Vec<String>,
}

/// A designator as a frame prints it, read back: `X<n>`, or a team letter
/// and a bijective base-26 slot.
fn designator(word: &str) -> Designator {
    if let Some(number) = word.strip_prefix(crate::arena::state::PROVISIONAL_PREFIX) {
        return Designator::Provisional(number.parse().expect("a provisional number"));
    }
    let mut letters = word.chars();
    let team = match letters.next() {
        Some('R') => crate::arena::state::PartyId::RED,
        Some('B') => crate::arena::state::PartyId::BLUE,
        other => panic!("no side is lettered {other:?}"),
    };
    let slot = letters.fold(0_u16, |slot, letter| {
        slot * 26 + (u16::from(u8::try_from(letter).expect("ASCII")) - u16::from(b'A') + 1)
    });
    Designator::Slot { team, slot }
}

fn words(wire: &WireWords) -> SectionWords {
    SectionWords {
        condition: SmolStr::new(&wire.condition),
        armour: wire.armour,
        internals: wire.internals,
    }
}

fn decoded(wire: &WireCatchup) -> Catchup {
    Catchup {
        away_ms: wire.away_s * 1_000,
        cell_then: wire.cell_then,
        cell_now: wire.cell_now,
        power_then: wire.power_then,
        power_now: wire.power_now,
        wreck: wire.wreck,
        contacts_new: wire.contacts_new.iter().map(|w| designator(w)).collect(),
        contacts_lost: wire.contacts_lost.iter().map(|w| designator(w)).collect(),
        contacts_more: wire.contacts_more,
        sections: wire
            .sections
            .iter()
            .map(|s| (SmolStr::new(&s.id), words(&s.then), words(&s.now)))
            .collect(),
        sections_more: wire.sections_more,
        rounds_spent: wire
            .rounds_spent
            .iter()
            .map(|r| (SmolStr::new(&r.weapon), r.rounds))
            .collect(),
        rounds_more: wire.rounds_more,
        machine: wire
            .machine
            .as_ref()
            .map(|m| crate::arena::catchup::MachineWords {
                state: SmolStr::new(&m.state),
                goal: m.goal,
                objective: m.objective.as_deref().map(SmolStr::new),
            }),
    }
}

/// **16b cut Y.4**: `catchup_lines` says every case of the committed parity
/// fixture word for word — the file the TUI's `catchup_words` and the web's
/// `catchupLines` are pinned to as well — and `catchup_json` writes each
/// case's object back exactly as the fixture holds it, so the fixture is
/// the wire's own shape.
#[test]
fn y4_catchup_lines_say_every_case_of_the_parity_fixture() {
    let cases: Vec<WireCase> = serde_json::from_str(include_str!(
        "../../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
    ))
    .expect("catchup_lines.json is an array of cases");
    // Was 9 until 17b cut M5.3, which added the tenth (D14: the machine
    // `flying on its own mind`).
    assert_eq!(cases.len(), 10, "the ten shapes the fixture pins");
    let raw: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
    ))
    .expect("the same file, as values");
    for (case, raw) in cases.iter().zip(&raw) {
        let report = decoded(&case.catchup);
        assert_eq!(
            catchup_lines(designator(&case.callsign), &report),
            case.lines,
            "{}",
            case.lines.first().map_or("", String::as_str)
        );
        assert_eq!(
            Some(&catchup_json(&report)),
            raw.get("catchup"),
            "the fixture is the wire's shape"
        );
    }
}

/// **17b cut M5.3 (D14, doc 17 §24's index)**: the machine's `unknown` —
/// a hull flown by a mind that is not Standard Doctrine, whose memory says
/// nothing in the doctrine's words
/// ([`crate::arena::doctrine::MachineState::Unknown`]) — is said as the
/// mind's own, `  The machine was flying on its own mind under no order.`,
/// and no longer as the opened word (`… was unknown under no order.` until
/// that cut). Line mode here; the TUI's `catchup_words` (`mush-net-ssh`'s
/// test of the same name) and the web's `catchupLines` (vitest, the same
/// name) read the committed `catchup_lines.json`, whose tenth case IS this
/// line, so the three renderers say one sentence. The six doctrine states
/// keep their words; the line names no mind, so it is one sentence for
/// every mind that is not Doctrine, and every one fits the 120-column line.
#[test]
fn m5_d14_the_catch_up_says_its_own_mind_in_three_renderers() {
    use crate::arena::catchup::MachineWords;
    use crate::arena::doctrine::MachineState;
    const D14: &str = "  The machine was flying on its own mind under no order.";
    let base: WireCatchup = serde_json::from_value(serde_json::json!({
        "away_s": 30, "cell_then": [4, 4], "cell_now": [5, 4],
        "power_then": "online", "power_now": "online", "wreck": false,
        "contacts_new": [], "contacts_lost": [], "contacts_more": 0,
        "sections": [], "sections_more": 0,
        "rounds_spent": [], "rounds_more": 0, "machine": null
    }))
    .expect("a wire object");
    let mut report = decoded(&base);
    let callsign = designator("RA");
    // The arena's own words for such a hull, as `Arena::machine_state`
    // reads it and the `Live` keeps it.
    report.machine = Some(MachineWords::of_mind(MachineState::Unknown, None, None));
    let lines = catchup_lines(callsign, &report);
    assert_eq!(lines.get(1).map(String::as_str), Some(D14), "{lines:?}");
    assert!(
        !lines.iter().any(|line| line.contains("unknown under")),
        "the opened word is gone: {lines:?}"
    );
    // Under each order a machine can hold, the same phrase, under 120.
    for objective in ["hold", "advance", "engage", "withdraw", "screen", "support"] {
        report.machine = Some(crate::arena::catchup::MachineWords {
            state: SmolStr::new_static(MachineState::Unknown.as_str()),
            goal: None,
            objective: Some(SmolStr::new(objective)),
        });
        let line = catchup_lines(callsign, &report).swap_remove(1);
        assert!(
            line.starts_with("  The machine was flying on its own mind under a"),
            "{line}"
        );
        assert!(line.chars().count() < 120, "{line}");
    }
    // The six doctrine states keep their Y.4 words; only `unknown` moved.
    for state in MachineState::DOCTRINE {
        let words = machine_state_words(state.as_str());
        assert_ne!(words, "flying on its own mind", "{state:?}");
        assert!(!words.contains("mind"), "{state:?}: {words}");
    }
    assert_eq!(
        machine_state_words(MachineState::Unknown.as_str()),
        "flying on its own mind"
    );
    assert_eq!(
        MachineState::ALL.last(),
        Some(&MachineState::Unknown),
        "the one state the doctrine never writes"
    );
    // The committed parity fixture carries the line, so the TUI and the
    // web are pinned to these bytes too.
    let cases: Vec<WireCase> = serde_json::from_str(include_str!(
        "../../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
    ))
    .expect("catchup_lines.json is an array of cases");
    let said: Vec<&WireCase> = cases
        .iter()
        .filter(|case| {
            case.catchup
                .machine
                .as_ref()
                .is_some_and(|machine| machine.state == MachineState::Unknown.as_str())
        })
        .collect();
    assert_eq!(said.len(), 1, "one case of the fixture is D14's");
    let case = said[0];
    assert_eq!(case.lines.get(1).map(String::as_str), Some(D14));
    assert_eq!(
        catchup_lines(designator(&case.callsign), &decoded(&case.catchup)),
        case.lines
    );
}

/// **16b cut Y.4**: `away_s` is the floor of the milliseconds, on the wire
/// and on line 1 alike, and a negative count reads nought.
#[test]
fn y4_the_duration_is_the_wires_floor() {
    let mut report = decoded(
        &serde_json::from_value::<WireCatchup>(serde_json::json!({
            "away_s": 0, "cell_then": null, "cell_now": null,
            "power_then": "off", "power_now": "off", "wreck": false,
            "contacts_new": [], "contacts_lost": [], "contacts_more": 0,
            "sections": [], "sections_more": 0,
            "rounds_spent": [], "rounds_more": 0, "machine": null
        }))
        .expect("a wire object"),
    );
    report.away_ms = 134_999;
    let callsign = designator("RA");
    assert_eq!(
        catchup_lines(callsign, &report).first().map(String::as_str),
        Some("Back aboard RA after 2 m 14 s away.")
    );
    assert_eq!(
        catchup_json(&report).get("away_s"),
        Some(&serde_json::json!(134))
    );
    report.away_ms = -1;
    assert_eq!(
        catchup_lines(callsign, &report).first().map(String::as_str),
        Some("Back aboard RA after 0 s away.")
    );
}

#[test]
fn the_cold_refusal_carries_the_arenas_own_word() {
    // The cockpit refuses before the arena does; the two must read alike, so
    // the sentence has to contain the arena's own word for it.
    let word = Refusal::PowerNotOnline.as_str().replace('_', " ");
    assert!(
        POWER_NOT_ONLINE.to_ascii_lowercase().contains(&word),
        "`{POWER_NOT_ONLINE}` does not contain `{word}`"
    );
}

/// **Every refusal has a sentence, and the sentence is somebody's voice**
/// (16b cut F.3; F.2's judge).
///
/// Cut F.2 ended [`refusal_line`]'s match with a catch-all that formatted the
/// `thiserror` text, so a later cut routing a new variant into phase 4 would
/// have shipped `FIRE refused. Refused: that unit already has as many actions
/// queued as it may.` The match is exhaustive now; this is the other half,
/// which is that each arm says something a pilot can read. The three the judge
/// named — `Overheated`, `Wreck`, `Unknown` — are included with the rest,
/// because a list that skips the awkward ones is the gap all over again.
#[test]
fn every_refusal_has_a_sentence_a_pilot_can_read() {
    use crate::arena::perception::RefusalDetail;
    for refusal in [
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
        // 16b cut O.2.
        Refusal::NoJammer,
    ] {
        // 16b cut D.2 (`B-16`): the weapon is in the prefix now, so a pilot
        // with two guns reads which breech was refused.
        let line = refusal_line("fire", refusal, Some("cannon"), RefusalDetail::None);
        assert!(
            line.starts_with("FIRE cannon refused. "),
            "{refusal:?}: {line}"
        );
        let said = line.trim_start_matches("FIRE cannon refused. ");
        assert!(!said.is_empty(), "{refusal:?} says nothing");
        assert!(
            !said.starts_with("Refused:"),
            "{refusal:?} fell through to a catch-all: {line}"
        );
        assert!(said.ends_with('.'), "{refusal:?} is not a sentence: {line}");
        let opener = said.chars().next().unwrap_or(' ');
        assert!(
            opener.is_ascii_uppercase(),
            "{refusal:?} opens in lower case: {line}"
        );
    }
}

/// The three the judge named, in full, so their exact bytes are pinned.
#[test]
fn the_overheated_the_wreck_and_the_stray_unit_each_have_their_own_line() {
    use crate::arena::perception::RefusalDetail;
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Overheated,
            Some("laser"),
            RefusalDetail::Overheated { heat: 60, max: 60 },
        ),
        format!(
            "FIRE laser refused. {}",
            CockpitError::Overheated { heat: 60, max: 60 }
        )
    );
    // With no numbers behind it — a refusal staged before the cell was read —
    // it falls back to the cockpit's own constant rather than inventing a
    // nought that would read as "cold".
    assert_eq!(
        refusal_line("fire", Refusal::Overheated, None, RefusalDetail::None),
        format!("FIRE refused. {OVERHEATED}: let it cool.")
    );
    assert_eq!(
        refusal_line("fire", Refusal::Wreck, None, RefusalDetail::None),
        "FIRE refused. Your tank is a wreck."
    );
    assert_eq!(
        refusal_line("startup", Refusal::Unknown, None, RefusalDetail::None),
        "STARTUP refused. That tank is no longer in the match."
    );
}

/// **16b cut W.1**, the same rule one test up, for the two words `FIRE` gained:
/// the cockpit refuses before the arena does, so each early line has to contain
/// the arena's own word for the same refusal or the two can drift apart.
#[test]
fn the_fire_refusals_carry_the_arenas_own_words() {
    for (line, refusal) in [
        (NO_LOCK, Refusal::NoLock),
        (CYCLING, Refusal::Cycling),
        (EMPTY, Refusal::Empty),
    ] {
        let word = refusal.as_str().replace('_', " ");
        assert!(
            line.to_ascii_lowercase().contains(&word),
            "`{line}` does not contain `{word}`"
        );
    }
}

/// The line `WEAPONSTATUS` prints, at both ends of a cycle.
#[test]
fn a_weapon_row_renders_its_readiness_its_rounds_and_its_band() {
    let ready = WeaponRow {
        id: SmolStr::new("cannon"),
        ready: true,
        ready_in_ms: 0,
        rounds: 40,
        rounds_max: 40,
        range_band: Some(crate::arena::weapons::FrameBand {
            index: 1,
            of: 2,
            max_range_cm: 150_000,
            dispersion_mrad: 3,
        }),
        // 16b cut F.2: this weapon's own chance, not the platform's best.
        p_permille: Some(464),
        heat_per_shot: 0,
        armed: None,
    };
    // 16b cut D.2: `cannon` is one word, so it has no short name to open.
    assert_eq!(
        ready.render(std::slice::from_ref(&ready)),
        "cannon  ready  40 of 40 round(s)  band 1 of 2  46% to hit"
    );
    // Cycling, with the seconds rounded **up**: a pilot told "0 s" would try
    // again and be refused. No lock, so no band and no chance.
    let cycling = WeaponRow {
        ready: false,
        ready_in_ms: 3_001,
        rounds: 39,
        range_band: None,
        p_permille: None,
        ..ready.clone()
    };
    assert_eq!(
        cycling.render(std::slice::from_ref(&cycling)),
        "cannon  cycling (4 s)  39 of 40 round(s)"
    );
    // **16b cut D.2** (`B-17`): a two-word id opens its initials, and only
    // where no other row answers to them.
    let mg = WeaponRow {
        id: SmolStr::new("machine_gun"),
        ..ready.clone()
    };
    let rows = vec![ready.clone(), mg.clone()];
    assert_eq!(
        mg.render(&rows),
        "machine_gun (mg)  ready  40 of 40 round(s)  band 1 of 2  46% to hit"
    );
    let missile = WeaponRow {
        id: SmolStr::new("missile_gun"),
        ..ready.clone()
    };
    let shared = vec![mg.clone(), missile.clone()];
    assert_eq!(
        mg.render(&shared),
        "machine_gun  ready  40 of 40 round(s)  band 1 of 2  46% to hit",
        "a short name two rows answer to is not a name"
    );
}

#[test]
fn a_cockpit_seat_carries_exactly_the_three_channels() {
    let channels = cockpit_channels();
    assert_eq!(channels.len(), 3);
    for channel in Channel::ALL {
        assert!(
            ChannelId::new(channel.as_str()).is_some_and(|id| channels.contains(&id)),
            "{}",
            channel.as_str()
        );
    }
    // And the station is NOT named after one of them, which is what makes
    // `open_match` raise all three for it.
    assert_eq!(Channel::parse(STATION), None);
}

#[test]
fn speed_converts_both_ways_at_the_stock_tanks_numbers() {
    // 15 m/s is the stock tank's maximum: 54 kph.
    assert_eq!(cms_to_kph(1_500), 54);
    assert_eq!(kph_to_cms(54), 1_500);
    assert_eq!(kph_to_cms(0), 0);
    assert_eq!(kph_to_cms(-20), -555);
    assert_eq!(cms_to_kph(-555), -19);
    // Saturating, not panicking, at the edges.
    assert_eq!(kph_to_cms(i32::MAX), i32::MAX / 3_600);
}

/// **16b cut Q.1**: a contact id still *renders* as its sixteen digits — the
/// ledger, the deliberation brief and the frame's row key all carry that
/// string — and it is no longer something a pilot can **type**. The parse
/// half went with `parse_contact`; nothing replaced it in a second lane.
#[test]
fn a_contact_id_still_renders_as_sixteen_digits_and_is_no_longer_typable() {
    let id = ContactId(0x0123_4567_89ab_cdef);
    assert_eq!(id.to_string(), "0123456789abcdef");
    let rows = vec![row(
        id,
        Designator::Slot {
            team: crate::arena::state::PartyId::BLUE,
            slot: 1,
        },
        Some(2),
    )];
    assert_eq!(resolve_designator(&rows, &id.to_string()), None);
    assert_eq!(resolve_designator(&rows, "1"), None);
}

/// One row, with only the three fields [`resolve_designator`] reads.
fn row(id: ContactId, designator: Designator, alias: Option<u16>) -> ContactRow {
    ContactRow {
        id,
        designator,
        alias,
        class: SmolStr::new_static("blue"),
        bearing_deg: 87,
        relative_deg: 32,
        band: SmolStr::new_static("effective"),
        range_m: 450,
        age_ms: 2_000,
        cell: Some((4, 4)),
        friendly: false,
        aspect: SmolStr::new_static("front"),
        odds: crate::arena::view::OddsCell::Unarmed,
    }
}

/// **16b cut Q.1**: the two spellings a pilot may type, case-folded and
/// trimmed, and nothing else.
#[test]
fn a_designator_resolves_by_its_current_name_or_by_the_number_behind_it() {
    let id = ContactId(7);
    let slot = Designator::Slot {
        team: crate::arena::state::PartyId::BLUE,
        slot: 1,
    };
    let rows = vec![row(id, slot, Some(2))];
    for word in ["BA", "ba", " Ba ", "X2", "x2"] {
        assert_eq!(resolve_designator(&rows, word), Some(id), "{word}");
    }
    for word in ["", "ZZ", "X1", "B", "BAA", "2"] {
        assert_eq!(resolve_designator(&rows, word), None, "{word}");
    }
    // A row that is still provisional has no alias, and its slot spelling
    // resolves against nothing: `LOCK BA` on an unknown `BA` is refused
    // exactly as `LOCK ZZ` is.
    let unknown = vec![row(id, Designator::Provisional(2), None)];
    assert_eq!(resolve_designator(&unknown, "X2"), Some(id));
    assert_eq!(resolve_designator(&unknown, "BA"), None);
}

/// **16b cut Q.1**: `FIRE`'s line is untouched, and the read verbs have their
/// own — `FIRE` takes a weapon and never a contact, so "or name one" would
/// be a lie to its pilot.
#[test]
fn the_read_verbs_refusal_is_its_own_and_fires_is_unchanged() {
    assert_eq!(NO_LOCK, "No lock: LOCK a contact first.");
    assert_eq!(
        NO_LOCK_OR_NAME,
        "No lock: LOCK a contact first, or name one."
    );
    assert_ne!(NO_LOCK, NO_LOCK_OR_NAME);
    assert_eq!(CockpitError::NoLock.to_string(), NO_LOCK);
    assert_eq!(CockpitError::NoLockOrName.to_string(), NO_LOCK_OR_NAME);
}

/// **16b cut Q.1**: a plot older than one sensor pass says so, and a live one
/// says nothing — a track that announced `plotted 0 s ago` on every line
/// would be noise where the point is a warning.
#[test]
fn a_read_verb_says_how_stale_a_plot_is_only_once_it_is() {
    assert_eq!(stale_clause(0, 1_000), "");
    assert_eq!(stale_clause(1_000, 1_000), "", "one pass is not stale yet");
    assert_eq!(stale_clause(1_001, 1_000), ", plotted 1 s ago");
    assert_eq!(stale_clause(4_200, 1_000), ", plotted 4 s ago");
    // A cadence of zero is read as one rather than dividing by it, so two
    // milliseconds are already past it.
    assert_eq!(stale_clause(1, 0), "");
    assert_eq!(stale_clause(2, 0), ", plotted 0 s ago");
    // And it is a clause, not a sentence: the full stop is the caller's.
    assert!(!stale_clause(4_200, 1_000).ends_with('.'));
}

#[test]
fn every_registered_cockpit_verb_has_a_help_line_and_a_dispatch_arm() {
    for verb in COCKPIT_VERBS {
        assert!(!cockpit_help(verb).is_empty(), "{verb}");
        assert!(
            cockpit_help(verb)
                .to_ascii_lowercase()
                .contains(&verb.to_ascii_lowercase()),
            "{verb}'s help names it"
        );
    }
    // Was 16 at 17b cut M1a.4; 17b cut M5.4 appended MIND, the
    // seventeenth (doc 17 §12).
    assert_eq!(
        COCKPIT_VERBS.len(),
        17,
        "doc 16 section 3.7's nine, cut Q.1's two read verbs, cut Q.2's SCAN, cut Y.1's \
         ORDER, cut O.2's EMCON, cut AP.3's WATCH, 17b cut M1a.4's DRIVE and 17b cut \
         M5.4's MIND"
    );
}

// -----------------------------------------------------------------------
// 16b cut D.2
// -----------------------------------------------------------------------

/// One weapon row with an id and nothing else that matters here.
fn weapon_row(id: &str) -> WeaponRow {
    WeaponRow {
        id: SmolStr::new(id),
        ready: true,
        ready_in_ms: 0,
        rounds: 10,
        rounds_max: 10,
        range_band: None,
        p_permille: None,
        heat_per_shot: 0,
        armed: None,
    }
}

/// **`B-17`**: the short name is the id's initials, and a one-word id has
/// none — a single letter is a keystroke, not a name.
#[test]
fn d2_a_weapon_of_two_words_is_typable_as_its_initials() {
    assert_eq!(weapon_initials("machine_gun").as_deref(), Some("mg"));
    assert_eq!(weapon_initials("missile_rack_two").as_deref(), Some("mrt"));
    assert_eq!(weapon_initials("MACHINE_GUN").as_deref(), Some("mg"));
    assert_eq!(weapon_initials("cannon"), None);
    assert_eq!(weapon_initials("laser"), None);
    assert_eq!(weapon_initials(""), None);
    assert_eq!(weapon_initials("_"), None);
    // It is NOT `lobby::short_name`, which answers the last segment of a
    // canonical unit id. The two rules are opposite and are about different
    // kinds of id.
    assert_ne!(
        weapon_initials("tacsim_unit_line_tank").as_deref(),
        Some("line_tank")
    );
}

/// **`B-17`**: `FIRE mg` works, `FIRE can` works, and a word two weapons
/// answer to is refused saying which.
#[test]
fn d2_choose_weapon_takes_the_id_the_short_name_and_the_prefix() {
    let rows = vec![weapon_row("cannon"), weapon_row("machine_gun")];
    let chosen = |word: &str| {
        choose_weapon(&rows, word)
            .map(|row| row.id.to_string())
            .map_err(|err| err.to_string())
    };
    // (1) the exact id, case-folded and trimmed.
    assert_eq!(chosen("machine_gun"), Ok("machine_gun".to_owned()));
    assert_eq!(chosen("  MACHINE_GUN "), Ok("machine_gun".to_owned()));
    // (2) the short name.
    assert_eq!(chosen("mg"), Ok("machine_gun".to_owned()));
    assert_eq!(chosen("MG"), Ok("machine_gun".to_owned()));
    // (3) the prefix.
    assert_eq!(chosen("can"), Ok("cannon".to_owned()));
    assert_eq!(chosen("mach"), Ok("machine_gun".to_owned()));
    // None: the existing sentence, unchanged.
    assert_eq!(
        chosen("railgun"),
        Err("That unit carries no weapon called `railgun`.".to_owned())
    );
    // An empty word is the default weapon and never a search.
    assert_eq!(chosen(""), Ok(DEFAULT_WEAPON.to_owned()));
    assert_eq!(chosen("   "), Ok(DEFAULT_WEAPON.to_owned()));
    // 16b cut AE.3: a platform with no rows at all (a drone) — a bare
    // `FIRE` names nothing, so neither does the refusal; a named weapon
    // keeps the sentence that names it.
    assert_eq!(choose_weapon(&[], ""), Err(CockpitError::NoWeapon));
    assert_eq!(
        choose_weapon(&[], "  ").map_err(|err| err.to_string()),
        Err("This unit carries no weapon.".to_owned())
    );
    assert_eq!(
        choose_weapon(&[], "cannon").map_err(|err| err.to_string()),
        Err("That unit carries no weapon called `cannon`.".to_owned())
    );
    // Ambiguous at the SHORT-NAME step, and at the PREFIX step, each naming
    // the candidates in row order.
    let two = vec![weapon_row("machine_gun"), weapon_row("missile_gun")];
    assert_eq!(
        choose_weapon(&two, "mg").map_err(|err| err.to_string()),
        Err("`mg` could be machine_gun or missile_gun; say more.".to_owned())
    );
    assert_eq!(
        choose_weapon(&two, "m").map_err(|err| err.to_string()),
        Err("`m` could be machine_gun or missile_gun; say more.".to_owned())
    );
    // A step is not fallen through on ambiguity: `m` matching two short names
    // must not quietly become a prefix search that matches the same two.
    assert!(matches!(
        choose_weapon(&two, "mis"),
        Ok(row) if row.id == "missile_gun"
    ));
    // And the exact id always works, however crowded the rack is.
    assert!(matches!(
        choose_weapon(&two, "machine_gun"),
        Ok(row) if row.id == "machine_gun"
    ));
}

/// **16b cut AE.2**: the weapon rows a fielded unit type's frame carries, in
/// component order, loaded and ready — what `FIRE` chooses among.
fn fielded_rows(entry: &str) -> Vec<WeaponRow> {
    let catalog = crate::definitions::Catalog::build_fielded(&[], crate::NS)
        .unwrap_or_else(|error| panic!("the fielded content binds: {error}"));
    let key = mush_core::CanonicalId::new(entry).unwrap_or_else(|_| panic!("{entry}"));
    let (Some(spec), Some(binding)) = (catalog.specs.get(&key), catalog.bindings.get(&key)) else {
        panic!("{entry} is fielded")
    };
    let cells = crate::definitions::to_tactical_unit(spec, binding, ObjId(31))
        .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"));
    crate::match_::weapon_summaries(&catalog, &cells)
        .into_iter()
        .map(|weapon| WeaponRow {
            rounds: i64::from(weapon.count_per_load),
            rounds_max: i64::from(weapon.count_per_load),
            ..weapon_row(&weapon.id)
        })
        .collect()
}

/// **16b cut AE.2**: `FIRE`'s default is the content author's choice by
/// component order, under HEAD's unchanged rule — the cannon if carried,
/// else the first weapon. The scout car answers its autocannon, the heavy
/// its cannon (its rack is `FIRE missiles`, or the prefix `FIRE mis`), the
/// stock tank its cannon and the laser tank its beam; a one-word id has no
/// short name, so `FIRE auto` is a prefix; and `WEAPONSTATUS` prints the
/// new rows in the existing format.
#[test]
fn ae2_the_bare_fire_is_the_authors_choice_by_component_order() {
    use crate::definitions::content;
    let chosen = |entry: &str, word: &str| {
        choose_weapon(&fielded_rows(entry), word)
            .map(|row| row.id.to_string())
            .map_err(|err| err.to_string())
    };
    assert_eq!(chosen(content::SCOUT_MK1, ""), Ok("autocannon".to_owned()));
    assert_eq!(chosen(content::HEAVY_MK1, ""), Ok("cannon".to_owned()));
    assert_eq!(chosen(content::LINE_TANK, ""), Ok("cannon".to_owned()));
    assert_eq!(chosen(content::LINE_TANK_LASER, ""), Ok("laser".to_owned()));
    assert_eq!(
        chosen(content::SCOUT_MK1, "auto"),
        Ok("autocannon".to_owned())
    );
    assert_eq!(
        chosen(content::SCOUT_MK1, "mg"),
        Ok("machine_gun".to_owned())
    );
    assert_eq!(
        chosen(content::HEAVY_MK1, "missiles"),
        Ok("missiles".to_owned())
    );
    assert_eq!(chosen(content::HEAVY_MK1, "mis"), Ok("missiles".to_owned()));
    assert_eq!(
        chosen(content::HEAVY_MK1, "m"),
        Err("`m` could be machine_gun or missiles; say more.".to_owned())
    );
    assert_eq!(weapon_initials("autocannon"), None);
    assert_eq!(weapon_initials("missiles"), None);

    // The rows, in the existing format.
    let scout = fielded_rows(content::SCOUT_MK1);
    assert_eq!(
        scout
            .iter()
            .map(|row| row.render(&scout))
            .collect::<Vec<_>>(),
        vec![
            "autocannon  ready  240 of 240 round(s)",
            "machine_gun (mg)  ready  2000 of 2000 round(s)",
        ]
    );
    let mut heavy = fielded_rows(content::HEAVY_MK1);
    assert_eq!(
        heavy
            .iter()
            .map(|row| row.render(&heavy))
            .collect::<Vec<_>>(),
        vec![
            "cannon  ready  40 of 40 round(s)",
            "machine_gun (mg)  ready  2000 of 2000 round(s)",
            "missiles  ready  6 of 6 round(s)",
        ]
    );
    if let Some(rack) = heavy.iter_mut().find(|row| row.id == "missiles") {
        rack.ready = false;
        rack.ready_in_ms = 10_000;
        rack.rounds = 5;
    }
    assert_eq!(
        heavy.get(2).map(|row| row.render(&heavy)).as_deref(),
        Some("missiles  cycling (10 s)  5 of 6 round(s)")
    );
}

/// **`B-17`**: the row advertises a short name only where it is one.
#[test]
fn d2_a_weapon_row_opens_its_short_name_only_when_it_is_unshared() {
    let rows = vec![weapon_row("cannon"), weapon_row("machine_gun")];
    assert_eq!(weapon_short_of(&rows, "machine_gun").as_deref(), Some("mg"));
    assert_eq!(weapon_short_of(&rows, "cannon"), None);
    let shared = vec![weapon_row("machine_gun"), weapon_row("missile_gun")];
    assert_eq!(weapon_short_of(&shared, "machine_gun"), None);
    assert_eq!(weapon_short_of(&shared, "missile_gun"), None);
}

/// **`B-16`**: one sentence for every `FIRE` refusal, and the weapon is in
/// it. The `LOCK` and `STARTUP` shapes are untouched.
#[test]
fn d2_every_fire_refusal_names_the_weapon_and_the_others_do_not() {
    use crate::arena::intents::Refusal;
    use crate::arena::perception::RefusalDetail;
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Overheated,
            Some("machine_gun"),
            RefusalDetail::Overheated { heat: 60, max: 60 },
        ),
        "FIRE machine_gun refused. Overheated: 60 of 60. Let it cool."
    );
    assert_eq!(
        refusal_line("lock", Refusal::PowerNotOnline, None, RefusalDetail::None),
        format!("LOCK refused. {POWER_NOT_ONLINE}")
    );
    assert_eq!(
        refusal_line("startup", Refusal::Wreck, None, RefusalDetail::None),
        "STARTUP refused. Your tank is a wreck."
    );
    // The shape itself, and its empty-weapon case.
    assert_eq!(
        order_refused("heading", None, "Your tank cannot move."),
        "HEADING refused. Your tank cannot move."
    );
    assert_eq!(
        order_refused("fire", Some("  "), "No lock: LOCK a contact first."),
        "FIRE refused. No lock: LOCK a contact first."
    );
    // The cockpit's own pre-check builds the SAME string through the SAME
    // function, so the early answer and the late one cannot drift.
    assert_eq!(
        fire_refused("cannon", &CockpitError::NoLock).to_string(),
        refusal_line("fire", Refusal::NoLock, Some("cannon"), RefusalDetail::None)
    );
    assert_eq!(
        fire_refused("cannon", &CockpitError::Empty).to_string(),
        refusal_line("fire", Refusal::Empty, Some("cannon"), RefusalDetail::None)
    );
    assert_eq!(
        fire_refused("cannon", &CockpitError::Cycling { secs: 3 }).to_string(),
        refusal_line(
            "fire",
            Refusal::Cycling,
            Some("cannon"),
            RefusalDetail::Cycling { secs: 3 }
        )
    );
    assert_eq!(
        fire_refused("laser", &CockpitError::Overheated { heat: 60, max: 60 }).to_string(),
        refusal_line(
            "fire",
            Refusal::Overheated,
            Some("laser"),
            RefusalDetail::Overheated { heat: 60, max: 60 }
        )
    );
    assert_eq!(
        fire_refused("cannon", &CockpitError::PowerNotOnline).to_string(),
        refusal_line(
            "fire",
            Refusal::PowerNotOnline,
            Some("cannon"),
            RefusalDetail::None
        )
    );
    // `PowerStarting` has no `Refusal` of its own — the arena never answers
    // it — but it takes the same prefix, so a pilot reads one shape.
    assert_eq!(
        fire_refused("cannon", &CockpitError::PowerStarting).to_string(),
        format!("FIRE cannon refused. {POWER_STARTING}")
    );
}

/// **`B-03`**: the two local sentences, and the motion one pinned to the
/// arena's own word for the same condition.
#[test]
fn d2_the_wreck_and_the_motion_limit_have_their_own_sentences() {
    use crate::arena::intents::Refusal;
    use crate::arena::perception::RefusalDetail;
    assert_eq!(
        CockpitError::Destroyed.to_string(),
        "Your tank is destroyed and takes no orders. EXIT leaves the pod."
    );
    assert_eq!(CockpitError::Immobile.to_string(), IMMOBILE);
    assert_eq!(
        refusal_line(
            "heading",
            Refusal::MotionDisabled,
            None,
            RefusalDetail::None
        ),
        format!("HEADING refused. {IMMOBILE}"),
        "the local sentence and the arena's are one string"
    );
    assert_eq!(
        OWN_DESTROYED,
        "Your tank is destroyed. The match runs on without you: wait it out, EXIT to the \
         team room, or SIM ABORT to call it off."
    );
    assert_eq!(
        OWN_DISABLED,
        "Your tank can neither move nor shoot. The match runs on: wait it out, EXIT to the \
         team room, or SIM ABORT to call it off."
    );
    // Neither names anything but the pilot's own tank.
    for line in [OWN_DESTROYED, OWN_DISABLED, DESTROYED, IMMOBILE] {
        assert!(!line.contains("bearing"), "{line}");
        assert!(!line.contains('#'), "{line}");
    }
}

/// **`T-77`**: the module head, the verb table and [`COCKPIT_VERBS`] agree,
/// and the `STATUS` help names the callsign line.
#[test]
fn d2_the_verb_table_and_the_help_agree_with_the_selector_list() {
    // Was 16 at 17b cut M1a.4; 17b cut M5.4's MIND is the seventeenth.
    assert_eq!(COCKPIT_VERBS.len(), 17);
    let head = include_str!("../cockpit.rs");
    for verb in COCKPIT_VERBS {
        let upper = verb.to_uppercase();
        assert!(
            head.contains(&format!("//! {upper}")),
            "the module head's table omits {upper}"
        );
    }
    assert!(head.contains("//! BOARD"));
    assert!(head.contains("//! EXIT"));
    let status = cockpit_help("status");
    assert!(status.contains("callsign"), "{status}");
    assert!(status.contains("scope"), "{status}");
    assert!(
        !status.contains("tempo"),
        "the help still promises the arena's global tempo: {status}"
    );
}

// -- 16b cut V.1: the named ground ------------------------------------------

/// A spot at a cell, as the arena carries one.
fn spot(name: &str, x: u16, y: u16) -> KeySpot {
    KeySpot {
        name: SmolStr::new(name),
        x,
        y,
    }
}

/// The shipped proving ground's five, in authored order (the pack's own
/// `key_spots`; the fifth, The Bluff, is 16b cut AE.1's).
fn shipped() -> Vec<KeySpot> {
    vec![
        spot("The Ford", 64, 97),
        spot("The Crossroads", 129, 97),
        spot("North Barn", 106, 194),
        spot("East Compound", 208, 148),
        spot("The Bluff", 144, 111),
    ]
}

/// A pose on a cell's centre at ten-metre cells, heading `heading_mdeg`.
fn pose_on(x: i64, y: i64, heading_mdeg: u32) -> mush_sim::data::Pose {
    let mut pose = mush_sim::data::Pose::at(
        x * 1_000 + 500,
        y * 1_000 + 500,
        0,
        mush_sim::data::passive_rules_value(serde_json::json!({})).expect("an empty posture blob"),
    );
    pose.heading_mdeg = heading_mdeg;
    pose
}

/// **The alphabet** (16b cut V.1): as long as the cap, every letter its own,
/// and no letter a byte the map already draws or the pilot already reads.
///
/// This is about the SPOT letters only. Two existing markers already share a
/// byte with the ground — `x` (a hostile) is `Rubble`'s glyph and `+` (a
/// friendly) is `Door`'s — and that is not this cut's to fix.
#[test]
fn v1_the_spot_alphabet_is_the_cap_and_collides_with_nothing() {
    assert_eq!(
        SPOT_LETTERS.len(),
        crate::definitions::arena::MAX_KEY_SPOTS,
        "the cap is the alphabet's length"
    );
    for (i, a) in SPOT_LETTERS.iter().enumerate() {
        for b in SPOT_LETTERS.iter().skip(i + 1) {
            assert_ne!(a, b, "{a} is in the alphabet twice");
        }
    }
    let mut taken: Vec<char> = mush_atlas::TerrainClass::ALL
        .iter()
        .map(|class| class.glyph())
        .collect();
    for glyph in [YOU_GLYPH, UNKNOWN_GLYPH, FRIEND_GLYPH, HOSTILE_GLYPH] {
        taken.extend(glyph.chars());
    }
    taken.push(crate::arena::state::PROVISIONAL_PREFIX);
    for letter in SPOT_LETTERS {
        assert!(
            !taken.contains(&letter),
            "the spot letter {letter} is a byte the map or the scope already uses"
        );
    }
    assert_eq!(spot_letter(0), Some('A'));
    assert_eq!(
        spot_letter(3),
        Some('E'),
        "D is a platform, so the fourth is E"
    );
    assert_eq!(spot_letter(21), Some('Z'));
    assert_eq!(spot_letter(22), None, "past the alphabet is nothing");
}

/// **`resolve_spot`** (16b cut V.1): the whole name or the one letter, case
/// folded and trimmed, and nothing else — no prefix, and never a word with a
/// designator's shape.
#[test]
fn v1_a_spot_resolves_by_its_whole_name_or_its_letter_and_nothing_else() {
    let spots = shipped();
    let at = |word: &str| resolve_spot(&spots, word).map(|(index, _)| index);
    assert_eq!(at("The Ford"), Some(0));
    assert_eq!(at("the ford"), Some(0));
    assert_eq!(at("  THE FORD "), Some(0));
    assert_eq!(at("north barn"), Some(2));
    assert_eq!(at("a"), Some(0));
    assert_eq!(at("B"), Some(1));
    assert_eq!(at("c"), Some(2));
    // The fourth spot is `E`: `D` is not a spot letter at all.
    assert_eq!(at("e"), Some(3));
    assert_eq!(at("d"), None);
    // No prefix, no fragment, no near miss.
    for word in ["the", "The Fo", "Ford", "The Fjord", "north", "", "  "] {
        assert_eq!(at(word), None, "{word:?} names no spot");
    }
    // A designator is never a spot, in either case.
    for word in ["RA", "BA", "X1", "x1", "ra"] {
        assert_eq!(at(word), None, "{word} is a designator's shape");
    }
    // The spot answered is the one at that index.
    assert_eq!(
        resolve_spot(&spots, "e").map(|(_, spot)| spot.name.as_str()),
        Some("East Compound")
    );
}

/// **Range and bearing to a spot's cell centre** (16b cut V.1), on the
/// canonical fixture's pose — the pilot on cell (6, 4) at ten-metre cells,
/// so at (6,500, 4,500) cm — against hand-worked numbers.
///
/// * `A` at (25, 3): its centre is (25,500, 3,500), so dx = 19,000 and
///   dy = −1,000. The range is `isqrt(19,000² + 1,000²)` =
///   `isqrt(362,000,000)` = 19,026 cm, which rounds to 190 m at the 10 m
///   step. The bearing is `atan2(19,000, −1,000)` = 90° + `atan(1/19)` =
///   90° + 3.0128° = 93.013°, so 93,013 mdeg.
/// * `C` at (50, 25): dx = 44,000 and dy = 21,000, so `isqrt(1,936,000,000 +
///   441,000,000)` = `isqrt(2,377,000,000)` = 48,754 cm (490 m), bearing
///   `atan2(44, 21)` = 64.486° = 64,486 mdeg.
#[test]
fn v1_a_spot_s_range_and_bearing_are_read_to_its_cell_centre() {
    let pose = pose_on(6, 4, 45_000);
    let ford = spot("The Ford", 25, 3);
    let woods = spot("The Woods", 50, 25);
    assert_eq!(spot_range_cm(&pose, &ford, 1_000), 19_026);
    assert_eq!(spot_bearing_mdeg(&pose, &ford, 1_000), 93_013);
    assert_eq!(range_m_rounded(19_026, SPOT_RANGE_STEP_M), 190);
    assert_eq!(spot_range_cm(&pose, &woods, 1_000), 48_754);
    assert_eq!(spot_bearing_mdeg(&pose, &woods, 1_000), 64_486);
    assert_eq!(range_m_rounded(48_754, SPOT_RANGE_STEP_M), 490);
    // The two sentences, verbatim.
    assert_eq!(
        spot_range_line(&ford, 'A', 190),
        "Range to The Ford (A): 190 m."
    );
    assert_eq!(
        spot_bearing_line(&ford, 'A', 93, relative_mdeg(45_000, 93_013) / 1_000),
        "Bearing to The Ford (A): 093° (rel 048°)."
    );
    // Standing on the spot's own cell is range 0 and bearing 0, and the
    // range still rounds to one step (`range_m_rounded` never says nought).
    let on = pose_on(25, 3, 0);
    assert_eq!(spot_range_cm(&on, &ford, 1_000), 0);
    assert_eq!(spot_bearing_mdeg(&on, &ford, 1_000), 0);
}

/// **`nearest_spot`** (16b cut V.1): the least range, ties to the lowest
/// authored index, and nothing on ground that names none.
#[test]
fn v1_the_nearest_spot_ties_to_the_lowest_index() {
    let pose = pose_on(6, 4, 0);
    // West and east of the pilot, one cell each: the same range.
    let spots = vec![
        spot("West Gate", 5, 4),
        spot("East Gate", 7, 4),
        spot("Far Field", 60, 60),
    ];
    let (index, found, range) = nearest_spot(&spots, &pose, 1_000).expect("a spot");
    assert_eq!((index, found.name.as_str(), range), (0, "West Gate", 1_000));
    // Authored the other way round, the first is still the one kept.
    let flipped = vec![spots[1].clone(), spots[0].clone(), spots[2].clone()];
    assert_eq!(
        nearest_spot(&flipped, &pose, 1_000).map(|(index, spot, _)| (index, spot.name.clone())),
        Some((0, SmolStr::new("East Gate")))
    );
    // A strictly nearer later spot wins.
    let nearer = vec![spot("Far Field", 60, 60), spot("East Gate", 7, 4)];
    assert_eq!(
        nearest_spot(&nearer, &pose, 1_000).map(|(index, _, _)| index),
        Some(1)
    );
    assert!(nearest_spot(&[], &pose, 1_000).is_none());
    // The line: `West Gate` is due west, 10 m.
    assert_eq!(
        nearest_spot_line(&spots, &pose, 1_000).as_deref(),
        Some("  nearest spot: West Gate (A), 10 m at 270°")
    );
    assert_eq!(nearest_spot_line(&[], &pose, 1_000), None);
    // The shipped muster: red slot 1 on (6, 97), 58 cells short of the ford
    // on the same row — 580 m due east.
    assert_eq!(
        nearest_spot_line(&shipped(), &pose_on(6, 97, 90_000), 1_000).as_deref(),
        Some("  nearest spot: The Ford (A), 580 m at 090°")
    );
}

/// **`VIEW`'s off-window line** (16b cut V.1), verbatim: nothing when every
/// spot is drawn (or there are none), one spot, and two in authored order.
#[test]
fn v1_the_off_window_line_names_every_undrawn_spot_in_authored_order() {
    let pose = pose_on(6, 4, 45_000);
    let spots = vec![
        spot("The Ford", 25, 3),
        spot("The Roof", 14, 7),
        spot("The Woods", 50, 25),
    ];
    assert_eq!(off_window_spots_line(&[], &[], &pose, 1_000), None);
    assert_eq!(
        off_window_spots_line(&spots, &[0, 1, 2], &pose, 1_000),
        None,
        "every spot drawn"
    );
    assert_eq!(
        off_window_spots_line(&spots, &[0, 1], &pose, 1_000).as_deref(),
        Some("Off the window: C The Woods, 490 m at 064°.")
    );
    assert_eq!(
        off_window_spots_line(&spots, &[1], &pose, 1_000).as_deref(),
        Some("Off the window: A The Ford, 190 m at 093°; C The Woods, 490 m at 064°.")
    );
    // The shipped ground from the red muster, whose 39 by 20 window (x 0..38)
    // draws none of the five. 16b cut AE.1's fifth, The Bluff at (144, 111),
    // is 138 cells east and 14 north of (6, 97): 138.7 cells, 1390 m to the
    // ten, at 084°.
    assert_eq!(
        off_window_spots_line(&shipped(), &[], &pose_on(6, 97, 90_000), 1_000).as_deref(),
        Some(
            "Off the window: A The Ford, 580 m at 090°; B The Crossroads, 1230 m at 090°; \
             C North Barn, 1390 m at 045°; E East Compound, 2080 m at 075°; \
             F The Bluff, 1390 m at 084°."
        )
    );
}

/// **`spot_markers`** (16b cut V.1): clipped at every edge exactly as
/// `contact_markers` clips — the west and south edges are the corner itself,
/// the east and north edges are `corner + size`, exclusive — with cells
/// relative to the corner, one legend entry per marker, and the authored
/// indexes drawn.
#[test]
fn v1_spot_markers_clip_at_every_edge_and_carry_their_legend() {
    let spots = vec![
        spot("West Out", 9, 25),   // one column west of the window
        spot("West In", 10, 25),   // on its west edge
        spot("South Out", 20, 19), // one row south
        spot("South In", 20, 20),  // on its south edge
        spot("East In", 48, 30),   // on its east edge (10 + 39 - 1)
        spot("East Out", 49, 30),  // one column past it
        spot("North In", 30, 39),  // on its north edge (20 + 20 - 1)
        spot("North Out", 30, 40), // one row past it
    ];
    let (markers, legend, drawn) = spot_markers(&spots, (10, 20), (39, 20));
    assert_eq!(drawn, vec![1, 3, 4, 6]);
    let cells: Vec<serde_json::Value> = markers
        .iter()
        .filter_map(|marker| marker.get("cell").cloned())
        .collect();
    assert_eq!(
        cells,
        vec![
            serde_json::json!([0, 5]),
            serde_json::json!([10, 0]),
            serde_json::json!([38, 10]),
            serde_json::json!([20, 19]),
        ]
    );
    // The letter is the AUTHORED index's, not the drawn order's.
    assert_eq!(
        markers.first(),
        Some(&serde_json::json!({
            "id": "B",
            "name": "West In",
            "cell": [0, 5],
            "glyph": "B",
            "you": false,
            "spot": true,
        }))
    );
    assert_eq!(
        legend,
        vec![
            serde_json::json!(["B", "West In"]),
            serde_json::json!(["E", "South In"]),
            serde_json::json!(["F", "East In"]),
            serde_json::json!(["H", "North In"]),
        ]
    );
    // Nothing to draw, nothing drawn.
    let (none, no_legend, none_drawn) = spot_markers(&[], (0, 0), (39, 20));
    assert!(none.is_empty() && no_legend.is_empty() && none_drawn.is_empty());
}

/// **`drawn_spots`** (16b cut V.1) reads the drawing's own spots back off
/// `units` — every marker flagged `spot`, by its letter's authored index —
/// **including one a later marker covers**: a spot under a contact is drawn
/// (then overwritten), so `VIEW` never lists it off the window. The pilot's
/// own marker and the contacts are never counted, whatever their `id`.
#[test]
fn v1_the_drawn_spots_are_every_spot_marker_even_a_covered_one() {
    let window = serde_json::json!({
        "units": [
            { "id": "A", "name": "The Ford", "cell": [25, 3], "glyph": "A", "you": false, "spot": true },
            { "id": "C", "name": "The Roof", "cell": [14, 7], "glyph": "C", "you": false, "spot": true },
            { "id": "@", "cell": [19, 10], "glyph": "@", "you": true },
            // A contact on C's cell: it owns the cell, and C is still drawn.
            { "id": "X3", "cell": [14, 7], "glyph": "?", "you": false },
            // A contact whose designator starts with a spot letter is not one.
            { "id": "BA", "cell": [30, 12], "glyph": "x", "you": false },
        ]
    });
    assert_eq!(drawn_spots(&window), vec![0, 2]);
    assert!(drawn_spots(&serde_json::json!({})).is_empty());
    // Read against it, the off-window line names only the spot not drawn.
    let spots = vec![
        spot("The Ford", 25, 3),
        spot("North Gate", 7, 4),
        spot("The Roof", 14, 7),
    ];
    assert_eq!(
        off_window_spots_line(&spots, &drawn_spots(&window), &pose_on(6, 4, 0), 1_000).as_deref(),
        Some("Off the window: B North Gate, 10 m at 090°.")
    );
}

// -- 16b cut AG.1: `VIEW BRIEF`, the window in sentences --------------------
//
// Pure pins over hand-painted ground: none of these builds a `Frame` (it
// derives `Clone` and `Debug` only and this module has no builder), so every
// test that needs one lives in `tests/cockpit.rs` over `fixture::frame()`.

/// One cell of `class`, passable, at the datum.
fn ag1_cell(class: mush_atlas::TerrainClass) -> crate::arena::state::TerrainCell {
    crate::arena::state::TerrainCell {
        class,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// A one-row yard, `width` cells at `cell_cm`, road on even columns and grass
/// on odd — so no two neighbouring cells merge into one run.
fn ag1_striped(width: u16, cell_cm: u16) -> ArenaTerrain {
    let cells = (0..width)
        .map(|x| {
            ag1_cell(if x % 2 == 0 {
                mush_atlas::TerrainClass::Road
            } else {
                mush_atlas::TerrainClass::Grass
            })
        })
        .collect();
    ArenaTerrain::new(width, 1, cell_cm, cells).expect("a one-row yard")
}

/// The fixture yard's shape, re-stated (a test binary's `support::yard`
/// cannot be imported here): 96 by 64 of grass at 1,000 cm, a road along
/// y = 10, water x 20..=29 by y 2..=5, a wall block x 12..=15 by y 6..=8,
/// light woods x 40..=59 by y 20..=29.
fn ag1_yard() -> ArenaTerrain {
    use mush_atlas::TerrainClass;
    let (width, height) = (96_u16, 64_u16);
    let mut cells = vec![ag1_cell(TerrainClass::Grass); usize::from(width) * usize::from(height)];
    let mut paint = |xs: std::ops::Range<u16>, ys: std::ops::Range<u16>, class| {
        for y in ys {
            for x in xs.clone() {
                cells[usize::from(y) * usize::from(width) + usize::from(x)] = ag1_cell(class);
            }
        }
    };
    paint(0..96, 10..11, TerrainClass::Road);
    paint(20..30, 2..6, TerrainClass::Water);
    paint(12..16, 6..9, TerrainClass::Wall);
    paint(40..60, 20..30, TerrainClass::WoodsLight);
    ArenaTerrain::new(width, height, 1_000, cells).expect("the fixture yard's shape")
}

/// The eight sentences a pilot at `from` says of `terrain`'s window.
fn ag1_sentences(
    terrain: &ArenaTerrain,
    from: (u16, u16),
    origin: (u16, u16),
    size: (u16, u16),
) -> Vec<(String, RayEnd)> {
    BRIEF_BEARINGS
        .iter()
        .map(|(bearing, dx, dy)| {
            let (runs, end) = ground_ray(terrain, from, (*dx, *dy), origin, size);
            let line = ground_line(bearing, &runs, end, terrain.cell_cm(), *dx != 0 && *dy != 0);
            (line, end)
        })
        .collect()
}

/// **The class words** (16b cut AG.1): every class has a prose word, and
/// none is a label's snake case. The exhaustive match is the compile-time
/// guard for a new class; this is the run-time one for an empty word.
#[test]
fn ag1_class_prose_covers_every_class_with_a_prose_word() {
    for class in mush_atlas::TerrainClass::ALL {
        let word = class_prose(class);
        assert!(!word.trim().is_empty(), "{class:?} has no word");
        assert!(!word.contains('_'), "{class:?} reads as a label: {word}");
    }
    assert_eq!(
        class_prose(mush_atlas::TerrainClass::WoodsHeavy),
        "heavy woods"
    );
    assert_eq!(
        class_prose(mush_atlas::TerrainClass::WaterDeep),
        "deep water"
    );
    assert_eq!(class_prose(mush_atlas::TerrainClass::Ground), "bare ground");
    assert_eq!(class_prose(mush_atlas::TerrainClass::Door), "doorway");
}

/// **Cells to centimetres** (16b cut AG.1): `steps × cell_cm` straight, and
/// root two to three places on a diagonal, rounded at the brief's step.
#[test]
fn ag1_ray_cm_is_cells_at_the_cell_size_and_root_two_on_a_diagonal() {
    let step = brief_step_m(1_000);
    let straight: Vec<(u64, u64)> = [4, 5, 6, 15, 19]
        .into_iter()
        .map(|k| {
            let cm = ray_cm(k, 1_000, false);
            (cm, range_m_rounded(cm, step))
        })
        .collect();
    assert_eq!(
        straight,
        vec![
            (4_000, 40),
            (5_000, 50),
            (6_000, 60),
            (15_000, 150),
            (19_000, 190)
        ]
    );
    let diagonal: Vec<(u64, u64)> = [4, 5, 6, 15, 19]
        .into_iter()
        .map(|k| {
            let cm = ray_cm(k, 1_000, true);
            (cm, range_m_rounded(cm, step))
        })
        .collect();
    assert_eq!(
        diagonal,
        vec![
            (5_656, 60),
            (7_070, 70),
            (8_484, 80),
            (21_210, 210),
            (26_866, 270)
        ]
    );
    assert_eq!(ray_cm(0, 1_000, true), 0);
    assert_eq!(
        ray_cm(u32::MAX, u16::MAX, true),
        u64::from(u32::MAX) * u64::from(u16::MAX) * 1_414 / 1_000
    );
}

/// **The brief's step** (16b cut AG.1): the spot step at the shipped cell
/// and every coarser one, and no coarser than a cell below it — so at 500 cm
/// nineteen one-cell runs read nineteen different reaches.
#[test]
fn ag1_the_brief_step_is_the_spot_step_and_no_coarser_than_a_cell() {
    let steps: Vec<u64> = [50, 100, 200, 500, 1_000, 2_500]
        .into_iter()
        .map(brief_step_m)
        .collect();
    assert_eq!(steps, vec![1, 1, 2, 5, 10, 10]);
    let yard = ag1_striped(60, 500);
    let (runs, end) = ground_ray(&yard, (19, 0), (1, 0), (0, 0), (39, 1));
    assert_eq!(end, RayEnd::Window, "(39, 0) is on the arena");
    let reaches: Vec<u64> = runs
        .iter()
        .map(|run| range_m_rounded(ray_cm(run.steps, 500, false), brief_step_m(500)))
        .collect();
    assert_eq!(reaches, (1..=19).map(|k| k * 5).collect::<Vec<u64>>());
    assert!(
        reaches.windows(2).all(|pair| pair[0] < pair[1]),
        "every reach strictly past the last: {reaches:?}"
    );
}

/// **One class is one run, to the window on every side** (16b cut AG.1): a
/// 60-by-40 grass yard and the window `view_window` computes for a pilot at
/// (29, 20) — origin (10, 10), 39 by 20 — which ends inside the arena every
/// way. Nine cells north, ten south (`you.dy` is 10), nineteen east and
/// west; nine diagonal steps are 12,726 cm (`130 m`), ten 14,140 (`140 m`).
#[test]
fn ag1_a_ray_over_one_class_is_one_run_to_the_window() {
    let yard = ArenaTerrain::uniform(60, 40, 1_000, ag1_cell(mush_atlas::TerrainClass::Grass))
        .expect("a uniform yard");
    let said = ag1_sentences(&yard, (29, 20), (10, 10), (39, 20));
    let lines: Vec<&str> = said.iter().map(|(line, _)| line.as_str()).collect();
    assert_eq!(
        lines,
        vec![
            "North: 90 m of grass, then beyond the window.",
            "North-east: 130 m of grass, then beyond the window.",
            "East: 190 m of grass, then beyond the window.",
            "South-east: 140 m of grass, then beyond the window.",
            "South: 100 m of grass, then beyond the window.",
            "South-west: 140 m of grass, then beyond the window.",
            "West: 190 m of grass, then beyond the window.",
            "North-west: 130 m of grass, then beyond the window.",
        ]
    );
    assert!(said.iter().all(|(_, end)| *end == RayEnd::Window));
    let (runs, _) = ground_ray(&yard, (29, 20), (1, 1), (10, 10), (39, 20));
    assert_eq!(
        runs,
        vec![GroundRun {
            class: mush_atlas::TerrainClass::Grass,
            steps: 9
        }]
    );
}

/// **Alternating classes merge nothing** (16b cut AG.1): nineteen one-cell
/// runs east, reaches 10 to 190 m, eighteen joins and the tail — nineteen
/// `then`s.
#[test]
fn ag1_a_ray_over_alternating_classes_merges_nothing() {
    let yard = ag1_striped(60, 1_000);
    let (runs, end) = ground_ray(&yard, (19, 0), (1, 0), (0, 0), (39, 1));
    assert_eq!(runs.len(), 19, "cells 20..=38, one run each");
    assert_eq!(end, RayEnd::Window, "(39, 0) is on the arena");
    assert_eq!(
        runs.iter().map(|run| run.steps).collect::<Vec<u32>>(),
        (1..=19).collect::<Vec<u32>>()
    );
    let line = ground_line("East", &runs, end, 1_000, false);
    assert!(
        line.starts_with("East: 10 m of road, then grass to 20 m, then road to 30 m"),
        "{line}"
    );
    assert!(
        line.ends_with(", then road to 190 m, then beyond the window."),
        "{line}"
    );
    assert_eq!(line.matches("then").count(), 19, "{line}");
}

/// **The arena's edge is said as the edge** (16b cut AG.1): the fixture
/// yard, the pilot at (6, 4) and the clamped window (0, 0), 39 by 20 — the
/// eight sentences of the fixture's golden, verbatim.
#[test]
fn ag1_a_ray_at_the_arena_edge_says_the_edge() {
    let said = ag1_sentences(&ag1_yard(), (6, 4), (0, 0), (39, 20));
    let lines: Vec<&str> = said.iter().map(|(line, _)| line.as_str()).collect();
    assert_eq!(
        lines,
        vec![
            "North: 50 m of grass, then road to 60 m, then grass to 150 m, then beyond the window.",
            "North-east: 70 m of grass, then road to 80 m, then grass to 210 m, then beyond the \
             window.",
            "East: 130 m of grass, then water to 230 m, then grass to 320 m, then beyond the \
             window.",
            "South-east: 60 m of grass, then the edge of the arena.",
            "South: 40 m of grass, then the edge of the arena.",
            "South-west: 60 m of grass, then the edge of the arena.",
            "West: 60 m of grass, then the edge of the arena.",
            "North-west: 70 m of grass, then road to 80 m, then the edge of the arena.",
        ]
    );
    let ends: Vec<RayEnd> = said.iter().map(|(_, end)| *end).collect();
    assert_eq!(
        ends,
        vec![
            RayEnd::Window,
            RayEnd::Window,
            RayEnd::Window,
            RayEnd::Arena,
            RayEnd::Arena,
            RayEnd::Arena,
            RayEnd::Arena,
            RayEnd::Arena,
        ]
    );
}

/// **A pilot on the window's last cell says only the tail** (16b cut AG.1):
/// at (0, 0) of a window at (0, 0) — the one case production makes — west
/// and south are `the edge of the arena`. Then, a SYNTHETIC rectangle
/// `view_window` never produces (a pilot on a window edge that is not the
/// arena's): the north-east corner of a window inside a larger arena says
/// `beyond the window` with no run.
#[test]
fn ag1_a_pilot_on_the_window_s_last_cell_says_only_the_tail() {
    let yard = ag1_yard();
    let said = ag1_sentences(&yard, (0, 0), (0, 0), (39, 20));
    let at = |bearing: &str| {
        said.iter()
            .map(|(line, _)| line.as_str())
            .find(|line| line.starts_with(&format!("{bearing}:")))
            .map(str::to_owned)
    };
    assert_eq!(at("West").as_deref(), Some("West: the edge of the arena."));
    assert_eq!(
        at("South").as_deref(),
        Some("South: the edge of the arena.")
    );
    assert_eq!(
        at("South-west").as_deref(),
        Some("South-west: the edge of the arena.")
    );
    // Synthetic: view_window's clamp never leaves a pilot here.
    let wide = ArenaTerrain::uniform(60, 40, 1_000, ag1_cell(mush_atlas::TerrainClass::Grass))
        .expect("a uniform yard");
    let corner = ag1_sentences(&wide, (48, 29), (10, 10), (39, 20));
    let lines: Vec<&str> = corner.iter().map(|(line, _)| line.as_str()).collect();
    assert_eq!(lines.first().copied(), Some("North: beyond the window."));
    assert_eq!(
        lines.get(1).copied(),
        Some("North-east: beyond the window.")
    );
    assert_eq!(lines.get(2).copied(), Some("East: beyond the window."));
}

/// **A contact sentence is the row's own fields** (16b cut AG.1): the four
/// fixture contacts, with a friendly mark and an alias on two of them to
/// show that neither prints.
#[test]
fn ag1_the_contact_sentence_is_the_row_s_own_fields() {
    use crate::arena::state::PartyId;
    let row = |id: u64,
               designator: Designator,
               alias: Option<u16>,
               class: &str,
               (bearing_deg, relative_deg): (u32, u32),
               band: &str,
               range_m: u64,
               age_ms: i64,
               friendly: bool| ContactRow {
        id: ContactId(id),
        designator,
        alias,
        class: SmolStr::new(class),
        bearing_deg,
        relative_deg,
        band: SmolStr::new(band),
        range_m,
        age_ms,
        cell: None,
        friendly,
        aspect: SmolStr::new_static("front"),
        odds: crate::arena::view::OddsCell::Unarmed,
    };
    let rows = [
        row(
            1,
            Designator::Slot {
                team: PartyId::BLUE,
                slot: 1,
            },
            Some(1),
            "blue",
            (45, 0),
            "far",
            450,
            1_200,
            false,
        ),
        row(
            2,
            Designator::Slot {
                team: PartyId::RED,
                slot: 2,
            },
            None,
            "red",
            (270, 225),
            "close",
            100,
            400,
            true,
        ),
        row(
            3,
            Designator::Provisional(3),
            None,
            "unknown",
            (315, 270),
            "effective",
            200,
            3_500,
            false,
        ),
        row(
            4,
            Designator::Provisional(4),
            None,
            "unknown",
            (30, 345),
            "far",
            600,
            6_000,
            false,
        ),
    ];
    let said: Vec<String> = rows.iter().map(contact_sentence).collect();
    assert_eq!(
        said,
        vec![
            "BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.",
            "RB, red: bearing 270° (rel 225°), close, 100 m, 0 s old.",
            "X3, unknown: bearing 315° (rel 270°), effective, 200 m, 3 s old.",
            "X4, unknown: bearing 030° (rel 345°), far, 600 m, 6 s old.",
        ]
    );
}

/// **The in-window line names the named spots, in authored order** (16b cut
/// AG.1), with `spot_clause`'s figures: A The Ford from (6, 4) is 190 m at
/// 093°; B The Roof at (14, 7) is dx 8,000, dy 3,000, `isqrt` 8,544 cm (90 m
/// to the ten) at `atan2(8, 3)` = 69.4°, so 069°.
#[test]
fn ag1_in_window_spots_line_names_the_named_spots_in_authored_order() {
    let pose = pose_on(6, 4, 45_000);
    let spots = vec![
        spot("The Ford", 25, 3),
        spot("The Roof", 14, 7),
        spot("The Woods", 50, 25),
    ];
    assert_eq!(in_window_spots_line(&spots, &[], &pose, 1_000), None);
    assert_eq!(
        in_window_spots_line(&spots, &[0], &pose, 1_000).as_deref(),
        Some("In the window: A The Ford, 190 m at 093°.")
    );
    assert_eq!(
        in_window_spots_line(&spots, &[1], &pose, 1_000).as_deref(),
        Some("In the window: B The Roof, 90 m at 069°."),
        "a named spot that is not authored index 0"
    );
    let both = Some("In the window: A The Ford, 190 m at 093°; B The Roof, 90 m at 069°.");
    assert_eq!(
        in_window_spots_line(&spots, &[0, 1], &pose, 1_000).as_deref(),
        both
    );
    assert_eq!(
        in_window_spots_line(&spots, &[1, 0], &pose, 1_000).as_deref(),
        both,
        "authored order, not the order of `named`"
    );
    assert_eq!(in_window_spots_line(&[], &[0], &pose, 1_000), None);
}

/// **The header is the footer's facts** (16b cut AG.1), read off the
/// payload: the fixture's window, and the one `view_window` produces for a
/// pilot at (76, 97) on the shipped 256-by-256 ground (`y0 = 97 − 10`).
#[test]
fn ag1_the_header_reads_the_footer_s_facts() {
    let fixture = serde_json::json!({
        "origin_cell": [0, 0],
        "width": 39,
        "height": 20,
        "extent": [96, 64],
        "you": [6, 4],
        "cell_cm": 1000,
    });
    assert_eq!(
        brief_header("RA", 45_000, &fixture),
        "Window around RA: cells 0 to 38 east and 0 to 19 north of 96 by 64; you at (6, 4), \
         heading 045°; 10 m a cell."
    );
    let shipped = serde_json::json!({
        "origin_cell": [57, 87],
        "width": 39,
        "height": 20,
        "extent": [256, 256],
        "you": [19, 10],
        "cell_cm": 1000,
    });
    assert_eq!(
        brief_header("RA", 270_000, &shipped),
        "Window around RA: cells 57 to 95 east and 87 to 106 north of 256 by 256; you at \
         (76, 97), heading 270°; 10 m a cell."
    );
    // A payload with no keys at all reads as nought, and does not panic.
    assert_eq!(
        brief_header("RA", 0, &serde_json::json!({})),
        "Window around RA: cells 0 to 0 east and 0 to 0 north of 0 by 0; you at (0, 0), \
         heading 000°; 0 m a cell."
    );
}

/// **`named_spots` is the legend's spots, in authored indexes** (16b cut
/// AG.1): a drawn-and-covered spot is drawn and not named; and with A off
/// the window, `B` is `units[0]` and authored index 1 — the case that tells
/// an authored index from a position in `units`.
#[test]
fn ag1_named_spots_are_the_legend_s_in_authored_indexes() {
    let window = serde_json::json!({
        "units": [
            { "id": "A", "name": "The Ford", "cell": [25, 3], "glyph": "A", "you": false, "spot": true },
            { "id": "B", "name": "The Roof", "cell": [14, 7], "glyph": "B", "you": false, "spot": true },
            { "id": "#101", "cell": [6, 4], "glyph": "@", "you": true },
            { "id": "X3", "name": "unknown", "cell": [14, 7], "glyph": "?", "you": false },
        ],
        "legend": [[".", "grass"], ["@", "you"], ["?", "unknown"], ["A", "The Ford"]],
    });
    assert_eq!(drawn_spots(&window), vec![0, 1]);
    assert_eq!(named_spots(&window), vec![0]);
    let crossroads = serde_json::json!({
        "units": [
            { "id": "B", "name": "The Roof", "cell": [3, 4], "glyph": "B", "you": false, "spot": true },
            { "id": "C", "name": "The Woods", "cell": [9, 9], "glyph": "C", "you": false, "spot": true },
            { "id": "#101", "cell": [19, 10], "glyph": "@", "you": true },
            { "id": "X1", "name": "unknown", "cell": [3, 4], "glyph": "?", "you": false },
        ],
        "legend": [["@", "you"], ["?", "unknown"], ["C", "The Woods"]],
    });
    assert_eq!(drawn_spots(&crossroads), vec![1, 2]);
    assert_eq!(named_spots(&crossroads), vec![2]);
    assert!(named_spots(&serde_json::json!({})).is_empty());
}

// -----------------------------------------------------------------------
// 16b cut Y.1 — `ORDER`'s grammar, the pod pair and the acknowledgement
// -----------------------------------------------------------------------

fn y1_red(index: u16) -> crate::deliberation::SquadId {
    crate::deliberation::SquadId {
        team: crate::arena::state::PartyId::RED,
        index,
    }
}

fn y1_words(squad: SquadWord, objective: ObjectiveWords, hold_fire: bool) -> OrderWords {
    OrderWords {
        squad,
        objective,
        hold_fire,
    }
}

/// **16b cut Y.1**: every line of the grammar, both squad forms and `ALL`,
/// case-folded; a spot of two words with and without the `HOLD FIRE` tail;
/// and every usage error answering [`ORDER_USAGE`] verbatim.
#[test]
fn y1_parse_order_reads_every_line_of_the_grammar() {
    assert_eq!(
        parse_order("red-1 HOLD"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Hold,
            false
        ))
    );
    assert_eq!(
        parse_order("  Blue-2   hold  hold FIRE "),
        Ok(y1_words(
            SquadWord::Id(crate::deliberation::SquadId {
                team: crate::arena::state::PartyId::BLUE,
                index: 2,
            }),
            ObjectiveWords::Hold,
            true
        ))
    );
    assert_eq!(
        parse_order("1 WITHDRAW"),
        Ok(y1_words(
            SquadWord::Ordinal(1),
            ObjectiveWords::Withdraw,
            false
        ))
    );
    assert_eq!(
        parse_order("1 withdraw HOLD FIRE"),
        Ok(y1_words(
            SquadWord::Ordinal(1),
            ObjectiveWords::Withdraw,
            true
        ))
    );
    assert_eq!(
        parse_order("ALL engage"),
        Ok(y1_words(
            SquadWord::All,
            ObjectiveWords::Engage { contact: None },
            false
        ))
    );
    assert_eq!(
        parse_order("all ENGAGE ba"),
        Ok(y1_words(
            SquadWord::All,
            ObjectiveWords::Engage {
                contact: Some("ba".to_owned())
            },
            false
        ))
    );
    assert_eq!(
        parse_order("red-1 ENGAGE BA HOLD FIRE"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Engage {
                contact: Some("BA".to_owned())
            },
            true
        ))
    );
    assert_eq!(
        parse_order("red-1 ENGAGE HOLD FIRE"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Engage { contact: None },
            true
        ))
    );
    assert_eq!(
        parse_order("red-1 ADVANCE The Ford"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Advance {
                spot: "The Ford".to_owned()
            },
            false
        ))
    );
    assert_eq!(
        parse_order("1 advance The   Ford hold fire"),
        Ok(y1_words(
            SquadWord::Ordinal(1),
            ObjectiveWords::Advance {
                spot: "The Ford".to_owned()
            },
            true
        ))
    );
    assert_eq!(
        parse_order("red-1 ADVANCE a"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Advance {
                spot: "a".to_owned()
            },
            false
        ))
    );
    // Every usage error, one sentence.
    for bad in [
        "",
        "   ",
        "red-1",
        "red-1 DANCE",
        "HOLD",
        "green-1 HOLD",
        "squad HOLD",
        "red-1 ADVANCE",
        "red-1 ADVANCE HOLD FIRE",
        "red-1 ENGAGE BA XB",
        "red-1 HOLD now",
        "red-1 HOLD FIRE",
        "red-1 WITHDRAW HOLD",
        "red-1 WITHDRAW FIRE",
    ] {
        assert_eq!(
            parse_order(bad),
            Err(CockpitError::Usage(ORDER_USAGE.to_owned())),
            "{bad:?}"
        );
    }
    assert_eq!(
        CockpitError::Usage(ORDER_USAGE.to_owned()).to_string(),
        "Usage: ORDER <squad|ALL> HOLD | ADVANCE <spot> | ENGAGE [<contact>] | WITHDRAW \
         | SCREEN <bearing> | SUPPORT <callsign> [HOLD FIRE], or ORDER <squad|ALL> RELEASE"
    );
    assert_eq!(CockpitError::NoSuchSquad.to_string(), "No such squad.");
    assert_eq!(
        CockpitError::NoSquadToOrder.to_string(),
        "You have no squad to order."
    );
}

/// **16b cut Y.2** (doc 16 §2 W8): the span's two sentences and the rate's,
/// verbatim — each says only what is true: waiting is what frees a span, and
/// the grammar takes one squad or `ALL`.
#[test]
fn y2_the_span_and_rate_refusals_read_verbatim() {
    assert_eq!(
        CockpitError::SpanFull { held: 1 }.to_string(),
        "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one."
    );
    assert_eq!(
        CockpitError::SpanAll { would: 5, span: 4 }.to_string(),
        "ORDER ALL would put 5 squads under you; your span is 4. Order them one at a time."
    );
    assert_eq!(
        CockpitError::RateFull { n: 12 }.to_string(),
        "You have given 12 orders in the last minute; wait."
    );
}

/// **16b cut Y.2** (doc 16 §2 W8): the span's sentence is chosen by what
/// would pass, not by the squad word. `ALL`'s *one at a time* only when one
/// at a time does pass (the others leave room); *wait for one to run out*
/// whenever the others alone fill the span — including an `ALL` that names
/// one squad, as a wingman's does.
#[test]
fn y2_the_span_sentence_says_what_would_pass() {
    // Inside the span, a replacement included (named, not other): nothing.
    assert_eq!(span_refusal(0, 1, 1), None);
    assert_eq!(span_refusal(3, 1, 4), None);
    assert_eq!(span_refusal(0, 4, 4), None);
    // One squad named past the span — or `ALL` naming one squad, which is
    // the same count: the others fill it, so waiting, never "one at a
    // time", which would be refused as well.
    assert_eq!(
        span_refusal(1, 1, 1),
        Some(CockpitError::SpanFull { held: 1 })
    );
    assert_eq!(
        span_refusal(4, 1, 4),
        Some(CockpitError::SpanFull { held: 4 })
    );
    // `ALL` naming several past the span with room for one: one at a time.
    assert_eq!(
        span_refusal(2, 3, 4),
        Some(CockpitError::SpanAll { would: 5, span: 4 })
    );
    assert_eq!(
        span_refusal(0, 5, 4),
        Some(CockpitError::SpanAll { would: 5, span: 4 })
    );
    // `ALL` naming several while the others already fill it: waiting.
    assert_eq!(
        span_refusal(4, 3, 4),
        Some(CockpitError::SpanFull { held: 4 })
    );
}

/// **16b cut Z.2** (16c N6, Q44): `SCREEN <bearing>`, `SUPPORT <callsign>`
/// and `RELEASE` read with and without the `HOLD FIRE` tail where the arm
/// takes one; `SCREEN` with no word, a word that is not all digits, or 360
/// and above, `SUPPORT` with no word or two, and `RELEASE` with any tail
/// (`HOLD FIRE` included) are usage, verbatim; and the usage line fits the
/// TUI notices ring's 200-character line.
#[test]
fn z2_parse_order_reads_the_screen_the_support_and_the_release() {
    assert_eq!(
        parse_order("red-1 SCREEN 90"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Screen { bearing: 90 },
            false
        ))
    );
    assert_eq!(
        parse_order("1 screen 359 hold fire"),
        Ok(y1_words(
            SquadWord::Ordinal(1),
            ObjectiveWords::Screen { bearing: 359 },
            true
        ))
    );
    assert_eq!(
        parse_order("ALL screen 000"),
        Ok(y1_words(
            SquadWord::All,
            ObjectiveWords::Screen { bearing: 0 },
            false
        ))
    );
    assert_eq!(
        parse_order("1 SUPPORT ra"),
        Ok(y1_words(
            SquadWord::Ordinal(1),
            ObjectiveWords::Support {
                callsign: "ra".to_owned()
            },
            false
        ))
    );
    assert_eq!(
        parse_order("red-1 support RA HOLD FIRE"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Support {
                callsign: "RA".to_owned()
            },
            true
        ))
    );
    assert_eq!(
        parse_order("red-1 RELEASE"),
        Ok(y1_words(
            SquadWord::Id(y1_red(1)),
            ObjectiveWords::Release,
            false
        ))
    );
    assert_eq!(
        parse_order("all release"),
        Ok(y1_words(SquadWord::All, ObjectiveWords::Release, false))
    );
    for bad in [
        "red-1 SCREEN",
        "red-1 SCREEN 360",
        "red-1 SCREEN 1000",
        "red-1 SCREEN east",
        "red-1 SCREEN -90",
        "red-1 SCREEN +90",
        "red-1 SCREEN 90 45",
        "red-1 SCREEN HOLD FIRE",
        "red-1 SUPPORT",
        "red-1 SUPPORT RA RB",
        "red-1 SUPPORT HOLD FIRE",
        "red-1 RELEASE HOLD FIRE",
        "red-1 RELEASE now",
        "red-1 RELEASE RA",
    ] {
        assert_eq!(
            parse_order(bad),
            Err(CockpitError::Usage(ORDER_USAGE.to_owned())),
            "{bad:?}"
        );
    }
    assert!(
        ORDER_USAGE.chars().count() <= 200,
        "the usage line wraps inside the notices ring: {}",
        ORDER_USAGE.chars().count()
    );
    assert_eq!(
        CockpitError::NoSuchFriendly.to_string(),
        "No tank of your side flies that callsign."
    );
    assert_eq!(ObjectiveWords::Screen { bearing: 1 }.as_str(), "screen");
    assert_eq!(
        ObjectiveWords::Support {
            callsign: String::new()
        }
        .as_str(),
        "support"
    );
    assert_eq!(ObjectiveWords::Release.as_str(), "release");
}

/// **16b cut Z.2**: the four new order acknowledgements and the release's,
/// verbatim — the bearing printed as `BEARING` prints one, the leash in
/// metres, the charge by its callsign as the match prints it, the order
/// lines with the ten-minute tail and the release with none, singular for
/// one squad or a wingman and plural for several.
#[test]
fn z2_the_acknowledgements_say_what_the_roles_and_the_release_mean() {
    const TAIL: &str = " It stands for 10 minutes or until your next order to it.";
    let one = vec!["red-1".to_owned()];
    let screen = ObjectiveWords::Screen { bearing: 90 };
    let support = ObjectiveWords::Support {
        callsign: "ra".to_owned(),
    };
    let cases = [
        (
            order_ack(&one, &screen, None, None, false, 150),
            "Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on \
             what comes inside its arc without giving chase.",
        ),
        (
            order_ack(&one, &screen, None, None, true, 150),
            "Ordered red-1 to screen 090°, holding fire: it holds where it stands, faces \
             090° and locks what comes inside its arc but fires nothing.",
        ),
        // The two support arms were `… and fires first on what RA is
        // fighting.` / `… and locks what RA is fighting but fires nothing.`
        // until 17b cut M5.6: a supporter fights what it finds at its
        // station, and nothing it reads says what its charge is fighting
        // (OQ-6; STOP D, the text made true).
        (
            order_ack(&one, &support, Some("RA"), None, false, 150),
            "Ordered red-1 to support RA: it keeps within 150 m of RA and fights what it \
             finds there.",
        ),
        (
            order_ack(&one, &support, Some("RA"), None, true, 150),
            "Ordered red-1 to support RA, holding fire: it keeps within 150 m of RA and \
             locks what it finds but fires nothing.",
        ),
    ];
    for (said, want) in cases {
        assert_eq!(said, format!("{want}{TAIL}"));
    }
    // The three release forms were `Released … to doctrine: it fights on
    // its own reflexes until …` / `… they fight on their own reflexes …`
    // until 17b cut M5.3: "doctrine" is false of a mind sealed off it and
    // names the mind (the design note's M5.3 item 3).
    assert_eq!(
        order_ack(&one, &ObjectiveWords::Release, None, None, false, 150),
        "Released red-1 to its mind: it fights on its own until it is ordered again."
    );
    assert_eq!(
        order_ack(
            &["RB".to_owned()],
            &ObjectiveWords::Release,
            None,
            None,
            false,
            0
        ),
        "Released RB to its mind: it fights on its own until it is ordered again."
    );
    assert_eq!(
        order_ack(
            &["red-1".to_owned(), "red-2".to_owned()],
            &ObjectiveWords::Release,
            None,
            None,
            false,
            0
        ),
        "Released red-1 and red-2 to their minds: they fight on their own until they are \
         ordered again."
    );
}

/// **16b cut Y.1**: pods are claimed in pairs.
#[test]
fn y1_the_pod_pair_is_odd_with_the_next_and_even_with_the_last() {
    assert_eq!(pair(1), 2);
    assert_eq!(pair(2), 1);
    assert_eq!(pair(3), 4);
    assert_eq!(pair(4), 3);
    assert_eq!(pair(7), 8);
    assert_eq!(pair(0), 0, "no slot is 0; it pairs with itself");
    assert_eq!(pair(u8::MAX), u8::MAX, "saturating at the top");
    assert_eq!(HUMAN_ORDER_TTL_MS, 600_000);
}

/// **16b cut Y.1**: the acknowledgement, verbatim, for every arm — with and
/// without a hint, with `HOLD FIRE` — for one, two and three squads, and for
/// a wingman's callsign.
#[test]
fn y1_the_acknowledgement_says_what_the_order_means() {
    const TAIL: &str = " It stands for 10 minutes or until your next order to it.";
    let one = vec!["red-1".to_owned()];
    let ack =
        |objective: &ObjectiveWords,
         hint: Option<&str>,
         spot: Option<(&str, char)>,
         hold_fire: bool| order_ack(&one, objective, hint, spot, hold_fire, 150);
    let hold = ObjectiveWords::Hold;
    let advance = ObjectiveWords::Advance {
        spot: "the ford".to_owned(),
    };
    let engage = ObjectiveWords::Engage {
        contact: Some("ba".to_owned()),
    };
    let engage_any = ObjectiveWords::Engage { contact: None };
    let withdraw = ObjectiveWords::Withdraw;
    let cases = [
        (
            ack(&hold, None, None, false),
            "Ordered red-1 to hold: it stops where it stands and fires nothing.",
        ),
        (
            ack(&hold, None, None, true),
            "Ordered red-1 to hold, holding fire: it stops where it stands and fires \
             nothing.",
        ),
        (
            ack(&advance, None, Some(("The Ford", 'A')), false),
            "Ordered red-1 to advance on The Ford (A): it drives there and fights nothing \
             on the way.",
        ),
        (
            ack(&advance, None, Some(("The Ford", 'A')), true),
            "Ordered red-1 to advance on The Ford (A), holding fire: it drives there and \
             fights nothing on the way.",
        ),
        (
            ack(&engage, Some("BA"), None, false),
            "Ordered red-1 to engage BA: it closes and fires on BA where it can see it, \
             else on what it finds.",
        ),
        (
            ack(&engage_any, None, None, false),
            "Ordered red-1 to engage: it closes and fires on what it finds.",
        ),
        (
            ack(&engage, Some("BA"), None, true),
            "Ordered red-1 to engage BA, holding fire: it closes and locks but fires \
             nothing.",
        ),
        (
            ack(&engage_any, None, None, true),
            "Ordered red-1 to engage, holding fire: it closes and locks on what it finds \
             but fires nothing.",
        ),
        (
            ack(&withdraw, None, None, false),
            "Ordered red-1 to withdraw: it breaks for its entry zone and still fights \
             inside its bands.",
        ),
        (
            ack(&withdraw, None, None, true),
            "Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and \
             fires nothing.",
        ),
    ];
    for (said, want) in cases {
        assert_eq!(said, format!("{want}{TAIL}"));
    }
    // Two and three squads for a lead, a callsign for a wingman.
    let two = vec!["red-1".to_owned(), "red-2".to_owned()];
    assert_eq!(
        order_ack(&two, &hold, None, None, false, 150),
        format!(
            "Ordered red-1 and red-2 to hold: it stops where it stands and fires nothing.{TAIL}"
        )
    );
    let three = vec!["red-1".to_owned(), "red-2".to_owned(), "red-3".to_owned()];
    assert_eq!(
        order_ack(&three, &withdraw, None, None, false, 150),
        format!(
            "Ordered red-1, red-2 and red-3 to withdraw: it breaks for its entry zone and \
             still fights inside its bands.{TAIL}"
        )
    );
    let wingman = vec!["RB".to_owned()];
    assert_eq!(
        order_ack(&wingman, &engage, Some("X2"), None, false, 150),
        format!(
            "Ordered RB to engage X2: it closes and fires on X2 where it can see it, else \
             on what it finds.{TAIL}"
        )
    );
}

/// **16b cut Y.1**: the thirteenth selector is registered, walked by the
/// gate, and carries its help line.
///
/// The help line said `RELEASE hands them back to their own doctrine.`
/// until 17b cut M5.6: `… to their own minds.` (the M5.3 close's D9 ruling,
/// one word for every mind).
#[test]
fn y1_order_is_the_thirteenth_selector() {
    // 16b cut O.2 put `emcon` after it, so it is the thirteenth and no
    // longer the last; 16b cut AP.3 put `watch` after that, 17b cut
    // M1a.4 `drive` after `watch` and 17b cut M5.4 `mind` after `drive`
    // (the length was 16 until that cut).
    assert_eq!(COCKPIT_VERBS.len(), 17);
    assert_eq!(COCKPIT_VERBS.get(12), Some(&"order"));
    assert_eq!(
        cockpit_help("order"),
        "ORDER <squad|ALL> <objective>  command your side's machines: HOLD, ADVANCE <spot>, \
         ENGAGE [<contact>], WITHDRAW, SCREEN <bearing> or SUPPORT <callsign>, with HOLD \
         FIRE at the end to forbid shooting; RELEASE hands them back to their own minds. \
         A side's lead orders any squad of the side; anyone else their pod pair's tank."
    );
}

/// **16b cut O.2** (16c N12): the fourteenth selector's words, pinned — the
/// help that says what each position costs, the `STATUS` line, the local
/// refusal and the arena's own, which are one sentence after the prefix.
#[test]
fn o2_emcon_is_the_fourteenth_selector_and_its_words_are_pinned() {
    use crate::arena::perception::RefusalDetail;
    // 16b cut AP.3 appended `watch` after it, so it is the fourteenth and
    // no longer the last; 17b cut M1a.4's `drive` follows `watch`.
    assert_eq!(COCKPIT_VERBS.get(13), Some(&"emcon"));
    assert_eq!(
        cockpit_help("emcon"),
        "EMCON [on|off]      switch your jamming pod, if you carry one: on jams every sight \
         inside its bubble, yours among them; off clears your own sight and hides you from \
         nothing. On at check-out."
    );
    assert!(
        cockpit_help("status").contains(", your pod's switch if you carry one"),
        "{}",
        cockpit_help("status")
    );
    assert_eq!(emcon_status_line(true), "  emcon on: the pod radiates");
    assert_eq!(emcon_status_line(false), "  emcon off: the pod is silent");
    assert_eq!(
        CockpitError::NoJammer.to_string(),
        "This tank carries no jamming pod."
    );
    assert_eq!(
        refusal_line("emcon", Refusal::NoJammer, None, RefusalDetail::None),
        "EMCON refused. This tank carries no jamming pod."
    );
    assert_eq!(Refusal::NoJammer.as_str(), "no_jammer");
}

/// **17b cut M1a.4** (doc 17 §4 D-UM-10): the sixteenth selector, its help
/// line, its four replies, its usage line, its two refusals, the goal each
/// word forwards ([`drive_goal`]: a spot's name and its letter the spot's
/// own cell, of two spots, beside the reply naming it), the motor's
/// two notices — each rendered through [`notice_line`] — and the arena's
/// own refusal of a `DRIVE`, which names the verb through
/// [`IntentKind::as_str`].
#[test]
fn m1a4_drive_is_the_sixteenth_selector_and_its_words_are_pinned() {
    use crate::arena::perception::RefusalDetail;
    // Was 16 until 17b cut M5.4 appended `mind` after it; `drive` keeps
    // its place, the sixteenth.
    assert_eq!(COCKPIT_VERBS.len(), 17);
    assert_eq!(COCKPIT_VERBS.get(15), Some(&"drive"));
    assert_eq!(
        cockpit_help("drive"),
        "DRIVE <spot|x,y>    drive to a named spot of the ground, by its name or its map \
         letter, or to an arena cell, on the machine's own route-finding; HEADING or SPEED \
         takes the helm back. DRIVE HOLD stops and holds here; DRIVE OFF lets go."
    );
    assert_eq!(
        DRIVE_USAGE,
        "DRIVE <spot|x,y|HOLD|OFF>; STATUS names the nearest spot."
    );
    assert_eq!(DRIVE_HOLDING, "Holding here.");
    assert_eq!(DRIVE_RELEASED, "DRIVE off; the helm is yours.");
    assert_eq!(
        DRIVE_NOT_A_GOAL,
        "DRIVE refused. That is not a spot, a cell, HOLD or OFF."
    );
    assert_eq!(
        DRIVE_OFF_WINDOW,
        "DRIVE refused. That cell is not on the window."
    );
    assert_eq!(DRIVE_ARRIVED, "DRIVE: arrived.");
    assert_eq!(DRIVE_NO_ROUTE, "DRIVE: no route from here; holding.");
    // The cell parser: two unsigned integers, by comma or by space.
    assert_eq!(drive_cell("12,34"), Some((12, 34)));
    assert_eq!(drive_cell("12, 34"), Some((12, 34)));
    assert_eq!(drive_cell("12 34"), Some((12, 34)));
    assert_eq!(drive_cell("70000,1"), Some((70_000, 1)));
    for word in [
        "12", "12,", ",34", "12,34,5", "12 34 5", "ba", "x2", "-1,3", "a",
    ] {
        assert_eq!(drive_cell(word), None, "{word}");
    }
    // The word's reading: a spot's name and its letter forward that
    // spot's own cell — two spots, so a swapped or a wrong index shows —
    // and the reply names the same spot.
    let spots = [
        KeySpot {
            name: SmolStr::new_static("The Ford"),
            x: 12,
            y: 40,
        },
        KeySpot {
            name: SmolStr::new_static("Hill 7"),
            x: 50,
            y: 3,
        },
    ];
    let read = |word: &str| drive_goal(word, &spots, 64, 48);
    for (word, x, y, line) in [
        ("the ford", 12, 40, "Steering for The Ford (A)."),
        ("a", 12, 40, "Steering for The Ford (A)."),
        ("hill 7", 50, 3, "Steering for Hill 7 (B)."),
        ("b", 50, 3, "Steering for Hill 7 (B)."),
        ("12,34", 12, 34, "Steering for cell 12,34."),
        ("63 47", 63, 47, "Steering for cell 63,47."),
    ] {
        assert_eq!(
            read(word),
            Ok((MotorGoal::Cell { x, y }, line.to_owned())),
            "{word}"
        );
    }
    assert_eq!(
        read("hold"),
        Ok((MotorGoal::Hold, DRIVE_HOLDING.to_owned()))
    );
    assert_eq!(read("off"), Ok((MotorGoal::Off, DRIVE_RELEASED.to_owned())));
    assert_eq!(read(""), Err(CockpitError::Usage(DRIVE_USAGE.to_owned())));
    for word in ["64,1", "1,48", "70000,1"] {
        assert_eq!(
            read(word),
            Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned())),
            "{word}"
        );
    }
    for word in ["ba", "x2", "the", "c", "12", "12,"] {
        assert_eq!(
            read(word),
            Err(CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned())),
            "{word}"
        );
    }
    // No ground: every cell is off the window and no word is a spot.
    assert_eq!(
        drive_goal("1,1", &[], 0, 0),
        Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()))
    );
    assert_eq!(
        drive_goal("a", &[], 0, 0),
        Err(CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned()))
    );
    // The motor's two notices, through the one renderer.
    let frame = ap3_scene();
    for (outcome, line) in [
        (DriveOutcome::Arrived, DRIVE_ARRIVED),
        (DriveOutcome::NoRoute, DRIVE_NO_ROUTE),
    ] {
        assert_eq!(
            notice_line(&frame, &FrameNotice::Drive { outcome }, &[]),
            line
        );
    }
    // The arena's own refusal of a goal names the verb the pilot typed.
    let goal = IntentKind::Goal(MotorGoal::Off);
    assert_eq!(goal.as_str(), "drive");
    assert_eq!(
        refusal_line(
            goal.as_str(),
            Refusal::PowerNotOnline,
            None,
            RefusalDetail::None
        ),
        "DRIVE refused. Power not online: STARTUP first."
    );
    assert_eq!(
        refusal_line(goal.as_str(), Refusal::QueueFull, None, RefusalDetail::None),
        "DRIVE refused. Too many orders at once; try that again."
    );
}

/// **17b cut M5.6** (the design note's item 3; doc 17:1740-1742 as
/// corrected; the conventions' §3): `docs/help/minds.md` opens with three
/// lines, exactly, each under 120 columns — and each claim in them is the
/// code's: `SIM MIND` CHOOSES (the lobby's `Your slot will carry …`, read
/// through the verb by `tests/cockpit.rs`' `m5_sniper_line` and
/// `m5_a_chosen_mind_is_named_in_the_pilots_block_from_the_check_out`) and
/// the match's start seals (the check-out's [`lobby::seal_line`], called
/// here, its line quoted by the page); an unchosen slot's tank flies
/// Standard Doctrine on Line, the Registry's own (the lobby's own bare
/// read, the seal's name and charter), which `MIND` reads in the cockpit
/// (the seventeenth selector); and it flies from an `EXIT` to the end, or
/// from a drop past the grace until the end or a `BOARD` (the reconnect
/// line and the board's drop clause) — "board again" belongs to the drop
/// alone (`EXIT` is final, [`EXIT_FINAL`]).
///
/// **17b cut M5.6's review** (R19, R24): the claims are tied by the code's
/// own values, never by grepping a source file (the `EXIT` needle matched
/// this test's own literal); and the page's `MIND` example — its four lines
/// and the ten seconds' refusal — is [`mind_lines`] and [`profile_switch`]
/// through [`order_refused`] on the same frame, so neither drifts from the
/// verb unseen (the block's examples are `report.rs`'
/// `m5_help_minds_quotes_the_block_as_it_is_built`, the presets' terms
/// `card.rs`' `m5_help_minds_names_every_term_a_preset_flies`).
#[test]
fn m5_help_minds_opens_with_three_true_lines() {
    const OPENING: [&str; 3] = [
        "Your tank's core can fly it when you are not in the seat. SIM MIND chooses its mind; \
         the match's start seals it.",
        "With none chosen, a tank you leave flies Standard Doctrine on Line, the Registry's \
         own; MIND reads it in the cockpit.",
        "It flies from when you climb out to the end, or from a drop past the grace until the \
         end or you board again.",
    ];
    let help = include_str!("../../../../../docs/help/minds.md");
    let body: Vec<&str> = help.lines().collect();
    assert_eq!(body.first(), Some(&"# Minds"), "the topic's heading");
    assert_eq!(body.get(1), Some(&""), "then a blank line");
    assert_eq!(
        body.get(2..5),
        Some(&OPENING[..]),
        "the three lines, exactly"
    );
    assert_eq!(
        OPENING.map(|line| line.chars().count()),
        [112, 117, 108],
        "the design note's widths"
    );
    let quoted = |said: &str| help.lines().any(|line| line.trim_start() == said);
    // (1) SIM MIND chooses; the match's start seals, in the check-out's
    // own line, which the page quotes.
    let mut seal = crate::arena::mind::MindSeal::placeholder();
    seal.mind = "Standard Doctrine".into();
    seal.class = crate::arena::core::CoreClass::I;
    let frame = m6_frame(&["Sniper", "Line"]);
    let cfg = crate::config::ReflexConfig::default();
    let selected = ["Sniper", "Line"].map(|name| {
        (
            SmolStr::new_static(name),
            crate::arena::preset(name, &cfg).expect("preset"),
        )
    });
    let definition =
        crate::arena::runtime_card::stock_definition(&selected, crate::arena::core::CoreClass::I)
            .expect("definition");
    seal.cards = crate::arena::profile_seal::SealedCards::prepare(
        definition,
        &["Sniper".into(), "Line".into()],
        &crate::arena::core::ClassTable::STOCK.row(crate::arena::core::CoreClass::I),
        &[],
        &[Channel::Helm, Channel::Sensors],
        100,
    )
    .expect("cards");
    assert_eq!(
        seal.cards.selected.names().collect::<Vec<_>>(),
        frame.profiles.names().collect::<Vec<_>>()
    );
    let sealed = lobby::seal_line(&seal, false).unwrap_or_default();
    assert_eq!(
        sealed,
        "Sealed for this sortie: Standard Doctrine (Sniper, Line), Class I; MIND reads it."
    );
    assert!(quoted(&sealed), "the page quotes the seal line: {sealed}");
    // (2) The default, by the lobby's own words and the seal's identity;
    // MIND, the seventeenth selector, reads it.
    assert_eq!(
        lobby::MIND_UNCHOSEN,
        "Yours: none; a tank you leave flies Standard Doctrine on Line."
    );
    assert_eq!(crate::arena::doctrine::SEALED_MIND, "Standard Doctrine");
    assert_eq!(crate::arena::doctrine::SEALED_CHARTER, "registry");
    assert_eq!(crate::arena::card::PRESETS[0], "Line");
    assert!(COCKPIT_VERBS.contains(&"mind"));
    // (3) A drop is taken back with BOARD and reads `flown by its mind`;
    // an EXIT is not taken back (`EXIT`'s own sentence, above).
    assert_eq!(lobby::DROP_CLAUSE, ", flown by its mind");
    let w = mush_core::World::new_minimal();
    assert!(crate::match_::resume::mind_line(&w, ObjId(1)).ends_with("BOARD takes it back."));
    assert_eq!(
        EXIT_FINAL,
        "is on its own for the rest of the match; you cannot take it back."
    );
    // The page's `MIND` example: the four lines on a hull sealed on Sniper
    // and Line at Class I on a Class II core, and the ten seconds' refusal.
    let sniper_line = FrameMind {
        doctrine: true,
        profiles: m6_frame(&["Sniper", "Line"]).profiles,
        active: 0,
        class: crate::arena::core::CoreClass::I,
        core_class: crate::arena::core::CoreClass::II,
        switch_at_ms: None,
        off: false,
        ..m6_frame(&["Sniper", "Line"])
    };
    for line in mind_lines(&sniper_line, true, 0) {
        assert!(quoted(&line), "the page quotes MIND's line: {line}");
    }
    // 17b cut M5.7 (M5.6's carried item): `help cockpit`'s copy of the
    // four lines ("Your tank's mind") is pinned to the same frame, so the
    // two pages cannot drift apart unseen.
    let cockpit_page = include_str!("../../../../../docs/help/cockpit.md");
    for line in mind_lines(&sniper_line, true, 0) {
        assert!(
            cockpit_page.lines().any(|said| said.trim_start() == line),
            "help cockpit quotes MIND's line: {line}"
        );
    }
    let switched = FrameMind {
        active: 1,
        switch_at_ms: Some(40_000),
        ..sniper_line.clone()
    };
    let refusal = profile_switch(&switched, "Sniper", 34_000).map_or_else(
        |said| order_refused(profile_order(), None, &said),
        |_| String::new(),
    );
    assert_eq!(
        refusal,
        "MIND refused. One switch in ten seconds; the next in 6 s."
    );
    assert!(
        help.contains(&format!("`{refusal}`")),
        "the page quotes the refusal: {refusal}"
    );
    // Every line of the page is under 120 columns.
    for line in help.lines() {
        assert!(line.chars().count() < 120, "{line}");
    }
}

/// **17b cut M5.4** (doc 17 §12 "The `MIND` verb"): the seventeenth
/// selector, appended after `drive`; its help line; its row in the module
/// head's table; its first line in the three readings of the frame (a
/// doctrine seal at the classes read live, a mind that is not Standard
/// Doctrine, a row dropped to the ROM); the keyframe's `mind` object
/// exactly, on an armed hull and an unarmed one, before and after a
/// switch; and the arena's refusal of a switch, which names the verb
/// through [`IntentKind::as_str`].
#[test]
fn m5_mind_is_the_seventeenth_selector_and_its_words_are_pinned() {
    use crate::arena::core::CoreClass;
    use crate::arena::perception::RefusalDetail;
    assert_eq!(COCKPIT_VERBS.len(), 17);
    assert_eq!(COCKPIT_VERBS.get(15), Some(&"drive"));
    assert_eq!(COCKPIT_VERBS.get(16), Some(&"mind"));
    assert_eq!(
        cockpit_help("mind"),
        "MIND [PROFILE <name>|RULES]  your tank's sealed mind: what it is, its profiles and \
         the one it will fly when you leave the seat, and who holds each channel; it is \
         dormant while you fly. MIND PROFILE <name> switches to another sealed profile, \
         once in ten seconds. MIND RULES [slot] reads live pilot rules; MIND RULE <slot> <rule or OFF> replaces or removes one."
    );
    let head = include_str!("../cockpit.rs");
    assert!(
        head.contains(
            "//! MIND [PROFILE <name>|RULES] your tank's sealed mind, dormant while you \
             fly; PROFILE switches its profile"
        ),
        "the module head's MIND row"
    );
    let sniper_line = FrameMind {
        doctrine: true,
        profiles: m6_frame(&["Sniper", "Line"]).profiles,
        active: 0,
        class: CoreClass::I,
        core_class: CoreClass::II,
        switch_at_ms: None,
        off: false,
        ..m6_frame(&["Sniper", "Line"])
    };
    assert_eq!(
        mind_head_line(&sniper_line),
        "Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II \
         core."
    );
    let own = FrameMind {
        name: "Field Pilot".into(),
        doctrine: false,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_head_line(&own),
        "Mind: Field Pilot, sealed for this sortie; it thinks at Class I on a Class II core."
    );
    let off = FrameMind {
        off: true,
        class: CoreClass::Zero,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_head_line(&off),
        "Mind: Standard Doctrine, off for the match: the hull is on the ROM."
    );
    // The keyframe's object, exactly (the M5 design note's M5.4 item 3).
    assert_eq!(
        mind_json(&sniper_line, true),
        serde_json::json!({
            "name": "Standard Doctrine",
            "card": {"schema": sniper_line.card.schema(), "values": sniper_line.card.values()},
            "sealed_rules": [],
            "profiles": ["Sniper", "Line"],
            "active": 0,
            "class": "I",
            "core": "II",
            "channels": ["helm", "sensors", "fire"],
            "next_switch_ms": null,
            "off": false,
        })
    );
    let switched = FrameMind {
        active: 1,
        switch_at_ms: Some(42_000),
        ..own.clone()
    };
    assert_eq!(
        mind_json(&switched, false),
        serde_json::json!({
            "name": "Field Pilot",
            "card": {"schema": switched.card.schema(), "values": switched.card.values()},
            "sealed_rules": [],
            "profiles": ["Sniper", "Line"],
            "active": 1,
            "class": "I",
            "core": "II",
            "channels": ["helm", "sensors"],
            "next_switch_ms": 42_000,
            "off": false,
        })
    );
    // The arena's own refusal of a switch names the verb the pilot typed.
    let switch = IntentKind::Profile { index: 1 };
    assert_eq!(switch.as_str(), "mind");
    assert_eq!(
        refusal_line(switch.as_str(), Refusal::Profile, None, RefusalDetail::None),
        "MIND refused. That profile cannot be switched to now."
    );
}

/// Every ordered set of one to four of the six presets — every
/// [`crate::arena::card::ProfileSet`] a seal can hold (17b cut M5.4's
/// width tests walk them all).
fn m5_every_profile_set() -> Vec<crate::arena::runtime_card::RuntimeProfiles> {
    use crate::arena::card::{PRESETS, ProfileSet};
    let mut sets: Vec<Vec<&str>> = vec![Vec::new()];
    let mut all = Vec::new();
    for _ in 0..ProfileSet::MOST {
        let mut longer = Vec::new();
        for set in &sets {
            for name in PRESETS {
                if !set.contains(&name) {
                    let mut next = set.clone();
                    next.push(name);
                    longer.push(next);
                }
            }
        }
        all.extend(longer.iter().cloned());
        sets = longer;
    }
    all.iter().map(|names| m6_frame(names).profiles).collect()
}

/// Whether a line holds a word of sixteen hex digits — a seal or contact
/// id, which no line a pilot reads may carry.
fn m5_has_sixteen_hex(line: &str) -> bool {
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word.len() >= 16 && word.chars().all(|c| c.is_ascii_hexdigit()))
}

/// **17b cut M5.4** (doc 17 §12 "The `MIND` verb"; the M5 design note's
/// M5.4 item 4; OQ-8): bare `MIND`'s block is at most seven lines, each
/// under 120 columns and free of any word of sixteen hex digits, over
/// EVERY sealed set (one to four presets, the widest pair `Skirmisher,
/// Rearguard` among them) at every active index, a Doctrine seal and a
/// non-Doctrine one, an armed hull and an unarmed one, a live row and a
/// dropped one, a hull with no core above Class 0 and one sealed at Class
/// 0 on a real core, before a switch, inside
/// the ten seconds and after them; and so are `MIND RULES`' line and every `MIND PROFILE`
/// sentence (each refusal composed through [`order_refused`], the
/// acknowledgement). The block's words are pinned exactly on the note's
/// examples: the widest pair, an unarmed hull, a dropped row, a
/// non-Doctrine seal, the set of one every unchosen slot seals, a running
/// countdown — and [`NO_SEAL`], what a hull no seal reached reads.
#[test]
fn m5_the_mind_block_is_at_most_seven_lines_under_120_columns() {
    use crate::arena::core::CoreClass;
    let set = |names: &[&str]| m6_frame(names).profiles;
    let sniper_line = FrameMind {
        doctrine: true,
        profiles: set(&["Sniper", "Line"]),
        active: 0,
        class: CoreClass::I,
        core_class: CoreClass::II,
        switch_at_ms: None,
        off: false,
        ..m6_frame(&["Sniper", "Line"])
    };
    // The note's block, exactly.
    assert_eq!(
        mind_lines(&sniper_line, true, 90_000),
        [
            "Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class \
             II core.",
            "Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten \
             seconds.",
            "Channels: you hold helm, sensors and fire; the mind is dormant while you fly.",
            "Rules: none sealed; your own armings are under WEAPONSTATUS.",
        ]
    );
    // The widest pair, unarmed, the second profile flown, six seconds
    // (rounded up) from the next switch on the mirror's clock.
    let widest = FrameMind {
        profiles: set(&["Skirmisher", "Rearguard"]),
        active: 1,
        switch_at_ms: Some(40_000),
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_lines(&widest, false, 34_001),
        [
            "Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class \
             II core.",
            "Profile: Rearguard, 2 of 2 (also Skirmisher); one switch in ten seconds, the \
             next in 6 s.",
            "Channels: you hold helm and sensors (no weapons station on this hull); the mind \
             is dormant while you fly.",
            "Rules: none sealed; your own armings are under WEAPONSTATUS.",
        ]
    );
    // At the next switch's millisecond the countdown is gone.
    assert_eq!(
        mind_profile_line(&widest, 40_000),
        "Profile: Rearguard, 2 of 2 (also Skirmisher); MIND PROFILE Skirmisher switches, \
         once in ten seconds."
    );
    assert_eq!(
        crate::arena::intents::PROFILE_SWITCH_MS,
        10_000,
        "the block says ten seconds"
    );
    // A dropped row: only the first line moves (the ROM flies the card
    // the profile names, so the switch still means something).
    let dropped = FrameMind {
        off: true,
        class: CoreClass::Zero,
        ..sniper_line.clone()
    };
    let lines = mind_lines(&dropped, true, 0);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Mind: Standard Doctrine, off for the match: the hull is on the ROM.")
    );
    assert_eq!(lines.get(1..), mind_lines(&sniper_line, true, 0).get(1..));
    // A non-Doctrine seal names no mind.
    let own = FrameMind {
        name: "Field Pilot".into(),
        doctrine: false,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_head_line(&own),
        "Mind: Field Pilot, sealed for this sortie; it thinks at Class I on a Class II core."
    );
    // A hull with no core above Class 0 (`target_mk1` has none).
    let coreless = FrameMind {
        class: CoreClass::Zero,
        core_class: CoreClass::Zero,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_head_line(&coreless),
        "Mind: Standard Doctrine, sealed for this sortie; with no core above Class 0, the \
         hull is on the ROM."
    );
    // A hull sealed at Class 0 on a real core (`fill_class` "0") is the
    // ROM's as well: the class the ROM rule reads, never "thinks at Class
    // 0".
    let capped = FrameMind {
        class: CoreClass::Zero,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_head_line(&capped),
        "Mind: Standard Doctrine, sealed for this sortie at Class 0 on a Class II core: the \
         hull is on the ROM."
    );
    assert_eq!(
        mind_head_line(&FrameMind {
            name: "Field Pilot".into(),
            doctrine: false,
            core_class: CoreClass::III,
            ..capped.clone()
        }),
        "Mind: Field Pilot, sealed for this sortie at Class 0 on a Class III core: the hull \
         is on the ROM."
    );
    // The set of one every unchosen slot seals.
    let line_alone = FrameMind {
        profiles: m6_frame(&["Line"]).profiles,
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_profile_line(&line_alone, 0),
        "Profile: Line, 1 of 1; no other is sealed to switch to."
    );
    assert_eq!(
        mind_rules_line(&sniper_line),
        "No rules are sealed on Sniper or Line; your own armings are under WEAPONSTATUS."
    );
    assert_eq!(
        mind_rules_line(&line_alone),
        "No rules are sealed on Line; your own armings are under WEAPONSTATUS."
    );
    assert_eq!(NO_SEAL, "This hull carries no seal.");
    // Three or four: the target is the form, the others listed.
    let four = FrameMind {
        profiles: set(&["Skirmisher", "Rearguard", "Sniper", "Hunter"]),
        ..sniper_line.clone()
    };
    assert_eq!(
        mind_profile_line(&four, 0),
        "Profile: Skirmisher, 1 of 4 (also Rearguard, Sniper and Hunter); MIND PROFILE <name> \
         switches, once in ten seconds."
    );

    // Every set, every reading: the bounds.
    let base_frame = m6_frame(&["Line"]);
    let sets = m5_every_profile_set();
    assert_eq!(
        sets.len(),
        6 + 30 + 120 + 360,
        "every ordered set of 1 to 4"
    );
    let mut widest_line = 0;
    let mut checked = 0_u32;
    let mut bounded = |said: &[String]| {
        for line in said {
            let width = line.chars().count();
            assert!(width < 120, "{width}: {line}");
            assert!(!m5_has_sixteen_hex(line), "{line}");
            widest_line = widest_line.max(width);
        }
        checked = checked.saturating_add(1);
    };
    for profiles in sets {
        for active in 0..u8::try_from(profiles.len()).expect("bounded set") {
            for switch_at_ms in [None, Some(40_000)] {
                // 30,001: ten seconds to go, rounded up.
                for now_ms in [0, 30_001, 40_000] {
                    // The switch's sentences read the set, the index and
                    // the clock alone.
                    let mind = FrameMind {
                        doctrine: true,
                        profiles: profiles.clone(),
                        active,
                        class: CoreClass::I,
                        core_class: CoreClass::II,
                        switch_at_ms,
                        off: false,
                        ..base_frame.clone()
                    };
                    let mut said = vec![mind_rules_line(&mind)];
                    for word in crate::arena::card::PRESETS
                        .iter()
                        .copied()
                        .chain(["bogus", ""])
                    {
                        said.push(match profile_switch(&mind, word, now_ms) {
                            Ok((_, name)) => profile_ack(name),
                            Err(why) => order_refused(profile_order(), None, &why),
                        });
                    }
                    bounded(&said);
                    for doctrine in [true, false] {
                        for (off, class, core_class) in [
                            (false, CoreClass::I, CoreClass::II),
                            (false, CoreClass::III, CoreClass::III),
                            (false, CoreClass::Zero, CoreClass::Zero),
                            (false, CoreClass::Zero, CoreClass::III),
                            (true, CoreClass::Zero, CoreClass::II),
                        ] {
                            let mind = FrameMind {
                                doctrine,
                                class,
                                core_class,
                                off,
                                ..mind.clone()
                            };
                            for armed in [true, false] {
                                let said = mind_lines(&mind, armed, now_ms);
                                assert!(said.len() <= 7, "{said:?}");
                                bounded(&said);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 0);
    assert!(widest_line < 120, "{widest_line}");
}

/// **17b cut M5.4** (the M5 design note's M5.4 item 5): every `MIND
/// PROFILE` pre-check refuses off the frame in one line opening `MIND
/// refused.`, composed through [`order_refused`] from the intent's own
/// word exactly as the arena's `profile` refusal is: a preset the mind
/// does not carry (named as the preset is, never as typed), a word no
/// preset answers (never echoed), the profile already flown (the arena
/// refuses the same case, so the same prefix), and a switch inside the
/// ten seconds — its seconds read against the clock the caller hands it
/// (the mirror's arena clock, `mind_now_ms`), rounded up, gone at the
/// next switch's millisecond; and a cold hull's switch is not refused
/// (no power test: the frame carries none to read). A word in any case
/// names its preset; the acknowledgement names the profile the mind will
/// fly. `MIND TRACE` is refused in its own form, and [`NO_SEAL`] and
/// [`NO_FRAME_YET`] after the prefix read as the arena's words do.
#[test]
fn m5_mind_profile_refuses_off_the_frame_in_one_line() {
    use crate::arena::core::CoreClass;
    let refused = |said: &str| order_refused(profile_order(), None, said);
    let mind = FrameMind {
        doctrine: true,
        profiles: m6_frame(&["Sniper", "Line"]).profiles,
        active: 0,
        class: CoreClass::I,
        core_class: CoreClass::II,
        switch_at_ms: None,
        off: false,
        ..m6_frame(&["Sniper", "Line"])
    };
    let check = |mind: &FrameMind, word: &str, now_ms: i64| {
        profile_switch(mind, word, now_ms)
            .map(|(index, name)| (index, profile_ack(name)))
            .map_err(|said| refused(&said))
    };
    assert_eq!(
        check(&mind, "Hunter", 0),
        Err(
            "MIND refused. Hunter is not sealed on this mind; it carries Sniper and Line."
                .to_owned()
        )
    );
    assert_eq!(
        check(&mind, "hUNTER", 0),
        check(&mind, "Hunter", 0),
        "the preset's own name"
    );
    assert_eq!(
        check(&mind, "berserker", 0),
        Err(
            "MIND refused. No profile of that name is sealed on this mind; it carries Sniper \
             and Line."
                .to_owned()
        )
    );
    assert_eq!(
        check(&mind, "sniper", 0),
        Err("MIND refused. Sniper is already the profile.".to_owned())
    );
    assert_eq!(
        check(&mind, "LINE", 0),
        Ok((
            1,
            "Profile Line: the mind will fly it when you leave the seat.".to_owned()
        ))
    );
    // Switched to Line at 30,000: the next is allowed at 40,000.
    let switched = FrameMind {
        active: 1,
        switch_at_ms: Some(40_000),
        ..mind.clone()
    };
    for (now_ms, secs) in [(30_000, 10), (34_000, 6), (34_001, 6), (39_999, 1)] {
        assert_eq!(
            check(&switched, "Sniper", now_ms),
            Err(format!(
                "MIND refused. One switch in ten seconds; the next in {secs} s."
            )),
            "at {now_ms} ms"
        );
        assert_eq!(next_switch_s(&switched, now_ms), Some(secs));
    }
    assert_eq!(
        check(&switched, "Sniper", 40_000),
        Ok((
            0,
            "Profile Sniper: the mind will fly it when you leave the seat.".to_owned()
        ))
    );
    assert_eq!(next_switch_s(&switched, 40_000), None);
    // The profile already flown is said before the window.
    assert_eq!(
        check(&switched, "line", 31_000),
        Err("MIND refused. Line is already the profile.".to_owned())
    );
    // A dropped row, a coreless hull and a hull sealed at Class 0 still
    // switch: the ROM flies the card the profile names (17b cut M5.2's
    // adoption on the row).
    for odd in [
        FrameMind {
            off: true,
            class: CoreClass::Zero,
            ..mind.clone()
        },
        FrameMind {
            class: CoreClass::Zero,
            core_class: CoreClass::Zero,
            ..mind.clone()
        },
        FrameMind {
            class: CoreClass::Zero,
            ..mind.clone()
        },
    ] {
        assert_eq!(check(&odd, "Line", 0).map(|(index, _)| index), Ok(1));
    }
    // The set of one has nothing to switch to.
    let alone = FrameMind {
        profiles: m6_frame(&["Line"]).profiles,
        ..mind.clone()
    };
    assert_eq!(
        check(&alone, "line", 0),
        Err("MIND refused. Line is already the profile.".to_owned())
    );
    // One prefix for every refused switch, whichever side refuses it.
    assert_eq!(refused(NO_SEAL), "MIND refused. This hull carries no seal.");
    assert_eq!(
        refused(NO_FRAME_YET),
        "MIND refused. The panel has told you nothing yet."
    );
    assert_eq!(
        refused(&CockpitError::Destroyed.to_string()),
        "MIND refused. Your tank is destroyed and takes no orders. EXIT leaves the pod."
    );
    assert_eq!(
        refusal_line(profile_order(), Refusal::Profile, None, RefusalDetail::None),
        refused(PROFILE_REFUSED)
    );
    assert_eq!(
        order_refused("mind trace", None, MIND_NO_TRACE),
        "MIND TRACE refused. No mind trace is available."
    );
    assert_eq!(
        MIND_USAGE,
        "MIND [PROFILE <name>|CARD|RULES [slot]|RULE <slot> <rule or OFF>|TRACE [1..64]]; bare MIND reads your sealed mind."
    );
}

// -----------------------------------------------------------------------
// 16b cut AP.1
// -----------------------------------------------------------------------

/// **`B-86`, the formula and the floor skip**: a cycling cannon's countdown
/// is the frame's own less the wall time since receipt, held at nought and
/// ready there; a ready row stays ready at every instant; and a floored read
/// is [`frame_weapons`]'s, untouched, at every instant.
///
/// The frame is a real one, the opening frame of a stepped scout-and-tank
/// arena, with its rack replaced by the two rows this is about.
#[test]
fn ap1_the_aged_rows_count_down_from_receipt_and_a_floor_leaves_them_alone() {
    use crate::arena::state::PartyId;
    use crate::arena::test_support::{arena_at_epoch, pose, scout, tank};
    use crate::arena::weapons::FrameWeapon;
    use std::time::{Duration, Instant};
    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    let mut arena = arena_at_epoch(
        vec![
            scout(RED, PartyId::RED, pose(1_500, 1_500)),
            tank(BLUE, PartyId::BLUE, pose(6_500, 6_500)),
        ],
        1,
    );
    let mut frame = arena
        .step(0, &[], &[])
        .frames
        .into_iter()
        .map(|(_, frame)| frame)
        .find(|frame| frame.unit == RED)
        .expect("red is sent its opening frame");
    let row = |id: &'static str, ready: bool, ready_in_ms: i64| FrameWeapon {
        id: SmolStr::new_static(id),
        ready,
        ready_in_ms,
        rounds: 10,
        rounds_max: 10,
        range_band: None,
        p_permille: None,
        bands_cm: vec![100_000],
        heat_per_shot: 0,
        armed: None,
    };
    frame.weapons = vec![row("cannon", false, 2_000), row("machine_gun", true, 0)];
    let now = Instant::now();
    let held_at = |back_ms: u64| CockpitFrame {
        match_id: crate::arena_task::MatchId(1),
        frame: frame.clone(),
        ranges: BTreeMap::new(),
        arena_ms: 0,
        received: now
            .checked_sub(Duration::from_millis(back_ms))
            .expect("the clock reaches that far back"),
    };
    let cannon = |rows: &[WeaponRow]| {
        rows.iter()
            .find(|row| row.id == "cannon")
            .map(|row| (row.ready, row.ready_in_ms))
            .expect("the cannon row")
    };
    let gun = |rows: &[WeaponRow]| {
        rows.iter()
            .find(|row| row.id == "machine_gun")
            .map(|row| (row.ready, row.ready_in_ms))
            .expect("the machine gun row")
    };

    // Half a second to go, and the line says so, rounded up.
    let rows = aged_weapons(&held_at(1_500), now, false);
    assert_eq!(cannon(&rows), (false, 500));
    let line = rows
        .iter()
        .find(|row| row.id == "cannon")
        .map(|row| row.render(&rows))
        .expect("the cannon row");
    assert!(line.starts_with("cannon  cycling (1 s)  "), "{line}");
    // At the ready instant and long past it: ready, at nought.
    for back_ms in [2_000, 9_000] {
        let rows = aged_weapons(&held_at(back_ms), now, false);
        assert_eq!(cannon(&rows), (true, 0), "{back_ms} ms after receipt");
    }
    // The ready row stays ready, and nothing but the countdown moves.
    for back_ms in [0, 1_500, 2_000, 9_000] {
        let held = held_at(back_ms);
        let rows = aged_weapons(&held, now, false);
        assert_eq!(gun(&rows), (true, 0), "{back_ms} ms after receipt");
        for (aged, own) in rows.iter().zip(frame_weapons(&held.frame)) {
            assert_eq!(
                WeaponRow {
                    ready: own.ready,
                    ready_in_ms: own.ready_in_ms,
                    ..aged.clone()
                },
                own,
                "only the countdown is aged"
            );
        }
        // Floored: the frame's own rows, untouched, at every instant.
        assert_eq!(
            aged_weapons(&held, now, true),
            frame_weapons(&held.frame),
            "{back_ms} ms after receipt, under a floor"
        );
    }
    // At receipt itself, the frame's own figure.
    assert_eq!(
        cannon(&aged_weapons(&held_at(0), now, false)),
        (false, 2_000)
    );
}

// -----------------------------------------------------------------------
// 16b cut AP.3
// -----------------------------------------------------------------------

/// **The help's own scene** (16b cut AP.3): `RA` online at 24 kph heading
/// east, a cannon ready and a machine gun three seconds from it, locked on
/// `BA` (a fresh plot at 450 m, `far` on the shipped 600 m sight) beside an
/// unidentified `X3` whose plot is four and a half seconds old. A real
/// opening frame of a stepped arena, its fields replaced.
fn ap3_scene() -> Frame {
    use crate::arena::perception::FrameContact;
    use crate::arena::state::PartyId;
    use crate::arena::test_support::{arena_at_epoch, pose, scout, tank};
    use crate::arena::weapons::FrameWeapon;
    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    let mut arena = arena_at_epoch(
        vec![
            scout(RED, PartyId::RED, pose(1_500, 1_500)),
            tank(BLUE, PartyId::BLUE, pose(6_500, 6_500)),
        ],
        1,
    );
    let mut frame = arena
        .step(0, &[], &[])
        .frames
        .into_iter()
        .map(|(_, frame)| frame)
        .find(|frame| frame.unit == RED)
        .expect("red is sent its opening frame");
    frame.callsign = Designator::Slot {
        team: PartyId::RED,
        slot: 1,
    };
    frame.party = PartyId::RED;
    frame.power = Power::Online;
    frame.speed_cms = 667;
    frame.pose.heading_mdeg = 90_000;
    frame.held = false;
    frame.wreck = false;
    frame.sight_cm = 60_000;
    frame.notices.clear();
    frame.contacts = vec![
        FrameContact {
            id: ContactId(1),
            designator: Designator::Slot {
                team: PartyId::BLUE,
                slot: 1,
            },
            alias: Some(1),
            class: Classification::Classified { party: PartyId::BLUE },
            moving: false,
            bearing_mdeg: 87_000,
            range_cm: 45_000,
            age_ms: 800,
            cell: Some((6, 2)),
            condition: None,
            aspect: crate::arena::perception::Aspect::Front,
            facing: crate::arena::perception::ContactFacing::Front,
            reads_dead: None,
            fatal: None,
            unit_type: None,
        },
        FrameContact {
            id: ContactId(2),
            designator: Designator::Provisional(3),
            alias: None,
            class: Classification::Unknown,
            moving: false,
            bearing_mdeg: 315_000,
            range_cm: 22_000,
            age_ms: 4_500,
            cell: Some((0, 3)),
            condition: None,
            aspect: crate::arena::perception::Aspect::Front,
            facing: crate::arena::perception::ContactFacing::Front,
            reads_dead: None,
            fatal: None,
            unit_type: None,
        },
    ];
    frame.lock = Some(ContactId(1));
    let gun = |id: &'static str, ready: bool, ready_in_ms: i64, p: u32| FrameWeapon {
        id: SmolStr::new_static(id),
        ready,
        ready_in_ms,
        rounds: 10,
        rounds_max: 20,
        range_band: None,
        p_permille: Some(p),
        bands_cm: vec![100_000, 300_000],
        heat_per_shot: 0,
        armed: None,
    };
    frame.weapons = vec![
        gun("cannon", true, 0, 460),
        gun("machine_gun", false, 2_500, 210),
    ];
    frame
}

/// The scene's three lines, over its own rows and no range memory.
fn ap3_lines(frame: &Frame) -> Vec<String> {
    watch_lines(frame, &BTreeMap::new(), &frame_weapons(frame), 1_000)
}

/// **16b cut AP.3** (16c N19): the digest is the help's three lines for the
/// help's numbers, one line for a bare platform, a plot's age only once it
/// is stale, a `; +N more` tail that keeps every line within 120 columns on
/// both fitted lines, the flags in `STATUS`'s order, the friendly mark —
/// and no prefix any reader of a `STATUS`, `CONTACTS` or lock line could
/// take, no tempo word and nothing that moves with the platform's cell.
#[test]
fn ap3_watch_lines_are_the_three_lines_within_120_columns() {
    use crate::arena::clock::Tempo;
    use crate::arena::perception::FrameContact;
    use crate::arena::state::PartyId;
    use crate::arena::weapons::FrameWeapon;

    // (1) `docs/help/cockpit.md`'s "Watching from line mode", verbatim.
    let scene = ap3_scene();
    assert_eq!(
        ap3_lines(&scene),
        vec![
            "Watch RA: online, 24 kph, heading 090°".to_owned(),
            "Watch guns: cannon ready, machine_gun (mg) cycling (3 s); lock BA, 450 m, 46% \
             to hit"
                .to_owned(),
            "Watch contacts: BA blue 087° far 450 m; X3 unknown 315° effective 200 m, \
             plotted 4 s ago"
                .to_owned(),
        ]
    );
    let help = include_str!("../../../../../docs/help/cockpit.md");
    for line in ap3_lines(&scene) {
        assert!(
            help.contains(&format!("  {line}\n")),
            "the help's example omits {line:?}"
        );
    }

    // (2) No gun and no contact: the own line alone.
    let mut bare = scene.clone();
    bare.weapons.clear();
    bare.contacts.clear();
    bare.lock = None;
    assert_eq!(
        ap3_lines(&bare),
        vec!["Watch RA: online, 24 kph, heading 090°".to_owned()]
    );
    // A gun and no lock, and a lock whose chance is not resolved yet.
    let mut unlocked = scene.clone();
    unlocked.lock = None;
    assert!(
        ap3_lines(&unlocked)
            .get(1)
            .is_some_and(|line| line.ends_with("cycling (3 s); no lock")),
        "{:?}",
        ap3_lines(&unlocked)
    );
    let mut unresolved = scene.clone();
    for weapon in &mut unresolved.weapons {
        weapon.p_permille = None;
    }
    assert!(
        ap3_lines(&unresolved)
            .get(1)
            .is_some_and(|line| line.ends_with("; lock BA, 450 m")),
        "{:?}",
        ap3_lines(&unresolved)
    );
    // An empty gun reads `empty`, ready or cycling — `FIRE` refuses it
    // either way — so the last round leaving changes the block.
    let mut dry = scene.clone();
    for weapon in &mut dry.weapons {
        weapon.rounds = 0;
    }
    assert_eq!(
        ap3_lines(&dry).get(1).map(String::as_str),
        Some("Watch guns: cannon empty, machine_gun (mg) empty; lock BA, 450 m, 46% to hit"),
        "an empty gun read as ready or cycling"
    );
    assert_ne!(ap3_lines(&dry), ap3_lines(&scene));

    // (3) A plot's age prints only once it is older than the pass.
    let mut aged = scene.clone();
    if let Some(contact) = aged.contacts.first_mut() {
        contact.age_ms = 1_000;
    }
    let at_pass = ap3_lines(&aged);
    assert!(
        at_pass
            .get(2)
            .is_some_and(|line| line.starts_with("Watch contacts: BA blue 087° far 450 m; ")),
        "{at_pass:?}"
    );
    if let Some(contact) = aged.contacts.first_mut() {
        contact.age_ms = 1_001;
    }
    let past_pass = ap3_lines(&aged);
    assert!(
        past_pass
            .get(2)
            .is_some_and(|line| line
                .starts_with("Watch contacts: BA blue 087° far 450 m, plotted 1 s ago; ")),
        "{past_pass:?}"
    );

    // (4) Twenty-three contacts of the widest shipped words: one line,
    // within 120, a prefix of the list in frame order and the count of
    // the rest.
    let mut crowded = scene.clone();
    crowded.sight_cm = 0;
    crowded.lock = None;
    crowded.contacts = (0_u16..23)
        .map(|n| FrameContact {
            id: ContactId(100 + u64::from(n)),
            designator: Designator::Provisional(10 + n),
            alias: None,
            class: Classification::Unknown,
            moving: false,
            bearing_mdeg: 315_000,
            range_cm: 150_000,
            age_ms: 12_000,
            cell: Some((1, 1)),
            condition: None,
            aspect: crate::arena::perception::Aspect::Front,
            facing: crate::arena::perception::ContactFacing::Front,
            reads_dead: None,
            fatal: None,
            unit_type: None,
        })
        .collect();
    let lines = ap3_lines(&crowded);
    let contacts = lines.get(2).expect("a contacts line");
    assert!(contacts.chars().count() <= WATCH_COLUMNS, "{contacts}");
    let (listed, more) = contacts
        .rsplit_once("; +")
        .expect("twenty-three contacts do not fit one line");
    let more: usize = more
        .strip_suffix(" more")
        .and_then(|n| n.parse().ok())
        .expect("the tail counts what it left out");
    let shown: Vec<&str> = listed
        .strip_prefix("Watch contacts: ")
        .expect("the head")
        .split("; ")
        .collect();
    assert!(!shown.is_empty(), "{contacts}");
    assert_eq!(shown.len() + more, 23, "{contacts}");
    for (n, entry) in shown.iter().enumerate() {
        assert_eq!(
            *entry,
            format!(
                "X{} unknown 315° effective 1500 m, plotted 12 s ago",
                10 + n
            ),
            "the entries are the first of the list, in its order: {contacts}"
        );
    }

    // (5) Twelve guns likewise, and the lock is still said after the tail.
    let mut armed = scene.clone();
    armed.weapons = (0..12)
        .map(|n| FrameWeapon {
            id: SmolStr::new(format!("heavy_laser{n}")),
            ready: false,
            ready_in_ms: 11_500,
            rounds: 10,
            rounds_max: 10,
            range_band: None,
            p_permille: Some(460),
            bands_cm: vec![100_000, 300_000],
            heat_per_shot: 0,
            armed: None,
        })
        .collect();
    let lines = ap3_lines(&armed);
    let guns = lines.get(1).expect("a guns line");
    assert!(guns.chars().count() <= WATCH_COLUMNS, "{guns}");
    assert!(guns.ends_with("; lock BA, 450 m, 46% to hit"), "{guns}");
    let (listed, rest) = guns.split_once("; +").expect("twelve guns do not fit");
    let more: usize = rest
        .split_once(" more; lock ")
        .and_then(|(n, _)| n.parse().ok())
        .expect("the tail before the lock");
    let shown: Vec<&str> = listed
        .strip_prefix("Watch guns: ")
        .expect("the head")
        .split(", ")
        .collect();
    assert_eq!(shown.len() + more, 12, "{guns}");
    for (n, entry) in shown.iter().enumerate() {
        // Each shares `hl` with the others, so none advertises it.
        assert_eq!(*entry, format!("heavy_laser{n} cycling (12 s)"), "{guns}");
    }

    // (6) The flags, in `STATUS`'s order; the friendly mark on a lock.
    let mut wreck = scene.clone();
    wreck.held = true;
    wreck.wreck = true;
    assert_eq!(
        ap3_lines(&wreck).first().map(String::as_str),
        Some("Watch RA: online, 24 kph, heading 090°, HELD, DESTROYED")
    );
    let mut friendly = scene.clone();
    friendly.contacts.push(FrameContact {
        id: ContactId(3),
        designator: Designator::Slot {
            team: PartyId::RED,
            slot: 2,
        },
        alias: Some(2),
        class: Classification::Classified { party: PartyId::RED },
        moving: false,
        bearing_mdeg: 180_000,
        range_cm: 12_000,
        age_ms: 0,
        cell: Some((1, 0)),
        condition: None,
        aspect: crate::arena::perception::Aspect::Front,
        facing: crate::arena::perception::ContactFacing::Front,
        reads_dead: None,
        fatal: None,
        unit_type: None,
    });
    friendly.lock = Some(ContactId(3));
    assert!(
        ap3_lines(&friendly)
            .get(1)
            .is_some_and(|line| line.ends_with("; lock RB (FRIENDLY), 100 m, 46% to hit")),
        "{:?}",
        ap3_lines(&friendly)
    );

    // (7) No prefix a reader of `STATUS`, `CONTACTS` or the lock takes; no
    // tempo word; nothing that moves with the cell, the position or the
    // arena's tempo.
    for frame in [&scene, &bare, &wreck, &friendly, &crowded, &armed] {
        let lines = ap3_lines(frame);
        for line in &lines {
            assert!(line.chars().count() <= WATCH_COLUMNS, "{line}");
            assert!(line.starts_with("Watch "), "{line}");
            assert!(!line.contains(": power "), "{line}");
            assert!(!line.starts_with("Locked on "), "{line}");
            assert!(!line.contains("  bearing "), "{line}");
            for word in ["quiet", "alert", "engaged", "Quiet", "Alert", "Engaged"] {
                assert!(!line.contains(word), "a tempo word: {line}");
            }
        }
        for tempo in [Tempo::Quiet, Tempo::Alert, Tempo::Engaged] {
            let mut moved = (*frame).clone();
            moved.tempo = tempo;
            moved.cell = Some((7, 7));
            moved.pose.x_cm += 4_000;
            moved.pose.y_cm -= 3_000;
            assert_eq!(ap3_lines(&moved), lines, "the block moved with {tempo:?}");
        }
    }
}

/// **17b cut M1a.5: the seat grants the channel table** (doc 17 §4,
/// D-UM-47): `cockpit_channels()` is exactly the set of
/// `COCKPIT_CHANNEL_IDS`, which is the arena's three channels' words.
#[test]
fn m1a5_the_seat_grants_the_channel_table() {
    let granted: BTreeSet<String> = cockpit_channels()
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    let table: BTreeSet<String> = crate::arena::state::COCKPIT_CHANNEL_IDS
        .iter()
        .map(|id| (*id).to_owned())
        .collect();
    assert_eq!(granted, table);
    assert_eq!(granted.len(), Channel::ALL.len());
}

// -----------------------------------------------------------------------
// 17b cut M1b.1
// -----------------------------------------------------------------------

/// **17b cut M1b.1**: the three new refusal words' sentences — the floor's,
/// naming the floor in whole per cents, its fallback for an `Unlawful` with
/// no floor (a `Contact` target), the count's and the warrant's — each
/// through the one prefix every `FIRE` refusal has; and the two
/// `CockpitError`s as their one home.
#[test]
fn m1b1_the_new_refusals_say_their_sentences() {
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Unlawful,
            Some("cannon"),
            RefusalDetail::Floor { permille: 100 }
        ),
        "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
    );
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Unlawful,
            Some("cannon"),
            RefusalDetail::Floor { permille: 50 }
        ),
        "FIRE cannon refused. The Compact floor is 5%; a condition may not ask for less."
    );
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Unlawful,
            Some("cannon"),
            RefusalDetail::None
        ),
        "FIRE cannon refused. That rule is not lawful for this unit."
    );
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::NoSlot,
            Some("missiles"),
            RefusalDetail::None
        ),
        "FIRE missiles refused. Every rule slot is occupied; MIND RULE <slot> OFF frees one."
    );
    assert_eq!(
        refusal_line(
            "fire",
            Refusal::Unwarranted,
            Some("cannon"),
            RefusalDetail::None
        ),
        "FIRE cannon refused. That has no warrant."
    );
    assert_eq!(
        CockpitError::Unlawful { floor_pct: 10 }.to_string(),
        "The Compact floor is 10%; a condition may not ask for less."
    );
    // 17b cut M5.2: the arena's word for a `MIND PROFILE`, composed from
    // the intent's own word as every staged refusal is.
    assert_eq!(
        refusal_line(
            IntentKind::Profile { index: 1 }.as_str(),
            Refusal::Profile,
            None,
            RefusalDetail::None
        ),
        "MIND refused. That profile cannot be switched to now."
    );
    assert_eq!(
        CockpitError::NoSlot.to_string(),
        "Every rule slot is occupied; MIND RULE <slot> OFF frees one."
    );
    // The pre-check's half composes the same string (M1b.4's path).
    assert_eq!(
        fire_refused("cannon", &CockpitError::Unlawful { floor_pct: 10 }).to_string(),
        "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
    );
}

/// A miss's launch cell is the gunner's own fact even when its target and
/// reason were redacted. The line stays bare; the keyframe carries the cell.
#[test]
fn m5r_miss_keyframes_keep_the_own_launch_cell_after_target_redaction() {
    let mut frame = ap3_scene();
    frame.notices = vec![
        FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new_static(""),
            launch_cell: Some((2, 7)),
        },
        FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new_static(""),
            launch_cell: None,
        },
    ];
    let keyframe = status_frame(crate::arena_task::MatchId(1), &frame);
    let notices = keyframe["notices"].as_array().expect("notices");
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[0]["launch_cell"], serde_json::json!([2, 7]));
    assert_eq!(notices[1]["launch_cell"], serde_json::Value::Null);
    for notice in notices {
        assert_eq!(notice["text"], "Miss.");
        assert_eq!(notice["contact"], serde_json::Value::Null);
    }
}

/// **17b cut M1b.1**: the two slot notices' sentences (defined here,
/// staged by 17b cut M1b.4) over both halves of `SlotFired`'s contact and
/// every `HeldWhy`, and their keyframe objects' keys — every other notice's
/// object unmoved, and a refused `unlawful` carrying `floor_permille`.
#[test]
fn m1b1_the_slot_notices_say_their_sentences_and_carry_their_keys() {
    use crate::arena::conditions::HeldWhy;
    use crate::arena::perception::{Aspect, Named};
    let mut frame = ap3_scene();
    let ba = frame_contacts(&frame)
        .into_iter()
        .find(|row| row.designator.to_string() == "BA")
        .map(|row| Named {
            id: row.id,
            designator: row.designator,
        })
        .expect("the scene's BA");
    let held = |why: HeldWhy| FrameNotice::SlotHeld {
        slot: 1,
        weapon: SmolStr::new("cannon"),
        why,
    };
    let cases = [
        (
            FrameNotice::SlotFired {
                slot: 1,
                weapon: SmolStr::new("cannon"),
                contact: Some(ba),
                p_permille: 620,
                aspect: Aspect::Left,
            },
            "Cannon fires at BA by S1 (62%, side).",
        ),
        (
            FrameNotice::SlotFired {
                slot: 2,
                weapon: SmolStr::new("machine_gun"),
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
                range_m: 1_200,
                min_m: 200,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 1200 m, outside 200 to 600.",
        ),
        (
            held(HeldWhy::Range {
                contact: ba,
                range_m: 1_200,
                min_m: 0,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 1200 m, outside under 600.",
        ),
        (
            held(HeldWhy::Range {
                contact: ba,
                range_m: 100,
                min_m: 200,
                max_m: u32::MAX,
            }),
            "S1 held the cannon: BA at 100 m, outside past 200.",
        ),
        (
            held(HeldWhy::Rounds {
                rounds: 8,
                keeps: 8,
            }),
            "S1 held the cannon: 8 rounds, keeps 8.",
        ),
    ];
    let rows = frame_contacts(&frame);
    for (notice, said) in &cases {
        assert_eq!(notice_line(&frame, notice, &rows), *said);
        assert_eq!(notice.section(), None);
        assert!(!notice.spent());
        assert_eq!(notice.detail(), RefusalDetail::None);
    }
    // The keyframe's objects.
    frame.notices = cases.iter().map(|(notice, _)| notice.clone()).collect();
    frame.notices.push(FrameNotice::Refused {
        order: SmolStr::new("fire"),
        refusal: Refusal::Unlawful,
        weapon: Some(SmolStr::new("cannon")),
        detail: RefusalDetail::Floor { permille: 100 },
    });
    let keyframe = status_frame(crate::arena_task::MatchId(1), &frame);
    let notices = keyframe["notices"].as_array().cloned().unwrap_or_default();
    assert_eq!(notices.len(), cases.len() + 1);
    let fired = &notices[0];
    assert_eq!(fired["kind"], "slot_fired");
    assert_eq!(fired["slot"], 1);
    assert_eq!(fired["weapon"], "cannon");
    assert_eq!(fired["designator"], "BA");
    assert_eq!(fired["percent"], 62);
    assert_eq!(fired["aspect"], "left");
    assert_eq!(fired["detail"], serde_json::Value::Null);
    assert_eq!(notices[1]["designator"], serde_json::Value::Null);
    let odds = &notices[2];
    assert_eq!(odds["kind"], "slot_held");
    assert_eq!(odds["slot"], 1);
    assert_eq!(odds["weapon"], "cannon");
    assert_eq!(
        odds["detail"],
        serde_json::json!({
            "why": "odds", "percent": 54, "floor": 60, "aspect": null, "range_m": null,
            "min_m": null, "max_m": null, "rounds": null, "keeps": null
        })
    );
    assert_eq!(notices[3]["detail"]["why"], "aspect");
    assert_eq!(notices[3]["detail"]["aspect"], "front");
    assert_eq!(notices[3]["designator"], "BA");
    assert_eq!(
        notices[4]["detail"],
        serde_json::json!({
            "why": "range", "percent": null, "floor": null, "aspect": null,
            "range_m": 1200, "min_m": 200, "max_m": 600, "rounds": null, "keeps": null
        })
    );
    assert_eq!(notices[7]["detail"]["why"], "rounds");
    assert_eq!(notices[7]["detail"]["keeps"], 8);
    let refused = &notices[cases.len()];
    assert_eq!(refused["refusal"], "unlawful");
    assert_eq!(
        refused["detail"],
        serde_json::json!({ "floor_permille": 100 })
    );
    assert!(
        refused.get("slot").is_none(),
        "a notice that is not a slot's carries no slot key"
    );
}
