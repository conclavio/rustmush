//! **The port readiness report** (doc 18 cut IC3; doc 18 §4.7 "Readiness";
//! lead ruling L14).
//!
//! A pure read of everything the port entries bound: the lane's latched version
//! against the one this server ships, the sky the lane stands under, the world
//! rate, the treasury, and per port its map, rooms, domain, institutions (each
//! with its account and capital) and programme. `@tacsim/port status` renders
//! it; cut IC6 gates `+coop/enrol` on [`Readiness::is_ready`] and reads the
//! typed [`LineState`]s, never the text. Cut IC4's hulls line has landed
//! ([`Binding::Hulls`], computed by `crate::port::hull::hull_line`); cut IC8
//! adds its own (the pools and listings).
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
use crate::port::def::{PortDef, room_key, tranches};
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
fn money_major(w: &World, major: u64) -> String {
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
    let pack: Option<&RulesPack> = latched
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
            let motion = if ephemeris.rate == 0 {
                "constant (rate 0)".to_owned()
            } else {
                format!("moving (rate {})", ephemeris.rate)
            };
            let (state, revision) = match sky.revision() {
                Ok(digest) => (
                    LineState::Ready,
                    digest.to_hex().chars().take(12).collect::<String>(),
                ),
                Err(_) => (LineState::Flagged, "unavailable".to_owned()),
            };
            report.push(
                None,
                Binding::Sky,
                state,
                format!("sky: {motion}, epoch {}, rev {revision}", epoch_date(epoch)),
            );
        }
    }

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

    // ---- institutions ----
    let orgs = w.module_state::<OrgStore>();
    let economy = w.module_state::<EconomyStore>();
    // (id, name, capital_major, tranche count): authored, or from the map.
    let rows: Vec<(CanonicalId, String, u64, usize)> = match def {
        Some(def) => def
            .orgs
            .iter()
            .map(|org| {
                (
                    org.id.clone(),
                    org.name.clone(),
                    org.capital_major,
                    tranches(org.capital_major).len(),
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
                // The capital the row was created for (L4), and its tranches.
                (
                    id.clone(),
                    name,
                    row.capital_major,
                    tranches(row.capital_major).len(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
