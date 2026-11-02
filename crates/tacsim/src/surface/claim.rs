//! **The salvage claim** (doc 36 cut GC.6; Q-36-24 to Q-36-28, Q-36-30; doc 27 FO-1, FO-4,
//! Q-S1–Q-S4, Q-S13–Q-S15, Q-F3, Q-R1, P-3).
//!
//! A hull loaded into a carrier that is not its own party's or a Friend's sits in the carrier
//! operator's custody with a claim owed (GC.5, [`crate::surface::salvage`]). This module is
//! that claim's life, the host of `mush_tactics`' salvage library made live and durable:
//!
//! - **The tenure pin** (Q-S3, Q-36-26). When a tug starts an attach, the ground's tenure at
//!   the casualty's cell, the casualty's title and who saw it are pinned on the site's record
//!   ([`TenurePin`], in [`crate::surface::tow::Tows::pins`]). The pin is kept across the same
//!   tug's detach and reattach, a tow a field interrupted, and a restart with the attachment
//!   intact; a `release`, or another tug's attach, starts it again. The load carries it into
//!   the custody record.
//! - **Filing** (Q-36-24, Q-36-25). `+salvage/claim <hull> go` by the custodian files an
//!   economy contract at the site's port registry (the Freeport Registry for every frontier
//!   site): `mush_economy::file_salvage_claim`, a `RealUtc` contest window of
//!   `[tacsim.recovery].claim_window_s` (48 real hours), which the world clock's outages pause
//!   through the economy's own `contract_live`. A written-off lease rig is Occator's alone and is
//!   refused at filing (Q-36-29). The custody record's seam goes from `Owed` to
//!   `Filed { contract }`, and the library's [`SalvageClaim`] is kept beside it. **Notice** goes
//!   by IC mail to the title holder and, on owned ground, to the landholder ([`ClaimMail`]); it
//!   is never a broadcast (doc 27 Q-S4).
//! - **Evidence** (Q-36-28). A party's own sensor and `known` records (the pin's witnesses: a
//!   unit of theirs in sight of the casualty, or their own `known` row placing it there, when
//!   the attach began), the durable surface black box read with bay access
//!   (`+salvage/inspect`, P-3: aboard the carrier that holds the hull), and reports delivered by
//!   another holder (`+salvage/report <hull>=<who>`). The RAM tape in `SalvageClaimBook` is
//!   retired for the ground. GC.5's `picked:unlawful` outcomes, an unlawful recovery and an
//!   unlawful scrapping are written to the site's evidence log
//!   ([`crate::surface::site_record::SiteRecord::unlawful`]) for a later crime system: evidence
//!   only, no enforcement verb (Q-36-30).
//! - **Contest and adjudication** (Q-36-27). `+salvage/contest <hull> go` files the
//!   challenger's best ground through the library's rules ([`SalvageClaim::contest`]): proven
//!   unlawful recovery with evidence they hold, else bare prior title, which never defeats a
//!   lawful claim (Q-R1). A contest that would vacate settles the claim at once; otherwise the
//!   window runs out and the claim settles by [`SalvageClaim::adjudicate`]. An uncontested claim
//!   becomes title: the registry moves it through the economy's journal
//!   (`mush_economy::award_salvage_title`), so GC.8b's rights table then reads the claimant as
//!   the title holder. A vacated claim's contract is closed and the custody record goes: the
//!   title never moved.
//! - **Remains scrapped where they lie** (GC.3's follow-up, Q-36-39). `+tv/<n> scrap [n] [go]`
//!   takes remains lying within a cell of the rig off the site's record and mints their
//!   structural mass as one `structural_alloys` lot at `SCRAP_KG_PER_T` in the same bundle, into
//!   the hold of the rig's own carrier (provisional). It is open to the hull's title holder,
//!   which a winning claimant is once the award lands; untitled remains are open to anyone, and
//!   the evidence log records it on owned ground for anyone but the landholder.
//!
//! **Where it is written.** One action writes the site's record: GC.4's `tacsim_surface_tow`,
//! with two more ops, `claim` (file, contest, report, inspect and settle) and `scrap`. The
//! commands submit as the actor; the surface tick's claim tick submits a settlement as the
//! custodian once the economy says the window has run (or a contest vacates it), after the
//! title award or the contract's close has posted.

use std::sync::Arc;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use mush_tactics::field_ops::{
    ContestGround, CustodyTitleStatus, GroundTenure, RecoveryAuthorityContext, RecoveryOpId,
    SalvageClaim, SalvageClaimId, SalvageClaimState, SalvageEvidenceKind, SalvageEvidenceRecord,
    SalvageSubject, evaluate_recovery_entitlement,
};
use serde::{Deserialize, Serialize};

use crate::surface::excursion::{cells_apart, site_cell};
use crate::surface::salvage::{ClaimSeam, SalvageCustody};
use crate::surface::site_record::{GroundKind, KnownWhat, SiteKey, SiteRecord};

/// The verb.
pub const SALVAGE_VERB: &str = "+salvage";
/// Its help.
pub const SALVAGE_HELP: &str = "+salvage  --  Salvage claims on hulls recovered into a carrier's custody. +salvage lists the claims you stand in. +salvage/claim <hull> [go] files the custodian's claim at the site's port registry; the title holder, and the landholder on owned ground, are told by IC mail. +salvage/contest <hull> [go] contests it on the best ground you hold: evidence of an unlawful recovery, else bare title (which never defeats a lawful claim). +salvage/report <hull>=<who> [go] hands your evidence to another. +salvage/inspect <hull> reads the hull's black box from aboard the carrier that holds it. Bare, each previews and commits nothing; go commits exactly the preview.";
/// The most witnesses a pin keeps.
pub const MAX_WITNESSES: usize = 16;
/// The most delivered or read evidence records one custody keeps.
pub const MAX_EVIDENCE: usize = 16;
/// The most entries the site's evidence log keeps; past it the oldest goes.
pub const MAX_UNLAWFUL: usize = 64;
/// The salvage award's registered title issuer: the lane's registry
/// (`crate::port::hull::REGISTRY_ISSUER`, already a title issuer on every world).
pub const AWARD_ISSUER: &str = crate::port::hull::REGISTRY_ISSUER;

/// `+salvage` usage.
pub const SALVAGE_USAGE: &str = "Usage: +salvage, +salvage/claim <hull> [go], +salvage/contest <hull> [go], +salvage/report <hull>=<who> [go] or +salvage/inspect <hull>.";
/// Nothing to list.
pub const NO_CLAIMS: &str = "You stand in no salvage claim.";
/// No custody record names the hull: `{hull}`.
pub const NOT_IN_CUSTODY: &str = "No salvage custody names {hull}.";
/// Only the custodian files: `{name}`.
pub const NOT_CUSTODIAN: &str = "Only the {name}'s custodian may file a claim on it.";
/// Already filed: `{name}`, `{contract}`.
pub const ALREADY_FILED: &str = "The claim on the {name} is filed already (contract #{contract}).";
/// Not filed yet: `{name}`.
pub const NOT_FILED: &str = "No claim on the {name} has been filed yet.";
/// The Occator write-off refusal: `{name}`.
pub const WRITTEN_OFF: &str =
    "The {name} is an Occator write-off: it is Occator's alone, and no claim on it is filed.";
/// No title to move: `{name}`.
pub const NO_TITLE: &str =
    "The {name} carries no title the registry can name, so no claim on it can be filed.";
/// The claim preview: `{name}`, `{registry}`, `{window}`, `{told}`.
pub const CLAIM_PREVIEW: &str = "You would file a salvage claim on the {name} at the {registry}: a contest window of {window}, the world clock's outages excluded. {told} +salvage/claim {hull} go";
/// The claim filed: `{name}`, `{registry}`, `{contract}`, `{window}`.
pub const CLAIM_FILED: &str = "You file a salvage claim on the {name} at the {registry} (contract #{contract}). It becomes your title if nobody defeats it within {window}.";
/// The notice: `{name}`, `{claimant}`, `{registry}`, `{site}`, `{window}`, `{contract}`.
pub const NOTICE_BODY: &str = "{claimant} has filed a salvage claim on the {name}, recovered at {site}, at the {registry} (contract #{contract}). It becomes {claimant}'s title unless it is defeated within {window}. +salvage/contest {hull} go contests it.";
/// The notice's subject: `{name}`.
pub const NOTICE_SUBJECT: &str = "Salvage claim: the {name}";
/// The contest preview on evidence: `{name}`, `{kind}`.
pub const CONTEST_PREVIEW_EVIDENCE: &str = "You would contest the claim on the {name} on your evidence of an unlawful recovery ({kind}). +salvage/contest {hull} go";
/// The contest preview on bare title: `{name}`.
pub const CONTEST_PREVIEW_TITLE: &str = "You would contest the claim on the {name} on your bare title. Bare title never defeats a lawful claim; only evidence of an unlawful recovery does. +salvage/contest {hull} go";
/// Nothing to contest on: `{name}`.
pub const NO_GROUND: &str =
    "You hold neither evidence of how the {name} was recovered nor its title.";
/// The contest filed: `{name}`.
pub const CONTEST_FILED: &str = "You file your contest of the claim on the {name}.";
/// The window is closed: `{name}`.
pub const WINDOW_CLOSED: &str = "The contest window on the {name} has closed.";
/// The window is paused: `{name}`.
pub const WINDOW_PAUSED: &str = "The registry's books are closed while the world clock is stopped; the {name}'s contest window is paused.";
/// No evidence to hand on: `{name}`.
pub const NO_EVIDENCE: &str = "You hold no evidence of how the {name} was recovered.";
/// The report preview: `{name}`, `{who}`.
pub const REPORT_PREVIEW: &str = "You would hand your evidence of the {name}'s recovery to {who}. +salvage/report {hull}={who} go";
/// The report delivered: `{name}`, `{who}`.
pub const REPORT_SENT: &str = "You hand your evidence of the {name}'s recovery to {who}.";
/// No such character: `{who}`.
pub const NO_SUCH_WHO: &str = "There is no one called {who}.";
/// Inspection needs bay access: `{name}`, `{carrier}`.
pub const NO_BAY_ACCESS: &str =
    "The {name}'s black box is read aboard the {carrier}, which holds it.";
/// The inspection: `{name}`, `{lines}`.
pub const INSPECTED: &str =
    "You read the {name}'s black box: {lines}. What you read is your evidence of its recovery.";
/// A field is open where the custody is kept: `{site}`.
pub const FIELD_OPEN: &str = "The registry's books for {site} wait while a field is open there.";
/// The award told: `{name}`, `{claimant}`, `{registry}`.
pub const AWARDED: &str = "The {registry} awards the {name} to {claimant}: the salvage claim stood, and the title is theirs.";
/// The vacate told: `{name}`, `{registry}`.
pub const VACATED: &str = "The {registry} vacates the salvage claim on the {name}: the recovery is proven unlawful, and the title stays where it was.";

// ---- the pin ------------------------------------------------------------------------------

/// **How a witness saw the casualty** (Q-36-28).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessKind {
    /// A unit of theirs stood in sight of the casualty when the attach began.
    Sensor,
    /// Their own `known` row placed the casualty at its cell when the attach began.
    Known,
}

/// **One witness of an attach** (Q-36-28): who holds the record, and through which unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    /// Who holds the record.
    pub holder: ObjId,
    /// The unit that saw it (the casualty itself for a `known` record).
    pub unit: ObjId,
    /// How.
    pub kind: WitnessKind,
}

/// **The tenure pinned when an attach began** (doc 27 Q-S3, doc 36 Q-36-26): the ground's
/// tenure at the casualty's cell, its title then, and who saw it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenurePin {
    /// The casualty.
    pub casualty: ObjId,
    /// The tug whose attach pinned it.
    pub tug: ObjId,
    /// The tug's operator.
    pub recoverer: ObjId,
    /// The casualty's title holder then, when it had a title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<PrincipalId>,
    /// Who answers for that title: the character who holds it, or, for an organization's
    /// title, the hull's lessee (provisional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answering: Option<ObjId>,
    /// The ground's tenure at the casualty's cell.
    pub tenure: GroundTenure,
    /// Who saw it, at most [`MAX_WITNESSES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub witnesses: Vec<Witness>,
    /// When it was pinned, IC.
    pub pinned_ic: i64,
}

impl TenurePin {
    /// The landholder, on owned ground.
    #[must_use]
    pub fn landholder(&self) -> Option<ObjId> {
        match &self.tenure {
            GroundTenure::Owned { landholder, .. } => Some(*landholder),
            GroundTenure::Unclaimed => None,
        }
    }
}

/// **Pin `pin` on a site's tows** at an attach's start (Q-36-26): an earlier pin by the same
/// tug on the same casualty is kept (a detach and reattach, an interrupted tow); any other is
/// replaced.
pub(crate) fn pin(pins: &mut Vec<TenurePin>, pin: TenurePin) {
    if pins
        .iter()
        .any(|kept| kept.casualty == pin.casualty && kept.tug == pin.tug)
    {
        return;
    }
    pins.retain(|kept| kept.casualty != pin.casualty);
    let at = pins.partition_point(|kept| kept.casualty < pin.casualty);
    pins.insert(at, pin);
    while pins.len() > crate::surface::tow::MAX_TOW_ACTS {
        let oldest = pins
            .iter()
            .enumerate()
            .min_by_key(|(_, kept)| (kept.pinned_ic, kept.casualty))
            .map(|(at, _)| at);
        match oldest {
            Some(at) => {
                pins.remove(at);
            }
            None => break,
        }
    }
}

/// **The title of `hull` now**, and who answers for it.
fn title_of(w: &World, hull: ObjId) -> (Option<PrincipalId>, Option<ObjId>) {
    let title = w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().asset(hull).map(|a| a.title_owner.clone()));
    let answering = match &title {
        Some(PrincipalId::Object(holder)) => Some(*holder),
        Some(_) => w
            .module_state::<crate::store::TacsimStore>()
            .and_then(|store| store.tv_lease_on(hull))
            .map(|lease| lease.pilot),
        None => None,
    };
    (title, answering)
}

/// **The ground's tenure at `cell` on `site`** (FO-4, Q-S1): the live claim on its block, when a
/// character holds one. An organization's seed reads as open ground (provisional: a tenure names
/// a character).
pub(crate) fn tenure_at(ctx: &Ctx<'_>, site: &CanonicalId, cell: (i64, i64)) -> GroundTenure {
    use crate::port::site_map::SurfaceReads;
    let scheme = crate::surface::chart::site_scheme(ctx.w, site);
    let pose = mush_tactics::field_ops::FieldPose {
        site: ObjId(1),
        x_cm: cell.0.saturating_mul(1_000).saturating_add(500),
        y_cm: cell.1.saturating_mul(1_000).saturating_add(500),
    };
    let Some(letter) = crate::field_ops::block_letter_at_site_pose(pose, scheme) else {
        return GroundTenure::Unclaimed;
    };
    let now = ctx.w.world_ic_now().at;
    let claim = ctx
        .block_tenure(&crate::surface::claims::BlockKey {
            site: site.clone(),
            letter,
        })
        .ok()
        .flatten()
        .and_then(|record| record.claim)
        .filter(|claim| claim.live(now));
    match claim.map(|claim| claim.holder) {
        Some(PrincipalId::Object(landholder)) if landholder.0 != 0 => GroundTenure::Owned {
            landholder,
            claim_id: Some(smol_str::SmolStr::new(format!(
                "{site}/{}",
                letter.to_ascii_lowercase()
            ))),
        },
        _ => GroundTenure::Unclaimed,
    }
}

/// **Who saw `casualty` at `cell`** (Q-36-28): the principal behind every other unit on the
/// site's ground within sight of it (its party's owner, else its leader, else its pilot), and
/// every principal whose own `known` row places a casualty or a wreck at that cell. The tug,
/// the casualty and the recoverer's own units are never witnesses.
fn witnesses(
    ctx: &Ctx<'_>,
    record: &SiteRecord,
    site: &CanonicalId,
    (casualty, tug): (ObjId, ObjId),
    cell: (i64, i64),
) -> Vec<Witness> {
    let mut near: Vec<ObjId> = Vec::new();
    if let Some(units) = ctx.w.module_state::<mush_tactics::UnitStore>() {
        for unit in units.units.keys().copied() {
            if unit == casualty || unit == tug {
                continue;
            }
            let Some((there, pose)) = crate::surface::tow::ground_pose(ctx.w, unit) else {
                continue;
            };
            if &there == site
                && cells_apart(cell, site_cell(pose.x_cm, pose.y_cm))
                    <= crate::surface::presence::SIGHT_CELLS
            {
                near.push(unit);
            }
        }
    }
    // A carrier on the pad is berthed, not on the ground: it sees from its ramp.
    for hull in crate::surface::recovery::world_hulls(ctx.w) {
        if hull == casualty || hull == tug || near.contains(&hull) {
            continue;
        }
        if let Some((entry, _, ramp, _, _)) = crate::surface::tick::hull_view_berth(ctx.w, hull)
            && &entry == site
            && cells_apart(cell, site_cell(ramp.x_cm, ramp.y_cm))
                <= crate::surface::presence::SIGHT_CELLS
        {
            near.push(hull);
        }
    }
    near.sort_unstable();
    let mut found: Vec<Witness> = Vec::new();
    for entrant in crate::field::parties::entrants(ctx, site, &near) {
        let holder = entrant
            .owner
            .or(entrant.leader)
            .or(entrant.pilot.as_ref().map(|(pilot, _)| *pilot));
        if let Some(holder) = holder
            && holder != ctx.actor
            && !found.iter().any(|w| w.holder == holder)
        {
            found.push(Witness {
                holder,
                unit: entrant.unit,
                kind: WitnessKind::Sensor,
            });
        }
    }
    for row in &record.known {
        let placed = row.items.iter().any(|item| {
            matches!(item.0, KnownWhat::Casualty | KnownWhat::Wreck) && item.cell() == cell
        });
        if placed && row.principal != ctx.actor && !found.iter().any(|w| w.holder == row.principal)
        {
            found.push(Witness {
                holder: row.principal,
                unit: casualty,
                kind: WitnessKind::Known,
            });
        }
    }
    found.truncate(MAX_WITNESSES);
    found
}

/// **The pin an attach of `casualty` by `tug` starts now** (the command's reading; the
/// start's prepare writes it).
pub(crate) fn pin_for(
    ctx: &Ctx<'_>,
    record: &SiteRecord,
    site: &CanonicalId,
    (tug, casualty): (ObjId, ObjId),
    cell: (i64, i64),
) -> TenurePin {
    let (title, answering) = title_of(ctx.w, casualty);
    TenurePin {
        casualty,
        tug,
        recoverer: ctx.actor,
        title,
        answering,
        tenure: tenure_at(ctx, site, cell),
        witnesses: witnesses(ctx, record, site, (casualty, tug), cell),
        pinned_ic: ctx.w.world_ic_now().at,
    }
}

// ---- the evidence log (Q-36-30) -----------------------------------------------------------

/// **What an unlawful act was** (Q-36-30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlawfulKind {
    /// GC.5's `picked:unlawful`: a crate taken off owned ground.
    Pickup,
    /// A hull recovered off owned ground with neither its title holder's nor the landholder's
    /// authority, loaded into a non-friend's bay.
    Recovery,
    /// Untitled remains scrapped off owned ground by anyone but the landholder.
    Scrap,
    /// A salvage claim vacated on proof of an unlawful recovery.
    Vacated,
}

/// **One unlawful act on a site's ground** (Q-36-30): evidence for a later crime system, never
/// an enforcement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlawfulAct {
    /// What it was.
    pub kind: UnlawfulKind,
    /// Who did it.
    pub actor: ObjId,
    /// The hull, or the object number on the site, it was done to.
    pub subject: u32,
    /// Whose ground: the landholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landholder: Option<ObjId>,
    /// When, IC.
    pub at_ic: i64,
}

/// **Write `act` to the site's evidence log**, past [`MAX_UNLAWFUL`] dropping the oldest.
pub(crate) fn log_unlawful(record: &mut SiteRecord, act: UnlawfulAct) {
    record.unlawful.push(act);
    while record.unlawful.len() > MAX_UNLAWFUL {
        record.unlawful.remove(0);
    }
}

// ---- evidence -----------------------------------------------------------------------------

/// The evidence kind's words.
fn evidence_words(kind: &SalvageEvidenceKind) -> &'static str {
    match kind {
        SalvageEvidenceKind::DirectWitness { .. } => "your own record of the ground",
        SalvageEvidenceKind::SensorLog { .. } => "your own sensor record",
        SalvageEvidenceKind::CustodyInspection { .. } => "an inspection in the bay",
        SalvageEvidenceKind::RecoveredBlackBox { .. } => "the hull's black box",
    }
}

/// **The evidence `who` holds of `custody`'s recovery** (Q-36-28): their own witness record
/// from the pin, then any delivered report or black-box reading of theirs. `None` without a
/// recovery operation to bind it to.
#[must_use]
pub fn evidence_of(custody: &SalvageCustody, who: ObjId) -> Option<SalvageEvidenceRecord> {
    let operation = custody.operation?;
    let pin = custody.pin.as_ref();
    if let Some(witness) = pin.and_then(|pin| pin.witnesses.iter().find(|w| w.holder == who)) {
        let at = pin.map_or(custody.recovered_ic, |pin| pin.pinned_ic);
        return Some(SalvageEvidenceRecord {
            operation_id: operation,
            casualty: custody.hull,
            holder: who,
            kind: match witness.kind {
                WitnessKind::Sensor => SalvageEvidenceKind::SensorLog {
                    sensor_unit: witness.unit,
                    recorded_at_ic: at,
                },
                WitnessKind::Known => SalvageEvidenceKind::DirectWitness {
                    witness: who,
                    observed_at_ic: at,
                },
            },
        });
    }
    custody
        .evidence
        .iter()
        .find(|record| record.holder == who && record.operation_id == operation)
        .cloned()
}

// ---- the claim's orders -------------------------------------------------------------------

/// **How a settled claim ends** (Q-36-27).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Settlement {
    /// The window ran out with no contest that vacates it: the title is the claimant's.
    Award,
    /// A contest proved the recovery unlawful (or the procedure defective): the claim is void.
    Vacate,
    /// The contract was cancelled outside the claim (staff): the claim goes back to owed.
    Withdrawn,
}

/// **One server-written claim order** (this module's head), carried by GC.4's tow action under
/// its `claim` op, keyed by the hull.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "claim", rename_all = "snake_case")]
pub enum ClaimOrder {
    /// `+salvage/claim <hull> go`.
    File {
        /// The site whose record holds the custody.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// The economy contract filed, `ContractId.0`.
        contract: u64,
        /// The window's effective end, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
        /// Who was told.
        told: Vec<ObjId>,
    },
    /// `+salvage/contest <hull> go`.
    Contest {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// The ground, as the command read it.
        ground: ContestGround,
        /// The window's effective end now, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
    },
    /// `+salvage/report <hull>=<who> go`.
    Report {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// Who receives it.
        to: ObjId,
    },
    /// `+salvage/inspect <hull>`.
    Inspect {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
    },
    /// The tick's settlement.
    Settle {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// How it ends.
        settlement: Settlement,
        /// The window's effective end, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
    },
}

impl ClaimOrder {
    /// The site whose record it writes.
    #[must_use]
    pub fn site(&self) -> &CanonicalId {
        match self {
            Self::File { site, .. }
            | Self::Contest { site, .. }
            | Self::Report { site, .. }
            | Self::Inspect { site, .. }
            | Self::Settle { site, .. } => site,
        }
    }

    /// The hull it is about.
    #[must_use]
    pub fn hull(&self) -> ObjId {
        match self {
            Self::File { hull, .. }
            | Self::Contest { hull, .. }
            | Self::Report { hull, .. }
            | Self::Inspect { hull, .. }
            | Self::Settle { hull, .. } => *hull,
        }
    }
}

/// The custody row over `hull` on `record`, mutably.
fn custody_mut(record: &mut SiteRecord, hull: ObjId) -> Option<&mut SalvageCustody> {
    record.custody.iter_mut().find(|row| row.hull == hull)
}

/// **The library's claim** a filing opens over `custody` (Q-36-24): the claimant the custodian,
/// the prior owner whoever answers for the title, and the entitlement evaluated from the pinned
/// tenure (Q-S1, Q-S3).
///
/// # Errors
///
/// The refusal code: no title to name, or the library refuses the ids.
fn open_claim(
    custody: &SalvageCustody,
    contract: u64,
    (now_real, window_end): (i64, i64),
) -> Result<SalvageClaim, &'static str> {
    let pin = custody.pin.as_ref();
    let prior = pin.and_then(|pin| pin.answering).ok_or("claim_no_title")?;
    let tenure = pin.map_or(GroundTenure::Unclaimed, |pin| pin.tenure.clone());
    let entitlement = evaluate_recovery_entitlement(&RecoveryAuthorityContext {
        recoverer: custody.holder,
        unit_title_holder: prior,
        has_owner_authorization: false,
        has_landholder_authorization: false,
        ground_tenure_at_attach: tenure.clone(),
    })
    .map_err(|_| "claim_no_title")?;
    let operation = custody.operation.ok_or("claim_no_operation")?;
    Ok(SalvageClaim {
        id: SalvageClaimId::new(contract).map_err(|_| "claim_no_contract")?,
        subject: SalvageSubject::Hull { unit: custody.hull },
        excluded_components: std::collections::BTreeSet::new(),
        claimant: custody.holder,
        prior_owner: prior,
        operation_id: operation,
        entitlement_at_attach: entitlement,
        ground_tenure_at_attach: tenure,
        filed_at_real_s: now_real,
        deadline_real_s: window_end,
        paused_at_real_s: None,
        total_paused_real_s: 0,
        contests: Vec::new(),
        documented_repairs_at_risk: Vec::new(),
        state: SalvageClaimState::Open,
    })
}

/// **A claim order's prepare** (the tow provider's `claim` op): every fact re-checked against
/// the record in the act's own transaction. Answers the outcome's result word.
///
/// # Errors
///
/// The refusal code.
pub(crate) fn prepare(
    record: &mut SiteRecord,
    actor: ObjId,
    order: &ClaimOrder,
    now_ic: i64,
) -> Result<String, &'static str> {
    let hull = order.hull();
    let custody = custody_mut(record, hull).ok_or("claim_no_custody")?;
    match order {
        ClaimOrder::File {
            contract,
            window_end,
            now_real,
            told,
            ..
        } => {
            if custody.holder != actor {
                return Err("claim_not_custodian");
            }
            if custody.claim != ClaimSeam::Owed {
                return Err("claim_filed");
            }
            let claim = open_claim(custody, *contract, (*now_real, *window_end))?;
            custody.claim = ClaimSeam::Filed {
                contract: *contract,
            };
            custody.file = Some(Box::new(claim));
            custody.told.clone_from(told);
            custody.told.truncate(MAX_WITNESSES);
            Ok("filed".to_owned())
        }
        ClaimOrder::Contest {
            ground,
            window_end,
            now_real,
            ..
        } => {
            if let ContestGround::UnlawfulRecovery { evidence } = ground
                && evidence_of(custody, actor).as_ref() != Some(evidence)
            {
                return Err("claim_no_evidence");
            }
            if matches!(ground, ContestGround::BarePriorTitle)
                && custody
                    .file
                    .as_ref()
                    .is_none_or(|file| file.prior_owner != actor)
            {
                return Err("claim_no_ground");
            }
            let mut file = custody.file.as_deref().cloned().ok_or("claim_not_filed")?;
            file.deadline_real_s = *window_end;
            file.contest(actor, ground.clone(), *now_real)
                .map_err(|_| "claim_contest_refused")?;
            custody.file = Some(Box::new(file));
            Ok("contested".to_owned())
        }
        ClaimOrder::Report { to, .. } => {
            let mut evidence = evidence_of(custody, actor).ok_or("claim_no_evidence")?;
            if *to == actor || to.0 == 0 {
                return Err("claim_no_recipient");
            }
            evidence.holder = *to;
            if !custody.evidence.contains(&evidence) {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Err("claim_evidence_full");
                }
                custody.evidence.push(evidence);
            }
            Ok("reported".to_owned())
        }
        ClaimOrder::Inspect { .. } => {
            let operation = custody.operation.ok_or("claim_no_operation")?;
            let read = SalvageEvidenceRecord {
                operation_id: operation,
                casualty: hull,
                holder: actor,
                kind: SalvageEvidenceKind::RecoveredBlackBox {
                    hull,
                    read_at_ic: now_ic,
                },
            };
            if !custody
                .evidence
                .iter()
                .any(|e| e.holder == actor && e.operation_id == operation)
            {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Err("claim_evidence_full");
                }
                custody.evidence.push(read);
            }
            Ok("inspected".to_owned())
        }
        ClaimOrder::Settle {
            settlement,
            window_end,
            now_real,
            ..
        } => settle(record, hull, *settlement, (*window_end, *now_real), now_ic),
    }
}

/// **A claim settled** (Q-36-27): adjudicated by the library's rules, which must agree with the
/// settlement the tick read; the custody record goes (an award's hull is the claimant's own, a
/// vacated claim's custody is void), and a vacated claim is written to the evidence log.
fn settle(
    record: &mut SiteRecord,
    hull: ObjId,
    settlement: Settlement,
    (window_end, now_real): (i64, i64),
    now_ic: i64,
) -> Result<String, &'static str> {
    let custody = custody_mut(record, hull).ok_or("claim_no_custody")?;
    if settlement == Settlement::Withdrawn {
        custody.claim = ClaimSeam::Owed;
        custody.file = None;
        return Ok("withdrawn".to_owned());
    }
    let mut file = custody.file.as_deref().cloned().ok_or("claim_not_filed")?;
    file.deadline_real_s = window_end;
    let status = file.adjudicate(now_real).map_err(|_| "claim_not_due")?;
    let awarded = matches!(file.state, SalvageClaimState::AwardedToClaimant { .. });
    if awarded != (settlement == Settlement::Award) {
        return Err("claim_settled_otherwise");
    }
    let CustodyTitleStatus::ResolvedTitle { .. } = status else {
        return Err("claim_not_due");
    };
    custody.file = Some(Box::new(file));
    let (holder, landholder) = (
        custody.holder,
        custody.pin.as_ref().and_then(TenurePin::landholder),
    );
    record.custody.retain(|row| row.hull != hull);
    if awarded {
        Ok("awarded".to_owned())
    } else {
        log_unlawful(
            record,
            UnlawfulAct {
                kind: UnlawfulKind::Vacated,
                actor: holder,
                subject: hull.0,
                landholder,
                at_ic: now_ic,
            },
        );
        Ok("vacated".to_owned())
    }
}

// ---- notice (Q-36-25) ---------------------------------------------------------------------

/// The IC mail seam a claim's notice goes through: `(ctx, from, to, subject, body)`.
pub type ClaimMailFn =
    dyn Fn(&mut Ctx<'_>, ObjId, &[ObjId], &str, &str) -> Result<(), String> + Send + Sync;

/// **The IC mail hook** (Q-36-25): installed by the server after composition
/// ([`install_claim_mail`], `mush-mail`'s notice); empty on a world without mail, where the
/// notice is the online line alone.
#[derive(Clone, Default)]
pub struct ClaimMail(Option<Arc<ClaimMailFn>>);

impl std::fmt::Debug for ClaimMail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ClaimMail").field(&self.0.is_some()).finish()
    }
}

/// **Install the claim notice's IC mail** on `w`: `false` when tacsim's slot is not composed.
pub fn install_claim_mail(w: &mut World, mail: Arc<ClaimMailFn>) -> bool {
    match w.module_state_mut::<ClaimMail>() {
        Some(slot) => {
            slot.0 = Some(mail);
            true
        }
        None => false,
    }
}

/// The site's clerk, whose name a registry's letters go under; else the actor.
fn clerk_of(w: &World, site: &CanonicalId) -> Option<ObjId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .find(|map| &map.entry == site)?
        .site
        .as_ref()
        .and_then(|row| row.clerk)
        .map(ObjId)
}

/// **Tell `to` by IC mail**, and online: the mail when the hook is installed (a refused letter
/// is logged), the line always.
fn tell(ctx: &mut Ctx<'_>, site: &CanonicalId, to: &[ObjId], subject: &str, body: &str) {
    if to.is_empty() {
        return;
    }
    let mail = ctx
        .w
        .module_state::<ClaimMail>()
        .and_then(|slot| slot.0.clone());
    if let Some(mail) = mail {
        let from = clerk_of(ctx.w, site).unwrap_or(ctx.actor);
        if let Err(error) = mail(ctx, from, to, subject, body) {
            tracing::warn!(%error, "tacsim: a salvage claim's notice was not mailed");
        }
    }
    for who in to {
        ctx.out
            .sys_to_actor(*who, "tacsim.salvage", body.to_owned());
    }
}

// ---- reads --------------------------------------------------------------------------------

/// The registry a site's claims are filed at: the lane's Registry (the Freeport Registry for
/// every frontier site).
fn registry_name(w: &World) -> String {
    crate::port::claims::registry_info(w).map_or_else(|| "Registry".to_owned(), |r| r.name)
}

/// A unit's name as the lines print it.
fn name(w: &World, unit: ObjId) -> String {
    crate::port::coop::name_of(w, unit)
}

/// A site's name.
fn site_name(w: &World, site: &CanonicalId) -> String {
    crate::surface::chart::lane_sites(w)
        .into_iter()
        .find(|s| &s.entry == site)
        .map_or_else(|| site.to_string(), |s| s.name)
}

/// A window's length in words.
fn window_words(secs: i64) -> String {
    let secs = secs.max(0);
    if secs >= 3_600 {
        format!("{} real hours", (secs + 3_599) / 3_600)
    } else {
        format!("{} real minutes", ((secs + 59) / 60).max(1))
    }
}

/// **Every custody record on every lane site's record**, with its site.
fn custody_rows(ctx: &Ctx<'_>) -> Vec<(CanonicalId, SalvageCustody)> {
    let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for site in crate::surface::chart::lane_sites(ctx.w) {
        let key = SiteKey(site.entry.clone());
        if let Ok(Some(record)) = ctx.action_state(&handles.families.sites, &key) {
            for row in &record.custody {
                rows.push((site.entry.clone(), row.clone()));
            }
        }
    }
    rows
}

/// The hull a `+salvage` argument names among the custody rows: `#n`, or a hull's name.
fn named(ctx: &Ctx<'_>, word: &str) -> Result<(CanonicalId, SalvageCustody), CmdError> {
    let wanted = word.trim().trim_start_matches('#');
    let (site, row) = custody_rows(ctx)
        .into_iter()
        .find(|(_, row)| {
            row.hull.0.to_string() == wanted
                || name(ctx.w, row.hull).eq_ignore_ascii_case(word.trim())
        })
        .ok_or_else(|| CmdError::msg(NOT_IN_CUSTODY.replace("{hull}", word.trim())))?;
    if promoted(ctx, &site) {
        return Err(CmdError::msg(
            FIELD_OPEN.replace("{site}", &site_name(ctx.w, &site)),
        ));
    }
    Ok((site, row))
}

/// Whether a field is open on `site`: every claim act waits (Q-CF-30).
fn promoted(ctx: &Ctx<'_>, site: &CanonicalId) -> bool {
    crate::surface::tow::record_or_new(ctx, site)
        .open_field()
        .is_some()
}

/// The economy's window over contract `id`: its effective end and whether it is paused.
fn window(w: &World, id: u64) -> Option<(i64, bool)> {
    mush_economy::salvage_claim_window(w, ContractId(id))
}

/// The economy contract's state.
fn contract_state(w: &World, id: u64) -> Option<ContractState> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contract(ContractId(id))
        .map(|c| c.state)
}

/// The claim window configured, real seconds.
fn window_secs(w: &World) -> i64 {
    w.module_state::<crate::store::TacsimStore>()
        .map_or(mush_economy::SALVAGE_CLAIM_WINDOW_SECS, |store| {
            store.config().recovery.claim_window_s
        })
        .clamp(1, mush_economy::MAX_SALVAGE_CLAIM_WINDOW_SECS)
}

// ---- the command --------------------------------------------------------------------------

/// Split `rest` into its words and a trailing `go`.
fn split_go(rest: &str) -> (String, bool) {
    let trimmed = rest.trim();
    match trimmed.rsplit_once(char::is_whitespace) {
        Some((head, "go")) => (head.trim().to_owned(), true),
        _ if trimmed == "go" => (String::new(), true),
        _ => (trimmed.to_owned(), false),
    }
}

/// Submit `order` as `actor` through the tow action.
fn submit(ctx: &mut Ctx<'_>, actor: ObjId, order: ClaimOrder, tag: u32) -> bool {
    let hull = order.hull();
    crate::surface::tow::submit(
        ctx,
        actor,
        hull,
        crate::surface::tow::TowOrder::Claim {
            order: Box::new(order),
        },
        tag,
    )
}

/// **`+salvage`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn salvage_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().to_owned();
    if ctx.switches.has("claim") {
        let (word, go) = split_go(&raw);
        return claim_cmd(ctx, &word, go);
    }
    if ctx.switches.has("contest") {
        let (word, go) = split_go(&raw);
        return contest_cmd(ctx, &word, go);
    }
    if ctx.switches.has("report") {
        let (rest, go) = split_go(&raw);
        let (word, who) = rest
            .split_once('=')
            .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
        return report_cmd(ctx, word.trim(), who.trim(), go);
    }
    if ctx.switches.has("inspect") {
        return inspect_cmd(ctx, raw.trim());
    }
    if !raw.trim().is_empty() {
        return Err(CmdError::msg(SALVAGE_USAGE));
    }
    list_cmd(ctx)
}

/// `+salvage`: the claims the actor stands in.
fn list_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let mut lines: Vec<String> = Vec::new();
    for (site, row) in custody_rows(ctx) {
        let pin = row.pin.as_ref();
        let mut roles: Vec<&str> = Vec::new();
        if row.holder == me {
            roles.push("custodian");
        }
        if pin.and_then(|p| p.answering) == Some(me) {
            roles.push("title holder");
        }
        if pin.and_then(TenurePin::landholder) == Some(me) {
            roles.push("landholder");
        }
        if evidence_of(&row, me).is_some() {
            roles.push("you hold evidence");
        }
        if roles.is_empty() && !row.told.contains(&me) {
            continue;
        }
        let claim = match (&row.claim, row.file.as_deref()) {
            (ClaimSeam::Owed, _) => "a claim owed".to_owned(),
            (ClaimSeam::Filed { contract }, file) => {
                let contests = file.map_or(0, |f| f.contests.len());
                let when = match window(ctx.w, *contract) {
                    Some((_, true)) => "its window paused".to_owned(),
                    Some((end, false)) => format!(
                        "its window closing in {}",
                        window_words(end.saturating_sub(ctx.w.now()))
                    ),
                    None => "its window closed".to_owned(),
                };
                format!("filed as contract #{contract}, {when}, {contests} contest(s)")
            }
        };
        lines.push(format!(
            "#{} the {} — recovered at {}, in the {}'s bay; {claim}; {}.",
            row.hull.0,
            name(ctx.w, row.hull),
            site_name(ctx.w, &site),
            name(ctx.w, row.carrier),
            roles.join(", ")
        ));
    }
    let text = if lines.is_empty() {
        NO_CLAIMS.to_owned()
    } else {
        lines.join("\n")
    };
    ctx.out.sys_to_actor(me, "tacsim.salvage", text);
    Ok(())
}

/// `+salvage/claim <hull> [go]`.
fn claim_cmd(ctx: &mut Ctx<'_>, word: &str, go: bool) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let hull_arg = format!("#{}", row.hull.0);
    if crate::port::tv::written_off(ctx.w, row.hull) {
        return Err(CmdError::msg(WRITTEN_OFF.replace("{name}", &label)));
    }
    if row.holder != ctx.actor {
        return Err(CmdError::msg(NOT_CUSTODIAN.replace("{name}", &label)));
    }
    if let ClaimSeam::Filed { contract } = row.claim {
        return Err(CmdError::msg(
            ALREADY_FILED
                .replace("{name}", &label)
                .replace("{contract}", &contract.to_string()),
        ));
    }
    let pin = row.pin.clone();
    let answering = pin.as_ref().and_then(|p| p.answering);
    if answering.is_none() || row.operation.is_none() {
        return Err(CmdError::msg(NO_TITLE.replace("{name}", &label)));
    }
    let mut told: Vec<ObjId> = answering.into_iter().collect();
    if let Some(landholder) = pin.as_ref().and_then(TenurePin::landholder)
        && !told.contains(&landholder)
    {
        told.push(landholder);
    }
    told.retain(|who| *who != ctx.actor);
    let registry = registry_name(ctx.w);
    let secs = window_secs(ctx.w);
    let window_text = window_words(secs);
    if !go {
        let told_words = if told.is_empty() {
            "Nobody else is told.".to_owned()
        } else {
            format!(
                "{} told by IC mail.",
                told.iter()
                    .map(|who| name(ctx.w, *who))
                    .collect::<Vec<_>>()
                    .join(" and ")
                    + if told.len() == 1 { " is" } else { " are" }
            )
        };
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            CLAIM_PREVIEW
                .replace("{name}", &label)
                .replace("{registry}", &registry)
                .replace("{window}", &window_text)
                .replace("{told}", &told_words)
                .replace("{hull}", &hull_arg),
        );
        return Ok(());
    }
    let remainder = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|account| account.id)
        .ok_or_else(|| CmdError::msg(crate::port::claims::CLAIMS_FAILED))?;
    let audience: Vec<PrincipalId> = told.iter().map(|who| PrincipalId::Object(*who)).collect();
    let contract = mush_economy::file_salvage_claim(
        ctx,
        mush_economy::SalvageClaimSpec {
            name: smol_str::SmolStr::new(format!("Salvage claim: the {label}")),
            claimant: ctx.actor,
            hull: row.hull,
            audience,
            window_secs: secs,
            remainder_to: remainder,
        },
    )
    .map_err(|error| {
        tracing::warn!(hull = row.hull.0, %error, "tacsim: a salvage claim was not filed");
        match error {
            mush_economy::ContractError::NoSuchAsset(_) => {
                CmdError::msg(NO_TITLE.replace("{name}", &label))
            }
            _ => CmdError::msg(crate::port::claims::CLAIMS_FAILED),
        }
    })?;
    let window_end =
        window(ctx.w, contract.0).map_or(ctx.w.now().saturating_add(secs), |(end, _)| end);
    let actor = ctx.actor;
    let order = ClaimOrder::File {
        site: site.clone(),
        hull: row.hull,
        contract: contract.0,
        window_end,
        now_real: ctx.w.now(),
        told: told.clone(),
    };
    if !submit(ctx, actor, order, 1) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.salvage",
        CLAIM_FILED
            .replace("{name}", &label)
            .replace("{registry}", &registry)
            .replace("{contract}", &contract.0.to_string())
            .replace("{window}", &window_text),
    );
    let body = NOTICE_BODY
        .replace("{claimant}", &name(ctx.w, actor))
        .replace("{name}", &label)
        .replace("{site}", &site_name(ctx.w, &site))
        .replace("{registry}", &registry)
        .replace("{contract}", &contract.0.to_string())
        .replace("{window}", &window_text)
        .replace("{hull}", &hull_arg);
    tell(
        ctx,
        &site,
        &told,
        &NOTICE_SUBJECT.replace("{name}", &label),
        &body,
    );
    Ok(())
}

/// The filed claim's contract and its open window, or the line that refuses.
fn open_window(ctx: &Ctx<'_>, row: &SalvageCustody, label: &str) -> Result<(u64, i64), CmdError> {
    let ClaimSeam::Filed { contract } = row.claim else {
        return Err(CmdError::msg(NOT_FILED.replace("{name}", label)));
    };
    match window(ctx.w, contract) {
        Some((_, true)) => Err(CmdError::msg(WINDOW_PAUSED.replace("{name}", label))),
        Some((end, false))
            if ctx.w.now() <= end
                && contract_state(ctx.w, contract).is_some_and(|s| !s.is_terminal()) =>
        {
            Ok((contract, end))
        }
        _ => Err(CmdError::msg(WINDOW_CLOSED.replace("{name}", label))),
    }
}

/// `+salvage/contest <hull> [go]`.
fn contest_cmd(ctx: &mut Ctx<'_>, word: &str, go: bool) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let hull_arg = format!("#{}", row.hull.0);
    let (_, end) = open_window(ctx, &row, &label)?;
    let me = ctx.actor;
    let (ground, preview) = match evidence_of(&row, me) {
        Some(evidence) => {
            let preview = CONTEST_PREVIEW_EVIDENCE
                .replace("{name}", &label)
                .replace("{kind}", evidence_words(&evidence.kind))
                .replace("{hull}", &hull_arg);
            (ContestGround::UnlawfulRecovery { evidence }, preview)
        }
        None if row.file.as_ref().is_some_and(|f| f.prior_owner == me) => (
            ContestGround::BarePriorTitle,
            CONTEST_PREVIEW_TITLE
                .replace("{name}", &label)
                .replace("{hull}", &hull_arg),
        ),
        None => return Err(CmdError::msg(NO_GROUND.replace("{name}", &label))),
    };
    if !go {
        ctx.out.sys_to_actor(me, "tacsim.salvage", preview);
        return Ok(());
    }
    let order = ClaimOrder::Contest {
        site,
        hull: row.hull,
        ground,
        window_end: end,
        now_real: ctx.w.now(),
    };
    if !submit(ctx, me, order, 2) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        me,
        "tacsim.salvage",
        CONTEST_FILED.replace("{name}", &label),
    );
    Ok(())
}

/// `+salvage/report <hull>=<who> [go]`.
fn report_cmd(ctx: &mut Ctx<'_>, word: &str, who: &str, go: bool) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let me = ctx.actor;
    if evidence_of(&row, me).is_none() {
        return Err(CmdError::msg(NO_EVIDENCE.replace("{name}", &label)));
    }
    let to = ctx
        .w
        .iter()
        .find(|o| o.kind == mush_core::ObjKind::Player && o.name.eq_ignore_ascii_case(who))
        .map(|o| o.id)
        .ok_or_else(|| CmdError::msg(NO_SUCH_WHO.replace("{who}", who)))?;
    let fill = |line: &str| {
        line.replace("{name}", &label)
            .replace("{who}", who)
            .replace("{hull}", &format!("#{}", row.hull.0))
    };
    if !go {
        ctx.out
            .sys_to_actor(me, "tacsim.salvage", fill(REPORT_PREVIEW));
        return Ok(());
    }
    let order = ClaimOrder::Report {
        site: site.clone(),
        hull: row.hull,
        to,
    };
    if !submit(ctx, me, order, 3) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out
        .sys_to_actor(me, "tacsim.salvage", fill(REPORT_SENT));
    let body = format!(
        "{} hands you their evidence of the {label}'s recovery: +salvage lists it.",
        name(ctx.w, me)
    );
    tell(
        ctx,
        &site,
        &[to],
        &NOTICE_SUBJECT.replace("{name}", &label),
        &body,
    );
    Ok(())
}

/// **Whether `who` stands aboard `carrier`**: physical bay access (P-3).
fn aboard(w: &World, who: ObjId, carrier: ObjId) -> bool {
    let Some(at) = w.get(who).and_then(|o| o.location) else {
        return false;
    };
    crate::port::ship::hull_view(w, carrier)
        .is_some_and(|view| at == carrier || view.row.rooms.values().any(|room| ObjId(*room) == at))
}

/// `+salvage/inspect <hull>`.
fn inspect_cmd(ctx: &mut Ctx<'_>, word: &str) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let me = ctx.actor;
    if !aboard(ctx.w, me, row.carrier) {
        return Err(CmdError::msg(
            NO_BAY_ACCESS
                .replace("{name}", &label)
                .replace("{carrier}", &name(ctx.w, row.carrier)),
        ));
    }
    let lines = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map_or(0, |store| store.black_box(row.hull).len());
    let lines_text = match lines {
        0 => "it holds no lines".to_owned(),
        1 => "one line".to_owned(),
        n => format!("{n} lines"),
    };
    let order = ClaimOrder::Inspect {
        site,
        hull: row.hull,
    };
    if !submit(ctx, me, order, 4) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        me,
        "tacsim.salvage",
        INSPECTED
            .replace("{name}", &label)
            .replace("{lines}", &lines_text),
    );
    Ok(())
}

// ---- the tick -----------------------------------------------------------------------------

/// **The claims' tick** (on the surface tick, beside the tow's): every filed claim whose window
/// has run or which a contest vacates is settled — the economy first (the title award, or the
/// contract's close), then the site's record — and the parties are told.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now_real = ctx.w.now();
    let registry = registry_name(ctx.w);
    let Some(issuer) = mush_org::SystemPrincipalId::new(AWARD_ISSUER) else {
        return;
    };
    for (site, row) in custody_rows(ctx) {
        let ClaimSeam::Filed { contract } = row.claim else {
            continue;
        };
        let Some(file) = row.file.as_deref() else {
            continue;
        };
        if !file.state.is_pending() || ctx.authoritative_pending(row.holder) || promoted(ctx, &site)
        {
            continue;
        }
        let state = contract_state(ctx.w, contract);
        let (end, paused) = window(ctx.w, contract).unwrap_or((file.deadline_real_s, false));
        let settlement = if file.has_vacating_contest() {
            Settlement::Vacate
        } else if matches!(state, Some(ContractState::Cancelled) | None) {
            Settlement::Withdrawn
        } else if matches!(state, Some(ContractState::Expired)) || (!paused && now_real >= end) {
            Settlement::Award
        } else {
            continue;
        };
        let label = name(ctx.w, row.hull);
        let ready = match settlement {
            Settlement::Award => match mush_economy::award_salvage_title(
                ctx,
                ContractId(contract),
                row.holder,
                row.hull,
                issuer.clone(),
            ) {
                Ok(_) => true,
                Err(mush_economy::EconomyError::RetryShortly) => false,
                Err(error) => {
                    tracing::warn!(hull = row.hull.0, contract, %error, "tacsim: a salvage award did not post; retried next tick");
                    false
                }
            },
            Settlement::Vacate => {
                match mush_economy::close_salvage_claim(
                    ctx,
                    ContractId(contract),
                    row.holder,
                    row.hull,
                ) {
                    Ok(done) => done,
                    Err(error) => {
                        tracing::warn!(hull = row.hull.0, contract, %error, "tacsim: a vacated salvage claim's contract did not close");
                        false
                    }
                }
            }
            Settlement::Withdrawn => true,
        };
        if !ready {
            continue;
        }
        let order = ClaimOrder::Settle {
            site: site.clone(),
            hull: row.hull,
            settlement,
            window_end: end,
            now_real,
        };
        if !submit(ctx, row.holder, order, 5) {
            continue;
        }
        let mut told: Vec<ObjId> = vec![row.holder];
        for who in row
            .told
            .iter()
            .copied()
            .chain(file.contests.iter().map(|contest| contest.challenger))
        {
            if !told.contains(&who) {
                told.push(who);
            }
        }
        let line = match settlement {
            Settlement::Award => AWARDED
                .replace("{registry}", &registry)
                .replace("{name}", &label)
                .replace("{claimant}", &name(ctx.w, row.holder)),
            Settlement::Vacate => VACATED
                .replace("{registry}", &registry)
                .replace("{name}", &label),
            Settlement::Withdrawn => continue,
        };
        tell(
            ctx,
            &site,
            &told,
            &NOTICE_SUBJECT.replace("{name}", &label),
            &line,
        );
    }
}

// ---- remains scrapped where they lie (GC.3's follow-up) -----------------------------------

/// The scrap's preview: `{n}`, `{what}`, `{t}`, `{carrier}`.
pub const SCRAP_PREVIEW: &str = "Rig {n} can break up {what} where it lies: {t} t of structural_alloys into the {carrier}'s hold. +tv/{n} scrap go";
/// The scrap done: `{n}`, `{what}`, `{t}`, `{carrier}`.
pub const SCRAPPED: &str = "Rig {n} breaks up {what} where it lies: {t} t of structural_alloys go into the {carrier}'s hold.";
/// Nothing to scrap: `{n}`.
pub const NO_REMAINS: &str = "No remains lie within a cell of rig {n}.";
/// The named object is not remains in reach: `{n}`, `{id}`.
pub const REMAINS_NOT_IN_REACH: &str = "Object {id} is not remains within a cell of rig {n}.";
/// Not the title holder: `{what}`.
pub const SCRAP_NOT_TITLED: &str = "Only the title holder of {what} may break them up; a winning claimant holds that title once the registry awards it.";
/// Too little to scrap: `{what}`.
pub const SCRAP_TOO_LITTLE: &str = "{what} hold less than a tonne of structural mass.";
/// No hold to take the alloys: `{n}`.
pub const SCRAP_NO_HOLD: &str =
    "Rig {n}'s carrier has no hold on this ground to take the alloys into.";
/// `+tv/<n> scrap` usage.
pub const SCRAP_USAGE: &str = "Usage: +tv/<n> scrap [object number] [go].";

/// **A scrap's facts**, fixed by the server for the tow provider's prepare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrapFacts {
    /// The site.
    pub site: CanonicalId,
    /// The remains' number on the site.
    pub object: u32,
    /// Their hull, as the command read it.
    pub of: Option<ObjId>,
    /// Their mass, kg, as the command read it.
    pub mass_kg: u64,
    /// The tonnes of `structural_alloys` they yield.
    pub alloys_t: u64,
    /// The hold vault the lot is laid into, `VaultId.0`.
    pub hold: u64,
    /// The lot's id, derived from the site and the object.
    pub lot: u64,
    /// Whether the scrap is lawful (FO-4).
    pub lawful: bool,
    /// The landholder, on owned ground.
    pub landholder: Option<ObjId>,
}

/// **The lot a site's remains scrap into**: a hash of the site and the object number.
///
/// # Errors
///
/// A hash that does not build.
pub fn remains_lot(site: &CanonicalId, object: u32) -> Result<mush_inventory::LotId, String> {
    let hash = mush_core::CanonicalHash::for_canonical_value(
        "tacsim.remains_lot.v1",
        1,
        &serde_json::json!({"site": site.as_str(), "object": object}),
        256,
    )
    .map_err(|e| e.to_string())?;
    let head: [u8; 8] = hash
        .as_bytes()
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "a canonical hash is shorter than a lot id".to_owned())?;
    Ok(mush_inventory::LotId(u64::from_be_bytes(head)))
}

/// **`+tv/<n> scrap [n] [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn scrap_cmd(
    ctx: &mut Ctx<'_>,
    rig: &crate::surface::act::RigAt,
    named_object: Option<u32>,
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let (site, pose) = crate::surface::tow::ground_pose(ctx.w, rig.tv)
        .ok_or_else(|| CmdError::msg(crate::surface::tow::NOT_ON_GROUND.replace("{n}", &n)))?;
    let here = site_cell(pose.x_cm, pose.y_cm);
    let record = crate::surface::tow::record_or_new(ctx, &site);
    let remains: Vec<&crate::surface::site_record::GroundObject> = record
        .objects
        .iter()
        .filter(|object| matches!(object.kind, GroundKind::Remains { .. }))
        .filter(|object| {
            cells_apart(here, object.pose.cell())
                <= crate::surface::salvage::PICKUP_REACH_CELLS
                    .saturating_add(u64::from(object.footprint / 2))
        })
        .collect();
    let found = match named_object {
        Some(id) => remains
            .iter()
            .find(|object| object.id == id)
            .copied()
            .ok_or_else(|| {
                CmdError::msg(
                    REMAINS_NOT_IN_REACH
                        .replace("{n}", &n)
                        .replace("{id}", &id.to_string()),
                )
            })?,
        None => remains
            .first()
            .copied()
            .ok_or_else(|| CmdError::msg(NO_REMAINS.replace("{n}", &n)))?,
    };
    let GroundKind::Remains { of, mass_kg, .. } = &found.kind else {
        return Err(CmdError::msg(NO_REMAINS.replace("{n}", &n)));
    };
    let what = match of {
        Some(hull) => format!("the remains of the {}", name(ctx.w, *hull)),
        None => "the remains".to_owned(),
    };
    let tenure = tenure_at(ctx, &site, found.pose.cell());
    let landholder = match &tenure {
        GroundTenure::Owned { landholder, .. } => Some(*landholder),
        GroundTenure::Unclaimed => None,
    };
    let title = of.map(|hull| title_of(ctx.w, hull).0).unwrap_or_default();
    let lawful = match &title {
        Some(PrincipalId::Object(holder)) if *holder == ctx.actor => true,
        Some(_) => return Err(CmdError::msg(SCRAP_NOT_TITLED.replace("{what}", &what))),
        None => landholder.is_none_or(|holder| holder == ctx.actor),
    };
    let alloys_t = mass_kg / crate::port::bay_work::strip::SCRAP_KG_PER_T;
    if alloys_t == 0 {
        return Err(CmdError::msg(SCRAP_TOO_LITTLE.replace("{what}", &what)));
    }
    let hold = crate::port::ship::hull_view(ctx.w, rig.hull)
        .and_then(|view| crate::port::ship::hold_vault(ctx.w, &view))
        .filter(|_| {
            crate::surface::tick::hull_view_berth(ctx.w, rig.hull)
                .is_some_and(|(there, ..)| there == site)
        })
        .ok_or_else(|| CmdError::msg(SCRAP_NO_HOLD.replace("{n}", &n)))?;
    let carrier = name(ctx.w, rig.hull);
    let fill = |line: &str| {
        line.replace("{n}", &n)
            .replace("{what}", &what)
            .replace("{t}", &alloys_t.to_string())
            .replace("{carrier}", &carrier)
    };
    if !go {
        ctx.out
            .sys_to_actor(ctx.actor, "tacsim.tow", fill(SCRAP_PREVIEW));
        return Ok(());
    }
    let lot = remains_lot(&site, found.id).map_err(CmdError::msg)?;
    let facts = ScrapFacts {
        site,
        object: found.id,
        of: *of,
        mass_kg: *mass_kg,
        alloys_t,
        hold: hold.0,
        lot: lot.0,
        lawful,
        landholder,
    };
    let actor = ctx.actor;
    if !crate::surface::tow::submit(
        ctx,
        actor,
        rig.tv,
        crate::surface::tow::TowOrder::Scrap {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(actor, "tacsim.tow", fill(SCRAPPED));
    Ok(())
}

/// The recovery operation a load closed, as the custody record keeps it.
#[must_use]
pub fn operation_of(custody: &SalvageCustody) -> Option<RecoveryOpId> {
    custody.operation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::salvage::{ClaimSeam, SalvageCustody};

    fn custody(pin: TenurePin) -> SalvageCustody {
        SalvageCustody {
            hull: ObjId(7),
            holder: ObjId(20),
            carrier: ObjId(21),
            recovered_by: ObjId(22),
            recovered_ic: 100,
            left_ic: Some(10),
            claim: ClaimSeam::Owed,
            operation: Some(RecoveryOpId::new(3).expect("an op")),
            pin: Some(pin),
            file: None,
            told: Vec::new(),
            evidence: Vec::new(),
        }
    }

    fn pin_on(tenure: GroundTenure) -> TenurePin {
        TenurePin {
            casualty: ObjId(7),
            tug: ObjId(22),
            recoverer: ObjId(20),
            title: Some(PrincipalId::Object(ObjId(30))),
            answering: Some(ObjId(30)),
            tenure,
            witnesses: vec![Witness {
                holder: ObjId(30),
                unit: ObjId(31),
                kind: WitnessKind::Sensor,
            }],
            pinned_ic: 50,
        }
    }

    /// **Q-36-26**: the same tug's reattach keeps the pin; another tug's attach replaces it.
    #[test]
    fn gc6_a_pin_survives_the_same_tugs_reattach_and_not_another_tugs() {
        let mut pins: Vec<TenurePin> = Vec::new();
        pin(&mut pins, pin_on(GroundTenure::Unclaimed));
        let mut later = pin_on(GroundTenure::Owned {
            landholder: ObjId(40),
            claim_id: None,
        });
        later.pinned_ic = 90;
        pin(&mut pins, later.clone());
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].pinned_ic, 50, "the same tug keeps its first pin");
        later.tug = ObjId(23);
        pin(&mut pins, later);
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].tug, ObjId(23), "another tug starts it again");
        assert_eq!(pins[0].landholder(), Some(ObjId(40)));
    }

    /// **Q-36-27, Q-36-28, Q-R1**: on owned ground a witness's own record vacates the claim; bare
    /// title alone never does, so an uncontested (or barely contested) claim is awarded at the
    /// deadline; and a report hands evidence on.
    #[test]
    fn gc6_evidence_vacates_an_unlawful_claim_and_bare_title_never_does() {
        let owned = GroundTenure::Owned {
            landholder: ObjId(40),
            claim_id: None,
        };
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        record.custody.push(custody(pin_on(owned)));
        let file = ClaimOrder::File {
            site: record.site.clone(),
            hull: ObjId(7),
            contract: 9,
            window_end: 1_000,
            now_real: 100,
            told: vec![ObjId(30), ObjId(40)],
        };
        assert_eq!(
            prepare(&mut record, ObjId(21), &file, 1),
            Err("claim_not_custodian")
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 1).as_deref(),
            Ok("filed")
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 1),
            Err("claim_filed")
        );
        let filed = record.custody[0].file.clone().expect("the claim");
        assert!(!filed.entitlement_at_attach.is_lawful(), "{filed:?}");
        // The landholder holds no evidence until the title holder reports to them.
        assert!(evidence_of(&record.custody[0], ObjId(40)).is_none());
        let report = ClaimOrder::Report {
            site: record.site.clone(),
            hull: ObjId(7),
            to: ObjId(40),
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &report, 2),
            Err("claim_no_evidence")
        );
        assert_eq!(
            prepare(&mut record, ObjId(30), &report, 2).as_deref(),
            Ok("reported")
        );
        let evidence = evidence_of(&record.custody[0], ObjId(40)).expect("delivered");
        let contest = ClaimOrder::Contest {
            site: record.site.clone(),
            hull: ObjId(7),
            ground: ContestGround::UnlawfulRecovery { evidence },
            window_end: 1_000,
            now_real: 200,
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &contest, 3).as_deref(),
            Ok("contested")
        );
        assert!(
            record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .has_vacating_contest()
        );
        let award = settle_on(&record, Settlement::Award, 1_200);
        assert_eq!(
            prepare(&mut record, ObjId(20), &award, 4),
            Err("claim_settled_otherwise")
        );
        let vacate = settle_on(&record, Settlement::Vacate, 1_200);
        assert_eq!(
            prepare(&mut record, ObjId(20), &vacate, 4).as_deref(),
            Ok("vacated")
        );
        assert!(record.custody.is_empty());
        assert_eq!(record.unlawful.len(), 1);
        assert_eq!(record.unlawful[0].kind, UnlawfulKind::Vacated);

        // Open ground: lawful; bare title is recorded and does not vacate; awarded at the end.
        record
            .custody
            .push(custody(pin_on(GroundTenure::Unclaimed)));
        let refile = file_on(&record, 10);
        assert_eq!(
            prepare(&mut record, ObjId(20), &refile, 5).as_deref(),
            Ok("filed")
        );
        let bare = ClaimOrder::Contest {
            site: record.site.clone(),
            hull: ObjId(7),
            ground: ContestGround::BarePriorTitle,
            window_end: 1_000,
            now_real: 300,
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &bare, 6),
            Err("claim_no_ground")
        );
        assert_eq!(
            prepare(&mut record, ObjId(30), &bare, 6).as_deref(),
            Ok("contested")
        );
        let early = settle_on(&record, Settlement::Award, 900);
        assert_eq!(
            prepare(&mut record, ObjId(20), &early, 7),
            Err("claim_not_due")
        );
        let due = settle_on(&record, Settlement::Award, 1_000);
        assert_eq!(
            prepare(&mut record, ObjId(20), &due, 7).as_deref(),
            Ok("awarded")
        );
        assert!(record.custody.is_empty());
    }

    fn file_on(record: &SiteRecord, contract: u64) -> ClaimOrder {
        ClaimOrder::File {
            site: record.site.clone(),
            hull: ObjId(7),
            contract,
            window_end: 1_000,
            now_real: 100,
            told: vec![ObjId(30)],
        }
    }

    fn settle_on(record: &SiteRecord, settlement: Settlement, now_real: i64) -> ClaimOrder {
        ClaimOrder::Settle {
            site: record.site.clone(),
            hull: ObjId(7),
            settlement,
            window_end: 1_000,
            now_real,
        }
    }
}
