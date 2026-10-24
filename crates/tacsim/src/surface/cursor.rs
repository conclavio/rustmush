//! **The pocket cursor sidecar** (doc 20 cut SM.6; §3.4, D-20-5): `tacsim.pockets`, one
//! [`PocketCursor`] per surface deposit, keyed by the deposit's id — the record of what the
//! surface provider has drawn from each of the block's ore pockets of the deposit's family,
//! and **the lock's one home** (a [`PocketLock`] on a row; SM.7 writes it).
//!
//! It is industry's `ExternalDraws` answer: the composed `SurfaceDraws`
//! (`crate::composition::industry_seams`) reports [`PocketCursor::drawn_total`], and industry's
//! 60 s sweep checks `initial_reserve − pool == Σ drawn` for every surface deposit, closing
//! the whole block on a mismatch. Written in the **same bundle** as every draw from the
//! deposit's pool — SM.6's worked transfer (`tacsim_site_work`) inserts it; SM.7's cycles
//! update it — so the invariant holds from the first sweep.
//!
//! **Derived truth, server-side by design** (§3.4; the brief's reviewer finding F6): a row's
//! `drawn_t` is `⌊t × ‰ / 1000⌋` of a pocket's true tonnes and a cursor's existence says the
//! block holds that family's ore. The family must serialise (it is durable), so it is not in
//! `truth_serialization_policy`; it is **never projected to a player** — no outcome, offer or
//! player text carries a cursor row. The pits SM.7 and SM.9 draw are derived from it there.
//!
//! Pockets are named by SM.4a's address vocabulary (`crate::surface::address`): the block's
//! pocket number `1..=256`, never a second scheme.

use mush_core::ObjId;
use mush_industry::OreDepositId;
use serde::{Deserialize, Serialize};

/// The family's id.
pub const POCKETS_FAMILY: &str = "tacsim.pockets";
/// The record's schema.
pub const POCKET_CURSOR_SCHEMA: u16 = 1;
/// Rows in one cursor: at most the block's 64 ore pockets of one family (§3.2: a field
/// holds at most 16 ore pockets of a family, a block four fields; §11).
pub const MAX_CURSOR_ROWS: usize = 64;
/// A rich pocket's top tonnage (§3.2): no pocket gives more.
pub const MAX_POCKET_TONNES: u32 = 24;
/// The pocket numbers of a block: `1..=256`.
pub const MAX_POCKET_NUMBER: u16 = 256;

/// **One deposit's cursor** (doc 20 §3.4): which of the block's pockets have given what.
/// Derived truth, never projected (the module doc).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketCursor {
    /// [`POCKET_CURSOR_SCHEMA`].
    pub schema: u16,
    /// The deposit this cursor accounts for (re-checked against the key by every reader).
    pub deposit: OreDepositId,
    /// One row per pocket drawn or locked, pocket numbers strictly increasing.
    pub rows: Vec<CursorRow>,
}

/// **One pocket's row**: its number in the block and the tonnes drawn from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorRow {
    /// The block's pocket number, `1..=256` (`crate::surface::address`).
    pub pocket: u16,
    /// Whole tonnes drawn, at most [`MAX_POCKET_TONNES`].
    pub drawn_t: u32,
    /// The rig working it, if any — the lock's one home (SM.7 writes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<PocketLock>,
}

/// **A pocket's lock** (SM.7): the rig and its act ordinal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketLock {
    /// The terrain vehicle.
    pub tv: ObjId,
    /// The rig's act ordinal that holds it.
    pub ordinal: u32,
}

impl PocketCursor {
    /// The record's own invariants: the schema; at most [`MAX_CURSOR_ROWS`] rows, pocket
    /// numbers strictly increasing in `1..=256`; every `drawn_t` at most
    /// [`MAX_POCKET_TONNES`]; every row drawn or locked.
    ///
    /// # Errors
    ///
    /// The first rule broken, in words.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != POCKET_CURSOR_SCHEMA {
            return Err(format!(
                "a pocket cursor is schema {POCKET_CURSOR_SCHEMA}, not {}",
                self.schema
            ));
        }
        if self.rows.len() > MAX_CURSOR_ROWS {
            return Err(format!(
                "a pocket cursor holds at most {MAX_CURSOR_ROWS} rows"
            ));
        }
        let mut last = 0_u16;
        for row in &self.rows {
            if row.pocket == 0 || row.pocket > MAX_POCKET_NUMBER {
                return Err(format!(
                    "pocket {} is not a block's pocket (1..={MAX_POCKET_NUMBER})",
                    row.pocket
                ));
            }
            if row.pocket <= last {
                return Err(
                    "a pocket cursor's rows are in strictly increasing pocket order".to_owned(),
                );
            }
            last = row.pocket;
            if row.drawn_t > MAX_POCKET_TONNES {
                return Err(format!(
                    "pocket {} gives at most {MAX_POCKET_TONNES} t",
                    row.pocket
                ));
            }
            if row.drawn_t == 0 && row.lock.is_none() {
                return Err(format!(
                    "pocket {}'s row is neither drawn nor locked",
                    row.pocket
                ));
            }
        }
        Ok(())
    }

    /// Σ `drawn_t`, checked (64 rows of 24 t cannot overflow, but the sum is checked anyway).
    #[must_use]
    pub fn drawn_total(&self) -> Option<u64> {
        self.rows
            .iter()
            .try_fold(0_u64, |sum, row| sum.checked_add(u64::from(row.drawn_t)))
    }

    /// The row for `pocket`, if any.
    #[must_use]
    pub fn row(&self, pocket: u16) -> Option<&CursorRow> {
        self.rows.iter().find(|row| row.pocket == pocket)
    }
}

/// A cursor's key text: the deposit id as industry renders it, `{:016x}`.
#[must_use]
pub fn cursor_key_text(deposit: &OreDepositId) -> String {
    format!("{:016x}", deposit.0)
}

/// The deposit a cursor key text names (strict: 16 lowercase hex digits).
#[must_use]
pub fn cursor_key_parse(text: &str) -> Option<OreDepositId> {
    (text.len() == 16
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| u64::from_str_radix(text, 16).ok())
    .flatten()
    .map(OreDepositId)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pocket: u16, drawn_t: u32) -> CursorRow {
        CursorRow {
            pocket,
            drawn_t,
            lock: None,
        }
    }

    fn cursor(rows: Vec<CursorRow>) -> PocketCursor {
        PocketCursor {
            schema: POCKET_CURSOR_SCHEMA,
            deposit: OreDepositId(7),
            rows,
        }
    }

    /// Every refusal of the check, the total and a 65-row cursor.
    #[test]
    fn sm6_the_cursor_check_refuses_every_bad_row() {
        assert!(
            cursor(vec![row(1, 3), row(17, 24), row(256, 1)])
                .check()
                .is_ok()
        );
        assert!(cursor(Vec::new()).check().is_ok());
        let mut bad = cursor(vec![row(1, 3)]);
        bad.schema = 2;
        assert!(bad.check().is_err(), "the schema");
        assert!(cursor(vec![row(0, 1)]).check().is_err(), "pocket 0");
        assert!(cursor(vec![row(257, 1)]).check().is_err(), "pocket 257");
        assert!(
            cursor(vec![row(5, 1), row(5, 1)]).check().is_err(),
            "repeated"
        );
        assert!(
            cursor(vec![row(6, 1), row(5, 1)]).check().is_err(),
            "out of order"
        );
        assert!(
            cursor(vec![row(5, 25)]).check().is_err(),
            "past a rich pocket"
        );
        assert!(
            cursor(vec![row(5, 0)]).check().is_err(),
            "neither drawn nor locked"
        );
        let locked = CursorRow {
            pocket: 5,
            drawn_t: 0,
            lock: Some(PocketLock {
                tv: ObjId(9),
                ordinal: 1,
            }),
        };
        assert!(cursor(vec![locked]).check().is_ok(), "a lock alone stands");
        let full: Vec<CursorRow> = (1..=64).map(|n| row(n, 24)).collect();
        assert!(cursor(full.clone()).check().is_ok());
        assert_eq!(cursor(full).drawn_total(), Some(64 * 24));
        let over: Vec<CursorRow> = (1..=65).map(|n| row(n, 1)).collect();
        assert!(cursor(over).check().is_err(), "65 rows");
        assert_eq!(cursor(vec![row(1, 3), row(2, 4)]).row(2), Some(&row(2, 4)));
        assert_eq!(cursor(vec![row(1, 3)]).row(2), None);
    }

    /// The key text round-trips and is strict.
    #[test]
    fn sm6_the_cursor_key_is_industrys_id_text() {
        let id = OreDepositId(0x00ab_cdef_0123_4567);
        assert_eq!(cursor_key_text(&id), "00abcdef01234567");
        assert_eq!(cursor_key_parse(&cursor_key_text(&id)), Some(id));
        assert_eq!(cursor_key_parse("00ABCDEF01234567"), None);
        assert_eq!(cursor_key_parse("abc"), None);
    }

    /// The wire is strict.
    #[test]
    fn sm6_the_cursor_wire_is_strict() {
        let good = serde_json::json!({ "schema": 1, "deposit": 7, "rows": [{ "pocket": 1, "drawn_t": 2 }] });
        assert!(serde_json::from_value::<PocketCursor>(good).is_ok());
        let extra = serde_json::json!({ "schema": 1, "deposit": 7, "rows": [], "x": 1 });
        assert!(serde_json::from_value::<PocketCursor>(extra).is_err());
    }
}
