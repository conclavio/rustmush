//! **The testimony producer** (doc 23 cut S.4b-T; §17.8, D-23-19, Q-23-16, Q-23-17).
//!
//! A field contact is earned when the author's own hull told the target apart by its own sensors
//! in an IC field. The bridge's `on_chunk` hands every chunk of a field's own log
//! (`field.<site>.<ordinal>.<segment>`) here as it lands, parsed once for every reader
//! ([`note_chunk`]), so a void loses at most the unshipped tail.
//!
//! - **Provenance.** Only a `Classified` line carrying its own
//!   `classify:<viewer>:<target>:<pass_seq>` draw counts ([`own_sensors`]): a classification that
//!   arrived over the comms mesh carries no draw (the sharer's pass recorded one) and earns the
//!   recipient nothing, and a `Detected` blip is not "saw".
//! - **The author.** The ledger records no seat change, so the author is the pilot seated at
//!   admission (`FieldUnit::pilot`), else the unit's party leader — the seal's warrant principal
//!   (`admit::custody_unit` seals under the entrant's leader; D-23-5). A pilot who boards
//!   mid-field earns from the next admission on.
//! - **Persistence.** Each author whose ring changed gets its whole
//!   [`crate::minds::earned_contacts::EarnedRecord`] written in the chunk's bundle; the boot
//!   reads them back (`crate::minds::earned_contacts::read`).

use std::collections::BTreeSet;

use mush_core::{Ctx, ObjId};

use super::FieldBook;
use super::record::FieldLive;
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena_task::MatchId;
use crate::minds::earned_contacts::{EarnedContacts, earned_key};
use crate::store::push_op;

/// **Whether `event` is a classification the viewer's own sensors made**: a `Classified` line
/// whose draws carry its own `classify:<viewer>:<target>:` label. The viewer and the target.
#[must_use]
pub(crate) fn own_sensors(event: &LedgerEvent) -> Option<(ObjId, ObjId)> {
    let EventKind::Classified { viewer, target, .. } = &event.kind else {
        return None;
    };
    let label = format!("classify:{}:{}:", viewer.0, target.0);
    event
        .draws
        .iter()
        .any(|(drawn, _)| drawn.starts_with(&label))
        .then_some((*viewer, *target))
}

/// The author of what `unit` saw: its admission pilot, else its party's leader.
fn author_of(live: &FieldLive, unit: ObjId) -> Option<ObjId> {
    let row = live.units.iter().find(|row| row.unit == unit)?;
    row.pilot.as_ref().map(|(pilot, _)| *pilot).or_else(|| {
        live.parties
            .iter()
            .find(|party| party.party == row.party)
            .and_then(|party| party.leader)
    })
}

/// **A chunk of a field's log passed** (the bridge's ledger branch): admit every own-sensor
/// classification to its author's ring, and write each ring that changed. Any other task's
/// chunk is not a field's and is ignored.
pub(crate) fn note_chunk(ctx: &mut Ctx<'_>, task: MatchId, events: &[LedgerEvent]) {
    let Some(earned) = ctx.w.module_state::<FieldBook>().and_then(|book| {
        let live = &book.hosted(book.site_of_task(task)?)?.live;
        Some(
            events
                .iter()
                .filter_map(own_sensors)
                .filter_map(|(viewer, target)| Some((author_of(live, viewer)?, target)))
                .collect::<Vec<_>>(),
        )
    }) else {
        return;
    };
    if earned.is_empty() {
        return;
    }
    let Some(index) = ctx.w.module_state_mut::<EarnedContacts>() else {
        return;
    };
    let mut changed = BTreeSet::new();
    for (author, target) in earned {
        if index.admit(author, target) {
            changed.insert(author);
        }
    }
    let records: Vec<_> = changed
        .into_iter()
        .map(|author| index.record(author))
        .collect();
    for record in records {
        match serde_json::to_value(&record) {
            Ok(value) => push_op(
                ctx.out,
                mush_core::ModuleWrite::JsonSet {
                    key: earned_key(record.author),
                    value,
                },
            ),
            Err(error) => {
                tracing::error!(%error, author = record.author.0, "tacsim: an earned-contacts ring did not encode");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::PartyId;
    use crate::arena::state::ContactId;

    fn classified(viewer: u32, target: u32, draws: Vec<(String, u64)>) -> LedgerEvent {
        LedgerEvent {
            seq: 1,
            t_ms: 0,
            kind: EventKind::Classified {
                viewer: ObjId(viewer),
                contact: ContactId(1),
                target: ObjId(target),
                team: PartyId::nth(1).expect("B"),
            },
            draws,
            hash: 0,
        }
    }

    /// **D-23-19's provenance**: the viewer's own pass carries its draw; a mesh share carries
    /// none, a draw for another pair is not this line's, and a detection is never a
    /// classification.
    #[test]
    fn s4bt_only_an_own_sensor_classification_is_testimony() {
        let own = classified(4, 9, vec![("classify:4:9:3".to_owned(), 17)]);
        assert_eq!(own_sensors(&own), Some((ObjId(4), ObjId(9))));
        assert_eq!(
            own_sensors(&classified(4, 9, Vec::new())),
            None,
            "a mesh share"
        );
        assert_eq!(
            own_sensors(&classified(4, 9, vec![("classify:5:9:3".to_owned(), 17)])),
            None,
            "another viewer's draw"
        );
        assert_eq!(
            own_sensors(&classified(4, 9, vec![("classify:4:90:3".to_owned(), 17)])),
            None,
            "another target's draw"
        );
        let detected = LedgerEvent {
            kind: EventKind::Detected {
                viewer: ObjId(4),
                contact: ContactId(1),
                target: ObjId(9),
            },
            ..own
        };
        assert_eq!(own_sensors(&detected), None, "a blip is not testimony");
    }

    /// A field on Ahuna Flank: party A's unit 7 with pilot 4 at admission and unit 8 with
    /// none (party A led by 3), party B's raider 9 (nobody leads it), party C's unit 10 with
    /// pilot 5.
    fn live() -> FieldLive {
        use super::super::record::{FIELD_LIVE_FORMAT, FieldParty, FieldUnit, PartyKey};
        use crate::surface::site_record::FieldId;
        let party = |n: usize| PartyId::nth(n).expect("a letter");
        let unit = |unit: u32, party: PartyId, pilot: Option<u32>| FieldUnit {
            unit: ObjId(unit),
            party,
            pilot: pilot.map(|pilot| (ObjId(pilot), smol_str::SmolStr::new("cockpit"))),
            epoch: None,
            aboard: Vec::new(),
            arrived: false,
        };
        let led = |n: usize, by: Option<u32>, key: u32| FieldParty {
            party: party(n),
            key: PartyKey::Unit { unit: ObjId(key) },
            leader: by.map(ObjId),
        };
        FieldLive {
            format: FIELD_LIVE_FORMAT,
            field: FieldId {
                site: mush_core::CanonicalId::new("ahuna_port").expect("canonical"),
                ordinal: 1,
            },
            opened_ic: 0,
            segment: 1,
            at_ms: 0,
            parties: vec![led(0, Some(3), 7), led(1, None, 9), led(2, Some(5), 10)],
            units: vec![
                unit(7, party(0), Some(4)),
                unit(8, party(0), None),
                unit(9, party(1), None),
                unit(10, party(2), Some(5)),
            ],
            homes: Vec::new(),
            fire: None,
            owed_exits: Vec::new(),
            held: Vec::new(),
        }
    }

    /// **D-23-19's author** (Q-23-16): the admission pilot, else the party's leader (the seal's
    /// warrant principal); a unit of a party nobody leads, with no pilot, earns for nobody.
    #[test]
    fn s4bt_the_author_is_the_admission_pilot_else_the_party_leader() {
        let live = live();
        assert_eq!(
            author_of(&live, ObjId(7)),
            Some(ObjId(4)),
            "the seated pilot"
        );
        assert_eq!(
            author_of(&live, ObjId(8)),
            Some(ObjId(3)),
            "the warrant principal"
        );
        assert_eq!(author_of(&live, ObjId(9)), None, "nobody leads it");
        assert_eq!(author_of(&live, ObjId(11)), None, "not in the field");
    }

    fn ctx<'a>(engine: &'a mut mush_core::Engine, out: &'a mut mush_core::Outbox) -> Ctx<'a> {
        Ctx {
            w: &mut engine.world,
            actor: mush_core::GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(crate::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        }
    }

    fn world() -> mush_testkit::TestWorld {
        mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::with_config(
            crate::TacsimConfig {
                enabled: true,
                ..crate::TacsimConfig::default()
            },
        ))])
    }

    /// **S.4b-T's producer over a field's chunk** (§17.10's test, its negative half): the rig
    /// that told the raider apart earns it for its pilot, in one ring written in the chunk's
    /// bundle; its party-mate that held the raider only over the mesh earns nothing for its
    /// leader, a rig that only detected it earns nothing for its pilot, and a bystander nothing.
    /// Another task's chunk is not a field's. The ring reads back at boot.
    #[test]
    fn s4bt_a_field_chunk_earns_only_its_own_sensor_classifications() {
        let task = MatchId(u64::MAX - 1);
        let events = vec![
            classified(7, 9, vec![("classify:7:9:2".to_owned(), 11)]),
            classified(8, 9, Vec::new()),
            LedgerEvent {
                kind: EventKind::Detected {
                    viewer: ObjId(10),
                    contact: ContactId(2),
                    target: ObjId(9),
                },
                ..classified(10, 9, Vec::new())
            },
            // A repeat in the same chunk writes the ring once.
            classified(7, 9, vec![("classify:7:9:5".to_owned(), 12)]),
        ];
        let mut tw = world();
        let mut out = mush_core::Outbox::new();
        // Not a field's task yet: nothing is earned.
        note_chunk(&mut ctx(tw.engine_mut(), &mut out), task, &events);
        assert!(!crate::minds::earned_contacts::has_earned(
            tw.world(),
            ObjId(4),
            ObjId(9)
        ));
        if let Some(book) = tw.world_mut().module_state_mut::<FieldBook>() {
            let live = live();
            book.live
                .insert(live.field.site.clone(), super::super::Hosted { task, live });
        }
        note_chunk(&mut ctx(tw.engine_mut(), &mut out), task, &events);
        let has = |tw: &mush_testkit::TestWorld, who: u32| {
            crate::minds::earned_contacts::has_earned(tw.world(), ObjId(who), ObjId(9))
        };
        assert!(has(&tw, 4), "the rig's pilot earned the raider");
        assert!(!has(&tw, 3), "the blind radio recipient's leader did not");
        assert!(!has(&tw, 5), "the rig that only detected it did not");
        assert!(!has(&tw, 6), "the bystander did not");
        let writes: Vec<mush_core::ModuleOp> = out
            .take_deferred()
            .into_iter()
            .filter_map(|deferred| match deferred {
                mush_core::Deferred::Persist(op) => Some(op),
                _ => None,
            })
            .collect();
        assert_eq!(writes.len(), 1, "one ring: {writes:?}");
        let (key, value) = match &writes[0].op {
            mush_core::ModuleWrite::JsonSet { key, value } => (key.clone(), value.clone()),
            other => panic!("a ring is one document: {other:?}"),
        };
        assert_eq!(key.as_str(), "tacsim:earned:4");
        // The same chunk again changes nothing and writes nothing.
        note_chunk(&mut ctx(tw.engine_mut(), &mut out), task, &events);
        assert!(out.take_deferred().is_empty());
        // A boot over the document restores the ring; one naming another author is skipped.
        let mut booted = world();
        let mut wrong = value.clone();
        wrong["author"] = serde_json::json!(5);
        crate::minds::earned_contacts::read(
            &mut ctx(booted.engine_mut(), &mut out),
            &[
                (key, value),
                (smol_str::SmolStr::new("tacsim:earned:6"), wrong),
            ],
        );
        assert!(has(&booted, 4), "the ring survives the reboot");
        assert!(!has(&booted, 5) && !has(&booted, 6));
    }
}
