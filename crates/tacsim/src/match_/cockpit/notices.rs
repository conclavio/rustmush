//! Cockpit notice rendering, catch-up reports, lock/mind/profile commands, and receiver delivery.

use super::*;

/// **The line one notice prints** (16b cut S.2, extended by 16b cut F.2; doc 16
/// §3.6: *notices fire once per transition*, §3.4 step 4, §5: *`Notice` events for
/// transitions and refusals*).
///
/// The one renderer, so the bridge's output line, the frame's `notices` array and
/// any later surface cannot word the same transition three ways. `rows` is the
/// frame's own contact list, which is where an acquisition's bearing and band come
/// from; a `lost` contact is not in it any more and is named by its id alone.
/// `frame` is the frame the notice rides — a lock's percentage is the one this
/// pulse resolved, read off the very rows `WEAPONSTATUS` prints, so the two can
/// never be two numbers. Every sentence is otherwise built from the notice's
/// **own** fields: the frame is a source of numbers and never of identities, so
/// two locks that resolved in one pulse each name the contact they were about.
///
/// **The observer rule is checked sentence by sentence here.** A shooter's line
/// names the contact its own platform holds and never a unit; a victim's line
/// names the victim's own section and facet and carries no id, no bearing, no team
/// and no weapon; a blocked line names a cause and never a cell's occupant.
///
/// **16b cut Q.1**: every contact is named by its short designator and never by
/// the sixteen digits, and **one notice renders to nothing** — a `classified`
/// whose contact was never provisional, which is a mesh arrival whose
/// acquisition already read `New contact RB: …`. An empty answer means *say
/// nothing*, and [`notice_lines`] is where that is honoured.
#[must_use]
pub fn notice_line(frame: &Frame, notice: &FrameNotice, rows: &[ContactRow]) -> String {
    match notice {
        FrameNotice::Contact {
            kind,
            contact,
            class,
            was,
        } => {
            let row = rows.iter().find(|row| row.id == contact.id);
            let named = contact.designator;
            match kind {
                NoticeKind::Acquired => match row {
                    Some(row) => format!(
                        "New contact {named}: bearing {:03}°, {}.",
                        row.bearing_deg, row.band
                    ),
                    None => format!("New contact {named}."),
                },
                // **16b cut Q.1**: the rename, in one sentence, quoting what the
                // pilot has been calling it. A classification that was never
                // provisional — a mesh arrival, or a match run with
                // `designate_on_detection` on — says **nothing**: its acquisition
                // already read `New contact RB: …`, and a second line renaming
                // `RB` to `RB` is noise.
                NoticeKind::Classified => match was {
                    Some(was) => {
                        format!("Contact {was} identified: {named}, {}.", class_word(*class))
                    }
                    None => String::new(),
                },
                // **16b cut D.4** (`B-20`): the other direction of the same
                // sentence. A contact that was told apart and has come back a
                // number is quoted under both names, because the pilot has been
                // typing the first one — and the first one stops working the
                // moment this line is printed, which is the whole reason it is
                // printed. It is **not** an alias: see `resolve_designator`.
                NoticeKind::Unidentified => match was {
                    Some(was) => format!("{was} is unidentified again: {named}."),
                    None => String::new(),
                },
                // 16b cut W.1: the lock broke because the contact stopped being
                // held. Said at once, because the pilot's next `FIRE` is going to
                // be refused for it.
                NoticeKind::LockLost => format!("Lock on {named} broken: contact gone."),
                // `Lost`, and anything a later cut puts in the contact arm
                // without a sentence of its own.
                _ => format!("Contact {named} lost."),
            }
        }
        // 16b cut F.2: doc 16 §3.5 step 5's percentage, said when the lock
        // resolves. The same sentence `WEAPONSTATUS` prints, from the same place.
        //
        // The percentage is quoted **only** when the frame's lock is still this
        // notice's contact. `LOCK` is a discrete, queued intent, so two of them
        // drain in one pulse and the frame carries the second one's lock: a
        // superseded lock's line would otherwise be rendered with the chance
        // against a contact the pilot has already stopped aiming at, and both
        // lines would name the same id. The bare sentence is the honest answer
        // for a lock that was taken and then replaced inside one pulse.
        FrameNotice::Locked { contact } => {
            let named = contact.designator;
            match frame_lock(frame) {
                Some(held) if held == contact.id => {
                    lock_line(frame).unwrap_or_else(|| format!("Locked on {named}."))
                }
                _ => format!("Locked on {named}."),
            }
        }
        // **16b cut Q.2**: the section word, where the shooter has earned one —
        // the platform holds the target and has told it apart. `Hit on BA: turret.`
        // against `Hit on X2.` against `Hit.`, which are the three things a gunner
        // can honestly be told about where a round went.
        // **16b cut D.2**: `(FRIENDLY)` after the designator (`B-18`), and the
        // one clause a gunner who has told the target apart earns when the round
        // found a section that was already gone (`B-15`). A gunner holding the
        // target *unidentified* carries no section and no `spent`, so his line is
        // byte for byte what it was.
        FrameNotice::Hit {
            contact,
            section,
            spent,
        } => {
            let already = if *spent { ", already destroyed" } else { "" };
            match (contact, section) {
                (Some(contact), Some(section)) => format!(
                    "Hit on {}{}: {section}{already}.",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                (Some(contact), None) => format!(
                    "Hit on {}{}.",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                (None, _) => "Hit.".to_owned(),
            }
        }
        // **16b cut AE.2**: a miss on a target the platform no longer holds
        // carries no reason (`Arena::stage_pilot_notice` stages it empty), so it
        // is the one word: where the target went and what stood round it are
        // not the gunner's to know once the contact is gone. `Hit.` is its twin.
        FrameNotice::Miss {
            contact, reason, ..
        } => match contact {
            Some(contact) => format!("Miss on {}: {}.", contact.designator, miss_words(reason)),
            None if reason.is_empty() => "Miss.".to_owned(),
            None => format!("Miss: {}.", miss_words(reason)),
        },
        FrameNotice::Kill { contact } => match contact {
            Some(contact) => format!("{} destroyed.", contact.designator),
            None => "Target destroyed.".to_owned(),
        },
        FrameNotice::Struck {
            section,
            armor_section,
            facet,
            armor_left,
            armor_max,
            internals_left,
            internals_max,
        } => {
            // Doc 16 §3.3: a section with no armour of its own is **struck
            // through** another's, and the armour numbers are then that other
            // section's cell. The sentence says so rather than hanging a foreign
            // facet off the section that took the round — a pilot who reads
            // `the engine's rear facet` on a tank whose engine has no facets is
            // being told something about their own machine that is not true.
            let place = if armor_section == section {
                format!("the {section}'s {facet} facet")
            } else {
                format!("the {section}, through the {armor_section}'s {facet} facet")
            };
            if *armor_left > 0 {
                format!("Hit on {place}: {armor_left} of {armor_max} armour left.")
            } else {
                format!("Hit on {place}: armour gone, {internals_left} of {internals_max} inside.")
            }
        }
        // **16b cut D.2** (`B-15`): the round found a section that was already
        // destroyed, so there are no numbers left to report and one sentence says
        // the whole of it — at most once every `SPENT_NOTICE_MS` per section, so
        // six of these in ten seconds is one.
        FrameNotice::StruckSpent { section } => {
            format!("Hit on the {section}, which is already destroyed.")
        }
        FrameNotice::SectionLost {
            section,
            consequences,
        } => {
            let cost = consequence_words(consequences);
            if cost.is_empty() {
                format!("The {section} is destroyed.")
            } else {
                format!("The {section} is destroyed: {cost}.")
            }
        }
        // **16b cut Q.2**: the gunner's half. The victim's line above says what it
        // cost *them*; this one says only that the section went, because what a
        // lost track does to a tank is the crew's own knowledge and a gunner
        // watching from 400 m has not read it.
        FrameNotice::TargetSectionLost { contact, section } => {
            format!("{}'s {section} is destroyed.", contact.designator)
        }
        FrameNotice::Refused {
            order,
            refusal,
            weapon,
            detail,
        } => refusal_line(order, *refusal, weapon.as_deref(), *detail),
        FrameNotice::Blocked { cause } => match cause {
            BlockedCause::Impassable => "Held: the ground ahead will not take the tank.".to_owned(),
            BlockedCause::Occupied => "Held: something is in the way.".to_owned(),
            BlockedCause::Edge => "Held: this is the edge of the arena.".to_owned(),
            // 16b cut X.6: the ground ahead would take the tank; the slope onto
            // it will not. The difference is worth a sentence of its own, because
            // what a pilot does about it is drive round rather than give up.
            BlockedCause::TooSteep => "Held: the slope ahead is too steep for the tank.".to_owned(),
        },
        // **16b cut Q.1**: the far end of `STARTUP`. Three words, because the
        // only thing a pilot waiting on the sequence wants is to know it is over.
        FrameNotice::PowerOnline => "Power online.".to_owned(),
        // **16b cut D.2** (`B-03`): the pilot's own tank, and nothing about who
        // did it. Both sentences say what the pilot may do next, because the
        // thing the smoke matches found is somebody sitting in a dead tank with
        // no idea whether the match was still going on.
        FrameNotice::OwnDestroyed => OWN_DESTROYED.to_owned(),
        FrameNotice::OwnDisabled => OWN_DISABLED.to_owned(),
        // **16b cut D.4** (`B-22`): the pilot's own machine. It says that the
        // sight is being interfered with and nothing whatever about the pod — no
        // source, no bearing, no side, no id — which is 16c §5 Q17's own sentence.
        FrameNotice::Receiver { degraded } => if *degraded {
            RECEIVER_DEGRADED
        } else {
            RECEIVER_CLEAR
        }
        .to_owned(),
        // **16b cut O.2** (16c N12): the pilot's own pod, and what the switch
        // costs said in the line — on, the carrier's own sight is inside the
        // bubble with everybody else's; off, the sight clears and the bubble
        // hides nobody. The radius is the pod's own; nothing here names any
        // sight it reaches.
        FrameNotice::Emcon { on: true, radius_m } => format!(
            "EMCON on: the pod radiates; every sight inside {radius_m} m is jammed, your own \
             among them."
        ),
        FrameNotice::Emcon { on: false, .. } => {
            "EMCON off: the pod is silent; your own sight clears, and nothing hides you.".to_owned()
        }
        // **17b cut M1a.4** (doc 17 §4 D-UM-10): the motor's two answers to the
        // pilot's own `DRIVE`, said once each. Nothing about the route, the cell
        // or any other hull.
        FrameNotice::Drive {
            outcome: DriveOutcome::Arrived,
        } => DRIVE_ARRIVED.to_owned(),
        FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        } => DRIVE_NO_ROUTE.to_owned(),
        // **17b cut M1b.1** (doc 17 §5: *every slot shot is announced once*;
        // 17b cut M1b.4 stages it): the pilot's own slot and weapon, the lock
        // its platform holds by its designator (with the friendly mark by the
        // existing rule), and the chance and the side word that row earned.
        FrameNotice::SlotFired {
            slot,
            weapon,
            contact,
            p_permille,
            aspect,
        } => {
            let gun = capitalised(weapon);
            let percent = p_permille / 10;
            let side = aspect.side_word();
            match contact {
                Some(contact) => format!(
                    "{gun} fires at {}{} by S{slot} ({percent}%, {side}).",
                    contact.designator,
                    friendly_mark(frame, contact.id)
                ),
                None => format!("{gun} fires by S{slot} ({percent}%, {side})."),
            }
        }
        // **17b cut M1b.1** (doc 17 §5: *a held shot said at most once per
        // contact per 10 s*; 17b cut M1b.4 stages it): the slot, the weapon and
        // the term that failed.
        FrameNotice::SlotHeld { slot, weapon, why } => {
            format!("S{slot} held the {weapon}: {}.", held_words(why))
        }
    }
}

/// **A weapon id with its first letter upper-cased** (17b cut M1b.1; 17b's
/// Q-M2): `Cannon`, `Machine_gun` — the slot shot's opening word.
pub(super) fn capitalised(weapon: &str) -> String {
    let mut letters = weapon.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// **Why a slot held, in plain words** (17b cut M1b.1; doc 17 §5's held
/// sentence): `54% under your 60%`, `BA shows its front`, `BA at 1200 m,
/// outside 200 to 600` (`under {max}` when the slot has no nearest, `past
/// {min}` when it has no furthest), `8 rounds, keeps 8`.
pub(super) fn held_words(why: &HeldWhy) -> String {
    match why {
        HeldWhy::Odds { permille, floor } => {
            format!("{}% under your {}%", permille / 10, floor / 10)
        }
        HeldWhy::Aspect { contact, aspect } => {
            format!("{} shows its {}", contact.designator, aspect.side_word())
        }
        HeldWhy::Range {
            contact,
            range_m,
            min_m,
            max_m,
        } => {
            let bounds = if *min_m == 0 {
                format!("under {max_m}")
            } else if *max_m == u32::MAX {
                format!("past {min_m}")
            } else {
                format!("{min_m} to {max_m}")
            };
            format!("{} at {range_m} m, outside {bounds}", contact.designator)
        }
        HeldWhy::Rounds { rounds, keeps } => format!("{rounds} rounds, keeps {keeps}"),
    }
}

/// **A held slot's reason as the keyframe carries it** (17b cut M1b.1): the
/// notice object's `detail`, `{ "why", "percent", "floor", "aspect",
/// "range_m", "min_m", "max_m", "rounds", "keeps" }` — every key on every
/// held notice, the other whys' `null` — in whole per cents and metres, the
/// facet word for the aspect. The contact's designator rides the notice's own
/// `designator` key.
pub(super) fn held_detail(why: &HeldWhy) -> serde_json::Value {
    let null = serde_json::Value::Null;
    let (word, percent, floor, aspect, range_m, min_m, max_m, rounds, keeps) = match why {
        HeldWhy::Odds { permille, floor } => (
            "odds",
            serde_json::json!(permille / 10),
            serde_json::json!(floor / 10),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
        ),
        HeldWhy::Aspect { aspect, .. } => (
            "aspect",
            null.clone(),
            null.clone(),
            serde_json::json!(aspect.as_str()),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
        ),
        HeldWhy::Range {
            range_m,
            min_m,
            max_m,
            ..
        } => (
            "range",
            null.clone(),
            null.clone(),
            null.clone(),
            serde_json::json!(range_m),
            serde_json::json!(min_m),
            serde_json::json!(max_m),
            null.clone(),
            null,
        ),
        HeldWhy::Rounds { rounds, keeps } => (
            "rounds",
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
            null,
            serde_json::json!(rounds),
            serde_json::json!(keeps),
        ),
    };
    serde_json::json!({
        "why": word,
        "percent": percent,
        "floor": floor,
        "aspect": aspect,
        "range_m": range_m,
        "min_m": min_m,
        "max_m": max_m,
        "rounds": rounds,
        "keeps": keeps,
    })
}

/// **`(FRIENDLY)` where this frame's own row for `contact` says so** (16b cut
/// D.2, `B-18`), and the empty string everywhere else.
///
/// Read off the frame's rows at render time and never carried on the notice: a
/// contact's side is a thing the platform resolves and may re-resolve, and an
/// unidentified return has no side at all — so an `unknown` row is never marked,
/// by construction.
pub(super) fn friendly_mark(frame: &Frame, contact: ContactId) -> &'static str {
    let friendly = frame
        .contacts
        .iter()
        .find(|held| held.id == contact)
        .is_some_and(|held| held.relation.is_friendly());
    if friendly { FRIENDLY_MARK } else { "" }
}

/// **The line a pilot reads when their own tank becomes a wreck** (16b cut D.2,
/// `B-03`).
pub const OWN_DESTROYED: &str = "Your tank is destroyed. The match runs on without you: wait it \
                                 out, EXIT to the team room, or SIM ABORT to call it off.";

/// **The line a pilot reads when their own tank can neither move nor shoot**
/// (16b cut D.2, `B-03`).
pub const OWN_DISABLED: &str = "Your tank can neither move nor shoot. The match runs on: wait it \
                                out, EXIT to the team room, or SIM ABORT to call it off.";

/// **The line a pilot reads when their own receiver goes into interference** (16b
/// cut D.4, `B-22`).
///
/// It names no source, because the platform has none to name: the fold is read at
/// the pilot's own cell and nothing about the emitter reaches it
/// ([`crate::arena::perception::Receiver`]).
pub const RECEIVER_DEGRADED: &str = "Receiver degraded: something is jamming the sight.";

/// **The line a pilot reads when their own receiver comes back out of it** (16b
/// cut D.4, `B-22`).
pub const RECEIVER_CLEAR: &str = "Receiver clear.";

/// **`STATUS`'s receiver line** (16b cut D.4, `B-22`), split out of the verb the
/// way [`range_line`] is, so the one sentence can be pinned against a value
/// rather than against a booted server.
///
/// The budget the platform's last pass really ran with against what that budget
/// is when nothing is jamming, and what is left of the reach. Three integers
/// about the pilot's own machine and **nothing about the emitter**: no source, no
/// bearing, no side, no cell, no object number and no designator, which is 16c
/// §5 Q17 read literally. See [`crate::arena::perception::Receiver`] for the two
/// limits the integers are declared to have.
#[must_use]
pub fn receiver_line(receiver: crate::arena::perception::Receiver) -> String {
    format!(
        "  receiver degraded: {} of {} through cover, {}% reach",
        receiver.budget,
        receiver.budget_max,
        receiver.reach_permille / 10
    )
}

/// **A miss reason in plain words** (16b cut F.2), one per
/// [`crate::arena::damage::MISS_REASONS`] — the ledger's own vocabulary read as a
/// gunner hears it. An unknown word is printed as it stands rather than swallowed,
/// so a seventh reason is a sentence somebody has to come and write.
///
/// **16b cut V.3** made it `pub(crate)`: a pilot's report says a miss in exactly
/// these words (`crate::match_::report`).
pub(crate) fn miss_words(reason: &str) -> &str {
    match reason {
        damage::TARGET_GONE => "the target is gone",
        damage::WRECK => "the target is already a wreck",
        damage::MOVED => "the target moved",
        damage::BLOCKED => "the line was blocked",
        damage::COVER => "the target is behind cover",
        damage::DISPERSION => "the round went wide",
        damage::REFUSED => "the round found nothing to break",
        other => other,
    }
}

/// **What a lost section costs the tank, in plain words** (16b cut F.2), joined in
/// authored order.
///
/// The words are `crate::arena::resolution::Consequence::label`'s, so the pilot's
/// sentence and the black box name the same things. The **list** is not the
/// ledger line's: `crate::arena::damage::Arena::fired_consequences` has already
/// dropped the one consequence that can be authored and not fire — a biped's
/// `disable_motion_when_all_of` on the first of its two legs — so every word
/// joined here is true of the tank as it now stands.
///
/// **16b cut V.3** made it `pub(crate)`: a pilot's report says a lost section of
/// its own tank in these words, over the same fired list — the debrief's fold
/// puts the ledger line's authored list through the question
/// `fired_consequences` asks (`crate::match_::report::OwnTank`).
pub(crate) fn consequence_words(consequences: &[SmolStr]) -> String {
    consequences
        .iter()
        .map(|label| match label.as_str() {
            "destroy_unit" => "the tank is finished",
            "remove_weapons" => "its weapons are gone",
            "slow" => "the tank is slowed",
            "no_turn_toward" => "it will not turn that way",
            "disable_motion" | "disable_motion_when_all_of" => "the tank cannot move",
            // **17b cut M3.6** (doc 17 §9, D-UM-34): the core's two words, now
            // that the shipped turrets carry them — a raw internal word never
            // reaches a pilot. Each is joined only where it is true of the
            // tank now: `fired_consequences` (and the debrief's `OwnTank`)
            // drop a `destroy_core` on a hull never fitted with a core and a
            // `degrade_core` that left the core no lower than its sealed
            // class (a degrade at the floor).
            "destroy_core" => "its core is gone",
            "degrade_core" => "its core is degraded",
            other => other,
        })
        .collect::<Vec<&str>>()
        .join("; ")
}

/// **The arena's sentence for a profile switch it refused** (17b cut M5.2;
/// [`crate::arena::intents::Refusal::Profile`]): the half after `MIND
/// refused.`, which [`order_refused`] composes from the intent's word.
pub const PROFILE_REFUSED: &str = "That profile cannot be switched to now.";

/// **The line an arena-side refusal prints** (16b cut F.2).
///
/// It is the **same sentence** the cockpit's own pre-check prints for the same
/// condition, built from the same [`CockpitError`] — so the answer a pilot gets
/// when the frame was fresh enough to refuse locally and the answer they get a
/// pulse later are one string, and neither can drift. The prefix names the order,
/// because a refusal that arrives a pulse after the pilot typed something has to
/// say what it is about.
///
/// **Every [`Refusal`](crate::arena::intents::Refusal) variant is matched by
/// name** (16b cut F.3). Cut F.2 ended the match with a catch-all that formatted
/// the `thiserror` text, so a later cut that routed a new variant into phase 4
/// would have shipped a pilot the sentence `FIRE refused. Refused: that unit
/// already has as many actions queued as it may.` — lower-case, doubly prefixed,
/// in nobody's voice. A new variant is now a compile error instead, which is a
/// sentence somebody has to come and write.
pub(crate) fn refusal_line(
    order: &str,
    refusal: crate::arena::intents::Refusal,
    weapon: Option<&str>,
    detail: RefusalDetail,
) -> String {
    use crate::arena::intents::Refusal;
    let said = match (refusal, detail) {
        (Refusal::NoContact, _) => CockpitError::NoSuchContact.to_string(),
        (Refusal::NoLock, _) => CockpitError::NoLock.to_string(),
        (Refusal::Cycling, RefusalDetail::Cycling { secs }) => {
            CockpitError::Cycling { secs }.to_string()
        }
        (Refusal::Cycling, _) => CockpitError::Cycling { secs: 0 }.to_string(),
        (Refusal::Empty, _) => CockpitError::Empty.to_string(),
        (Refusal::Overheated, RefusalDetail::Overheated { heat, max }) => {
            CockpitError::Overheated { heat, max }.to_string()
        }
        // 16b cut F.3: a sentence, not the bare word. `OVERHEATED` is the column
        // heading `WEAPONSTATUS` prints and the opener `CockpitError::Overheated`
        // builds on, and a refusal staged before the heat cell was read has no
        // numbers to put after it — so it says the half of that sentence that is
        // still true rather than trailing off.
        (Refusal::Overheated, _) => format!("{OVERHEATED}: let it cool."),
        (Refusal::PowerNotOnline, _) => CockpitError::PowerNotOnline.to_string(),
        (Refusal::NoWeapon, _) => {
            CockpitError::NoSuchWeapon(weapon.unwrap_or_default().to_owned()).to_string()
        }
        // The rest cannot be produced by a pilot's own order today — a wreck and a
        // unit the arena has let go are refused at phase 2 unless the tank died
        // between the queue and the apply, which is exactly when a pilot most
        // wants to be told. One short sentence each, in the same voice.
        (Refusal::Wreck, _) => "Your tank is a wreck.".to_owned(),
        (Refusal::Unknown, _) => "That tank is no longer in the match.".to_owned(),
        (Refusal::Epoch, _) => "Your seat changed hands; try that again.".to_owned(),
        (Refusal::Channel, _) => "That order does not travel on this station.".to_owned(),
        (Refusal::QueueFull, _) => "Too many orders at once; try that again.".to_owned(),
        (Refusal::MotionDisabled, _) => "Your tank cannot move.".to_owned(),
        (Refusal::Linkdead, _) => "Your seat has gone linkdead.".to_owned(),
        (Refusal::NotMachine, _) => "That tank has a pilot in it.".to_owned(),
        (Refusal::NoCommander, _) => "That lance leader has no active command hull.".to_owned(),
        // 16b cut O.2: the cockpit refuses it locally off the frame; the arena's
        // word reaches here only when the frame was not yet there to say so.
        (Refusal::NoJammer, _) => CockpitError::NoJammer.to_string(),
        // Name the captured floor only for a below-floor term. An invalid
        // shape or lost Contact classification has no floor detail.
        (Refusal::Unlawful, RefusalDetail::Floor { permille }) => CockpitError::Unlawful {
            floor_pct: permille / 10,
        }
        .to_string(),
        (Refusal::Unlawful, _) => "That rule is not lawful for this unit.".to_owned(),
        // The count's own word.
        (Refusal::NoSlot, _) => CockpitError::NoSlot.to_string(),
        // The law's other word (gated for a mind from 17b cut M1b.2).
        (Refusal::Unwarranted, _) => "That has no warrant.".to_owned(),
        // **17b cut M5.2** (doc 17 §11, D-UM-44): the arena's word for a
        // `MIND PROFILE` it cannot take, said as `MIND refused. That profile
        // cannot be switched to now.`; the cockpit's own pre-checks (17b cut
        // M5.4) say the specific case through the same `order_refused`.
        (Refusal::Profile, _) => PROFILE_REFUSED.to_owned(),
    };
    order_refused(order, weapon, &said)
}

/// **The one shape every refused order's sentence has** (16b cut D.2, `B-16`):
/// `HEADING refused. …`, and `FIRE machine_gun refused. …` where the order named
/// a weapon.
///
/// Cut F.2's prefix was the order word alone, so a pilot who carried two guns
/// read `FIRE refused. Still cycling: 3 s to go.` and had to guess which breech
/// was busy — and the cockpit's own pre-check said a *different* sentence for the
/// same condition, with no prefix at all. Both halves compose through here now,
/// so the early answer and the late one are one string and a pilot who fires two
/// weapons reads which one was refused.
///
/// The weapon goes **after the order word and before `refused`**, because that is
/// where the pilot typed it.
#[must_use]
pub fn order_refused(order: &str, weapon: Option<&str>, said: &str) -> String {
    match weapon {
        Some(weapon) if !weapon.trim().is_empty() => {
            format!("{} {weapon} refused. {said}", order.to_uppercase())
        }
        _ => format!("{} refused. {said}", order.to_uppercase()),
    }
}

/// **A `FIRE` the cockpit refused against its own mirror** (16b cut D.2,
/// `B-16`), in the sentence the arena would have said a pulse later.
///
/// The `said` half is the [`CockpitError`]'s own `Display`, which is where each
/// of those sentences already lives — so this adds the prefix and invents
/// nothing.
pub(super) fn fire_refused(weapon: &str, said: &CockpitError) -> CockpitError {
    CockpitError::Refused(order_refused(
        IntentKind::fire(weapon).as_str(),
        Some(weapon),
        &said.to_string(),
    ))
}

/// Every line the notices on one frame print, in order — what the bridge says and
/// what a read verb's frame carries, from one place.
///
/// **16b cut Q.1**: a notice that renders to nothing is dropped rather than said
/// as a blank line (see [`notice_line`]). The filter is here, at the one funnel
/// every spoken line goes through, so there is one rule and not one per caller.
#[must_use]
pub fn notice_lines(frame: &Frame) -> Vec<String> {
    let rows = frame_contacts(frame);
    frame
        .notices
        .iter()
        .map(|notice| notice_line(frame, notice, &rows))
        .filter(|line| !line.is_empty())
        .collect()
}

/// **What the machine was doing, in the pilot's words** (16b cut Y.4), by
/// [`crate::arena::doctrine::MachineState::as_str`]'s word. A word this build
/// predates reads as itself with its underscores opened out, which is the
/// panels' own fallback for a word they do not know.
///
/// **17b cut M5.3 (D14, doc 17 §24's index)**: `unknown` — a hull whose sealed
/// mind is not Standard Doctrine, whose memory says nothing in the doctrine's
/// words ([`crate::arena::doctrine::MachineState::Unknown`]) — reads `flying on
/// its own mind`, not the opened word: `  The machine was flying on its own
/// mind under no order.` The TUI's `catchup_words` and the web's
/// `MACHINE_STATE_WORDS` say the same, pinned by `catchup_lines.json`.
/// Reachable only with a fixture mind before 17b cut M6.
pub(super) fn machine_state_words(state: &str) -> String {
    match state {
        "startup" => "starting up".to_owned(),
        "patrol" => "patrolling".to_owned(),
        "approach" => "closing on a contact".to_owned(),
        "engage" => "engaging".to_owned(),
        "withdraw" => "withdrawing toward its entry zone".to_owned(),
        "disabled" => "disabled".to_owned(),
        "unknown" => "flying on its own mind".to_owned(),
        other => other.replace('_', " "),
    }
}

/// **What the machine was under** (16b cut Y.4), by the standing order's
/// objective word ([`crate::deliberation::Objective::as_str`]), or `no order`
/// under doctrine.
pub(super) fn machine_order_words(objective: Option<&str>) -> String {
    match objective {
        None => "no order".to_owned(),
        Some("hold") => "a hold order".to_owned(),
        Some("advance") => "an advance order".to_owned(),
        Some("engage") => "an engage order".to_owned(),
        Some("withdraw") => "a withdraw order".to_owned(),
        Some(other) => format!("a {} order", other.replace('_', " ")),
    }
}

/// `(x, y)`, the way every cell on the catch-up reads.
pub(super) fn cell_words((x, y): (u16, u16)) -> String {
    format!("({x}, {y})")
}

/// **One list clause of the catch-up's contacts line** (16b cut Y.4): `none
/// new` / `new BA, X3` / `new BA, X3 and 3 more`.
pub(super) fn listed_clause(word: &str, names: &[String], more: u32) -> String {
    match (names.is_empty(), more) {
        (true, 0) => format!("none {word}"),
        (true, more) => format!("{word} {more} more"),
        (false, 0) => format!("{word} {}", names.join(", ")),
        (false, more) => format!("{word} {} and {more} more", names.join(", ")),
    }
}

/// **A list with its overflow** (16b cut Y.4): `a, b` or `a, b and 2 more`, or
/// `none_word` when there is nothing at all.
pub(super) fn listed_or(parts: &[String], separator: &str, more: u32, none_word: &str) -> String {
    match (parts.is_empty(), more) {
        (true, 0) => none_word.to_owned(),
        (true, more) => format!("{more} more"),
        (false, 0) => parts.join(separator),
        (false, more) => format!("{} and {more} more", parts.join(separator)),
    }
}

/// **One section's clause of the catch-up** (16b cut Y.4): the word change when
/// the word moved (`left_track intact then, destroyed now`), else the armour
/// that fell and the internals that fell (`hull armour 230 then, 90 now,
/// internals 60 then, 40 now`).
pub(super) fn section_clause(id: &str, then: &SectionWords, now: &SectionWords) -> String {
    if then.condition != now.condition {
        return format!("{id} {} then, {} now", then.condition, now.condition);
    }
    let mut clauses: Vec<String> = Vec::new();
    if now.armour < then.armour {
        clauses.push(format!("armour {} then, {} now", then.armour, now.armour));
    }
    if now.internals < then.internals {
        clauses.push(format!(
            "internals {} then, {} now",
            then.internals, now.internals
        ));
    }
    if clauses.is_empty() {
        id.to_owned()
    } else {
        format!("{id} {}", clauses.join(", "))
    }
}

/// **The catch-up report, as the pilot reads it** (16b cut Y.4; doc 07 §1.3
/// step 5): one pure function of the report, and the block line mode is sent
/// as one `Sys` line tagged [`crate::arena_bridge::NOTICE_SENTENCE_TAG`]. Both
/// panels compose the same lines from the frame's `catchup` object
/// (`mush_net_ssh`'s `catchup_words`, the web's `catchupLines`), and
/// `web/src/lib/fixtures/cockpit/catchup_lines.json` pins all three.
///
/// ```text
/// Back aboard RA after 2 m 14 s away.
///   The machine was withdrawing toward its entry zone under no order.
///   Position: cell (64, 97) then, (70, 101) now.
///   Contacts: new BA, X3; lost X2.
///   Sections: left_track intact then, destroyed now; hull armour 230 then, 90 now.
///   Rounds spent: cannon 3, machine_gun 40.
///   Power online.
/// ```
///
/// Line 1's duration is `away_ms.div_euclid(1_000)` whole seconds through
/// [`crate::match_::duration_words`] — the wire's own `away_s`, so the line
/// client and both rings print one figure — and it says `away` and not `on
/// its mind` (`on doctrine` here until 17b cut M5.7), because the count starts
/// at the `Linkdead`, before the machine had the tank. An eighth line, `Your
/// tank is a wreck.`, follows the power line when it is one. A capped list
/// ends `and N more`; the contacts' one
/// overflow count goes on the one list that is full — only a full list can
/// have overflowed — and when both are full, which cannot tell whose it was,
/// it is its own clause, `3 more not named`.
///
/// Nothing on any line names a unit, a pilot, an object number or a position
/// of any other unit: the contacts are the platform's own designators, held now
/// or held then (doc 16 §1, §3.6).
#[must_use]
pub fn catchup_lines(callsign: Designator, catchup: &Catchup) -> Vec<String> {
    let away_s = catchup.away_ms.div_euclid(1_000);
    let mut lines = vec![format!(
        "Back aboard {callsign} after {} away.",
        crate::match_::duration_words(away_s.saturating_mul(1_000))
    )];
    lines.push(match &catchup.machine {
        Some(machine) => format!(
            "  The machine was {} under {}.",
            machine_state_words(&machine.state),
            machine_order_words(machine.objective.as_deref())
        ),
        None => "  The machine had no state.".to_owned(),
    });
    lines.push(match (catchup.cell_then, catchup.cell_now) {
        (_, None) => "  Position: unknown.".to_owned(),
        (None, Some(now)) => format!("  Position: cell unknown then, {} now.", cell_words(now)),
        (Some(then), Some(now)) if then == now => {
            format!("  Position: cell {}, unchanged.", cell_words(now))
        }
        (Some(then), Some(now)) => format!(
            "  Position: cell {} then, {} now.",
            cell_words(then),
            cell_words(now)
        ),
    });
    let new: Vec<String> = catchup
        .contacts_new
        .iter()
        .map(ToString::to_string)
        .collect();
    let lost: Vec<String> = catchup
        .contacts_lost
        .iter()
        .map(ToString::to_string)
        .collect();
    lines.push(contacts_line(&new, &lost, catchup.contacts_more));
    let sections: Vec<String> = catchup
        .sections
        .iter()
        .map(|(id, then, now)| section_clause(id, then, now))
        .collect();
    lines.push(format!(
        "  Sections: {}.",
        listed_or(&sections, "; ", catchup.sections_more, "unchanged")
    ));
    let rounds: Vec<String> = catchup
        .rounds_spent
        .iter()
        .map(|(weapon, rounds)| format!("{weapon} {rounds}"))
        .collect();
    lines.push(format!(
        "  Rounds spent: {}.",
        listed_or(&rounds, ", ", catchup.rounds_more, "none")
    ));
    lines.push(format!("  Power {}.", catchup.power_now.as_str()));
    if catchup.wreck {
        lines.push("  Your tank is a wreck.".to_owned());
    }
    lines
}

/// **The catch-up's contacts line** (16b cut Y.4): `none new, none lost` when
/// nothing moved, else the two clauses split by `; `.
///
/// The wire carries one overflow count for both lists (`contacts_more`), and
/// only a list cut at [`MAX_CATCHUP_CONTACTS`] names can have overflowed, so
/// the count goes after the one list that is full. When both are full the
/// count cannot say whose it was — a platform that gained exactly twelve and
/// lost fifteen must not read as fifteen gained — so it is said as its own
/// clause, `3 more not named`, and so is a count beside no full list, which
/// the arena never sends.
pub(super) fn contacts_line(new: &[String], lost: &[String], more: u32) -> String {
    if new.is_empty() && lost.is_empty() && more == 0 {
        return "  Contacts: none new, none lost.".to_owned();
    }
    let new_full = new.len() >= MAX_CATCHUP_CONTACTS;
    let lost_full = lost.len() >= MAX_CATCHUP_CONTACTS;
    let (new_more, lost_more, unnamed) = match (more, new_full, lost_full) {
        (0, _, _) => (0, 0, 0),
        (more, true, false) => (more, 0, 0),
        (more, false, true) => (0, more, 0),
        (more, _, _) => (0, 0, more),
    };
    let clauses = format!(
        "{}; {}",
        listed_clause("new", new, new_more),
        listed_clause("lost", lost, lost_more)
    );
    if unnamed == 0 {
        format!("  Contacts: {clauses}.")
    } else {
        format!("  Contacts: {clauses}; {unnamed} more not named.")
    }
}

/// **The catch-up report on the wire** (16b cut Y.4): the `catchup` object of
/// [`status_frame`]. Every figure is the report's own; `away_s` is
/// `away_ms.div_euclid(1_000)`, the one integer the three renderers print.
pub(super) fn catchup_json(catchup: &Catchup) -> serde_json::Value {
    let words = |words: &SectionWords| {
        serde_json::json!({
            "condition": words.condition.as_str(),
            "armour": words.armour,
            "internals": words.internals,
        })
    };
    serde_json::json!({
        "away_s": catchup.away_ms.div_euclid(1_000),
        "cell_then": catchup.cell_then.map(|(x, y)| [x, y]),
        "cell_now": catchup.cell_now.map(|(x, y)| [x, y]),
        "power_then": catchup.power_then.as_str(),
        "power_now": catchup.power_now.as_str(),
        "wreck": catchup.wreck,
        "contacts_new": catchup
            .contacts_new
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>(),
        "contacts_lost": catchup
            .contacts_lost
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>(),
        "contacts_more": catchup.contacts_more,
        "sections": catchup
            .sections
            .iter()
            .map(|(id, then, now)| serde_json::json!({
                "id": id.as_str(),
                "then": words(then),
                "now": words(now),
            }))
            .collect::<Vec<serde_json::Value>>(),
        "sections_more": catchup.sections_more,
        "rounds_spent": catchup
            .rounds_spent
            .iter()
            .map(|(weapon, rounds)| serde_json::json!({
                "weapon": weapon.as_str(),
                "rounds": rounds,
            }))
            .collect::<Vec<serde_json::Value>>(),
        "rounds_more": catchup.rounds_more,
        "machine": catchup.machine.as_ref().map(|machine| serde_json::json!({
            "state": machine.state.as_str(),
            "goal": machine.goal.map(|(x, y)| [x, y]),
            "objective": machine.objective.as_ref().map(SmolStr::as_str),
        })),
    })
}

/// **The line the arena's `Locked` notice and `WEAPONSTATUS` both print about the
/// lock** (16b cut W.2; doc 16 §3.5 step 5: *the cockpit shows `p_permille` as a
/// percentage in `WEAPONSTATUS` and on `LOCK` before any shot*).
///
/// The `LOCK` verb itself no longer prints it: 16b cut F.2 moved that to the
/// arena's own notice, said when the lock resolves, so there is one owner of the
/// sentence rather than a cockpit echo beside it (16b §0.2).
///
/// `Locked on 3f2c…, 32% to hit.` when the arena has resolved one, and `Locked on
/// 3f2c….` when it has not — which is the pulse between a `LOCK` being typed and the
/// frame that answers it, and a platform carrying no weapon at all. `None` for no
/// lock, so the caller says nothing rather than saying there is nothing.
///
/// The percentage is the row of **the weapon a bare `FIRE` would fire**
/// (`fired_by_default`), rounded to whole per cent: it is the same number the
/// draw will be compared against, resolved by the arena through doc 16 §3.5's five
/// steps with the trace and the cover in it, and never a second estimate computed
/// here.
///
/// **16b cut F.2** changed which number that is. Cut W.2 showed the **best** of
/// the platform's weapons, so a pilot who read `46% to hit` and reached for the
/// coaxial during the cannon's cycle fired at 21 %; doc 16 §3.5 step 5 defines
/// `p_permille` per shot, and the shot a bare `FIRE` takes is the cannon's. Each
/// weapon's own chance is on its `WEAPONSTATUS` row.
///
/// **What it is computed from** (16b cut F.3): the locked contact's **plot** —
/// where the platform last saw the target, and how it was facing and moving then
/// — through `crate::arena::resolution::Arena::lock_aim`, which resolves the
/// range and the cover, and
/// `crate::arena::Arena::resolve_seen`, which reads the face and the
/// crossing speed off the same sighting. So this sentence, every `WEAPONSTATUS`
/// row's percentage and the contact row's own band are three readings of one plot
/// and cannot be three numbers. The shot itself is resolved against where the
/// target *is*, so for a target that has moved since the sighting the advertised
/// chance and the chance the draw is compared against differ; that is what a
/// stale track costs, and `resolve_seen` has the argument.
#[must_use]
pub fn lock_line(frame: &Frame) -> Option<String> {
    // 16b cut Q.1: the short name, off the frame's own row for that contact — a
    // lock is always one of the rows, because the arena clears a lock the moment
    // its contact stops being held.
    let named = lock_designator(frame)?;
    // 16b cut D.2 (`B-18`): a lock on a friend says so, here and therefore on
    // `WEAPONSTATUS` too. Nothing is refused.
    let mark = frame_lock(frame).map_or("", |contact| friendly_mark(frame, contact));
    Some(
        match fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille) {
            Some(permille) => format!("Locked on {named}{mark}, {}% to hit.", permille / 10),
            None => format!("Locked on {named}{mark}."),
        },
    )
}

/// **Every weapon the platform carries, live** (16b cut W.1), as the arena's frame
/// carries it.
///
/// This is the reading that matters while a match runs: the arena is the single
/// writer of `ready_at_ms` and `ammo` (D-16-1), so the frame's numbers are the
/// truth and [`weapons_of`]'s structural read of the M32 component cells is the
/// answer for a unit no arena holds — a tank in the motor pool, or a pilot who has
/// boarded and whose arena has not spoken yet.
///
/// **An empty answer is an answer** (16b cut D.1): a frame that lists no weapon
/// means the platform carries none, and `carried` does not go looking for an
/// older one.
#[must_use]
pub fn frame_weapons(frame: &Frame) -> Vec<WeaponRow> {
    frame
        .weapons
        .iter()
        .map(|weapon| WeaponRow {
            id: weapon.id.clone(),
            ready: weapon.ready,
            ready_in_ms: weapon.ready_in_ms,
            rounds: i64::from(weapon.rounds),
            rounds_max: i64::from(weapon.rounds_max),
            range_band: weapon.range_band,
            p_permille: weapon.p_permille,
            // 16b cut D.2 (`B-16`): straight off the frame's own row.
            heat_per_shot: weapon.heat_per_shot,
            // 17b cut M1b.3: the arming, off the frame's own row too.
            armed: weapon.armed.clone(),
        })
        .collect()
}

/// **The weapon rows as the pilot may act on them now** (16b cut AP.1, 16c
/// `B-86`): [`frame_weapons`] off the held frame, each row's countdown aged by
/// the wall time since the world received that frame
/// ([`CockpitFrame::received`]).
///
/// A frame is never re-sent for a countdown change
/// ([`crate::arena::weapons::FrameWeapon::same_as`] leaves `ready_in_ms` out, on
/// purpose), so the `ready_in_ms` a pilot holds is the figure at the step that
/// built the frame, and read as it stands it refused every `FIRE` typed after the
/// gun's true ready instant and before the ready frame landed — a pulse late on
/// every cycle. Aged, the row reads `remaining = ready_in_ms − elapsed`, held at
/// nought, and is `ready` once `remaining` is nought; a row that was ready stays
/// ready. Nothing else on the row moves.
///
/// **`floored` leaves the rows the frame's own.** Under a staff `@arena/tempo`
/// floor the arena moves at most `max_catch_up_steps × period` of arena time per
/// wake of `max(period, floor)` wall time, so the wall clock runs ahead of the
/// arena's and an aged row would forward early into the arena's own `Still
/// cycling` refusal; a frame built under a floor since lifted carries the same
/// wall time with it. The cockpit's `floored` answers both.
///
/// It is the verbs' reading and never the panel's: the keyframe
/// ([`status_frame_held`]) carries the frame's own figures, which the TUI and the
/// web print as they stand until the next frame — neither counts down — so a
/// `WEAPONSTATUS` line can read `ready` beside a re-pushed panel that still
/// reads the frame's own seconds.
#[must_use]
pub fn aged_weapons(held: &CockpitFrame, now: std::time::Instant, floored: bool) -> Vec<WeaponRow> {
    let rows = frame_weapons(&held.frame);
    if floored {
        return rows;
    }
    // The saturating read: an elapsed past `i64::MAX` ms is every countdown done.
    let elapsed =
        i64::try_from(now.saturating_duration_since(held.received).as_millis()).unwrap_or(i64::MAX);
    rows.into_iter()
        .map(|row| {
            let remaining = row.ready_in_ms.saturating_sub(elapsed).max(0);
            WeaponRow {
                ready: row.ready || remaining == 0,
                ready_in_ms: remaining,
                ..row
            }
        })
        .collect()
}

/// **Whether the held frame's countdown must stay unaged** (16b cut AP.1, 16c
/// `B-86`): a staff tempo floor stands on its match
/// ([`crate::arena_bridge::MatchMirror::tempo_floor_ms`]), or the frame was built
/// at or before the arena time a floor was lifted at
/// ([`crate::arena_bridge::MatchMirror::floor_lifted_at_ms`] against
/// [`CockpitFrame::arena_ms`]). In the second case the rows are the frame's own
/// until the next frame, which carries an arena time past the lift. A match the
/// registry no longer mirrors has no floor.
pub(super) fn floored(w: &World, held: &CockpitFrame) -> bool {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.mirror(held.match_id))
        .is_some_and(|mirror| {
            mirror.tempo_floor_ms.is_some()
                || mirror
                    .floor_lifted_at_ms
                    .is_some_and(|at| held.arena_ms <= at)
        })
}

// ---------------------------------------------------------------------------
// K2: `BOARD`
// ---------------------------------------------------------------------------

/// **`BOARD`** — climb into your pod and take the tank's cockpit (doc 16 §3.7).
///
/// See the module head for the order and why it is that order. Every refusal
/// happens before the first write; a seat that refuses after the body is in the
/// pod takes the body back out.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn board(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // **16b cut I.4**: nothing here unbinds a staff watcher, because a watcher
    // cannot reach this verb's success: `BOARD` needs the actor's own centre
    // slot (read below), `@arena/watch` refuses anybody who holds one — theirs
    // or their account's — and a slot is claimed only while the centre is not
    // busy, one match at a time. `open_match` keeps the one defensive unbind.
    // **16b cut F.4**: the layer, first. A pod on a server with the layer off has
    // no tank behind it, so boarding one can only end in a refusal further down
    // the path; this is that refusal, said at the word the player typed.
    lobby::require_layer(ctx.w)?;
    let Some(CentreRoom::Team(team)) = centre::where_in_centre(ctx.w, actor) else {
        return Err(CockpitError::OutsideTeamRoom);
    };
    // Every refusal, read out of the centre in one borrow.
    //
    // **16b cut Y.3** (16c `N4`): the state arm is a match on the centre's
    // state, the slot and the match record's state, because `BOARD` now means a
    // second thing in a running match — a dropped pilot taking their tank back
    // (doc 07 §1.3). `Boarding` is the path it always was, byte for byte.
    let reclaim_min_ms = lobby::configs(ctx.w).map_or_else(
        || crate::config::MatchConfig::default().reclaim_min_ms,
        |(_, m, _)| m.reclaim_min_ms,
    );
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let (match_id, pod, index, reclaim) = {
        let book = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .ok_or(CockpitError::Unavailable)?;
        let centre = book.centre();
        // The slot before the state, deliberately: somebody with no slot at all
        // is told the thing they can act on ("claim one"), not the thing they
        // cannot (what the centre happens to be doing).
        let slot = centre
            .slot_of_pilot(actor)
            .cloned()
            .ok_or(SimError::NoSlot)?;
        let record = centre
            .match_id
            .and_then(|match_id| book.get(match_id))
            .map(|record| record.state);
        // The pod's tank, by name, for the three `Running` sentences: the
        // pilot's own, which they may always be told about (doc 16 §1).
        let tank_name = || {
            slot.pod
                .and_then(|pod| centre::tank_of(ctx.w, pod))
                .or(slot.unit)
                .map(|tank| display_name(ctx.w, tank))
                .unwrap_or_default()
        };
        let reclaim = match (centre.state, record) {
            (MatchState::Boarding, _) => {
                if slot.team != team {
                    return Err(CockpitError::WrongRoom {
                        team: slot.team.as_str(),
                    });
                }
                if slot.is_seated() {
                    return Err(CockpitError::AlreadyAboard);
                }
                false
            }
            // A stop is already in flight (`unattended_deadline`, `SIM ABORT`):
            // the centre is still `Running` until `Ended` arrives, and a pilot
            // who took the tank back now would read *yours again* and then the
            // end. The record's word is the true one.
            (MatchState::Running, Some(MatchState::Stopping)) => {
                return Err(SimError::WrongState {
                    state: MatchState::Stopping.as_str(),
                }
                .into());
            }
            (MatchState::Running, _) if slot.is_dropped() => {
                let since = slot
                    .dropped_at_ms
                    .map_or(0, |dropped| now_ms.saturating_sub(dropped));
                let wait_ms = i64::from(reclaim_min_ms).saturating_sub(since);
                if wait_ms > 0 {
                    return Err(CockpitError::HandingOver {
                        unit: tank_name(),
                        // Rounded up: a pilot counts in seconds, and "0" would
                        // be a lie with a millisecond still to go.
                        secs: wait_ms.saturating_add(999) / 1_000,
                    });
                }
                true
            }
            // Out of the cockpit and not by a drop: an `EXIT`, which is one-way
            // (doc 16 §11's N.1 amendment), whatever came before it.
            (MatchState::Running, _) if slot.left => {
                return Err(CockpitError::LeftForGood { unit: tank_name() });
            }
            (MatchState::Running, _) => return Err(CockpitError::AlreadyAboard),
            (state, _) => {
                return Err(SimError::WrongState {
                    state: state.as_str(),
                }
                .into());
            }
        };
        let pod = slot.pod.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        let match_id = centre.match_id.ok_or(SimError::WrongState {
            state: centre.state.as_str(),
        })?;
        (match_id, pod, slot.index, reclaim.then_some(slot.team))
    };
    if let Some(side) = reclaim {
        return reclaim_seat(ctx, match_id, pod, side, index);
    }
    let tank = centre::tank_of(ctx.w, pod).ok_or(SimError::NoUnit {
        team: team.as_str(),
        index,
    })?;
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);

    // (1) The station, on the TANK, with the POD as its console.
    mush_sim::declare_station(
        ctx,
        tank,
        &station,
        STATION_ROLE,
        &cockpit_channels(),
        Some(pod),
        STATION_PRIORITY,
    )
    .map_err(|error| CockpitError::Station(error.to_string()))?;

    // (2) The body, inside the pod. One seat, so no compartment: the pack
    // authors none, and naming one the space does not declare is a refusal.
    mush_sim::place_interior(ctx, actor, pod, None)
        .map_err(|error| CockpitError::Hatch(error.to_string()))?;

    // (3) The seat. On a refusal the body comes back out, because a pilot shut
    // in a pod with no cockpit is the one state this verb must not leave behind.
    if let Err(error) = mush_sim::request_seat_activation(
        ctx,
        &mush_sim::SeatRequest::Activate {
            unit: tank,
            station: station.clone(),
            // Not a proxy: the pilot flies their own tank, so the seated actor
            // and the driver are the same character and `driving_actor` is a
            // fixed point on them.
            npc: actor,
            granted: cockpit_channels(),
            fallback: None,
        },
    ) {
        if let Some(room) = room {
            put_out(ctx, actor, room);
        }
        return Err(CockpitError::Seat(error.to_string()));
    }
    // The activation already republished the seated actor's grants; this is the
    // idempotent restatement doc 16 §3.7 names, and it costs one recompute.
    mush_sim::links::refresh_actor_links(ctx.w, actor);

    // **16b cut F.1**: what this used to say was "the panel wakes: STARTUP,
    // then HEADING and SPEED" — and every one of those verbs is refused until
    // the last pilot boards, because nothing is deployed until the trigger
    // fires. The first pilot to board always typed `STARTUP` and was told they
    // were not at the station of a deployed unit. So the line says what is
    // actually true: the panel waits for the other pods, or for the deadline,
    // and leaving once the match is running is final.
    let waiting = lobby::centre(ctx.w)
        .and_then(|centre| centre.board_deadline_ms)
        .map(|deadline| {
            deadline
                .saturating_sub(ctx.w.now().saturating_mul(1_000))
                .max(0)
                / 1_000
        });
    let pilot_line = match waiting {
        Some(secs) => format!(
            "You climb into {}. The panel wakes when every pilot is seated, or in {secs} \
             second(s). EXIT to get out — once the match is running, leaving abandons the tank \
             for the rest of it.",
            display_name(ctx.w, pod)
        ),
        None => format!(
            "You climb into {}. The panel wakes when every pilot is seated. EXIT to get out — \
             once the match is running, leaving abandons the tank for the rest of it.",
            display_name(ctx.w, pod)
        ),
    };
    let room_line = format!(
        "{} climbs into {}.",
        display_name(ctx.w, actor),
        display_name(ctx.w, pod)
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, pilot_line);
    // **16b cut B.4** (`B-38`): the pod's interior, after the sentence that put
    // the pilot there — `room_view` takes any object, so this is what `look`
    // from inside the pod shows. The failed-seat rollback above shows nothing
    // beyond its error.
    ctx.show_room(actor, pod);
    if let Some(room) = room {
        tell_room(ctx, room, Vec::new(), room_line);
    }

    // (4) The centre, and — when this was the last seat — cut K.1's trigger.
    //
    // A refusal here is the check-out's own, which cut K.1 has already announced
    // and rolled back: the pilot keeps their pod and their seat (both harmless,
    // and `EXIT` gives them up), so the line is reported and nothing is undone.
    crate::match_::slot_seated(ctx, match_id, actor, tank)?;
    Ok(())
}

/// **`BOARD` in a running match: take a dropped tank back** (16b cut Y.3; 16c
/// `N4`; doc 07 §1.3 "Reclaiming control"; doc 16 §3.8 "Disconnects").
///
/// [`board`] reaches this for a pilot whose slot [`lobby::Slot::is_dropped`] —
/// the linkdead grace ran out, or they quit — once
/// `[tacsim.match].reclaim_min_ms` has passed since the drop, with the record
/// still `Running`. It is F.1's seat path made **idempotent against what the
/// drop left behind**: the body still in the pod, the cockpit billet still this
/// actor's on the tank, the focus relinquished and the registry binding gone.
///
/// 0. **the room**: the body is inside its own pod — where a drop leaves it —
///    or standing in the slot's team room, where a restart's boot sweep or an
///    `@arena/reset` would have put it; anywhere else is
///    [`CockpitError::OutsideTeamRoom`];
/// 1. **the station**, declared exactly as [`board`] declares it (idempotent);
/// 2. **the body**, placed in the pod only when it is not already there — and
///    remembered, because only a body this verb moved is put back out;
/// 3. **the seat**, activated exactly as [`board`] activates it: the billet is
///    still this actor's, and the activation re-takes it and claims the focus.
///    On a refusal the body goes back out if step 2 moved it, and the pilot
///    reads [`CockpitError::Seat`];
/// 4. the actor's links, restated;
/// 5. **the arena**: the registry binds the pilot again and
///    [`crate::arena_bridge::reseat`] sends `Control { Live }` on every channel
///    at a **new** epoch, exactly as a revival inside the grace does (doc 07
///    §1.3 step 2) — the arena puts them back on its frame roster and takes the
///    tank from its mind (the reflex controller until 17b cut M4.3) at its next
///    pulse, where it stands;
/// 6. **the centre**: [`lobby::slot_reclaimed`] — the slot back in its cockpit,
///    the unattended clock cleared, the leads resettled (the heir keeps the
///    lead: 16c Q42's default);
/// 7. **the lines**: to the pilot that the tank is theirs again where it stands
///    — the last thing this verb says to them before the pod's interior (16b cut
///    Y.4 appends what happened meanwhile after it) — and to the team room, only
///    when step 2 moved the body, that they climbed back in.
///
/// No confirmation step (16c's default: the tank comes back where it is), and
/// no `slot_seated`: that is `Boarding`'s trigger, and the match is running.
pub(super) fn reclaim_seat(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    pod: ObjId,
    team: crate::arena::state::PartyId,
    index: u8,
) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // (0) The room: the pilot's own pod, or their side's team room.
    let in_pod = pod_of_body(ctx.w, actor) == Some(pod);
    let in_team_room = pod_of_body(ctx.w, actor).is_none()
        && centre::where_in_centre(ctx.w, actor) == Some(CentreRoom::Team(team));
    if !in_pod && !in_team_room {
        return Err(CockpitError::OutsideTeamRoom);
    }
    let tank = centre::tank_of(ctx.w, pod).ok_or(SimError::NoUnit {
        team: team.as_str(),
        index,
    })?;
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);

    // (1) The station, as `board` declares it.
    mush_sim::declare_station(
        ctx,
        tank,
        &station,
        STATION_ROLE,
        &cockpit_channels(),
        Some(pod),
        STATION_PRIORITY,
    )
    .map_err(|error| CockpitError::Station(error.to_string()))?;

    // (2) The body, only when it is not in the pod already.
    let moved = !in_pod;
    if moved {
        mush_sim::place_interior(ctx, actor, pod, None)
            .map_err(|error| CockpitError::Hatch(error.to_string()))?;
    }

    // (3) The seat. The billet is still this actor's after a drop, and the
    // activation re-takes it; a body this verb moved goes back out on a refusal,
    // and one it found in the pod stays where the drop left it.
    if let Err(error) = mush_sim::request_seat_activation(
        ctx,
        &mush_sim::SeatRequest::Activate {
            unit: tank,
            station: station.clone(),
            npc: actor,
            granted: cockpit_channels(),
            fallback: None,
        },
    ) {
        if moved && let Some(room) = room {
            put_out(ctx, actor, room);
        }
        return Err(CockpitError::Seat(error.to_string()));
    }
    // (4) As `board`: the idempotent restatement.
    mush_sim::links::refresh_actor_links(ctx.w, actor);

    // (5) The arena: bound again, then `Live` at a new epoch on every channel.
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.bind_pilot(actor, match_id, tank);
    }
    crate::arena_bridge::reseat(ctx, tank, actor);

    // (6) The centre.
    lobby::slot_reclaimed(ctx, match_id, actor, tank)?;

    // (7) The lines.
    let pilot_line = format!(
        "You climb back into {}: {} is yours again where it stands.",
        display_name(ctx.w, pod),
        display_name(ctx.w, tank)
    );
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, pilot_line);
    if moved && let Some(room) = room {
        let room_line = format!(
            "{} climbs back into {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, pod)
        );
        tell_room(ctx, room, Vec::new(), room_line);
    }
    ctx.show_room(actor, pod);
    Ok(())
}

// ---------------------------------------------------------------------------
// K2: `EXIT`
// ---------------------------------------------------------------------------

/// **What an `EXIT` during `Running` says of the tank, after its name** (16b
/// cut F.1): the leave is final — `You climb out of the pod. RA is on its own
/// for the rest of the match; you cannot take it back.` One constant, so
/// `help minds`' gate test reads the words `EXIT` says (17b cut M5.6's review,
/// R24: it grepped this file, and its own needle matched itself).
pub(crate) const EXIT_FINAL: &str =
    "is on its own for the rest of the match; you cannot take it back.";

/// **`EXIT`** — climb out (doc 16 §3.7).
///
/// The order is the reverse of [`board`]'s, with the authority first:
///
/// 1. `Control { Released }` on all three channels, which raises the arena's
///    epoch and **purges whatever that seat had queued** — the reason a pilot who
///    types `FIRE` and `EXIT` in one burst fires nothing;
/// 2. the seat, through the same one seam that took it, which clears the
///    narrowing, relinquishes the focus and fires the station's declared fallback
///    if it has one;
/// 3. the body, back into the team room;
/// 4. the centre, when the match is still `Boarding`, so the trigger stops
///    counting a pod nobody is in.
///
/// # Errors
///
/// [`CockpitError`], as the line the pilot sees.
pub fn exit(ctx: &mut Ctx<'_>) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // **T11's guardrail** (16b cut K.2; `ProxyAct::Abandonment`'s station-lane
    // face, restored by this cut). A seat is tactical discretion: a driver lent
    // somebody else's crew member may not climb out of their unit and leave it
    // idling in a live match. A pilot in their own tank is not a proxy driver, so
    // the predicate stands aside for them and this costs them nothing.
    mush_sim::seat_restriction(ctx.w, actor, mush_sim::ProxyAct::Abandonment)
        .map_err(CockpitError::Guarded)?;
    let pod = pod_of_body(ctx.w, actor).ok_or(CockpitError::NotInAPod)?;
    let room = ctx.w.get(pod).and_then(|object| object.location);
    let station = station_id().ok_or(CockpitError::Unavailable)?;
    let seats: Vec<ObjId> = seats_of(ctx.w, actor)
        .into_iter()
        .filter(|(_, held)| *held == station)
        .map(|(unit, _)| unit)
        .collect();
    // 16b cut F.1: whether this is the one-way leave of doc 16 §11's N.1
    // amendment, read before the seat goes.
    let running = lobby::centre(ctx.w).is_some_and(|centre| centre.state == MatchState::Running);

    // (1) and (2): the arena hears first, then the seat is stood down and the
    // billet given back — all of it through [`hand_seat_back`], the one way this
    // crate lets a cockpit seat go.
    for unit in &seats {
        hand_seat_back(ctx, *unit, &station, actor, true);
    }
    // **16b cut Y.3**: and the leave is final, whatever came before it. A pilot
    // who dropped, reconnected inside the pod and typed `EXIT` still held the
    // billet (so the loop above handed it back), but their slot was already
    // `left` and `slot_left` changed nothing — the drop's stamp would have
    // survived and the sentence below, *you cannot take it back*, been false.
    if running {
        lobby::slot_exited(ctx, actor);
    }
    // (3) The body. The pod is an unanchored interior space, so there is no
    // domain to `egress` onto: the placement is released where it stands and the
    // character walks back into the room the pod is in.
    if let Some(room) = room {
        put_out(ctx, actor, room);
    }
    // **16b cut F.1**: during `Running` this is final (doc 16 §11's N.1
    // amendment: *a released seat hands the tank to the reflex controller and the
    // pilot cannot reclaim it* — to a unit mind since 17b cut M4.3), and the
    // pilot is told so in the same breath
    // rather than discovering it when `BOARD` refuses. No confirmation step: the
    // verb does what it says, and what it says is now the whole truth.
    let line = match (running, seats.first()) {
        (true, Some(unit)) => format!(
            "You climb out of {}. {} {EXIT_FINAL}",
            display_name(ctx.w, pod),
            display_name(ctx.w, *unit)
        ),
        _ => format!("You climb out of {}.", display_name(ctx.w, pod)),
    };
    // The slot itself was marked `left` by [`hand_seat_back`]'s own `Released`
    // above, in [`crate::arena_bridge::send_control`], where every route to a
    // released seat passes — not here, because `EXIT` is only one of them.
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    if let Some(room) = room {
        let text = format!(
            "{} climbs out of {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, pod)
        );
        tell_room(ctx, room, vec![actor], text);
        // **16b cut B.4** (`B-38`): and where they are standing, AFTER the
        // sentence that explains the move — never inside `put_out`, or the
        // room would land before "You climb out".
        ctx.show_room(actor, room);
    }

    // (4) The centre, while it is still boarding.
    let boarding = lobby::centre(ctx.w).and_then(|centre| {
        (centre.state == MatchState::Boarding && centre.slot_of_pilot(actor).is_some())
            .then_some(centre.match_id)?
    });
    if let Some(match_id) = boarding {
        crate::match_::slot_vacated(ctx, match_id, actor)?;
    }
    Ok(())
}

/// Take one character out of the pod it is placed inside and stand it in `room`.
///
/// Two steps, because the pod is **unanchored**: the placement is released where
/// it is (`keep`, so nothing is projected to the recovery room) and the body is
/// then moved into the room the pod stands in. An unplaced object projects
/// nowhere, so boot's own projection reconcile leaves it exactly here.
pub(super) fn put_out(ctx: &mut Ctx<'_>, actor: ObjId, room: ObjId) {
    if let Err(error) = mush_sim::unplace(ctx, actor, "left a simulator pod") {
        tracing::debug!(%actor, %error, "tacsim: a pilot's pod placement did not release");
    }
    let from = ctx.w.get(actor).and_then(|object| object.location);
    if from == Some(room) {
        return;
    }
    if let Err(error) = ctx.w.move_to(actor, room) {
        tracing::error!(%actor, %room, %error, "tacsim: a pilot could not leave their pod");
        return;
    }
    ctx.emit_world_event(&mush_core::WorldEvent::Moved {
        who: actor,
        from,
        to: room,
    });
}

/// **What one sweep gave back** (16b cut F.1).
///
/// Two lists rather than two counts, because the caller that reports to a
/// staffer wants to name what it found and the caller that decides whether
/// anything changed wants to ask one question.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EjectReport {
    /// The bodies taken out of a pod, in pod order.
    pub bodies: Vec<ObjId>,
    /// The cockpit billets given back, as `(tank, pilot)`, in pod order.
    pub seats: Vec<(ObjId, ObjId)>,
}

impl EjectReport {
    /// Whether the sweep found nothing at all — which is what an idle centre and
    /// a second `@arena/reset` both answer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.seats.is_empty()
    }

    /// Fold one pod's findings into the whole.
    pub(super) fn merge(&mut self, other: EjectReport) {
        self.bodies.extend(other.bodies);
        self.seats.extend(other.seats);
    }
}

/// **Give one cockpit seat back** (16b cut F.1) — the one way, wherever a seat
/// has to be let go.
///
/// Three steps, and the order is `EXIT`'s because `EXIT`'s was right, and since
/// 16b cut Z.4 a fourth after them:
///
/// 1. `Control { Released }` on all three channels, which raises the arena's
///    epoch and **purges whatever that seat had queued** — the reason a pilot who
///    types `FIRE` and `EXIT` in one burst fires nothing — and, since this cut,
///    takes the pilot off the arena's frame roster and off the registry's pilot
///    binding ([`send_control`]);
/// 2. the seat, stood down through the one seam that took it (the narrowing
///    cleared, the focus relinquished, the station's fallback fired);
/// 3. the **billet**, given back. A stand-down deliberately leaves the billet
///    where it is, which is right for an NPC's own station and wrong for a pod
///    lent for one match: the billet *is* the loan, and one left behind is the
///    next pilot's `StationHeld`;
/// 4. **the side's net** (16b cut Z.4), left through
///    [`crate::match_::net::leave_pilot`]: the pilot joined it with the match
///    and leaves it with the seat, and reads comsys's own `You have been
///    removed from channel RedNet.` once.
///
/// `by_pilot` is whose choice it was. `Release` and `Revoke` reach one
/// `stand_down` in `mush-sim`; the variant is the statement, and a sweep after an
/// abort or at boot must use the one that does not require the absent driver's
/// account to still resolve.
///
/// Answers whether the billet went, which is the half that strands a pod.
pub(crate) fn hand_seat_back(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    station: &mush_sim::StationId,
    actor: ObjId,
    by_pilot: bool,
) -> bool {
    for channel in Channel::ALL {
        if let Err(error) = send_control(ctx, unit, channel, ControlState::Released, Some(actor)) {
            tracing::debug!(
                %unit,
                channel = channel.as_str(),
                %error,
                "tacsim: a released cockpit seat could not be announced to the arena"
            );
        }
    }
    let request = if by_pilot {
        mush_sim::SeatRequest::Release {
            unit,
            station: station.clone(),
            npc: actor,
        }
    } else {
        mush_sim::SeatRequest::Revoke {
            unit,
            station: station.clone(),
            npc: actor,
        }
    };
    if let Err(error) = mush_sim::request_seat_activation(ctx, &request) {
        tracing::debug!(
            %unit,
            %error,
            "tacsim: a cockpit seat did not stand down"
        );
    }
    let vacated = match mush_sim::vacate_station(ctx, unit, station, actor) {
        // `false` is "there was no billet to give back", which is the answer a
        // second sweep gets and is not a failure.
        Ok(vacated) => vacated,
        Err(error) => {
            tracing::error!(
                %unit,
                %error,
                "tacsim: a cockpit billet could not be given back; the next pilot of that pod \
                 will be refused and staff must @spatial/unassign it"
            );
            false
        }
    };
    // 4. **16b cut Z.4 (16c `N8`): the radio goes with the seat**, whatever the
    //    billet answered. Both side nets, idempotent — a pilot on neither (a
    //    staff-seated one, a second sweep) reads nothing — so this one line
    //    covers `EXIT`, a seated `SIM LEAVE`, the check-in and the boot's sweep.
    //    A drop past the grace never reaches here, and the pilot stays on their
    //    net for the reclaim.
    crate::match_::net::leave_pilot(ctx, actor);
    vacated
}

/// Every actor holding the cockpit billet on `unit`, given it back (16b cut
/// F.1).
pub(super) fn hand_back_seats_of_unit(ctx: &mut Ctx<'_>, unit: ObjId) -> EjectReport {
    let Some(station) = station_id() else {
        return EjectReport::default();
    };
    let held: Vec<ObjId> = ctx
        .w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&unit))
        .map(|host| {
            host.seats
                .iter()
                .filter(|(at, _)| **at == station)
                .map(|(_, seat)| seat.actor)
                .collect()
        })
        .unwrap_or_default();
    let mut report = EjectReport::default();
    for actor in held {
        if hand_seat_back(ctx, unit, &station, actor, false) {
            report.seats.push((unit, actor));
        }
    }
    report
}

/// Every cockpit billet one actor holds, given back (16b cut F.1).
pub(super) fn hand_back_seats_of_actor(ctx: &mut Ctx<'_>, actor: ObjId) -> EjectReport {
    let Some(station) = station_id() else {
        return EjectReport::default();
    };
    let units: Vec<ObjId> = seats_of(ctx.w, actor)
        .into_iter()
        .filter(|(_, held)| *held == station)
        .map(|(unit, _)| unit)
        .collect();
    let mut report = EjectReport::default();
    for unit in units {
        if hand_seat_back(ctx, unit, &station, actor, false) {
            report.seats.push((unit, actor));
        }
    }
    report
}

/// **Open every pod, give back every cockpit billet, and stand whoever is
/// inside in the team room** (16b cut F.1; doc 16 §3.8: *seats, leases, focus and
/// pods released atomically*).
///
/// This replaces cut K.2's `disembark_all`, which did the **body** half alone.
/// That was the bug the 2026-09-18 review found in four shapes at once: an abort
/// during `Boarding`, the board deadline emptying a side, a `SIM LEAVE` by a
/// seated pilot and a restart mid-match all left the persisted `assignment:<tank>`
/// billet behind, and `@arena/reset` then moved the body out without it — after
/// which the pilot could not even `EXIT`, and the next pilot of that pod was
/// refused `StationHeld` forever. A sweep that opens a hatch and leaves the seat
/// held is not a release.
///
/// Keyed on the **pods** and on their tanks rather than on the centre's slots, so
/// it is equally the repair a staff `@arena/reset` performs on a centre some
/// earlier failure left with somebody aboard, and the boot pass's answer to a
/// restart ([`eject_all_at_boot`], its silent sibling). Idempotent, and fallible
/// per pod without failing the others.
pub(crate) fn eject_all(ctx: &mut Ctx<'_>) -> EjectReport {
    sweep_pods(ctx, true, &BTreeSet::new())
}

/// **[`eject_all`] at boot** (16b cut U.4, 16c `B-37`), from the module's
/// `on_all_loaded`: the same bodies moved and the same billets given back, and
/// **nothing said**.
///
/// Boot output reaches nobody — the drivers drop it, because no session is
/// attached yet — and on the split server a pilot's session survives the restart
/// of the server half and is still waiting to be told why they are standing in
/// the team room. So each `(body, pod)` it opens is recorded on the match book
/// ([`crate::match_::MatchBook::defer_hatch`]) and the module's boarding tick
/// says the hatch line and the room once that body's session is attached, for
/// at most [`crate::match_::HATCH_HOLD_S`] ([`crate::match_::sim`]'s
/// `board_tick`).
///
/// **16b cut AF.3: a resumed match is exempt.** When the boot resumed the
/// centre's match (the centre `Running` on a match id, which only the resume
/// leaves it at boot), its slots' pods and their tanks are left alone: the
/// bodies stay inside, no hatch line is owed, and the billets stay on the
/// tanks — Y.3's reclaim re-runs the seat path idempotently from the pod with
/// the billet held. Every other pod is swept as before, and a crash during
/// `Boarding` (no marker, no snapshot, nothing resumed) still opens the hatch.
pub(crate) fn eject_all_at_boot(ctx: &mut Ctx<'_>) -> EjectReport {
    let keep: BTreeSet<ObjId> = lobby::centre(ctx.w)
        .filter(|centre| centre.state == MatchState::Running && centre.match_id.is_some())
        .map(|centre| {
            centre
                .slots
                .iter()
                .filter_map(|slot| slot.pod)
                .flat_map(|pod| [Some(pod), centre::tank_of(ctx.w, pod)])
                .chain(centre.slots.iter().map(|slot| slot.unit))
                .flatten()
                .collect()
        })
        .unwrap_or_default();
    sweep_pods(ctx, false, &keep)
}

/// Both sweeps' one body. `told` says the hatch line and the room at once; its
/// absence defers them to the boarding tick. `keep` names pods and tanks the
/// sweep leaves exactly as they are (16b cut AF.3: a resumed match's).
pub(super) fn sweep_pods(ctx: &mut Ctx<'_>, told: bool, keep: &BTreeSet<ObjId>) -> EjectReport {
    let pods: Vec<(ObjId, Option<ObjId>, Option<ObjId>)> = centre::POD_ENTRIES
        .iter()
        .filter_map(|(team, index, _)| centre::pod_object(ctx.w, *team, *index))
        .map(|pod| {
            (
                pod,
                ctx.w.get(pod).and_then(|object| object.location),
                centre::tank_of(ctx.w, pod),
            )
        })
        .collect();
    let mut report = EjectReport::default();
    for (pod, room, tank) in pods {
        // The billets on the pod's own tank, whether or not anybody is still
        // sitting in the pod: the two halves are stranded independently.
        if let Some(tank) = tank.filter(|tank| !keep.contains(tank)) {
            report.merge(hand_back_seats_of_unit(ctx, tank));
        }
        if keep.contains(&pod) {
            continue;
        }
        let Some(room) = room else {
            continue;
        };
        let aboard: Vec<ObjId> = ctx
            .w
            .get(pod)
            .map(|object| object.contents.clone())
            .unwrap_or_default();
        for body in aboard {
            if ctx
                .w
                .get(body)
                .is_none_or(|object| object.kind != mush_core::ObjKind::Player)
            {
                continue;
            }
            // A pilot who wandered into somebody else's pod still holds their own
            // tank's billet; the pod's tank alone would not find it.
            report.merge(hand_back_seats_of_actor(ctx, body));
            put_out(ctx, body, room);
            if told {
                tell_hatch_open(ctx, body, pod, room);
            } else if let Some(book) = ctx.w.module_state_mut::<crate::match_::MatchBook>() {
                book.defer_hatch(body, pod);
            }
            report.bodies.push(body);
        }
    }
    report
}

/// **Tell one body its pod is open**: the hatch line, then the room it is
/// standing in (16b cut B.4, `B-38`: after the line that explains it).
///
/// [`eject_all`] says it at once; the boot's sweep defers it, and the module's
/// first tick says it here (16b cut U.4) — with the room the body stands in by
/// then, which is the room the sweep put it in unless it has walked since.
pub(crate) fn tell_hatch_open(ctx: &mut Ctx<'_>, body: ObjId, pod: ObjId, room: ObjId) {
    let line = format!("The hatch of {} swings open.", display_name(ctx.w, pod));
    ctx.out.sys_to_actor(body, NOTICE_CATEGORY, line);
    ctx.show_room(body, room);
}

/// **The per-pilot form of [`eject_all`]** (16b cut F.1): one pilot's cockpit
/// billet given back and one pilot's body put out of whatever pod it is in.
///
/// `SIM LEAVE` by a seated pilot is what wants it — dropping the slot while the
/// pilot sits in the pod holding the tank's billet is what made the pod
/// unboardable for the next crew — and so is any other place that has one person
/// in hand rather than a centre.
pub(crate) fn eject_pilot(ctx: &mut Ctx<'_>, actor: ObjId) -> EjectReport {
    let mut report = hand_back_seats_of_actor(ctx, actor);
    let Some(pod) = pod_of_body(ctx.w, actor) else {
        return report;
    };
    let Some(room) = ctx.w.get(pod).and_then(|object| object.location) else {
        return report;
    };
    put_out(ctx, actor, room);
    // 16b cut B.4 (`B-38`): as `eject_all`, the room after the hatch line.
    tell_hatch_open(ctx, actor, pod, room);
    report.bodies.push(actor);
    report
}

// ---------------------------------------------------------------------------
// K3: the cockpit verbs (twelve at K3; `ORDER`, the thirteenth, is below
// under "Commanding the machines"; `EMCON`, the fourteenth, follows `STARTUP`,
// `WATCH`, the fifteenth, follows `EMCON`, `DRIVE`, the sixteenth, follows
// `WATCH`, and `MIND`, the seventeenth, follows `DRIVE`)
// ---------------------------------------------------------------------------

/// `STARTUP` — bring power up (doc 16 §3.3).
///
/// **16b cut D.1: it reads the mirror first, and it stamps a hint.** It used to
/// forward the intent whatever the power was and say `Startup sequence begun;
/// power comes online shortly.` — so a second `STARTUP` on a tank that was
/// already running said it again and the pilot waited for a `Power online.` that
/// had already been said and would never come again. Three answers now:
///
/// * [`Power::Online`]: `Power is already online.`, and nothing is sent;
/// * [`Power::Starting`]: [`POWER_STARTING`], and nothing is sent — the arena is
///   already doing it, whether the mirror knows that from a frame or from the
///   hint this function stamped;
/// * [`Power::Off`]: the intent is forwarded as before, the hint is stamped, and
///   the pilot reads the sentence they always read.
///
/// The hint expires one second past a whole `Quiet` period, which is the longest
/// a frame can be away; a lost `STARTUP` therefore tells the pilot to type it
/// again, which by then is the true advice. See
/// [`crate::arena_bridge::ArenaRegistry::starting_hints`] for the four ways it
/// ends.
pub(super) fn startup(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): before the power, so a pilot in a wreck is never told
    // to wait for a startup that is not coming.
    require_alive(ctx.w, seat.driver)?;
    match power_of(ctx.w, seat.driver) {
        Power::Online => {
            ctx.out
                .sys_to_actor(actor, NOTICE_CATEGORY, "Power is already online.");
            return Ok(());
        }
        Power::Starting => {
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, POWER_STARTING);
            return Ok(());
        }
        Power::Off => {}
    }
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Startup)?;
    // The optimism is stamped only once the order is away: a refused forward is
    // not a startup in progress.
    let until_s = hint_until_s(ctx.w);
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.hint_starting(seat.driver, until_s);
    }
    ctx.out.sys_to_actor(
        actor,
        NOTICE_CATEGORY,
        "Startup sequence begun; power comes online shortly.",
    );
    Ok(())
}

/// **How long the `Starting` hint stands** when the configuration cannot be read
/// (16b cut D.1), seconds: one whole `Quiet` period of the shipped default, plus
/// one.
pub(super) const HINT_FALLBACK_S: i64 = 6;

/// **The world second a hint stamped now expires at** (16b cut D.1; shared by
/// `EMCON`'s hint since 16b cut O.2): now plus one whole `Quiet` period in
/// seconds, rounded up, plus one — the longest a frame can be away, and one
/// second more. [`HINT_FALLBACK_S`] when the configuration cannot be read.
pub(super) fn hint_until_s(w: &World) -> i64 {
    let stands_s = crate::match_::lobby::configs(w).map_or(HINT_FALLBACK_S, |(arena_cfg, _, _)| {
        // Whole seconds, rounded up, plus one: the hint must outlast the
        // longest a frame can be away.
        let quiet_ms = i64::from(arena_cfg.tempo.quiet_ms).max(0);
        quiet_ms.saturating_add(999) / 1_000 + 1
    });
    w.now().saturating_add(stands_s)
}

/// **`STATUS`'s pod line** (16b cut O.2, 16c N12), printed after the receiver
/// line and only for a platform that carries a pod ([`Frame::ecm`]).
#[must_use]
pub const fn emcon_status_line(on: bool) -> &'static str {
    if on {
        "  emcon on: the pod radiates"
    } else {
        "  emcon off: the pod is silent"
    }
}

/// `EMCON [on|off]` — the jamming pod's switch (16b cut O.2; 16c N12).
///
/// **Bare**, it reads the held frame: `EMCON on: the pod radiates.` or `EMCON
/// off: the pod is silent.`, [`CockpitError::NoJammer`] for a tank with no pod,
/// and [`CockpitError::NoFrameYet`] before the first frame. **With a word**
/// (`on` or `off`, any case; anything else is the usage line) it is a discrete
/// order on the sensors channel:
///
/// * refused locally with [`CockpitError::NoJammer`] when the held frame says
///   the tank carries no pod, and with [`CockpitError::NoFrameYet`] before the
///   first frame — the switch cannot be known to exist, and the arena would
///   refuse it anyway if it does not;
/// * answered `EMCON is already on.` / `EMCON is already off.`, forwarding
///   nothing, when the switch is already where the order puts it — read off
///   **the bridge's hint first and the mirror second**
///   ([`crate::arena_bridge::ArenaRegistry::emcon_hint`]; the cut O plan's item
///   38). An intent is buffered to the next pulse and never steps the arena,
///   so the mirror is up to one `Quiet` period stale: without the hint `EMCON
///   off` then `EMCON on` inside one pulse would answer the second from a
///   mirror that still read `on`, and the tank would end silent while the
///   pilot was told otherwise. With it, both are forwarded with true replies,
///   and the arena resolves them in order to the state the pilot left;
/// * otherwise forwarded, the hint stamped with `STARTUP`'s expiry
///   ([`hint_until_s`]), and answered `EMCON coming on; the pod radiates from
///   the next pulse.` / `EMCON going off; the pod is silent from the next
///   pulse.`
///
/// The wreck comes first, as it does for every order (`require_alive`). Power
/// is not asked: the switch is a position, and the arena's jamming fold is
/// what reads power. What the arena makes of the order is said to the pilot's
/// own frame as an `emcon_on`/`emcon_off` notice, which names what switching
/// off costs.
pub(super) fn emcon(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    require_alive(ctx.w, seat.driver)?;
    let word = rest.trim().to_ascii_lowercase();
    let wanted = match word.as_str() {
        "" => None,
        "on" => Some(true),
        "off" => Some(false),
        _ => return Err(CockpitError::Usage("EMCON on|off.".to_owned())),
    };
    let held = last_frame(ctx.w, seat.driver).ok_or(CockpitError::NoFrameYet)?;
    let Some(mirror) = held.frame.ecm else {
        return Err(CockpitError::NoJammer);
    };
    let Some(on) = wanted else {
        let line = if mirror {
            "EMCON on: the pod radiates."
        } else {
            "EMCON off: the pod is silent."
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    };
    // The hint first, the mirror second (the cut O plan's item 38).
    let now = ctx.w.now();
    let current = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.emcon_hint(seat.driver, now))
        .unwrap_or(mirror);
    if current == on {
        let line = if on {
            "EMCON is already on."
        } else {
            "EMCON is already off."
        };
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        return Ok(());
    }
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Emcon { on })?;
    // Stamped only once the order is away, as `STARTUP`'s is.
    let until_s = hint_until_s(ctx.w);
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.hint_emcon(seat.driver, on, until_s);
    }
    let line = if on {
        "EMCON coming on; the pod radiates from the next pulse."
    } else {
        "EMCON going off; the pod is silent from the next pulse."
    };
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`WATCH ON`'s reply** (16b cut AP.3, 16c N19), said before the first block.
pub const WATCHING: &str = "Watching: your own line, your guns and your contacts are pushed as they change. \
     WATCH OFF stops it.";

/// **`WATCH OFF`'s reply** (16b cut AP.3).
pub const WATCH_OFF: &str = "Watch off.";

/// **Bare `WATCH`, while the switch is on** (16b cut AP.3).
pub const WATCH_IS_ON: &str = "Watch is on.";

/// **Bare `WATCH`, while it is off** — which is how every seat starts (16b cut
/// AP.3).
pub const WATCH_IS_OFF: &str = "Watch is off.";

/// **`WATCH`'s usage line** (16b cut AP.3), for any word but `on` and `off`.
pub const WATCH_USAGE: &str = "WATCH [ON|OFF]; bare WATCH says which.";

/// `WATCH [ON|OFF]` — the line-mode digest's switch (16b cut AP.3; 16c N19).
///
/// **Why it exists.** A line client is pull-only: the bridge pushes the
/// `arena.cockpit` keyframe on every changed frame and prints the frame's
/// transition notices, but `mush_ansi::render` prints a panel only in the grid
/// shape, so a line-mode pilot learns their speed, their guns' readiness and
/// their contacts' ranges only by typing `STATUS`, `WEAPONSTATUS` and
/// `CONTACTS` — and three reads in five said nothing new. With the switch on,
/// the bridge pushes the pilot [`watch_lines`]' block after each keyframe whose
/// block reads differently from the last one pushed
/// (`crate::arena_bridge::on_frames`). Off by default (16c N19's owner
/// default), so a pilot who never types it reads exactly what they read before.
///
/// * `WATCH ON` (any case) sets the flag
///   ([`ArenaRegistry::set_line_watch`]), says [`WATCHING`] and then — when the
///   panel has told the pilot anything — that frame's block at once, as ONE
///   `Sys` tagged [`NOTICE_SENTENCE_TAG`], built with the **aged** weapon rows
///   (`carried`, 16b cut AP.1) so a pilot who types `WATCH ON` and
///   `WEAPONSTATUS` in one breath reads one countdown, not two. The block is
///   recorded ([`ArenaRegistry::note_watch_block`]) so the next frame's
///   identical block is not said again;
/// * `WATCH OFF` clears it and forgets the last block, and says [`WATCH_OFF`];
/// * bare `WATCH` says [`WATCH_IS_ON`] or [`WATCH_IS_OFF`];
/// * any other word is [`WATCH_USAGE`].
///
/// The flag is keyed on [`Seat::driver`], the pilot the registry binds and the
/// one the frames go to, and it is the **seat's**: the registry drops it with
/// the seat (`unbind_pilot`) and with the match (`untrack`), so a pilot who
/// boards again types it again. Nothing crosses the seam and nothing is
/// written but the RAM row: it is a view, never an order, and a wreck may
/// watch.
pub(super) fn watch(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let on = match rest.trim().to_ascii_lowercase().as_str() {
        "" => {
            let watching = ctx
                .w
                .module_state::<ArenaRegistry>()
                .is_some_and(|registry| registry.line_watching(seat.driver));
            let line = if watching { WATCH_IS_ON } else { WATCH_IS_OFF };
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
            return Ok(());
        }
        "on" => true,
        "off" => false,
        _ => return Err(CockpitError::Usage(WATCH_USAGE.to_owned())),
    };
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.set_line_watch(seat.driver, on);
    }
    if !on {
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, WATCH_OFF);
        return Ok(());
    }
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, WATCHING);
    let Some(held) = last_frame(ctx.w, seat.driver) else {
        return Ok(());
    };
    // 16b cut AP.1: read once, at the top of the block, as `FIRE` and
    // `WEAPONSTATUS` read it — the aged rows, so this block and a
    // `WEAPONSTATUS` typed in the same breath say one countdown.
    let now = std::time::Instant::now();
    let weapons = carried(ctx.w, seat.unit, Some(&held), now);
    let block = watch_lines(&held.frame, &held.ranges, &weapons, pass_ms(ctx.w)).join("\n");
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        // Said whatever the last block was — the pilot asked — and recorded,
        // so the next frame's identical block is not said again.
        registry.note_watch_block(seat.driver, &block);
    }
    ctx.out.sys_to_actor(actor, NOTICE_SENTENCE_TAG, block);
    Ok(())
}

// ---------------------------------------------------------------------------
// `DRIVE`, the sixteenth selector (17b cut M1a.4; doc 17 §4 D-UM-10)
// ---------------------------------------------------------------------------

/// **`DRIVE`'s usage line** (17b cut M1a.4), for a bare `DRIVE`.
pub const DRIVE_USAGE: &str = "DRIVE <spot|x,y|HOLD|OFF>; STATUS names the nearest spot.";

/// **`DRIVE HOLD`'s reply** (17b cut M1a.4).
pub const DRIVE_HOLDING: &str = "Holding here.";

/// **`DRIVE OFF`'s reply** (17b cut M1a.4).
pub const DRIVE_RELEASED: &str = "DRIVE off; the helm is yours.";

/// **The one byte-identical refusal for any word `DRIVE` cannot drive to**
/// (17b cut M1a.4): not a spot's name or letter, not a cell, not `HOLD` or
/// `OFF`. Never an echo of the word, and a contact's designator is "not a
/// spot" like any other word — so nothing about a held or an unheld contact is
/// read or said (doc 16 §3.6's one-answer rule, read for a verb that takes no
/// contact at all).
pub const DRIVE_NOT_A_GOAL: &str = "DRIVE refused. That is not a spot, a cell, HOLD or OFF.";

/// **`DRIVE x,y` off the arena window** (17b cut M1a.4).
pub const DRIVE_OFF_WINDOW: &str = "DRIVE refused. That cell is not on the window.";

/// **What the motor says when a `DRIVE` arrives** (17b cut M1a.4), off
/// [`crate::arena::perception::DriveOutcome::Arrived`].
pub const DRIVE_ARRIVED: &str = "DRIVE: arrived.";

/// **What the motor says when no route reaches a `DRIVE`'s goal** (17b cut
/// M1a.4), off [`crate::arena::perception::DriveOutcome::NoRoute`]: the goal
/// is dropped and the tank holds where it is.
pub const DRIVE_NO_ROUTE: &str = "DRIVE: no route from here; holding.";

/// **The two integers of `DRIVE x,y` or `DRIVE x y`** (17b cut M1a.4), or
/// `None` for a word that is not exactly two unsigned integers — `12`, `12,`
/// and `12,34,5` are not cells. Wide enough that an integer off any window
/// reads as a cell off the window rather than as "not a cell".
pub(super) fn drive_cell(word: &str) -> Option<(u64, u64)> {
    let (x, y) = match word.split_once(',') {
        Some((x, y)) => (x.trim(), y.trim()),
        None => {
            let mut parts = word.split_whitespace();
            let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
                return None;
            };
            (x, y)
        }
    };
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// `DRIVE <spot|x,y|HOLD|OFF>` — hand the helm to the tank's own drive-by-wire
/// (17b cut M1a.4; doc 17 §4 D-UM-3 and D-UM-10 with the M1a plan's departure
/// (f): the word is `DRIVE` because core's baseline `goto`, with its aliases
/// `go` and `move`, holds that name and the command registry refuses a
/// collision; doc 16 §3.7).
///
/// The local pre-checks first, in `HEADING`'s order and words — the wreck
/// ([`CockpitError::Destroyed`]), the dead engine ([`CockpitError::Immobile`]),
/// the cold or starting tank ([`CockpitError::PowerNotOnline`],
/// [`CockpitError::PowerStarting`]) — because a goal is a helm order the arena
/// would refuse for the same three. Then the word, trimmed and case-folded,
/// read by [`drive_goal`]:
///
/// * nothing: [`DRIVE_USAGE`];
/// * `HOLD`: [`crate::arena::intents::MotorGoal::Hold`], [`DRIVE_HOLDING`] —
///   the tank stops and keeps no goal;
/// * `OFF`: [`crate::arena::intents::MotorGoal::Off`], [`DRIVE_RELEASED`] —
///   the goal is dropped and the tank keeps its last heading and speed;
/// * two integers `x,y` or `x y`: a cell of the arena window, `Steering for
///   cell 12,34.`; a cell off the window is [`DRIVE_OFF_WINDOW`];
/// * a key spot's whole name or its map letter, exactly as `RANGE` resolves
///   one ([`resolve_spot`]): its cell, `Steering for The Ford (A).`;
/// * anything else — a contact's designator among them — [`DRIVE_NOT_A_GOAL`],
///   byte for byte.
///
/// The goal is staged through the same forward every ordering verb uses, as a
/// continuous [`IntentKind::Goal`] on the helm channel, and counted on the
/// seat's rate by the arena. What the motor makes of it comes back as a notice
/// ([`DRIVE_ARRIVED`], [`DRIVE_NO_ROUTE`]); the route itself is never shown.
pub(super) fn drive(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    // As `HEADING`: the wreck, then the motion limit, then the power.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    let word = rest.trim().to_ascii_lowercase();
    // The ground is read only for a word that may name a place on it.
    let (spots, cols, rows) = if matches!(word.as_str(), "" | "hold" | "off") {
        (Vec::new(), 0, 0)
    } else {
        let held = last_frame(ctx.w, seat.driver);
        match_of(ctx.w, seat.driver, held.as_ref())
            .and_then(|match_id| {
                ctx.w
                    .module_state::<ArenaRegistry>()
                    .and_then(|registry| registry.arena_of(match_id))
                    .map(|ground| {
                        (
                            ground.key_spots.clone(),
                            ground.terrain.width(),
                            ground.terrain.height(),
                        )
                    })
            })
            .unwrap_or((Vec::new(), 0, 0))
    };
    let (goal, line) = drive_goal(&word, &spots, cols, rows)?;
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Goal(goal))?;
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **What one `DRIVE` word asks of the motor, and what the pilot is told**
/// (17b cut M1a.4): [`drive`]'s whole reading of the word, pure over the
/// ground's key spots and its window of `cols × rows` cells, so the goal a
/// spot's name or letter forwards is the cell the reply names. `word` is
/// already trimmed and case-folded; the arms are [`drive`]'s list. A missing
/// ground (`cols == rows == 0`, no spots) reads every cell as off the window
/// and every other word as not a goal.
pub(super) fn drive_goal(
    word: &str,
    spots: &[KeySpot],
    cols: u16,
    rows: u16,
) -> Result<(MotorGoal, String), CockpitError> {
    match word {
        "" => Err(CockpitError::Usage(DRIVE_USAGE.to_owned())),
        "hold" => Ok((MotorGoal::Hold, DRIVE_HOLDING.to_owned())),
        "off" => Ok((MotorGoal::Off, DRIVE_RELEASED.to_owned())),
        _ => {
            if let Some((x, y)) = drive_cell(word) {
                let on_window = (u16::try_from(x), u16::try_from(y));
                let (Ok(x), Ok(y)) = on_window else {
                    return Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()));
                };
                if x >= cols || y >= rows {
                    return Err(CockpitError::Refused(DRIVE_OFF_WINDOW.to_owned()));
                }
                return Ok((
                    MotorGoal::Cell { x, y },
                    format!("Steering for cell {x},{y}."),
                ));
            }
            let not_a_goal = || CockpitError::Refused(DRIVE_NOT_A_GOAL.to_owned());
            let (index, spot) = resolve_spot(spots, word).ok_or_else(not_a_goal)?;
            let letter = spot_letter(index).ok_or_else(not_a_goal)?;
            Ok((
                MotorGoal::Cell {
                    x: spot.x,
                    y: spot.y,
                },
                format!("Steering for {} ({letter}).", spot.name),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// `MIND`, the seventeenth selector (17b cut M5.4; doc 17 §12 "The `MIND` verb")
// ---------------------------------------------------------------------------

/// **`MIND`'s usage line** (17b cut M5.4), for a form it does not take — a
/// word that is not `PROFILE`, `RULES` or `TRACE`, `PROFILE` with no name,
/// `RULES` with a word after it.
pub const MIND_USAGE: &str = "MIND [PROFILE <name>|CARD|RULES [slot]|RULE <slot> <rule or OFF>|TRACE [1..64]]; bare MIND reads your sealed mind.";

/// **What `MIND` says of a hull no seal reached** (17b cut M5.4): the frame
/// carries no [`Frame::mind`]. Bare `MIND` and `MIND RULES` say it alone;
/// `MIND PROFILE` says it after `MIND refused.`.
pub const NO_SEAL: &str = "This hull carries no seal.";

/// **The block's channel line on an armed hull** (17b cut M5.4; doc 17 §12,
/// OQ-8): the pilot holds every channel, and the sealed mind waits.
pub const MIND_CHANNELS_ARMED: &str =
    "Channels: you hold helm, sensors and fire; the mind is dormant while you fly.";

/// **The block's channel line on a hull with no weapons station** (17b cut
/// M5.4; doc 17 §11's inert fire half, §22's italic line): the frame's own
/// rule — no weapon rows — as the keyframe's `channels` reads it.
pub const MIND_CHANNELS_UNARMED: &str = "Channels: you hold helm and sensors (no weapons station \
                                          on this hull); the mind is dormant while you fly.";

/// **The block's rules line** (17b cut M5.4; OQ-4, OQ-8): no rule is sealed
/// on any profile before the rule-language cut (its own, after 17b cut
/// M5.7's gate), and the pilot's own condition slots are `FIRE … WHEN`'s,
/// read by `WEAPONSTATUS`.
pub const MIND_RULES_LINE: &str = "Rules: none sealed; your own armings are under WEAPONSTATUS.";

/// The same refusal for an unauthorized or unavailable trace.
pub const MIND_NO_TRACE: &str = "No mind trace is available.";

/// The verb word every `MIND PROFILE` refusal opens with, the intent's own
/// ([`IntentKind::as_str`] of [`IntentKind::Profile`]) — so the cockpit's
/// pre-checks and the arena's `profile` refusal compose one prefix through
/// [`order_refused`].
pub(super) fn profile_order() -> &'static str {
    IntentKind::Profile { index: 0 }.as_str()
}

/// **The arena millisecond the cockpit reads a mind's time against** (17b
/// cut M5.4; the M5 design note's M5.4 items 4 and 5: "the mirror's arena
/// clock"): [`crate::arena_bridge::MatchMirror::now_ms`], the arena's own
/// clock as the last `Frames` message reported it — moved every arena
/// second whether or not anybody was sent a frame — never the held frame's
/// `t_ms`, which a quiet hull's frame leaves stale. (The mirror's
/// `arena_ms`, the field the note names, is the last closed ledger chunk's
/// stamp — nought for a match's first minutes — so the countdown reads the
/// clock the note means; declared.) The held frame's own
/// [`CockpitFrame::arena_ms`] where the registry no longer mirrors the match;
/// never below it.
pub(super) fn mind_now_ms(w: &World, held: &CockpitFrame) -> i64 {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.mirror(held.match_id))
        .map_or(held.arena_ms, |mirror| mirror.now_ms)
        .max(held.arena_ms)
}

/// **The whole seconds until the next switch is allowed** (17b cut M5.4),
/// rounded up as every countdown a pilot reads is, read against `now_ms` (the
/// mirror's arena clock, `mind_now_ms`); `None` when a switch is allowed
/// now — before any switch, or once [`FrameMind::switch_at_ms`] is past.
#[must_use]
pub fn next_switch_s(mind: &FrameMind, now_ms: i64) -> Option<i64> {
    let left = mind.switch_at_ms?.saturating_sub(now_ms);
    (left > 0).then(|| left.saturating_add(999) / 1_000)
}

/// **`MIND`'s first line** (17b cut M5.4; doc 17 §12; the M5 design note's
/// M5.4 item 4): the mind, that it is sealed for this sortie, and the two
/// classes the frame reads LIVE ([`FrameMind`]) —
/// `Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on
/// a Class II core.` A mind that is not Standard Doctrine reads `your own
/// mind` (the arena keeps no other mind's name); a row dropped to the ROM
/// reads `Mind: Standard Doctrine, off for the match: the hull is on the
/// ROM.`, the words of `crate::arena_bridge`'s `ROM_TAIL`. A live row whose
/// mind class is 0 is the ROM's too (`Arena::collect_think`'s rule: no core,
/// a core at Class 0, or a seal capped at Class 0 hands every think to the
/// ROM), so "it thinks at Class 0" would be false of it: a hull whose core
/// is gone or reads Class 0 — `target_mk1`, which a pilot may field, has
/// none — reads `Mind: Standard Doctrine, sealed for this sortie; with no
/// core above Class 0, the hull is on the ROM.`, and a hull sealed at Class
/// 0 on a real core (`[tacsim.minds].fill_class = "0"`, a lawful setting,
/// seals every seated hull there) reads `Mind: Standard Doctrine, sealed for
/// this sortie at Class 0 on a Class II core: the hull is on the ROM.` (17b
/// cut M5.4: both are keyed on the class the ROM rule reads, not on the
/// core's alone).
#[must_use]
pub fn mind_head_line(mind: &FrameMind) -> String {
    use crate::arena::core::CoreClass;
    let name = mind.name.as_str();
    if mind.off {
        return format!("Mind: {name}, off for the match: the hull is on the ROM.");
    }
    if mind.class == CoreClass::Zero {
        if mind.core_class == CoreClass::Zero {
            return format!(
                "Mind: {name}, sealed for this sortie; with no core above Class 0, the hull is \
                 on the ROM."
            );
        }
        return format!(
            "Mind: {name}, sealed for this sortie at Class 0 on a Class {} core: the hull is on \
             the ROM.",
            mind.core_class.as_str()
        );
    }
    format!(
        "Mind: {name}, sealed for this sortie; it thinks at Class {} on a Class {} core.",
        mind.class.as_str(),
        mind.core_class.as_str()
    )
}

/// **`MIND`'s profile line** (17b cut M5.4; doc 17 §11, §12; OQ-3): the
/// profile the mind will fly, its place in the sealed set and the others,
/// and the switch —
/// `Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in
/// ten seconds.` (`MIND PROFILE <name>` past two profiles), or, inside ten
/// seconds of the last switch, `…; one switch in ten seconds, the next in 6
/// s.` — the seconds read against `now_ms`, the mirror's arena clock
/// (`mind_now_ms`). A set of one, every hull nobody chose a mind for, reads
/// `Profile: Line, 1 of 1; no other is sealed to switch to.`
#[must_use]
pub fn mind_profile_line(mind: &FrameMind, now_ms: i64) -> String {
    let names: Vec<&str> = mind.profiles.names().collect();
    // `ProfileSet::card`'s rule for an index past the set: the Line card.
    let active = mind.profiles.name(mind.active).unwrap_or("Line");
    let head = format!(
        "Profile: {active}, {} of {}",
        u16::from(mind.active).saturating_add(1),
        names.len()
    );
    let others: Vec<&str> = names
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != usize::from(mind.active))
        .map(|(_, name)| *name)
        .collect();
    if others.is_empty() {
        return format!("{head}; no other is sealed to switch to.");
    }
    let also = format!("{head} (also {})", lobby::listed(&others, "and"));
    if let Some(secs) = next_switch_s(mind, now_ms) {
        return format!("{also}; one switch in ten seconds, the next in {secs} s.");
    }
    let target = match others.as_slice() {
        [one] => one,
        _ => "<name>",
    };
    format!("{also}; MIND PROFILE {target} switches, once in ten seconds.")
}

/// **Bare `MIND`'s block** (17b cut M5.4; doc 17 §12 "The `MIND` verb"; the
/// M5 design note's M5.4 item 4; OQ-8): four lines, at most seven, each under
/// 120 columns at the widest preset pair —
///
/// ```text
/// Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II core.
/// Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds.
/// Channels: you hold helm, sensors and fire; the mind is dormant while you fly.
/// Rules: none sealed; your own armings are under WEAPONSTATUS.
/// ```
///
/// [`mind_head_line`], [`mind_profile_line`] (its time read against `now_ms`,
/// the mirror's arena clock), [`MIND_CHANNELS_ARMED`] or
/// [`MIND_CHANNELS_UNARMED`] by `armed` (the frame's own rule: it carries a
/// weapon row), and [`MIND_RULES_LINE`]. No ops figure, no seal id, no card.
#[must_use]
pub fn mind_lines(mind: &FrameMind, armed: bool, now_ms: i64) -> Vec<String> {
    let channels = if armed {
        MIND_CHANNELS_ARMED
    } else {
        MIND_CHANNELS_UNARMED
    };
    let count = mind.card.rules().map_or(0, <[_]>::len);
    let rules = if count == 0 {
        MIND_RULES_LINE.to_owned()
    } else {
        format!("Rules: {count} sealed on the active profile; MIND RULES reads their full terms.")
    };
    [
        mind_head_line(mind),
        mind_profile_line(mind, now_ms),
        channels.to_owned(),
        rules,
    ]
    .iter()
    .flat_map(|line| wrap_rule_words(line))
    .collect()
}

/// **`MIND RULES`' line** (17b cut M5.4; OQ-4): `No rules are sealed on
/// Sniper or Line; your own armings are under WEAPONSTATUS.`
#[must_use]
pub fn mind_rules_line(mind: &FrameMind) -> String {
    let count = mind.card.rules().map_or(0, <[_]>::len);
    if count > 0 {
        return format!(
            "{count} rules are sealed on the active profile; MIND RULES reads their full terms."
        );
    }
    let names: Vec<&str> = mind.profiles.names().collect();
    format!(
        "No rules are sealed on {}; your own armings are under WEAPONSTATUS.",
        lobby::listed(&names, "or")
    )
}

/// **What a `MIND PROFILE <word>` asks of a sealed mind** (17b cut M5.4; the
/// M5 design note's M5.4 item 5): the index to stage and the profile's
/// name, or the half of the refusal after `MIND refused.` — read off the
/// held frame's [`FrameMind`] and `now_ms`, the mirror's arena clock
/// (`mind_now_ms`), in this order:
///
/// * a word that names no sealed profile: `Hunter is not sealed on this
///   mind; it carries Sniper and Line.` (the preset's own name, never an
///   echo of the word), or for a word that is no preset at all `No profile
///   of that name is sealed on this mind; it carries Sniper and Line.`;
/// * the profile already flown: `Sniper is already the profile.`;
/// * inside ten seconds of the last switch: `One switch in ten seconds; the
///   next in 6 s.`
///
/// No power test (a cold hull switches, 17b cut M5.2). The arena checks the
/// same three, and the seal, at the landing, and says one generic sentence
/// for all four ([`PROFILE_REFUSED`]) — `NoJammer`'s precedent, declared.
///
/// # Errors
///
/// The refusal's sentence after its prefix.
pub fn profile_switch<'a>(
    mind: &'a FrameMind,
    word: &str,
    now_ms: i64,
) -> Result<(u8, &'a str), String> {
    let names: Vec<&str> = mind.profiles.names().collect();
    let code = crate::arena::card::preset_code(word);
    let index = names
        .iter()
        .position(|held| held.eq_ignore_ascii_case(word))
        .and_then(|at| u8::try_from(at).ok());
    let Some((index, name)) = index.and_then(|index| Some((index, mind.profiles.name(index)?)))
    else {
        let carries = lobby::listed(&names, "and");
        return Err(
            match code.and_then(|code| crate::arena::card::PRESETS.get(usize::from(code))) {
                Some(preset) => {
                    format!("{preset} is not sealed on this mind; it carries {carries}.")
                }
                None => {
                    format!("No profile of that name is sealed on this mind; it carries {carries}.")
                }
            },
        );
    };
    if index == mind.active {
        return Err(format!("{name} is already the profile."));
    }
    if let Some(secs) = next_switch_s(mind, now_ms) {
        return Err(format!("One switch in ten seconds; the next in {secs} s."));
    }
    Ok((index, name))
}

/// **`MIND PROFILE`'s acknowledgement** (17b cut M5.4): `Profile Sniper: the
/// mind will fly it when you leave the seat.` — said once the switch is on
/// its way; the arena's word follows only if it refuses it.
#[must_use]
pub fn profile_ack(name: &str) -> String {
    format!("Profile {name}: the mind will fly it when you leave the seat.")
}

/// `MIND [PROFILE <name>|RULES|TRACE [n]]` — the hull's sealed mind, read from
/// the seat (17b cut M5.4; doc 17 §12 "The `MIND` verb"; OQ-8). Every form
/// reads the held keyframe ([`Frame::mind`]) and, for a time, the mirror's
/// arena clock ([`mind_now_ms`]) — never the arena:
///
/// * bare, [`mind_lines`]' block (at most seven lines; four today), or
///   [`NO_SEAL`] on a hull no seal reached, or [`NO_FRAME_YET`] before any
///   frame; then the frame it read, as every read verb pushes it
///   ([`say_and_frame`]);
/// * `RULES`, [`mind_rules_line`] (or [`NO_SEAL`] / [`NO_FRAME_YET`]), and the
///   frame;
/// * `PROFILE <name>`, the switch: the wreck refused first, as for every
///   order, then the pre-checks — no frame, no seal, and [`profile_switch`]'s
///   three — each one line opening `MIND refused.` through [`order_refused`]
///   from the intent's own word, exactly as the arena's own refusal
///   ([`PROFILE_REFUSED`]) is composed; no power test. Then
///   [`IntentKind::Profile`] is staged through the seat on helm (the arena
///   takes it from a seat alone, `apply_profile`), and [`profile_ack`] is
///   said;
/// * `TRACE [n]`, the authorized retained think ring in M6; unavailable and
///   unauthorized reads share `MIND TRACE refused. No mind trace is available.`
///   A refusal names the form rather than a station intent;
/// * anything else, [`MIND_USAGE`].
pub(super) fn mind(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let (form, tail) = first_word(rest.trim());
    match form.to_ascii_lowercase().as_str() {
        "" => {
            let held = last_frame(ctx.w, seat.driver);
            let lines = match held.as_ref() {
                None => vec![NO_FRAME_YET.to_owned()],
                Some(held) => match held.frame.mind.as_ref() {
                    None => vec![NO_SEAL.to_owned()],
                    Some(mind) => mind_lines(
                        mind,
                        !held.frame.weapons.is_empty(),
                        mind_now_ms(ctx.w, held),
                    ),
                },
            };
            say_and_frame(ctx, held.as_ref(), lines);
            Ok(())
        }
        "rules" | "rule" => station_rules(ctx, seat, rest),
        "profile" if !tail.is_empty() => mind_profile(ctx, seat, tail),
        "card" if tail.is_empty() => mind_card(ctx, seat),
        "trace" => mind_trace(ctx, seat, tail),
        _ => Err(CockpitError::Usage(MIND_USAGE.to_owned())),
    }
}

pub(super) fn mind_trace(ctx: &mut Ctx<'_>, seat: &Seat, tail: &str) -> Result<(), CockpitError> {
    let count = if tail.is_empty() {
        None
    } else {
        Some(
            tail.parse::<usize>()
                .ok()
                .filter(|n| (1..=64).contains(n))
                .ok_or_else(|| CockpitError::Usage(MIND_USAGE.into()))?,
        )
    };
    // Authorization is independent of seat possession; a borrowed hull is not
    // permission to inspect somebody else's retained decisions.
    let rows = crate::arena_bridge::mind_traces(ctx.w, ctx.actor, seat.unit, count)
        .ok_or_else(|| CockpitError::Refused(order_refused("mind trace", None, MIND_NO_TRACE)))?;
    crate::minds::trace_feed::select(ctx, seat.unit, count, true)
        .map_err(|said| CockpitError::Refused(order_refused("mind trace", None, &said)))?;
    for line in crate::minds::trace_feed::lines(&rows)
        .iter()
        .flat_map(|line| wrap_rule_words(line))
    {
        ctx.out.sys_to_actor(ctx.actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

pub(super) fn mind_card(ctx: &mut Ctx<'_>, seat: &Seat) -> Result<(), CockpitError> {
    let held =
        last_frame(ctx.w, seat.driver).ok_or_else(|| CockpitError::Refused(NO_FRAME_YET.into()))?;
    let mind = held
        .frame
        .mind
        .as_ref()
        .ok_or_else(|| CockpitError::Refused(NO_SEAL.into()))?;
    let mut lines = vec![format!(
        "Sealed card: {} / {}. Edits belong in the workshop working copy.",
        mind.name,
        mind.profiles.name(mind.active).unwrap_or("?")
    )];
    let schema = mind.card.schema();
    if let Some(schema) = schema {
        for section in &schema.sections {
            let absent = section.station == Channel::Fire && held.frame.weapons.is_empty();
            lines.push(format!(
                "{}{}:",
                section.label,
                if absent { " (no station; inert)" } else { "" }
            ));
            for field in &section.fields {
                if let Some(value) = mind.card.get(&field.id) {
                    let value = serde_json::to_string(value).unwrap_or_else(|_| "?".into());
                    lines.push(format!("  {} ({}): {value}", field.label, field.id));
                }
            }
        }
    }
    if lines.len() == 1 {
        lines.push("No editable card fields.".into());
    }
    say_and_frame(
        ctx,
        Some(&held),
        lines
            .iter()
            .flat_map(|line| wrap_rule_words(line))
            .collect(),
    );
    Ok(())
}

/// Dispatch native rules from the pilot's permitted picture. Mutation remains
/// one queued intent; the arena rechecks slots, classification and authority.
pub(super) fn station_rules(
    ctx: &mut Ctx<'_>,
    seat: &Seat,
    command: &str,
) -> Result<(), CockpitError> {
    use crate::arena::rules::{RuleCommand, RuleContact, RuleContext, RuleWeapon, parse_command};
    let read = first_word(command).0.eq_ignore_ascii_case("rules");
    if !read {
        require_alive(ctx.w, seat.driver)?;
    }
    let held = last_frame(ctx.w, seat.driver)
        .ok_or_else(|| CockpitError::Refused(NO_FRAME_YET.to_owned()))?;
    let weapons: Vec<RuleWeapon> = held
        .frame
        .weapons
        .iter()
        .map(|weapon| RuleWeapon {
            id: weapon.id.clone(),
            rounds_max: weapon.rounds_max,
        })
        .collect();
    let mut contacts = Vec::new();
    for contact in &held.frame.contacts {
        let classified = matches!(
            contact.class,
            crate::arena::state::Classification::Classified { .. }
        );
        contacts.push(RuleContact {
            name: contact.designator.to_string().into(),
            id: contact.id,
            classified,
        });
        if let Some(alias) = contact.alias {
            contacts.push(RuleContact {
                name: format!("X{alias}").into(),
                id: contact.id,
                classified,
            });
        }
    }
    let (verb, rest) = first_word(command);
    let refusal_word = if verb.eq_ignore_ascii_case("fire") {
        "fire"
    } else if verb.eq_ignore_ascii_case("lock") {
        "lock"
    } else {
        "mind rule"
    };
    let refused_weapon = (refusal_word == "fire")
        .then(|| {
            choose_weapon(&frame_weapons(&held.frame), first_word(rest).0)
                .ok()
                .map(|row| row.id)
        })
        .flatten();
    let parsed = parse_command(
        command,
        &RuleContext {
            weapons: &weapons,
            floor_permille: u16::try_from(held.frame.odds_floor_permille).unwrap_or(u16::MAX),
            contacts: &contacts,
        },
    )
    .map_err(|error| {
        CockpitError::Refused(order_refused(
            refusal_word,
            refused_weapon.as_deref(),
            &error.to_string(),
        ))
    })?;
    let (arming, lines) = match parsed {
        RuleCommand::Read { slot } => {
            let mut lines = Vec::new();
            if slot.is_none()
                && let Some(mind) = held.frame.mind.as_ref()
            {
                let rules = mind.card.rules().unwrap_or_default();
                if !rules.is_empty() {
                    lines.push("Sealed active profile rules:".to_owned());
                    for (at, rule) in rules.iter().enumerate() {
                        lines.extend(wrap_rule_words(&format!(
                            "{}. {}",
                            at + 1,
                            crate::arena::schema::rule_words(rule)
                        )));
                    }
                }
            }
            lines.push("Current station rules (manual changes are not saved):".to_owned());
            // Doc 21 cut G.12a: the master gate, said only while it holds —
            // after the head, so `None armed.` still reads as before.
            if !held.frame.weapons_free {
                lines.push(WEAPONS_HOLD_LINE.to_owned());
            }
            let current_head = lines.len();
            for entry in held
                .frame
                .rules
                .iter()
                .filter(|entry| slot.is_none_or(|slot| entry.slot == slot))
            {
                lines.extend(rule_read_lines(entry, &held.frame));
            }
            if lines.len() == current_head {
                lines.push(
                    if slot.is_some() {
                        "No active rule in that slot."
                    } else {
                        "None armed."
                    }
                    .to_owned(),
                );
            }
            say_and_frame(ctx, Some(&held), lines);
            return Ok(());
        }
        RuleCommand::Remove { slot } => {
            let rule = held
                .frame
                .rules
                .iter()
                .find(|entry| entry.slot == slot)
                .ok_or_else(|| CockpitError::Refused("No active rule in that slot.".to_owned()))?;
            (
                Arming::Remove {
                    slot,
                    channel: rule.rule.channel(),
                },
                vec![format!("Clearing rule S{slot}.")],
            )
        }
        RuleCommand::Arm(arming) => {
            let mut lines = Vec::new();
            if let Arming::Rules { rules, .. } = &arming {
                for rule in rules {
                    lines.extend(wrap_rule_words(&format!(
                        "Queued: {}",
                        crate::arena::rules::rule_words(rule, None)
                    )));
                }
            }
            (arming, lines)
        }
    };
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Condition(arming))?;
    for line in lines {
        ctx.out.sys_to_actor(ctx.actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// Complete English readback, wrapped at the terminal-width floor.
pub(super) fn wrap_rule_words(words: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in words.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > WATCH_COLUMNS {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for ch in word.chars() {
            if line.len() + ch.len_utf8() > WATCH_COLUMNS {
                lines.push(std::mem::take(&mut line));
            }
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub(super) fn rule_read_lines(
    entry: &crate::arena::weapons::FrameRule,
    frame: &Frame,
) -> Vec<String> {
    let target = entry.rule.fire().and_then(|fire| match fire.target {
        SlotTarget::Contact { id } => frame
            .contacts
            .iter()
            .find(|contact| {
                contact.id == id
                    && matches!(
                        contact.class,
                        crate::arena::state::Classification::Classified { .. }
                    )
            })
            .map(|contact| contact.designator.to_string()),
        _ => None,
    });
    wrap_rule_words(&format!(
        "S{}: {}",
        entry.slot,
        crate::arena::rules::rule_words(&entry.rule, target.as_deref())
    ))
}

/// Every native rule's typed terms and canonical English, in slot order.
pub(super) fn rule_json(entry: &crate::arena::weapons::FrameRule) -> serde_json::Value {
    serde_json::json!({ "slot": entry.slot, "rule": entry.rule,
        "text": crate::arena::rules::rule_words(&entry.rule, None) })
}

/// `MIND PROFILE <name>` — see [`mind`].
pub(super) fn mind_profile(ctx: &mut Ctx<'_>, seat: &Seat, word: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let refused = |said: &str| CockpitError::Refused(order_refused(profile_order(), None, said));
    // The wreck first, as for every order — in this verb's one prefix.
    require_alive(ctx.w, seat.driver).map_err(|said| refused(&said.to_string()))?;
    let held = last_frame(ctx.w, seat.driver).ok_or_else(|| refused(NO_FRAME_YET))?;
    let mind = held.frame.mind.as_ref().ok_or_else(|| refused(NO_SEAL))?;
    let now_ms = mind_now_ms(ctx.w, &held);
    let (index, name) = profile_switch(mind, word, now_ms).map_err(|said| refused(&said))?;
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Profile { index })?;
    ctx.out
        .sys_to_actor(actor, NOTICE_CATEGORY, profile_ack(name));
    Ok(())
}
