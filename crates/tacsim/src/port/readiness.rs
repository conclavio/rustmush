//! **The port readiness report** (doc 18 cut IC3; doc 18 §4.7 "Readiness";
//! lead ruling L14).
//!
//! A pure read of everything the port entries bound: the lane's latched version
//! against the one this server ships, the sky the lane stands under, the world
//! rate, the treasury, and per port its map, rooms, domain, institutions (each
//! with its account and capital) and programme. `@tacsim/port status` renders
//! it; cut IC6 gates `+coop/enrol` on [`Readiness::is_ready`] and reads the
//! typed [`LineState`]s, never the text. Cut IC4's hulls line has landed
//! ([`Binding::Hulls`], computed by `crate::port::hull::hull_line`). Cut IC8's
//! lines — the market facts, and per port its pools, listings and bids — read
//! typed state, so they come from [`readiness_in`], which takes a module context
//! and splices them into this world-only report (lead ruling L9).
//!
//! **The report's lane pack** is the latched version when this server loads it,
//! else the shipped [`LANE_VERSION`] when loaded, else none — always an exact
//! `(id, version)` lookup, never `TacsimStore::resolve`, which would pick a bare
//! id across every loaded pack.

use std::collections::BTreeSet;

use mush_core::output::display_name;
use mush_core::{CanonicalId, World};
use mush_economy::{AccountId, AccountStatus, Amount, EconomyStore, InstrumentId, ObligationId};
use mush_org::{OrgId, OrgStore, PrincipalId, SystemPrincipalId};

use crate::definitions::content::{LANE_PACK, LANE_VERSION};
use crate::economy::CREDIT;
use crate::pack::{PackEntry, RulesPack};
use crate::port::apply::{room_by_key, room_references};
use crate::port::def::{PortDef, capital_schedule, room_key};
use crate::port::map::{PortMap, PortMapState, port_map_key};
use crate::sky::{Sky, epoch_date};
use crate::store::TacsimStore;

/// The world rate play runs at (doc 18 §4.4, R3): 24 IC seconds per real second.
pub const PLAY_RATE: u32 = 24;

/// The treasury a port names when no port entry is loaded to say otherwise.
const DEFAULT_TREASURY: &str = "tacsim.treasury";

/// **What one line reports on.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// The lane pack's latched version.
    Lane,
    /// The sky the lane stands under.
    Sky,
    /// The world rate.
    Rate,
    /// The treasury account.
    Treasury,
    /// One port's map (absent or unreadable).
    Map,
    /// One port's rooms.
    Rooms,
    /// One port's passive domain.
    Domain,
    /// One institution: its org, its account and its capital (L14).
    Org,
    /// One port's programme.
    Programme,
    /// One port's programme hulls (cut IC4): made, interior complete, titled
    /// to the programme org and laid up at a port.
    Hulls,
    /// Cut IC8: one vendor's storefront and how many of its stock rows are
    /// seeded (minted and dissolved into the pool).
    Pools,
    /// Cut IC8: one ask's listing against its storefront pool.
    Listings,
    /// Cut IC8: one standing bid (reported, not gated: lead ruling L8; gated on
    /// Missing by cut IC12; since cut P-B on Missing alone — a house that cannot
    /// fund a tonne, or a bid the clerk re-posts, reads Pending, is paged once a
    /// boot by the port tick and never closes the desk: Q-18-60, R10).
    Bids,
    /// Cut IC8: whether anything answers the `market` fact namespace, which a
    /// drawn sale's contract settles on.
    MarketFacts,
    /// Cut IC10: whether anything answers the `inventory` fact namespace, which a
    /// sale into a bid's contract settles on (lead 2g). Since cut IC14 (row 1) also
    /// Missing when the applied-marker mirror is incomplete, since an incomplete
    /// mirror answers nothing (E7c2 fold R3).
    InventoryFacts,
    /// Doc 18 §9 risk 19, cut IC14: the inventory mirror's headroom; reported and
    /// paged, never gated, as the sky is (proposed Q-18-72).
    Markers,
    /// Doc 20 cut SM.1: the geology key's state; reported, never gated, in
    /// D-18-50's sky manner (`crate::surface::key::geology_line`).
    Geology,
    /// Doc 20 cut SM.2: a site's block places and clerk; reported, never gated
    /// (`crate::port::site::site_line`).
    Site,
    /// Doc 20 cut SM.3a: a lessor's rig pool; reported, never gated
    /// (`crate::port::vehicle::vehicles_line`, which also names a programme hull
    /// whose refit bay is not laid up).
    Vehicles,
    /// Doc 20 cut SM.5: the prospecting licence the port's `licence` block issues;
    /// reported, never gated (`crate::port::licence::licence_line`).
    Licence,
}

/// **A line's state.** Only [`LineState::Ready`] counts as ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineState {
    /// In place.
    Ready,
    /// Made and waiting on the economy's next sweep.
    Pending,
    /// Absent, unreadable or wrong.
    Missing,
    /// In place but not as play needs it (an old lane, a test rate on a real
    /// world, an unreadable sky revision).
    Flagged,
}

impl LineState {
    /// The word the report prints in brackets.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            LineState::Ready => "ready",
            LineState::Pending => "pending",
            LineState::Missing => "missing",
            LineState::Flagged => "flagged",
        }
    }
}

/// **One line of the report.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessLine {
    /// The port entry the line is about, for a per-port line.
    pub port: Option<CanonicalId>,
    /// What it reports on.
    pub binding: Binding,
    /// Its state.
    pub state: LineState,
    /// The sentence.
    pub text: String,
}

/// **The whole report**, in render order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readiness {
    /// The lines.
    pub lines: Vec<ReadinessLine>,
    /// **Doc 20 cut SM.2 (lead override 3)** — the port entries that are mining sites
    /// (their `def` carries a `site` block). Their lines are reported like every port's
    /// and **never gate `+coop/enrol`** ([`crate::port::coop::enrol_gate`]): mining
    /// content never closes the trade run's front door; a departure **to** a site whose
    /// lines are not all ready is refused at planning instead (`port_ready`).
    pub sites: BTreeSet<CanonicalId>,
}

impl Readiness {
    /// Whether every line is [`LineState::Ready`].
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.lines.iter().all(|line| line.state == LineState::Ready)
    }

    /// The lines that are not ready.
    pub fn not_ready(&self) -> impl Iterator<Item = &ReadinessLine> {
        self.lines
            .iter()
            .filter(|line| line.state != LineState::Ready)
    }

    /// The summary line the report ends with.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_ready() {
            "Ready: every binding above is in place.".to_owned()
        } else {
            format!(
                "Not ready: {} of {} lines are pending, missing or flagged.",
                self.not_ready().count(),
                self.lines.len()
            )
        }
    }

    /// The report as `@tacsim/port status` prints it: a header, each line with
    /// its state word (per-port lines indented two spaces), the summary last.
    #[must_use]
    pub fn render(&self) -> Vec<String> {
        let mut out = vec!["Port readiness:".to_owned()];
        for line in &self.lines {
            let indent = if line.port.is_some() { "  " } else { "" };
            out.push(format!("{indent}{} [{}]", line.text, line.state.word()));
        }
        out.push(self.summary());
        out
    }

    fn push(
        &mut self,
        port: Option<&CanonicalId>,
        binding: Binding,
        state: LineState,
        text: String,
    ) {
        self.lines.push(ReadinessLine {
            port: port.cloned(),
            binding,
            state,
            text,
        });
    }
}

/// The credit instrument, for the renders.
fn credit() -> Option<InstrumentId> {
    InstrumentId::new(CREDIT)
}

/// An amount in major units, rendered on the credit's scale and symbol.
pub(crate) fn money_major(w: &World, major: u64) -> String {
    let (Some(economy), Some(instrument)) = (w.module_state::<EconomyStore>(), credit()) else {
        return format!("{major}cr");
    };
    let scale = economy
        .snapshot()
        .instrument(&instrument)
        .map_or(crate::economy::CREDIT_SCALE, |def| def.scale);
    let minor = 10_i64
        .checked_pow(u32::from(scale))
        .and_then(|factor| i64::try_from(major).ok()?.checked_mul(factor));
    match minor {
        Some(minor) => {
            mush_economy::render_amount(economy.snapshot(), &instrument, Amount::from_minor(minor))
        }
        None => format!("{major}cr"),
    }
}

/// The word for a capital tranche **no sweep will settle**: an obligation out
/// of the due pool that has not settled (cancelled, delinquent, suspended).
/// `None` for one still in the due pool (Active, Grace — pending) or Satisfied.
fn stuck_word(status: &mush_economy::ObligationStatus) -> Option<&'static str> {
    use mush_economy::ObligationStatus as S;
    match status {
        S::Active | S::Grace { .. } | S::Satisfied => None,
        S::Delinquent => Some("delinquent"),
        S::Suspended { .. } => Some("suspended"),
        S::Cancelled => Some("cancelled"),
    }
}

/// **The report** (doc 18 §4.7). A pure read: nothing is written.
#[must_use]
pub fn readiness(w: &World) -> Readiness {
    let mut report = Readiness::default();
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return report;
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        report.push(
            None,
            Binding::Lane,
            LineState::Missing,
            format!("lane: {LANE_PACK} is not applied on this world."),
        );
        return report;
    };
    let latched = store.applied_in(w, &lane);

    // ---- the lane ----
    let (state, text) = match latched {
        None => (
            LineState::Missing,
            format!("lane: {LANE_PACK} is not applied on this world."),
        ),
        Some(v) if v == LANE_VERSION => (
            LineState::Ready,
            format!("lane: {LANE_PACK}@{v} applied; this server ships @{LANE_VERSION}."),
        ),
        Some(v) if v < LANE_VERSION => (
            LineState::Flagged,
            format!(
                "lane: {LANE_PACK}@{v} applied; this server ships @{LANE_VERSION}. Take it with \
                 @atlas/pack plan {LANE_PACK}."
            ),
        ),
        Some(v) if store.pack(&lane, v).is_some() => (
            LineState::Flagged,
            format!("lane: {LANE_PACK}@{v} applied; this server ships @{LANE_VERSION}."),
        ),
        Some(v) => (
            LineState::Flagged,
            format!(
                "lane: {LANE_PACK}@{v} applied; this server ships @{LANE_VERSION} and does not \
                 load @{v}."
            ),
        ),
    };
    report.push(None, Binding::Lane, state, text);

    // The report's lane pack: the latched version when loaded, else the shipped.
    let (pack, ports) = report_ports(store, w);

    // ---- the sky ----
    match pack {
        None => report.push(
            None,
            Binding::Sky,
            LineState::Missing,
            "sky: no lane pack is loaded on this server.".to_owned(),
        ),
        Some(pack) => {
            let ephemeris = &store.config().time.ephemeris;
            let sky = Sky::from_pack(pack, ephemeris);
            let body_of = |place: &CanonicalId| match pack.entry(place) {
                Some(PackEntry::Location(def)) => def.body.clone(),
                _ => None,
            };
            // The first port entry's location body; for a pack without ports,
            // the first place that stands on a body.
            let body = ports
                .iter()
                .find_map(|(_, def)| body_of(&def.location))
                .or_else(|| {
                    pack.entries().iter().find_map(|(_, entry)| match entry {
                        PackEntry::Location(def) => def.body.clone(),
                        _ => None,
                    })
                });
            let epoch = body
                .and_then(|body| sky.epoch_of(&body))
                .unwrap_or_else(|| sky.config_epoch());
            // Doc 19 cut A.6 (§8): the motion and the model, so an operator reads
            // "real motion" and "kepler" where the slice read "constant".
            let motion = match ephemeris.rate {
                0 => "constant (rate 0)".to_owned(),
                1_000 => "moving (rate 1000, real motion)".to_owned(),
                rate => format!("moving (rate {rate})"),
            };
            let (state, revision) = match sky.revision() {
                Ok(digest) => (
                    LineState::Ready,
                    digest.to_hex().chars().take(12).collect::<String>(),
                ),
                Err(_) => (LineState::Flagged, "unavailable".to_owned()),
            };
            let text = format!(
                "sky: {motion}, {}, epoch {}, rev {revision}",
                sky.model(),
                epoch_date(epoch)
            );
            // Doc 18 cut IC12 (§4.8 "pages if the sky rate is non-zero while any
            // lane body lacks `epoch_ic`"): a moving sky moves such a body from the
            // server's epoch, not the date its numbers were read for — flagged here,
            // paged once per boot by the port tick (`port_tick`'s `sky_pages`), and
            // never gated (D-18-50). Rate 0 is untouched byte for byte.
            let (state, text) = match body_without_epoch(pack, ephemeris) {
                Some(body) => (
                    LineState::Flagged,
                    format!("{text}; {body} has no epoch_ic, so it moves from the server's epoch."),
                ),
                None => (state, text),
            };
            report.push(None, Binding::Sky, state, text);
        }
    }

    // ---- doc 20 cut SM.1: the geology key, right after the sky ----
    report.lines.push(crate::surface::key::geology_line(w));

    // ---- the world rate ----
    match w.world_timeline().epochs().last().map(|epoch| epoch.rate) {
        None => report.push(
            None,
            Binding::Rate,
            LineState::Missing,
            "world rate: this world has no time generation.".to_owned(),
        ),
        Some(rate) if rate == PLAY_RATE => report.push(
            None,
            Binding::Rate,
            LineState::Ready,
            format!("world rate: {rate} IC seconds per real second."),
        ),
        Some(rate) if store.test_world() => report.push(
            None,
            Binding::Rate,
            LineState::Ready,
            format!("world rate: {rate} IC seconds per real second (test world)."),
        ),
        Some(rate) => report.push(
            None,
            Binding::Rate,
            LineState::Flagged,
            format!("world rate: {rate} IC seconds per real second; play runs at {PLAY_RATE}."),
        ),
    }

    // ---- the treasury ----
    let mut treasuries: Vec<&str> = Vec::new();
    for (_, def) in &ports {
        if !treasuries.contains(&def.treasury.as_str()) {
            treasuries.push(def.treasury.as_str());
        }
    }
    if treasuries.is_empty() {
        treasuries.push(DEFAULT_TREASURY);
    }
    for treasury in treasuries {
        match treasury_account(w, treasury) {
            Some(account) => report.push(
                None,
                Binding::Treasury,
                LineState::Ready,
                format!("treasury: system:{treasury}, account #{account}."),
            ),
            None => report.push(
                None,
                Binding::Treasury,
                LineState::Missing,
                format!("treasury: no account is open for system:{treasury}."),
            ),
        }
    }

    // ---- the ports ----
    let mut seen: BTreeSet<CanonicalId> = BTreeSet::new();
    for (entry, def) in &ports {
        seen.insert(entry.clone());
        port_lines(&mut report, w, store, &lane, entry, Some(def));
    }
    // Any map present whose entry the report's pack does not author.
    for (map_pack, entry) in store.port_map_claims() {
        if map_pack == &lane && seen.contains(entry) {
            continue;
        }
        port_lines(&mut report, w, store, map_pack, entry, None);
    }
    report
}

/// **The `InventoryFacts` line** (doc 18 cut IC10; cut IC14 row 1): Ready when a
/// provider answers the `inventory` namespace and the applied-marker mirror is
/// complete (or no inventory store is composed to say otherwise — the `Markers`
/// line reports that); Missing when nothing answers, or (cut IC14) when the mirror
/// is incomplete, since an incomplete mirror answers **nothing** (E7c2 fold R3) and
/// no sale into a bid could settle.
#[must_use]
pub(crate) fn inventory_facts_line(provider: bool, complete: Option<bool>) -> ReadinessLine {
    let (state, text) = match (provider, complete) {
        (false, _) => (
            LineState::Missing,
            "inventory facts: nothing answers the inventory namespace, so sales into bids \
             refuse.",
        ),
        (true, Some(false)) => (
            LineState::Missing,
            "inventory facts: the applied-marker mirror is incomplete, so sales into bids \
             cannot settle.",
        ),
        (true, Some(true) | None) => (
            LineState::Ready,
            "inventory facts: answered, so sales into bids can settle.",
        ),
    };
    ReadinessLine {
        port: None,
        binding: Binding::InventoryFacts,
        state,
        text: text.to_owned(),
    }
}

/// **The `Markers` line** (doc 18 §9 risk 19, cut IC14 row 1): the inventory
/// mirror's headroom — `held` applied markers of the mirror's `max`. Ready while
/// `held` is at most three quarters of `max`; Flagged above it (the operator's
/// signal: past `max` every inventory fact goes dark, world-wide); Missing with no
/// inventory store composed. Reported and paged once per boot by the port tick,
/// **never** gated ([`gated`](crate::port::coop::gated); proposed Q-18-72).
#[must_use]
pub(crate) fn markers_line(held: Option<usize>, max: usize) -> ReadinessLine {
    let (state, text) = match held {
        None => (
            LineState::Missing,
            "markers: no inventory store is composed.".to_owned(),
        ),
        Some(n) if n.saturating_mul(4) <= max.saturating_mul(3) => (
            LineState::Ready,
            format!("markers: {n} of {max} applied markers held."),
        ),
        Some(n) => (
            LineState::Flagged,
            format!(
                "markers: {n} of {max} applied markers held; past {max} every inventory fact \
                 goes dark."
            ),
        ),
    };
    ReadinessLine {
        port: None,
        binding: Binding::Markers,
        state,
        text,
    }
}

/// **The report with cut IC8's lines** (doc 18 §4.7 "Boot-once acts and the
/// readiness report"; lead ruling L9): [`readiness`] — its IC3, IC4 and IC6 lines
/// byte-identical — with the `MarketFacts` line spliced in right after the
/// treasury line(s), (cut IC10) the `InventoryFacts` line right after it and (cut
/// IC14) the `Markers` headroom line right after that, and per port, after its last line (the hulls line when it
/// has one), one `Pools` line per vendor, one `Listings` line per ask and one
/// `Bids` line per bid, in authored order. Pools and receipts are typed state,
/// readable only through a module context, which is why this takes `ctx`;
/// `@tacsim/port status` and `+coop/enrol`'s gate both call it.
#[must_use]
pub fn readiness_in(ctx: &mush_core::Ctx<'_>) -> Readiness {
    let mut report = readiness(ctx.w);
    let w: &World = ctx.w;
    // ---- market facts, after the treasury ----
    let facts = if mush_economy::has_fact_provider(w, mush_market::NS) {
        ReadinessLine {
            port: None,
            binding: Binding::MarketFacts,
            state: LineState::Ready,
            text: "market facts: answered, so drawn sales can settle.".to_owned(),
        }
    } else {
        ReadinessLine {
            port: None,
            binding: Binding::MarketFacts,
            state: LineState::Missing,
            text: "market facts: nothing answers the market namespace, so drawn sales refuse."
                .to_owned(),
        }
    };
    // ---- doc 18 cut IC10: inventory facts, right after the market facts ----
    let inventory = w.module_state::<mush_inventory::InventoryStore>();
    let inventory_facts = inventory_facts_line(
        mush_economy::has_fact_provider(w, mush_inventory::NS),
        inventory.map(mush_inventory::InventoryStore::is_complete),
    );
    // ---- doc 18 §9 risk 19, cut IC14: the mirror's headroom, right after ----
    let markers = markers_line(
        inventory.map(mush_inventory::InventoryStore::markers),
        mush_inventory::MAX_APPLIED_MARKERS,
    );
    let after_treasury = report
        .lines
        .iter()
        .rposition(|line| line.binding == Binding::Treasury)
        .map_or(report.lines.len(), |i| i.saturating_add(1));
    report.lines.insert(after_treasury, facts);
    report
        .lines
        .insert(after_treasury.saturating_add(1), inventory_facts);
    report
        .lines
        .insert(after_treasury.saturating_add(2), markers);
    // ---- per port, after its last line ----
    let Some(store) = w.module_state::<TacsimStore>() else {
        return report;
    };
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return report;
    };
    let (_, ports) = report_ports(store, w);
    for (entry, def) in &ports {
        if def.vendors.is_empty() {
            continue;
        }
        let PortMapState::Present(map) = store.port_map(&lane, entry) else {
            continue;
        };
        let lines = commerce_lines(ctx, entry, def, map);
        let at = report
            .lines
            .iter()
            .rposition(|line| line.port.as_ref() == Some(entry))
            .map_or(report.lines.len(), |i| i.saturating_add(1));
        for (offset, line) in lines.into_iter().enumerate() {
            report.lines.insert(at.saturating_add(offset), line);
        }
    }
    report
}

/// The word a quantity of `good` is counted in (`t`, `crew-days`).
fn unit_word(goods: Option<&crate::definitions::GoodsTable>, good: &CanonicalId) -> String {
    match goods
        .and_then(|g| g.good(good))
        .map(|row| row.ledger_unit.as_str())
    {
        Some("crew_day") => "crew-days".to_owned(),
        Some(other) => other.to_owned(),
        None => "t".to_owned(),
    }
}

/// An amount on the credit's render.
fn money(w: &World, amount: Amount) -> String {
    match (w.module_state::<EconomyStore>(), credit()) {
        (Some(economy), Some(instrument)) => {
            mush_economy::render_amount(economy.snapshot(), &instrument, amount)
        }
        _ => format!("{}", amount.minor()),
    }
}

/// **What `readiness_in` read of one refresh row** (doc 20 cut SM.5, §7.3; factored
/// out by the lead's pre-commit round, item 3, so each line is tested without a world).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RefreshRead {
    /// No `BidRow` is recorded: the drip's case 1 posts it.
    Unrecorded,
    /// The listing stands Wanted.
    Standing {
        /// The listing.
        listing: u64,
        /// Its unfilled intake.
        quantity: u64,
        /// Its price, rendered.
        price: String,
        /// Whether the house can fund one unit at that price (`board::funds_one`).
        funded: bool,
    },
    /// Gone past its recorded deadline (or the market unreadable): the drip's case 4
    /// posts one drip at its next act.
    Lapsed {
        /// The recorded listing.
        listing: u64,
    },
    /// Gone before its recorded deadline — withdrawn by hand: the drip's case 5 waits for
    /// that deadline.
    Withdrawn {
        /// The recorded listing.
        listing: u64,
        /// The recorded deadline, as `when::render_due` prints it.
        due: String,
    },
}

/// **A refresh row's Bids line** (doc 20 cut SM.5, §7.3): never Missing — mining content
/// never closes the desk, and a Pending Bids line never shuts it (`coop::shuts`); the
/// intake drip heals each case. Each line says what the drip will actually do (the lead's
/// pre-commit round, item 3): an exhausted bid waits for the next IC day only while its
/// house can fund a unit at its price, and one withdrawn by hand waits for its recorded
/// deadline, not the next drip. Pure.
pub(crate) fn refresh_bid_line(
    head: &str,
    unit: &str,
    refresh: crate::port::def::RefreshDef,
    read: &RefreshRead,
) -> (LineState, String) {
    match read {
        RefreshRead::Unrecorded => (
            LineState::Pending,
            format!("{head}: no bid is posted yet; the clerk posts it at the next drip."),
        ),
        RefreshRead::Standing {
            listing,
            quantity: 0,
            funded: true,
            ..
        } => (
            LineState::Ready,
            format!(
                "{head}: listing #{listing} not buying until the next IC day; its intake is \
                 taken."
            ),
        ),
        RefreshRead::Standing {
            listing,
            quantity: 0,
            price,
            funded: false,
        } => (
            LineState::Pending,
            format!(
                "{head}: listing #{listing} not buying; its intake is taken, and its house \
                 cannot fund 1 {unit} at {price}/{unit}."
            ),
        ),
        RefreshRead::Standing {
            listing,
            quantity,
            price,
            funded: true,
        } => (
            LineState::Ready,
            format!(
                "{head}: listing #{listing} at {price}/{unit}, {quantity} of {} {unit} wanted; \
                 {} {unit} more each IC day.",
                refresh.cap, refresh.per_ic_day
            ),
        ),
        RefreshRead::Standing {
            listing,
            funded: false,
            ..
        } => (
            LineState::Pending,
            format!("{head}: listing #{listing}; not buying until its capital settles."),
        ),
        RefreshRead::Lapsed { listing } => (
            LineState::Pending,
            format!(
                "{head}: listing #{listing} is gone; the clerk posts it again at the next drip."
            ),
        ),
        RefreshRead::Withdrawn { listing, due } => (
            LineState::Pending,
            format!(
                "{head}: listing #{listing} was withdrawn; the clerk posts it again at its \
                 deadline, {due}."
            ),
        ),
    }
}

/// One vendor's stock rows for `good` (or all of them, `None`): `Ok((seeded rows,
/// rows, receipted quantity))`, or `Err` when a receipt or a lot cannot be read.
fn seeded_rows(
    ctx: &mush_core::Ctx<'_>,
    row: &crate::port::map::VendorRow,
    good: Option<&CanonicalId>,
) -> Result<(usize, usize, u64), ()> {
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .ok_or(())?;
    let inventory = handles.inventory.as_ref().ok_or(())?;
    let mut seeded = 0usize;
    let mut rows = 0usize;
    let mut total = 0u64;
    for stock in row
        .stock
        .iter()
        .filter(|s| good.is_none_or(|g| &s.good == g))
    {
        rows = rows.saturating_add(1);
        let id = crate::port::families::StockIssueId::parse(&stock.id).ok_or(())?;
        let Some(receipt) = crate::port::stock::receipt_at(ctx, &handles.families.port_stock, &id)?
        else {
            continue;
        };
        total = total.saturating_add(receipt.qty);
        let standing = ctx
            .action_state(&inventory.lots, &mush_inventory::record::LotId(receipt.lot))
            .map_err(|_| ())?;
        if standing.is_none() {
            seeded = seeded.saturating_add(1);
        }
    }
    Ok((seeded, rows, total))
}

/// **One port's IC8 lines** (cut IC8): the `Pools`, `Listings` and `Bids` lines.
#[allow(clippy::too_many_lines)]
fn commerce_lines(
    ctx: &mush_core::Ctx<'_>,
    entry: &CanonicalId,
    def: &PortDef,
    map: &PortMap,
) -> Vec<ReadinessLine> {
    let w: &World = ctx.w;
    let port = Some(entry.clone());
    let mut out: Vec<ReadinessLine> = Vec::new();
    let mut push = |binding: Binding, state: LineState, text: String| {
        out.push(ReadinessLine {
            port: port.clone(),
            binding,
            state,
            text,
        });
    };
    let goods = w.module_state::<TacsimStore>().and_then(TacsimStore::goods);
    let market = w.module_state::<mush_market::MarketStore>();
    let economy = w.module_state::<EconomyStore>();
    let org_name = |org: &CanonicalId| {
        def.orgs
            .iter()
            .find(|o| &o.id == org)
            .map_or_else(|| org.to_string(), |o| o.name.clone())
    };
    // ---- pools, per vendor ----
    for vendor in &def.vendors {
        let org = &vendor.org;
        let name = org_name(org);
        let Some(row) = map.vendors.get(org) else {
            push(
                Binding::Pools,
                LineState::Missing,
                format!("{entry} vendor {org}: not made."),
            );
            continue;
        };
        let head = format!(
            "{entry} vendor {org}: {name}, storefront vault #{}",
            row.vault
        );
        let vault_active = economy.is_some_and(|e| {
            e.snapshot()
                .vault(mush_economy::VaultId(row.vault))
                .is_some_and(|v| v.status == mush_economy::VaultStatus::Active)
        });
        if !vault_active {
            push(
                Binding::Pools,
                LineState::Missing,
                format!("{head} is not active."),
            );
            continue;
        }
        let registered = row.vendor.is_some_and(|v| {
            market.is_some_and(|m| {
                m.vendor(mush_market::VendorId(v))
                    .is_some_and(mush_market::Vendor::is_trading)
            })
        });
        if !registered {
            push(
                Binding::Pools,
                LineState::Missing,
                format!("{head}; it is not registered with the market."),
            );
            continue;
        }
        let clerk = map.clerk.as_ref().map_or(0, |c| c.thing);
        let clerk_live = w
            .get(mush_core::ObjId(clerk))
            .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING));
        if map.clerk.is_none() || !clerk_live {
            push(
                Binding::Pools,
                LineState::Missing,
                format!("{head}; its stock clerk #{clerk} does not exist."),
            );
            continue;
        }
        match seeded_rows(ctx, row, None) {
            Err(()) => push(
                Binding::Pools,
                LineState::Flagged,
                format!("{entry} vendor {org}: its stock cannot be read."),
            ),
            Ok((seeded, rows, _)) => push(
                Binding::Pools,
                if seeded == rows {
                    LineState::Ready
                } else {
                    LineState::Pending
                },
                format!("{head}; {seeded} of {rows} stock rows seeded."),
            ),
        }
    }
    // ---- listings, per ask ----
    for vendor in &def.vendors {
        let org = &vendor.org;
        for ask in &vendor.asks {
            let good = &ask.good;
            let unit = unit_word(goods, good);
            let head = format!("{entry} ask {org} {good}");
            let Some(row) = map.vendors.get(org) else {
                push(
                    Binding::Listings,
                    LineState::Pending,
                    format!("{head}: not seeded yet."),
                );
                continue;
            };
            let endpoint = mush_inventory::record::EndpointRef(mush_economy::VaultId(row.vault));
            let Ok(pool) = crate::port_tick::read_pool(ctx, endpoint, good) else {
                push(
                    Binding::Listings,
                    LineState::Flagged,
                    format!("{head}: its storefront cannot be read."),
                );
                continue;
            };
            let reading = market
                .map(|m| crate::port::stock::ask_reading(m, row.vault, good))
                .unwrap_or_default();
            let undrawn = reading.undrawn;
            match reading.listing {
                Some((listing, q)) => {
                    let price = market
                        .and_then(|m| m.listing(listing))
                        .map_or_else(String::new, |l| money(w, l.price));
                    let within = q <= pool && pool <= q.saturating_add(undrawn);
                    if !within {
                        push(
                            Binding::Listings,
                            LineState::Flagged,
                            format!(
                                "{head}: listing #{listing} lists {q} {unit} but the storefront \
                                 holds {pool} {unit}."
                            ),
                        );
                    } else if undrawn == 0 {
                        push(
                            Binding::Listings,
                            LineState::Ready,
                            format!(
                                "{head}: listing #{listing} at {price}/{unit}, {q} {unit} listed; \
                                 the storefront holds {pool} {unit}."
                            ),
                        );
                    } else {
                        push(
                            Binding::Listings,
                            LineState::Ready,
                            format!(
                                "{head}: listing #{listing} at {price}/{unit}, {q} {unit} listed; \
                                 the storefront holds {pool} {unit}, {undrawn} {unit} in hand-over."
                            ),
                        );
                    }
                }
                None => match seeded_rows(ctx, row, Some(good)) {
                    Err(()) => push(
                        Binding::Listings,
                        LineState::Flagged,
                        format!("{head}: its storefront cannot be read."),
                    ),
                    Ok((seeded, rows, _)) if seeded < rows || rows == 0 => push(
                        Binding::Listings,
                        LineState::Pending,
                        format!("{head}: not seeded yet."),
                    ),
                    Ok((_, _, total)) if pool <= undrawn => push(
                        Binding::Listings,
                        LineState::Ready,
                        format!("{head}: SOLD OUT: seeded {total} {unit}, 0 left."),
                    ),
                    Ok(_) => push(
                        Binding::Listings,
                        LineState::Pending,
                        format!("{head}: {pool} {unit} in the storefront wait to be listed."),
                    ),
                },
            }
        }
    }
    // ---- bids, per bid (lead ruling L8 reported them; IC12 gated them; cut P-B
    // gates Missing alone, `coop::shuts` — a Pending line here is reported, paged
    // once a boot by `port_tick` through `unfunded_bids`, and never gated) ----
    for vendor in &def.vendors {
        let org = &vendor.org;
        for bid in &vendor.bids {
            let good = &bid.good;
            let unit = unit_word(goods, good);
            let head = format!("{entry} bid {org} {good}");
            let recorded = map.vendors.get(org).and_then(|row| row.bids.get(good));
            // Doc 20 cut SM.5 (§7.3): a refreshed intake's problems are never Missing —
            // mining content never closes the desk, and a Pending Bids line never shuts
            // it (`coop::shuts`); the intake drip heals each of them.
            if let Some(refresh) = bid.refresh {
                let read = match recorded {
                    None => RefreshRead::Unrecorded,
                    Some(recorded) => {
                        let listing = mush_market::ListingId(recorded.listing);
                        let live = market
                            .and_then(|m| m.listing(listing).map(|l| (m, l)))
                            .filter(|(_, l)| l.source.is_wanted());
                        match live {
                            Some((m, l)) => RefreshRead::Standing {
                                listing: recorded.listing,
                                quantity: l.quantity,
                                price: money(w, l.price),
                                funded: crate::port::board::funds_one(w, m, listing),
                            },
                            // Withdrawn by hand before its deadline: the drip's case 5
                            // waits for that deadline (`posts_afresh`'s rule).
                            None if market.is_some() && w.now() < recorded.expires => {
                                RefreshRead::Withdrawn {
                                    listing: recorded.listing,
                                    due: crate::port::when::render_due(w, recorded.expires),
                                }
                            }
                            None => RefreshRead::Lapsed {
                                listing: recorded.listing,
                            },
                        }
                    }
                };
                let (state, text) = refresh_bid_line(&head, &unit, refresh, &read);
                push(Binding::Bids, state, text);
                continue;
            }
            let Some(recorded) = recorded else {
                push(
                    Binding::Bids,
                    LineState::Missing,
                    format!("{head}: no bid is posted."),
                );
                continue;
            };
            let listing = mush_market::ListingId(recorded.listing);
            let live = market
                .and_then(|m| m.listing(listing))
                .filter(|l| l.source.is_wanted());
            // Doc 18 cut IC10: an exhausted bid stands at quantity 0 — its intake is
            // taken — and an absent one is either lapsed (the clerk posts it again
            // while its recorded intake is the authored one; else the next apply
            // records the authored terms, after which the clerk does) or was
            // withdrawn by hand before its deadline.
            match (live, market) {
                (Some(l), Some(_)) if l.quantity == 0 => push(
                    Binding::Bids,
                    LineState::Ready,
                    format!("{head}: listing #{listing} not buying; its intake is taken."),
                ),
                (Some(l), Some(m)) => {
                    if crate::port::board::funds_one(w, m, listing) {
                        push(
                            Binding::Bids,
                            LineState::Ready,
                            format!(
                                "{head}: listing #{listing} at {}/{unit}, {} of {} {unit} wanted.",
                                money(w, l.price),
                                l.quantity,
                                bid.intake
                            ),
                        );
                    } else {
                        push(
                            Binding::Bids,
                            LineState::Pending,
                            format!(
                                "{head}: listing #{listing}; not buying until its capital settles."
                            ),
                        );
                    }
                }
                _ if w.now() < recorded.expires => push(
                    Binding::Bids,
                    LineState::Missing,
                    format!("{head}: no bid is posted."),
                ),
                _ if crate::port::vendor::posts_afresh(w, Some(recorded), bid) => push(
                    Binding::Bids,
                    LineState::Pending,
                    format!("{head}: listing #{listing} lapsed; the clerk posts it again."),
                ),
                _ => push(
                    Binding::Bids,
                    LineState::Missing,
                    format!("{head}: listing #{listing} lapsed; it waits for the next apply."),
                ),
            }
        }
    }
    out
}

/// **The standing bids whose house cannot fund one unit after its capital settled**
/// (doc 18 Q-18-60, the owner's R10; cut P-B), as `(port entry, org, good)` in
/// authored order: every lane port with a present map, every vendor bid recorded
/// there whose listing stands Wanted with intake left, whose house's capital has
/// settled (every tranche Satisfied — the port's `Org` line Ready) and whose house's
/// available balance is still under one unit's price (`board::funds_one`). These are
/// the bids [`readiness_in`] reads Pending, "not buying until its capital settles",
/// for play's reasons and not the boot's: a fresh world's bids are posted before the
/// first sweep settles the capital, and that window reads Pending too, beside an
/// `Org` line that says why. A pure read for the port tick's once-a-boot page
/// (`port_tick::funds_pages`); never a gate.
#[must_use]
pub(crate) fn unfunded_bids(w: &World) -> Vec<(CanonicalId, CanonicalId, CanonicalId)> {
    let mut out = Vec::new();
    let (Some(store), Some(market), Ok(lane)) = (
        w.module_state::<TacsimStore>(),
        w.module_state::<mush_market::MarketStore>(),
        CanonicalId::new(LANE_PACK),
    ) else {
        return out;
    };
    let economy = w.module_state::<EconomyStore>();
    let (_, ports) = report_ports(store, w);
    for (entry, def) in &ports {
        let PortMapState::Present(map) = store.port_map(&lane, entry) else {
            continue;
        };
        for vendor in &def.vendors {
            let Some(row) = map.vendors.get(&vendor.org) else {
                continue;
            };
            // The capital settled: every tranche the org row was created with is
            // Satisfied (the `Org` line's Ready reading); a row with none to wait on
            // counts as settled.
            let settled = map.orgs.get(&vendor.org).is_some_and(|org| {
                org.capital.iter().all(|t| {
                    economy.is_some_and(|e| {
                        e.snapshot()
                            .obligation(ObligationId(t.obligation))
                            .is_some_and(|o| {
                                matches!(o.status, mush_economy::ObligationStatus::Satisfied)
                            })
                    })
                })
            });
            if !settled {
                continue;
            }
            for bid in &vendor.bids {
                let Some(recorded) = row.bids.get(&bid.good) else {
                    continue;
                };
                let listing = mush_market::ListingId(recorded.listing);
                let standing = market
                    .listing(listing)
                    .is_some_and(|l| l.source.is_wanted() && l.quantity > 0);
                if standing && !crate::port::board::funds_one(w, market, listing) {
                    out.push((entry.clone(), vendor.org.clone(), bid.good.clone()));
                }
            }
        }
    }
    out
}

/// **The lane body a moving sky would move from the server's epoch** (doc 18 cut
/// IC12; §4.8): when the configured sky rate is non-zero, the first `body` entry of
/// `pack` (canonical-id order, the pack's own entry order) without an `epoch_ic`;
/// `None` at rate 0 — the constant sky holds every body at its authored phase, so a
/// missing `epoch_ic` moves nothing — and `None` when every body carries one.
/// [`readiness`] flags the Sky line on it; the port tick pages it once per boot.
#[must_use]
pub(crate) fn body_without_epoch(
    pack: &RulesPack,
    ephemeris: &crate::config::EphemerisConfig,
) -> Option<CanonicalId> {
    if ephemeris.rate == 0 {
        return None;
    }
    pack.entries().iter().find_map(|(id, entry)| match entry {
        PackEntry::Body(def) if def.epoch_ic.is_none() => Some(id.clone()),
        _ => None,
    })
}

/// **The report's lane pack and its port entries** (doc 18 §4.7; factored out
/// by cut IC6 so `+coop`, `+ship` and the look hook resolve the ports exactly as
/// the report does): the latched version when this server loads it, else the
/// shipped [`LANE_VERSION`] when loaded, else none — an exact `(id, version)`
/// lookup, never `TacsimStore::resolve` — and its `port` entries in pack order.
pub(crate) fn report_ports<'a>(
    store: &'a TacsimStore,
    w: &World,
) -> (Option<&'a RulesPack>, Vec<(CanonicalId, &'a PortDef)>) {
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return (None, Vec::new());
    };
    let pack: Option<&RulesPack> = store
        .applied_in(w, &lane)
        .and_then(|v| store.pack(&lane, v))
        .or_else(|| store.pack(&lane, LANE_VERSION))
        .map(|loaded| &loaded.pack);
    let ports: Vec<(CanonicalId, &PortDef)> = pack
        .map(|pack| {
            pack.entries()
                .iter()
                .filter_map(|(id, entry)| match entry {
                    PackEntry::Port(def) => Some((id.clone(), def)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    (pack, ports)
}

/// **Whether one port's own lines are all Ready** (doc 20 cut SM.2, lead override 3):
/// the lines [`readiness`] reports for `entry` of the report's lane pack — its map,
/// rooms, domain, site, institutions, programme and hulls — every one Ready. A port the
/// report's pack does not author is not ready. What a departure to a site asks before
/// it plans ([`crate::port::voyage`]'s `SITE_CLOSED`).
#[must_use]
pub(crate) fn port_ready(w: &World, entry: &CanonicalId) -> bool {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return false;
    };
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return false;
    };
    let (_, ports) = report_ports(store, w);
    let Some((_, def)) = ports.into_iter().find(|(id, _)| id == entry) else {
        return false;
    };
    let mut report = Readiness::default();
    port_lines(&mut report, w, store, &lane, entry, Some(def));
    report.is_ready()
}

/// The lowest-id Active `tacsim.credit` account `system:<treasury>` holds.
fn treasury_account(w: &World, treasury: &str) -> Option<AccountId> {
    let owner = PrincipalId::System(SystemPrincipalId::new(treasury)?);
    let instrument = credit()?;
    w.module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(&owner)
        .filter(|a| a.status == AccountStatus::Active && a.instrument == instrument)
        .map(|a| a.id)
        .min()
}

/// One port's lines: its map, then (when readable) its rooms, its domain, one
/// line per institution and its programme. `def` is `None` for a map no loaded
/// lane pack authors; its lines come from the map alone.
#[allow(clippy::too_many_lines)]
fn port_lines(
    report: &mut Readiness,
    w: &World,
    store: &TacsimStore,
    pack: &CanonicalId,
    entry: &CanonicalId,
    def: Option<&PortDef>,
) {
    let port = Some(entry);
    let key = port_map_key(pack, entry);
    // Doc 20 cut SM.2 (lead override 3): a site port's lines never gate `+coop/enrol`,
    // its Map line included — so the port is named a site before the map is read, and an
    // absent or unreadable site map closes the pad (`port_ready`), never the desk.
    if def.is_some_and(|def| def.site.is_some()) {
        report.sites.insert(entry.clone());
    }
    let map: &PortMap = match store.port_map(pack, entry) {
        PortMapState::Unreadable => {
            report.push(
                port,
                Binding::Map,
                LineState::Missing,
                format!("{entry}: its port map {key} is unreadable."),
            );
            return;
        }
        PortMapState::Absent => {
            report.push(
                port,
                Binding::Map,
                LineState::Missing,
                format!("{entry}: not applied."),
            );
            return;
        }
        PortMapState::Present(map) => map,
    };

    // ---- rooms ----
    let gallery = def.and_then(|def| {
        room_key(&def.area, &def.gallery)
            .ok()
            .and_then(|key| room_by_key(w, &key))
    });
    if let Some(def) = def {
        let refs: BTreeSet<&str> = room_references(def).into_iter().collect();
        let unresolved: Vec<&str> = refs
            .iter()
            .copied()
            .filter(|reference| {
                room_key(&def.area, reference)
                    .ok()
                    .and_then(|key| room_by_key(w, &key))
                    .is_none()
            })
            .collect();
        let total = refs.len();
        let resolved = total.saturating_sub(unresolved.len());
        let docked =
            gallery.is_some_and(|room| w.has_marker(room, mush_atlas::def::PORT_DOCK_MARKER));
        let marker = mush_atlas::def::PORT_DOCK_MARKER;
        let (state, tail) = if !unresolved.is_empty() {
            (
                LineState::Missing,
                format!("unresolved: {}", unresolved.join(", ")),
            )
        } else if docked {
            (LineState::Ready, format!("the gallery carries {marker}"))
        } else {
            (
                LineState::Missing,
                format!("the gallery does not carry {marker}"),
            )
        };
        report.push(
            port,
            Binding::Rooms,
            state,
            format!("{entry} rooms: {resolved} of {total} resolved; {tail}."),
        );
    }

    // ---- domain ----
    let spatial = w.module_state::<mush_sim::data::SpatialState>();
    let (state, text) = match map.domain {
        None => (
            LineState::Missing,
            format!("{entry} domain: none is recorded in {key}."),
        ),
        Some(domain) => {
            let record = spatial
                .and_then(|s| s.domains.get(&mush_sim::data::DomainId(domain)))
                .filter(|d| d.status == mush_sim::data::DomainStatus::Active);
            match record {
                None => (
                    LineState::Missing,
                    format!("{entry} domain: #{domain} is not active on this world."),
                ),
                Some(record) if map.host.is_some_and(|host| host != record.host.0) => (
                    LineState::Missing,
                    format!(
                        "{entry} domain: #{domain} is hosted on {}, not the room its map names.",
                        display_name(w, record.host)
                    ),
                ),
                Some(record) if gallery.is_some_and(|g| g != record.presentation_location) => (
                    LineState::Missing,
                    format!(
                        "{entry} domain: #{domain} presents in {}, not the gallery.",
                        display_name(w, record.presentation_location)
                    ),
                ),
                Some(record) => (
                    LineState::Ready,
                    format!(
                        "{entry} domain: #{domain} on {}, presents in {}.",
                        display_name(w, record.host),
                        display_name(w, record.presentation_location)
                    ),
                ),
            }
        }
    };
    report.push(port, Binding::Domain, state, text);

    // ---- doc 20 cut SM.2: the site's block places and clerk ----
    if def.is_some_and(|def| def.site.is_some()) {
        let (state, text) = crate::port::site::site_line(w, entry, map);
        report.push(port, Binding::Site, state, text);
    }

    // ---- institutions ----
    let orgs = w.module_state::<OrgStore>();
    let economy = w.module_state::<EconomyStore>();
    // (id, name, capital_major, tranche count): authored, or from the map. Doc 20
    // cut SM.5 (§7.3): the capital is the base with every raise, and the tranches
    // are the schedule's.
    let rows: Vec<(CanonicalId, String, u64, usize)> = match def {
        Some(def) => def
            .orgs
            .iter()
            .map(|org| {
                (
                    org.id.clone(),
                    org.name.clone(),
                    org.capital_total().unwrap_or(org.capital_major),
                    capital_schedule(org.capital_major, &org.raises).len(),
                )
            })
            .collect(),
        None => map
            .orgs
            .iter()
            .map(|(id, row)| {
                let name = orgs
                    .and_then(|o| o.state().org(OrgId(row.org)))
                    .map_or_else(|| format!("org #{}", row.org), |o| o.name.to_string());
                // The capital the row was created for (L4) with the raises it has
                // begun (doc 20 cut SM.5), and its tranches.
                let capital = row
                    .raises
                    .iter()
                    .try_fold(row.capital_major, |total, raise| total.checked_add(*raise))
                    .unwrap_or(row.capital_major);
                (
                    id.clone(),
                    name,
                    capital,
                    capital_schedule(row.capital_major, &row.raises).len(),
                )
            })
            .collect(),
    };
    for (id, name, capital, want) in rows {
        let head = format!("{entry} org {id}: {name}");
        let Some(row) = map.orgs.get(&id) else {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!("{head}; the org is not created."),
            );
            continue;
        };
        if orgs.and_then(|o| o.state().org(OrgId(row.org))).is_none() {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!("{head}; org #{} does not exist.", row.org),
            );
            continue;
        }
        let account = row
            .account
            .filter(|a| economy.is_some_and(|e| e.snapshot().account(AccountId(*a)).is_some()));
        let Some(account) = account else {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!("{head}, org #{}; its account is not open.", row.org),
            );
            continue;
        };
        let missing = row.capital.iter().find(|t| {
            economy.is_none_or(|e| {
                e.snapshot()
                    .obligation(ObligationId(t.obligation))
                    .is_none()
            })
        });
        if let Some(gone) = missing {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!(
                    "{head}, org #{}, account #{account}; capital obligation #{} does not exist.",
                    row.org, gone.obligation
                ),
            );
            continue;
        }
        if row.capital.len() < want {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!(
                    "{head}, org #{}, account #{account}; {} of {want} capital tranches are \
                     created.",
                    row.org,
                    row.capital.len()
                ),
            );
            continue;
        }
        // A tranche no sweep will settle — cancelled, delinquent (a live mint-cap
        // re-check refused it past its grace) or suspended: out of the due pool
        // (`ObligationStatus::in_due_pool`), so "pending" would tell staff to
        // wait for a sweep that never comes. Missing, naming it.
        let stuck = row.capital.iter().find_map(|t| {
            economy
                .and_then(|e| e.snapshot().obligation(ObligationId(t.obligation)))
                .and_then(|o| stuck_word(&o.status).map(|word| (t.obligation, word)))
        });
        if let Some((obligation, word)) = stuck {
            report.push(
                port,
                Binding::Org,
                LineState::Missing,
                format!(
                    "{head}, org #{}, account #{account}; capital obligation #{obligation} is \
                     {word}, and no sweep will settle it.",
                    row.org
                ),
            );
            continue;
        }
        let settled = row
            .capital
            .iter()
            .filter(|t| {
                economy.is_some_and(|e| {
                    e.snapshot()
                        .obligation(ObligationId(t.obligation))
                        .is_some_and(|o| {
                            matches!(o.status, mush_economy::ObligationStatus::Satisfied)
                        })
                })
            })
            .count();
        let state = if settled == want {
            LineState::Ready
        } else {
            LineState::Pending
        };
        report.push(
            port,
            Binding::Org,
            state,
            format!(
                "{head}, org #{}, account #{account}, capital {}: {settled} of {want} tranches \
                 settled.",
                row.org,
                money_major(w, capital)
            ),
        );
    }

    // ---- the programme ----
    if let Some(programme) = def.and_then(|def| def.programme.as_ref()) {
        let org_name = def
            .and_then(|def| def.orgs.iter().find(|org| org.id == programme.org))
            .map_or_else(|| programme.org.to_string(), |org| org.name.clone());
        let account = map
            .orgs
            .get(&programme.org)
            .and_then(|row| row.account)
            .filter(|a| economy.is_some_and(|e| e.snapshot().account(AccountId(*a)).is_some()));
        match account {
            Some(account) => report.push(
                port,
                Binding::Programme,
                LineState::Ready,
                format!(
                    "{entry} programme {}: {org_name}, account #{account}, floor {}, stake {} for \
                     each of {} hulls.",
                    programme.id,
                    money_major(w, programme.floor_major),
                    money_major(w, programme.stake_major),
                    programme.pool
                ),
            ),
            None => report.push(
                port,
                Binding::Programme,
                LineState::Missing,
                format!(
                    "{entry} programme {}: {org_name} has no account open.",
                    programme.id
                ),
            ),
        }
    }

    // ---- the hulls (cut IC4) ----
    if let Some((state, text)) =
        def.and_then(|def| crate::port::hull::hull_line(w, store, entry, def, map))
    {
        report.push(port, Binding::Hulls, state, text);
    }

    // ---- the rig pool (doc 20 cut SM.3a), right after the hulls ----
    if let Some((state, text)) =
        def.and_then(|def| crate::port::vehicle::vehicles_line(w, store, entry, def, map))
    {
        report.push(port, Binding::Vehicles, state, text);
    }

    // ---- the prospecting licence (doc 20 cut SM.5), after the rig pool ----
    if let Some((state, text)) =
        def.and_then(|def| crate::port::licence::licence_line(w, entry, def, map))
    {
        report.push(port, Binding::Licence, state, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc 20 cut SM.5 (§7.3; the lead's pre-commit round, items 3(a), 3(b) and 6(c)): every
    /// refresh row's Bids line, whole, and its state — none Missing. An exhausted bid
    /// waits for the next IC day only while its house can fund a unit at its price; one
    /// withdrawn by hand waits for its recorded deadline, named; one lapsed comes back at
    /// the next drip.
    #[test]
    fn sm5_each_refresh_bid_line_says_what_the_drip_does() {
        let head = "ceres_port bid freeport_exchange metal_ore";
        let refresh = crate::port::def::RefreshDef {
            per_ic_day: 40,
            cap: 240,
        };
        let line = |read: RefreshRead| refresh_bid_line(head, "t", refresh, &read);
        let standing = |quantity: u64, funded: bool| RefreshRead::Standing {
            listing: 7,
            quantity,
            price: "70.00cr".to_owned(),
            funded,
        };
        let cases = [
            (
                RefreshRead::Unrecorded,
                LineState::Pending,
                "ceres_port bid freeport_exchange metal_ore: no bid is posted yet; the clerk \
                 posts it at the next drip.",
            ),
            (
                standing(0, true),
                LineState::Ready,
                "ceres_port bid freeport_exchange metal_ore: listing #7 not buying until the \
                 next IC day; its intake is taken.",
            ),
            (
                standing(0, false),
                LineState::Pending,
                "ceres_port bid freeport_exchange metal_ore: listing #7 not buying; its intake \
                 is taken, and its house cannot fund 1 t at 70.00cr/t.",
            ),
            (
                standing(154, true),
                LineState::Ready,
                "ceres_port bid freeport_exchange metal_ore: listing #7 at 70.00cr/t, 154 of \
                 240 t wanted; 40 t more each IC day.",
            ),
            (
                standing(154, false),
                LineState::Pending,
                "ceres_port bid freeport_exchange metal_ore: listing #7; not buying until its \
                 capital settles.",
            ),
            (
                RefreshRead::Lapsed { listing: 7 },
                LineState::Pending,
                "ceres_port bid freeport_exchange metal_ore: listing #7 is gone; the clerk \
                 posts it again at the next drip.",
            ),
            (
                RefreshRead::Withdrawn {
                    listing: 7,
                    due: "14:00 UTC tomorrow (IC 2350-05-21; in 20 h 5 min)".to_owned(),
                },
                LineState::Pending,
                "ceres_port bid freeport_exchange metal_ore: listing #7 was withdrawn; the \
                 clerk posts it again at its deadline, 14:00 UTC tomorrow (IC 2350-05-21; in \
                 20 h 5 min).",
            ),
        ];
        for (read, state, text) in cases {
            assert_eq!(line(read.clone()), (state, text.to_owned()), "{read:?}");
        }
    }

    #[test]
    fn ic3_the_summary_counts_what_is_not_ready() {
        let mut report = Readiness::default();
        report.push(None, Binding::Lane, LineState::Ready, "lane".to_owned());
        assert!(report.is_ready());
        assert_eq!(report.summary(), "Ready: every binding above is in place.");
        let port = CanonicalId::new("ceres_port").ok();
        report.push(
            port.as_ref(),
            Binding::Org,
            LineState::Pending,
            "org".to_owned(),
        );
        assert!(!report.is_ready());
        assert_eq!(
            report.summary(),
            "Not ready: 1 of 2 lines are pending, missing or flagged."
        );
        assert_eq!(
            report.render(),
            vec![
                "Port readiness:".to_owned(),
                "lane [ready]".to_owned(),
                "  org [pending]".to_owned(),
                "Not ready: 1 of 2 lines are pending, missing or flagged.".to_owned(),
            ]
        );
    }

    /// Doc 18 §9 risk 19, cut IC14: the `Markers` line is Ready up to three
    /// quarters of the mirror's cap and Flagged past it (3,072 / 3,073 of 4,096),
    /// Missing with no store — and it never gates `+coop/enrol`.
    #[test]
    fn ic14_the_markers_line_reports_headroom_and_never_gates() {
        let max = mush_inventory::MAX_APPLIED_MARKERS;
        let ready = markers_line(Some(3_072), max);
        assert_eq!(
            (ready.binding, ready.state, ready.text.as_str()),
            (
                Binding::Markers,
                LineState::Ready,
                "markers: 3072 of 4096 applied markers held."
            )
        );
        let flagged = markers_line(Some(3_073), max);
        assert_eq!(flagged.state, LineState::Flagged);
        assert_eq!(
            flagged.text,
            "markers: 3073 of 4096 applied markers held; past 4096 every inventory fact goes \
             dark."
        );
        let none = markers_line(None, max);
        assert_eq!(
            (none.state, none.text.as_str()),
            (
                LineState::Missing,
                "markers: no inventory store is composed."
            )
        );
        for line in [flagged, none] {
            let report = Readiness {
                lines: vec![line],
                ..Readiness::default()
            };
            assert!(
                crate::port::coop::enrol_gate(&report).is_ok(),
                "reported, never gated"
            );
            assert!(!report.is_ready());
        }
    }

    /// Doc 18 cut IC14 row 1: an incomplete applied-marker mirror answers no
    /// inventory fact, so the `InventoryFacts` line goes Missing — and that line
    /// gates enrolment.
    #[test]
    fn ic14_an_incomplete_mirror_closes_enrolment() {
        let incomplete = inventory_facts_line(true, Some(false));
        assert_eq!(
            (
                incomplete.binding,
                incomplete.state,
                incomplete.text.as_str()
            ),
            (
                Binding::InventoryFacts,
                LineState::Missing,
                "inventory facts: the applied-marker mirror is incomplete, so sales into bids \
                 cannot settle."
            )
        );
        let report = Readiness {
            lines: vec![incomplete],
            ..Readiness::default()
        };
        assert!(
            crate::port::coop::enrol_gate(&report).is_err(),
            "the gate shuts on an incomplete mirror"
        );
        assert_eq!(
            inventory_facts_line(true, Some(true)).text,
            "inventory facts: answered, so sales into bids can settle."
        );
        assert_eq!(inventory_facts_line(true, None).state, LineState::Ready);
        assert_eq!(
            inventory_facts_line(false, Some(true)).text,
            "inventory facts: nothing answers the inventory namespace, so sales into bids \
             refuse."
        );
    }

    /// Doc 18 Q-18-60 (the owner's R10 of 2026-10-18), cut P-B: a `Bids` line that
    /// reads Pending — a house that cannot fund a tonne, or a lapsed bid the clerk
    /// re-posts — leaves `+coop/enrol` open while the report still counts it; one
    /// that reads Missing — withdrawn by hand, or lapsed and waiting for the next
    /// apply — closes the desk, and the gate names it.
    #[test]
    fn pb_a_funds_pending_bid_leaves_enrolment_open_and_a_missing_one_closes_it() {
        let port = CanonicalId::new("stickney_port").ok();
        let bid = |state: LineState, text: &str| ReadinessLine {
            port: port.clone(),
            binding: Binding::Bids,
            state,
            text: text.to_owned(),
        };
        for text in [
            "stickney_port bid kasei metals: listing #7; not buying until its capital settles.",
            "stickney_port bid kasei metals: listing #7 lapsed; the clerk posts it again.",
        ] {
            let line = bid(LineState::Pending, text);
            assert!(!crate::port::coop::shuts(&line), "{text}");
            let report = Readiness {
                lines: vec![line],
                ..Readiness::default()
            };
            assert!(
                crate::port::coop::enrol_gate(&report).is_ok(),
                "a pending bid leaves the desk open: {text}"
            );
            assert!(!report.is_ready(), "the report still counts it: {text}");
        }
        for text in [
            "stickney_port bid kasei metals: no bid is posted.",
            "stickney_port bid kasei metals: listing #7 lapsed; it waits for the next apply.",
        ] {
            let line = bid(LineState::Missing, text);
            assert!(crate::port::coop::shuts(&line), "{text}");
            let report = Readiness {
                lines: vec![line],
                ..Readiness::default()
            };
            let refused =
                crate::port::coop::enrol_gate(&report).expect_err("a missing bid closes the desk");
            assert_eq!(
                (refused.binding, refused.state, refused.text.as_str()),
                (Binding::Bids, LineState::Missing, text)
            );
        }
    }
}
