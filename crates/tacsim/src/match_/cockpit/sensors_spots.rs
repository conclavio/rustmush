//! Watch digest, heading/speed/lock/range/bearing/scan commands, named ground spots, and brief view.

use super::*;

// ---------------------------------------------------------------------------
// The line-mode digest (16b cut AP.3; 16c N19)
// ---------------------------------------------------------------------------

/// **The widest a digest line may be**, in characters (16b cut AP.3): the
/// terminal-width floor (16c cut AB), so a line client at the floor never
/// wraps one.
pub const WATCH_COLUMNS: usize = 120;

/// **The line-mode digest** (16b cut AP.3, 16c `N19`): one to three lines of
/// the pilot's own picture, which the bridge pushes a `WATCH ON` pilot as one
/// `Sys` whenever it reads differently from the last it pushed.
///
/// 1. **The own line, always**: `Watch RA: online, 24 kph, heading 090°`, with
///    `, HELD` and `, DESTROYED` after it as `STATUS` prints them, in that
///    order — `STATUS`'s second line after its colon, **without** the word
///    `power` (a reader that splits on `": power "`, as the bot's does, must
///    never take the digest for a `STATUS` reply) and **without** a position
///    (the block must not change with every cell).
/// 2. **The guns line, only while `weapons` is not empty**: `Watch guns:
///    cannon ready, machine_gun (mg) cycling (3 s); lock BA, 450 m, 46% to
///    hit` — the rows **given**, in order, each with the short name a pilot may
///    type ([`weapon_short_of`]) and its readiness rounded up to the second as
///    [`WeaponRow::render`] rounds it — or `empty` once its rounds are gone,
///    whatever its cycle, because `FIRE` refuses an empty gun and a gun that
///    read `ready` with nothing in it would be a lie the pilot only finds out
///    by firing; then the lock: its designator, ` (FRIENDLY)`
///    on a friend (`friendly_mark`), the held range of its row and the chance
///    of the shot a bare `FIRE` takes (`fired_by_default`, the figure
///    [`lock_line`] prints); `lock BA, 450 m` with no chance resolved; `no
///    lock` with none. The caller chooses the rows: the bridge passes the
///    frame's own ([`frame_weapons`]), the verb the aged ones (`carried`).
/// 3. **The contacts line, only while the platform holds a contact**: `Watch
///    contacts: BA blue 087° far 450 m; X3 unknown 315° effective 200 m,
///    plotted 4 s ago` — the rows `CONTACTS` prints, in its order, the range
///    held against `ranges` ([`frame_contacts_held`]), and a plot's age only
///    once [`stale_clause`] says it — older than the sensor pass `pass_ms`. A
///    fresh plot's age is never printed, or a held contact would change the
///    block every second.
///
/// **The fit rule** for lines 2 and 3: entries go in in order while the line
/// stays within [`WATCH_COLUMNS`] characters with room kept for the tail; the
/// first that does not fit ends the line `; +N more`, `N` the entries left
/// out. The lock is always printed: the gun rows give way to it.
///
/// **The observer rule by construction** (doc 16 §1, §5): a pure function of
/// the pilot's own frame, the ranges the pilot was told and the rows it is
/// given, and nothing a pilot could not read off `STATUS`, `WEAPONSTATUS` and
/// `CONTACTS` — designators only, never an object number, a name or a pilot,
/// and no position. **No tempo word** (16c `T-59`): `STATUS` prints the
/// pilot's own scope and never the arena's tempo, and neither does this.
/// **A range says so** (16c `Q-PT1`): the header ends `, RANGE` while the
/// match is one ([`on_range`]), after the `HELD` and `DESTROYED` marks.
#[must_use]
pub fn watch_lines(
    frame: &Frame,
    ranges: &BTreeMap<ContactId, u64>,
    weapons: &[WeaponRow],
    pass_ms: i64,
) -> Vec<String> {
    let mut lines = vec![format!(
        "Watch {}: {}, {} kph, heading {:03}°{}{}{}",
        frame.callsign,
        frame.power.as_str(),
        cms_to_kph(frame.speed_cms),
        frame.pose.heading_mdeg / 1_000,
        if frame.held { ", HELD" } else { "" },
        if frame.wreck { ", DESTROYED" } else { "" },
        // 16c `Q-PT1`: the match's public mode, beside the pilot's own marks.
        if on_range(frame) { ", RANGE" } else { "" }
    )];
    let rows = frame_contacts_held(frame, ranges);
    if !weapons.is_empty() {
        let guns: Vec<String> = weapons
            .iter()
            .map(|row| {
                let named = match weapon_short_of(weapons, &row.id) {
                    Some(short) => format!("{} ({short})", row.id),
                    None => row.id.to_string(),
                };
                // An empty gun says so before its cycle: `FIRE` refuses it
                // whatever the breech is doing (`CockpitError::Empty`), and
                // the last round leaving is a change the block must push —
                // `WEAPONSTATUS` prints the rounds, and this has no room to.
                if row.rounds <= 0 {
                    format!("{named} empty")
                } else if row.ready {
                    format!("{named} ready")
                } else {
                    format!(
                        "{named} cycling ({} s)",
                        row.ready_in_ms.saturating_add(999) / 1_000
                    )
                }
            })
            .collect();
        let locked = frame_lock(frame).and_then(|lock| rows.iter().find(|row| row.id == lock));
        let lock = match locked {
            Some(row) => {
                let mark = friendly_mark(frame, row.id);
                match fired_by_default(&frame.weapons).and_then(|weapon| weapon.p_permille) {
                    Some(permille) => format!(
                        "; lock {}{mark}, {} m, {}% to hit",
                        row.designator,
                        row.range_m,
                        permille / 10
                    ),
                    None => format!("; lock {}{mark}, {} m", row.designator, row.range_m),
                }
            }
            None => "; no lock".to_owned(),
        };
        let columns = WATCH_COLUMNS.saturating_sub(lock.chars().count());
        let mut line = fit_entries("Watch guns: ", &guns, ", ", columns);
        line.push_str(&lock);
        lines.push(line);
    }
    if !rows.is_empty() {
        let contacts: Vec<String> = rows
            .iter()
            .map(|row| {
                format!(
                    "{} {} {:03}° {} {} m{}",
                    row.designator,
                    row.class,
                    row.bearing_deg,
                    row.band,
                    row.range_m,
                    stale_clause(row.age_ms, pass_ms)
                )
            })
            .collect();
        lines.push(fit_entries(
            "Watch contacts: ",
            &contacts,
            "; ",
            WATCH_COLUMNS,
        ));
    }
    lines
}

/// **One digest line, fitted** (16b cut AP.3): `head` and then `entries`
/// joined by `sep`, in order, while the line stays within `columns`
/// characters — keeping room, whenever entries would remain, for the tail
/// `; +N more` that says how many were left out. Stops at the first entry that
/// does not fit, so what is printed is always a prefix of the list in its own
/// order. Characters and not bytes, because `°` is two bytes and one column.
pub(super) fn fit_entries(head: &str, entries: &[String], sep: &str, columns: usize) -> String {
    let mut line = head.to_owned();
    let mut width = head.chars().count();
    let mut taken = 0_usize;
    for entry in entries {
        let joiner = if taken == 0 { "" } else { sep };
        let after = width + joiner.chars().count() + entry.chars().count();
        let left = entries.len().saturating_sub(taken + 1);
        let tail = if left == 0 {
            0
        } else {
            more_tail(left).chars().count()
        };
        if after + tail > columns {
            break;
        }
        line.push_str(joiner);
        line.push_str(entry);
        width = after;
        taken += 1;
    }
    let left = entries.len().saturating_sub(taken);
    if left > 0 {
        if taken > 0 {
            line.push_str(&more_tail(left));
        } else {
            // Nothing fitted at all (a width no shipped word reaches): the
            // count alone, with no separator before it.
            line.push_str(&format!("+{left} more"));
        }
    }
    line
}

/// The tail a fitted digest line ends with when entries were left out.
pub(super) fn more_tail(left: usize) -> String {
    format!("; +{left} more")
}

/// A trailing `QUIET` on `HEADING` and `SPEED` (doc 21 §7.3, 2026-10-27): the
/// cockpit picture's keys send the verb with this word, and the order's
/// acknowledgement — `Steering to …°.`, `Making … kph.` — is not spoken. A
/// held `w` or a held turn sends an order every quarter second, and the owner's
/// first playtest read each one in the comms; the HUD already shows the
/// ordered speed and heading. A **refusal is still spoken** whichever way the
/// order came: the pilot must learn why the tank did not move. Answers the
/// rest of the line without the word, and whether it was there.
pub(super) fn quiet_flag(rest: &str) -> (&str, bool) {
    match rest.trim_end().rsplit_once(char::is_whitespace) {
        Some((head, word)) if word.eq_ignore_ascii_case("quiet") => (head, true),
        _ => (rest, false),
    }
}

/// `HEADING <deg> [QUIET]` — 0 to 359, clockwise from north (doc 16 §3.7);
/// `QUIET` is [`quiet_flag`].
pub(super) fn heading(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let (rest, quiet) = quiet_flag(rest);
    let deg: u32 = rest
        .trim()
        .parse()
        .ok()
        .filter(|deg| *deg < 360)
        .ok_or_else(|| CockpitError::Usage("HEADING <0-359>, clockwise from north.".to_owned()))?;
    // 16b cut D.2 (`B-03`), in the arena's own order: the wreck, then the motion
    // limit, then the power.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Heading {
            mdeg: deg.saturating_mul(1_000),
        },
    )?;
    if !quiet {
        let line = format!("Steering to {deg}°.");
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// `SPEED <kph> [QUIET]` — the unit's reverse ceiling to its own maximum (doc 16
/// §3.7); `QUIET` is [`quiet_flag`]. The wire is centimetres a second (doc 16
/// §3.2's integers), so the conversion is here and the pilot never types one.
///
/// **16b cut D.3 (`T-60`) made the refusal true.** It read `That unit makes 54
/// kph; SPEED 0 to 54.` — a range the verb did not enforce at either end: it
/// compared the **magnitude**, so `SPEED -54` was accepted on a tank whose
/// reverse ceiling is 40 per cent of its forward one, and the arena silently
/// clamped it to `-21` at apply. The two ceilings are different numbers and the
/// sentence now names both, read the way
/// [`crate::arena::intents`] reads them at apply:
/// [`crate::arena::state::TrackedRules::reverse_ceiling_cms`].
pub(super) fn speed(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let actor = ctx.actor;
    let (rest, quiet) = quiet_flag(rest);
    let kph: i32 =
        rest.trim().parse().ok().ok_or_else(|| {
            CockpitError::Usage("SPEED <kph>; a negative speed reverses.".to_owned())
        })?;
    let (max_cms, reverse_cms) = speed_ceilings(ctx.w, seat.unit);
    let max_kph = cms_to_kph(i32::try_from(max_cms).unwrap_or(i32::MAX));
    let reverse_kph = cms_to_kph(i32::try_from(reverse_cms).unwrap_or(i32::MAX));
    if kph > max_kph || kph < -reverse_kph {
        return Err(CockpitError::Usage(format!(
            "That unit makes {max_kph} kph ahead and {reverse_kph} astern; SPEED -{reverse_kph} \
             to {max_kph}."
        )));
    }
    // 16b cut D.2 (`B-03`): as `HEADING`.
    require_alive(ctx.w, seat.driver)?;
    require_mobile(ctx.w, seat.driver)?;
    require_power(ctx.w, seat.driver)?;
    forward_intent(
        ctx,
        seat.driver,
        seat.unit,
        IntentKind::Speed {
            cms: kph_to_cms(kph),
        },
    )?;
    if !quiet {
        let line = format!("Making {kph} kph.");
        ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    }
    Ok(())
}

/// `LOCK <contact>` — hold one of the platform's contacts (doc 16 §3.7).
///
/// One refusal for every id the platform does not hold, whether it is malformed,
/// somebody else's or expired (doc 16 §3.6). **16b cut S.2 gave it something to
/// resolve**: the id is looked up in the platform's own contact list, which is the
/// world's mirror of `crate::arena::state::ShadowContacts` — the store
/// `ShadowContacts::resolve` answers from, keyed by the same
/// `(viewer platform, id)` pair and refusing with the same one sentence. A pilot
/// naming a malformed id, another platform's id or an expired one takes the same
/// path, byte for byte, because all three simply fail to be in the list.
///
/// **16b cut W.1 gave the order its effect**: the arena's phase 4 records the lock
/// on the platform, journals one `locked` line, and puts the contact's id on the
/// frame — so the `Locking {id}.` this says is now a statement about the arena and
/// not only about the world's mirror. A contact the platform then *loses* clears the
/// lock and the pilot is told so once (`NoticeKind::LockLost`).
///
/// **16b cut F.2 took the percentage off this verb.** Cut W.2 printed it here off
/// the frame the world already held, which meant a first `LOCK` was silent and a
/// repeat quoted the *previous* pulse's chance; the arena now says it itself when
/// the lock resolves ([`crate::arena::perception::FrameNotice::Locked`]), at the
/// timestamp it resolves at. There is one owner of the sentence and no second
/// lane (16b §0.2), so this says `Locking {id}.` and nothing else.
pub(super) fn lock(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    if rest
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case("when"))
    {
        return station_rules(ctx, seat, &format!("LOCK {rest}"));
    }
    let actor = ctx.actor;
    // 16b cut D.2 (`B-03`): a wreck takes no order at all, and the arena's own
    // refusal would be a pulse away. It is the FIRST thing this verb does, so a
    // pilot in a wreck is never answered about which contacts are out there.
    require_alive(ctx.w, seat.driver)?;
    // **16b cut D.3 (`T-60`): a bare `LOCK` is usage and not a refusal.** An
    // empty argument is not a contact id that failed to resolve — it is a pilot
    // who has not said which one — and `No such contact.` over it read as a
    // statement about the scope. The one byte-identical refusal is still the
    // answer to every *non-empty* word, which is what keeps `LOCK BA` against an
    // unheld, a still-unknown and a re-acquired-unknown `BA` indistinguishable
    // from `LOCK ZZ`.
    if rest.trim().is_empty() {
        return Err(CockpitError::Usage(
            "LOCK <contact>; CONTACTS lists them.".to_owned(),
        ));
    }
    let held = last_frame(ctx.w, seat.driver)
        .map(|held| frame_contacts_held(&held.frame, &held.ranges))
        .unwrap_or_default();
    // **16b cut Q.1**: the word is a designator, not sixteen digits, and the
    // resolution is against the platform's own rows — so an id that names
    // nothing, an id that names somebody else's contact and an id that has
    // expired all take one path, byte for byte.
    let contact = resolve_designator(&held, rest).ok_or(CockpitError::NoSuchContact)?;
    // 16b cut D.2 (`B-18`): the echo says `(FRIENDLY)` where the row is one, off
    // the row's own `friendly` — which is `false` for every unidentified return,
    // so a blip is never marked. Nothing is refused.
    let named = held
        .iter()
        .find(|row| row.id == contact)
        .map(|row| {
            format!(
                "{}{}",
                row.designator,
                if row.friendly { FRIENDLY_MARK } else { "" }
            )
        })
        .unwrap_or_default();
    forward_intent(ctx, seat.driver, seat.unit, IntentKind::Lock { contact })?;
    let line = format!("Locking {named}.");
    ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
    Ok(())
}

/// **`RANGE [<designator>]`** — how far away one contact plotted (16b cut Q.1;
/// doc 16 §3.6, the owner's ask of 2026-09-19).
///
/// The named contact, else the platform's lock, else [`NO_LOCK_OR_NAME`]. It
/// reads the **last frame's plot** and nothing else — the same figure
/// `CONTACTS` and `WEAPONSTATUS` are reading — so three surfaces cannot give
/// three distances.
///
/// A locked contact earns [`LOCK_RANGE_STEP_M`] and the rest of the scope
/// [`CONTACT_RANGE_STEP_M`]: a lock is a fire-control solution and the platform
/// is tracking that one return continuously.
///
/// **16b cut V.1**: a word that names no held contact is tried against the
/// ground's named spots second ([`named_target`]), so `RANGE The Ford` and
/// `RANGE a` answer [`spot_range_line`]; a word that is neither is still the one
/// byte-identical [`NO_SUCH_CONTACT`].
pub(super) fn range(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let (spots, cell_cm) = spots_of(ctx.w, match_of(ctx.w, seat.driver, held.as_ref()));
    let line = match named_target(held.as_ref(), &spots, rest)? {
        Target::Contact(frame, row) => range_line(frame, &row, pass_ms(ctx.w)),
        Target::Spot {
            frame,
            letter,
            spot,
        } => spot_range_line(
            &spot,
            letter,
            range_m_rounded(
                spot_range_cm(&frame.pose, &spot, cell_cm),
                SPOT_RANGE_STEP_M,
            ),
        ),
    };
    say_and_frame(ctx, held.as_ref(), vec![line]);
    Ok(())
}

/// **The sentence `RANGE` says** (16b cut Q.1), split out from the world read the
/// way [`stale_clause`] is — so the step a lock earns and the staleness clause are
/// one pure function, and the branch between the two steps can be pinned against a
/// plot held still rather than against a live approach.
///
/// A locked contact earns [`lock_range_step_m`] of its own plot's age and the rest
/// of the scope [`CONTACT_RANGE_STEP_M`]: a lock is a fire-control solution and
/// the platform is tracking that one return continuously — but no finer than the
/// plot in front of the pilot can carry (16b cut D.4, `T-20`). The age is the row's
/// own, which is the `1 s` `CONTACTS` already prints.
///
/// **16b cut B.4** (`T-60`): an unlocked row prints `row.range_m` — the figure the
/// row already carries, which is the HELD figure ([`frame_contacts_held`]) — and
/// does not recompute it. Recomputing from the plot here re-opened the 50 m edge
/// the row's memory closes, so `RANGE` could read `450 m` while `CONTACTS` and
/// both panels held `400 m`; printing the row's own figure is what makes "three
/// surfaces cannot give three distances" true again. Only the lock is recomputed
/// from the plot, at [`lock_range_step_m`] of its age: its finer step is the
/// frame's `lock_range_m`, the same call of the same function, and keeps no memory
/// (16b cut D.4's ruling).
#[must_use]
pub fn range_line(frame: &Frame, row: &ContactRow, pass_ms: i64) -> String {
    let locked = frame_lock(frame) == Some(row.id);
    let metres = if locked {
        frame
            .contacts
            .iter()
            .find(|contact| contact.id == row.id)
            .map_or(row.range_m, |contact| {
                range_m_rounded(contact.range_cm, lock_range_step_m(row.age_ms))
            })
    } else {
        row.range_m
    };
    format!(
        "Range to {}: {metres} m{}.",
        row.designator,
        stale_clause(row.age_ms, pass_ms)
    )
}

/// **`BEARING [<designator>]`** — which way one contact plotted (16b cut Q.1),
/// absolute and off the platform's own nose. The same rules [`range`] follows,
/// the second resolution arm included (16b cut V.1).
pub(super) fn bearing(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let (spots, cell_cm) = spots_of(ctx.w, match_of(ctx.w, seat.driver, held.as_ref()));
    let line = match named_target(held.as_ref(), &spots, rest)? {
        Target::Contact(_, row) => bearing_line(&row, pass_ms(ctx.w)),
        Target::Spot {
            frame,
            letter,
            spot,
        } => {
            let bearing = spot_bearing_mdeg(&frame.pose, &spot, cell_cm);
            spot_bearing_line(
                &spot,
                letter,
                bearing / 1_000,
                relative_mdeg(frame.pose.heading_mdeg, bearing) / 1_000,
            )
        }
    };
    say_and_frame(ctx, held.as_ref(), vec![line]);
    Ok(())
}

/// **The sentence `BEARING` says** (16b cut Q.1), for the reason [`range_line`] is
/// its own function. A bearing has one step, so this is the designator, the two
/// degrees and the staleness clause.
#[must_use]
pub fn bearing_line(row: &ContactRow, pass_ms: i64) -> String {
    format!(
        "Bearing to {}: {:03}° (rel {:03}°){}.",
        row.designator,
        row.bearing_deg,
        row.relative_deg,
        stale_clause(row.age_ms, pass_ms)
    )
}

/// **`SCAN [<designator>]`** — what is left of one contact, section by section
/// (16b cut Q.2; doc 16 §3.6, the owner's ask of 2026-09-19: *see the enemy
/// status, not exact numbers but green/yellow/red*).
///
/// The same argument rules [`range`] follows: the named contact, else the
/// platform's lock, else [`NO_LOCK_OR_NAME`]. What it prints is the reading the
/// arena put on the contact ([`crate::arena::perception::FrameContact::condition`])
/// and never a second computation, so the words a pilot reads and the bands a
/// panel draws are one answer.
pub(super) fn scan(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), CockpitError> {
    let held = last_frame(ctx.w, seat.driver);
    let (frame, row) = named_row(held.as_ref(), rest)?;
    let lines = scan_lines(frame, &row);
    say_and_frame(ctx, held.as_ref(), lines);
    Ok(())
}

/// **The two-space indent a wrapped `SCAN` run carries** — `STATUS`'s damage
/// block's own, so the two blocks line up under one another on a narrow terminal.
pub(super) const SCAN_INDENT: &str = "  ";

/// **The lines `SCAN` says about one contact** (16b cut Q.2), split out from the
/// world read the way [`range_line`] is, so every sentence can be pinned against a
/// frame built by hand.
///
/// Four answers, and each is a different thing the pilot does not know:
///
/// * **unidentified** — `X2 is not identified yet; there is nothing to read.`
///   Section ids are a chassis vocabulary, so a blip has no sections to read
///   bands into; it says so rather than refusing, because the pilot does hold the
///   contact and telling them otherwise would be a lie about their own scope.
/// * **classified, no reading** — `No reading on BA yet: close the range for a
///   look.` The close look is the price, and this is the only line that says what
///   the price is.
/// * **a reading** — the header with the **look's** own age, then one
///   `section band` pair per section, comma-separated and wrapped at
///   `DAMAGE_COLUMNS`.
/// * **unheld** — never reaches this function: `named_row` answers the one
///   byte-identical [`NO_SUCH_CONTACT`] first.
///
/// **Never a number other than the age.** The bands are what a sight is worth
/// (`crate::arena::condition`), and there is no exact figure on this path for a
/// later surface to print.
#[must_use]
pub fn scan_lines(frame: &Frame, row: &ContactRow) -> Vec<String> {
    let named = row.designator;
    if row.class == "unknown" {
        return vec![format!(
            "{named} is not identified yet; there is nothing to read."
        )];
    }
    let reading = frame
        .contacts
        .iter()
        .find(|held| held.id == row.id)
        .and_then(|held| held.condition);
    let Some(reading) = reading else {
        return vec![format!(
            "No reading on {named} yet: close the range for a look."
        )];
    };
    // The vocabulary the bands are in the order of. A reading whose table is not
    // on the frame cannot be produced by the arena — the index is handed out by
    // the walk that builds the list — and is answered with no sections rather
    // than with a guess.
    let names: &[SmolStr] = frame
        .scan_tables
        .get(usize::from(reading.table))
        .map_or(&[], Vec::as_slice);
    let mut lines = vec![format!(
        "Scan of {named}, read {} s ago:",
        reading.age_ms / 1_000
    )];
    let mut run = String::new();
    for (index, section) in names.iter().enumerate() {
        let Some(band) = reading.bands.band(index) else {
            continue;
        };
        let cell = format!("{section} {}", band.as_str());
        // The comma stays on the line it follows, so a wrapped run reads as one
        // sentence rather than as two lists. **16b cut B.4** (`B-26`): the test
        // counts all three characters the cell can cost — the `, ` it is joined
        // with AND the `,` the run takes if it wraps here instead — so no run,
        // comma included, ever passes [`DAMAGE_COLUMNS`] (it was `+ 2`, and a
        // run could print at 77). `section_lines` needs no such margin: it
        // joins with two spaces and pushes no comma on a wrap.
        if !run.is_empty() && run.len() + 3 + cell.len() > DAMAGE_COLUMNS {
            run.push(',');
            lines.push(run.clone());
            run.clear();
        }
        if run.is_empty() {
            run.push_str(SCAN_INDENT);
        } else {
            run.push_str(", ");
        }
        run.push_str(&cell);
    }
    if !run.is_empty() {
        lines.push(run);
    }
    lines
}

/// **The contact a read verb is about** (16b cut Q.1): the one the pilot named,
/// else the platform's lock.
///
/// # Errors
///
/// [`CockpitError::NoSuchContact`] for a word that resolves to nothing — the one
/// byte-identical refusal — and [`CockpitError::NoLockOrName`] when nothing was
/// named and there is no lock to fall back on.
pub(super) fn named_row<'a>(
    held: Option<&'a CockpitFrame>,
    rest: &str,
) -> Result<(&'a Frame, ContactRow), CockpitError> {
    let named = rest.trim();
    // A platform the arena has not spoken to yet holds nothing, and the two
    // refusals have to stay in the right order even then: a **name** takes the
    // one byte-identical `No such contact.` whether the scope is empty or
    // missing, because a pilot who could tell those two apart could tell a name
    // that exists from one that does not.
    let (frame, ranges) = match held {
        Some(held) => (&held.frame, &held.ranges),
        None if named.is_empty() => return Err(CockpitError::NoLockOrName),
        None => return Err(CockpitError::NoSuchContact),
    };
    // 16b cut B.4 (`T-60`): the rows the pilot was told, ranges held.
    let rows = frame_contacts_held(frame, ranges);
    let id = if named.is_empty() {
        frame_lock(frame).ok_or(CockpitError::NoLockOrName)?
    } else {
        resolve_designator(&rows, named).ok_or(CockpitError::NoSuchContact)?
    };
    rows.into_iter()
        .find(|row| row.id == id)
        .ok_or(CockpitError::NoSuchContact)
        .map(|row| (frame, row))
}

/// **What `RANGE` or `BEARING` is about** (16b cut V.1): a held contact, as
/// [`named_row`] resolves one, or one of the ground's named spots.
pub(super) enum Target<'a> {
    /// A contact the platform holds, on the frame that holds it.
    Contact(&'a Frame, ContactRow),
    /// A key spot, with the frame whose pose the range and bearing are read from.
    Spot {
        /// The pilot's last frame.
        frame: &'a Frame,
        /// The spot's marker letter ([`spot_letter`] of its authored index).
        letter: char,
        /// The spot itself.
        spot: KeySpot,
    },
}

/// **The target of a read verb that takes a contact or a spot** (16b cut V.1):
/// `RANGE` and `BEARING` only.
///
/// The word goes to [`resolve_designator`] first and to [`resolve_spot`] second,
/// and a miss on both is [`CockpitError::NoSuchContact`] — **the one
/// byte-identical `No such contact.`**, so `RANGE The Fjord` cannot be told from
/// `RANGE ZZ`, and neither from `RANGE BA` against a `BA` the platform has not
/// found. The two arms cannot collide: a one-character word is never a
/// designator (the shortest is two, `RA` or `X1`), and the load-time name check
/// (`definitions::arena`'s `check_spot_name`) keeps a spot's name from ever
/// having a designator's shape.
///
/// An empty word is the lock, exactly as [`named_row`] reads it. With **no frame
/// held**, a contact word keeps [`CockpitError::NoSuchContact`] (as
/// [`named_row`] answers it: a pilot who could tell an empty scope from a missing
/// one could tell a name that exists from one that does not), and a spot word
/// answers [`CockpitError::NoFrameYet`] — the spot is surveyed ground every
/// pilot knows, and what is missing is the pilot's own position.
///
/// `SCAN` and `LOCK` keep [`named_row`] and [`resolve_designator`] alone: a spot
/// is ground and neither verb means anything about ground.
///
/// # Errors
///
/// [`CockpitError::NoSuchContact`], [`CockpitError::NoLockOrName`] and
/// [`CockpitError::NoFrameYet`], as above.
pub(super) fn named_target<'a>(
    held: Option<&'a CockpitFrame>,
    spots: &[KeySpot],
    rest: &str,
) -> Result<Target<'a>, CockpitError> {
    let named = rest.trim();
    if named.is_empty() {
        return named_row(held, rest).map(|(frame, row)| Target::Contact(frame, row));
    }
    // The designator arm, first and exactly as `named_row` resolves it.
    if let Some(held) = held {
        let rows = frame_contacts_held(&held.frame, &held.ranges);
        if let Some(row) = resolve_designator(&rows, named)
            .and_then(|id| rows.into_iter().find(|row| row.id == id))
        {
            return Ok(Target::Contact(&held.frame, row));
        }
    }
    // The spot arm. A miss here is the one refusal, whatever the frame.
    let (index, spot) = resolve_spot(spots, named).ok_or(CockpitError::NoSuchContact)?;
    let letter = spot_letter(index).ok_or(CockpitError::NoSuchContact)?;
    let Some(held) = held else {
        return Err(CockpitError::NoFrameYet);
    };
    Ok(Target::Spot {
        frame: &held.frame,
        letter,
        spot: spot.clone(),
    })
}

/// **How old a plot has to be before a read verb says so** (16b cut Q.1): the
/// sensor pass's own period, read the way [`lobby::configs`] reads it.
///
/// A plot older than one pass is a plot the platform did not refresh last time it
/// looked, which is exactly when a pilot needs to know they are shooting at a
/// memory. The sentence is [`stale_clause`]'s; this is the only world read the two
/// read verbs make.
pub(crate) fn pass_ms(w: &World) -> i64 {
    lobby::configs(w).map_or(1_000_i64, |(_, _, sensors)| {
        i64::from(sensors.pass_ms.max(1))
    })
}

/// **The clause itself** (16b cut Q.1), split out from the configuration read so
/// the sentence can be pinned without a world: `, plotted 4 s ago` past the
/// threshold, and nothing at or under it.
#[must_use]
pub fn stale_clause(age_ms: i64, pass_ms: i64) -> String {
    if age_ms > pass_ms.max(1) {
        format!(", plotted {} s ago", age_ms / 1_000)
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// The named ground (16b cut V.1; doc 16 §3.2 "Terrain")
// ---------------------------------------------------------------------------
//
// Doc 16 §3.2 rules the proving ground surveyed: its terrain is known to every
// pilot at check-out, its units are not. The arena pack's `key_spots` are part
// of that ground — The Ford, The Crossroads, North Barn, East Compound and,
// since 16b cut AE.1, The Bluff — and this is where a pilot reads them: a letter on the map ([`SPOT_LETTERS`]), a
// name in the legend, `RANGE`/`BEARING <spot>`, `STATUS`'s nearest spot and
// `VIEW`'s off-window line. Every function here is a pure function of the
// authored spots and the pilot's OWN pose, so nothing in this section can say
// anything about another unit; and ground does not age, so none of it carries
// a stale clause.

/// **The ground's named spots and its cell size** (16b cut V.1), off the match's
/// own definition ([`ArenaRegistry::arena_of`]).
///
/// A clone of at most [`crate::definitions::arena::MAX_KEY_SPOTS`] rows, made
/// only when a verb is typed. A match the registry holds no ground for answers
/// no spots, which every caller reads as "the ground names none".
pub(crate) fn spots_of(w: &World, match_id: Option<MatchId>) -> (Vec<KeySpot>, u16) {
    match_id
        .and_then(|match_id| {
            w.module_state::<ArenaRegistry>()
                .and_then(|registry| registry.arena_of(match_id))
        })
        .map_or((Vec::new(), 1), |ground| {
            (ground.key_spots.clone(), ground.terrain.cell_cm())
        })
}

/// The match a pilot's cockpit reads: the last frame's, else the registry's own
/// binding — which is how a read verb typed before the first frame still knows
/// whose ground it is standing on (16b cut V.1).
pub(super) fn match_of(w: &World, pilot: ObjId, held: Option<&CockpitFrame>) -> Option<MatchId> {
    held.map(|held| held.match_id).or_else(|| {
        w.module_state::<ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot))
            .map(|(match_id, _)| match_id)
    })
}

/// **The spot a pilot's word names** (16b cut V.1), with its authored index, or
/// `None`.
///
/// Trimmed and ASCII case-folded: the whole name (`the ford`), else a
/// one-character word equal to the spot's letter (`a`). **No prefixes**: `the`
/// is not The Ford, because a prefix that two spots share is a word that names
/// two places. A spot past [`SPOT_LETTERS`] — which the load-time cap makes
/// impossible — is never resolved, by name or otherwise.
///
/// It can never answer a designator's word: a one-character word is shorter
/// than every designator (`RA` and `X1` are two), and the load-time name check
/// (`definitions::arena`'s `check_spot_name`) refuses a name with a designator's
/// shape. So the two arms of `RANGE` and `BEARING` never both match.
#[must_use]
pub fn resolve_spot<'a>(spots: &'a [KeySpot], word: &str) -> Option<(usize, &'a KeySpot)> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let lettered = || spots.iter().zip(SPOT_LETTERS).enumerate();
    if let Some((index, (spot, _))) =
        lettered().find(|(_, (spot, _))| spot.name.eq_ignore_ascii_case(word))
    {
        return Some((index, spot));
    }
    let mut chars = word.chars();
    let (Some(one), None) = (chars.next(), chars.next()) else {
        return None;
    };
    lettered()
        .find(|(_, (_, letter))| letter.eq_ignore_ascii_case(&one))
        .map(|(index, (spot, _))| (index, spot))
}

/// The centre of a spot's cell, arena centimetres: `x × cell_cm + cell_cm / 2`,
/// the same centre a pose placed on that cell stands at.
pub(super) fn spot_centre_cm(spot: &KeySpot, cell_cm: u16) -> (i64, i64) {
    let cell = i64::from(cell_cm);
    (
        i64::from(spot.x)
            .saturating_mul(cell)
            .saturating_add(cell / 2),
        i64::from(spot.y)
            .saturating_mul(cell)
            .saturating_add(cell / 2),
    )
}

/// **How far a spot is from a pose**, whole centimetres, over the ground (16b
/// cut V.1): 2-D, from the pose to the spot's cell centre, through the arena's
/// one range function ([`crate::arena::index::range_between`]) with both heights
/// at the datum — a spot is a place on the map, not a point in the air.
#[must_use]
pub fn spot_range_cm(pose: &mush_sim::data::Pose, spot: &KeySpot, cell_cm: u16) -> u64 {
    let (sx, sy) = spot_centre_cm(spot, cell_cm);
    crate::arena::index::range_between(pose.x_cm, pose.y_cm, 0, sx, sy, 0)
}

/// **Which way a spot is from a pose**, millidegrees clockwise from north (16b
/// cut V.1), through [`crate::arena::perception::bearing_of`] — the function a
/// contact's bearing is read with — so the cockpit and the arena agree on north.
#[must_use]
pub fn spot_bearing_mdeg(pose: &mush_sim::data::Pose, spot: &KeySpot, cell_cm: u16) -> u32 {
    let (sx, sy) = spot_centre_cm(spot, cell_cm);
    crate::arena::perception::bearing_of(sx.saturating_sub(pose.x_cm), sy.saturating_sub(pose.y_cm))
}

/// **The sentence `RANGE <spot>` says** (16b cut V.1):
/// `Range to The Ford (A): 430 m.` — metres already rounded to
/// [`SPOT_RANGE_STEP_M`]. No stale clause: ground does not age.
#[must_use]
pub fn spot_range_line(spot: &KeySpot, letter: char, metres: u64) -> String {
    format!("Range to {} ({letter}): {metres} m.", spot.name)
}

/// **The sentence `BEARING <spot>` says** (16b cut V.1):
/// `Bearing to The Ford (A): 087° (rel 357°).` — the absolute bearing and the
/// same off the platform's own nose, in whole degrees.
#[must_use]
pub fn spot_bearing_line(
    spot: &KeySpot,
    letter: char,
    bearing_deg: u32,
    relative_deg: u32,
) -> String {
    format!(
        "Bearing to {} ({letter}): {bearing_deg:03}° (rel {relative_deg:03}°).",
        spot.name
    )
}

/// **The spot nearest a pose** (16b cut V.1): its authored index, the spot and
/// its range in centimetres ([`spot_range_cm`]); the least range, ties to the
/// lowest index. `None` when the ground names no spot. A spot past
/// [`SPOT_LETTERS`] is not considered, for the reason [`resolve_spot`] skips it.
#[must_use]
pub fn nearest_spot<'a>(
    spots: &'a [KeySpot],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<(usize, &'a KeySpot, u64)> {
    let mut best: Option<(usize, &KeySpot, u64)> = None;
    for (index, (spot, _)) in spots.iter().zip(SPOT_LETTERS).enumerate() {
        let range = spot_range_cm(pose, spot, cell_cm);
        // Strictly nearer only, so the first of two equal ranges keeps it.
        if best.is_none_or(|(_, _, held)| range < held) {
            best = Some((index, spot, range));
        }
    }
    best
}

/// **`STATUS`'s nearest-spot line** (16b cut V.1):
/// `  nearest spot: The Ford (A), 120 m at 045°` — the range rounded to
/// [`SPOT_RANGE_STEP_M`] and the absolute bearing. `None` when the ground names
/// no spot, and `STATUS` then prints no line at all.
#[must_use]
pub fn nearest_spot_line(
    spots: &[KeySpot],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    let (index, spot, range) = nearest_spot(spots, pose, cell_cm)?;
    let letter = spot_letter(index)?;
    Some(format!(
        "  nearest spot: {} ({letter}), {} m at {:03}°",
        spot.name,
        range_m_rounded(range, SPOT_RANGE_STEP_M),
        spot_bearing_mdeg(pose, spot, cell_cm) / 1_000
    ))
}

/// **`VIEW`'s line about the named ground its window could not draw** (16b cut
/// V.1): every spot whose index is not in `drawn`, in authored order, as
/// `<letter> <name>, <range> m at <bearing>°`, joined by `; ` and closed with a
/// full stop:
///
/// `Off the window: C North Barn, 1010 m at 045°; E East Compound, 2090 m at 070°.`
///
/// `drawn` is the spots the window placed ([`spot_markers`]' third answer, or
/// read back off a window's `units`) — so a spot hidden under a contact, whose
/// cell IS in the window, is drawn and is not listed. `None` when every spot is
/// drawn or the ground names none.
#[must_use]
pub fn off_window_spots_line(
    spots: &[KeySpot],
    drawn: &[usize],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    // 16b cut AG.1: each clause through `spot_clause`, which the brief's
    // in-window line shares, so the two lines cannot word a spot two ways.
    let parts: Vec<String> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .enumerate()
        .filter(|(index, _)| !drawn.contains(index))
        .map(|(_, (spot, letter))| spot_clause(letter, spot, pose, cell_cm))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("Off the window: {}.", parts.join("; ")))
}

/// **One spot as `VIEW`'s spot lines word it** (16b cut V.1's clause, lifted out
/// by 16b cut AG.1): `<letter> <name>, <range> m at <bearing>°` — the range 2-D
/// to the cell centre ([`spot_range_cm`]) rounded to [`SPOT_RANGE_STEP_M`], the
/// bearing absolute ([`spot_bearing_mdeg`]) in whole degrees. The off-window
/// line and the brief's in-window line both call it.
pub(super) fn spot_clause(
    letter: char,
    spot: &KeySpot,
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> String {
    format!(
        "{letter} {}, {} m at {:03}°",
        spot.name,
        range_m_rounded(spot_range_cm(pose, spot, cell_cm), SPOT_RANGE_STEP_M),
        spot_bearing_mdeg(pose, spot, cell_cm) / 1_000
    )
}

// ---------------------------------------------------------------------------
// The brief (16b cut AG.1; 16c N17)
// ---------------------------------------------------------------------------
//
// `VIEW BRIEF` is a SECOND RENDERER OF THE ONE FRAME (doc 16 §5: one feed, and
// every renderer draws the same structure): the payload `VIEW` would push now,
// said as sentences for a screen reader or a pilot who would rather read. It
// reads exactly what the grid draws — the payload's own rectangle from the
// payload's own `@`, the contacts as `CONTACTS` words them, the spots the
// legend names — and says nothing the grid would not draw or name. Every
// function below is pure and world-free; `view`'s brief arm gathers the frame,
// the ground and the payload and calls [`brief_lines`]. Nothing here crosses
// the arena seam, runs per pulse or adds a frame key: the observer rule (doc
// 16 §1) holds by construction, because every input is the pilot's own frame,
// the surveyed ground (doc 16 §3.2) and a pure function of the two.

/// **The eight bearings a brief walks, in compass order** (16b cut AG.1): the
/// word, then the step in cells. `dy` positive is NORTH — arena y grows north,
/// and `terrain_rows`' row 0 is the window's south edge.
pub const BRIEF_BEARINGS: [(&str, i8, i8); 8] = [
    ("North", 0, 1),
    ("North-east", 1, 1),
    ("East", 1, 0),
    ("South-east", 1, -1),
    ("South", 0, -1),
    ("South-west", -1, -1),
    ("West", -1, 0),
    ("North-west", -1, 1),
];

/// **Root two, per mille** (16b cut AG.1): a diagonal step's length in straight
/// steps, to three places — integer, as every figure in the cockpit is.
pub const DIAGONAL_PER_MILLE: u64 = 1_414;

/// **How far `steps` cells reach, centimetres** (16b cut AG.1):
/// `steps × cell_cm`, times [`DIAGONAL_PER_MILLE`] / 1,000 on a diagonal,
/// saturating. At 1,000 cm cells five diagonal steps are 7,070 cm, six 8,484,
/// fifteen 21,210 and nineteen 26,866.
#[must_use]
pub fn ray_cm(steps: u32, cell_cm: u16, diagonal: bool) -> u64 {
    let per_mille = if diagonal { DIAGONAL_PER_MILLE } else { 1_000 };
    u64::from(steps)
        .saturating_mul(u64::from(cell_cm))
        .saturating_mul(per_mille)
        / 1_000
}

/// **The step a brief's reaches are rounded to**, metres (16b cut AG.1):
/// `cell_cm / 100`, clamped to `1 ..= SPOT_RANGE_STEP_M`.
///
/// At the shipped 1,000 cm cell, and every coarser one, that is the spot step,
/// ten metres. Below it the step is the cell's own, because a run is at least
/// one cell long and two runs may never read the same reach: at 500 cm cells a
/// one-cell run is `5 m` and the next `10 m`, where a fixed ten would read both
/// as `10 m`. The one size it cannot serve is 50 cm — the metre floor can merge
/// two adjacent one-cell reaches — which `RESOLUTIONS_CM` admits and no shipped
/// arena uses.
#[must_use]
pub fn brief_step_m(cell_cm: u16) -> u64 {
    (u64::from(cell_cm) / 100).clamp(1, SPOT_RANGE_STEP_M)
}

/// **A terrain class as a brief says it** (16b cut AG.1): prose for a reader —
/// `heavy woods`, `deep water`, `bare ground` — where the legend keeps
/// `TerrainClass::label` (`woods_heavy`), which is a renderer's key to a glyph
/// and not a word anyone reads aloud. This table is the one place the two
/// vocabularies meet, and it is exhaustive: a new class fails to compile here
/// until it is given its words.
#[must_use]
pub fn class_prose(class: mush_atlas::TerrainClass) -> &'static str {
    use mush_atlas::TerrainClass;
    match class {
        TerrainClass::Void => "nothing",
        TerrainClass::Paved => "paving",
        TerrainClass::Ground => "bare ground",
        TerrainClass::Grass => "grass",
        TerrainClass::Water => "water",
        TerrainClass::Floor => "floor",
        TerrainClass::Rubble => "rubble",
        TerrainClass::Road => "road",
        TerrainClass::Sand => "sand",
        TerrainClass::Rock => "rock",
        TerrainClass::Wall => "wall",
        TerrainClass::Door => "doorway",
        TerrainClass::Platform => "platform",
        TerrainClass::WoodsLight => "light woods",
        TerrainClass::WoodsHeavy => "heavy woods",
        TerrainClass::WaterDeep => "deep water",
        TerrainClass::Ice => "ice",
        TerrainClass::Regolith => "regolith",
    }
}

/// **One run of one class along a bearing** (16b cut AG.1). `steps` is the
/// CUMULATIVE count of steps from the pilot's cell at which the run ends, so a
/// run's reach is [`ray_cm`] of it and the first run's reach is its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundRun {
    /// The ground class of every cell in the run.
    pub class: mush_atlas::TerrainClass,
    /// The step from the pilot's cell at which the run ends.
    pub steps: u32,
}

/// **What stopped a walk** (16b cut AG.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayEnd {
    /// The next cell is on the arena but outside the window: where the map
    /// stops (`beyond the window`).
    Window,
    /// The next cell is off the arena: where the ground stops (`the edge of the
    /// arena`).
    Arena,
}

/// **The ground along one bearing, run by run** (16b cut AG.1).
///
/// From `from` — the pilot's cell, which is not described — the walk steps
/// `step` one cell at a time (checked `i32` arithmetic, then `u16::try_from`).
/// A cell inside the rectangle `origin.0 .. origin.0 + size.0` by
/// `origin.1 .. origin.1 + size.1` AND on the terrain extends it: its class
/// merges into the last run when equal, else opens a new one. The first cell
/// that is not stops it — [`RayEnd::Arena`] when that cell is off the terrain
/// (negative, or `terrain.get` is `None`), [`RayEnd::Window`] otherwise. A zero
/// step walks nowhere and answers no runs and [`RayEnd::Window`]; no bearing of
/// [`BRIEF_BEARINGS`] is one.
///
/// **The bounds are the clamped window's** ([`view_window`]). A pilot at the
/// centre of a 39-by-20 window walks at most 19 cells east or west, **9 north
/// and 10 south** (`you.dy` is 10: rows 0..=9 lie south of the pilot, 11..=19
/// north). A pilot the clamp put at a window's edge walks the whole span — at
/// most 38 cells straight and 19 diagonal — so a line holds at most 38 runs.
/// And with a `view_window` payload a pilot stands on a window's edge only when
/// that edge is the arena's: the clamp `x0 = (cx − 19).min(W − 39)` leaves `cx`
/// on the window's east edge only at `cx = W − 1` and on its west only at
/// `cx = 0`, and `cy` likewise, so a walk with no runs from production always
/// ends [`RayEnd::Arena`].
#[must_use]
pub fn ground_ray(
    terrain: &ArenaTerrain,
    from: (u16, u16),
    step: (i8, i8),
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<GroundRun>, RayEnd) {
    let mut runs: Vec<GroundRun> = Vec::new();
    if step == (0, 0) {
        return (runs, RayEnd::Window);
    }
    let (west, south) = (i32::from(origin.0), i32::from(origin.1));
    let east = west.saturating_add(i32::from(size.0));
    let north = south.saturating_add(i32::from(size.1));
    let (mut x, mut y) = (i32::from(from.0), i32::from(from.1));
    let mut steps: u32 = 0;
    loop {
        let (Some(nx), Some(ny)) = (
            x.checked_add(i32::from(step.0)),
            y.checked_add(i32::from(step.1)),
        ) else {
            return (runs, RayEnd::Arena);
        };
        (x, y) = (nx, ny);
        let cell = match (u16::try_from(x), u16::try_from(y)) {
            (Ok(cx), Ok(cy)) => terrain.get(cx, cy),
            _ => None,
        };
        let Some(cell) = cell else {
            return (runs, RayEnd::Arena);
        };
        if x < west || x >= east || y < south || y >= north {
            return (runs, RayEnd::Window);
        }
        steps = steps.saturating_add(1);
        match runs.last_mut() {
            Some(run) if run.class == cell.class => run.steps = steps,
            _ => runs.push(GroundRun {
                class: cell.class,
                steps,
            }),
        }
    }
}

/// **One bearing's sentence** (16b cut AG.1), verbatim:
///
/// `North: 50 m of grass, then road to 60 m, then grass to 150 m, then beyond
/// the window.`
///
/// The first run states its own length, every later run the reach it ends at,
/// each [`ray_cm`] of its cumulative steps rounded by [`range_m_rounded`] at
/// [`brief_step_m`]; the tail is `then beyond the window.` for
/// [`RayEnd::Window`] (where the map stops) or `then the edge of the arena.`
/// for [`RayEnd::Arena`] (where the ground stops). With no runs at all it is
/// `North: the edge of the arena.` — the one no-runs line a `view_window`
/// payload can produce ([`ground_ray`]) — or `North: beyond the window.`,
/// formatted for completeness over a synthetic rectangle and not one
/// [`view_window`] makes.
#[must_use]
pub fn ground_line(
    bearing: &str,
    runs: &[GroundRun],
    end: RayEnd,
    cell_cm: u16,
    diagonal: bool,
) -> String {
    use std::fmt::Write as _;
    let step = brief_step_m(cell_cm);
    let reach = |run: &GroundRun| range_m_rounded(ray_cm(run.steps, cell_cm, diagonal), step);
    let tail = match end {
        RayEnd::Window => "beyond the window.",
        RayEnd::Arena => "the edge of the arena.",
    };
    let Some((first, later)) = runs.split_first() else {
        return format!("{bearing}: {tail}");
    };
    let mut line = format!(
        "{bearing}: {} m of {}",
        reach(first),
        class_prose(first.class)
    );
    for run in later {
        // Writing to a `String` cannot fail.
        let _ = write!(
            line,
            ", then {} to {} m",
            class_prose(run.class),
            reach(run)
        );
    }
    let _ = write!(line, ", then {tail}");
    line
}

/// **One contact as the brief says it** (16b cut AG.1), verbatim:
///
/// `BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.`
///
/// The row's own fields and nothing else — the designator, the class word
/// (`unknown` or the side), the bearing absolute and off the nose, the band,
/// the range and the age in whole seconds — the very figures
/// [`ContactRow::render`] prints for `CONTACTS`, in a sentence. No lock mark, no
/// friendly mark, no condition, because `CONTACTS` prints none of them.
#[must_use]
pub fn contact_sentence(row: &ContactRow) -> String {
    format!(
        "{}, {}: bearing {:03}° (rel {:03}°), {}, {} m, {} s old.",
        row.designator,
        row.class,
        row.bearing_deg,
        row.relative_deg,
        row.band,
        row.range_m,
        row.age_ms / 1_000
    )
}

/// **The named spots the window shows** (16b cut AG.1), the mirror of
/// [`off_window_spots_line`]: every authored index in `named`, in AUTHORED
/// order (not `named`'s), each through the off-window line's own clause, joined
/// by `; `:
///
/// `In the window: A The Ford, 190 m at 093°.`
///
/// `named` is the spots the window's LEGEND names — `view_window`'s owner rule,
/// the letters a line client prints as `legend: … A The Ford` — and not every
/// drawn spot: a spot drawn and then covered by a contact or the `@` is named
/// in no legend, so the brief does not name it either (the brief says nothing
/// the grid's legend does not; 16b cut AG.1's Q-AG2 default). Such a spot
/// still reaches the pilot through `STATUS`'s nearest-spot line and
/// `RANGE <spot>`. `None` when nothing is named.
#[must_use]
pub fn in_window_spots_line(
    spots: &[KeySpot],
    named: &[usize],
    pose: &mush_sim::data::Pose,
    cell_cm: u16,
) -> Option<String> {
    let parts: Vec<String> = spots
        .iter()
        .zip(SPOT_LETTERS)
        .enumerate()
        .filter(|(index, _)| named.contains(index))
        .map(|(_, (spot, letter))| spot_clause(letter, spot, pose, cell_cm))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("In the window: {}.", parts.join("; ")))
}

/// **Where the window is, in one sentence** (16b cut AG.1), verbatim:
///
/// `Window around RA: cells 0 to 38 east and 0 to 19 north of 96 by 64; you at
/// (6, 4), heading 045°; 10 m a cell.`
///
/// Read off the payload [`view_window`] wrote, never recomputed: `origin_cell`
/// is the window's corner, `width` and `height` its size (the far cell is
/// `corner + size − 1`, saturating), `extent` the arena's size, `origin_cell +
/// you` the pilot's own cell, `cell_cm / 100` the metres a cell. These are the
/// grid footer's facts (`mush_ansi::panel`'s `grid_lines`) in words on a
/// terminal of at least 80 by 24; below that the line client clips the grid
/// and prints the clipped viewport's footer, and the payload — so this
/// header — is unclipped. The callsign and the heading are the frame's two
/// facts, taken as primitives so a test can pin the sentence without a frame.
/// A missing key reads as `0`; never a panic.
#[must_use]
pub fn brief_header(callsign: &str, heading_mdeg: u32, window: &serde_json::Value) -> String {
    let (x0, y0) = window_pair(window, "origin_cell");
    let (dx, dy) = window_pair(window, "you");
    let (arena_w, arena_h) = window_pair(window, "extent");
    let width = window_number(window, "width");
    let height = window_number(window, "height");
    let metres = window_number(window, "cell_cm") / 100;
    format!(
        "Window around {callsign}: cells {x0} to {} east and {y0} to {} north of {arena_w} by \
         {arena_h}; you at ({}, {}), heading {:03}°; {metres} m a cell.",
        x0.saturating_add(width).saturating_sub(1),
        y0.saturating_add(height).saturating_sub(1),
        x0.saturating_add(dx),
        y0.saturating_add(dy),
        heading_mdeg / 1_000
    )
}

/// One whole-number key of a window payload, `0` when it is missing.
pub(super) fn window_number(window: &serde_json::Value, key: &str) -> u64 {
    window
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

/// One `[a, b]` pair of a window payload, each `0` when missing.
pub(super) fn window_pair(window: &serde_json::Value, key: &str) -> (u64, u64) {
    let pair = window.get(key).and_then(serde_json::Value::as_array);
    let at = |n: usize| {
        pair.and_then(|pair| pair.get(n))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    (at(0), at(1))
}

/// **The whole brief** (16b cut AG.1; 16c N17): the payload `VIEW` would push
/// now, said as sentences, in this order —
///
/// 1. [`brief_header`];
/// 2. the eight [`ground_line`]s over [`BRIEF_BEARINGS`], walked by
///    [`ground_ray`] from the payload's own `origin_cell + you` — the cell the
///    grid draws the `@` on, never `terrain.cell_of(&frame.pose)`, which is
///    [`view_window`]'s business — to the payload's own `origin_cell`, `width`
///    and `height`, a bearing diagonal when both its steps are non-zero;
/// 3. one [`contact_sentence`] per row of [`frame_contacts_held`] in frame
///    order, or [`NO_CONTACTS_IN_WINDOW`] when there are none — the brief's
///    own sentence since 16b cut AP.4, so a reader never takes it for a
///    `CONTACTS` reply's `No contacts.`;
/// 4. [`beyond_window_line`], when it says anything;
/// 5. [`in_window_spots_line`] over the spots the payload's legend names;
/// 6. [`off_window_spots_line`] over the spots the payload DREW, as `VIEW`
///    reads it — so a covered spot is in neither spot line, as it is in
///    neither of `VIEW`'s;
/// 7. [`BRIEF_END_LINE`], always, last (16b cut AP.4, 16c `N20`): the
///    brief's fixed close, so a script knows where it ends.
///
/// **Nothing else, ever.** The brief describes the payload's own rectangle
/// from the payload's own `@`, names the legend's own spots, and says nothing
/// the grid would not draw or name; the ground under a marker is surveyed
/// ground the pulled `arena.map` carries, said where the grid draws the glyph.
/// Every sentence is a function of the pilot's own frame, the surveyed ground
/// and the payload, itself a pure function of those two — the observer rule
/// (doc 16 §1) by construction. A payload missing a key reads it as `0` and
/// lets the walk answer; the shape test in `VIEW` refuses such a payload
/// before it gets here.
///
/// **Length**: at most `13 + C` lines for `C` contacts — one header, eight
/// bearings, `max(C, 1)` contact lines, at most one each of the beyond,
/// in-window and off-window lines, and the end line; thirteen at `C = 0`. A
/// platform holds at most one contact per other unit, so the centre's six a
/// side bounds a brief at 24 lines and 24 `Sys` events. **Width**: none. A brief is `Sys` prose that each
/// renderer wraps at its own width; Q-I1's 78-column budget is the grid's, and
/// nothing here is laid out to it. The longest line — 38 alternating one-cell
/// runs from a window's edge — is about 800 characters and wraps as prose.
#[must_use]
pub fn brief_lines(
    frame: &Frame,
    terrain: &ArenaTerrain,
    spots: &[KeySpot],
    window: &serde_json::Value,
    ranges: &BTreeMap<ContactId, u64>,
) -> Vec<String> {
    let cell_cm = terrain.cell_cm();
    let cells = |value: u64| u16::try_from(value).unwrap_or(u16::MAX);
    let (x0, y0) = window_pair(window, "origin_cell");
    let (dx, dy) = window_pair(window, "you");
    let origin = (cells(x0), cells(y0));
    let size = (
        cells(window_number(window, "width")),
        cells(window_number(window, "height")),
    );
    let you = (cells(x0.saturating_add(dx)), cells(y0.saturating_add(dy)));
    let mut lines = vec![brief_header(
        &frame.callsign.to_string(),
        frame.pose.heading_mdeg,
        window,
    )];
    for (bearing, step_x, step_y) in BRIEF_BEARINGS {
        let (runs, end) = ground_ray(terrain, you, (step_x, step_y), origin, size);
        lines.push(ground_line(
            bearing,
            &runs,
            end,
            cell_cm,
            step_x != 0 && step_y != 0,
        ));
    }
    let rows = frame_contacts_held(frame, ranges);
    if rows.is_empty() {
        lines.push(NO_CONTACTS_IN_WINDOW.to_owned());
    }
    lines.extend(rows.iter().map(contact_sentence));
    lines.extend(beyond_window_line(frame, window));
    lines.extend(in_window_spots_line(
        spots,
        &named_spots(window),
        &frame.pose,
        cell_cm,
    ));
    lines.extend(off_window_spots_line(
        spots,
        &drawn_spots(window),
        &frame.pose,
        cell_cm,
    ));
    lines.push(BRIEF_END_LINE.to_owned());
    lines
}

/// **The spot markers a window carries, the legend entries they would earn, and
/// which spots they are** (16b cut V.1), clipped to the window `origin` and
/// `size` exactly as [`contact_markers`] clips, with cells relative to the
/// window's own corner.
///
/// One marker is `{ "id": "A", "name": "The Ford", "cell": [dx, dy], "glyph":
/// "A", "you": false, "spot": true }`: `id` is display text on the grid shape,
/// as a contact marker's is, `name` the authored name, and `spot: true` is what
/// tells it from a contact. One legend entry is `["A", "The Ford"]`; whether it
/// is published is [`view_window`]'s call, after every marker is placed (the
/// owner rule). The third answer is the authored indexes drawn, in order.
///
/// Surveyed ground and nothing else: a spot names no unit.
#[must_use]
pub fn spot_markers(
    spots: &[KeySpot],
    origin: (u16, u16),
    size: (u16, u16),
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>, Vec<usize>) {
    let (x0, y0) = origin;
    let (width, height) = size;
    let mut markers: Vec<serde_json::Value> = Vec::new();
    let mut legend: Vec<serde_json::Value> = Vec::new();
    let mut drawn: Vec<usize> = Vec::new();
    for (index, (spot, letter)) in spots.iter().zip(SPOT_LETTERS).enumerate() {
        if spot.x < x0 || spot.y < y0 {
            continue;
        }
        let (dx, dy) = (spot.x.saturating_sub(x0), spot.y.saturating_sub(y0));
        if dx >= width || dy >= height {
            continue;
        }
        let letter = letter.to_string();
        markers.push(serde_json::json!({
            "id": letter,
            "name": spot.name.as_str(),
            "cell": [u32::from(dx), u32::from(dy)],
            "glyph": letter,
            "you": false,
            "spot": true,
        }));
        legend.push(serde_json::json!([letter, spot.name.as_str()]));
        drawn.push(index);
    }
    (markers, legend, drawn)
}

/// **The spots a composed window drew** (16b cut V.1), read back off its
/// `units` — the markers with `spot: true`, by their letter — so `VIEW`'s
/// off-window line counts the drawing's own spots and cannot drift from it, the
/// way [`beyond_window_line`] counts its contacts.
pub(super) fn drawn_spots(window: &serde_json::Value) -> Vec<usize> {
    spot_marker_walk(window)
        .into_iter()
        .map(|(index, _)| index)
        .collect()
}

/// **The spots a composed window NAMED** (16b cut AG.1): [`drawn_spots`]' walk
/// with one more filter — the markers whose `[id, name]` pair is an entry of the
/// payload's `legend`, which is exactly the spots [`view_window`]'s owner rule
/// named and a line client prints as `legend: … A The Ford`. A spot drawn and
/// then covered by a later marker is in [`drawn_spots`] and NOT here.
///
/// The answer is **authored indexes**, by the marker's letter, never a marker's
/// position in `units`: [`spot_markers`] pushes only the in-window spots, so
/// near The Crossroads on the shipped ground `B` is `units[0]` and authored
/// index 1.
pub(super) fn named_spots(window: &serde_json::Value) -> Vec<usize> {
    let legend = window
        .get("legend")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    spot_marker_walk(window)
        .into_iter()
        .filter(|(_, marker)| match (marker.get("id"), marker.get("name")) {
            (Some(id), Some(name)) => legend.contains(&serde_json::json!([id, name])),
            _ => false,
        })
        .map(|(index, _)| index)
        .collect()
}

/// The one walk [`drawn_spots`] and [`named_spots`] share: every `spot: true`
/// marker of a window's `units`, with the authored index its letter maps to
/// through [`SPOT_LETTERS`]; a marker whose `id` is no spot letter is skipped.
pub(super) fn spot_marker_walk(window: &serde_json::Value) -> Vec<(usize, &serde_json::Value)> {
    window
        .get("units")
        .and_then(serde_json::Value::as_array)
        .map(|units| {
            units
                .iter()
                .filter(|unit| unit.get("spot").and_then(serde_json::Value::as_bool) == Some(true))
                .filter_map(|unit| {
                    let letter = unit
                        .get("id")
                        .and_then(serde_json::Value::as_str)?
                        .chars()
                        .next()?;
                    let index = SPOT_LETTERS.iter().position(|held| *held == letter)?;
                    Some((index, unit))
                })
                .collect()
        })
        .unwrap_or_default()
}
