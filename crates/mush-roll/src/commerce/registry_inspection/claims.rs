//! **The quirk claim** (doc 17h cut M8g G3; doc 17 §10 "The quirk", D-UM-55): an author's
//! testimony to the Registry naming hulls their own sensors earned, kept as one
//! [`ClaimRecord`] under the author. A claim is testimony, never a finding: the engine confirms
//! and denies nothing, and only an inspection or a box read establishes a fact (M8a). Each
//! hull it names is the scoped fact `minds.claimed` (`hull/<hull>` at the claim's sequence,
//! [`ClaimRecord::facts`]), which M8d parses and no public condition resolves.
//!
//! The record is the Roll's (family [`CLAIM_FAMILY`]), written from tacsim's transaction
//! through the inspection writer's delegation, as the inspection record is: tacsim's provider
//! reads what the author earned and holds; the writer lands the record.
use super::*;
use mush_core::ClockStamp;
use serde::Deserializer;
use smol_str::SmolStr;

/// The most claim records the family keeps for one author (G3's bound): `seq` counts an
/// author's claims from 1 and the record is keyed `author/<author>/slot/<seq mod 32>`
/// ([`claim_key`]), so the 33rd claim overwrites the slot of the first, in the same
/// transaction. The newest is the slot with the highest `seq`.
pub const CLAIMS_PER_AUTHOR: u64 = 32;
/// The most hulls one claim names.
pub const MAX_CLAIM_HULLS: usize = 8;
/// The most bytes of the author's own words one claim carries.
pub const MAX_CLAIM_TEXT: usize = 512;

/// **The key of `author`'s claim number `seq`**: `author/<author>/slot/<seq mod 32>`, in base
/// 16 at fixed widths (eight and two), the ring slot.
#[must_use]
pub fn claim_key(author: ObjId, seq: u64) -> String {
    format!(
        "author/{:08x}/slot/{:02x}",
        author.0,
        seq % CLAIMS_PER_AUTHOR
    )
}

/// **Every slot key of `author`'s ring**, for the read that finds the newest `seq`.
pub fn claim_slots(author: ObjId) -> impl Iterator<Item = String> {
    (0..CLAIMS_PER_AUTHOR).map(move |slot| claim_key(author, slot))
}

/// Whether `key` is a claim ring key (the family's key shape).
pub(crate) fn valid_key(key: &str) -> bool {
    let mut parts = key.split('/');
    let hex = |v: &str, width: usize| {
        v.len() == width
            && v.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    };
    parts.next() == Some("author")
        && parts.next().is_some_and(|v| hex(v, 8))
        && parts.next() == Some("slot")
        && parts.next().is_some_and(|v| {
            hex(v, 2) && u64::from_str_radix(v, 16).is_ok_and(|n| n < CLAIMS_PER_AUTHOR)
        })
        && parts.next().is_none()
}

/// Whether `text` is a claim's words: one line, trimmed, printable, bounded.
#[must_use]
pub fn valid_text(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_CLAIM_TEXT
        && text.trim() == text
        && !text.chars().any(char::is_control)
}

/// **One claim** (G3): who filed it, with which Registry, the hulls named, the author's words,
/// when. Immutable once written; it confirms nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ClaimWire")]
pub struct ClaimRecord {
    pub format: u16,
    /// The filing's commit.
    pub claim: mush_core::CommitToken,
    pub action: DefinitionRef,
    /// The Registry the claim is filed with.
    pub authority: RegistryInspectionPolicy,
    /// The author (a charter title holder when it was filed).
    pub author: ObjId,
    /// The author's claim count, this one included (from 1): the ring slot's order.
    pub seq: u64,
    /// The hulls named, ascending and distinct, each earned by the author's own sensors.
    pub hulls: Vec<ObjId>,
    /// The author's own words.
    pub text: SmolStr,
    pub at_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimWire {
    format: u16,
    claim: mush_core::CommitToken,
    action: DefinitionRef,
    authority: RegistryInspectionPolicy,
    author: ObjId,
    seq: u64,
    #[serde(deserialize_with = "hulls")]
    hulls: Vec<ObjId>,
    text: SmolStr,
    at_ic: ClockStamp,
}
fn hulls<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ObjId>, D::Error> {
    super::policy::bounded::<D, ObjId, MAX_CLAIM_HULLS>(d)
}
impl TryFrom<ClaimWire> for ClaimRecord {
    type Error = PublicationError;
    fn try_from(w: ClaimWire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            claim: w.claim,
            action: w.action,
            authority: w.authority,
            author: w.author,
            seq: w.seq,
            hulls: w.hulls,
            text: w.text,
            at_ic: w.at_ic,
        };
        v.validate()?;
        Ok(v)
    }
}
impl ClaimRecord {
    /// **The scoped facts this claim produces** (`minds.claimed`): one per hull named, keyed
    /// `hull/<hull>` at the claim's sequence. Scoped to the Registry: no public contract
    /// condition resolves one (doc 17h cut M8d K6, as `minds.unwarranted`).
    #[must_use]
    pub fn facts(&self) -> Vec<mush_economy::ExternalRef> {
        self.hulls
            .iter()
            .map(|hull| super::super::facts::protocol::claimed_reference(*hull, self.seq))
            .collect()
    }
}
impl Record for ClaimRecord {
    const FAMILY: &'static str = CLAIM_FAMILY;
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        claim_key(self.author, self.seq)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.authority.validate()?;
        super::super::WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?;
        if self.format != 1
            || self.seq == 0
            || self.author.0 == 0
            || self.action != self.authority.claim_action().map_err(bad)?
            || self.hulls.is_empty()
            || self.hulls.len() > MAX_CLAIM_HULLS
            || self.hulls.iter().any(|hull| hull.0 == 0)
            || self
                .hulls
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
            || !valid_text(&self.text)
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
