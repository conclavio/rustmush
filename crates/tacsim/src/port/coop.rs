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
     so far, and your cash against the buyout, anywhere.\n+coop/buyout  at the Registry & Co-op \
     desk with your Dray docked at Ceres: pay the buyout on your papers and the Registry enters \
     the hull in your name.\n+coop/release  at the desk with your Dray docked at Ceres and her \
     cargo hold empty: hand the charter back; the stake stays yours.";

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

/// `+coop`'s charter line for a graduate (doc 18 cut IC13): `{name}`, `{mark}`.
pub const GRADUATED_LINE: &str =
    "Your hull: the {name} ({mark}), graduated: bought out, and the Co-op's lay has ended.";
/// `+coop`'s charter line while a buyout is entered (doc 18 cut IC13).
pub const BUYOUT_PENDING_LINE: &str = "Your buyout of the {name} ({mark}) is being entered.";

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

/// **Why an enrolment ended** (doc 18 cut IC13, lead ruling L6): one cause carries
/// graduation, release, dormancy, revocation (with its staff attribution) and a
/// lease ended elsewhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EndCause {
    /// The buyout settled (status Graduated).
    BoughtOut,
    /// `+coop/release`.
    Released,
    /// The tick's dormancy recall.
    Dormant,
    /// `@coop/revoke`: the staff member and their words.
    Revoked {
        /// The staff member.
        by: ObjId,
        /// Their reason, 1..=[`MAX_REVOKE_WHY_BYTES`] bytes.
        why: String,
    },
    /// The lease ended outside the programme's verbs (staff `@contract/cancel`, an
    /// attestation, a re-lease that could not be made): the IC6 observer's fold.
    LeaseEnded,
}

/// The longest reason `@coop/revoke` records (doc 18 cut IC13).
pub const MAX_REVOKE_WHY_BYTES: usize = 200;

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
    /// The real second the current lease came into force: `enrolled_at` at signing;
    /// a re-lease after a refunded buyout moves it (cut IC13). The dormancy clock starts here.
    pub leased_at: i64,
    /// The buyout contract (`ContractId.0`) from `+coop/buyout` on; kept after graduation.
    pub buyout: Option<u64>,
    /// The real second the enrolment became `Graduated` or `Revoked`.
    pub ended_at: Option<i64>,
    /// Why it ended.
    pub ended: Option<EndCause>,
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
        // Cut IC13: the lease stamp, the buyout and the end.
        if self.leased_at < self.enrolled_at {
            return Err("leased_at must not be before enrolled_at".to_owned());
        }
        if self.ended_at.is_some() != self.ended.is_some() {
            return Err("ended_at and ended are set together".to_owned());
        }
        let over = matches!(
            self.status,
            EnrolmentStatus::Graduated | EnrolmentStatus::Revoked
        );
        if self.ended.is_some() != over {
            return Err("an enrolment carries an end iff it is graduated or revoked".to_owned());
        }
        match (&self.status, &self.ended) {
            (EnrolmentStatus::Graduated, Some(EndCause::BoughtOut)) => {}
            (EnrolmentStatus::Graduated, _) => {
                return Err("a graduated enrolment ended bought out".to_owned());
            }
            (_, Some(EndCause::BoughtOut)) => {
                return Err("only a graduated enrolment ended bought out".to_owned());
            }
            _ => {}
        }
        if matches!(
            self.status,
            EnrolmentStatus::Graduated | EnrolmentStatus::BuyoutPending
        ) && self.buyout.is_none()
        {
            return Err("a buyout pending or graduated names its buyout contract".to_owned());
        }
        if let Some(EndCause::Revoked { why, .. }) = &self.ended
            && (why.is_empty() || why.len() > MAX_REVOKE_WHY_BYTES)
        {
            return Err(format!(
                "a revocation's reason must be 1..={MAX_REVOKE_WHY_BYTES} bytes"
            ));
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

/// How a pilot holds a programme hull (cut IC13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// A live Trust lease: the charter.
    Charter(ContractId),
    /// The pilot holds the hull's title: a graduate.
    Title,
}

/// **Who may operate** (D-18-23, cut IC13): the hull a pilot commands and on what standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Helm {
    /// The hull.
    pub hull: ObjId,
    /// The charter or the title.
    pub standing: Standing,
}

/// Whether `hull`'s asset is `Active` and titled to `owner`.
pub(crate) fn titled_to(w: &World, hull: ObjId, owner: &PrincipalId) -> bool {
    w.module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().asset(hull))
        .is_some_and(|asset| {
            asset.lifecycle == mush_economy::AssetLifecycle::Active && asset.title_owner == *owner
        })
}

/// **The operate gate after graduation** (D-18-23 read with cut IC13's lead ruling 2b):
/// the live charter when there is one (never the document), else the lowest-id hull of
/// the programme port's map whose asset is `Active` and titled to `Object(pilot)`.
#[must_use]
pub fn helm_of(w: &World, pilot: ObjId) -> Option<Helm> {
    if let Some(charter) = live_charter(w, pilot) {
        return Some(Helm {
            hull: charter.hull,
            standing: Standing::Charter(charter.contract),
        });
    }
    let port = programme_port(w)?;
    let owner = PrincipalId::Object(pilot);
    port.map
        .hulls
        .iter()
        .map(|row| ObjId(row.hull))
        .filter(|hull| titled_to(w, *hull, &owner))
        .min()
        .map(|hull| Helm {
            hull,
            standing: Standing::Title,
        })
}

/// **Who commands a programme hull**: its live charter's tenant, else the `Object`
/// title holder of a programme-map hull. `None` for a laid-up Trust hull.
#[must_use]
pub fn helm_holder(w: &World, hull: ObjId) -> Option<ObjId> {
    if let Some(tenant) = charter_tenant(w, hull) {
        return Some(tenant);
    }
    let port = programme_port(w)?;
    if !port.map.hulls.iter().any(|row| row.hull == hull.0) {
        return None;
    }
    let asset = w.module_state::<EconomyStore>()?.snapshot().asset(hull)?;
    match &asset.title_owner {
        PrincipalId::Object(owner) if asset.lifecycle == mush_economy::AssetLifecycle::Active => {
            Some(*owner)
        }
        _ => None,
    }
}

/// **Whether a stowage's hold is still someone's** (doc 18 cut IC13; reviewer F1): a
/// vault on the stowage that is **not** an `Active` vault owned by the Trust. No vault
/// at all, or the Trust's own Active vault (a recalled hull's re-homed hold, L5), is
/// not held; a vault owned by anyone else, or a Trust vault in any other status, is.
/// [`hull_is_free`] and the readiness hulls line read this one predicate.
#[must_use]
pub(crate) fn stowage_held(economy: &EconomyStore, trust: &PrincipalId, stowage: ObjId) -> bool {
    economy
        .snapshot()
        .vault_by_backing(&CustodyRef::SpatialInterior(stowage))
        .is_some_and(|vault| !(vault.owner == *trust && vault.status == VaultStatus::Active))
}

/// **Whether a hull may be chartered** (`next_loaner`'s rule made concrete,
/// lead ruling L9): live; its asset `Active` and titled to the Trust; no live
/// Trust lease on it; `Direct` on the programme port's own domain and not under
/// way; its airlock and stowage recorded; and its stowage not [`stowage_held`] —
/// no vault, or the Trust's own Active vault, which cut IC13's recall leaves
/// behind (the next enrolment re-owns it).
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
    let backed = stowage_held(economy, &trust, ObjId(*stowage));
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

/// **Whether a readiness line's binding gates `+coop/enrol`** (doc 18 cut IC12;
/// §4.7 "IC12 lands the full report and the gate over all of it", read with
/// D-18-50). Every binding gates but the sky. The match is exhaustive and has no
/// catch-all arm on purpose: a binding a later cut adds must be placed here by
/// hand, so no new line is ever silently ungated.
#[must_use]
pub const fn gated(binding: Binding) -> bool {
    match binding {
        Binding::Lane
        | Binding::Rate
        | Binding::Treasury
        | Binding::Map
        | Binding::Rooms
        | Binding::Domain
        | Binding::Org
        | Binding::Programme
        | Binding::Hulls
        | Binding::Pools
        | Binding::Listings
        | Binding::Bids
        | Binding::MarketFacts
        | Binding::InventoryFacts => true,
        // D-18-50: the readiness gate checks the world rate and reports the sky; it
        // never gates on it.
        Binding::Sky => false,
    }
}

/// **The enrolment gate** (lead ruling L1; doc 18 §4.7 "IC12 lands the full report
/// and the gate over all of it", read with D-18-50; cut IC12 lead ruling L5): the
/// first line whose binding is [`gated`] and whose state is not Ready — every line
/// of the report but the sky, which D-18-50 keeps reported and never gated.
///
/// Since cut IC12 Treasury and Bids gate too (IC8's lead ruling L8 reported Bids
/// without gating them, a deferral to this cut): a missing treasury account, or a
/// hand-withdrawn or apply-waiting bid, closes enrolment; a lapsed bid the clerk
/// re-posts closes it for the fire or two the clerk takes, as a Pending Listings
/// line does (seeded stock the arm has not listed yet); an exhausted bid (quantity
/// 0, "not buying") reads Ready. The draw mirror's lag is inside Ready
/// (`q ≤ p ≤ q + undrawn`). [`Readiness::summary`] still reports every line, so a
/// flagged sky reads "Not ready" on `@tacsim/port status` while enrolment is open.
/// A player never sees the failing line: `+coop/enrol` logs it and answers
/// [`NOT_OPEN`].
///
/// # Errors
///
/// The first failing line.
pub fn enrol_gate(r: &Readiness) -> Result<(), &ReadinessLine> {
    match r
        .lines
        .iter()
        .find(|line| gated(line.binding) && line.state != LineState::Ready)
    {
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
pub(crate) fn money_minor(ctx: &Ctx<'_>, minor: i64) -> String {
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
pub(crate) fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", line.into());
}

/// The office room and its name, and the port's name.
pub(crate) fn office_of(w: &World, port: &ProgrammePort<'_>) -> (Option<ObjId>, String, String) {
    let office = room_key(&port.def.area, &port.programme.office)
        .ok()
        .and_then(|key| room_by_key(w, &key));
    let office_name = office.map_or_else(|| port.programme.office.clone(), |room| name_of(w, room));
    (office, office_name, port_name(port.def))
}

/// The actor's own enrolment, as the store holds it.
pub(crate) fn own_enrolment(w: &World, actor: ObjId) -> Option<Enrolment> {
    w.module_state::<TacsimStore>()?
        .enrolment_of(actor)
        .cloned()
}

/// What the Co-op's books hold for the asker, read by gateway account.
pub(crate) enum Books<'a> {
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
pub(crate) fn account_books<'a>(
    w: &'a World,
    actor: ObjId,
    own: Option<&'a Enrolment>,
) -> Books<'a> {
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
pub(crate) fn row_of<'a>(port: &ProgrammePort<'a>, hull: ObjId) -> Option<&'a HullRow> {
    port.map.hulls.iter().find(|row| row.hull == hull.0)
}

/// **`+coop`, `+coop/enrol`, `+coop/statement`** (doc 18 §4.1), and cut IC13's
/// `+coop/buyout` and `+coop/release`.
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
    } else if ctx.switches.has("buyout") {
        crate::port::buyout::buyout_cmd(ctx)
    } else if ctx.switches.has("release") {
        crate::port::recall::release_cmd(ctx)
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
                // Cut IC13: a graduate, and a buyout being entered.
                Books::Present(e) if e.status == EnrolmentStatus::Graduated => GRADUATED_LINE
                    .replace("{name}", &e.name)
                    .replace("{mark}", &e.mark),
                Books::Present(e) if e.status == EnrolmentStatus::BuyoutPending => {
                    BUYOUT_PENDING_LINE
                        .replace("{name}", &e.name)
                        .replace("{mark}", &e.mark)
                }
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
    } else if e.status == EnrolmentStatus::Graduated {
        // Cut IC13.
        "Charter: bought out; the hull is yours."
    } else if e.status == EnrolmentStatus::BuyoutPending {
        "Charter: buyout being entered."
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
    /// Cut IC13 (reviewer F1): the Trust's Active vault already on the stowage (a
    /// recalled hull's re-homed hold), proved in step 9 and re-owned after the post;
    /// `None` creates one.
    existing: Option<mush_economy::VaultId>,
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
    // Cut IC13 (reviewer F1; IC6 L6 holds): a vault already on the stowage is proved
    // here, before any write — it must be the Trust's own Active vault on exactly this
    // backing (a recalled hull's hold, L5) — and re-owned in 10f, after the post.
    let existing = match economy.snapshot().vault_by_backing(&backing) {
        None => None,
        Some(v) if v.owner == trust && v.status == VaultStatus::Active && v.backing == backing => {
            Some(v.id)
        }
        Some(v) => {
            tracing::error!(
                vault = v.id.0,
                owner = ?v.owner,
                "tacsim: +coop/enrol: the stowage holds a vault that is not the Trust's own"
            );
            return Err(failed("the stowage vault"));
        }
    };
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
        existing,
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
        leased_at: enrolled_at,
        buyout: None,
        ended_at: None,
        ended: None,
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
    // 10b. The vault id (nothing persisted until 10f): a recalled hull's re-homed
    // hold keeps its id (cut IC13); otherwise one is allocated.
    let vault_id = match plan.existing {
        Some(id) => id,
        None => {
            let Some(id) = ctx
                .w
                .module_state_mut::<EconomyStore>()
                .and_then(EconomyStore::alloc_vault)
            else {
                tracing::error!("tacsim: +coop/enrol: the vault allocator is spent");
                return Err(failed_after_writes(ctx));
            };
            id
        }
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
    // 10f. The hold vault on the hull's stowage (backing proved in step 9). Cut
    // IC13: a recalled hull's re-homed hold is re-owned to the pilot instead, with
    // the fuel and stores the last captain left in it (owner item O1).
    let hold_name = SmolStr::new(format!("{} hold", plan.mark));
    match plan.existing {
        None => mush_economy::create_vault(
            ctx,
            Vault {
                id: vault_id,
                owner: PrincipalId::Object(actor),
                scope: None,
                name: hold_name,
                backing: CustodyRef::SpatialInterior(plan.stowage),
                status: VaultStatus::Active,
                revision: 1,
            },
        ),
        Some(id) => {
            // Step 9 proved the vault exists and the owner is no System principal, so
            // this cannot answer `false`; past the post nothing is fallible (L6), so
            // a breach is logged and the enrolment stands.
            if !mush_economy::rehome_vault(ctx, id, PrincipalId::Object(actor), None, hold_name) {
                tracing::error!(
                    vault = id.0,
                    hull = plan.hull.0,
                    pilot = actor.0,
                    "tacsim: +coop/enrol: invariant breach: the re-homed hold vault was not \
                     re-owned; the enrolment stands; re-own the hold vault by hand"
                );
            }
        }
    }
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
/// enrolment for that lease is written back `Revoked` (cause `LeaseEnded`) and
/// its pilot told.
///
/// **Cut IC13 (lead ruling L7)**: a `BuyoutPending` enrolment whose lease or whose
/// buyout contract reaches a terminal state goes to
/// [`crate::port::buyout::settle_buyout`], which reads both contracts from the
/// economy (never the event alone) and graduates, re-charters, or waits.
/// `Graduated` and `Revoked` enrolments, `Disputed` and every non-terminal state:
/// nothing. Never panics, no `?`; a no-op on a roster without the TacSim layer.
#[derive(Debug, Default)]
pub struct CoopObserver;

impl EconomyObserver for CoopObserver {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        match ev {
            EconomicEvent::ContractChanged { id, state, .. } => match state {
                ContractStateKind::Settled
                | ContractStateKind::Cancelled
                | ContractStateKind::Expired => on_terminal(ctx, *id),
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

/// A contract reached a terminal state (cut IC13, item 5): the enrolment whose lease
/// it is (`Active` → the IC6 fold; `BuyoutPending` → the buyout's verdict), then the
/// enrolment whose buyout it is (`BuyoutPending` → the verdict).
fn on_terminal(ctx: &mut Ctx<'_>, id: ContractId) {
    let (by_lease, by_buyout) = match ctx.w.module_state::<TacsimStore>() {
        Some(store) => (
            store.enrolment_for_lease(id.0).cloned(),
            store.enrolment_for_buyout(id.0).cloned(),
        ),
        None => return,
    };
    if let Some(e) = by_lease {
        match e.status {
            EnrolmentStatus::Active => revoke(ctx, id),
            EnrolmentStatus::BuyoutPending => crate::port::buyout::settle_buyout(ctx, &e),
            EnrolmentStatus::Graduated | EnrolmentStatus::Revoked => {}
        }
    }
    // Re-read: the lease arm above may have moved the enrolment on.
    let by_buyout = by_buyout.and_then(|e| {
        ctx.w
            .module_state::<TacsimStore>()
            .and_then(|store| store.enrolment_for_buyout(id.0))
            .filter(|now| now.account == e.account)
            .cloned()
    });
    if let Some(e) = by_buyout.filter(|e| e.status == EnrolmentStatus::BuyoutPending) {
        crate::port::buyout::settle_buyout(ctx, &e);
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
    // Cut IC13 (item 2): the end is stamped with its cause; the tick's Return arm
    // takes the hull back into the pool once she is docked empty at Ceres (L4).
    let revoked = Enrolment {
        status: EnrolmentStatus::Revoked,
        ended_at: Some(ctx.w.now()),
        ended: Some(EndCause::LeaseEnded),
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
            leased_at: 1_000,
            buyout: None,
            ended_at: None,
            ended: None,
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

    /// Doc 18 cut IC13 item 2 (lead ruling L6): the buyout, the end stamp and its
    /// cause ride the strict schema-1 wire, and `check()` holds them together.
    #[test]
    fn ic13_the_enrolment_carries_its_buyout_and_its_end() {
        let ended = |status, cause: EndCause| Enrolment {
            status,
            buyout: Some(9),
            ended_at: Some(2_000),
            ended: Some(cause),
            ..sample()
        };
        let revoked = EndCause::Revoked {
            by: ObjId(1),
            why: "rules breach".to_owned(),
        };
        let every = [
            ended(EnrolmentStatus::Graduated, EndCause::BoughtOut),
            ended(EnrolmentStatus::Revoked, EndCause::Released),
            ended(EnrolmentStatus::Revoked, EndCause::Dormant),
            ended(EnrolmentStatus::Revoked, revoked.clone()),
            ended(EnrolmentStatus::Revoked, EndCause::LeaseEnded),
            Enrolment {
                status: EnrolmentStatus::BuyoutPending,
                buyout: Some(9),
                ..sample()
            },
            Enrolment {
                leased_at: 5_000,
                ..sample()
            },
        ];
        for e in every {
            assert!(e.check().is_ok(), "{e:?} failed its check");
            let value = serde_json::to_value(&e).expect("serializes");
            let back: Enrolment = serde_json::from_value(value).expect("round-trips");
            assert_eq!(back, e);
        }
        // The wire spelling.
        let value = serde_json::to_value(ended(EnrolmentStatus::Revoked, revoked)).expect("json");
        assert_eq!(
            value["ended"],
            serde_json::json!({"revoked": {"by": 1, "why": "rules breach"}})
        );
        assert_eq!(value["status"], "revoked");
        assert_eq!(value["buyout"], 9);
        assert_eq!(value["ended_at"], 2_000);
        let value =
            serde_json::to_value(ended(EnrolmentStatus::Revoked, EndCause::Dormant)).expect("json");
        assert_eq!(value["ended"], "dormant");
        let value = serde_json::to_value(sample()).expect("json");
        assert_eq!(value["leased_at"], 1_000);
        assert!(value["buyout"].is_null() && value["ended"].is_null());
        // Strict: an unknown field, and an unknown field inside a cause, refuse; the new
        // fields carry no default.
        let mut extra = value.clone();
        extra["graduated_at"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Enrolment>(extra).is_err());
        let mut inner = serde_json::to_value(ended(
            EnrolmentStatus::Revoked,
            EndCause::Revoked {
                by: ObjId(1),
                why: "x".to_owned(),
            },
        ))
        .expect("json");
        inner["ended"]["revoked"]["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Enrolment>(inner).is_err());
        let mut bare = value;
        if let Some(map) = bare.as_object_mut() {
            map.remove("leased_at");
        }
        assert!(serde_json::from_value::<Enrolment>(bare).is_err());
        // Each new `check()` rule refuses its broken case.
        for broken in [
            // leased_at before enrolled_at.
            Enrolment {
                leased_at: 999,
                ..sample()
            },
            // ended_at without ended, and the reverse.
            Enrolment {
                ended_at: Some(2_000),
                ..sample()
            },
            Enrolment {
                status: EnrolmentStatus::Revoked,
                ended: Some(EndCause::Dormant),
                ..sample()
            },
            // An end on a live enrolment; no end on an ended one.
            Enrolment {
                ended_at: Some(2_000),
                ended: Some(EndCause::Dormant),
                ..sample()
            },
            Enrolment {
                status: EnrolmentStatus::Revoked,
                ..sample()
            },
            // Graduated ⇒ bought out, with its buyout; BuyoutPending ⇒ its buyout.
            ended(EnrolmentStatus::Graduated, EndCause::Dormant),
            Enrolment {
                buyout: None,
                ..ended(EnrolmentStatus::Graduated, EndCause::BoughtOut)
            },
            Enrolment {
                status: EnrolmentStatus::BuyoutPending,
                ..sample()
            },
            // Bought out is a graduate's cause only.
            ended(EnrolmentStatus::Revoked, EndCause::BoughtOut),
            // A revocation's reason is 1..=200 bytes.
            ended(
                EnrolmentStatus::Revoked,
                EndCause::Revoked {
                    by: ObjId(1),
                    why: String::new(),
                },
            ),
            ended(
                EnrolmentStatus::Revoked,
                EndCause::Revoked {
                    by: ObjId(1),
                    why: "x".repeat(MAX_REVOKE_WHY_BYTES + 1),
                },
            ),
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

    /// Doc 18 cut IC12 (closes IC6's "the `enrol_gate` unit test shuts on 3 of 8
    /// bindings"): for every binding, a report whose only non-Ready line is that
    /// binding's — in each non-Ready state — shuts the gate on that line iff the
    /// binding is [`gated`]; the sky never shuts it (D-18-50).
    #[test]
    fn ic12_the_gate_refuses_on_every_gated_line_and_never_on_the_sky() {
        const EVERY: [Binding; 15] = [
            Binding::Lane,
            Binding::Sky,
            Binding::Rate,
            Binding::Treasury,
            Binding::Map,
            Binding::Rooms,
            Binding::Domain,
            Binding::Org,
            Binding::Programme,
            Binding::Hulls,
            Binding::Pools,
            Binding::Listings,
            Binding::Bids,
            Binding::MarketFacts,
            Binding::InventoryFacts,
        ];
        let port = CanonicalId::new("ceres_port").ok();
        let line = |binding, state| ReadinessLine {
            port: port.clone(),
            binding,
            state,
            text: format!("{binding:?} {state:?}"),
        };
        let open = Readiness {
            lines: EVERY
                .iter()
                .map(|binding| line(*binding, LineState::Ready))
                .collect(),
        };
        assert!(enrol_gate(&open).is_ok());
        for binding in EVERY {
            assert_eq!(gated(binding), binding != Binding::Sky, "{binding:?}");
            for state in [LineState::Pending, LineState::Missing, LineState::Flagged] {
                let mut report = open.clone();
                for row in &mut report.lines {
                    if row.binding == binding {
                        row.state = state;
                    }
                }
                if gated(binding) {
                    let refused = enrol_gate(&report).expect_err("the gate shuts");
                    assert_eq!((refused.binding, refused.state), (binding, state));
                } else {
                    assert!(
                        enrol_gate(&report).is_ok(),
                        "the sky is reported, never gated"
                    );
                    assert!(!report.is_ready(), "the summary still counts the sky");
                }
            }
        }
    }
}
