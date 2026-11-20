//! **The record of proven offenses** (doc 50 unit LW.2; Q-50-37, Q-50-38, Q-50-68 as ruled, doc
//! 50a A-10b, A-10c).
//!
//! Each domain keeps its evidence where it is: a site's evidence log, the customs rulings, the
//! Registry's inspections. A **proven** offense is written through one function,
//! [`record_offense`], into one record **per offender and per polity**
//! (`tacsim:offense:{offender}:{polity}`, [`OffenseRecord`]). That key is the ruling's
//! "actor by polity", marked expensive to reverse, so it is fixed here before any later unit
//! writes to it:
//!
//! - **The offender** ([`Offender`], Q-50-68) is whoever acted: the character for their own
//!   act, the organization for an act made `as=<org>`, or the gateway account a license is kept
//!   by (the license bar binds the account).
//! - **The polity** is the one that proved it: the offense binds only where that polity holds de
//!   facto ([`binds_at`], Q-50-38). Other ports ignore it, so crime has a geography.
//! - **The kind** is a closed list ([`OffenseKind`]). A new kind is a new variant, added by the
//!   unit that proves it.
//!
//! Penalties and the player's page read this record. Port security (doc 41 cut RS.7) is its first
//! writer and reader: `crate::surface::security` proves a salvage row here, fines it at the
//! ports its polity holds, and quarantines the docking hull there.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_faction::FactionId;
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::store::TacsimStore;
use crate::surface::claim::{UnlawfulAct, UnlawfulKind};

/// The record's key prefix: `tacsim:offense:{offender}:{polity}`.
pub const OFFENSE_PREFIX: &str = "tacsim:offense:";
/// The record's schema.
pub const OFFENSE_SCHEMA: u32 = 1;
/// The most proven offenses one record keeps; past it the oldest settled one goes.
pub const MAX_OFFENSE_ROWS: usize = 64;

/// **Who answers for an offense** (Q-50-68): whoever acted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Offender {
    /// A character, for their own act.
    Character(ObjId),
    /// An organization, for an act made `as=<org>`.
    Organization(OrgId),
    /// A gateway account, which keeps a license (a license bar binds it).
    Account(mush_core::AccountId),
}

impl Offender {
    /// The key's token for this offender: `c{id}`, `o{id}` or `a{hash}`.
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            Self::Character(id) => format!("c{}", id.0),
            Self::Organization(org) => format!("o{}", org.0),
            Self::Account(account) => format!("a{}", account.canonical_hash().to_hex()),
        }
    }

    /// The economy principal whose account a fine is drawn on: `None` for an account, which
    /// holds no money of its own.
    #[must_use]
    pub fn principal(&self) -> Option<PrincipalId> {
        match self {
            Self::Character(id) => Some(PrincipalId::Object(*id)),
            Self::Organization(org) => Some(PrincipalId::Organization(*org)),
            Self::Account(_) => None,
        }
    }
}

/// **One record's identity** (Q-50-37's key): the offender and the polity that proved it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OffenseKey {
    /// Who answers.
    pub offender: Offender,
    /// The polity that proved it, and where it binds.
    pub polity: FactionId,
}

impl OffenseKey {
    /// The record's document key.
    #[must_use]
    pub fn doc_key(&self) -> SmolStr {
        SmolStr::new(format!(
            "{OFFENSE_PREFIX}{}:{}",
            self.offender.token(),
            self.polity.0
        ))
    }
}

/// **The kinds of proven offense** (Q-50-37's closed list). Each later unit that proves a new
/// kind adds its variant here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffenseKind {
    /// An unlawful salvage pickup (doc 41 cut RS.7).
    SalvagePickup,
    /// Unlawful scrapping (doc 41 cut RS.7).
    SalvageScrap,
    /// An unlawful recovery (doc 41 cut RS.7).
    SalvageRecovery,
    /// **Holding a restricted good without its paper** in a docked hold where the port's law
    /// restricts it, found by a port audit (doc 50 unit LW.3; Q-50-65, Q-50-36).
    UnlicensedPossession,
    /// **Selling a restricted good privately without its paper**, proven by the buyer's report
    /// of their receipt (doc 50 unit LW.3; Q-50-66, Q-50-67).
    RestrictedSale,
    /// **Buying a restricted good privately without its paper**, proven by the seller's report
    /// of their receipt (doc 50 unit LW.3; Q-50-66).
    RestrictedPurchase,
}

impl OffenseKind {
    /// The offense a site's logged act proves, when it is one: a `Vacated` row proves the
    /// `Recovery` it follows, and a capture is doc 42's.
    #[must_use]
    pub const fn of_unlawful(kind: UnlawfulKind) -> Option<Self> {
        match kind {
            UnlawfulKind::Pickup => Some(Self::SalvagePickup),
            UnlawfulKind::Scrap => Some(Self::SalvageScrap),
            UnlawfulKind::Recovery => Some(Self::SalvageRecovery),
            UnlawfulKind::Vacated | UnlawfulKind::Capture => None,
        }
    }

    /// What the kind is called in a line.
    #[must_use]
    pub const fn what(self) -> &'static str {
        match self {
            Self::SalvagePickup => "an unlawful pickup",
            Self::SalvageScrap => "unlawful scrapping",
            Self::SalvageRecovery => "an unlawful recovery",
            Self::UnlicensedPossession => "holding a restricted good without its papers",
            Self::RestrictedSale => "selling a restricted good without its papers",
            Self::RestrictedPurchase => "buying a restricted good without its papers",
        }
    }
}

/// **How an offense was proven** (Q-41-24).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Proof {
    /// A `recovery` contest vacated the claim: the registry adjudicated it.
    Vacated,
    /// A witness reported it to the registry with evidence of their own.
    Reported {
        /// The witness.
        by: ObjId,
    },
    /// An authority's own inspection found it: a port audit (doc 50 unit LW.3; Q-50-35).
    Audit,
}

/// **Where the evidence is.** The domains keep their own: a row names it, and copies what the
/// domain will not keep (a site's log is a ring of 64).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Evidence {
    /// A row of a site's evidence log, copied at the proof.
    Site {
        /// The site.
        site: CanonicalId,
        /// The act, as the site logged it.
        act: UnlawfulAct,
    },
    /// **A port audit's finding** (doc 50 unit LW.3): what the docked hold held.
    Audit {
        /// The hull audited.
        hull: ObjId,
        /// The port's lane entry.
        entry: CanonicalId,
        /// The restricted good.
        good: CanonicalId,
        /// How many units the hold held of it.
        units: u64,
        /// When, IC.
        at_ic: i64,
    },
    /// **A party's receipt of a private sale** (doc 50 unit LW.3; Q-50-66), as it was reported.
    Receipt {
        /// The sale's quote, `QuoteId.0`.
        quote: u64,
        /// The venue.
        venue: CanonicalId,
        /// The good.
        good: CanonicalId,
        /// How many units were sold.
        units: u64,
        /// When it settled, IC.
        at_ic: i64,
    },
}

/// **What is owed to a reporter once the fine is collected** (doc 50 unit LW.3; Q-50-43,
/// Q-50-67): the price a reporting buyer paid, and the reward, both out of the fine and never more
/// than it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payout {
    /// The reporter.
    pub to: ObjId,
    /// The price the reporting buyer paid back, minor units (0 for a reporting seller).
    pub repay_minor: i64,
    /// Whether it has been paid.
    #[serde(default)]
    pub paid: bool,
}

/// **One proven offense.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenOffense {
    /// What it was.
    pub kind: OffenseKind,
    /// The evidence.
    pub evidence: Evidence,
    /// How it was proven.
    pub proof: Proof,
    /// When it was proven, IC.
    pub proven_ic: i64,
    /// The fine's one-term debt, `ObligationId.0`, once fined: the receipt that it is never
    /// charged twice. Settled when the debt is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debt: Option<u64>,
    /// **What the reporter is owed from the collected fine** (doc 50 unit LW.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout: Option<Payout>,
}

impl ProvenOffense {
    /// The site act this offense copies, when it is one.
    #[must_use]
    pub fn site_act(&self) -> Option<(&CanonicalId, &UnlawfulAct)> {
        match &self.evidence {
            Evidence::Site { site, act } => Some((site, act)),
            Evidence::Audit { .. } | Evidence::Receipt { .. } => None,
        }
    }

    /// The units of a good the offense is about (an audit's or a sale's), for a fine by the
    /// tonne; `None` for a salvage act.
    #[must_use]
    pub fn units(&self) -> Option<u64> {
        match &self.evidence {
            Evidence::Site { .. } => None,
            Evidence::Audit { units, .. } | Evidence::Receipt { units, .. } => Some(*units),
        }
    }

    /// Whether this is the same offense as `other`: one kind on the same evidence.
    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        self.kind == other.kind
            && match (&self.evidence, &other.evidence) {
                (Evidence::Site { site: a, act: x }, Evidence::Site { site: b, act: y }) => {
                    a == b
                        && x.kind == y.kind
                        && x.actor == y.actor
                        && x.subject == y.subject
                        && x.at_ic == y.at_ic
                }
                (
                    Evidence::Audit {
                        hull: h1,
                        entry: e1,
                        good: g1,
                        at_ic: t1,
                        ..
                    },
                    Evidence::Audit {
                        hull: h2,
                        entry: e2,
                        good: g2,
                        at_ic: t2,
                        ..
                    },
                ) => h1 == h2 && e1 == e2 && g1 == g2 && t1 == t2,
                (Evidence::Receipt { quote: a, .. }, Evidence::Receipt { quote: b, .. }) => a == b,
                _ => false,
            }
    }
}

/// **`tacsim:offense:{offender}:{polity}`**: the offenses one polity proved against one
/// offender.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OffenseRecord {
    /// [`OFFENSE_SCHEMA`].
    pub schema: u32,
    /// Whose record, and which polity's.
    pub key: OffenseKey,
    /// The proven offenses, oldest first, at most [`MAX_OFFENSE_ROWS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<ProvenOffense>,
}

impl OffenseRecord {
    /// An empty record for `key`.
    #[must_use]
    pub fn new(key: OffenseKey) -> Self {
        Self {
            schema: OFFENSE_SCHEMA,
            key,
            rows: Vec::new(),
        }
    }

    /// The record's own bounds.
    ///
    /// # Errors
    ///
    /// What is out of bounds.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != OFFENSE_SCHEMA {
            return Err(format!("schema {} is not {OFFENSE_SCHEMA}", self.schema));
        }
        if self.rows.len() > MAX_OFFENSE_ROWS {
            return Err(format!("{} rows, over {MAX_OFFENSE_ROWS}", self.rows.len()));
        }
        let Offender::Character(actor) = self.key.offender else {
            return Ok(());
        };
        if self
            .rows
            .iter()
            .filter_map(ProvenOffense::site_act)
            .any(|(_, act)| act.actor != actor)
        {
            return Err("a site act names another actor".to_owned());
        }
        Ok(())
    }
}

/// **An offense record's slot**: read, or claimed by an unreadable document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OffenseSlot {
    /// Read and checked.
    Present(Box<OffenseRecord>),
    /// The key is claimed but the document will not read: nothing is proven or fined on it
    /// until staff repair it, and no hull is held on it.
    Unreadable,
}

/// The slot of `key`'s record, when one is claimed.
fn slot<'w>(w: &'w World, key: &OffenseKey) -> Option<&'w OffenseSlot> {
    w.module_state::<TacsimStore>()?.offenses.get(key)
}

/// **The record `key` names**, when one reads.
#[must_use]
pub fn record_of(w: &World, key: &OffenseKey) -> Option<OffenseRecord> {
    match slot(w, key)? {
        OffenseSlot::Present(record) => Some((**record).clone()),
        OffenseSlot::Unreadable => None,
    }
}

/// **Every readable record against `offender`**, one a polity, in polity order.
#[must_use]
pub fn records_against(w: &World, offender: Offender) -> Vec<OffenseRecord> {
    w.module_state::<TacsimStore>()
        .map(|store| {
            store
                .offenses
                .iter()
                .filter(|(key, _)| key.offender == offender)
                .filter_map(|(_, slot)| match slot {
                    OffenseSlot::Present(record) => Some((**record).clone()),
                    OffenseSlot::Unreadable => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// **Who holds a lane place de facto** (Q-50-38): the polity whose proven offenses bind there.
/// `None` for a place no region names, where no offense binds.
#[must_use]
pub fn jurisdiction_at(w: &World, place: &CanonicalId) -> Option<FactionId> {
    crate::polity::control_at(w, place)?.de_facto
}

/// **Whether an offense `polity` proved binds at `place`**: that polity holds it de facto.
#[must_use]
pub fn binds_at(w: &World, polity: FactionId, place: &CanonicalId) -> bool {
    jurisdiction_at(w, place) == Some(polity)
}

/// **Who answers for `hull` at docking** (Q-50-68): an organization's commissioned hull (doc 47
/// cut CO.9, on the owned-hull register) through its organization; any other through her pilot,
/// the helm holder. A Co-op charter is titled to the Co-op but answers through its tenant, as
/// she commands her. `None` for a hull nobody commands.
#[must_use]
pub fn answerer_for_hull(w: &World, hull: ObjId) -> Option<Offender> {
    let title = w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()));
    if let Some(PrincipalId::Organization(org)) = title
        && crate::port::owned::owned(w, hull).is_some()
    {
        return Some(Offender::Organization(org));
    }
    crate::port::coop::helm_holder(w, hull).map(Offender::Character)
}

/// **Write one proven offense** (Q-50-37): the one function every prover calls. It is added to
/// the record of `key` unless that record holds it already, and answers whether it was added.
/// A record that will not read takes nothing until staff repair it (logged). A full record drops
/// its oldest settled offense (`settled` decides); a record full of unsettled ones takes no more
/// (logged).
pub fn record_offense(
    ctx: &mut Ctx<'_>,
    key: OffenseKey,
    offense: ProvenOffense,
    settled: impl Fn(&World, &ProvenOffense) -> bool,
) -> bool {
    let mut record = match slot(ctx.w, &key) {
        None => OffenseRecord::new(key),
        Some(OffenseSlot::Present(record)) => (**record).clone(),
        Some(OffenseSlot::Unreadable) => {
            tracing::error!(
                key = %key.doc_key(),
                "tacsim: a proven offense meets an unreadable record; staff repair it"
            );
            return false;
        }
    };
    if record.rows.iter().any(|row| row.same_as(&offense)) {
        return false;
    }
    if record.rows.len() >= MAX_OFFENSE_ROWS {
        let w = &*ctx.w;
        if let Some(i) = record.rows.iter().position(|row| settled(w, row)) {
            record.rows.remove(i);
        } else {
            tracing::error!(
                key = %key.doc_key(),
                "tacsim: an offense record is full of unsettled fines; a proven offense is not kept"
            );
            return false;
        }
    }
    // Doc 50 unit LW.4 (Q-50-40): an official notice at once when a matter opens.
    if let Offender::Character(who) = key.offender {
        let line = OFFENSE_NOTICE
            .replace(
                "{polity}",
                &crate::port::record::polity_name(ctx.w, key.polity),
            )
            .replace("{what}", offense.kind.what());
        ctx.out.sys_to_actor(who, "tacsim.law", line);
    }
    record.rows.push(offense);
    crate::store::put_offense(ctx, &record);
    true
}

/// **The official notice of a newly proven offense** (doc 50 unit LW.4, Q-50-40): `{polity}`,
/// `{what}`. It never names who reported.
pub const OFFENSE_NOTICE: &str = "Official notice: {polity} has recorded an offense against you: {what}. +record shows it, its      fine and what it costs you.";

/// **Rewrite a record whose rows' receipts changed** (a fine opened on a row): the prover's
/// own follow-up, never a new offense.
pub(crate) fn put_receipts(ctx: &mut Ctx<'_>, record: &OffenseRecord) {
    crate::store::put_offense(ctx, record);
}

/// **The record key a document key names**, for a character or an organization: the inverse of
/// [`OffenseKey::doc_key`]. An account's token does not carry back to its id (`None`), so an
/// account's record is known by the key inside its document.
#[must_use]
pub fn key_of_doc(key: &str) -> Option<OffenseKey> {
    let rest = key.strip_prefix(OFFENSE_PREFIX)?;
    let (token, polity) = rest.rsplit_once(':')?;
    let polity = FactionId(polity.parse().ok()?);
    let offender = if let Some(id) = token.strip_prefix('c') {
        Offender::Character(ObjId(id.parse().ok()?))
    } else {
        Offender::Organization(OrgId(token.strip_prefix('o')?.parse().ok()?))
    };
    Some(OffenseKey { offender, polity })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(actor: u32, at_ic: i64) -> UnlawfulAct {
        UnlawfulAct {
            kind: UnlawfulKind::Recovery,
            actor: ObjId(actor),
            subject: 9,
            landholder: None,
            at_ic,
        }
    }

    /// **Doc 50 unit LW.2: the key is actor by polity** (Q-50-37). A character's and an
    /// organization's records under one polity are two documents, and one offender's under two
    /// polities are two; the document key carries both back.
    #[test]
    fn lw2_the_key_is_actor_by_polity() {
        let c = OffenseKey {
            offender: Offender::Character(ObjId(7)),
            polity: FactionId(3),
        };
        let o = OffenseKey {
            offender: Offender::Organization(OrgId(7)),
            polity: FactionId(3),
        };
        let other = OffenseKey {
            polity: FactionId(4),
            ..c
        };
        assert_eq!(c.doc_key().as_str(), "tacsim:offense:c7:3");
        assert_eq!(o.doc_key().as_str(), "tacsim:offense:o7:3");
        assert_ne!(c.doc_key(), other.doc_key());
        assert_eq!(key_of_doc(other.doc_key().as_str()), Some(other));
        assert_eq!(key_of_doc(o.doc_key().as_str()), Some(o));
        let round: OffenseKey =
            serde_json::from_value(serde_json::to_value(o).expect("encodes")).expect("decodes");
        assert_eq!(round, o);
    }

    /// The record's own rules: the closed kinds a site row maps to, the schema, and a
    /// character's record holds only that character's acts.
    #[test]
    fn lw2_a_record_checks_its_kinds_and_its_actor() {
        assert_eq!(
            OffenseKind::of_unlawful(UnlawfulKind::Scrap),
            Some(OffenseKind::SalvageScrap)
        );
        assert_eq!(OffenseKind::of_unlawful(UnlawfulKind::Capture), None);
        let key = OffenseKey {
            offender: Offender::Character(ObjId(7)),
            polity: FactionId(3),
        };
        let row = |actor| ProvenOffense {
            kind: OffenseKind::SalvageRecovery,
            evidence: Evidence::Site {
                site: CanonicalId::new("urvara_port").expect("id"),
                act: act(actor, 10),
            },
            proof: Proof::Vacated,
            proven_ic: 11,
            debt: None,
            payout: None,
        };
        let mut record = OffenseRecord::new(key);
        record.rows.push(row(7));
        assert!(record.check().is_ok());
        assert!(record.rows[0].same_as(&row(7)));
        assert!(!record.rows[0].same_as(&row(8)));
        record.rows.push(row(8));
        assert!(record.check().is_err(), "a row naming another actor");
    }
}
