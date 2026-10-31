//! Surface excursion refusal codes, user-facing refusal rendering, and sample fixtures.

use super::*;

// ---- the refusals ---------------------------------------------------------------------------

/// **Why an act refuses** (the brief's 3.3 and 3.6): one variant per public line, each
/// carrying the words its line names; [`Refusal::line`] renders it, [`Refusal::word`] is its
/// code on an offer's wire and [`Refusal::code`] the start's `port_error` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `TV_DARK`.
    Dark {
        /// The hull's name.
        name: String,
    },
    /// `TV_UNWIRED`.
    Unwired,
    /// `TV_NOT_ABOARD`.
    NotAboard {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_NOT_SITE`.
    NotSite {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_UNDER_WAY`.
    UnderWay {
        /// The hull's name.
        name: String,
    },
    /// `LAUNCH_CLEARED`.
    Cleared {
        /// The hull's name.
        name: String,
        /// The destination's name.
        port: String,
    },
    /// `LAUNCH_OUT`.
    Out {
        /// The slot.
        n: u8,
    },
    /// `LAUNCH_OPEN`.
    Open {
        /// The slot.
        n: u8,
    },
    /// `LAUNCH_FORFEIT`.
    Forfeit {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `LAUNCH_ENDED`.
    Ended {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `RECOVER_IN_BAY`.
    InBay {
        /// The slot.
        n: u8,
    },
    /// `RECOVER_ELSEWHERE`.
    Elsewhere {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `RECOVER_FAR`.
    Far {
        /// The slot.
        n: u8,
        /// Metres from the ramp (cells × 10).
        m: u64,
        /// The egress's label.
        ramp: String,
    },
    /// `DUMP_NOT_SITE`.
    DumpNotSite {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `DUMP_NO_SPOIL`.
    NoSpoil {
        /// The site's name.
        site: String,
    },
    /// `DUMP_EMPTY`.
    Empty {
        /// The slot.
        n: u8,
    },
    /// `RIG_BUSY`.
    Busy {
        /// The slot.
        n: u8,
        /// What it is doing.
        doing: String,
        /// When it is done.
        when: String,
    },
    /// `RESERVE_LOW`.
    Reserve {
        /// The slot.
        n: u8,
        /// The charge now, whole percent.
        now: u32,
        /// What the act would leave, whole percent.
        after: u32,
        /// The floor, whole percent.
        floor: u32,
    },
    /// `CAP_OPERATOR`, `CAP_SITE` or `CAP_WORLD`.
    Cap {
        /// Which cap.
        cap: CapRefused,
        /// The site's name.
        site: String,
        /// The cap's figure (the site's is the configured field cap, doc 32 cut CF.2).
        figure: u32,
    },
    /// `LEARN_NOTHING` (doc 20 cut SM.4a; §5, D4).
    NothingNew,
    /// `CHART_FULL` (doc 20 cut SM.4a; §5's caps, D1, D3).
    ChartFull {
        /// The body's display name.
        body: String,
    },
    /// `GROUND_COLD` (doc 20 cut SM.4a; lead ruling 6): the site's ground is not cached yet.
    GroundCold {
        /// The site's name.
        site: String,
        /// The site's port entry (the command asks for its warm).
        entry: CanonicalId,
    },
    /// `INSTRUMENTS_DARK` (doc 20 cut SM.4a; D16, D25).
    InstrumentsDark,
    /// `NO_FOOTING` (doc 20 cut SM.4a; §5): the rig stands where the mast reads nothing.
    NoFooting {
        /// The slot.
        n: u8,
        /// The terrain's word.
        terrain: String,
    },
    /// `DRIVE_OFF_SITE` (doc 20 cut SM.4b): the target is off the site.
    OffSite {
        /// The site's name.
        site: String,
    },
    /// `DRIVE_HERE` (doc 20 cut SM.4b): the rig stands on the target's cell.
    Here {
        /// The slot.
        n: u8,
    },
    /// `NO_PATH` (doc 20 cut SM.4b; §8): no way over the ground within the site's budget.
    NoPath {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
    },
    /// `TOO_WINDING` (doc 20 cut SM.4b; R1): more than 32 waypoints even at 80 m.
    Winding {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
        /// The waypoints the widest freeze kept.
        needed: usize,
    },
    /// `NO_WAY_HOME` (doc 20 cut SM.4b; R2): the end stand has no way back to the ramp.
    NoWayHome {
        /// The slot.
        n: u8,
        /// The target in words.
        target: String,
        /// The egress's label.
        ramp: String,
    },
    /// `DRIVE_UNPLANNED` (doc 20 cut SM.4b; the brief's decision P2): a start of a drive with no
    /// order of the actor's for the rig (a forged start, or an order lost to a restart).
    Unplanned,
    /// `DRIVE_MOVED` (doc 20 cut SM.4b): the rig no longer stands where its drive's leg begins.
    Moved,
    /// `RECOVER_SHORT` (doc 20 cut SM.4b; lead ruling 5, decision P6): the rig's charge cannot
    /// cover its own leg home.
    LegShort {
        /// The slot.
        n: u8,
        /// The charge now, whole percent (`?` when the battery cannot be read).
        now: String,
        /// The leg's charge, whole percent rounded up (`?` likewise).
        need: String,
        /// The egress's label.
        ramp: String,
    },
    /// `RECOVER_DARK` (doc 20 cut SM.4b; R3): the way home's terrain cannot be mapped this boot.
    RecoverDark {
        /// The slot.
        n: u8,
        /// The egress's label.
        ramp: String,
    },
    /// `MINE_IN_BAY` (doc 20 cut SM.7; the brief's decision R11): the drill works only a pocket it
    /// stands on — no implied launch.
    MineInBay {
        /// The slot.
        n: u8,
    },
    /// `MINE_RUNNING` (doc 20 cut SM.7; decision R1): a run holds the rig; only a scan or a
    /// sounding may start beside it.
    MineRunning {
        /// The slot.
        n: u8,
        /// The run's pocket.
        addr: String,
        /// The cycle in flight.
        k: u8,
        /// The run's cycles.
        cycles: u8,
        /// When the cycle in flight is due, in words.
        when: String,
    },
    /// `MINE_NO_FOOTING` (doc 20 cut SM.7): the rig stands where the drill bites nothing.
    MineNoFooting {
        /// The slot.
        n: u8,
        /// The terrain's word.
        terrain: String,
    },
    /// **The licence** (doc 20 cut SM.7; lead ruling 5, reviewer F21): `MINE_NO_LICENCE` for no
    /// licence at all, else `+licence`'s own line for the standing
    /// (`crate::port::licence::standing_line`), formatted when the plan is made.
    Licence {
        /// The line.
        line: String,
    },
    /// `MINE_CLAIMED` (doc 20 §5, verbatim; lead ruling 5).
    Claimed,
    /// `MINE_LOCKED` (doc 20 §6, verbatim; lead ruling 3, decision R4).
    Locked,
    /// `MINE_HOPPER_FULL`: no cycle fits the hopper.
    HopperFull {
        /// The slot.
        n: u8,
        /// The hopper's words.
        hopper: String,
    },
    /// `MINE_HOLD_FULL`: no cycle fits the hold's unreserved room.
    HoldFull {
        /// The hull's name.
        name: String,
    },
    /// `MINE_WORKED_OUT`: the pocket's tonnes of the family are drawn.
    WorkedOut {
        /// The pocket's address.
        addr: String,
        /// The ore's words.
        ore: String,
    },
    /// `MINE_NO_FIT` (§6's line, with the slot): a named count that does not fit.
    NoFit {
        /// The slot.
        n: u8,
        /// The count named.
        m: u8,
        /// The count that fits.
        fit: u8,
        /// What bound it.
        bound: crate::surface::mine::Bound,
        /// The family word as typed, with a leading space, or empty.
        fam: String,
    },
    /// `MINE_BLIND_ROOM` (lead ruling 2): the only room line for an unread pocket.
    BlindRoom {
        /// The blind room, whole tonnes.
        room: u32,
    },
    /// `MINE_BLIND_COUNT`: a count named on an unread pocket.
    BlindCount {
        /// The slot.
        n: u8,
        /// The family word as typed, with a leading space, or empty.
        fam: String,
    },
    /// `MINE_NO_ORE`: the operator's reading shows none of the family named.
    NoOre {
        /// The pocket's address.
        addr: String,
        /// The ore's words.
        ore: String,
    },
    /// `MINE_ALL_BARREN`: the operator's reading shows the pocket barren.
    AllBarren {
        /// The pocket's address.
        addr: String,
    },
    /// `MINE_FORFEIT` (doc 20 cut SM.7; the brief's decision R15): the rig's deposit is forfeit.
    MineForfeit {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `MINE_ENDED` (decision R15): the rig's economy lease ended.
    MineEnded {
        /// The slot.
        n: u8,
        /// Occator's counter.
        counter: String,
        /// Its port.
        port: String,
    },
    /// `MINE_CHANGED` (the brief's 3.4): the start's re-plan differs from the order's — `go`
    /// commits exactly the preview or nothing.
    MineChanged,
    /// `UNLOAD_UNDER_WAY` (doc 20 cut SM.8).
    UnloadUnderWay {
        /// The hull's name.
        name: String,
    },
    /// `UNLOAD_NOT_SITE` (doc 20 cut SM.8; decision R5): an unload works only on a site's pad.
    UnloadNotSite {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
    },
    /// `UNLOAD_OUT` (doc 20 cut SM.8; decision R9): the rig stands on the site's ground.
    UnloadOut {
        /// The slot.
        n: u8,
        /// The site's name.
        site: String,
    },
    /// `UNLOAD_NO_ROOM` (doc 20 cut SM.8; decision R18): the hold takes nothing of it.
    UnloadNoRoom {
        /// The slot.
        n: u8,
        /// The hull's name.
        name: String,
        /// The hopper's load in words.
        load: String,
    },
    /// `UNLOAD_NO_FIT` (doc 20 cut SM.8; decisions R10, R18): a named tonnage past the fit, which
    /// is at least 1.
    UnloadNoFit {
        /// The slot.
        n: u8,
        /// The tonnes named.
        q: u64,
        /// The tonnes that fit.
        fit: u64,
        /// The good's label.
        ore: String,
    },
    /// `UNLOAD_NO_STOCK` (doc 20 cut SM.8; the lead's pre-commit round, item 4): a named tonnage
    /// past the hopper's own stock of the first good, which is at least 1 — the hold's room was
    /// not what bound it.
    UnloadNoStock {
        /// The slot.
        n: u8,
        /// The tonnes of the good the hopper holds.
        fit: u64,
        /// The good's label.
        ore: String,
    },
    /// `UNLOAD_CHANGED` (doc 20 cut SM.8; §7.1 "re-checked at prepare"): the hopper or the hold's
    /// room changed between the preview and the start.
    UnloadChanged {
        /// The slot.
        n: u8,
    },
    /// `UNLOAD_MARKERS` (doc 20 cut SM.11, the lead's pre-commit round, item 1): the queued
    /// unload's start, while the hopper's marker ring could not record the move — the queue
    /// stands and the surface tick starts it again under its attempt rule.
    UnloadMarkers {
        /// The slot.
        n: u8,
    },
    /// `PLAN_LIVE` (doc 20 cut SM.11; the brief's D8): a live plan holds its rig from every hand
    /// start.
    PlanLive {
        /// The slot.
        n: u8,
        /// The plan's number.
        no: u32,
    },
    /// `PLAN_ALREADY` (doc 20 cut SM.11): filing over a live plan.
    PlanFiled {
        /// The slot.
        n: u8,
        /// The live plan's number.
        no: u32,
    },
    /// `PLAN_CHANGED` (doc 20 cut SM.11): the start's re-compile differs from the preview.
    PlanChanged {
        /// The slot.
        n: u8,
    },
    /// `PLAN_WAITS` (doc 20 cut SM.11): the plan's next step cannot start now.
    PlanWaits {
        /// The slot.
        n: u8,
    },
    /// `PLAN_CHANGED`'s line (doc 20 cut SM.11): the plan moved on between the decision and the
    /// landing.
    PlanMoved {
        /// The slot.
        n: u8,
    },
    /// `PLAN_NONE` (doc 20 cut SM.11).
    PlanNone {
        /// The slot.
        n: u8,
    },
    /// `PLAN_ALREADY_ENDED` (doc 20 cut SM.11): a clear landing on a plan that ended by itself.
    PlanEnded {
        /// The slot.
        n: u8,
        /// The plan's number.
        no: u32,
    },
}

/// The offer-wire words and the start's messages, in one table (the prepare's
/// `port_error` needs a `&'static str`).
pub(super) const REFUSAL_CODES: [(&str, &str); 64] = [
    ("tv_dark", "that rig cannot be read"),
    ("tv_unwired", "rigs cannot be worked on this server"),
    ("not_aboard", "the operator is not aboard the hull"),
    ("not_site", "the hull is not settled on a site's pad"),
    ("under_way", "the hull is under way"),
    ("cleared", "the hull is cleared to leave"),
    ("out", "that rig is already out of the bay"),
    ("open", "that rig has an excursion open"),
    ("forfeit", "that rig's deposit is forfeit"),
    ("ended", "that rig's lease has ended"),
    ("in_bay", "that rig is in the bay"),
    ("elsewhere", "that rig is not beside the hull"),
    ("far", "that rig is too far from the ramp"),
    ("dump_not_site", "the hull is not settled on a site's pad"),
    ("no_spoil", "the site has no spoil laid out"),
    ("empty", "the hopper is empty"),
    ("busy", "that rig is busy"),
    ("reserve", "the rig's charge is under the reserve"),
    ("cap", "that is as many rigs out as may be"),
    ("nothing_new", "you learn nothing new here"),
    ("chart_full", "your chart of that body is full"),
    ("ground_cold", "the site's ground is still being mapped"),
    ("instruments_dark", "the instruments cannot read the ground"),
    ("no_footing", "the rig stands where the mast reads nothing"),
    ("off_site", "that point is off the site"),
    ("here", "the rig is there already"),
    ("no_path", "there is no way there over the ground"),
    ("winding", "that way winds too much to drive in one leg"),
    (
        "no_way_home",
        "the rig could not get back to the ramp from there",
    ),
    // Doc 20 cut SM.7 (reviewer F7): a mine order lost to a restart, or replaced by the rig's
    // newer drive order, must not be told about a drive.
    ("unplanned", "that act was not planned; preview it again"),
    ("moved", "that rig has moved since the drive was planned"),
    ("leg_short", "the rig's charge cannot cover the drive"),
    (
        "recover_dark",
        "the rig's way back cannot be mapped just now",
    ),
    // Doc 20 cut SM.7: mining's sixteen.
    ("mine_in_bay", "the rig is in the bay"),
    ("mine_running", "that rig is drilling"),
    (
        "mine_no_footing",
        "the rig stands where the drill bites nothing",
    ),
    ("unlicensed", "the operator holds no licence in force"),
    ("claimed", "that block is under claim"),
    ("locked", "another rig is working that pocket"),
    ("hopper_full", "the hopper has no room for another cycle"),
    (
        "hold_full",
        "the hold has no unreserved room for another cycle",
    ),
    ("worked_out", "that pocket is worked out"),
    ("no_fit", "those cycles do not fit"),
    (
        "blind_room",
        "an unread pocket needs room for the richest cycle",
    ),
    ("blind_count", "an unread pocket is cored one cycle"),
    ("no_ore", "the reading shows no ore to mine"),
    ("mine_forfeit", "the rig's deposit is forfeit"),
    ("mine_ended", "the rig's lease has ended"),
    (
        "mine_changed",
        "that run changed since its preview; preview it again",
    ),
    // Doc 20 cut SM.8: unloading's seven (the lead's pre-commit round added `unload_no_stock`).
    ("unload_under_way", "the hull is under way"),
    ("unload_not_site", "the hull is not on a site's pad"),
    ("unload_out", "the rig is not in the bay"),
    ("unload_no_room", "the hold has no room"),
    ("unload_no_fit", "that much does not fit the hold"),
    ("unload_no_stock", "the hopper holds less than that"),
    (
        "unload_changed",
        "the hopper or the hold's room changed; preview it again",
    ),
    // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): the queued unload's start while the
    // hopper's unload log is full.
    (
        "unload_markers",
        "the rig's unload log is full; try again later",
    ),
    // Doc 20 cut SM.11: the fill plan's seven (prepare refusals, never a world plan's).
    ("plan_live", "that rig is working a plan"),
    ("plan_filed", "that rig is already working a plan"),
    (
        "plan_changed",
        "that plan changed since its preview; preview it again",
    ),
    ("plan_waits", "that rig's plan is waiting"),
    ("plan_moved", "the plan moved on"),
    ("plan_none", "that rig has no plan"),
    ("plan_ended", "that plan has already ended"),
];

/// **Every refusal word** (doc 20 cut SM.11; test support for the plan's refusal table, which
/// iterates the start's own list).
#[cfg(test)]
pub(crate) fn refusal_words() -> Vec<&'static str> {
    REFUSAL_CODES.iter().map(|(word, _)| *word).collect()
}

/// The start's message for an offer-wire word; an unknown word reads as dark.
#[must_use]
pub(crate) fn code_of_word(word: &str) -> &'static str {
    REFUSAL_CODES
        .iter()
        .find(|(w, _)| *w == word)
        .map_or("that rig cannot be read", |(_, message)| message)
}

impl Refusal {
    /// The offer-wire word.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Refusal::Dark { .. } => "tv_dark",
            Refusal::Unwired => "tv_unwired",
            Refusal::NotAboard { .. } => "not_aboard",
            Refusal::NotSite { .. } => "not_site",
            Refusal::UnderWay { .. } => "under_way",
            Refusal::Cleared { .. } => "cleared",
            Refusal::Out { .. } => "out",
            Refusal::Open { .. } => "open",
            Refusal::Forfeit { .. } => "forfeit",
            Refusal::Ended { .. } => "ended",
            Refusal::InBay { .. } => "in_bay",
            Refusal::Elsewhere { .. } => "elsewhere",
            Refusal::Far { .. } => "far",
            Refusal::DumpNotSite { .. } => "dump_not_site",
            Refusal::NoSpoil { .. } => "no_spoil",
            Refusal::Empty { .. } => "empty",
            Refusal::Busy { .. } => "busy",
            Refusal::Reserve { .. } => "reserve",
            Refusal::Cap { .. } => "cap",
            Refusal::NothingNew => "nothing_new",
            Refusal::ChartFull { .. } => "chart_full",
            Refusal::GroundCold { .. } => "ground_cold",
            Refusal::InstrumentsDark => "instruments_dark",
            Refusal::NoFooting { .. } => "no_footing",
            Refusal::OffSite { .. } => "off_site",
            Refusal::Here { .. } => "here",
            Refusal::NoPath { .. } => "no_path",
            Refusal::Winding { .. } => "winding",
            Refusal::NoWayHome { .. } => "no_way_home",
            Refusal::Unplanned => "unplanned",
            Refusal::Moved => "moved",
            Refusal::LegShort { .. } => "leg_short",
            Refusal::RecoverDark { .. } => "recover_dark",
            Refusal::MineInBay { .. } => "mine_in_bay",
            Refusal::MineRunning { .. } => "mine_running",
            Refusal::MineNoFooting { .. } => "mine_no_footing",
            Refusal::Licence { .. } => "unlicensed",
            Refusal::Claimed => "claimed",
            Refusal::Locked => "locked",
            Refusal::HopperFull { .. } => "hopper_full",
            Refusal::HoldFull { .. } => "hold_full",
            Refusal::WorkedOut { .. } => "worked_out",
            Refusal::NoFit { .. } => "no_fit",
            Refusal::BlindRoom { .. } => "blind_room",
            Refusal::BlindCount { .. } => "blind_count",
            Refusal::NoOre { .. } | Refusal::AllBarren { .. } => "no_ore",
            Refusal::MineForfeit { .. } => "mine_forfeit",
            Refusal::MineEnded { .. } => "mine_ended",
            Refusal::MineChanged => "mine_changed",
            Refusal::UnloadUnderWay { .. } => "unload_under_way",
            Refusal::UnloadNotSite { .. } => "unload_not_site",
            Refusal::UnloadOut { .. } => "unload_out",
            Refusal::UnloadNoRoom { .. } => "unload_no_room",
            Refusal::UnloadNoFit { .. } => "unload_no_fit",
            Refusal::UnloadNoStock { .. } => "unload_no_stock",
            Refusal::UnloadChanged { .. } => "unload_changed",
            Refusal::UnloadMarkers { .. } => "unload_markers",
            Refusal::PlanLive { .. } => "plan_live",
            Refusal::PlanFiled { .. } => "plan_filed",
            Refusal::PlanChanged { .. } => "plan_changed",
            Refusal::PlanWaits { .. } => "plan_waits",
            Refusal::PlanMoved { .. } => "plan_moved",
            Refusal::PlanNone { .. } => "plan_none",
            Refusal::PlanEnded { .. } => "plan_ended",
        }
    }

    /// The start's `port_error` message: a short lowercase sentence.
    #[must_use]
    pub fn code(&self) -> &'static str {
        code_of_word(self.word())
    }

    /// The line the player hears.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Refusal::Dark { name } => tv::TV_DARK.replace("{name}", name),
            Refusal::Unwired => tv::TV_UNWIRED.to_owned(),
            Refusal::NotAboard { name } => tv::TV_NOT_ABOARD.replace("{name}", name),
            Refusal::NotSite { name } => tv::LAUNCH_NOT_SITE.replace("{name}", name),
            Refusal::UnderWay { name } => tv::LAUNCH_UNDER_WAY.replace("{name}", name),
            Refusal::Cleared { name, port } => tv::LAUNCH_CLEARED
                .replace("{name}", name)
                .replace("{port}", port),
            Refusal::Out { n } => tv::LAUNCH_OUT.replace("{n}", &n.to_string()),
            Refusal::Open { n } => tv::LAUNCH_OPEN.replace("{n}", &n.to_string()),
            Refusal::Forfeit { n, counter, port } => tv::LAUNCH_FORFEIT
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::Ended { n, counter, port } => tv::LAUNCH_ENDED
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::InBay { n } => tv::RECOVER_IN_BAY.replace("{n}", &n.to_string()),
            Refusal::Elsewhere { n, name } => tv::RECOVER_ELSEWHERE
                .replace("{n}", &n.to_string())
                .replace("{name}", name),
            Refusal::Far { n, m, ramp } => tv::RECOVER_FAR
                .replace("{n}", &n.to_string())
                .replace("{m}", &m.to_string())
                .replace("{ramp}", ramp),
            Refusal::DumpNotSite { n, name } => tv::DUMP_NOT_SITE
                .replace("{n}", &n.to_string())
                .replace("{name}", name),
            Refusal::NoSpoil { site } => tv::DUMP_NO_SPOIL.replace("{site}", site),
            Refusal::Empty { n } => tv::DUMP_EMPTY.replace("{n}", &n.to_string()),
            Refusal::Busy { n, doing, when } => tv::RIG_BUSY
                .replace("{n}", &n.to_string())
                .replace("{doing}", doing)
                .replace("{when}", when),
            Refusal::Reserve {
                n,
                now,
                after,
                floor,
            } => tv::RESERVE_LOW
                .replace("{n}", &n.to_string())
                .replace("{now}", &now.to_string())
                .replace("{after}", &after.to_string())
                .replace("{floor}", &floor.to_string()),
            Refusal::Cap { cap, site, figure } => {
                let template = match cap {
                    CapRefused::Operator => tv::CAP_OPERATOR,
                    CapRefused::Site => tv::CAP_SITE,
                    CapRefused::World => tv::CAP_WORLD,
                };
                template
                    .replace("{cap}", &figure.to_string())
                    .replace("{site}", site)
            }
            Refusal::NothingNew => tv::LEARN_NOTHING.to_owned(),
            Refusal::ChartFull { body } => tv::CHART_FULL.replace("{body}", body),
            Refusal::GroundCold { site, .. } => tv::GROUND_COLD.replace("{site}", site),
            Refusal::InstrumentsDark => tv::INSTRUMENTS_DARK.to_owned(),
            Refusal::NoFooting { n, terrain } => tv::NO_FOOTING
                .replace("{n}", &n.to_string())
                .replace("{terrain}", terrain),
            Refusal::OffSite { site } => tv::DRIVE_OFF_SITE.replace("{site}", site),
            Refusal::Here { n } => tv::DRIVE_HERE.replace("{n}", &n.to_string()),
            Refusal::NoPath { n, target } => tv::NO_PATH
                .replace("{n}", &n.to_string())
                .replace("{target}", target),
            Refusal::Winding { target, .. } => tv::TOO_WINDING
                .replace("{target}", target)
                .replace("{cap}", &crate::surface::MAX_LEG_WAYPOINTS.to_string()),
            Refusal::NoWayHome { n, target, ramp } => tv::NO_WAY_HOME
                .replace("{n}", &n.to_string())
                .replace("{target}", target)
                .replace("{ramp}", ramp),
            Refusal::Unplanned => tv::DRIVE_UNPLANNED.to_owned(),
            Refusal::Moved => tv::DRIVE_MOVED.to_owned(),
            Refusal::LegShort { n, now, need, ramp } => tv::RECOVER_SHORT
                .replace("{n}", &n.to_string())
                .replace("{now}", now)
                .replace("{need}", need)
                .replace("{ramp}", ramp),
            Refusal::RecoverDark { n, ramp } => tv::RECOVER_DARK
                .replace("{n}", &n.to_string())
                .replace("{ramp}", ramp),
            Refusal::MineInBay { n } => tv::MINE_IN_BAY.replace("{n}", &n.to_string()),
            Refusal::MineRunning {
                n,
                addr,
                k,
                cycles,
                when,
            } => tv::MINE_RUNNING
                .replace("{n}", &n.to_string())
                .replace("{addr}", addr)
                .replace("{k}", &k.to_string())
                .replace("{cycles}", &cycles.to_string())
                .replace("{when}", when),
            Refusal::MineNoFooting { n, terrain } => tv::MINE_NO_FOOTING
                .replace("{n}", &n.to_string())
                .replace("{terrain}", terrain),
            Refusal::Licence { line } => line.clone(),
            Refusal::Claimed => tv::MINE_CLAIMED.to_owned(),
            Refusal::Locked => tv::MINE_LOCKED.to_owned(),
            Refusal::HopperFull { n, hopper } => tv::MINE_HOPPER_FULL
                .replace("{n}", &n.to_string())
                .replace("{hopper}", hopper),
            Refusal::HoldFull { name } => tv::MINE_HOLD_FULL.replace("{name}", name),
            Refusal::WorkedOut { addr, ore } => tv::MINE_WORKED_OUT
                .replace("{addr}", addr)
                .replace("{ore}", ore),
            Refusal::NoFit {
                n,
                m,
                fit,
                bound,
                fam,
            } => {
                let ord = crate::surface::mine::ordinal_word(*fit);
                let why = match bound {
                    crate::surface::mine::Bound::Hopper => tv::FIT_HOPPER.replace("{ord}", ord),
                    crate::surface::mine::Bound::Hold => tv::FIT_HOLD.replace("{ord}", ord),
                    crate::surface::mine::Bound::WorkedOut => tv::FIT_POCKET.replace("{ord}", ord),
                    crate::surface::mine::Bound::Cap => tv::FIT_CAP.to_owned(),
                };
                tv::MINE_NO_FIT
                    .replace("{m}", &m.to_string())
                    .replace("{why}", &why)
                    .replace("{n}", &n.to_string())
                    .replace("{fam}", fam)
                    .replace("{fit}", &fit.to_string())
            }
            Refusal::BlindRoom { room } => tv::MINE_BLIND_ROOM.replace("{room}", &room.to_string()),
            Refusal::BlindCount { n, fam } => tv::MINE_BLIND_COUNT
                .replace("{n}", &n.to_string())
                .replace("{fam}", fam),
            Refusal::NoOre { addr, ore } => tv::MINE_NO_ORE
                .replace("{addr}", addr)
                .replace("{ore}", ore),
            Refusal::AllBarren { addr } => tv::MINE_ALL_BARREN.replace("{addr}", addr),
            Refusal::MineForfeit { n, counter, port } => tv::MINE_FORFEIT
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::MineEnded { n, counter, port } => tv::MINE_ENDED
                .replace("{n}", &n.to_string())
                .replace("{counter}", counter)
                .replace("{port}", port),
            Refusal::MineChanged => tv::MINE_CHANGED.to_owned(),
            Refusal::UnloadUnderWay { name } => tv::UNLOAD_UNDER_WAY.replace("{name}", name),
            Refusal::UnloadNotSite { n, name } => tv::UNLOAD_NOT_SITE
                .replace("{name}", name)
                .replace("{n}", &n.to_string()),
            Refusal::UnloadOut { n, site } => tv::UNLOAD_OUT
                .replace("{n}", &n.to_string())
                .replace("{site}", site),
            Refusal::UnloadNoRoom { n, name, load } => tv::UNLOAD_NO_ROOM
                .replace("{name}", name)
                .replace("{n}", &n.to_string())
                .replace("{load}", load),
            Refusal::UnloadNoFit { n, q, fit, ore } => tv::UNLOAD_NO_FIT
                .replace("{q}", &q.to_string())
                .replace("{n}", &n.to_string())
                .replace("{fit}", &fit.to_string())
                .replace("{ore}", ore),
            Refusal::UnloadNoStock { n, fit, ore } => tv::UNLOAD_NO_STOCK
                .replace("{n}", &n.to_string())
                .replace("{fit}", &fit.to_string())
                .replace("{ore}", ore),
            Refusal::UnloadChanged { n } => tv::UNLOAD_CHANGED.replace("{n}", &n.to_string()),
            Refusal::UnloadMarkers { n } => tv::UNLOAD_MARKERS.replace("{n}", &n.to_string()),
            Refusal::PlanLive { n, no } => tv::PLAN_LIVE
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
            Refusal::PlanFiled { n, no } => tv::PLAN_ALREADY
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
            Refusal::PlanChanged { n } | Refusal::PlanMoved { n } => {
                tv::PLAN_CHANGED.replace("{n}", &n.to_string())
            }
            Refusal::PlanWaits { n } => tv::PLAN_WAITS
                .replace("{n}", &n.to_string())
                .replace("{why}", tv::WAIT_NEXT),
            Refusal::PlanNone { n } => tv::PLAN_NONE.replace("{n}", &n.to_string()),
            Refusal::PlanEnded { n, no } => tv::PLAN_ALREADY_ENDED
                .replace("{n}", &n.to_string())
                .replace("{no}", &no.to_string()),
        }
    }
}

/// **One sample of every refusal variant** (test support: `sm3b_every_refusal_word_has_a_message`
/// and the plan's refusal table iterate it).
#[cfg(test)]
pub(crate) fn refusal_samples() -> Vec<Refusal> {
    vec![
        Refusal::Dark {
            name: String::new(),
        },
        Refusal::Unwired,
        Refusal::NotAboard {
            name: String::new(),
        },
        Refusal::NotSite {
            name: String::new(),
        },
        Refusal::UnderWay {
            name: String::new(),
        },
        Refusal::Cleared {
            name: String::new(),
            port: String::new(),
        },
        Refusal::Out { n: 1 },
        Refusal::Open { n: 1 },
        Refusal::Forfeit {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::Ended {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::InBay { n: 1 },
        Refusal::Elsewhere {
            n: 1,
            name: String::new(),
        },
        Refusal::Far {
            n: 1,
            m: 20,
            ramp: String::new(),
        },
        Refusal::DumpNotSite {
            n: 1,
            name: String::new(),
        },
        Refusal::NoSpoil {
            site: String::new(),
        },
        Refusal::Empty { n: 1 },
        Refusal::Busy {
            n: 1,
            doing: String::new(),
            when: String::new(),
        },
        Refusal::Reserve {
            n: 1,
            now: 1,
            after: 0,
            floor: 5,
        },
        Refusal::Cap {
            cap: CapRefused::Operator,
            site: String::new(),
            figure: 4,
        },
        Refusal::NothingNew,
        Refusal::ChartFull {
            body: String::new(),
        },
        Refusal::GroundCold {
            site: String::new(),
            entry: CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")),
        },
        Refusal::InstrumentsDark,
        Refusal::NoFooting {
            n: 1,
            terrain: String::new(),
        },
        Refusal::OffSite {
            site: String::new(),
        },
        Refusal::Here { n: 1 },
        Refusal::NoPath {
            n: 1,
            target: String::new(),
        },
        Refusal::Winding {
            n: 1,
            target: String::new(),
            needed: 33,
        },
        Refusal::NoWayHome {
            n: 1,
            target: String::new(),
            ramp: String::new(),
        },
        Refusal::Unplanned,
        Refusal::Moved,
        Refusal::LegShort {
            n: 1,
            now: "2".to_owned(),
            need: "3".to_owned(),
            ramp: String::new(),
        },
        Refusal::RecoverDark {
            n: 1,
            ramp: String::new(),
        },
        Refusal::MineInBay { n: 1 },
        Refusal::MineRunning {
            n: 1,
            addr: String::new(),
            k: 1,
            cycles: 4,
            when: String::new(),
        },
        Refusal::MineNoFooting {
            n: 1,
            terrain: String::new(),
        },
        Refusal::Licence {
            line: tv::MINE_NO_LICENCE.to_owned(),
        },
        Refusal::Claimed,
        Refusal::Locked,
        Refusal::HopperFull {
            n: 1,
            hopper: String::new(),
        },
        Refusal::HoldFull {
            name: String::new(),
        },
        Refusal::WorkedOut {
            addr: String::new(),
            ore: String::new(),
        },
        Refusal::NoFit {
            n: 1,
            m: 4,
            fit: 3,
            bound: Bound::Hopper,
            fam: String::new(),
        },
        Refusal::BlindRoom { room: 3 },
        Refusal::BlindCount {
            n: 1,
            fam: String::new(),
        },
        Refusal::NoOre {
            addr: String::new(),
            ore: String::new(),
        },
        Refusal::AllBarren {
            addr: String::new(),
        },
        Refusal::MineForfeit {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::MineEnded {
            n: 1,
            counter: String::new(),
            port: String::new(),
        },
        Refusal::MineChanged,
        Refusal::UnloadUnderWay {
            name: String::new(),
        },
        Refusal::UnloadNotSite {
            n: 1,
            name: String::new(),
        },
        Refusal::UnloadOut {
            n: 1,
            site: String::new(),
        },
        Refusal::UnloadNoRoom {
            n: 1,
            name: String::new(),
            load: String::new(),
        },
        Refusal::UnloadNoFit {
            n: 1,
            q: 9,
            fit: 3,
            ore: String::new(),
        },
        Refusal::UnloadNoStock {
            n: 1,
            fit: 3,
            ore: String::new(),
        },
        Refusal::UnloadChanged { n: 1 },
        Refusal::UnloadMarkers { n: 1 },
        Refusal::PlanLive { n: 1, no: 2 },
        Refusal::PlanFiled { n: 1, no: 2 },
        Refusal::PlanChanged { n: 1 },
        Refusal::PlanWaits { n: 1 },
        Refusal::PlanMoved { n: 1 },
        Refusal::PlanNone { n: 1 },
        Refusal::PlanEnded { n: 1, no: 2 },
    ]
}
