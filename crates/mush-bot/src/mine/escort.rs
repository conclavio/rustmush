//! **The escort pilot in an IC field** (doc 32 G-4, its second half): a player who leases
//! Occator's armed rig (doc 32 cut CF.6c) and flies it beside a miner. All inputs are this SSH
//! session's terminal text.
//!
//! The lane is the mining pilot's kit-up and hop (enrol, refit, licence, bunkers, the lift)
//! with the escort's lease in place of the prospector's. `+tv/lease escort` previews and
//! `+tv/lease escort go` signs, at Ceres Freeport. Then the operational loan
//! (`+tv/1/permissions go`), whose committed loan `+tv/1/permissions` reads back. On the site the
//! pilot walks to the bay, boards the armed rig's cab (`+tv/1 board`, the printed `+focus`,
//! `@station/handoff` when the focus did not already give control), launches it down the ramp,
//! reads `+tv/1`'s `on the ground at the … ramp`, and drives [`OFF_RAMP`] off the ramp's own cell,
//! which every hull on the pad launches its rigs onto. When a field opens on the site, the
//! escort:
//!
//! 1. reads its own cell from `VIEW BRIEF` (`you at (x, y)`) and takes its ramp
//!    [`OFF_RAMP_CELLS`] west of it: no line names the ramp's cell;
//! 2. finds the raider. Line text names a contact's party but never its relation, so it sets
//!    parties aside: its own (`+field`'s `Your party is C`), its captain's (the captain's page,
//!    `Captain: party A.`), and every party whose contact its `LOCK` reads `(FRIENDLY)` (its
//!    own Dray-P, a standing friend). The one left is the raider, and it pages its captain
//!    `Raider: party D, contact DQ.`;
//! 3. declares, `+field/mark <party>=hostile`, reads `WEAPONSTATUS` for its main gun and
//!    `FIRE`s it on the lock until a round lands (`Hit on …` / `Hit.`), at most [`SHOTS`]
//!    times. A field opens every seat at weapons hold (Q-CF-41), which binds only `FIRE … WHEN`
//!    rules, so a hand `FIRE` is the pilot's own;
//! 4. withdraws: `DRIVE x,y` to the ramp cell, `VIEW BRIEF` until it stands within one cell of
//!    it, then CF.5's exit, `+tv/1 recover go` (`… makes for the ramp …`), and `+tv/1` until the
//!    rig is in the bay;
//! 5. waits out the field (`+field` until no field holds its party), leaves the cab
//!    (`+tv/1 leave`, `+tv/bay out`) and sails home.
//!
//! It types only [`in_vocabulary`]: no staff verb and no `FIRE … WHEN` rule.
use super::{Beat, Kind, Lane, Verdict, lines};
use crate::arena::lines::{Line as ArenaLine, classify};
use serde::{Deserialize, Serialize};

/// How many times the escort fires before it withdraws without a hit.
pub const SHOTS: u32 = 12;
/// A poll's interval, milliseconds.
pub const POLL_MS: u64 = 3_000;
/// How long the escort waits at the ramp for a field, in [`WAIT_POLL_MS`] reads (forty minutes).
pub const WAIT_TRIES: u32 = 480;
/// Its interval, milliseconds.
pub const WAIT_POLL_MS: u64 = 5_000;
/// `tacsim::field::exit::EXIT_ASKED`'s middle: the field lets the unit go at its ramp.
pub const EXIT_ASKED: &str = " makes for the ramp: it leaves field ";
/// `tacsim::port::tv::WHERE_RAMP`'s head: a rig standing at its ramp.
pub const AT_RAMP: &str = "on the ground at the ";
/// A rig in its bay, as `+tv` prints it.
pub const IN_BAY: &str = ", in the bay;";
/// **Off the ramp after the launch**: three cells east, so the ramp's own cell stays free.
pub const OFF_RAMP: &str = "+tv/1 drive e 30 go";
/// The cells that drive moves (30 m at 10 m a cell).
pub const OFF_RAMP_CELLS: u32 = 3;

/// **The own cell `VIEW BRIEF`'s header names** (`tacsim::match_::cockpit::brief_header`):
/// `… you at (131, 124), heading 135°; …`, read off the whole window, because a header past the
/// PTY's columns wraps.
pub fn own_cell(window: &str) -> Option<(u32, u32)> {
    let window = lines::normalise(window);
    let (_, rest) = window.split_once("you at (")?;
    let (pair, _) = rest.split_once(")")?;
    let (x, y) = pair.split_once(", ")?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// **A scored hit, as the shooter is told it** (`tacsim::match_::cockpit::notice_line`'s
/// `Hit` notice): `Hit on BK: hull.`, `Hit on X2.` or `Hit.`; never the victim's
/// `Hit on the …`.
pub fn is_hit_scored(line: &str) -> bool {
    let body = line.trim();
    body == "Hit." || (body.starts_with("Hit on ") && !body.starts_with("Hit on the "))
}

/// What the escort did: the report's `escort` block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The lease's signature.
    pub leased: Option<String>,
    /// The launch's tell and the ramp read.
    pub launched: bool,
    pub at_ramp: bool,
    /// The field's ordinal.
    pub opened: Option<u32>,
    /// The cell it launched to, from `VIEW BRIEF`.
    pub ramp_cell: Option<(u32, u32)>,
    /// Its own party, its captain's (by page), the parties its `LOCK` read `(FRIENDLY)`, and
    /// the contacts it locked to tell them.
    pub own_party: Option<String>,
    pub captain_party: Option<String>,
    pub friendly: Vec<String>,
    pub probed: Vec<String>,
    /// The raider it took and its party.
    pub raider: Option<String>,
    pub raider_party: Option<String>,
    /// The report it paged its captain.
    pub paged: Option<String>,
    pub marked: Option<String>,
    /// The gun it fired, how many times, and the hits it was told of.
    pub weapon: Option<String>,
    pub shots: u32,
    pub hits: u32,
    /// The withdrawal: the field's exit at the ramp, and home in the bay.
    pub exit_asked: Option<String>,
    pub in_bay: bool,
    /// The field settled before the escort sailed.
    pub settled: bool,
}

/// An escort step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Loans,
    ReadLoans,
    Passage,
    Hold,
    Bay,
    Board,
    Focus,
    Handoff,
    Launch,
    Ramp,
    OffRamp,
    Wait,
    Where,
    Status,
    Contacts,
    Probe,
    Page,
    Mark,
    Weapons,
    Lock,
    Fire,
    Drive,
    Beside,
    Exit,
    InBay,
    Settle,
    Leave,
    BayOut,
    /// Back to the airlock, where a hull is left from at home.
    BackPassage,
    Airlock,
}

/// One escort step as the lane's window judges it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Task {
    pub phase: Phase,
}

/// The escort's state.
#[derive(Default)]
pub(super) struct Escort {
    /// The captain it flies for, whom it pages the raider it found (`--escort-for`).
    captain: Option<String>,
    unit: u64,
    opened: bool,
    settled: bool,
    ramp: Option<(u32, u32)>,
    /// Its own party, its captain's (by page), and the parties a `LOCK` read `(FRIENDLY)`.
    own: Option<char>,
    captain_party: Option<char>,
    friendly: Vec<char>,
    /// The contact a `LOCK` is telling, and the raider once told.
    probing: Option<String>,
    raider: Option<String>,
    weapon: Option<String>,
    shots: u32,
    hits: u32,
}

impl Escort {
    /// An escort flying for `captain` (paged the raider it finds), or for nobody.
    pub(super) fn flying_for(captain: Option<String>) -> Escort {
        Escort {
            captain,
            ..Escort::default()
        }
    }
}

/// The classified field contacts in `raw`: designator and party letter.
fn classified(raw: &[String]) -> Vec<(String, char)> {
    raw.iter()
        .filter_map(|line| lines::field_contact(line))
        .filter_map(|(designator, party)| party.map(|party| (designator, party)))
        .collect()
}

/// **The next contact to tell**: the first classified one whose party is not set aside (its
/// own, its captain's, a friend's). With a captain to fly for, none until the captain's page has
/// named its party: before it the captain's contacts are no different from the raider's.
fn candidate(raw: &[String], state: &Escort) -> Option<String> {
    if state.captain.is_some() && state.captain_party.is_none() {
        return None;
    }
    classified(raw)
        .into_iter()
        .find(|(_, party)| {
            Some(*party) != state.own
                && Some(*party) != state.captain_party
                && !state.friendly.contains(party)
        })
        .map(|(designator, _)| designator)
}

/// The escort's main gun in `WEAPONSTATUS`: the weapon with the fewest rounds (the gun, never
/// its coaxial machine gun), by the id `FIRE` takes.
fn main_gun(raw: &[String]) -> Option<String> {
    raw.iter()
        .filter_map(|line| match classify(line) {
            ArenaLine::WeaponRow(row) => Some(row),
            _ => None,
        })
        .min_by_key(|row| row.rounds_max)
        .map(|row| row.id)
}

impl Lane {
    fn escort_state(&mut self) -> &mut Escort {
        self.escort.get_or_insert_default()
    }

    fn escort_evidence(&mut self) -> Option<&mut Evidence> {
        self.report.escort.as_mut()
    }

    /// Whether this lane flies the escort.
    pub(super) fn is_escort(&self) -> bool {
        self.escort.is_some()
    }

    /// The design word the escort's lease names.
    pub(super) fn lease_line(&self, go: bool) -> &'static str {
        match (self.is_escort(), go) {
            (true, false) => "+tv/lease escort",
            (true, true) => "+tv/lease escort go",
            (false, false) => "+tv/lease",
            (false, true) => "+tv/lease go",
        }
    }

    /// The lease signed: the escort's operational loan, before the rest of the kit-up.
    pub(super) fn escort_loans(&mut self, signed: &str) {
        if let Some(evidence) = self.escort_evidence() {
            evidence.leased = Some(signed.to_owned());
        }
        self.escort_send(Phase::Loans, "+tv/1/permissions go", 0);
    }

    /// The hull settled on the site: to the bay and the cab.
    pub(super) fn escort_aboard(&mut self) {
        self.escort_send(Phase::Passage, "in", 0);
    }

    /// **One complete line**, read for the site's field notices.
    pub(super) fn escort_line(&mut self, raw: &str) {
        let site = self.site.name();
        let line = lines::normalise(raw);
        let waiting = matches!(
            &self.step,
            Some(step) if matches!(&step.kind, Kind::Escort(task) if task.phase == Phase::Wait)
        );
        let Some(state) = self.escort.as_mut() else {
            return;
        };
        if !state.opened
            && let Some(ordinal) = lines::field_opened(&line, site)
        {
            state.opened = true;
            if let Some(evidence) = self.report.escort.as_mut() {
                evidence.opened = Some(ordinal);
            }
            self.events.push(super::LaneEvent::Said(format!(
                "field {ordinal} opened on {site}: the escort fights from the ramp"
            )));
            if waiting {
                self.escort_send(Phase::Where, "VIEW BRIEF", 0);
            }
            return;
        }
        if let Some(party) = lines::captain_report(&line) {
            state.captain_party = Some(party);
        }
        if state.opened && lines::field_settled(&line, site).is_some() {
            state.settled = true;
        }
    }

    fn escort_send(&mut self, phase: Phase, line: &str, not_before: u64) {
        if !in_vocabulary(line) {
            self.fail(
                not_before,
                format!("escort command outside its vocabulary: {line}"),
            );
            return;
        }
        let (beat, timeout_ms) = match phase {
            Phase::Loans | Phase::ReadLoans => (Beat::KitUp, super::ANSWER_MS),
            Phase::Launch | Phase::OffRamp => (Beat::OnPad, super::ACT_MS),
            Phase::Wait
            | Phase::Settle
            | Phase::Leave
            | Phase::BayOut
            | Phase::BackPassage
            | Phase::Airlock => (Beat::Home, super::ANSWER_MS),
            _ => (Beat::OnPad, super::ANSWER_MS),
        };
        let beat = match phase {
            Phase::Wait
            | Phase::Where
            | Phase::Status
            | Phase::Contacts
            | Phase::Probe
            | Phase::Mark => Beat::DriveStakeMine,
            Phase::Weapons | Phase::Lock | Phase::Fire | Phase::Drive | Phase::Beside => {
                Beat::DriveStakeMine
            }
            _ => beat,
        };
        self.send(beat, Kind::Escort(Task { phase }), line, not_before);
        if let Some(step) = self.step.as_mut() {
            step.timeout_ms = timeout_ms;
        }
    }

    /// What an escort step's window says.
    pub(super) fn escort_verdict(
        &self,
        task: &Task,
        opened: u64,
        text: &str,
        raw: &[String],
        quiet: bool,
        now: u64,
    ) -> Verdict {
        let Some(state) = self.escort.as_ref() else {
            return Verdict::Fail("an escort step with no escort".into());
        };
        let poll = |needle: &'static str, wait: u64, tries: u32| {
            if quiet {
                Verdict::Retry {
                    needle,
                    wait,
                    tries,
                }
            } else {
                Verdict::Wait
            }
        };
        let done_if = |yes: bool| {
            if yes && quiet {
                Verdict::Done
            } else {
                Verdict::Wait
            }
        };
        match task.phase {
            Phase::Loans => done_if(lines::has(text, "Occator is recording")),
            Phase::ReadLoans => {
                if lines::has(text, "operate_crew allowed now") && quiet {
                    Verdict::Done
                } else {
                    poll("the operating loan is not in force yet", POLL_MS, 20)
                }
            }
            Phase::Passage => done_if(lines::has(text, "Central Passage")),
            Phase::Hold => done_if(lines::has(text, "Cargo Hold")),
            Phase::Bay => done_if(lines::has(text, "entered vehicle bay")),
            Phase::Board => done_if(lines::has(text, "Choose +focus")),
            Phase::Focus => done_if(quiet && now >= opened.saturating_add(1_000)),
            Phase::Handoff => {
                if lines::has(text, "You take foreground control") && quiet {
                    Verdict::Done
                } else if quiet && now >= opened.saturating_add(3_000) {
                    Verdict::Retry {
                        needle: "the cab's control has not come",
                        wait: POLL_MS,
                        tries: 10,
                    }
                } else {
                    Verdict::Wait
                }
            }
            Phase::Launch => {
                if lines::has(text, lines::LAUNCHED) && quiet {
                    Verdict::Done
                } else if lines::has(text, lines::COLD) && quiet {
                    poll("the ground is still being mapped", POLL_MS, 20)
                } else {
                    Verdict::Wait
                }
            }
            Phase::Ramp => {
                if lines::has(text, AT_RAMP) && quiet {
                    Verdict::Done
                } else {
                    poll("the rig is not at its ramp yet", POLL_MS, 20)
                }
            }
            Phase::OffRamp => {
                done_if(lines::has(text, lines::ARRIVED) || lines::has(text, lines::DRIVE_HERE))
            }
            // The field's open notice moves the lane on (`escort_line`); until then, `+tv/1`.
            Phase::Wait => {
                if state.opened {
                    Verdict::Done
                } else {
                    poll("no field has opened", WAIT_POLL_MS, WAIT_TRIES)
                }
            }
            Phase::Where => {
                let you = own_cell(text);
                if you.is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("VIEW BRIEF printed no header yet", POLL_MS, 20)
                }
            }
            Phase::Status => {
                if lines::own_party(text).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("no party of ours in the field yet", POLL_MS, 30)
                }
            }
            Phase::Contacts => {
                if candidate(raw, state).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("no contact to tell classified yet", POLL_MS, 80)
                }
            }
            Phase::Page => done_if(lines::has(text, "You paged ")),
            // Each read waits for its own answer: a field's notices keep arriving in the window,
            // so a quiet window alone is not the answer.
            Phase::Probe => {
                done_if(text.contains("Locked on ") || text.contains("No such contact"))
            }
            Phase::Mark => {
                if lines::has(text, lines::PARTY_MARKED) && quiet {
                    Verdict::Done
                } else if text.contains("No party in your field") || text.contains("Usage") {
                    Verdict::Fail(format!("the mark was refused: {text}"))
                } else {
                    Verdict::Wait
                }
            }
            Phase::Weapons => {
                if text.contains("This unit carries no weapon") && quiet {
                    Verdict::Fail(format!("the escort's hull carries no weapon now: {text}"))
                } else {
                    done_if(main_gun(raw).is_some())
                }
            }
            Phase::Lock => done_if(
                text.contains("Locked on ")
                    || text.contains("No such contact")
                    || now >= opened.saturating_add(POLL_MS),
            ),
            Phase::Fire => {
                if raw.iter().any(|line| is_hit_scored(line)) && quiet {
                    Verdict::Done
                } else {
                    done_if(now >= opened.saturating_add(POLL_MS))
                }
            }
            Phase::Drive => done_if(true),
            Phase::Beside => {
                let you = own_cell(text);
                let near = you
                    .zip(state.ramp)
                    .is_some_and(|((x, y), (rx, ry))| x.abs_diff(rx) <= 1 && y.abs_diff(ry) <= 1);
                if near && quiet {
                    Verdict::Done
                } else {
                    poll("the escort is not beside its ramp yet", POLL_MS, 60)
                }
            }
            Phase::Exit => {
                done_if(lines::has(text, EXIT_ASKED) || lines::has(text, lines::RIG_BACK))
            }
            Phase::InBay => {
                if lines::has(text, IN_BAY) && quiet {
                    Verdict::Done
                } else {
                    poll("the escort is not in the bay yet", POLL_MS, 60)
                }
            }
            Phase::Settle => {
                if (state.settled || lines::has(text, lines::NO_FIELD)) && quiet {
                    Verdict::Done
                } else {
                    poll("the field has not settled", 5_000, 480)
                }
            }
            Phase::Leave => done_if(lines::has(text, "give back your rig billet")),
            Phase::BayOut => done_if(lines::has(text, "returned to cargo hold")),
            Phase::BackPassage => done_if(lines::has(text, "Central Passage")),
            Phase::Airlock => done_if(lines::has(text, "Airlock")),
        }
    }

    /// An escort step decided: the next one.
    #[allow(clippy::too_many_lines)] // one arm a step, in the lane's order
    pub(super) fn escort_next(
        &mut self,
        phase: Phase,
        raw: &[String],
        text: &str,
        now: u64,
    ) -> Result<(), String> {
        match phase {
            Phase::Loans => self.escort_send(Phase::ReadLoans, "+tv/1/permissions", 0),
            // The rest of the kit-up is the mining pilot's: the rigs, the chart, the bunkers.
            Phase::ReadLoans => self.send(Beat::KitUp, Kind::Rigs, "+tv", 0),
            Phase::Passage => self.escort_send(Phase::Hold, "hold", 0),
            Phase::Hold => self.escort_send(Phase::Bay, "+tv/bay in", 0),
            Phase::Bay => self.escort_send(Phase::Board, "+tv/1 board", 0),
            Phase::Board => {
                let focus = text
                    .split_once("Choose ")
                    .and_then(|(_, rest)| rest.split_once(" then "))
                    .map(|(focus, _)| focus.trim().to_owned())
                    .ok_or("the cab printed no +focus")?;
                let unit = focus
                    .strip_prefix("+focus #")
                    .and_then(|rest| rest.strip_suffix("/pilot"))
                    .and_then(|n| n.parse::<u64>().ok())
                    .ok_or_else(|| format!("an unreadable focus: {focus}"))?;
                self.escort_state().unit = unit;
                self.escort_send(Phase::Focus, &focus, 0);
            }
            // A cab whose billet is the lessee's gives control on the focus itself; the handoff
            // is only for a control the focus did not take.
            Phase::Focus if lines::has(text, "You take foreground control") => {
                self.escort_send(Phase::Launch, "+tv/1 launch go", 0);
            }
            Phase::Focus => {
                let unit = self.escort_state().unit;
                self.escort_send(
                    Phase::Handoff,
                    &format!("@station/handoff #{unit}/pilot"),
                    0,
                );
            }
            Phase::Handoff => self.escort_send(Phase::Launch, "+tv/1 launch go", 0),
            Phase::Launch => {
                self.milestone(super::Milestone::Launched, now);
                if let Some(evidence) = self.escort_evidence() {
                    evidence.launched = true;
                }
                self.escort_send(Phase::Ramp, "+tv/1", 0);
            }
            // Off the ramp's own cell, which every rig of every hull on the pad launches onto: a
            // field holds one unit a cell, and a second there refuses the whole field.
            Phase::Ramp => {
                if let Some(evidence) = self.escort_evidence() {
                    evidence.at_ramp = true;
                }
                self.escort_send(Phase::OffRamp, OFF_RAMP, 0);
            }
            Phase::OffRamp => {
                if self.escort_state().opened {
                    self.escort_send(Phase::Where, "VIEW BRIEF", 0);
                } else {
                    self.escort_send(Phase::Wait, "+tv/1", now.saturating_add(WAIT_POLL_MS));
                }
            }
            Phase::Wait => self.escort_send(Phase::Where, "VIEW BRIEF", 0),
            Phase::Where => {
                let (x, y) = own_cell(text).ok_or("no VIEW BRIEF header")?;
                // The ramp is where it launched: OFF_RAMP_CELLS west of where it stands.
                let ramp = (
                    x.checked_sub(OFF_RAMP_CELLS)
                        .ok_or("the escort stands west of its ramp")?,
                    y,
                );
                self.escort_state().ramp = Some(ramp);
                if let Some(evidence) = self.escort_evidence() {
                    evidence.ramp_cell = Some(ramp);
                }
                self.escort_send(Phase::Status, "+field", 0);
            }
            Phase::Status => {
                let own = lines::own_party(text).ok_or("no party letter in +field")?;
                self.escort_state().own = Some(own);
                if let Some(evidence) = self.escort_evidence() {
                    evidence.own_party = Some(own.to_string());
                }
                self.escort_send(Phase::Contacts, "CONTACTS", 0);
            }
            Phase::Contacts => {
                let looked = {
                    let state = self.escort_state();
                    candidate(raw, state).ok_or("no contact to tell")?
                };
                let captain_party = {
                    let state = self.escort_state();
                    state.probing = Some(looked.clone());
                    state.captain_party
                };
                if let Some(evidence) = self.escort_evidence() {
                    evidence.probed.push(looked.clone());
                    evidence.captain_party = captain_party.map(|p| p.to_string());
                }
                self.escort_send(Phase::Probe, &format!("LOCK {looked}"), 0);
            }
            // The lock tells a friend from the rest: its own Dray-P is a standing friend.
            Phase::Probe => {
                let looked = self
                    .escort_state()
                    .probing
                    .take()
                    .ok_or("no contact told")?;
                let party = looked.chars().next().ok_or("an empty designator")?;
                if text.contains("No such contact") {
                    self.escort_send(Phase::Contacts, "CONTACTS", now.saturating_add(POLL_MS));
                } else if lines::lock_is_friendly(text) {
                    self.escort_state().friendly.push(party);
                    if let Some(evidence) = self.escort_evidence() {
                        evidence.friendly.push(party.to_string());
                    }
                    self.escort_send(Phase::Contacts, "CONTACTS", 0);
                } else {
                    self.escort_state().raider = Some(looked.clone());
                    if let Some(evidence) = self.escort_evidence() {
                        evidence.raider = Some(looked.clone());
                        evidence.raider_party = Some(party.to_string());
                    }
                    let report = lines::RAIDER_REPORT
                        .replace("{party}", &party.to_string())
                        .replace("{designator}", &looked);
                    match self.escort_state().captain.clone() {
                        Some(captain) => {
                            if let Some(evidence) = self.escort_evidence() {
                                evidence.paged = Some(report.clone());
                            }
                            self.escort_send(Phase::Page, &format!("page {captain}={report}"), 0);
                        }
                        None => self.escort_mark(),
                    }
                }
            }
            Phase::Page => self.escort_mark(),
            Phase::Mark => {
                let ack = text
                    .split_once(lines::PARTY_MARKED)
                    .and_then(|(_, rest)| rest.split_once('.'))
                    .map(|(mark, _)| format!("{}{mark}.", lines::PARTY_MARKED));
                if let Some(evidence) = self.escort_evidence() {
                    evidence.marked = ack;
                }
                self.escort_send(Phase::Weapons, "WEAPONSTATUS", 0);
            }
            Phase::Weapons => {
                let gun = main_gun(raw).ok_or("WEAPONSTATUS named no weapon")?;
                self.escort_state().weapon = Some(gun.clone());
                if let Some(evidence) = self.escort_evidence() {
                    evidence.weapon = Some(gun);
                }
                self.escort_lock();
            }
            Phase::Lock => {
                if text.contains("No such contact") {
                    // The raider's contact is gone: another look.
                    self.escort_state().raider = None;
                    self.escort_send(Phase::Contacts, "CONTACTS", now.saturating_add(POLL_MS));
                } else {
                    self.escort_fire(now, 0);
                }
            }
            Phase::Fire => {
                let hits = u32::try_from(raw.iter().filter(|line| is_hit_scored(line)).count())
                    .unwrap_or(u32::MAX);
                let (shots, total) = {
                    let state = self.escort_state();
                    state.shots = state.shots.saturating_add(1);
                    state.hits = state.hits.saturating_add(hits);
                    (state.shots, state.hits)
                };
                if let Some(evidence) = self.escort_evidence() {
                    evidence.shots = shots;
                    evidence.hits = total;
                }
                if text.contains("No lock") {
                    self.escort_lock();
                } else if total > 0 || shots >= SHOTS {
                    self.escort_withdraw();
                } else {
                    self.escort_fire(now, POLL_MS);
                }
            }
            Phase::Drive => {
                self.escort_send(Phase::Beside, "VIEW BRIEF", now.saturating_add(POLL_MS));
            }
            Phase::Beside => self.escort_send(Phase::Exit, "+tv/1 recover go", 0),
            Phase::Exit => {
                if let Some(evidence) = self.escort_evidence() {
                    evidence.exit_asked = raw
                        .iter()
                        .find(|line| line.contains(EXIT_ASKED.trim()))
                        .cloned();
                }
                self.escort_send(Phase::InBay, "+tv/1", now.saturating_add(POLL_MS));
            }
            Phase::InBay => {
                if let Some(evidence) = self.escort_evidence() {
                    evidence.in_bay = true;
                }
                self.milestone(super::Milestone::Recovered, now);
                self.escort_send(Phase::Settle, "+field", 0);
            }
            Phase::Settle => {
                self.escort_state().settled = true;
                if let Some(evidence) = self.escort_evidence() {
                    evidence.settled = true;
                }
                self.escort_send(Phase::Leave, "+tv/1 leave", 0);
            }
            Phase::Leave => self.escort_send(Phase::BayOut, "+tv/bay out", 0),
            // `+ship/leave` at home is typed at the airlock, as the hired-crew expedition walks it.
            Phase::BayOut => self.escort_send(Phase::BackPassage, "passage", 0),
            Phase::BackPassage => self.escort_send(Phase::Airlock, "airlock", 0),
            Phase::Airlock => self.send(Beat::Home, Kind::PlanHome, "+ship/plan ceres", 0),
        }
        Ok(())
    }

    /// `+field/mark <party>=hostile` on the raider's party.
    fn escort_mark(&mut self) {
        let Some(party) = self
            .escort_state()
            .raider
            .as_ref()
            .and_then(|d| d.chars().next())
        else {
            self.fail(0, "a mark with no raider".into());
            return;
        };
        self.escort_send(Phase::Mark, &format!("+field/mark {party}=hostile"), 0);
    }

    fn escort_lock(&mut self) {
        let Some(raider) = self.escort_state().raider.clone() else {
            self.fail(0, "a lock with no raider".into());
            return;
        };
        self.escort_send(Phase::Lock, &format!("LOCK {raider}"), 0);
    }

    fn escort_fire(&mut self, now: u64, after: u64) {
        let Some(gun) = self.escort_state().weapon.clone() else {
            self.fail(now, "a shot with no gun".into());
            return;
        };
        self.escort_send(
            Phase::Fire,
            &format!("FIRE {gun}"),
            now.saturating_add(after),
        );
    }

    /// Back to the ramp cell it launched to.
    fn escort_withdraw(&mut self) {
        let Some((x, y)) = self.escort_state().ramp else {
            self.fail(0, "a withdrawal with no ramp cell".into());
            return;
        };
        // For the ramp's own cell: the motor stops short of a cell another hull's rig stands on,
        // and any cell within one of the ramp is CF.5's exit.
        self.escort_send(Phase::Drive, &format!("DRIVE {x},{y}"), 0);
    }

    /// **Home and docked**: the escort has nothing to sell, so the run ends a success there.
    pub(super) fn escort_home(&mut self, now: u64) {
        self.report.success = true;
        self.milestone(super::Milestone::DockedHome, now);
        self.beat = Beat::Done;
        let (shots, hits) = self
            .escort
            .as_ref()
            .map_or((0, 0), |state| (state.shots, state.hits));
        let outcome = format!(
            "escorted at {}: {shots} shots, {hits} landed, withdrawn and home",
            self.site.name()
        );
        self.events.push(super::LaneEvent::Said(outcome.clone()));
        self.result = Some(super::LaneResult {
            status: "done".to_owned(),
            outcome,
        });
    }
}

/// **The escort's lines**: the mining pilot's vocabulary and the escort's own, each in closed
/// form.
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 512 || line.contains(['\n', '\r']) || line.starts_with("@channel") {
        return false;
    }
    if super::in_vocabulary(line) {
        return true;
    }
    let digits = |s: &str| !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit());
    if [
        "+tv/lease escort",
        "+tv/lease escort go",
        "+tv/1/permissions go",
        "+tv/1/permissions",
        "in",
        "hold",
        "passage",
        "airlock",
        "+tv/bay in",
        "+tv/bay out",
        "+tv/1 board",
        "+tv/1 leave",
        OFF_RAMP,
        "+tv/1",
        "+field",
        "VIEW BRIEF",
        "CONTACTS",
        "WEAPONSTATUS",
    ]
    .contains(&line)
    {
        return true;
    }
    // The one page: the raider's report to the captain, by name.
    if let Some((name, report)) = line.strip_prefix("page ").and_then(|r| r.split_once('=')) {
        return (1..=32).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_alphanumeric())
            && lines::raider_report(&format!("x{}{report}", lines::PAGED)).is_some();
    }
    for prefix in ["+focus #", "@station/handoff #"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.strip_suffix("/pilot").is_some_and(digits);
        }
    }
    if let Some(rest) = line.strip_prefix("+field/mark ") {
        return rest
            .strip_suffix("=hostile")
            .is_some_and(|letter| letter.len() == 1 && lines::PARTY_LETTERS.contains(letter));
    }
    if let Some(rest) = line.strip_prefix("LOCK ") {
        return lines::is_field_designator(rest);
    }
    if let Some(gun) = line.strip_prefix("FIRE ") {
        return (1..=48).contains(&gun.len())
            && gun
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    }
    if let Some(cell) = line.strip_prefix("DRIVE ") {
        return cell
            .split_once(',')
            .is_some_and(|(x, y)| x.len() <= 4 && y.len() <= 4 && digits(x) && digits(y));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_escorts_verbs_are_closed() {
        for good in [
            "+tv/lease escort",
            "+tv/lease escort go",
            "+tv/1 board",
            "+focus #546/pilot",
            "@station/handoff #546/pilot",
            "+tv/1 launch go",
            "VIEW BRIEF",
            "CONTACTS",
            "+field/mark B=hostile",
            "WEAPONSTATUS",
            "LOCK BK",
            "FIRE cannon",
            "DRIVE 128,124",
            "+tv/1 drive e 30 go",
            "+tv/1 recover go",
            "+tv/1 leave",
            "+ship/lift ceres",
            "page Fieldcap=Raider: party B, contact BK.",
        ] {
            assert!(in_vocabulary(good), "{good}");
        }
        for bad in [
            "FIRE cannon WHEN hostile",
            "FIRE ALL",
            "FIRE cannon OFF",
            "DRIVE here",
            "DRIVE 1,2,3",
            "LOCK bk",
            "SCAN BK\n@destroy #1",
            "+field/fire E",
            "+field/mark B=friendly",
            "@field/raiders tacsim.raider.ahuna_skulks",
            "@station/handoff #546/gunner",
            "ORDER DT:1:1:1 1 WITHDRAW",
            "page Fieldcap=hello",
            "page Fieldcap=Raider: party B, contact AK.",
            "page Field cap=Raider: party B, contact BK.",
        ] {
            assert!(!in_vocabulary(bad), "{bad}");
        }
    }

    #[test]
    fn the_escort_reads_its_hits_scans_and_gun() {
        let wrapped = "Window around CL: cells 112 to 150 east and 114 to 133 north of 256 by 256; you at \n                       (131, 124), heading 135°; 10 m a cell.";
        assert_eq!(own_cell(wrapped), Some((131, 124)));
        assert_eq!(own_cell("No contacts in the window."), None);
        assert!(is_hit_scored("Hit on BK: hull."));
        assert!(is_hit_scored("Hit on X2."));
        assert!(is_hit_scored("Hit."));
        assert!(!is_hit_scored(
            "Hit on the hull's front facet: 280 of 400 armour left."
        ));
        let rows = [
            "cannon (105)  ready  38 of 40 round(s)".to_owned(),
            "machine_gun (mg)  ready  380 of 500 round(s)".to_owned(),
        ];
        assert_eq!(main_gun(&rows).as_deref(), Some("cannon"), "{rows:?}");
    }
}
