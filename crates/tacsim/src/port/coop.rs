//! **The Co-op's First Captain programme** (doc 18 cut IC6; doc 18 §4.1,
//! D-18-6, D-18-21, D-18-23, D-18-24, D-18-25; lead rulings L1–L21 of the IC6
//! brief).
//!
//! A pilot at the Registry & Co-op desk types `+coop/enrol <name>` and, in **one
//! command and one batch**, becomes a First Captain: Ceres Clearing opens their
//! `tacsim.credit` account when they have none (D-18-6), the Co-op First Captain
//! Trust leases them its lowest-id free Dray through the economy's own
//! `offer_lease` + `accept_contract` (rent 1 minor on the Manual clock, D-18-21,
//! L3), the Trust posts the stake as an org actor keyed by the lease
//! (`tacsim.backer.stake/<lease>`), the hull's Hold Stowage gets the pilot's hold
//! vault (L4), the hull takes the pilot's name and keeps its registry mark, and
//! one [`Enrolment`] document records the frozen terms (D-18-24, D-18-25).
//!
//! **The operate gate is the live lease, never this document** (D-18-23, L8):
//! [`live_charter`] reads the economy's contracts and nothing else. The document
//! is the account ledger — "one enrolment per gateway account, ever" (D-18-25,
//! L5), keyed `tacsim:coop:{account}` so an unreadable one bars only its own
//! account — and the frozen terms a statement reads. Cut IC10 folds sales into
//! it; cut IC13 adds the buyout and graduation. A staff-ended lease is folded
//! into [`EnrolmentStatus::Revoked`] by [`CoopObserver`].
//!
//! Every write of the enrolment follows recovery's rule
//! (`mush-campaign/src/recoverycmd.rs` `try_grant`): every check first, then
//! account → vault id → offer → accept → **post last** → vault record, rename,
//! document (L6). A refusal after the offer ends the lease through
//! `mush_economy::end_lease`; past the post nothing is fallible, so nothing is
//! unwound.

use mush_core::{Attr, CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, ContractId, ContractState, ContractStateKind,
    CreditPolicy, CustodyProvider, CustodyRef, Deliverable, EconomicCause, EconomicClock,
    EconomicContract, EconomicEvent, EconomicSchedule, EconomyObserver, EconomyPlan, EconomyStore,
    GracePolicy, IdempotencyKey, InstrumentId, LeaseKey, LeaseSpec, ObligationTemplate,
    PostOutcome, RequestedLine, Vault, VaultStatus,
};
use mush_org::{OrgId, OrgScopeId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::commands::NOT_ENABLED;
use crate::definitions::content::LANE_PACK;
use crate::economy::CREDIT;
use crate::port::apply::{credit_scale, minor_of, room_by_key};
use crate::port::def::{PortDef, ProgrammeDef, room_key};
use crate::port::map::{HullRow, PortMap, PortMapState};
use crate::port::readiness::{
    Binding, LineState, Readiness, ReadinessLine, readiness_in, report_ports,
};
use crate::store::{TacsimStore, put_enrolment};

/// The verb.
pub const COOP_VERB: &str = "+coop";

/// `+coop`'s help (doc 18 §4.1).
pub const COOP_HELP: &str = "+coop  --  The Belt Carriers' Cooperative's First Captain \
     programme. The Co-op stakes a new captain a Dray on a working charter and twenty-five \
     thousand in working cash, takes a quarter of trading margin over its reference prices until \
     the hull is bought out, and enrols each pilot once, ever. +coop reads the terms and how many \
     Drays are free, anywhere.\n+coop/enrol <name>  sign at the Registry & Co-op desk in Ceres \
     Freeport: Ceres Clearing opens your credit account, the Co-op charters you its next free \
     Dray under the name you give it (its registry mark stays), stakes you, and opens the ship's \
     hold in your name.\n+coop/statement  your charter's terms, its sales and the Co-op's share \
     so far, and your cash against the buyout, anywhere.";

/// The prefix of every enrolment document (`tacsim:coop:{account}`).
pub const COOP_PREFIX: &str = "tacsim:coop:";

/// The enrolment document's schema version.
pub const ENROLMENT_SCHEMA: u16 = 1;

/// The stake's idempotency namespace (doc 18 §4.1: `tacsim.backer.stake/<lease
/// contract>`).
pub const STAKE_KEY_NAMESPACE: &str = "tacsim.backer.stake";

/// The longest ship name a pilot may give, in bytes.
pub const MAX_HULL_NAME_BYTES: usize = 32;

/// The longest name, mark or laid-up name an enrolment document may carry.
const MAX_ENROLMENT_TEXT_BYTES: usize = 120;

/// **The registrar's line** — doc 18 §4.1's fiction line, verbatim.
pub const REGISTRAR_LINE: &str = "The Co-op stakes new captains: a Co-op Dray on a working \
     charter, twenty-five thousand in working cash, and a quarter of your trading margin to the \
     Co-op until you buy the hull out at the price on your papers. Sell under the Co-op's \
     reference price and the Co-op takes nothing. Go a week without a departure and the Co-op \
     takes its Dray back.";

/// The Co-op desk officer who countersigns (18a §4; doc 18 §2 step 4).
pub const COUNTERSIGN: &str = "Ilse Oduya-Brandt";

/// Who opens a new captain's account (D-18-6).
pub const ACCOUNT_OPENER: &str = "Ceres Clearing";

/// The Yard's new-Dray comparator, major units (doc 18 §4.1; 18a C05's lay
/// board).
pub const NEW_DRAY_MAJOR: u64 = 120_000;

/// The description a chartered hull takes when it has none (lead ruling L10),
/// `{mark}` replaced.
pub const HULL_DESC_TEMPLATE: &str = "A Dray-class bulk hauler, {mark}, cradled with its \
     umbilicals slack: blunt, grey and webbed with reaction-mass hoses.";

/// The programme is not on this world.
pub const NOT_OPEN_HERE: &str = "The Co-op's programme is not open on this world.";
/// `+coop/enrol` with no name.
pub const NAME_USAGE: &str = "Usage: +coop/enrol <a name for your ship>";
/// The gateway account is unknown (fail closed).
pub const NO_ACCOUNT_MATCH: &str =
    "The Registry cannot match you to an account, so the Co-op cannot enrol you.";
/// One enrolment per gateway account, ever (D-18-25).
pub const ENROLLED_ONCE: &str =
    "The Registry already has you on its books: the Co-op enrols each pilot once, ever.";
/// The account's enrolment document does not read.
pub const BOOKS_UNREADABLE: &str =
    "The Co-op's books for your account cannot be read right now. Ask staff to look at them.";
/// The readiness gate is shut (L1).
pub const NOT_OPEN: &str = "The Co-op is not enrolling yet: the programme is still being set up.";
/// A ship name the Registry refuses.
pub const BAD_NAME: &str = "A ship's name is 1 to 32 letters, digits, spaces, hyphens, \
     apostrophes or full stops, starting with a letter.";
/// A ship name already on the register.
pub const NAME_TAKEN: &str = "The Registry already lists a Dray by that name.";
/// No hull is free.
pub const NO_HULL: &str = "The Co-op has no Dray free to charter.";
/// The programme's capital is at its floor.
pub const NO_CAPITAL: &str = "The Co-op's programme has no capital left to stake a new captain.";
/// An enrolment step that should not fail did (lead ruling L16: the text
/// promises only that no charter stands).
pub const FAILED: &str =
    "The Registry could not complete your enrolment; your charter was not signed.";
/// No live charter and no ended one.
pub const NO_CHARTER: &str = "You hold no charter.";

/// **A charter whose document is dark** (lead ruling L21): the hull's current
/// name and mark replace `{name}` and `{mark}`.
pub const CHARTER_BOOKS_DARK: &str = "Your charter: the {name} ({mark}); the Co-op's books for \
     your account cannot be read right now.";

/// **A charter on the books whose lease is not in force** (D-18-23: the lease is
/// the operate gate, the document only the record): the pilot's own `Active`
/// enrolment with no live charter — the lease disputed, or ended without the
/// observer's fold reaching the document. `{name}` and `{mark}` are the
/// enrolment's.
pub const CHARTER_NOT_IN_FORCE: &str = "Your charter of the {name} ({mark}) is not in force.";

/// **`tacsim:coop:{account}`** — the one key of one gateway account's
/// enrolment (lead ruling L5; the `tacsim:tutorial:` precedent). Never logged:
/// its tail is the account's canonical hash.
#[must_use]
pub fn enrolment_key(account: mush_core::AccountId) -> SmolStr {
    SmolStr::new(format!("{COOP_PREFIX}{}", account.canonical_hash()))
}

/// **The stake's key** — `tacsim.backer.stake/<lease contract id>` (doc 18
/// §4.1): contract-scoped, so a replay of the same enrolment is `AlreadyPosted`.
#[must_use]
pub fn stake_key(lease: ContractId) -> IdempotencyKey {
    IdempotencyKey::new(STAKE_KEY_NAMESPACE, lease.0.to_string())
}

/// **Where an enrolment stands.** `Active` and `Revoked` are written in cut IC6;
/// `BuyoutPending` and `Graduated` are cut IC13's (the observer already honours
/// `BuyoutPending`, doc 18 §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrolmentStatus {
    /// The charter is in force.
    Active,
    /// A buyout is under way (cut IC13).
    BuyoutPending,
    /// The hull was bought out (cut IC13).
    Graduated,
    /// The charter ended without a buyout (staff cancel, cut IC13's recall).
    Revoked,
}

/// **One gateway account's enrolment** (doc 18 §4.1; D-18-24, D-18-25): the
/// account ledger and the frozen terms. Bounded and whole-document; strict on
/// the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrolment {
    /// [`ENROLMENT_SCHEMA`].
    pub schema: u16,
    /// The gateway account: the key.
    pub account: mush_core::AccountId,
    /// The character who enrolled.
    pub pilot: ObjId,
    /// The programme (`coop.first_captain`).
    pub programme: CanonicalId,
    /// The port map's pack.
    pub pack: CanonicalId,
    /// The port entry (`ceres_port`).
    pub entry: CanonicalId,
    /// The chartered hull.
    pub hull: ObjId,
    /// Its registry mark (`FPC-D-0401`).
    pub mark: String,
    /// The name it was laid up under (`Slack Water`).
    pub laid_up_name: String,
    /// The name the pilot gave it (`Salt Moth`).
    pub name: String,
    /// The lease, `ContractId.0`.
    pub lease: u64,
    /// The hold vault, `VaultId.0`.
    pub vault: u64,
    /// The pilot's `tacsim.credit` account, `AccountId.0`.
    pub credit_account: u64,
    /// The stake, minor units.
    pub stake_minor: i64,
    /// The Co-op's share of trading margin, frozen at signing (D-18-24).
    pub share_permille: u32,
    /// The buyout price, frozen at signing (D-18-25), minor units.
    pub buyout_minor: i64,
    /// When it was signed (`World::now`).
    pub enrolled_at: i64,
    /// Where it stands.
    pub status: EnrolmentStatus,
    /// Sales folded so far (cut IC10).
    #[serde(default)]
    pub sales: u32,
    /// Trading margin folded so far, minor units (cut IC10).
    #[serde(default)]
    pub margin_minor: i64,
    /// The Co-op's share paid so far, minor units (cut IC10).
    #[serde(default)]
    pub shares_minor: i64,
}

impl Enrolment {
    /// The document's own invariants (the loader runs it; a failure is an
    /// unreadable document).
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != ENROLMENT_SCHEMA {
            return Err(format!("schema {} is not {ENROLMENT_SCHEMA}", self.schema));
        }
        for (field, text) in [
            ("mark", &self.mark),
            ("name", &self.name),
            ("laid_up_name", &self.laid_up_name),
        ] {
            if text.is_empty() || text.len() > MAX_ENROLMENT_TEXT_BYTES {
                return Err(format!(
                    "{field} must be 1..={MAX_ENROLMENT_TEXT_BYTES} bytes"
                ));
            }
        }
        if self.stake_minor <= 0 || self.buyout_minor <= 0 {
            return Err("stake_minor and buyout_minor must be positive".to_owned());
        }
        if self.share_permille > 1_000 {
            return Err(format!(
                "share_permille {} is over 1000",
                self.share_permille
            ));
        }
        if self.margin_minor < 0 || self.shares_minor < 0 {
            return Err("the sale counters must not be negative".to_owned());
        }
        Ok(())
    }
}

/// **One account's enrolment, as the store holds it** (lead ruling L5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnrolmentState<'a> {
    /// No key claims the account.
    Absent,
    /// The document read.
    Present(&'a Enrolment),
    /// A key claims the account but its body did not read: it bars
    /// re-enrolment and staff repair it.
    Unreadable,
}

// ---- resolution ---------------------------------------------------------------

/// **The programme's port**, resolved as the readiness report resolves it.
#[derive(Clone, Debug)]
pub struct ProgrammePort<'a> {
    /// The port map's pack (the lane).
    pub pack: CanonicalId,
    /// The port entry.
    pub entry: CanonicalId,
    /// The entry.
    pub def: &'a PortDef,
    /// Its programme block.
    pub programme: &'a ProgrammeDef,
    /// Its map.
    pub map: &'a PortMap,
}

impl ProgrammePort<'_> {
    /// The programme org's map row.
    fn trust_row(&self) -> Option<&crate::port::PortOrgRow> {
        self.map.orgs.get(&self.programme.org)
    }

    /// The Trust as a principal.
    #[must_use]
    pub fn trust(&self) -> Option<PrincipalId> {
        self.trust_row()
            .map(|row| PrincipalId::Organization(OrgId(row.org)))
    }

    /// The Trust's programme account.
    #[must_use]
    pub fn trust_account(&self) -> Option<AccountId> {
        self.trust_row().and_then(|row| row.account).map(AccountId)
    }

    /// The Trust's root scope.
    #[must_use]
    pub fn trust_scope(&self) -> Option<OrgScopeId> {
        self.trust_row().map(|row| OrgScopeId(row.root_scope))
    }
}

/// **The programme's port** (doc 18 §4.1): through the readiness report's lane
/// pack, the first port entry (pack order) with a `programme` whose map is
/// present. `None` on a world with no such port.
#[must_use]
pub fn programme_port(w: &World) -> Option<ProgrammePort<'_>> {
    let store = w.module_state::<TacsimStore>()?;
    let lane = CanonicalId::new(LANE_PACK).ok()?;
    let (_, ports) = report_ports(store, w);
    ports.into_iter().find_map(|(entry, def)| {
        let programme = def.programme.as_ref()?;
        match store.port_map(&lane, &entry) {
            PortMapState::Present(map) => Some(ProgrammePort {
                pack: lane.clone(),
                entry,
                def,
                programme,
                map,
            }),
            PortMapState::Absent | PortMapState::Unreadable => None,
        }
    })
}

/// The port's player-facing name: the IC-return table's row for its venue,
/// else the venue id.
#[must_use]
pub fn port_name(def: &PortDef) -> String {
    crate::ic_return::PORTS
        .iter()
        .find(|port| port.id == def.venue.as_str())
        .map_or_else(|| def.venue.to_string(), |port| port.name.to_owned())
}

/// Whether `obj` is live: present and not `GOING`.
fn live(w: &World, obj: ObjId) -> bool {
    w.get(obj)
        .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
}

/// An object's own name, unadorned.
pub(crate) fn name_of(w: &World, obj: ObjId) -> String {
    w.get(obj).map_or_else(String::new, |o| o.name.clone())
}

/// Whether `c` pledges `hull` as `trust`'s collateral.
fn pledges(c: &EconomicContract, trust: &PrincipalId, hull: ObjId) -> bool {
    c.consent.terms.deliverables.iter().any(|d| {
        matches!(
            d,
            Deliverable::Collateral { object, from } if *object == hull && from == trust
        )
    })
}

/// **The Trust's live lease on one hull** (`live_lease_on`'s shape,
/// `mush-campaign/src/recovery.rs`): non-terminal, issued by `trust`, pledging
/// `hull` as the Trust's collateral and creating an obligation. The free-hull
/// rule and the readiness hulls line read it (L8).
#[must_use]
pub fn trust_lease_on<'a>(
    w: &'a World,
    trust: &PrincipalId,
    hull: ObjId,
) -> Option<&'a EconomicContract> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contracts()
        .find(|c| {
            !c.state.is_terminal()
                && c.issuer == *trust
                && pledges(c, trust, hull)
                && mush_economy::is_lease_shaped(c)
        })
}

/// **The tenant of a lease**: its first object party that is not the issuer
/// (`hull_line`'s extraction, factored for doc 18 cut IC9).
#[must_use]
pub(crate) fn lease_tenant(lease: &EconomicContract) -> Option<ObjId> {
    lease.consent.parties.iter().find_map(|party| match party {
        PrincipalId::Object(obj) if *party != lease.issuer => Some(*obj),
        _ => None,
    })
}

/// **The tenant of a programme hull's live Trust lease** (doc 18 cut IC9): who the
/// carrier hooks tell, when they are not connected, that the hull passed a waypoint
/// or docked. `None` for a hull with no live Trust lease, or off the programme.
#[must_use]
pub fn charter_tenant(w: &World, hull: ObjId) -> Option<ObjId> {
    let trust = programme_port(w)?.trust()?;
    lease_tenant(trust_lease_on(w, &trust, hull)?)
}

/// **A live charter**: the lease and the hull it pledges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charter {
    /// The lease.
    pub contract: ContractId,
    /// The hull.
    pub hull: ObjId,
}

/// **The operate gate** (D-18-23; lead ruling L8): an `Active` lease from the
/// programme's Trust that names `pilot` among its parties and pledges one of
/// the programme port's hulls as the Trust's collateral — the lowest contract id
/// if several. **Never reads the enrolment document.**
#[must_use]
pub fn live_charter(w: &World, pilot: ObjId) -> Option<Charter> {
    let port = programme_port(w)?;
    let trust = port.trust()?;
    let tenant = PrincipalId::Object(pilot);
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contracts()
        .filter(|c| {
            c.state == ContractState::Active
                && c.issuer == trust
                && c.consent.parties.contains(&tenant)
        })
        .filter_map(|c| {
            port.map
                .hulls
                .iter()
                .map(|row| ObjId(row.hull))
                .find(|hull| pledges(c, &trust, *hull))
                .map(|hull| Charter {
                    contract: c.id,
                    hull,
                })
        })
        .min_by_key(|charter| charter.contract)
}

/// **Whether a hull may be chartered** (`next_loaner`'s rule made concrete,
/// lead ruling L9): live; its asset `Active` and titled to the Trust; no live
/// Trust lease on it; `Direct` on the programme port's own domain and not under
/// way; its airlock and stowage recorded; and no vault on its stowage (a revoked
/// hull's hold stands until cut IC13's recall).
#[must_use]
pub fn hull_is_free(w: &World, port: &ProgrammePort<'_>, row: &HullRow) -> bool {
    let hull = ObjId(row.hull);
    let Some(trust) = port.trust() else {
        return false;
    };
    let Some(store) = w.module_state::<TacsimStore>() else {
        return false;
    };
    let Some((interior, _)) = crate::port::hull::hull_spec(store, port.programme) else {
        return false;
    };
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let titled = economy.snapshot().asset(hull).is_some_and(|asset| {
        asset.lifecycle == mush_economy::AssetLifecycle::Active && asset.title_owner == trust
    });
    let docked = matches!(
        (mush_sim::pose_of(w, hull), port.map.domain),
        (Some((domain, _)), Some(home)) if domain.0 == home
    );
    let under_way = w
        .module_state::<mush_sim::JourneyBook>()
        .is_some_and(|book| book.under_way(hull));
    if !row.rooms.contains_key(&interior.space) {
        return false;
    }
    let Some(stowage) = row.rooms.get(&interior.store) else {
        return false;
    };
    let backed = economy
        .snapshot()
        .vault_by_backing(&CustodyRef::SpatialInterior(ObjId(*stowage)))
        .is_some();
    live(w, hull)
        && titled
        && trust_lease_on(w, &trust, hull).is_none()
        && docked
        && !under_way
        && !backed
}

/// **Every free hull**, ascending hull id; the next loaner is the first.
#[must_use]
pub fn free_hulls<'a>(w: &World, port: &ProgrammePort<'a>) -> Vec<&'a HullRow> {
    let mut rows: Vec<&'a HullRow> = port
        .map
        .hulls
        .iter()
        .filter(|row| hull_is_free(w, port, row))
        .collect();
    rows.sort_by_key(|row| row.hull);
    rows
}

/// **The enrolment gate** (lead ruling L1; doc 18 §4.7): the first line whose
/// binding is Lane, Rate, Map, Rooms, Domain, Org, Programme or Hulls — and,
/// since cut IC8 (§4.7 "IC8 adds pools seeded and listings equal to pools"; its
/// lead ruling L8), Pools, Listings or MarketFacts — and, since cut IC10 (lead
/// 2g), InventoryFacts, which a sale into a bid settles on — and whose state is not
/// Ready. Sky, Treasury and Bids are reported, not gated (a bid waits on the
/// capital sweep the Org lines already gate). A Pending Listings line — seeded
/// stock the arm has not listed yet — closes enrolment for the fire or two the
/// arm takes; the draw mirror's lag is inside Ready (`q ≤ p ≤ q + undrawn`).
///
/// # Errors
///
/// The first failing line.
pub fn enrol_gate(r: &Readiness) -> Result<(), &ReadinessLine> {
    match r.lines.iter().find(|line| {
        matches!(
            line.binding,
            Binding::Lane
                | Binding::Rate
                | Binding::Map
                | Binding::Rooms
                | Binding::Domain
                | Binding::Org
                | Binding::Programme
                | Binding::Hulls
                | Binding::Pools
                | Binding::Listings
                | Binding::MarketFacts
                | Binding::InventoryFacts
        ) && line.state != LineState::Ready
    }) {
        Some(line) => Err(line),
        None => Ok(()),
    }
}

/// The name with its runs of spaces collapsed and its ends trimmed.
fn normalize_name(raw: &str) -> String {
    raw.split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The name's shape alone (no world): 1..=[`MAX_HULL_NAME_BYTES`] bytes of
/// ASCII letters, digits, space, `'`, `-` and `.`, starting with a letter.
fn name_shape_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_HULL_NAME_BYTES
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '\'' | '-' | '.'))
}

/// **The Registry's name check**: trimmed and collapsed, the shape rule
/// ([`BAD_NAME`]), and not equal (ASCII case-insensitive) to any programme
/// hull's current name or any row's mark ([`NAME_TAKEN`]).
///
/// # Errors
///
/// [`BAD_NAME`] or [`NAME_TAKEN`].
pub fn check_hull_name(
    w: &World,
    port: &ProgrammePort<'_>,
    raw: &str,
) -> Result<String, &'static str> {
    let name = normalize_name(raw);
    if !name_shape_ok(&name) {
        return Err(BAD_NAME);
    }
    let taken = port.map.hulls.iter().any(|row| {
        row.mark.eq_ignore_ascii_case(&name)
            || name_of(w, ObjId(row.hull)).eq_ignore_ascii_case(&name)
    });
    if taken {
        return Err(NAME_TAKEN);
    }
    Ok(name)
}

// ---- the verbs ----------------------------------------------------------------

/// The credit instrument.
fn credit() -> Option<InstrumentId> {
    InstrumentId::new(CREDIT)
}

/// An amount in minor units through the economy's one render (`25000.00cr`).
fn money_minor(ctx: &Ctx<'_>, minor: i64) -> String {
    match credit() {
        Some(instrument) => mush_economy::money(ctx, &instrument, Amount::from_minor(minor)),
        None => format!("{minor}"),
    }
}

/// An amount in major units at the credit's scale.
fn money_major(ctx: &Ctx<'_>, major: u64) -> String {
    match minor_of(major, credit_scale(ctx.w)) {
        Some(minor) => money_minor(ctx, minor),
        None => format!("{major}cr"),
    }
}

/// Say one line to the actor under the `tacsim` tag.
fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", line.into());
}

/// The office room and its name, and the port's name.
fn office_of(w: &World, port: &ProgrammePort<'_>) -> (Option<ObjId>, String, String) {
    let office = room_key(&port.def.area, &port.programme.office)
        .ok()
        .and_then(|key| room_by_key(w, &key));
    let office_name = office.map_or_else(|| port.programme.office.clone(), |room| name_of(w, room));
    (office, office_name, port_name(port.def))
}

/// The actor's own enrolment, as the store holds it.
fn own_enrolment(w: &World, actor: ObjId) -> Option<Enrolment> {
    w.module_state::<TacsimStore>()?
        .enrolment_of(actor)
        .cloned()
}

/// What the Co-op's books hold for the asker, read by gateway account.
enum Books<'a> {
    /// No enrolment for the account (or the character, when the account does
    /// not resolve).
    Absent,
    /// The account's enrolment — perhaps another character's (an alt).
    Present(&'a Enrolment),
    /// The account's document is held unreadable.
    Unreadable,
}

/// **The asker's books, by gateway account** (D-18-25, L5: one enrolment per
/// account, so the document is the account's, not the character's). An
/// unresolved account falls back to `own` — the character's own enrolment —
/// as does an account with no document of its own.
fn account_books<'a>(w: &'a World, actor: ObjId, own: Option<&'a Enrolment>) -> Books<'a> {
    let state = w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor))
        .and_then(|account| {
            w.module_state::<TacsimStore>()
                .map(|store| match store.enrolment_state(account) {
                    EnrolmentState::Present(e) => Books::Present(e),
                    EnrolmentState::Unreadable => Books::Unreadable,
                    EnrolmentState::Absent => Books::Absent,
                })
        });
    match (state, own) {
        (Some(Books::Present(e)), _) => Books::Present(e),
        (Some(Books::Unreadable), _) => Books::Unreadable,
        (Some(Books::Absent) | None, Some(e)) => Books::Present(e),
        (Some(Books::Absent) | None, None) => Books::Absent,
    }
}

/// The row of `hull` in the port's map.
fn row_of<'a>(port: &ProgrammePort<'a>, hull: ObjId) -> Option<&'a HullRow> {
    port.map.hulls.iter().find(|row| row.hull == hull.0)
}

/// **`+coop`, `+coop/enrol`, `+coop/statement`** (doc 18 §4.1).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn coop_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let enabled = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if !enabled {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    if programme_port(ctx.w).is_none() {
        return Err(CmdError::msg(NOT_OPEN_HERE));
    }
    if ctx.switches.has("enrol") {
        enrol(ctx)
    } else if ctx.switches.has("statement") {
        statement(ctx)
    } else {
        terms(ctx)
    }
}

/// `+coop`: the registrar's line, the terms, the free hulls and the charter
/// line, the live lease first (D-18-23; L21).
fn terms(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let (lines, warn_lease, books_dark) = {
        let Some(port) = programme_port(ctx.w) else {
            return Err(CmdError::msg(NOT_OPEN_HERE));
        };
        let programme = port.programme;
        let stake = money_major(ctx, programme.stake_major);
        let buyout = money_major(ctx, programme.buyout_major);
        let new_dray = money_major(ctx, NEW_DRAY_MAJOR);
        let free = free_hulls(ctx.w, &port).len();
        let mut lines = vec![
            format!("The registrar says, \"{REGISTRAR_LINE}\""),
            format!(
                "Terms: a {stake} stake; {}‰ of trading margin over the Co-op's reference \
                 prices; buyout {buyout}, fixed at signing (a new Dray from the Yard is \
                 {new_dray}); one enrolment per pilot, ever.",
                programme.share_permille
            ),
            format!("Drays free: {free} of {}.", programme.pool),
        ];
        let enrolment = own_enrolment(ctx.w, actor);
        let mut warn_lease = None;
        let mut books_dark = false;
        let charter_line = match live_charter(ctx.w, actor) {
            Some(charter) => match enrolment.as_ref().filter(|e| e.lease == charter.contract.0) {
                Some(e) => format!(
                    "Your charter: the {} ({}); buyout {}. +coop/statement reads your lay.",
                    e.name,
                    e.mark,
                    money_minor(ctx, e.buyout_minor)
                ),
                None => {
                    warn_lease = Some(charter.contract.0);
                    let mark =
                        row_of(&port, charter.hull).map_or_else(String::new, |r| r.mark.clone());
                    CHARTER_BOOKS_DARK
                        .replace("{name}", &name_of(ctx.w, charter.hull))
                        .replace("{mark}", &mark)
                }
            },
            // No live charter: the books are the gateway account's (L5), so an
            // alt on an enrolled account, or a pilot whose lease is disputed, is
            // never invited to an enrolment the desk would refuse.
            None => match account_books(ctx.w, actor, enrolment.as_ref()) {
                Books::Present(e) if e.pilot != actor => ENROLLED_ONCE.to_owned(),
                Books::Present(e) if e.status == EnrolmentStatus::Revoked => {
                    format!("Your charter of the {} ({}) has ended.", e.name, e.mark)
                }
                Books::Present(e) => CHARTER_NOT_IN_FORCE
                    .replace("{name}", &e.name)
                    .replace("{mark}", &e.mark),
                Books::Unreadable => {
                    books_dark = true;
                    BOOKS_UNREADABLE.to_owned()
                }
                Books::Absent => {
                    let (_, office, port_name) = office_of(ctx.w, &port);
                    format!(
                        "Enrol at the {office} in {port_name}: +coop/enrol <a name for your ship>."
                    )
                }
            },
        };
        lines.push(charter_line);
        (lines, warn_lease, books_dark)
    };
    if let Some(lease) = warn_lease {
        tracing::warn!(lease, "tacsim: a live charter has no readable enrolment");
    }
    if books_dark {
        tracing::error!(
            "tacsim: +coop found an unreadable enrolment for the asker's account; staff repair the \
             tacsim:coop: document"
        );
    }
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// `+coop/statement`: the frozen terms, the counters and the cash.
fn statement(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(e) = own_enrolment(ctx.w, actor) else {
        if let Some(charter) = live_charter(ctx.w, actor) {
            tracing::warn!(
                lease = charter.contract.0,
                "tacsim: +coop/statement found a live charter with no readable enrolment"
            );
            return Err(CmdError::msg(BOOKS_UNREADABLE));
        }
        return Err(CmdError::msg(NO_CHARTER));
    };
    let stake = money_minor(ctx, e.stake_minor);
    let buyout = money_minor(ctx, e.buyout_minor);
    let margin = money_minor(ctx, e.margin_minor);
    let shares = money_minor(ctx, e.shares_minor);
    let cash = match crate::port::credit::own_account(ctx.w, actor).and_then(Account::available) {
        Some(available) => format!(
            "  Cash: {} available; the buyout is {buyout}.",
            money_minor(ctx, available.minor())
        ),
        None => format!("  Cash: no account is open; the buyout is {buyout}."),
    };
    // The lease is the authority on "in force" (D-18-23, L8), never the
    // document: a disputed lease, or one whose end the observer has not folded,
    // is not in force whatever the enrolment's status says.
    let in_force = live_charter(ctx.w, actor).is_some_and(|c| c.contract.0 == e.lease);
    let charter = if in_force {
        "Charter: in force."
    } else if e.status == EnrolmentStatus::Revoked {
        "Charter: ended."
    } else {
        "Charter: not in force."
    };
    let lines = [
        format!(
            "Statement: the {} ({}), First Captain programme.",
            e.name, e.mark
        ),
        format!(
            "  Terms: stake {stake}; {}‰ of trading margin; buyout {buyout}.",
            e.share_permille
        ),
        format!(
            "  Sales: {}; margin {margin}; Co-op share paid {shares}.",
            e.sales
        ),
        cash,
        format!("  {charter}"),
    ];
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// Everything `+coop/enrol` resolved before its first write (step 9).
struct EnrolPlan {
    account: mush_core::AccountId,
    trust: PrincipalId,
    trust_scope: OrgScopeId,
    trust_account: AccountId,
    instrument: InstrumentId,
    hull: ObjId,
    hull_owner: ObjId,
    mark: String,
    laid_up_name: String,
    stowage: ObjId,
    gallery_name: String,
    name: String,
    stake_minor: i64,
    buyout_minor: i64,
    share_permille: u32,
    programme: CanonicalId,
    pack: CanonicalId,
    entry: CanonicalId,
}

/// Steps 1–9: every check and every resolution, read-only. `Err` carries the
/// refusal text.
#[allow(clippy::too_many_lines)]
fn plan_enrolment(ctx: &Ctx<'_>) -> Result<EnrolPlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let Some(port) = programme_port(w) else {
        return Err(NOT_OPEN_HERE.to_owned());
    };
    // 1. The name is given.
    let raw = ctx.args.raw().trim();
    if raw.is_empty() {
        return Err(NAME_USAGE.to_owned());
    }
    // 2. At the office.
    let (office, office_name, port_name) = office_of(w, &port);
    let here = w.get(actor).and_then(|o| o.location);
    if office.is_none() || here != office {
        return Err(format!(
            "Enrolment is signed at the {office_name} in {port_name}."
        ));
    }
    // 3. The gateway account (fail closed).
    let Some(account) = w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor))
    else {
        return Err(NO_ACCOUNT_MATCH.to_owned());
    };
    // 4. One enrolment ever (D-18-25, L5); the live lease is a second guard.
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Err(NOT_ENABLED.to_owned());
    };
    match store.enrolment_state(account) {
        EnrolmentState::Present(_) => return Err(ENROLLED_ONCE.to_owned()),
        EnrolmentState::Unreadable => {
            tracing::error!(
                "tacsim: +coop/enrol found an unreadable enrolment for the enrolling account; \
                 staff repair the tacsim:coop: document"
            );
            return Err(BOOKS_UNREADABLE.to_owned());
        }
        EnrolmentState::Absent => {}
    }
    if live_charter(w, actor).is_some() {
        return Err(ENROLLED_ONCE.to_owned());
    }
    // 5. The readiness gate (L1).
    let report = readiness_in(ctx);
    if let Err(line) = enrol_gate(&report) {
        tracing::info!(line = %line.text, "tacsim: +coop/enrol is closed by the readiness gate");
        return Err(NOT_OPEN.to_owned());
    }
    // 6. The name.
    let name = check_hull_name(w, &port, raw).map_err(str::to_owned)?;
    // 7. The hull.
    let Some(row) = free_hulls(w, &port).first().copied() else {
        return Err(NO_HULL.to_owned());
    };
    // 8. The capital (`pool_can_fund`'s shape).
    let scale = credit_scale(w);
    let programme = port.programme;
    let (Some(stake_minor), Some(floor_minor), Some(buyout_minor)) = (
        minor_of(programme.stake_major, scale),
        minor_of(programme.floor_major, scale),
        minor_of(programme.buyout_major, scale),
    ) else {
        tracing::error!("tacsim: +coop/enrol: the programme's amounts overflow the credit scale");
        return Err(FAILED.to_owned());
    };
    let Some(economy) = w.module_state::<EconomyStore>() else {
        tracing::error!("tacsim: +coop/enrol: the economy is not composed");
        return Err(FAILED.to_owned());
    };
    let Some(trust_account) = port.trust_account() else {
        tracing::error!("tacsim: +coop/enrol: the Trust has no programme account");
        return Err(FAILED.to_owned());
    };
    let funded = economy
        .snapshot()
        .account(trust_account)
        .and_then(Account::available)
        .and_then(|available| available.minor().checked_sub(stake_minor))
        .is_some_and(|left| left >= floor_minor);
    if !funded {
        return Err(NO_CAPITAL.to_owned());
    }
    // 9. Resolve, still read-only.
    let failed = |what: &str| {
        tracing::error!(what, "tacsim: +coop/enrol could not resolve a step");
        FAILED.to_owned()
    };
    // A frozen economy refuses the offer (`offer_lease`'s first check), but
    // step 10a would already have opened the pilot's account by then — and the
    // command's refusal discards that write while RAM keeps the account. So the
    // freeze is read here, before the first write, and nothing is written.
    if economy.is_frozen() {
        tracing::warn!("tacsim: +coop/enrol refused: the economy is frozen");
        return Err(FAILED.to_owned());
    }
    let trust = port.trust().ok_or_else(|| failed("the Trust"))?;
    let trust_scope = port
        .trust_scope()
        .ok_or_else(|| failed("the Trust's scope"))?;
    let instrument = credit().ok_or_else(|| failed("the credit instrument"))?;
    let (interior, _) = crate::port::hull::hull_spec(store, programme)
        .ok_or_else(|| failed("the hull template"))?;
    crate::port::hull::berths_slot(programme, interior).map_err(|_| failed("the berths slot"))?;
    row.rooms
        .get(&interior.space)
        .ok_or_else(|| failed("the airlock"))?;
    let stowage = row
        .rooms
        .get(&interior.store)
        .map(|obj| ObjId(*obj))
        .ok_or_else(|| failed("the stowage"))?;
    let backing = CustodyRef::SpatialInterior(stowage);
    if mush_sim::InteriorCustodyProvider.backing_valid(w, &backing) != Some(true) {
        return Err(failed("the stowage backing"));
    }
    let hull = ObjId(row.hull);
    let hull_owner = w
        .get(hull)
        .map(|o| o.owner)
        .ok_or_else(|| failed("the hull"))?;
    let gallery_name = port
        .map
        .presentation
        .map(|g| name_of(w, ObjId(g)))
        .ok_or_else(|| failed("the gallery"))?;
    let plan = EnrolPlan {
        account,
        trust,
        trust_scope,
        trust_account,
        instrument,
        hull,
        hull_owner,
        mark: row.mark.clone(),
        // Doc 18 cut IC8 (lead 2d; lead ruling L18): a historical label, not an
        // identity, bounded here so a staff rename past the document's bound can
        // never strand the lowest free hull.
        laid_up_name: clamp_label(name_of(w, hull)),
        stowage,
        gallery_name,
        name,
        stake_minor,
        buyout_minor,
        share_permille: programme.share_permille,
        programme: programme.id.clone(),
        pack: port.pack.clone(),
        entry: port.entry.clone(),
    };
    // Doc 18 cut IC8 (lead 2d): the document's own invariants, checked on every
    // field the plan already knows, before the first write.
    if let Err(error) = enrolment_of(&plan, actor, 0, 0, 0, w.now()).check() {
        tracing::error!(%error, "tacsim: +coop/enrol: the enrolment would not check");
        return Err(FAILED.to_owned());
    }
    Ok(plan)
}

/// A label cut to [`MAX_ENROLMENT_TEXT_BYTES`] on a char boundary (doc 18 cut IC8).
fn clamp_label(mut text: String) -> String {
    if text.len() > MAX_ENROLMENT_TEXT_BYTES {
        let mut cut = MAX_ENROLMENT_TEXT_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        text.truncate(cut);
    }
    text
}

/// The enrolment document a plan writes, with the ids the writes mint.
fn enrolment_of(
    plan: &EnrolPlan,
    pilot: ObjId,
    lease: u64,
    vault: u64,
    credit_account: u64,
    enrolled_at: i64,
) -> Enrolment {
    Enrolment {
        schema: ENROLMENT_SCHEMA,
        account: plan.account,
        pilot,
        programme: plan.programme.clone(),
        pack: plan.pack.clone(),
        entry: plan.entry.clone(),
        hull: plan.hull,
        mark: plan.mark.clone(),
        laid_up_name: plan.laid_up_name.clone(),
        name: plan.name.clone(),
        lease,
        vault,
        credit_account,
        stake_minor: plan.stake_minor,
        share_permille: plan.share_permille,
        buyout_minor: plan.buyout_minor,
        enrolled_at,
        status: EnrolmentStatus::Active,
        sales: 0,
        margin_minor: 0,
        shares_minor: 0,
    }
}

/// **A refusal after the first write** (L6, L16). Dispatch rolls back only the
/// deferred queue when a handler returns `Err`, never RAM, so a refusal past
/// step 10a would discard the opened account's, the offered lease's and the
/// unwind's documents while RAM kept all three — an account and a cancelled
/// lease the store never received, gone at the next reboot with their ids
/// handed out again. Pinning the queue (the move `mush_economy::post` and
/// `mush_canon::commit` make for the same reason) keeps the residue L16 names
/// the same in RAM and on disk.
fn failed_after_writes(ctx: &mut Ctx<'_>) -> CmdError {
    ctx.out.pin_deferred();
    CmdError::msg(FAILED)
}

/// End a lease this enrolment offered (a refusal after the offer, L6).
fn unwind_lease(ctx: &mut Ctx<'_>, lease: ContractId, plan: &EnrolPlan) {
    let key = LeaseKey {
        lessor: plan.trust.clone(),
        premises: plan.hull,
    };
    match mush_economy::end_lease(ctx, lease, &key) {
        Ok(outcome) => {
            tracing::error!(
                lease = lease.0,
                ?outcome,
                "tacsim: +coop/enrol ended its own lease"
            );
        }
        Err(error) => {
            tracing::error!(lease = lease.0, %error, "tacsim: +coop/enrol could not end its own lease");
        }
    }
}

/// **`+coop/enrol <name>`** — ONE command, ONE batch (doc 18 §4.1; L6).
#[allow(clippy::too_many_lines)]
fn enrol(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let plan = plan_enrolment(ctx).map_err(CmdError::msg)?;
    let actor = ctx.actor;
    // 10a. The pilot's account, when absent (D-18-6).
    let existing = crate::port::credit::own_account(ctx.w, actor).map(|a| a.id);
    let (pilot_account, opened) = match existing {
        Some(id) => (id, false),
        None => {
            let Some(id) = ctx
                .w
                .module_state_mut::<EconomyStore>()
                .and_then(EconomyStore::alloc_account)
            else {
                tracing::error!("tacsim: +coop/enrol: the account allocator is spent");
                return Err(CmdError::msg(FAILED));
            };
            let name = SmolStr::new(name_of(ctx.w, actor));
            mush_economy::open_account(
                ctx,
                Account {
                    id,
                    owner: PrincipalId::Object(actor),
                    instrument: plan.instrument.clone(),
                    scope: None,
                    name,
                    status: AccountStatus::Active,
                    posted_balance: Amount::ZERO,
                    held_amount: Amount::ZERO,
                    last_hold_seq: 0,
                    credit: CreditPolicy::forbid(),
                    last_journal_seq: 0,
                    revision: 1,
                    approval: None,
                },
            );
            (id, true)
        }
    };
    // 10b. The vault id (nothing persisted until 10f).
    let Some(vault_id) = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .and_then(EconomyStore::alloc_vault)
    else {
        tracing::error!("tacsim: +coop/enrol: the vault allocator is spent");
        return Err(failed_after_writes(ctx));
    };
    // 10c. The lease (D-18-21; L3: 1 minor on the Manual clock, never due).
    let lease = match mush_economy::offer_lease(
        ctx,
        LeaseSpec {
            name: SmolStr::new(format!("First Captain charter, {}", plan.mark)),
            lessor: plan.trust.clone(),
            scope: Some(plan.trust_scope),
            tenants: vec![PrincipalId::Object(actor)],
            premises: plan.hull,
            rent: ObligationTemplate {
                debtor: PrincipalId::Object(actor),
                creditor: plan.trust.clone(),
                debit_account: pilot_account,
                credit_account: plan.trust_account,
                amount: Amount::from_minor(1),
                schedule: EconomicSchedule::new(EconomicClock::Manual, i64::MAX, None, 1),
                grace: GracePolicy::default(),
                notify_on_success: false,
            },
        },
    ) {
        Ok(lease) => lease,
        Err(error) => {
            tracing::error!(%error, "tacsim: +coop/enrol: the lease was not offered");
            return Err(failed_after_writes(ctx));
        }
    };
    // 10d. The pilot accepts, as themselves; the lease must be Active.
    let accepted = matches!(
        mush_economy::accept_contract(ctx, lease),
        Ok(mush_economy::AcceptOutcome::Accepted { .. })
    );
    let active = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().contract(lease).map(|c| c.state))
        == Some(ContractState::Active);
    if !(accepted && active) {
        tracing::error!(
            lease = lease.0,
            "tacsim: +coop/enrol: the lease did not come into force"
        );
        unwind_lease(ctx, lease, &plan);
        return Err(failed_after_writes(ctx));
    }
    // 10e. The stake — the post is the last fallible step.
    let (Some(debit), credit_amount) = (
        Amount::from_minor(plan.stake_minor).checked_neg(),
        Amount::from_minor(plan.stake_minor),
    ) else {
        unwind_lease(ctx, lease, &plan);
        return Err(failed_after_writes(ctx));
    };
    let stake_plan = EconomyPlan {
        description: format!("First Captain stake for #{}", actor.0),
        ..EconomyPlan::new(
            stake_key(lease),
            plan.trust.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static(STAKE_KEY_NAMESPACE),
                id: lease.0,
            },
            vec![
                RequestedLine {
                    account: plan.trust_account,
                    delta: debit,
                    memo: SmolStr::new_static(STAKE_KEY_NAMESPACE),
                },
                RequestedLine {
                    account: pilot_account,
                    delta: credit_amount,
                    memo: SmolStr::new_static(STAKE_KEY_NAMESPACE),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &stake_plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
        other => {
            tracing::error!(lease = lease.0, outcome = ?other, "tacsim: +coop/enrol: the stake did not post");
            unwind_lease(ctx, lease, &plan);
            return Err(failed_after_writes(ctx));
        }
    }
    // ---- past the post nothing is fallible (L6) ----
    // 10f. The hold vault on the hull's stowage (backing proved in step 9).
    mush_economy::create_vault(
        ctx,
        Vault {
            id: vault_id,
            owner: PrincipalId::Object(actor),
            scope: None,
            name: SmolStr::new(format!("{} hold", plan.mark)),
            backing: CustodyRef::SpatialInterior(plan.stowage),
            status: VaultStatus::Active,
            revision: 1,
        },
    );
    // 10g. The hull takes the pilot's name; the mark (its alias) stays. The
    // description is written under the hull's own owner, never the pilot's
    // (D-18-22).
    if let Err(error) = ctx.w.set_name(plan.hull, &plan.name) {
        tracing::error!(hull = plan.hull.0, %error, "tacsim: +coop/enrol could not rename the hull");
    }
    if ctx.w.attr(plan.hull, "desc").is_none() {
        let desc = HULL_DESC_TEMPLATE.replace("{mark}", &plan.mark);
        if let Err(error) = ctx
            .w
            .set_attr(plan.hull, "desc", Attr::text(desc, plan.hull_owner))
        {
            tracing::error!(hull = plan.hull.0, %error, "tacsim: +coop/enrol could not describe the hull");
        }
    }
    // 10h. The enrolment document, checked again on its final value (doc 18 cut
    // IC8, lead 2d): past the writes a failure is `failed_after_writes`, logged.
    let enrolment = enrolment_of(
        &plan,
        actor,
        lease.0,
        vault_id.0,
        pilot_account.0,
        ctx.w.now(),
    );
    if let Err(error) = enrolment.check() {
        tracing::error!(lease = lease.0, %error, "tacsim: +coop/enrol: the enrolment did not check");
        return Err(failed_after_writes(ctx));
    }
    put_enrolment(ctx, &enrolment);
    // 11. The reply (doc 18 §2 step 4).
    if opened {
        say(
            ctx,
            format!(
                "{ACCOUNT_OPENER} opens account #{} in your name.",
                pilot_account.0
            ),
        );
    }
    say(
        ctx,
        format!(
            "The Co-op charters you {}, laid up as the {}, now the {}, cradled at the {}.",
            plan.mark, plan.laid_up_name, plan.name, plan.gallery_name
        ),
    );
    let stake = money_minor(ctx, plan.stake_minor);
    let buyout = money_minor(ctx, plan.buyout_minor);
    say(
        ctx,
        format!("{stake} staked to you. Buyout {buyout}. {COUNTERSIGN} countersigns."),
    );
    Ok(())
}

// ---- the observer ---------------------------------------------------------------

/// **A staff-ended charter is seen** (doc 18 §4.1; lead ruling L17): on a
/// contract reaching `Settled`, `Cancelled` or `Expired`, the `Active`
/// enrolment for that lease is written back `Revoked` and its pilot told.
/// `BuyoutPending`, `Graduated` and `Revoked` enrolments and every other event:
/// nothing. Never panics, no `?`; a no-op on a roster without the TacSim layer.
#[derive(Debug, Default)]
pub struct CoopObserver;

impl EconomyObserver for CoopObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ContractChanged { id, state, .. } => match state {
                ContractStateKind::Settled
                | ContractStateKind::Cancelled
                | ContractStateKind::Expired => revoke(ctx, *id),
                ContractStateKind::Draft
                | ContractStateKind::Offered
                | ContractStateKind::Active
                | ContractStateKind::ReadyToSettle
                | ContractStateKind::Disputed => {}
            },
            EconomicEvent::TransactionPosted { .. }
            | EconomicEvent::HoldChanged { .. }
            | EconomicEvent::ObligationChanged { .. }
            | EconomicEvent::AssetTitleChanged { .. }
            | EconomicEvent::EscalationOpened { .. } => {}
        }
    }
}

/// Fold a terminal lease into its `Active` enrolment.
fn revoke(ctx: &mut Ctx<'_>, lease: ContractId) {
    let Some(enrolment) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| store.enrolment_for_lease(lease.0))
        .filter(|e| e.status == EnrolmentStatus::Active)
        .cloned()
    else {
        return;
    };
    let revoked = Enrolment {
        status: EnrolmentStatus::Revoked,
        ..enrolment
    };
    put_enrolment(ctx, &revoked);
    crate::port_tick::notify(
        ctx,
        revoked.pilot,
        &format!(
            "The Co-op has ended your charter of the {} ({}).",
            revoked.name, revoked.mark
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Enrolment {
        Enrolment {
            schema: ENROLMENT_SCHEMA,
            account: mush_core::AccountId::mint("pilot", 1).expect("an account"),
            pilot: ObjId(42),
            programme: CanonicalId::new("coop.first_captain").expect("id"),
            pack: CanonicalId::new("tacsim.pack.lane").expect("id"),
            entry: CanonicalId::new("ceres_port").expect("id"),
            hull: ObjId(500),
            mark: "FPC-D-0401".to_owned(),
            laid_up_name: "Slack Water".to_owned(),
            name: "Salt Moth".to_owned(),
            lease: 7,
            vault: 3,
            credit_account: 11,
            stake_minor: 2_500_000,
            share_permille: 250,
            buyout_minor: 9_000_000,
            enrolled_at: 1_000,
            status: EnrolmentStatus::Active,
            sales: 0,
            margin_minor: 0,
            shares_minor: 0,
        }
    }

    #[test]
    fn ic6_enrolment_wire_is_strict() {
        let e = sample();
        let value = serde_json::to_value(&e).expect("serializes");
        let back: Enrolment = serde_json::from_value(value.clone()).expect("round-trips");
        assert_eq!(back, e);
        assert_eq!(value["status"], "active");
        let mut extra = value.clone();
        extra["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Enrolment>(extra).is_err());
        // The IC10 counters default.
        let mut bare = value;
        if let Some(map) = bare.as_object_mut() {
            map.remove("sales");
            map.remove("margin_minor");
            map.remove("shares_minor");
        }
        assert_eq!(
            serde_json::from_value::<Enrolment>(bare).expect("defaults"),
            e
        );
        assert!(e.check().is_ok());
        for broken in [
            Enrolment {
                schema: 2,
                ..sample()
            },
            Enrolment {
                name: String::new(),
                ..sample()
            },
            Enrolment {
                mark: "x".repeat(121),
                ..sample()
            },
            Enrolment {
                stake_minor: 0,
                ..sample()
            },
            Enrolment {
                buyout_minor: -1,
                ..sample()
            },
            Enrolment {
                share_permille: 1_001,
                ..sample()
            },
            Enrolment {
                margin_minor: -1,
                ..sample()
            },
        ] {
            assert!(broken.check().is_err(), "{broken:?} passed its check");
        }
    }

    #[test]
    fn ic6_the_enrolment_key_is_the_gateway_account() {
        let a = mush_core::AccountId::mint("pilot", 1).expect("an account");
        let b = mush_core::AccountId::mint("other", 1).expect("an account");
        assert_eq!(
            enrolment_key(a).as_str(),
            format!("tacsim:coop:{}", a.canonical_hash())
        );
        assert_ne!(enrolment_key(a), enrolment_key(b));
        assert!(enrolment_key(a).starts_with(COOP_PREFIX));
    }

    #[test]
    fn ic6_stake_key_is_contract_scoped() {
        let one = stake_key(ContractId(7));
        let two = stake_key(ContractId(8));
        assert_ne!(one, two);
        assert_eq!(one.namespace, "tacsim.backer.stake");
        assert_eq!(one.key, "7");
    }

    #[test]
    fn ic6_hull_names_are_checked() {
        assert_eq!(normalize_name("  Salt   Moth  "), "Salt Moth");
        assert!(name_shape_ok("Salt Moth"));
        assert!(name_shape_ok("O'Hara's Run-2."));
        assert!(name_shape_ok(&"A".repeat(MAX_HULL_NAME_BYTES)));
        assert!(!name_shape_ok(&"A".repeat(MAX_HULL_NAME_BYTES + 1)));
        assert!(!name_shape_ok(""));
        assert!(!name_shape_ok("2nd Wind"));
        assert!(!name_shape_ok("-Dash"));
        assert!(!name_shape_ok("Salt/Moth"));
        assert!(!name_shape_ok("Salt_Moth"));
        assert!(!name_shape_ok("Sält Moth"));
    }

    #[test]
    fn ic6_the_gate_reads_only_its_bindings() {
        let port = CanonicalId::new("ceres_port").ok();
        let line = |binding, state| ReadinessLine {
            port: port.clone(),
            binding,
            state,
            text: format!("{binding:?} {state:?}"),
        };
        let open = Readiness {
            lines: vec![
                line(Binding::Lane, LineState::Ready),
                line(Binding::Sky, LineState::Missing),
                line(Binding::Treasury, LineState::Pending),
                line(Binding::Org, LineState::Ready),
                line(Binding::Hulls, LineState::Ready),
            ],
        };
        assert!(enrol_gate(&open).is_ok());
        for (binding, state) in [
            (Binding::Org, LineState::Pending),
            (Binding::Hulls, LineState::Missing),
            (Binding::Rate, LineState::Flagged),
        ] {
            let mut shut = open.clone();
            shut.lines.push(line(binding, state));
            let refused = enrol_gate(&shut).expect_err("the gate shuts");
            assert_eq!(refused.binding, binding);
            assert_eq!(refused.state, state);
        }
    }
}
