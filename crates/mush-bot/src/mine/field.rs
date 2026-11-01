//! **The captain in an IC field** (doc 32 G-4): the hired-crew expedition under a combat
//! contract, its captain riding rig 1's cab. All inputs are this SSH session's terminal text.
//!
//! The expedition ([`super::detachment`]) hires Rook Adeyemi under a combat contract, seats him
//! in rig 2 as the roster's slot 1 and the captain in rig 1's cab as slot 2, and launches both.
//! Rig 1 then works its block on the solo lane's own beats, scan to run, on open ground (a
//! staked claim needs its captain aboard the carrier, and this one rides the rig). When a field
//! opens on the site ([`lines::field_opened`], told to everyone on it), the step in hand is
//! dropped and the captain:
//!
//! 1. reads its own party's letter (`+field`);
//! 2. reads `CONTACTS` from the cab until a row classifies into a party that is not its own
//!    (the rows are `X1 unknown` until the rig's sensors have dwelt on them), and takes that
//!    party for the raider: the expedition is one party, so on a site with nobody else the
//!    other party is the one that opened the field;
//! 3. declares, `+field/mark <party>=hostile`;
//! 4. orders its escort, `ORDER DT:… 1 ENGAGE <designator>`, and reads `+sortie/orders` until
//!    the order's reply arrives: a NEGATIVE (the escort does not hold that contact) sends it
//!    back to step 2 for another, at most [`ENGAGE_TRIES`] times;
//! 5. holds, reading `CONTACTS` ([`HOLDS`] reads, [`HOLD_MS`] apart), then orders
//!    `ORDER DT:… 1 WITHDRAW` and reads its reply the same way;
//! 6. holds until the field settles (`+field` answers [`lines::NO_FIELD`], or the settled
//!    notice arrives), and resumes its fill plan: the run the field stopped, or the scan again.
//!
//! The lane's run after that takes it home: rig 1's own recover (which drives it home from its
//! pocket and closes the captain's flight), then the expedition's walk back, close and lift. A lift the bay gate refuses for a
//! unit still out is typed again with `leave=` naming what the refusal named.
//!
//! The bot types only [`in_vocabulary`]: no staff verb, nothing a model wrote.
use super::{Beat, Kind, Lane, Verdict, lines};
use serde::{Deserialize, Serialize};

/// How many ENGAGE orders the captain gives before it stops looking for a contact its escort
/// holds.
pub const ENGAGE_TRIES: u32 = 5;
/// How many `CONTACTS` reads the captain holds between the escort's WILCO and its WITHDRAW.
pub const HOLDS: u32 = 1;
/// How far apart they are, milliseconds.
pub const HOLD_MS: u64 = 5_000;
/// How long a contact may take to classify, in reads [`POLL_MS`] apart (three minutes).
pub const CONTACT_TRIES: u32 = 60;
/// A poll's interval, milliseconds.
pub const POLL_MS: u64 = 3_000;
/// How many `+field` reads the captain waits for the field to settle (forty minutes).
pub const SETTLE_TRIES: u32 = 480;
/// Their interval, milliseconds.
pub const SETTLE_POLL_MS: u64 = 5_000;

/// What the field showed the captain: the report's `field` block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The field's ordinal, from the site's notice.
    pub opened: Option<u32>,
    /// The captain's own party letter.
    pub own_party: Option<String>,
    /// The party it took for the raider, and the designator it ordered the escort onto.
    pub raider_party: Option<String>,
    pub raider: Option<String>,
    /// The escort's report the captain took it from, when one came (`B BK`).
    pub reported: Option<String>,
    /// The mark's acknowledgement.
    pub marked: Option<String>,
    /// The ENGAGE answered WILCO, and how many were answered NEGATIVE before it.
    pub engage_wilco: bool,
    pub negatives: u32,
    /// The WITHDRAW answered WILCO.
    pub withdraw_wilco: bool,
    /// The WITHDRAW's reply when it was not WILCO.
    pub withdraw_reply: Option<String>,
    /// The `CONTACTS` reads while it held.
    pub holds: u32,
    /// The field settled.
    pub settled: bool,
    /// The fill plan resumed after it.
    pub resumed: bool,
    /// What the departure left behind (`leave=`'s value), when the bay gate asked.
    pub left_behind: Option<String>,
    /// The bay gate's refusal of the plain lift.
    pub lift_refused: Option<String>,
    /// Rig 1's run after the field, launched again on its own: its last cycle's line.
    pub refill_run: Option<String>,
    /// `+tv`'s line for rig 1 once the captain is home: the rig the departure left.
    pub left_home: Option<String>,
}

/// A field step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Status,
    /// The captain's own party paged to its escort.
    PageEscort,
    Contacts,
    Mark,
    Engage,
    EngageReply,
    Hold,
    Withdraw,
    WithdrawReply,
    Settle,
    /// Rig 1 down the ramp again on its own, its drive to a pocket, and its run: the fill plan
    /// after the field.
    Relaunch,
    Redrive,
    Rerun,
    /// `+tv` at home: what the departure left on the site.
    LeftRead,
}

/// One field step as the lane's window judges it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Task {
    pub phase: Phase,
}

/// The captain's field state.
#[derive(Default)]
pub(super) struct Field {
    /// `ORDER DT:a:b:c`, once `+sortie/orders` printed it: the field watch is armed from then.
    pub fence: String,
    /// The escort it flies with, whom it pages its own party (`--escorted-by`).
    pub escort: Option<String>,
    /// The `+sortie/…` operation selector.
    pub operation: String,
    opened: bool,
    settled: bool,
    own: Option<char>,
    raider: Option<(char, String)>,
    /// The escort's report of the raider, by page (`lines::raider_report`).
    reported: Option<(char, String)>,
    marked: bool,
    /// The highest order sequence seen before the order in hand went.
    floor: u64,
    engages: u32,
    holds: u32,
    /// The pockets rig 1 has tried after the field, and the one in hand.
    refill_tried: u32,
    refill_addr: Option<String>,
}

impl Field {
    /// A captain flying with `escort` (paged its own party), or alone.
    pub fn with_escort(escort: Option<String>) -> Field {
        Field {
            escort,
            ..Field::default()
        }
    }

    /// Whether the field came and went.
    pub fn done(&self) -> bool {
        self.opened && self.settled
    }
}

/// How many units one departure may leave behind before the captain stops asking.
pub const LEAVE_TRIES: usize = 4;
/// How many pockets rig 1 tries after the field before the fill plan gives up.
pub const REFILL_POCKETS: u32 = 3;
/// `tacsim::port::tv::RUN_DONE`'s head: a run's last cycle line ends with it.
pub const RUN_DONE_HEAD: &str = " The run at ";

/// **The raider in a `CONTACTS` read**: the escort's report when it names a party the read
/// holds (its designator when the read holds that too); else the one other party, when
/// exactly one other has classified. Several other parties and no report is no answer yet:
/// line text gives no relation, so the captain waits for its escort's word.
fn raider_in(
    raw: &[String],
    own: Option<char>,
    reported: Option<&(char, String)>,
) -> Option<(String, char)> {
    let others: Vec<(String, char)> = raw
        .iter()
        .filter_map(|line| lines::field_contact(line))
        .filter_map(|(designator, party)| party.map(|party| (designator, party)))
        .filter(|(_, party)| Some(*party) != own)
        .collect();
    if let Some((party, designator)) = reported {
        return others
            .iter()
            .find(|(d, _)| d == designator)
            .or_else(|| others.iter().find(|(_, p)| p == party))
            .cloned();
    }
    let mut parties: Vec<char> = others.iter().map(|(_, p)| *p).collect();
    parties.sort_unstable();
    parties.dedup();
    (parties.len() == 1).then(|| others[0].clone())
}

/// The highest order sequence `window` lists for slot 1, else `floor`.
fn top(window: &str, floor: u64) -> u64 {
    lines::order_replies(window, 1)
        .into_iter()
        .map(|(sequence, _)| sequence)
        .fold(floor, u64::max)
}

/// The reply to the first order above `floor`, once one arrived.
fn reply_above(window: &str, floor: u64) -> Option<String> {
    lines::order_replies(window, 1)
        .into_iter()
        .filter(|(sequence, _)| *sequence > floor)
        .find_map(|(_, word)| word)
}

impl Lane {
    fn field_state(&mut self) -> &mut Field {
        self.field.get_or_insert_default()
    }

    fn field_evidence(&mut self) -> Option<&mut Evidence> {
        self.report.field.as_mut()
    }

    /// **Arm the field watch**: the escort's fence and the operation, then the block's work.
    pub(super) fn field_armed(&mut self, fence: String, operation: String) {
        let state = self.field_state();
        state.fence = fence;
        state.operation = operation;
    }

    /// **One complete line**, read for the site's field notices before any window sees it. The
    /// open notice drops the step in hand and starts the field's steps.
    pub(super) fn field_line(&mut self, raw: &str) {
        let site = self.site.name();
        let line = lines::normalise(raw);
        let Some(state) = self.field.as_mut() else {
            return;
        };
        if state.fence.is_empty() {
            return;
        }
        if !state.opened
            && let Some(ordinal) = lines::field_opened(&line, site)
        {
            state.opened = true;
            if let Some(evidence) = self.report.field.as_mut() {
                evidence.opened = Some(ordinal);
            }
            self.events.push(super::LaneEvent::Said(format!(
                "field {ordinal} opened on {site}: the captain holds the fill plan"
            )));
            self.field_send(Phase::Status, "+field", 0);
            return;
        }
        if let Some(report) = lines::raider_report(&line) {
            state.reported = Some(report);
        }
        if state.opened && !state.settled && lines::field_settled(&line, site).is_some() {
            state.settled = true;
        }
    }

    fn field_send(&mut self, phase: Phase, line: &str, not_before: u64) {
        if !in_vocabulary(line) {
            self.fail(
                not_before,
                format!("field command outside its vocabulary: {line}"),
            );
            return;
        }
        let (beat, timeout_ms) = match phase {
            Phase::Settle => (Beat::DriveStakeMine, super::ANSWER_MS),
            Phase::Relaunch | Phase::Redrive => (Beat::DriveStakeMine, super::ACT_MS),
            Phase::Rerun => (Beat::DriveStakeMine, super::RUN_MS),
            Phase::LeftRead => (Beat::Home, super::ANSWER_MS),
            _ => (Beat::OnPad, super::ANSWER_MS),
        };
        self.send(beat, Kind::Field(Task { phase }), line, not_before);
        if let Some(step) = self.step.as_mut() {
            step.timeout_ms = timeout_ms;
        }
    }

    /// What a field step's window says.
    pub(super) fn field_verdict(
        &self,
        task: &Task,
        opened: u64,
        text: &str,
        raw: &[String],
        quiet: bool,
        now: u64,
    ) -> Verdict {
        let Some(state) = self.field.as_ref() else {
            return Verdict::Fail("a field step with no field".into());
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
        match task.phase {
            Phase::Status => {
                if lines::own_party(text).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("no party of ours in the field yet", POLL_MS, 30)
                }
            }
            Phase::Contacts => {
                if raider_in(raw, state.own, state.reported.as_ref()).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll(
                        "no contact of another party classified yet",
                        POLL_MS,
                        CONTACT_TRIES,
                    )
                }
            }
            Phase::PageEscort => {
                if lines::has(text, "You paged ") && quiet {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::Mark => {
                if lines::has(text, lines::PARTY_MARKED) && quiet {
                    Verdict::Done
                } else if lines::has(text, "No party in your field has that letter.") {
                    Verdict::Fail(format!("the mark found no such party: {text}"))
                } else {
                    Verdict::Wait
                }
            }
            Phase::Engage | Phase::Withdraw => {
                if lines::has(text, "Usage") || lines::has(text, "Field order") {
                    Verdict::Fail(format!("the order was refused: {text}"))
                } else if quiet || now >= opened.saturating_add(2_000) {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::EngageReply | Phase::WithdrawReply => {
                if reply_above(text, state.floor).is_some() && quiet {
                    Verdict::Done
                } else {
                    poll("the escort's reply has not arrived", POLL_MS, 60)
                }
            }
            Phase::Hold => {
                if quiet {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::Settle => {
                if (state.settled || lines::has(text, lines::NO_FIELD)) && quiet {
                    Verdict::Done
                } else {
                    poll("the field has not settled", SETTLE_POLL_MS, SETTLE_TRIES)
                }
            }
            Phase::Relaunch => {
                if lines::has(text, lines::LAUNCHED) && quiet {
                    Verdict::Done
                } else if lines::has(text, lines::COLD) && quiet {
                    poll("the ground is still being mapped", POLL_MS, 20)
                } else {
                    Verdict::Wait
                }
            }
            Phase::Redrive => {
                let there = lines::has(text, lines::ARRIVED) || lines::has(text, lines::DRIVE_HERE);
                if there && quiet {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::Rerun => {
                let refused = [
                    lines::NO_ORE,
                    lines::WORKED_OUT,
                    lines::ALL_BARREN,
                    lines::CLAIMED,
                ]
                .iter()
                .any(|needle| lines::has(text, needle));
                if refused && quiet {
                    Verdict::Skip(text.to_owned())
                } else if lines::has(text, lines::CYCLE_HEAD)
                    && lines::has(text, RUN_DONE_HEAD)
                    && quiet
                {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
            Phase::LeftRead => {
                if lines::has(text, "Rig 1: ") && quiet {
                    Verdict::Done
                } else {
                    Verdict::Wait
                }
            }
        }
    }

    /// A field step decided: the next one.
    pub(super) fn field_next(
        &mut self,
        phase: Phase,
        raw: &[String],
        text: &str,
        now: u64,
    ) -> Result<(), String> {
        match phase {
            Phase::Status => {
                let own = lines::own_party(text).ok_or("no party letter in +field")?;
                self.field_state().own = Some(own);
                if let Some(evidence) = self.field_evidence() {
                    evidence.own_party = Some(own.to_string());
                }
                // Its escort sets this party aside to find the raider (`mine::lane::escort`).
                match self.field_state().escort.clone() {
                    Some(escort) => {
                        let word = lines::CAPTAIN_REPORT.replace("{party}", &own.to_string());
                        self.field_send(Phase::PageEscort, &format!("page {escort}={word}"), 0);
                    }
                    None => self.field_send(Phase::Contacts, "CONTACTS", 0),
                }
            }
            Phase::PageEscort => self.field_send(Phase::Contacts, "CONTACTS", 0),
            Phase::Contacts => {
                let (own, reported) = {
                    let state = self.field_state();
                    (state.own, state.reported.clone())
                };
                let (designator, party) =
                    raider_in(raw, own, reported.as_ref()).ok_or("no contact of another party")?;
                if let Some(evidence) = self.field_evidence() {
                    evidence.reported = reported.map(|(party, d)| format!("{party} {d}"));
                }
                self.field_state().raider = Some((party, designator.clone()));
                if let Some(evidence) = self.field_evidence() {
                    evidence.raider_party = Some(party.to_string());
                    evidence.raider = Some(designator.clone());
                }
                if self.field_state().marked {
                    self.field_engage();
                } else {
                    self.field_send(Phase::Mark, &format!("+field/mark {party}=hostile"), 0);
                }
            }
            Phase::Mark => {
                self.field_state().marked = true;
                let ack = text
                    .split_once(lines::PARTY_MARKED)
                    .and_then(|(_, rest)| rest.split_once('.'))
                    .map(|(mark, _)| format!("{}{mark}.", lines::PARTY_MARKED));
                if let Some(evidence) = self.field_evidence() {
                    evidence.marked = ack;
                }
                // One read of the outbox fixes the floor the ENGAGE's reply is read above.
                let operation = self.field_state().operation.clone();
                self.send(
                    Beat::OnPad,
                    Kind::FieldFloor,
                    &format!("+sortie/orders {operation}"),
                    0,
                );
            }
            Phase::Engage => {
                let operation = self.field_state().operation.clone();
                self.field_send(
                    Phase::EngageReply,
                    &format!("+sortie/orders {operation}"),
                    now.saturating_add(POLL_MS),
                );
            }
            Phase::EngageReply => {
                let floor = self.field_state().floor;
                let reply = reply_above(text, floor).ok_or("no reply")?;
                let top = top(text, floor);
                self.field_state().floor = top;
                if reply == "WILCO" {
                    if let Some(evidence) = self.field_evidence() {
                        evidence.engage_wilco = true;
                    }
                    self.field_send(Phase::Hold, "CONTACTS", now.saturating_add(HOLD_MS));
                } else {
                    if let Some(evidence) = self.field_evidence() {
                        evidence.negatives = evidence.negatives.saturating_add(1);
                    }
                    if self.field_state().engages >= ENGAGE_TRIES {
                        return Err(format!(
                            "the escort answered {reply} to {ENGAGE_TRIES} ENGAGE orders"
                        ));
                    }
                    self.field_send(Phase::Contacts, "CONTACTS", now.saturating_add(POLL_MS));
                }
            }
            Phase::Hold => {
                let holds = {
                    let state = self.field_state();
                    state.holds = state.holds.saturating_add(1);
                    state.holds
                };
                if let Some(evidence) = self.field_evidence() {
                    evidence.holds = holds;
                }
                if holds < HOLDS {
                    self.field_send(Phase::Hold, "CONTACTS", now.saturating_add(HOLD_MS));
                } else {
                    let line = format!("{} 1 WITHDRAW", self.field_state().fence);
                    self.field_send(Phase::Withdraw, &line, 0);
                }
            }
            Phase::Withdraw => {
                let operation = self.field_state().operation.clone();
                self.field_send(
                    Phase::WithdrawReply,
                    &format!("+sortie/orders {operation}"),
                    now.saturating_add(POLL_MS),
                );
            }
            Phase::WithdrawReply => {
                let floor = self.field_state().floor;
                let reply = reply_above(text, floor).ok_or("no reply")?;
                self.field_state().floor = top(text, floor);
                // A NEGATIVE is an escort that cannot take the order (a wreck, a lost frame): the
                // captain says so and holds on, as a player with a lost escort would.
                if let Some(evidence) = self.field_evidence() {
                    evidence.withdraw_wilco = reply == "WILCO";
                    if reply != "WILCO" {
                        evidence.withdraw_reply = Some(reply.clone());
                    }
                }
                if reply != "WILCO" {
                    self.events.push(super::LaneEvent::Said(format!(
                        "the escort answered {reply} to its WITHDRAW"
                    )));
                }
                self.field_send(Phase::Settle, "+field", now.saturating_add(SETTLE_POLL_MS));
            }
            Phase::Settle => {
                self.field_state().settled = true;
                if let Some(evidence) = self.field_evidence() {
                    evidence.settled = true;
                }
                self.events.push(super::LaneEvent::Said(
                    "the field settled: rig 1 comes home, the roster closes, and rig 1 goes back down on its own".into(),
                ));
                self.field_home(now);
            }
            Phase::Relaunch => self.field_redrive()?,
            Phase::Redrive => {
                let line = format!("+tv/1 mine {} go", self.family_word());
                self.field_send(Phase::Rerun, &line, 0);
            }
            Phase::Rerun => {
                let refused = [
                    lines::NO_ORE,
                    lines::WORKED_OUT,
                    lines::ALL_BARREN,
                    lines::CLAIMED,
                ]
                .iter()
                .any(|needle| lines::has(text, needle));
                if refused {
                    self.field_redrive()?;
                } else {
                    let run = raw
                        .iter()
                        .rev()
                        .find(|line| line.contains(lines::CYCLE_HEAD))
                        .cloned();
                    if let Some(evidence) = self.field_evidence() {
                        evidence.resumed = true;
                        evidence.refill_run = run;
                    }
                    self.send(Beat::Home, Kind::PlanHome, "+ship/plan ceres", 0);
                }
            }
            Phase::LeftRead => {
                let line = raw.iter().find(|line| line.contains("Rig 1: ")).cloned();
                if let Some(evidence) = self.field_evidence() {
                    evidence.left_home = line;
                }
                self.send(Beat::Home, Kind::Leave, "+ship/leave", 0);
            }
        }
        Ok(())
    }

    /// **The fill plan after the field** (the owner's step, doc 32 §15): rig 1 down the ramp again,
    /// from the expedition's close-out with the captain aboard.
    pub(super) fn field_relaunch(&mut self) {
        self.field_send(Phase::Relaunch, "+tv/1 launch go", 0);
    }

    /// Rig 1 to the next pocket of the captain's own reading: rig 1's first, then the rest in
    /// rank, at most [`REFILL_POCKETS`].
    fn field_redrive(&mut self) -> Result<(), String> {
        let tried = self.field_state().refill_tried;
        if tried >= REFILL_POCKETS {
            return Err(format!(
                "rig 1 found nothing to mine in {REFILL_POCKETS} pockets"
            ));
        }
        let family = self.family;
        let mut pockets: Vec<String> = self.target.iter().map(|p| p.addr.clone()).collect();
        pockets.extend(
            lines::ranked(&self.pool)
                .into_iter()
                .filter(|p| Some(p.family) == family)
                .map(|p| p.addr),
        );
        pockets.dedup();
        let addr = pockets
            .into_iter()
            .nth(usize::try_from(tried).unwrap_or(usize::MAX))
            .ok_or("no pocket of the captain's reading is left for rig 1")?;
        let state = self.field_state();
        state.refill_tried = tried.saturating_add(1);
        state.refill_addr = Some(addr.clone());
        self.field_send(Phase::Redrive, &format!("+tv/1 drive {addr} go"), 0);
        Ok(())
    }

    /// `+tv` once home: rig 1's line, the rig the departure left on the site.
    pub(super) fn field_left_read(&mut self) {
        self.field_send(Phase::LeftRead, "+tv", 0);
    }

    /// The floor's read decided (`Kind::FieldFloor`): the ENGAGE on the raider's designator.
    pub(super) fn field_floor_read(&mut self, text: &str) {
        let floor = self.field_state().floor;
        self.field_state().floor = top(text, floor);
        self.field_engage();
    }

    fn field_engage(&mut self) {
        let Some((_, designator)) = self.field_state().raider.clone() else {
            self.fail(0, "an ENGAGE with no raider".into());
            return;
        };
        let line = {
            let state = self.field_state();
            state.engages = state.engages.saturating_add(1);
            format!("{} 1 ENGAGE {designator}", state.fence)
        };
        self.field_send(Phase::Engage, &line, 0);
    }

    /// **Whether this lane's run should go home through the field's ending**: a field lane
    /// whose field came and went.
    pub(super) fn field_over(&self) -> bool {
        self.field.as_ref().is_some_and(Field::done)
    }

    /// **Home after the field**: rig 1's own recover, which drives it home from where it works
    /// and closes the captain's flight with it (the roster's recover takes only a unit already
    /// at the ramp); after its unload, the expedition's read of both slots returned and its
    /// walk back, close and lift. A fill plan that ended before any field came and went is a
    /// failure: the lane is the field's.
    pub(super) fn field_home(&mut self, now: u64) {
        if !self.field_over() {
            self.fail(
                now,
                "the fill plan ended before a field came and went on the site".into(),
            );
            return;
        }
        self.send(Beat::Home, Kind::Recover, "+tv/1 recover", 0);
    }

    /// **The `leave=` a refused lift names**: the hint's list when the refusal carries one,
    /// else the slot of `Rig {n} is out` or the `#unit` of `Unit #{u} is out`, added to what
    /// an earlier refusal named (the gate names one unit a time until `leave=` is given). At
    /// most [`LEAVE_TRIES`] lifts.
    pub(super) fn field_leave(&mut self, now: u64, text: &str) {
        let named = text
            .split_once(lines::LEAVE_HINT)
            .and_then(|(_, rest)| rest.split_once(':'))
            .map(|(list, _)| list.to_owned())
            .or_else(|| {
                let (_, rest) = text.split_once("Rig ")?;
                let (n, _) = rest.split_once(" is out on ")?;
                n.parse::<u8>().ok().map(|n| n.to_string())
            })
            .or_else(|| {
                let (_, rest) = text.split_once("Unit #")?;
                let (n, _) = rest.split_once(" is out on ")?;
                n.parse::<u64>().ok().map(|n| format!("#{n}"))
            });
        let Some(named) = named else {
            self.fail(now, format!("the lift was refused: {text}"));
            return;
        };
        let earlier = self
            .report
            .field
            .as_ref()
            .and_then(|e| e.left_behind.clone())
            .unwrap_or_default();
        let mut units: Vec<String> = earlier
            .split(',')
            .chain(named.split(','))
            .map(str::trim)
            .filter(|unit| !unit.is_empty())
            .map(str::to_owned)
            .collect();
        units.dedup();
        let list = units.join(",");
        if units.len() > LEAVE_TRIES || list == earlier {
            self.fail(
                now,
                format!("the lift was refused with leave={earlier}: {text}"),
            );
            return;
        }
        if let Some(evidence) = self.field_evidence() {
            evidence.left_behind = Some(list.clone());
            if evidence.lift_refused.is_none() {
                evidence.lift_refused = Some(text.to_owned());
            }
        }
        let line = format!("+ship/lift ceres leave={list}");
        if !in_vocabulary(&line) {
            self.fail(now, format!("leave= outside its vocabulary: {list}"));
            return;
        }
        self.send(Beat::Home, Kind::LiftHome, &line, 0);
    }
}

/// **The field's own lines**: the expedition's vocabulary ([`super::detachment::in_vocabulary`])
/// and the captain's field verbs, each in closed form.
pub fn in_vocabulary(line: &str) -> bool {
    if line.len() > 4096 || line.contains(['\n', '\r']) {
        return false;
    }
    if super::detachment::in_vocabulary(line) {
        return true;
    }
    if ["+field", "CONTACTS", "+tv/1 board", "+tv/1 leave"].contains(&line) {
        return true;
    }
    // The one page: the captain's own party, to its escort by name.
    if let Some((name, word)) = line.strip_prefix("page ").and_then(|r| r.split_once('=')) {
        return (1..=32).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_alphanumeric())
            && lines::captain_report(&format!("x{}{word}", lines::PAGED)).is_some();
    }
    if let Some(rest) = line.strip_prefix("+field/mark ") {
        return rest
            .strip_suffix("=hostile")
            .is_some_and(|letter| letter.len() == 1 && lines::PARTY_LETTERS.contains(letter));
    }
    if let Some(rest) = line.strip_prefix("+ship/lift ceres leave=") {
        return !rest.is_empty()
            && rest.split(',').all(|word| {
                let digits = word.strip_prefix('#').unwrap_or(word);
                !digits.is_empty()
                    && digits.len() <= 10
                    && digits.bytes().all(|b| b.is_ascii_digit())
            });
    }
    if let Some(rest) = line.strip_prefix("ORDER DT:") {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let Some((fence, order)) = rest.split_once(' ') else {
            return false;
        };
        if fence.split(':').count() != 3 || !fence.split(':').all(digits) {
            return false;
        }
        if order == "1 WITHDRAW" {
            return true;
        }
        return order
            .strip_prefix("1 ENGAGE ")
            .is_some_and(lines::is_field_designator);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_captains_field_verbs_are_closed() {
        for good in [
            "+field",
            "CONTACTS",
            "+tv/1 board",
            "+tv/1 leave",
            "+field/mark B=hostile",
            "ORDER DT:8589934593:1:1 1 ENGAGE BD",
            "ORDER DT:8589934593:1:1 1 ENGAGE X2",
            "ORDER DT:8589934593:1:1 1 WITHDRAW",
            "+ship/lift ceres leave=1",
            "+ship/lift ceres leave=1,#345",
            "+sortie/activate 3=combat",
            "+tv/1 mine metals go",
            "+tv/1 launch go",
            "+tv/1 drive F224 go",
            "+tv/1 mine volatiles go",
        ] {
            assert!(in_vocabulary(good), "{good}");
        }
        for bad in [
            "+field/mark B=friendly",
            "+field/mark I=hostile",
            "+field/mark E near=hostile",
            "+field/fire E",
            "ORDER DT:1:1:1 ALL WITHDRAW",
            "ORDER DT:1:1:1 1 ENGAGE bd",
            "ORDER DT:1:1 1 WITHDRAW",
            "ORDER DT:1:1:1 1 ENGAGE BD\n@destroy #1",
            "+ship/lift ceres leave=",
            "+ship/lift ceres leave=1;@boot",
            "@field/raiders tacsim.raider.ahuna_skulks",
            "FIRE cannon",
            "+tv/1 drive here go",
            "+tv/1 mine gold go",
            "+tv/3 launch go",
        ] {
            assert!(!in_vocabulary(bad), "{bad}");
        }
    }

    #[test]
    fn an_order_reply_is_read_above_its_floor() {
        let window = "Sent slot 1 order 1: WILCO received. Execution unconfirmed. \
                      Sent slot 1 order 2: submitted; no reply confirmed. Execution unconfirmed.";
        assert_eq!(top(window, 0), 2);
        assert_eq!(reply_above(window, 1), None);
        assert_eq!(reply_above(window, 0).as_deref(), Some("WILCO"));
        let answered = "Sent slot 1 order 2: NEGATIVE received. Execution unconfirmed.";
        assert_eq!(reply_above(answered, 1).as_deref(), Some("NEGATIVE"));
    }
}
