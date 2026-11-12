//! **Doc 45 cut AP.1 — an absent defender is told** (Q-45-17), over the composed world
//! (`driving`'s keyed rig), the arena's egress pumped by hand as `field_fr2.rs` pumps it, at both
//! site sizes.
//!
//! Three parties on one site. **Away** puts a carrier down, drives a rig out and logs off.
//! **Pilot** opens a field with a fire order at that rig and fights it down. **Third** sits in a
//! hull of its own on the same ground and fires nothing. **Nobody** has nothing on the site and
//! is offline too. The field settles, and:
//!
//! - Away, on logging in, reads under the reconnect heading that the field opened, that it
//!   settled, that the rig is lost and that the carrier is home, each once and in that order;
//! - Away has one letter, under the site's name, that names the lost rig;
//! - Nobody has nothing queued and no letter;
//! - Pilot and Third, who were connected throughout, heard the opening and the settling once
//!   each, not twice, and were sent no letter: they lost nothing.
//!
//! **What is a fixture.** The armed hulls are made and seated by a staff hand, and the rig is
//! placed and made fragile by one, as `field_host.rs`'s are. The mail is the seam the server
//! fills with `mush-mail` (`surface::claim::install_claim_mail`), filled here with a recorder:
//! this crate's test world composes no mail module. Everything else is the production field.

use std::sync::{Arc, Mutex};

use mush_core::{Audience, GOD, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena_task::ArenaOut;
use tacsim::field::notice::{AFTER_FIELD, Fate, LETTER_SUBJECT};
use tacsim::field::{FIELD_OPENED, FIELD_SETTLED};
use tacsim::port_tick::{NOTICE_HEADING, pending_notices};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    AHUNA, Rig, command, count, drain, land, launched_at, on_pad_as_at, pilot, said, stocked_tuned,
    to_cell_on, warm,
};
use crate::field_host_rig::{
    armed_on, egress, fragile, frame, seat, site_record_at, task, tune, weapon_of,
};

/// **One size the story runs at** (doc 43 rule 5): the site, its `+ship/undock` word, its name,
/// and three cells on one row: Third's hull, Pilot's hull four cells east of it facing east, and
/// Away's rig four cells east of that.
struct At {
    entry: &'static str,
    venue: &'static str,
    name: &'static str,
    third: (i64, i64),
    pilot: (i64, i64),
    rig: (i64, i64),
}

/// Ahuna Flank at 256 a side: GC.10's row.
const AHUNA_FLANK: At = At {
    entry: AHUNA,
    venue: "ahuna",
    name: "Ahuna Flank",
    third: (106, 124),
    pilot: (110, 124),
    rig: (114, 124),
};

/// Urvara Reach at 512 a side: row 268 across the 256 line (`field_story`'s fight).
const URVARA_REACH: At = At {
    entry: "urvara_port",
    venue: "urvara",
    name: "Urvara Reach",
    third: (246, 268),
    pilot: (250, 268),
    rig: (254, 268),
};

/// Everything the world said, to anyone, since the story began.
type Heard = Vec<(Audience, OutputEvent)>;
/// Every letter the mail seam was handed: its recipients, its subject, its body.
type Post = Arc<Mutex<Vec<(Vec<ObjId>, String, String)>>>;

/// Drop every connection of `who` and end their session (`excursions.rs`'s helper).
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

/// `field_host_rig::pump`, keeping what the world said while it staged the arena's messages:
/// the host's snapshot and close branches speak there.
fn pump(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    heard: &mut Heard,
) {
    let Some(tasks) = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return;
    };
    let mut staged = 0;
    while let Ok(message) = rx.try_recv() {
        if let ArenaOut::Ledger { chunk, .. } = &message
            && let Ok(text) = std::str::from_utf8(&chunk.bytes)
        {
            lines.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok()),
            );
        }
        if !tasks.stage(message) {
            break;
        }
        staged += 1;
    }
    if staged > 0 {
        heard.extend(rig.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        }));
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }
}

/// One world second, keeping what its ticks said (the admission speaks there), and every
/// submission they queued, landed.
fn second(rig: &mut Rig, heard: &mut Heard) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    heard.extend(rig.tw.fire_due(t.saturating_mul(1_000)));
    for submit in drain(&mut rig.tw) {
        let _ = land(rig, &submit);
    }
}

/// Pump and tick until `done` holds or `secs` of wall time pass; answers whether it held.
async fn until(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    heard: &mut Heard,
    secs: u64,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        pump(rig, rx, lines, heard);
        beat += 1;
        if beat.is_multiple_of(8) {
            second(rig, heard);
        }
        if done(rig, lines) {
            return true;
        }
    }
    false
}

/// The line that says what became of `units` (a fate's line of the close).
fn after(at: &At, fate: Fate, units: &str) -> String {
    AFTER_FIELD
        .replace("{n}", "1")
        .replace("{site}", at.name)
        .replace("{fate}", fate.word())
        .replace("{units}", units)
}

/// **The story at `at`** (this file's head).
#[allow(clippy::too_many_lines)] // one field, in order
async fn absent_gate(at: &At) {
    let mut rig = stocked_tuned(|cfg| {
        // `tune`'s quiet interval (15 s) is longer than a gun's cycle, so the field does not
        // settle between Pilot's shots.
        tune(cfg);
        // The black box's chunks close when full: summaries every pulse fill one in a few arena
        // seconds, so the line that says the rig fell reaches the story while the field runs.
        cfg.arena.ledger.summary_ms = 100;
    });
    let post: Post = Arc::default();
    {
        let post = Arc::clone(&post);
        assert!(
            tacsim::surface::claim::install_claim_mail(
                rig.tw.world_mut(),
                Arc::new(
                    move |_ctx: &mut mush_core::Ctx<'_>,
                          _from: ObjId,
                          to: &[ObjId],
                          subject: &str,
                          body: &str| {
                        post.lock().map_err(|error| error.to_string())?.push((
                            to.to_vec(),
                            subject.to_owned(),
                            body.to_owned(),
                        ));
                        Ok(())
                    },
                ),
            ),
            "the mail seam is composed"
        );
    }
    let site = (at.venue, at.entry);
    // Three parties on the ground, and one player with nothing there.
    let (a, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), site, 1);
    let (away, away_row, away_tvs) = on_pad_as_at(&mut rig, ("Away", 2, "Tally Moth"), site, 1);
    let (third, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), site, 1);
    let nobody = pilot(&mut rig, "Nobody", 4);
    let rig_out = away_tvs[0];
    launched_at(&mut rig, at.entry, away, rig_out);
    to_cell_on(&mut rig, at.entry, rig_out, at.rig);
    fragile(&mut rig, rig_out);
    let hull_a = armed_on(&mut rig, at.entry, "Red Hound", at.pilot, 90_000);
    let hull_c = armed_on(&mut rig, at.entry, "Grey Hound", at.third, 90_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_c, third);
    let name = |rig: &Rig, obj: ObjId| {
        rig.tw
            .world()
            .get(obj)
            .map(|object| object.name.clone())
            .expect("a named object")
    };
    let rig_name = name(&rig, rig_out);
    let carrier_name = name(&rig, ObjId(away_row.hull));

    // Away and Nobody log off before anything happens.
    disconnect(&mut rig.tw, away);
    disconnect(&mut rig.tw, nobody);
    let before = pending_notices(rig.tw.world(), away);

    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut heard: Heard = Vec::new();
    for _ in 0..3 {
        second(&mut rig, &mut heard);
    }

    // Pilot's fire order at the rig to the east opens the field.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, &mut heard, 15, |rig, _| {
            task(rig).is_some()
        })
        .await,
        "the field's task opens"
    );
    let opened = FIELD_OPENED.replace("{n}", "1").replace("{site}", at.name);
    assert!(
        until(&mut rig, &mut rx, &mut lines, &mut heard, 15, |rig, _| {
            pending_notices(rig.tw.world(), away).contains(&opened)
        })
        .await,
        "the opening is queued for Away, who has a rig and a carrier there: {:?}",
        pending_notices(rig.tw.world(), away)
    );

    // Pilot fights the rig down.
    let gun = weapon_of(&rig, hull_a);
    let cell = (
        u16::try_from(at.rig.0).expect("a cell"),
        u16::try_from(at.rig.1).expect("a cell"),
    );
    let contact = |rig: &Rig| {
        frame(rig, a).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(cell.0) <= 2 && y.abs_diff(cell.1) <= 2)
                })
                .cloned()
        })
    };
    let fell = until(
        &mut rig,
        &mut rx,
        &mut lines,
        &mut heard,
        120,
        |rig, lines| {
            if lines.iter().any(
                |line| matches!(line.kind, EventKind::UnitDestroyed { unit, .. } if unit == rig_out),
            ) {
                return true;
            }
            if let Some(contact) = contact(rig)
                && frame(rig, a).is_some_and(|frame| frame.lock != Some(contact.id))
            {
                let _ = rig.tw.run(a, &format!("LOCK {}", contact.designator));
            }
            let _ = rig.tw.run(a, &format!("FIRE {gun}"));
            false
        },
    )
    .await;
    assert!(
        fell,
        "Pilot's fire takes Away's rig: {} impacts on it, contact {:?}, frame {:?}",
        lines
            .iter()
            .filter(
                |line| matches!(line.kind, EventKind::Impact { target, .. } if target == rig_out)
            )
            .count(),
        contact(&rig).map(|contact| (contact.designator.to_string(), contact.cell)),
        frame(&rig, a).map(|frame| (
            frame.lock,
            frame
                .weapons
                .iter()
                .map(|row| row.rounds)
                .collect::<Vec<_>>()
        ))
    );
    let entry = at.entry;
    assert!(
        until(&mut rig, &mut rx, &mut lines, &mut heard, 90, |rig, _| {
            task(rig).is_none()
                && site_record_at(rig, entry).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes"
    );

    // What is queued for Away: the opening, the settling, and each unit by its fate, in order,
    // each once.
    let settled = FIELD_SETTLED.replace("{n}", "1").replace("{site}", at.name);
    let lost = after(at, Fate::Lost, &rig_name);
    let home = after(at, Fate::Home, &carrier_name);
    let pending: Vec<String> = pending_notices(rig.tw.world(), away)
        .into_iter()
        .filter(|line| !before.contains(line))
        .collect();
    let mine: Vec<&String> = pending
        .iter()
        .filter(|line| [&opened, &settled, &lost, &home].contains(line))
        .collect();
    assert_eq!(
        mine,
        vec![&opened, &settled, &lost, &home],
        "the opening, the settling, the lost rig and the carrier at home, once each: {pending:?}"
    );
    assert!(
        !pending
            .iter()
            .any(|line| line.contains("Red Hound") || line.contains("Grey Hound")),
        "nobody else's unit is in Away's notices: {pending:?}"
    );

    // One letter, Away's, naming the loss.
    let letters = post.lock().expect("the post").clone();
    let subject = LETTER_SUBJECT
        .replace("{n}", "1")
        .replace("{site}", at.name);
    let to_away: Vec<&(Vec<ObjId>, String, String)> = letters
        .iter()
        .filter(|(to, _, _)| to.contains(&away))
        .collect();
    assert_eq!(to_away.len(), 1, "one letter for the loss: {letters:?}");
    assert_eq!(to_away[0].1, subject);
    assert!(
        to_away[0].2.contains(&format!("{rig_name}: lost.")),
        "{}",
        to_away[0].2
    );
    assert!(
        to_away[0].2.contains(&format!("{carrier_name}: home.")),
        "{}",
        to_away[0].2
    );
    assert!(
        !letters
            .iter()
            .any(|(to, _, _)| to.contains(&a) || to.contains(&third) || to.contains(&nobody)),
        "nobody who lost nothing is written to: {letters:?}"
    );

    // A player with nothing on the site is told nothing.
    assert!(
        pending_notices(rig.tw.world(), nobody).is_empty(),
        "{:?}",
        pending_notices(rig.tw.world(), nobody)
    );

    // The connected parties heard the opening and the settling live, once each, and what became
    // of their own units.
    for (who, hull) in [(a, "Red Hound"), (third, "Grey Hound")] {
        let live = said(&rig.tw, &heard, who);
        assert_eq!(count(&live, &opened), 1, "the opening, once: {live}");
        assert_eq!(count(&live, &settled), 1, "the settling, once: {live}");
        assert_eq!(
            count(&live, &after(at, Fate::Holding, hull)),
            1,
            "its hull holds: {live}"
        );
        assert!(
            !live.contains(&lost),
            "another's loss is not its news: {live}"
        );
        assert!(
            pending_notices(rig.tw.world(), who).is_empty(),
            "said, not queued"
        );
    }

    // Away logs in and reads it, under the heading.
    let events = rig.tw.connect(away);
    let read = said(&rig.tw, &events, away);
    let rows: Vec<&str> = read.lines().collect();
    let row = |line: &str| {
        rows.iter()
            .position(|row| *row == line)
            .unwrap_or_else(|| panic!("Away reads {line:?} at login: {read}"))
    };
    let heading = row(NOTICE_HEADING);
    assert!(
        heading < row(&opened)
            && row(&opened) < row(&settled)
            && row(&settled) < row(&lost)
            && row(&lost) < row(&home),
        "in order, under the heading: {read}"
    );
    assert!(
        pending_notices(rig.tw.world(), away).is_empty(),
        "read once"
    );
    let events = rig.tw.connect(nobody);
    let read = said(&rig.tw, &events, nobody);
    assert!(
        !read.contains(NOTICE_HEADING) && !read.contains("ield 1"),
        "Nobody reads nothing of it: {read}"
    );
}

/// **AP.1 at 256**: an offline player reads at login that a field opened on their site and what
/// became of each of their units, and has a letter for the loss.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap1_ahuna_an_absent_defender_reads_the_field_and_its_losses_at_login() {
    absent_gate(&AHUNA_FLANK).await;
}

/// **AP.1 at 512**: the same, on Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap1_urvara_an_absent_defender_reads_the_field_and_its_losses_at_login() {
    absent_gate(&URVARA_REACH).await;
}
