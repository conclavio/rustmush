//! **Doc 17h cut M8g G4 — the leak battery** (doc 17h §3 M8g "The leak tests"; doc 17 D-UM-42;
//! Q-M8-2). An ungoverned core on a rig (M8b) sealing an unfiled draft aboard (M8e's
//! `+tv/<n> launch mind=draft:<name>`) is learned by **nobody** — a bystander, another operator,
//! a pilot, staff's non-Registry reads, the web — and only through the built channels:
//!
//! | channel | reader | proof here |
//! |---|---|---|
//! | the owner's box read, `+hull/box` (M8a) | the hull's owner (and staff, M8a's readers) | positive control, then every absence |
//! | the Registry's inspection, `@registry/inspect` (M8a) | the Registry's operator | positive control, then every absence |
//! | the attestation refusal (M8c) | the licence's holder, at renewal | the absence only: an unfiled draft has no licence, so its flight writes no seal mark; the positive control needs a filed, rated, licensed mind on a port world (G1's combined world) and is owed there |
//! | the claim's inspection (`@mind/claim`, G3) | the Registry's operator | owed: the verb is not on main; [`g4_the_claim_channel_is_owed_until_mind_claim_lands`] trips the day it lands |
//! | testimony | — | owed to the fight (Q-M8-2, H1) |
//!
//! `sim_lenses/observer_rule.rs`'s pattern, here rather than in the mushd lens harness for the
//! reason M8a's box test gives (`ic_sortie_surface.rs`, `m8a_the_owner_reads_the_box_and_nobody_else`):
//! that harness has no Dray-P on a site's pad with a leased rig, and the stock mushd roster
//! composes no mind commerce, so no Registry. This world is M8e's cored-rig excursion harness
//! (`excursions::stocked_fitted`) under the Active commerce with the Registry inspection composed
//! (`reboot_with_inspection`), the composition root's own modules. So:
//!
//! - **Five witnesses**, each asked the same questions **twice** — once before anything is fitted
//!   or sealed (the control: every question is a live verb that answers), and once after the
//!   draft is sealed and the Registry has inspected the rig, and again after a restart.
//! - **`ask_whole` + a sentinel**: every question is followed by a line the parser refuses, and
//!   the answer must end there — nothing of the question's arrives after it; a read submits
//!   nothing.
//! - **Every event of the act**, not only the answers: the launch, its finish, the inspection
//!   and the chronicle's tick are each captured whole and read as each witness would hear them,
//!   so a notice or a broadcast is caught as surely as an answer.
//! - **The web** is asked through the world's own named queries — the bytes the gateway forwards
//!   to a browser (`mind_market_web.rs` and `observer_rule.rs` prove the transport) — as staff
//!   and as the bystander.
//! - **[`no_sixteen_digit_word`]** on everything any witness is told.
//!
//! What a witness may not learn: that the core is ungoverned (`ungoverned`, the root's id), the
//! draft's name, the admissions (`unfiled`, `unlicensed`), that the seal is a draft's, and the
//! seal's digest. The owner-approved public lines of M8d (the canon line, which names nobody, and
//! the Red Dies clock) are the world's one signal and are scrubbed before the check — the line
//! says an inspection found an ungoverned core somewhere, never where or whose.
//!
//! **Dispatch-only** (G5): every test here is `#[ignore]`d and run by CI's dispatch (the
//! `m8-leak-battery` job, `--ignored`); never locally.

use mush_core::{Audience, Effects, GOD, Inbound, ObjId, Outbox};
use mush_proto::OutputEvent;
use serde_json::{Value, json};

use crate::excursions::{
    Rig, advance_to, command, drain, due_of, fire, inspections, land_all, land_receipt,
    no_refused_writes, on_pad, on_pad_as, reboot_with, reboot_with_inspection, record_of, teleport,
};
use crate::ic_sortie_surface::{announce, fit, hull_name, save_draft};

/// The draft the operator seals aboard; its canonical name is the one a leak would print.
const DRAFT: &str = "Zzqleak";
/// The Roll's canonical spelling of [`DRAFT`].
const DRAFT_CANONICAL: &str = "zzqleak";
/// A line the parser refuses: the end of every whole answer.
const SENTINEL: &str = "zzq-leak-sentinel";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";
/// The driver's uniform refusal of a Registry verb to a non-operator (M8a, A7).
const NOT_THROUGH: &str = "That did not go through. Nothing changed.";
/// **Staff's reads outside D-UM-42's witness set, by name** (the lead's ruling of 2026-10-29 on
/// the battery's first run, option (b); recorded in doc 17h, the owner may overrule it): the
/// `@unit/*` verbs are raw staff builder and debug tools, like reading the store, so `@unit/show`
/// naming the bound root's id (`…_ungoverned`) is no leak. They are still asked (the control
/// holds them to answering); only their answers are not checked. Staff's non-Registry in-world
/// reads stay checked — `look`, `examine`, `+hull/box` beyond the box's readers, the port verbs,
/// `@tacsim/port status`, the web.
const STAFF_EXEMPT: [&str; 1] = ["@unit/"];
/// The `@mind` family's answer to a switch it does not have: `@mind/claim` today (G3 adds it).
const UNKNOWN_CLAIM: &str = "Unknown switch \"/claim\" for @mind.";
/// The connection the web witness's queries arrive on.
const WEB_CONN: u64 = 7_000;

/// The owner-approved public lines of doc 17h cut M8d (D6, D7): the canon line, which names
/// nobody, the Red Dies clock's title and its consequence line
/// (`mush_faction::seed_red_dies_front`). They say an ungoverned core was found somewhere and
/// are the world's one signal; a witness may read them, so they are scrubbed before the check.
const PUBLIC_LINES: [&str; 3] = [
    tacsim::minds::registry_inspection::unwarranted::LINE,
    "The Red Dies: the Registry has found ungoverned cores enough to move the Republic.",
    "Ungoverned cores found",
];

/// One witness: who, the seat they hold, and whether they are asked as staff.
struct Witness {
    who: ObjId,
    seat: &'static str,
    staff: bool,
}

/// **Every word that would betray the sealed draft or the ungoverned core** to `seen`, after
/// the public lines are scrubbed; `digest8` is the seal's printed prefix. Each leak is noted in
/// `leaks` (the battery asks every question and fails once, naming them all).
fn learns_nothing(leaks: &mut Vec<String>, seen: &str, digest8: &str, what: &str) {
    let mut scrubbed = seen.to_owned();
    for line in PUBLIC_LINES {
        scrubbed = scrubbed.replace(line, "");
    }
    let lower = scrubbed.to_lowercase();
    for leak in [
        "ungoverned",
        DRAFT_CANONICAL,
        "unfiled",
        "unlicensed",
        "admission",
        "licence draft",
        "sortie",
        &digest8.to_lowercase(),
    ] {
        if lower.contains(leak) {
            leaks.push(format!("{what} learned {leak:?}:\n{seen}"));
        }
    }
    no_sixteen_digit_word(leaks, seen, what);
}

/// `observer_rule.rs`' check: no word of sixteen hex digits (a raw id or a digest) reaches a
/// surface (doc 17h's common rule: a digest is shown as its first eight characters or not at
/// all). A word right after a `.` is a decimal fraction (a panel's coordinates), not an id.
fn no_sixteen_digit_word(leaks: &mut Vec<String>, seen: &str, what: &str) {
    // Hex digits are ASCII, so a byte scan never splits a character.
    let bytes = seen.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_hexdigit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
            i += 1;
        }
        let fraction = start > 0 && bytes[start - 1] == b'.';
        if !fraction && i - start == 16 {
            leaks.push(format!(
                "{what} carried a sixteen-digit word ({}):\n{seen}",
                &seen[start..i]
            ));
        }
    }
}

/// Whether a labelled answer is not held to the leak check: staff's `+hull/box`, the box's own
/// reader (M8a; its positive control is step 3), and [`STAFF_EXEMPT`]'s verbs.
fn exempt(what: &str) -> bool {
    what.starts_with("staff")
        && (what.contains("`+hull/box")
            || STAFF_EXEMPT
                .iter()
                .any(|verb| what.contains(&format!("`{verb}"))))
}

/// **What `who` hears of `events`**: the text the harness's fan-out gives them, and every event
/// addressed to them or to everyone whole (a pushed panel's data included).
fn heard(rig: &Rig, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    let mut seen = rig.tw.seen_by(events, who).join("\n");
    for (audience, event) in events {
        if matches!(audience, Audience::Actor(a) if *a == who) || matches!(audience, Audience::All)
        {
            seen.push('\n');
            seen.push_str(&format!("{event:?}"));
        }
    }
    seen
}

/// **`line` as `who`, answered whole** (`observer_rule.rs`' `ask_whole`): the line, then the
/// sentinel, which must answer `Huh?` and nothing else; a read submits nothing. The answer.
fn ask_whole(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (answer, queued) = command(rig, who, line);
    assert!(
        queued.is_empty(),
        "{line} by #{} submitted: {answer}",
        who.0
    );
    let (end, queued) = command(rig, who, SENTINEL);
    assert!(
        end.trim_start().starts_with(HUH) && queued.is_empty(),
        "the sentinel after {line} answered {end:?}"
    );
    answer
}

/// **`name` as `who` over the web**: the world's own answer to a named query (the reply the
/// gateway forwards to the browser), whole.
fn ask_web(rig: &mut Rig, who: ObjId, name: &str, args: Value) -> String {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    rig.tw.engine_mut().step(
        Inbound::Query {
            conn: WEB_CONN,
            actor: who,
            id: 1,
            name: name.into(),
            args,
        },
        &mut out,
        &mut fx,
    );
    let events = out.take();
    assert!(
        !events.is_empty(),
        "the world answers the {name} query (#{})",
        who.0
    );
    format!("{events:?}")
}

/// Every line-mode question a witness asks about the rig `tv`; staff add their own reads.
fn questions(tv: ObjId, staff: bool) -> Vec<String> {
    let n = tv.0;
    let mut lines = vec![
        "look".to_owned(),
        format!("examine #{n}"),
        "+tv".to_owned(),
        "+ship".to_owned(),
        "+chart".to_owned(),
        "+claim/list".to_owned(),
        "+minds".to_owned(),
        "+minds/holdings".to_owned(),
        "+chronicle".to_owned(),
        format!("+hull/box #{n}"),
        format!("@registry/inspect #{n}"),
    ];
    if staff {
        lines.extend([
            format!("@unit/show #{n}"),
            format!("@unit/status #{n}"),
            "@tacsim/port status".to_owned(),
        ]);
    }
    lines
}

/// Every named query the web asks: the port's panels, the arena's, the minds' and the
/// market's (each registered by the tacsim layer on every launch format).
fn web_queries() -> Vec<(&'static str, Value)> {
    vec![
        (tacsim::port::site_map::SITE_MAP_FEED, json!({})),
        (
            tacsim::port::site_map::SITE_MAP_FEED,
            json!({"view": "site"}),
        ),
        (tacsim::port::chart_panel::SKY_CHART_FEED, json!({})),
        (tacsim::port::ground_chart::BODY_GROUND_FEED, json!({})),
        (tacsim::arena_bridge::FRAME_FEED, json!({})),
        (tacsim::arena_bridge::MAP_FEED, json!({})),
        (tacsim::arena_bridge::OBSERVER_FEED, json!({})),
        (tacsim::arena_bridge::REPLAY_FEED, json!({})),
        // `minds::trace_feed::FEED`, `minds::bulk::query::FEED`, `bench_service::FEED`.
        ("arena.mind", json!({})),
        ("mind.bulk", json!({})),
        ("mind.bench", json!({})),
        // `minds::market_commands::query::{MARKET_FEED, HOLDINGS_FEED}`.
        ("mind.market", json!({})),
        ("mind.holdings", json!({})),
    ]
}

/// **Every witness asks every question** (line mode, then the web for staff and the
/// bystander); each answer, labelled. `control` holds each answer to "a live verb answered".
fn ask_everything(
    rig: &mut Rig,
    witnesses: &[Witness],
    tv: ObjId,
    web: &[ObjId],
) -> Vec<(String, String)> {
    let mut answers = Vec::new();
    for w in witnesses {
        for line in questions(tv, w.staff) {
            let answer = ask_whole(rig, w.who, &line);
            answers.push((format!("{} ({}) `{line}`", w.seat, w.who.0), answer));
        }
    }
    for &who in web {
        for (name, args) in web_queries() {
            let answer = ask_web(rig, who, name, args.clone());
            answers.push((format!("the web as #{} `{name}` {args}", who.0), answer));
        }
    }
    answers
}

/// **The battery** (doc 17h §3 M8g G4; D-UM-42). See the module's doc for the table.
///
/// The world: the rig's owner and operator (Pilot, on Ahuna Flank's pad with one leased rig);
/// the Registry's one operator; **another operator** on the same site with their own rig out on
/// the ground; **a pilot** whose hull stands on the same pad, with no rig; **a bystander**
/// standing beside the owner; **staff** (GOD) with every non-Registry read; and **the web**,
/// asked as staff and as the bystander.
///
/// 1. The control: every witness asks every question before anything is fitted, and each is a
///    live verb that answers (never `Huh?`).
/// 2. The crime: staff bind the rig to the ungoverned Class I root (the refit's stand-in, as
///    M8e's tests do); the owner saves the draft and launches it aboard; the act's every event
///    is read as each witness hears it — nobody hears anything of it.
/// 3. **The box read**: its positive control first — the owner's `+hull/box` prints the sealed
///    draft, its class, `ungoverned`, `licence draft` and the seal's prefix, and staff (M8a's
///    readers) read the same; then every absence — the other witnesses are told `NO_BOX` and
///    nothing else.
/// 4. **The inspection**: its positive control first — the operator's receipt reads the core
///    ungoverned and the open trip's seal matching, and the record holds `governed: false` and
///    the draft state; then every absence — every other witness, staff included, is told the
///    driver's one sentence, nothing queued, and nobody else hears the receipt.
/// 5. **The finding's facts**: the chronicle commits the one public canon line, which names
///    nobody; `minds.unwarranted` for the rig answers nothing to a non-Registry E4 reader (the
///    `minds` provider is composed: the control).
/// 6. **The attestation channel's absence**: the draft's seal carries no licence, so no seal
///    mark is written and no attestation can be refused over this flight.
/// 7. Every witness asks every question again — and learns nothing; again after a restart, and
///    the owner's box reads the same.
#[test]
#[ignore = "dispatch-only (doc 17h M8g G5): the CI `m8-leak-battery` job runs it"]
#[allow(clippy::too_many_lines)]
fn g4_an_ungoverned_core_sealing_a_draft_is_learned_through_the_built_channels_only() {
    use tacsim::definitions::core_replacement::RIG_I_UNGOVERNED;
    use tacsim::minds::licence_state::SealLicenceState;
    use tacsim::minds::registry_inspection::{InspectionHost, unwarranted};

    let mut rig = crate::excursions::stocked_fitted();
    let registry = rig.tw.spawn_player("Registry operator");
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let n = tv.0;
    let (mut rig, policy) = reboot_with_inspection(rig, registry, &[registry, pilot]);
    assert_eq!(policy.operators(), [registry]);

    // The witnesses. Another operator: their own hull and rig, the rig out on the ground.
    let (other, _, other_tvs) = on_pad_as(&mut rig, "Other operator", 2, "Tally Moth", 1);
    let _ = crate::excursions::act_go(&mut rig, other, "+tv/1 launch go");
    let _ = crate::excursions::finish(&mut rig, other_tvs[0], other);
    // A pilot: their hull on the same pad, no rig.
    let (second, _, none) = on_pad_as(&mut rig, "Second pilot", 3, "Ember Moth", 0);
    assert!(none.is_empty());
    // A bystander, standing where the owner stands.
    let bystander = rig.tw.spawn_player("Bystander");
    let beside = rig
        .tw
        .world()
        .get(pilot)
        .and_then(|o| o.location)
        .expect("the owner stands somewhere");
    teleport(&mut rig.tw, bystander, beside);
    let witnesses = [
        Witness {
            who: bystander,
            seat: "the bystander",
            staff: false,
        },
        Witness {
            who: other,
            seat: "the other operator",
            staff: false,
        },
        Witness {
            who: second,
            seat: "a pilot",
            staff: false,
        },
        Witness {
            who: GOD,
            seat: "staff",
            staff: true,
        },
    ];
    let web = [GOD, bystander];

    // 1. The control: every question is a live verb that answers.
    for (what, answer) in ask_everything(&mut rig, &witnesses, tv, &web) {
        assert!(
            !answer.trim().is_empty() && !answer.trim_start().starts_with(HUH),
            "{what} is no live question: {answer:?}"
        );
    }

    // 2. The crime, every event read as each witness hears it.
    fit(&mut rig, tv, RIG_I_UNGOVERNED);
    save_draft(&mut rig, pilot, DRAFT);
    let mut act = rig
        .tw
        .run(pilot, &format!("+tv/1 launch mind=draft:{DRAFT} go"));
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one start");
    land_all(&mut rig, &queued);
    let out_ic = due_of(&mut rig.tw, tv);
    let (events, queued) = advance_to(&mut rig, out_ic);
    act.extend(events);
    land_all(&mut rig, &queued);
    let (events, queued) = fire(&mut rig);
    act.extend(events);
    land_all(&mut rig, &queued);
    let open = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .expect("the trip is open");
    let facts = open.seal.clone().expect("sealed");
    assert_eq!(facts.mind.as_str(), DRAFT_CANONICAL);
    assert!(!facts.governed);
    assert_eq!(facts.licence_state, Some(SealLicenceState::Draft));
    let digest8 = facts.seal_digest[..8].to_owned();

    // 3. The box read. The positive control first: the owner reads the sealed draft whole.
    let open_line = format!(
        "{} out · surface from the bay of {}\n  {DRAFT_CANONICAL} · Class I · ungoverned · licence \
         draft · seal {digest8}",
        tacsim::port::licence::ic_stamp(out_ic),
        hull_name(&rig, ObjId(row.hull)),
    );
    assert_eq!(
        ask_whole(&mut rig, pilot, &format!("+hull/box #{n}")),
        open_line
    );
    // Staff are the box's readers too (M8a, doc 17h §2.5): the same lines.
    assert_eq!(
        ask_whole(&mut rig, GOD, &format!("+hull/box #{n}")),
        open_line
    );
    // Then every absence: one sentence, nothing else.
    for w in witnesses.iter().filter(|w| !w.staff) {
        assert_eq!(
            ask_whole(&mut rig, w.who, &format!("+hull/box #{n}")),
            tacsim::surface::hull::NO_BOX,
            "{}",
            w.seat
        );
    }

    // 4. The inspection. The positive control first: the operator's receipt.
    let mut inspected = rig.tw.run(registry, &format!("@registry/inspect #{n}"));
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one inspection");
    let receipt = land_receipt(&mut rig, &queued[0])
        .expect("the inspection lands")
        .expect("a receipt");
    assert!(
        receipt.starts_with(&format!("Hull #{n}: Class I ungoverned · provenance "))
            && receipt.ends_with(&format!(" · seal {digest8} matches")),
        "{receipt}"
    );
    let records = inspections(&rig.tw);
    let record = records.last().expect("the record");
    assert_eq!(record.governed, Some(false));
    assert_eq!(record.licence_state, Some(SealLicenceState::Draft));
    let seq = record.seq;
    // The world task announces the landed slot; this harness installs without announcing.
    let family = InspectionHost::of(rig.tw.world())
        .expect("the inspection is composed")
        .writer()
        .family()
        .expect("the record family");
    announce(
        &mut rig,
        &family,
        vec![mush_roll::commerce::registry_inspection::inspection_key(
            tv, seq,
        )],
    );
    let (events, queued) = fire(&mut rig);
    inspected.extend(events);
    land_all(&mut rig, &queued);
    // Then every absence: every other witness, staff included, is refused before any hull is
    // looked at, and nothing is queued.
    for w in &witnesses {
        let (out, queued) = command(&mut rig, w.who, &format!("@registry/inspect #{n}"));
        assert_eq!(out, NOT_THROUGH, "{}", w.seat);
        assert!(queued.is_empty(), "{}", w.seat);
    }
    // Nobody but the actors heard anything of the act or the inspection.
    let mut leaks = Vec::new();
    for w in &witnesses {
        learns_nothing(
            &mut leaks,
            &heard(&rig, &act, w.who),
            &digest8,
            &format!("{} during the act", w.seat),
        );
        learns_nothing(
            &mut leaks,
            &heard(&rig, &inspected, w.who),
            &digest8,
            &format!("{} during the inspection", w.seat),
        );
    }

    // 5. The finding's facts. The chronicle's one canon line names nobody.
    let canon: Vec<String> = rig
        .tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|state| state.recent())
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == unwarranted::KIND)
        .map(|event| event.summary.clone())
        .collect();
    assert_eq!(
        canon,
        [unwarranted::LINE.to_owned()],
        "one line, naming nobody"
    );
    // `minds.unwarranted` is the Registry's (K6): a non-Registry E4 reader learns nothing. The
    // unscoped bridge (`mush_economy::WorldFacts::state`) refuses a reference its provider
    // scopes before it resolves anything, and the provider's public `seized` spelling passes the
    // same gate (the control), so the silence is the scope's, wherever the provider is
    // composed. This world composes none (`reboot_with_inspection` composes nothing past the
    // inspection), so no E4 reader here reaches `minds` at all; G1's combined world composes
    // it. (The resolve itself answers nothing for `unwarranted`: mush-roll's
    // `facts::m8d_tests::m8d_unwarranted_resolves_for_no_public_condition`.)
    let minds = mush_roll::commerce::facts::MindsFactProvider::new();
    let fact = |kind: &str| mush_economy::ExternalRef {
        provider: "minds".into(),
        kind: kind.into(),
        id: format!("hull/{n:016x}").into(),
        revision: Some(seq),
    };
    assert!(
        !mush_economy::FactProvider::requires_scope(&minds, &fact("seized")),
        "the control: a public spelling passes the gate"
    );
    assert!(
        mush_economy::FactProvider::requires_scope(&minds, &fact("unwarranted")),
        "scoped to the Registry"
    );

    // 6. The attestation channel's absence: the draft's seal carries no licence, so the finish
    // wrote no seal mark (M8e: a mark is written only for a licence that flew) and no renewal
    // can be refused over this flight. Its positive control is owed to G1's combined world.
    let sealed = record_of(&mut rig.tw, tv)
        .and_then(|record| record.sortie)
        .and_then(|open| open.seal)
        .expect("sealed");
    assert_eq!(sealed.holder, None, "a draft has no holder");

    // 7. Every witness asks everything again, and learns nothing; and after a restart.
    for (what, answer) in ask_everything(&mut rig, &witnesses, tv, &web) {
        if exempt(&what) {
            continue;
        }
        learns_nothing(&mut leaks, &answer, &digest8, &what);
    }
    let mut rig = reboot_with(rig, &[pilot, registry, other, second, bystander]);
    assert_eq!(
        ask_whole(&mut rig, pilot, &format!("+hull/box #{n}")),
        open_line,
        "the box reads the same after a restart"
    );
    for (what, answer) in ask_everything(&mut rig, &witnesses, tv, &web) {
        if exempt(&what) {
            continue;
        }
        learns_nothing(
            &mut leaks,
            &answer,
            &digest8,
            &format!("{what} after a restart"),
        );
    }
    assert!(
        leaks.is_empty(),
        "{} leak(s):\n\n{}",
        leaks.len(),
        leaks.join("\n\n")
    );
    no_refused_writes(&rig.tw);
}

/// **The claim channel is owed** (doc 17h §3 M8g G3; D-UM-55): `@mind/claim` is not on main, so
/// the claim's inspection — the fourth built channel — has no case here yet. This trips the day
/// the verb lands (today the `@mind` family answers its unknown-switch line): replace it with the
/// channel's case (the Registry's inspection of a claimed hull, its positive control first, then
/// every absence, in the battery's world).
#[test]
#[ignore = "dispatch-only (doc 17h M8g G5): the CI `m8-leak-battery` job runs it"]
fn g4_the_claim_channel_is_owed_until_mind_claim_lands() {
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, _, tvs) = on_pad(&mut rig, 1);
    let (out, queued) = command(
        &mut rig,
        pilot,
        &format!("@mind/claim #{} saw it", tvs[0].0),
    );
    assert!(
        out.trim() == UNKNOWN_CLAIM && queued.is_empty(),
        "`@mind/claim` has landed: write the claim channel's case here (G4): {out}"
    );
}
