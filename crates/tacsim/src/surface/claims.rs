//! **The block-level claim record and the site's claim log** (doc 20 cut SM.10; §9.2–§9.6,
//! D-20-12; the SM.10 brief's 3.4, decision D17, reviewer F4).
//!
//! - `tacsim.claims` ([`BlockTenure`], keyed [`BlockKey`] `{site}/{l}`, the letter lower-cased)
//!   is **the one public source of a block's tenure**: the site issuer's claim on the block
//!   ([`BlockClaim`]) and another issuer's paper over it ([`BlockPaper`]). It is written **only** by the composed
//!   tenure authority's staging (`crate::composition::industry_seams::SurfaceTenure`), inside
//!   industry's own tenure bundle, so it moves atomically with the block's industry claims —
//!   for a block with no opened deposit it is the claim alone. The commands, the site clerk,
//!   the map, the registry and the mining gate read it; industry's claims remain the
//!   per-deposit authorisation (the mining gate reads both), and the epoch and the cooling
//!   rows stay industry's (`mush_industry::TenureRecord`).
//! - `tacsim:claimlog:{site}` ([`ClaimLog`]) is the normal-lane record of what the site clerk
//!   has already done that the typed record cannot say: the lapses it chronicled, the refunds
//!   it paid, and the paper's adjudication contract. **An unreadable log is dropped loudly and
//!   paged once; it bars nothing** — a re-chronicle, and the lapse's tell with it (the judge's
//!   conformance F9), is then guarded by `mush_canon::records_for_source`, a refund is re-attempted only under its own
//!   idempotency key (an `AlreadyPosted` answer counts as done), and the paper contract is
//!   re-found by its issuer and name before any re-file. The `tacsim:mined:` precedent.
//!
//! **No count, no reserve.** Nothing here names how many deposits a block holds: a claim is
//! one act over the whole block (lead ruling 1), and every public line reads this record, so a
//! barren block and an ore-bearing one answer alike (SM.6's D14).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::port::def::BLOCK_LETTERS;
use crate::surface::geology::OreFamily;

/// The family's id.
pub const CLAIMS_FAMILY: &str = "tacsim.claims";
/// [`BlockTenure::schema`].
pub const CLAIM_SCHEMA: u16 = 1;
/// The claim log's document prefix: `tacsim:claimlog:{site}`.
pub const CLAIM_LOG_PREFIX: &str = "tacsim:claimlog:";
/// [`ClaimLog::schema`].
pub const CLAIM_LOG_SCHEMA: u16 = 1;
/// The most rows any one of the log's maps holds: one per block letter (the paper map one per
/// papers row, at most sixteen too).
pub const MAX_CLAIM_LOG_ROWS: usize = 16;

/// **The family key**: one record per (site, block letter).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockKey {
    /// The site's port entry.
    pub site: CanonicalId,
    /// The block letter, `A`..=`P`.
    pub letter: char,
}

impl BlockKey {
    /// `{site}/{l}`: the letter **lower-cased** (a typed-state key admits no upper-case byte).
    #[must_use]
    pub fn key_text(&self) -> String {
        format!("{}/{}", self.site, self.letter.to_ascii_lowercase())
    }

    /// The strict inverse of [`BlockKey::key_text`]: one `/`, a canonical site id and one
    /// lower-case block letter `a`..=`p`.
    #[must_use]
    pub fn parse(text: &str) -> Option<BlockKey> {
        let (site, letter) = text.rsplit_once('/')?;
        let mut chars = letter.chars();
        let (Some(lower), None) = (chars.next(), chars.next()) else {
            return None;
        };
        let letter = lower.to_ascii_uppercase();
        if !lower.is_ascii_lowercase() || !BLOCK_LETTERS.contains(&letter) {
            return None;
        }
        Some(BlockKey {
            site: CanonicalId::new(site).ok()?,
            letter,
        })
    }
}

/// **One block's public tenure** (doc 20 §9; cut SM.10, D17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockTenure {
    /// [`CLAIM_SCHEMA`].
    pub schema: u16,
    /// The site's port entry (the key's).
    pub site: CanonicalId,
    /// The block letter (the key's), one of `A`..=`P`.
    pub letter: String,
    /// The block place.
    pub place: ObjId,
    /// The **site issuer's** claim on the block: a player's stake, or an org's seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<BlockClaim>,
    /// Another issuer's paper over the block (the Pallas Syndicate's at Occator Rim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<BlockPaper>,
}

/// **The site issuer's claim on a block** (doc 20 §9.2–§9.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockClaim {
    /// A character (a player claim) or the org (a seed).
    pub holder: PrincipalId,
    /// The holder's gateway account at registration (player claims only): the two-claim cap's
    /// key, counted per account as the licence is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<mush_core::AccountId>,
    /// Industry's tenure epoch this holding speaks for.
    pub epoch: u32,
    /// The opening this holding was staked against (reviewer F3): `None` for epoch 0 ("since
    /// the block opened"), else the lapse instant the stake was made against. A contest's
    /// window opens here; a take-over keeps it, a contest transfer keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_ic: Option<i64>,
    /// The stake, take-over, contest transfer or seed instant; a renewal keeps it.
    pub registered_ic: i64,
    /// The current term's start.
    pub term_start_ic: i64,
    /// The current term's end.
    pub term_end_ic: i64,
    /// Σ `drawn_t` over the block's cursors at the term's start (lead ruling 4): worked this
    /// term is the cursors' sum now less this.
    pub worked_at_term_start: u64,
    /// The holder's discovery stamp: the stake's earliest hand-made reading, or the contest's
    /// heard sounding. `None` for a seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_ic: Option<i64>,
    /// The family the holder declared at the stake (public on the registry; D15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<OreFamily>,
    /// The last fee contract (`ContractId`'s number): the stake's, the renewal's or the
    /// contest's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee: Option<u64>,
    /// The term fee owed back to a contest's loser (paid once by the site clerk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refund: Option<RefundOwed>,
    /// When the claim lapsed (the clerk's landed lapse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lapsed_ic: Option<i64>,
}

/// **The loser's term fee, owed back** (doc 20 §9.4; reviewer F2): paid by the site clerk from
/// the Registry's account only when `paid_by` settled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefundOwed {
    /// The loser.
    pub to: ObjId,
    /// The amount, minor units of the credit.
    pub minor: i64,
    /// The loser's last fee contract (`ContractId`'s number).
    pub paid_by: u64,
}

/// **Another issuer's paper over a block** (doc 20 §9.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockPaper {
    /// The paper's issuer, a lane org id (`pallas_syndicate`).
    pub issuer: CanonicalId,
    /// The site's papers row (`occator_outer`).
    pub row: CanonicalId,
    /// The adjudication contract (`ContractId`'s number).
    pub contract: u64,
    /// Where the paper stands.
    pub state: PaperState,
    /// The renewal term's start at which the Registrar suspended it (the clerk's tell of that
    /// renewal names the suspension; a later renewal's does not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_ic: Option<i64>,
}

/// **Where a paper stands** (doc 20 §9.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperState {
    /// Filed and unruled: it bars nothing.
    Contested,
    /// Suspended by the site's issuer under the Registrar's rule.
    Suspended,
}

impl BlockClaim {
    /// Whether the claim stands at `now_ic`: not lapsed and its term not over.
    #[must_use]
    pub fn live(&self, now_ic: i64) -> bool {
        self.lapsed_ic.is_none() && self.term_end_ic >= now_ic
    }

    /// Whether the term is over and nothing has lapsed it yet (SM.7's R20: the mining gate
    /// reads it as open ground; `+registry` reads it "lapsing").
    #[must_use]
    pub fn lapsing(&self, now_ic: i64) -> bool {
        self.lapsed_ic.is_none() && self.term_end_ic < now_ic
    }

    /// Whether the holder is a character (a player claim), not an org (a seed).
    #[must_use]
    pub fn player_held(&self) -> bool {
        matches!(self.holder, PrincipalId::Object(_))
    }
}

impl BlockTenure {
    /// A record with neither a claim nor a paper.
    #[must_use]
    pub fn new(site: CanonicalId, letter: char, place: ObjId) -> BlockTenure {
        BlockTenure {
            schema: CLAIM_SCHEMA,
            site,
            letter: letter.to_string(),
            place,
            claim: None,
            paper: None,
        }
    }

    /// The record's key.
    #[must_use]
    pub fn key(&self) -> Option<BlockKey> {
        let mut chars = self.letter.chars();
        let (Some(letter), None) = (chars.next(), chars.next()) else {
            return None;
        };
        Some(BlockKey {
            site: self.site.clone(),
            letter,
        })
    }

    /// The record's own invariants: the schema; the letter `A`..=`P`; a claim's term starting
    /// before it ends and registered no later than it starts; its `opened_ic` no later than its
    /// discovery when both are present; and its `account` present exactly when its holder is a
    /// character.
    ///
    /// # Errors
    ///
    /// The first rule broken, in words.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != CLAIM_SCHEMA {
            return Err(format!(
                "a block tenure is schema {CLAIM_SCHEMA}, not {}",
                self.schema
            ));
        }
        if !self
            .key()
            .is_some_and(|key| BLOCK_LETTERS.contains(&key.letter))
        {
            return Err(format!("{} is not a block letter A to P", self.letter));
        }
        if let Some(claim) = &self.claim {
            if claim.term_start_ic >= claim.term_end_ic {
                return Err("a block claim's term starts before it ends".to_owned());
            }
            if claim.registered_ic > claim.term_start_ic {
                return Err("a block claim is registered no later than its term starts".to_owned());
            }
            if let (Some(opened), Some(discovered)) = (claim.opened_ic, claim.discovered_ic)
                && opened > discovered
            {
                return Err("a block claim's discovery is no earlier than its opening".to_owned());
            }
            if claim.account.is_some() != claim.player_held() {
                return Err(
                    "a block claim carries a gateway account exactly when a character holds it"
                        .to_owned(),
                );
            }
            if let Some(refund) = &claim.refund
                && refund.minor <= 0
            {
                return Err("a refund owed is a positive amount".to_owned());
            }
        }
        Ok(())
    }
}

// ---- the claim log -------------------------------------------------------------------------

/// **One site's claim log** (doc 20 cut SM.10; the brief's 3.4 item 14): what the site clerk
/// has already done, so an item found done falls through (reviewer F6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimLog {
    /// [`CLAIM_LOG_SCHEMA`].
    pub schema: u16,
    /// The site entry: the key.
    pub site: CanonicalId,
    /// Block letter → the last epoch whose lapse was chronicled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chronicled: BTreeMap<String, u32>,
    /// Block letter → the last epoch whose contest refund was paid (or found never owed).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refunded: BTreeMap<String, u32>,
    /// Papers row → its adjudication contract (`ContractId`'s number).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paper: BTreeMap<String, u64>,
}

impl ClaimLog {
    /// An empty log for `site`.
    #[must_use]
    pub fn new(site: CanonicalId) -> ClaimLog {
        ClaimLog {
            schema: CLAIM_LOG_SCHEMA,
            site,
            chronicled: BTreeMap::new(),
            refunded: BTreeMap::new(),
            paper: BTreeMap::new(),
        }
    }

    /// **`tacsim:claimlog:{site}`**.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{CLAIM_LOG_PREFIX}{}", self.site)
    }

    /// The log's own invariants: the schema; each map at most [`MAX_CLAIM_LOG_ROWS`] rows, the
    /// letter maps keyed by block letters.
    ///
    /// # Errors
    ///
    /// The first rule broken, in words.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != CLAIM_LOG_SCHEMA {
            return Err(format!(
                "a claim log is schema {CLAIM_LOG_SCHEMA}, not {}",
                self.schema
            ));
        }
        for (name, len) in [
            ("chronicled", self.chronicled.len()),
            ("refunded", self.refunded.len()),
            ("paper", self.paper.len()),
        ] {
            if len > MAX_CLAIM_LOG_ROWS {
                return Err(format!(
                    "a claim log's {name} holds at most {MAX_CLAIM_LOG_ROWS} rows"
                ));
            }
        }
        for letter in self.chronicled.keys().chain(self.refunded.keys()) {
            let mut chars = letter.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if BLOCK_LETTERS.contains(&c) => {}
                _ => return Err(format!("{letter} is not a block letter A to P")),
            }
        }
        Ok(())
    }
}

/// What the store holds under a claim log's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimLogState {
    /// A good log.
    Present(ClaimLog),
    /// A body that would not read: dropped loudly, paged once, and it bars nothing.
    Unreadable,
}

/// The site a claim log key names (`tacsim:claimlog:{site}`), strictly.
#[must_use]
pub fn claim_log_key_site(key: &str) -> Option<CanonicalId> {
    CanonicalId::new(key.strip_prefix(CLAIM_LOG_PREFIX)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap_or_else(|_| panic!("{text}"))
    }

    fn claim() -> BlockClaim {
        BlockClaim {
            holder: PrincipalId::Object(ObjId(7)),
            account: Some(
                mush_core::AccountId::mint("pilot", 1).unwrap_or_else(|_| panic!("an id")),
            ),
            epoch: 0,
            opened_ic: None,
            registered_ic: 100,
            term_start_ic: 100,
            term_end_ic: 200,
            worked_at_term_start: 0,
            discovered_ic: Some(90),
            declared: None,
            fee: Some(4),
            refund: None,
            lapsed_ic: None,
        }
    }

    /// The key round-trips and is strict.
    #[test]
    fn sm10_the_block_key_is_strict() {
        let key = BlockKey {
            site: cid("ahuna_port"),
            letter: 'C',
        };
        assert_eq!(key.key_text(), "ahuna_port/c");
        assert_eq!(BlockKey::parse("ahuna_port/c"), Some(key));
        assert_eq!(BlockKey::parse("ahuna_port/q"), None);
        assert_eq!(BlockKey::parse("ahuna_port/C"), None);
        assert_eq!(BlockKey::parse("ahuna_port/cd"), None);
        assert_eq!(BlockKey::parse("ahuna_port"), None);
    }

    /// Every refusal of the check, and live/lapsing.
    #[test]
    fn sm10_the_block_tenure_check_refuses_every_bad_record() {
        let mut record = BlockTenure::new(cid("ahuna_port"), 'C', ObjId(9));
        assert!(record.check().is_ok());
        record.claim = Some(claim());
        assert!(record.check().is_ok());
        let edit = |change: &dyn Fn(&mut BlockClaim)| {
            let mut bad = record.clone();
            if let Some(claim) = bad.claim.as_mut() {
                change(claim);
            }
            bad.check()
        };
        assert!(edit(&|c| c.term_end_ic = 100).is_err(), "an empty term");
        assert!(edit(&|c| c.registered_ic = 101).is_err(), "registered late");
        assert!(
            edit(&|c| c.opened_ic = Some(95)).is_err(),
            "opened after discovery"
        );
        assert!(
            edit(&|c| c.account = None).is_err(),
            "a player without an account"
        );
        assert!(
            edit(&|c| c.holder = PrincipalId::Organization(mush_org::OrgId(3))).is_err(),
            "an org with an account"
        );
        let mut bad = record.clone();
        bad.letter = "Q".to_owned();
        assert!(bad.check().is_err());
        let mut bad = record.clone();
        bad.schema = 2;
        assert!(bad.check().is_err());
        let live = claim();
        assert!(live.live(200) && !live.lapsing(200));
        assert!(!live.live(201) && live.lapsing(201));
        let mut lapsed = claim();
        lapsed.lapsed_ic = Some(150);
        assert!(!lapsed.live(150) && !lapsed.lapsing(300));
        // The wire is strict.
        let mut value = serde_json::to_value(&record).unwrap_or_default();
        if let Some(map) = value.as_object_mut() {
            map.insert("x".to_owned(), serde_json::json!(1));
        }
        assert!(serde_json::from_value::<BlockTenure>(value).is_err());
    }

    /// The log's bounds and key.
    #[test]
    fn sm10_the_claim_log_is_bounded() {
        let mut log = ClaimLog::new(cid("occator_port"));
        assert_eq!(log.key(), "tacsim:claimlog:occator_port");
        assert_eq!(claim_log_key_site(&log.key()), Some(cid("occator_port")));
        assert!(log.check().is_ok());
        log.chronicled.insert("Q".to_owned(), 1);
        assert!(log.check().is_err(), "not a letter");
        let mut log = ClaimLog::new(cid("occator_port"));
        for n in 0..17_u64 {
            log.paper.insert(format!("row{n}"), n);
        }
        assert!(log.check().is_err(), "seventeen paper rows");
    }
}
