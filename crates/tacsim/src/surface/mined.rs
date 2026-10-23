//! **The `tacsim.surface.mined` ring** (doc 20 cut SM.7; §6 "a bounded per-site ring publishes
//! the cycle as the fact `tacsim.surface.mined` for contracts and commissions"; the SM.7 brief's
//! lead ruling 6, its 3.10 and decision R12).
//!
//! One normal-lane document a site, `tacsim:mined:{site}` ([`MinedRing`]): the last
//! [`MAX_MINED_ENTRIES`] cycles that moved ore there, written **only by the `tacsim.surface`
//! tick's tell**, beside the projection and only once that projection's write landed (SM.4a's
//! D24 order), so a retold cycle never doubles. **Read by nothing yet** — contracts and
//! commissions are later cuts; the ring and the pocket cursor are their sources. It is the
//! normal lane's own record, never `industry.extraction.cycled` (industry's families are not
//! written by tacsim).
//!
//! A cycle that moved nothing (a barren core) is never pushed (decision R12): a future public
//! reader must never learn from the ring that a pocket is barren.

use mush_core::{CanonicalId, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The documents' prefix: `tacsim:mined:{site}`.
pub const MINED_PREFIX: &str = "tacsim:mined:";
/// [`MinedRing::schema`].
pub const MINED_SCHEMA: u16 = 1;
/// The most facts one site's ring keeps (a code constant: the brief's lead ruling 10 pins it).
pub const MAX_MINED_ENTRIES: usize = 64;

/// **One cycle that moved ore** — the fact `tacsim.surface.mined`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinedFact {
    /// The IC instant the cycle landed at (its own due instant).
    pub at_ic: i64,
    /// The terrain vehicle.
    pub tv: ObjId,
    /// Its operator.
    pub operator: ObjId,
    /// The pocket's address (`C117`).
    pub pocket: String,
    /// The ore good moved.
    pub good: CanonicalId,
    /// The tonnes moved, at least 1.
    pub tonnes: u32,
    /// The run's ordinal.
    pub ordinal: u32,
    /// The run's cycle.
    pub k: u8,
}

impl MinedFact {
    /// The ring's order: `(at_ic, tv, ordinal, k)`.
    fn order(&self) -> (i64, u32, u32, u8) {
        (self.at_ic, self.tv.0, self.ordinal, self.k)
    }
}

/// **One site's ring** (§6): bounded, whole document, strict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinedRing {
    /// [`MINED_SCHEMA`].
    pub schema: u16,
    /// The site entry: the key.
    pub site: CanonicalId,
    /// At most [`MAX_MINED_ENTRIES`] facts, strictly increasing in `(at_ic, tv, ordinal, k)`.
    pub entries: Vec<MinedFact>,
}

impl MinedRing {
    /// An empty ring for `site`.
    #[must_use]
    pub fn new(site: CanonicalId) -> MinedRing {
        MinedRing {
            schema: MINED_SCHEMA,
            site,
            entries: Vec::new(),
        }
    }

    /// **`tacsim:mined:{site}`**.
    #[must_use]
    pub fn key(&self) -> SmolStr {
        mined_key(&self.site)
    }

    /// **The ring's bounds**: the schema; at most [`MAX_MINED_ENTRIES`] facts, each with at least
    /// one tonne, strictly increasing in `(at_ic, tv, ordinal, k)` — so no fact is held twice.
    ///
    /// # Errors
    ///
    /// The first bound broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != MINED_SCHEMA {
            return Err(format!(
                "a mined ring is schema {MINED_SCHEMA}, not {}",
                self.schema
            ));
        }
        if self.entries.len() > MAX_MINED_ENTRIES {
            return Err(format!(
                "a mined ring keeps at most {MAX_MINED_ENTRIES} facts"
            ));
        }
        if self.entries.iter().any(|fact| fact.tonnes == 0) {
            return Err("a mined fact moved at least a tonne".to_owned());
        }
        if self
            .entries
            .windows(2)
            .any(|pair| match (pair.first(), pair.get(1)) {
                (Some(a), Some(b)) => a.order() >= b.order(),
                _ => true,
            })
        {
            return Err("a mined ring's facts are in strictly increasing order".to_owned());
        }
        Ok(())
    }
}

/// **Push one fact** (the brief's 3.10; reviewer F3: tells do NOT arrive in `at_ic` order — one
/// submission an operator a fire, back-off, a projection write refused once and retold a fire
/// later behind other rigs' newer facts). A fact already present (the same `tv`, `ordinal`, `k`)
/// is not pushed again (a retold cycle never doubles): `false`. Otherwise it is **inserted at its
/// place** in `(at_ic, tv, ordinal, k)` order, and past [`MAX_MINED_ENTRIES`] the smallest key is
/// dropped — so a fact older than every kept entry of a full ring is itself the one dropped, and
/// `push` returns `false` with the ring unchanged. A ring `push` built always passes `check`.
pub fn push(ring: &mut MinedRing, fact: MinedFact) -> bool {
    if ring
        .entries
        .iter()
        .any(|kept| kept.tv == fact.tv && kept.ordinal == fact.ordinal && kept.k == fact.k)
    {
        return false;
    }
    let order = fact.order();
    let at = ring
        .entries
        .iter()
        .position(|kept| kept.order() > order)
        .unwrap_or(ring.entries.len());
    if ring.entries.len() >= MAX_MINED_ENTRIES && at == 0 {
        return false;
    }
    ring.entries.insert(at, fact);
    while ring.entries.len() > MAX_MINED_ENTRIES {
        ring.entries.remove(0);
    }
    true
}

/// **`tacsim:mined:{site}`**.
#[must_use]
pub fn mined_key(site: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("{MINED_PREFIX}{site}"))
}

/// The inverse: the tail after [`MINED_PREFIX`] as a canonical site id.
#[must_use]
pub fn mined_key_site(key: &str) -> Option<CanonicalId> {
    key.strip_prefix(MINED_PREFIX)
        .and_then(|tail| CanonicalId::new(tail).ok())
}

/// **One site's ring, as the store holds it.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MinedState {
    /// The document read.
    Present(MinedRing),
    /// A key claims the ring but its body did not read: logged once; it bars nothing, and the
    /// tick's next write replaces it.
    Unreadable,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(at_ic: i64, tv: u32, ordinal: u32, k: u8) -> MinedFact {
        MinedFact {
            at_ic,
            tv: ObjId(tv),
            operator: ObjId(42),
            pocket: "C117".to_owned(),
            good: CanonicalId::new("metal_ore").unwrap_or_else(|_| panic!("an id")),
            tonnes: 2,
            ordinal,
            k,
        }
    }

    fn ring() -> MinedRing {
        MinedRing::new(CanonicalId::new("ahuna_port").unwrap_or_else(|_| panic!("an id")))
    }

    /// **The ring is bounded and never doubles** (the brief's 3.10; reviewer F3): 65 pushes keep 64
    /// with the oldest gone; a repeated fact is not pushed; an out-of-order push lands at its
    /// place and `check` passes; an out-of-order push older than everything in a full ring returns
    /// `false` and leaves the ring unchanged; `check` refuses a hand-built disordered ring, a
    /// zero-tonne fact and a 65-fact ring; the key round-trips.
    #[test]
    fn sm7_the_mined_ring_is_bounded_and_never_doubles() {
        let mut r = ring();
        for n in 0..65_u32 {
            assert!(push(&mut r, fact(1_000 + i64::from(n), 900, n + 1, 1)));
        }
        assert_eq!(r.entries.len(), MAX_MINED_ENTRIES);
        assert_eq!(
            r.entries.first().map(|f| f.at_ic),
            Some(1_001),
            "the oldest gone"
        );
        assert_eq!(r.check(), Ok(()));
        // A retell never doubles.
        let before = r.clone();
        assert!(!push(&mut r, fact(1_064, 900, 65, 1)));
        assert_eq!(r, before);
        // Out of order, inside the kept span: lands at its place.
        assert!(push(&mut r, fact(1_010, 901, 1, 3)));
        assert_eq!(r.check(), Ok(()));
        assert_eq!(r.entries.len(), MAX_MINED_ENTRIES);
        assert_eq!(r.entries.first().map(|f| f.at_ic), Some(1_002));
        assert!(r.entries.iter().any(|f| f.tv == ObjId(901)));
        // Older than everything in a full ring: dropped, unchanged.
        let before = r.clone();
        assert!(!push(&mut r, fact(10, 902, 1, 1)));
        assert_eq!(r, before);
        // A small ring takes an old fact at its place.
        let mut small = ring();
        assert!(push(&mut small, fact(500, 900, 2, 1)));
        assert!(push(&mut small, fact(400, 900, 1, 1)));
        assert_eq!(small.entries.first().map(|f| f.at_ic), Some(400));
        assert_eq!(small.check(), Ok(()));
        // `check` refuses what push never builds.
        let mut disordered = small.clone();
        disordered.entries.reverse();
        assert!(disordered.check().is_err());
        let mut zero = small.clone();
        if let Some(first) = zero.entries.first_mut() {
            first.tonnes = 0;
        }
        assert!(zero.check().is_err());
        let mut over = r.clone();
        over.entries.push(fact(99_999, 999, 1, 1));
        assert!(over.check().is_err());
        assert_eq!(r.key().as_str(), "tacsim:mined:ahuna_port");
        assert_eq!(
            mined_key_site("tacsim:mined:ahuna_port"),
            Some(r.site.clone())
        );
        assert_eq!(mined_key_site("tacsim:mindsrc:ab"), None);
        // The wire is strict.
        let wire = serde_json::to_value(&r).unwrap_or_else(|e| panic!("{e}"));
        let back: MinedRing =
            serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, r);
        let mut extra = wire;
        extra["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<MinedRing>(extra).is_err());
    }
}
