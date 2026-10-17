//! **The per-substep state hash** (16b cut A.1; doc 16 §3.1 "Determinism", §4
//! "Ledger events").
//!
//! *Each substep produces a state hash recorded with its pulse.* That one line
//! is what turns rate independence from a claim into a test: the cross-tempo
//! property test (16b A7) replays one scripted intent and control log at 5,000,
//! 1,000 and 500 ms and asserts an identical **sequence of these hashes**, so a
//! tempo that changed an outcome is caught at the substep it diverged in rather
//! than at the end of the match.
//!
//! For that to work the hash must be a function of the *quantised* state and of
//! nothing else — no addresses, no iteration order that depends on a hash map's
//! seed, no wall time, no floating point. Every byte folded in below is an
//! integer read in unit-id order, and the mix is written out here so that the
//! answer does not move when the toolchain does.
//!
//! **Hash v2** (16c `T-104`, 2026-10-12): the stream of bytes is the one
//! [`state_hash`]'s list gives, unchanged; the mix is [`Fold`], eight bytes a
//! multiply where FNV-1a 64 took one a byte, and a pending notice or intent
//! is folded by `fold_value`'s walk of its fields where its canonical JSON
//! text was. The RNG ([`crate::arena::rng`]) keeps [`fnv1a64_update`], so no
//! draw moved; every state hash did, once, with its pins re-blessed in the
//! same cut.

use crate::arena::state::Arena;

/// FNV-1a 64's offset basis.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64's prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold `bytes` into an FNV-1a 64 accumulator.
///
/// Public because [`crate::arena::rng::draw`] keys on this primitive: one
/// mix, written once, testable on its own. Since 16c `T-104` the state hash
/// folds with [`Fold`] instead; this stays the RNG's and the designator
/// book's, so neither moved.
#[must_use]
pub fn fnv1a64_update(state: u64, bytes: &[u8]) -> u64 {
    let mut hash = state;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// **The hash v2 fold** (16c `T-104`): the state hash's stream — the fields
/// [`state_hash`]'s list gives, in that order, every multi-byte integer
/// little-endian — folded a word at a time instead of a byte at a time.
///
/// Each call of [`Fold::bytes`] is one message. A message of at most eight
/// bytes is one word (its bytes little-endian in the low bytes, the rest
/// zero, its length in the top four bits) mixed once; a longer message is
/// its length as a word, then each full eight-byte word, then its tail word
/// tagged the same way. A word `w` is mixed as
/// `h = (h ^ w).wrapping_mul(K); h ^= h >> 32`: a bijection of `h` for any
/// `w` and of `w` for any `h`, so one changed word always changes the state,
/// the multiply carrying every low bit upward and the shift bringing the
/// high bits back down. [`Fold::finish`] applies splitmix64's finaliser so
/// every output bit depends on every input bit.
///
/// The hash is therefore a function of the *sequence of messages*, which is
/// what the callers' fixed order of fields makes it: a `u32` field is one
/// word whatever its value, a weapon name of nine bytes is its length and
/// two words. (FNV-1a 64 was a function of the concatenated bytes, which the
/// same fixed order made equally unambiguous; the difference is one
/// dependent multiply per word here against one per byte there, and the
/// VPS profile read the byte chain at 17 per cent of the CI step.) The
/// constants are written out here so the answer does not move when the
/// toolchain does.
#[derive(Clone, Copy, Debug)]
pub struct Fold {
    h: u64,
}

/// The fold's seed: FNV-1a 64's offset basis, kept for familiarity.
pub const FOLD_SEED: u64 = FNV_OFFSET_BASIS;
/// The word multiplier: the 64-bit golden-ratio constant (odd).
pub const FOLD_K: u64 = 0x9e37_79b9_7f4a_7c15;

impl Default for Fold {
    fn default() -> Self {
        Self::new()
    }
}

impl Fold {
    /// An empty fold.
    #[must_use]
    pub const fn new() -> Self {
        Self { h: FOLD_SEED }
    }

    #[inline]
    const fn mix(h: u64, w: u64) -> u64 {
        let h = (h ^ w).wrapping_mul(FOLD_K);
        h ^ (h >> 32)
    }

    /// One word of at most eight bytes, little-endian, the rest zero.
    #[inline]
    fn word(bytes: &[u8]) -> u64 {
        match *bytes {
            [a] => u64::from(a),
            [a, b] => u64::from(u16::from_le_bytes([a, b])),
            [a, b, c, d] => u64::from(u32::from_le_bytes([a, b, c, d])),
            [a, b, c, d, e, f, g, i] => u64::from_le_bytes([a, b, c, d, e, f, g, i]),
            _ => {
                let mut word = [0_u8; 8];
                let n = bytes.len().min(8);
                word[..n].copy_from_slice(&bytes[..n]);
                u64::from_le_bytes(word)
            }
        }
    }

    /// Fold one message.
    #[inline]
    pub fn bytes(&mut self, bytes: &[u8]) {
        if bytes.len() <= 8 {
            // The length rides in the top four bits of the word, so a byte and
            // the same byte zero-padded to a word are two messages.
            self.h = Self::mix(self.h, Self::word(bytes) ^ ((bytes.len() as u64) << 60));
            return;
        }
        self.h = Self::mix(self.h, bytes.len() as u64);
        let (words, tail) = bytes.as_chunks::<8>();
        for word in words {
            self.h = Self::mix(self.h, u64::from_le_bytes(*word));
        }
        if !tail.is_empty() {
            self.h = Self::mix(self.h, Self::word(tail) ^ ((tail.len() as u64) << 60));
        }
    }

    /// The hash of every message folded so far; the fold itself is
    /// unchanged, so a caller may read it and fold on.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        let mut h = self.h;
        h ^= h >> 30;
        h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
        h ^ (h >> 31)
    }
}

/// **A value folded field by field** (16c `T-104`): the pending notices
/// and the pending intents were folded as their canonical
/// `serde_json::to_vec` bytes, so that every field counted without a
/// hand-written fold a new field could miss. This keeps that property — it
/// is serde's own walk of the derive, every field and every variant, in
/// declaration order — and drops the text: a scalar folds as its
/// little-endian bytes, a string or byte slice as its length then its bytes,
/// an option as a presence byte then the value, a sequence or map as its
/// length (`u64::MAX` when unknown) then its items, an enum as its variant
/// index then its payload, a struct as its field count then its fields. The
/// VPS profile read the JSON at 3 per cent of the CI step.
pub(crate) fn fold_value<T: serde::Serialize + ?Sized>(fold: &mut Fold, value: &T) {
    // The serializer itself cannot fail; a `Serialize` impl that errors on
    // its own (none of ours do) folds what it folded before the error.
    let _ = value.serialize(FieldFold { fold });
}

/// The serializer behind `fold_value`: every call folds bytes and returns
/// `Ok(())`.
struct FieldFold<'a> {
    fold: &'a mut Fold,
}

/// [`FieldFold`]'s error type, never produced by the serializer itself.
#[derive(Debug)]
struct FoldError;

impl std::fmt::Display for FoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a value refused to serialize into the fold")
    }
}

impl std::error::Error for FoldError {}

impl serde::ser::Error for FoldError {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        FoldError
    }
}

impl FieldFold<'_> {
    fn len(&mut self, len: Option<usize>) {
        self.fold
            .bytes(&len.map_or(u64::MAX, |n| n as u64).to_le_bytes());
    }

    fn index(&mut self, index: u32) {
        self.fold.bytes(&index.to_le_bytes());
    }
}

impl<'a> serde::Serializer for FieldFold<'a> {
    type Ok = ();
    type Error = FoldError;
    type SerializeSeq = FieldFold<'a>;
    type SerializeTuple = FieldFold<'a>;
    type SerializeTupleStruct = FieldFold<'a>;
    type SerializeTupleVariant = FieldFold<'a>;
    type SerializeMap = FieldFold<'a>;
    type SerializeStruct = FieldFold<'a>;
    type SerializeStructVariant = FieldFold<'a>;

    fn serialize_bool(self, v: bool) -> Result<(), FoldError> {
        self.fold.bytes(&[u8::from(v)]);
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i128(self, v: i128) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<(), FoldError> {
        self.fold.bytes(&[v]);
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u128(self, v: u128) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_bits().to_le_bytes());
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<(), FoldError> {
        self.fold.bytes(&v.to_bits().to_le_bytes());
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<(), FoldError> {
        self.fold.bytes(&u32::from(v).to_le_bytes());
        Ok(())
    }

    fn serialize_str(mut self, v: &str) -> Result<(), FoldError> {
        self.len(Some(v.len()));
        self.fold.bytes(v.as_bytes());
        Ok(())
    }

    fn serialize_bytes(mut self, v: &[u8]) -> Result<(), FoldError> {
        self.len(Some(v.len()));
        self.fold.bytes(v);
        Ok(())
    }

    fn serialize_none(self) -> Result<(), FoldError> {
        self.fold.bytes(&[0_u8]);
        Ok(())
    }

    fn serialize_some<T: serde::Serialize + ?Sized>(self, v: &T) -> Result<(), FoldError> {
        self.fold.bytes(&[1_u8]);
        v.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), FoldError> {
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), FoldError> {
        Ok(())
    }

    fn serialize_unit_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
    ) -> Result<(), FoldError> {
        self.index(index);
        Ok(())
    }

    fn serialize_newtype_struct<T: serde::Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), FoldError> {
        v.serialize(self)
    }

    fn serialize_newtype_variant<T: serde::Serialize + ?Sized>(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        v: &T,
    ) -> Result<(), FoldError> {
        self.index(index);
        v.serialize(self)
    }

    fn serialize_seq(mut self, len: Option<usize>) -> Result<Self, FoldError> {
        self.len(len);
        Ok(self)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self, FoldError> {
        Ok(self)
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, FoldError> {
        Ok(self)
    }

    fn serialize_tuple_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, FoldError> {
        self.index(index);
        Ok(self)
    }

    fn serialize_map(mut self, len: Option<usize>) -> Result<Self, FoldError> {
        self.len(len);
        Ok(self)
    }

    fn serialize_struct(mut self, _: &'static str, len: usize) -> Result<Self, FoldError> {
        self.len(Some(len));
        Ok(self)
    }

    fn serialize_struct_variant(
        mut self,
        _: &'static str,
        index: u32,
        _: &'static str,
        len: usize,
    ) -> Result<Self, FoldError> {
        self.index(index);
        self.len(Some(len));
        Ok(self)
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

impl serde::ser::SerializeSeq for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_element<T: serde::Serialize + ?Sized>(&mut self, v: &T) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeTuple for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_element<T: serde::Serialize + ?Sized>(&mut self, v: &T) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleStruct for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_field<T: serde::Serialize + ?Sized>(&mut self, v: &T) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleVariant for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_field<T: serde::Serialize + ?Sized>(&mut self, v: &T) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeMap for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_key<T: serde::Serialize + ?Sized>(&mut self, key: &T) -> Result<(), FoldError> {
        key.serialize(FieldFold { fold: self.fold })
    }

    fn serialize_value<T: serde::Serialize + ?Sized>(&mut self, v: &T) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeStruct for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

impl serde::ser::SerializeStructVariant for FieldFold<'_> {
    type Ok = ();
    type Error = FoldError;

    fn serialize_field<T: serde::Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), FoldError> {
        v.serialize(FieldFold { fold: self.fold })
    }

    fn end(self) -> Result<(), FoldError> {
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// **How many times this thread has taken [`state_hash`]** (the M.3
    /// review's witness for 16c `T-23`), in test builds only: the lazy phase-4
    /// slot (`Arena::action_hash`) is closed again before a pulse returns, so
    /// whether phase 4 took a hash for nothing is visible only as a count. No
    /// outcome reads it, and it does not exist outside `cfg(test)`.
    pub(crate) static STATE_HASHES_TAKEN: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

/// **The state hash** of an arena, as the ledger records it.
///
/// [`Fold`] (hash v2, 16c `T-104`; FNV-1a 64 until then) over these bytes, in
/// this order, every multi-byte integer **little-endian**:
///
/// 1. For each unit, **in unit-id order** (the order [`Arena::units`] is kept
///    in):
///    1. the object number, `u32`;
///    2. `pose.x_cm`, `i64`;
///    3. `pose.y_cm`, `i64`;
///    4. `pose.z_cm`, `i32`;
///    5. `pose.heading_mdeg`, `u32`;
///    6. `speed_cms`, `i32`;
///    7. the power byte (`0` off, `1` starting, `2` online);
///    8. **the five limit bytes** (16b cut M.4, 16c `B-47`; four until then, when
///       one byte named at most one closed side): `slow_pct`; `no_turn_left`
///       (`0`/`1`); `no_turn_right` (`0`/`1`); `motion_disabled` (`0`/`1`);
///       `wreck` (`0`/`1`);
///    9. **what it is held against** (16b cut M.3, 16c `T-04`):
///       [`crate::arena::state::UnitState::held`] as a presence byte, then the
///       held cell's `x` and `y` as `u16` and the cause as one byte
///       ([`crate::arena::motion::BlockedCause::code`]: `0` impassable, `1`
///       occupied, `2` edge, `3` too steep) — `[0]` alone for none;
///    10. **the lock** (16b cut W.1): the contact id as `u64`, or `0` for none, which
///        is a value [`crate::arena::rng::draw_contact_id`] never mints;
///    11. **the weapon cells** (16b cut W.1), in the two `BTreeMap`s' own key order:
///        the count of `ready_at_ms` rows as `u64`, then per row the weapon id's
///        bytes and its `i64`; then the same for `ammo` with its `u32`. The id's
///        bytes go in because a row is a `(weapon, value)` pair and folding only the
///        values would make two weapons that swapped cycles one state;
///    12. **the controller byte** (16b cut N.1): `0` pilot, `2` mind (17b cut
///        M1a.5); `1` was the reflex controller's and is retired, never reused
///        (17b cut M4.6 deleted it and renumbered nothing);
///    13. **`grade_pct`** (16b cut X.6), `u16` — the uphill slope of the edge the
///        unit entered its cell by;
///    14. **the traffic hold** (16b cut E.3): a presence byte, then the held
///        cell's two `u16` and the `i64` arena millisecond it opened at;
///    15. **the motion carries** (16b cut Q.6), in field order: `carry.turn`
///        as `u32`, `carry.speed` as `u32`, `carry.east` as `i64` and
///        `carry.north` as `i64`; then (16b cut M.4, 16c `T-16`) the two sign
///        bytes `carry.turn_side` and `carry.speed_rate` in one call — which way
///        the turn carry was banked and by which rate the speed carry was, so a
///        replay about to zero a carry on a flip differs from one that is not;
///    16. **the platform's own receiver** (16b cut D.4): a presence byte, then
///        `budget` and `budget_max` as **one byte each** (the fold writes
///        `[1, budget, budget_max]` in one call), then `reach_permille` as `u16`
///        (16b cut M.3 corrected this line, which typed all three `u16`; 16c
///        `T-80`);
///    17. **the seat-order rate** (16b cut M.1, `T-82`): `orders_window_s` as
///        `i64`, then `orders_in_window` as `u32`. Persistent state that decides a
///        refusal notice is hashed: a replay whose pilot's order landed in a
///        different arena second would refuse a different later one;
///    18. **heat** (16b cut M.3, 16c `T-06`): a presence byte — `1` when
///        [`crate::arena::state::UnitState::heat_now`] answers — then that heat
///        as `u32` and [`crate::arena::state::HeatState::carry_milli`] as `u32`;
///        `[0]` alone for a unit that carries no heat, which is every stock tank;
///    19. **the pod's switch** (16b cut O.2, 16c N12): one byte,
///        [`crate::arena::state::UnitState::ecm_on`] (`1` radiating, `0`
///        silent), written for every unit whether it carries a pod or not. It
///        decides what every sight inside the pod's bubble sees — detection and
///        both ends of the mesh — and it is authoritative state with one writer
///        ([`crate::arena::intents::resolve_emcon`]), so a replay whose pilot's
///        switch landed a pulse later must not compare equal even before the
///        next pass shows the difference in a contact;
///    20. **the active profile** (17b cut M5.2; doc 17 §11, §12, D-UM-44):
///        [`crate::arena::state::UnitState::card_profile`] — `[1]`, then
///        `index` as `u8` and `switched_ms` as `i64` (`i64::MIN` for none, a
///        millisecond no arena reaches: a stamp-less index past the first is
///        no state a switch writes) — and **nothing at all for a unit at its
///        first profile and never switched**
///        ([`crate::arena::card::ActiveProfile::FIRST`]), so every match that
///        switches nothing — every production match, every fixture before
///        this cut — folds exactly the bytes it folded before it. The one
///        sub-item of item 1 that writes no byte for its default (where the
///        others always write at least one): items 3b to 3e's "no byte for
///        none" rule applied inside the unit's row. The whole default is the
///        condition — index `0` AND no stamp — so a switch back to the first
///        profile, which keeps its stamp, still folds. Both fields decide a
///        later card and a refusal (the seat window's reason, sub-item 17).
/// 2. **The spent-section window** (16b cut D.2), in the map's own key order: the
///    count of entries as `u64`, then per entry the platform told's `u32`, the
///    target's `u32`, the section id's bytes and the arena millisecond as `i64`.
/// 3. **The reflex machines** (16b cut N.1) — **deleted with the reflex
///    controller in 17b cut M4.6.** Item 3 folded one row per reflex machine in
///    unit-id order with **no count prefix** — its state byte, its target, its
///    standing order (with **16b cut Y.1**'s issuer, `0` for none, and **16b cut
///    Z.2**'s role payload), its endgame clocks and its search and hunt fields,
///    with the hull's motor row folded inside it — so an arena with no reflex row,
///    every match since 17b cut M4.3 put minds on the empty pods, folded no byte
///    for it, and its deletion moved the hash of no such match. The number is
///    kept, so 3b, 3c and 3d keep theirs. (A mind's order issuer, owner's STOP C
///    ruling, is a record field and is never folded: see "Never folded" below.)
///
///    **Item 3b, the motor rows** (17b cut M1a.1; doc 17 §4, D-UM-3): every
///    [`crate::arena::motor::MotorRow`] on [`Arena::motors`] — a pilot's `DRIVE`
///    (17b cut M1a.4) or a mind's (M1a.5) — in unit-id order: the unit's `u32`,
///    the goal's two `u16` (`u16::MAX` twice for none), the waypoint as `u16`
///    (`u16::MAX` for none), the route's length as `u64`, the avoided cells
///    (16b cut Q.7: their count as `u64`, then per entry the cell's two `u16` and
///    the stop's `i64` stamp), `hold_retry_ms` (16b cut O.1: a presence byte then
///    an `i64`), `searched_from` (16b cut AI.1: a presence byte, then the cell's
///    two `u16`) and `refused` (`[0]` alone for none; else `[1]`, the stand
///    cell's and the refused cell's two `u16` each, the cause's code byte, the
///    goal's two `u16` and the `i64` it lapses at). Per row and with **no count
///    prefix**, so an arena with no motor row adds no byte at all. A goal is a
///    decision that reaches a pose only substeps later, and `searched_from` and
///    `refused` each gate a later search with no line of their own — the reason
///    a row is folded at all.
///
///    Until 17b cut M4.6 item 3b skipped a hull with a reflex row, whose motor
///    row item 3 folded interleaved with the machine's own fields; with no
///    reflex row anywhere that guard skipped nothing, and it was deleted with
///    item 3, the fold unchanged. `hold_retry_ms` **stays folded** (critic §6.5
///    of 17b cut M4.5) although its only production writer — `follow`'s
///    `Fallback::Waypoint` branch — lost its only production passer with the
///    reflex's decision: dropping the byte would move the hash of every match
///    with a motor row.
///
///    **Item 3c, the unit minds** (17b cut M1a.5; doc 17 §3, §6, §8), right
///    after item 3b: every row of [`Arena::minds`], in unit-id order and with
///    **no count prefix** (so an arena with no mind adds no byte — every
///    match before 17b cut M4.3, which fielded minds; every production match
///    since folds a row per hull a mind flies, and a dormant row per hull a
///    pilot possessed): the unit's `u32`; `think_seq` as `u64`; `pending` as
///    `[0]`, or `[1]` then its `think_seq` `u64` and its `t_ms` `i64` (and,
///    since 17b cut M3.6, its answer: below); the
///    memory's length as `u64` then its bytes (its FNV-1a 64 digest as `u64`
///    until 16c `T-104`); the
///    notices' count as `u64`, then per notice **`fold_value`'s walk of
///    its fields** (its canonical `serde_json::to_vec` bytes until 16c
///    `T-104`) — every field, so two buffers that would
///    drive two different decisions (two `Miss` notices differing only in
///    their reason, a `Struck`'s figures, a `was`, a section) never fold the
///    same; the range memory's count as `u64`, then per entry the
///    `ContactId`'s `u64` and the figure's `u64`; the order as `[0]`, or
///    `[1]`, the objective's code byte (`MindObjective::code`) and its payload
///    (`Advance`'s two `u16`, `Engage`'s target as `u64` or `0`, `Screen`'s
///    facing as `u32`, `Support`'s charge as `u64` or `0` and — since 17b
///    cut M5.2, STOP E — the charge's unit
///    ([`crate::arena::mind::MindState::order_charge`]) as `u32` or `0`,
///    nothing for the other two), the `hold_fire` byte, the band byte (`0`
///    none, `1` close, `2` effective, `3` far), `expires_ms` as `i64` and the
///    squad word's bytes;
///    then `strikes`, one byte. Every field decides a later intent — a notice
///    a mind reads, a range it holds, an order it obeys — and none of them has
///    a ledger line of its own, so a replay that differed in one would
///    otherwise compare equal until the difference reached a pose.
///
///    **Since 17b cut M1b.1** each row ends, after `strikes`, with **the
///    card's bytes** ([`crate::arena::card::StockCard`], doc 17 §11): its
///    fields in declaration order — an enum as its code byte, a bool as one
///    byte, an integer little-endian, no separator — because the card is what
///    a mind thinks under and a later sub-cut's runner reads it.
///
///    **Since 17b cut M3.6** (D-UM-23 as amended, "hashed whole"; doc 17
///    §13's list; the cut's one re-bless, which moved
///    `M2_CROSS_PLATFORM_PINS`), carried since 17b cut M3.2 and folded here:
///    `pending`, when present, folds after its `[1]`, `think_seq` and `t_ms`
///    its ANSWER — the three epochs ([`crate::arena::mind::PendingThink`]'s
///    `epochs`, `Channel::ALL`'s order) as `u64` each, the intents' count as
///    `u64` then each intent by `fold_value` (its canonical
///    `serde_json::to_vec` bytes until 16c `T-104`), the new
///    memory's length as `u64` then its bytes (its FNV-1a 64 digest until
///    16c `T-104`); and after
///    `strikes`, before the card, the row's `dropped` as `[0]`, or `[1]` then
///    the cause's byte (strikes `1`, core `2`, abi `3`), then `seal_digest`'s
///    32 bytes. The answer had to be hashed whole because it is NOT a
///    function of the rest of the folded state: the think read notices
///    staged and drained inside one substep, which no substep hash folds (doc
///    17 §6; 17b cut M3.2's gate fixture
///    `m3_a_resume_between_n_and_n_plus_one_reproduces_the_hash_sequence_at_three_cadences`
///    turns on exactly that), so before this cut two runs differing only in a
///    pending answer hashed equal until its landing — the declared gap 17b
///    cuts M3.2 to M3.5 carried, closed here. A dormant row (17b cut M3.5, a
///    possessed hull's) folds the same bytes as any row; the in-source probe
///    `m3_every_part_of_the_answer_the_drop_and_the_seal_digest_moves_the_hash`
///    moves each part.
///
///    **Since 17b cut M4.1** (doc 17 §6 "Execution policy", D-UM-22; the
///    fold code unchanged) a think is collected at its millisecond and its
///    answer written at the head of the next substep, so the substep a
///    think falls in folds its `pending` as the FACTS — `[1]`, `think_seq`,
///    `t_ms`, the three epochs — with an EMPTY answer (`0` intents, a
///    `0`-length memory and the empty digest), and the answer is folded
///    from the next substep on, until the landing takes it off the row.
///    Whether a row's pending answer is written is the scratch set
///    [`Arena::awaiting`], never folded and never carried (an empty memory
///    is a legal clean answer); `Snapshot::encode` refuses an arena that
///    holds any such row by name. The mind-fixture pins over matches that
///    carry a mind (`M2_CROSS_PLATFORM_PINS`, the twin lanes' coverage and
///    record) moved once for this; no production pin did.
///
///    **Never folded**: [`crate::arena::state::UnitState::seal`] (17b cut
///    M3.2's `SealRef` — a per-match constant, the floor's argument below,
///    whose `closed` moves only with a ledger line, 17b cut M3.6's
///    `sortie_closed`, which the ledger's sequence folds — and, since 17b cut
///    M4.6, its `doctrine` bit, derived from the seal whose digest item 3c
///    folds, and since 17b cut M5.2 its `profiles`, the sealed set, likewise)
///    and
///    [`Arena::classes`] (17b cut M3.1's class table, captured at check-out
///    and carried from 17b cut M3.2: a per-match constant, likewise), and
///    [`crate::arena::mind::MindState::doctrine_state`] (17b cut M4.3: a
///    cache over the memory bytes item 3c folds, re-derived at the decode),
///    and [`crate::arena::mind::MindState::order_issuer`] (17b cut M4.6, the
///    owner's STOP C ruling of 2026-10-09: a record field, as `Order.by` is —
///    the hint is re-keyed once, at write time, in the issuer's scope, so the
///    issuer steers nothing after the order lands; carried by the snapshot,
///    read only by `Arena::live_orders_by_issuer`. The reflex rows' issuer,
///    item 3's 16b cut Y.1 fold, was folded because the reflex re-keyed its
///    hint at every tick, and went with item 3).
///
///    **Item 3d, the condition slots** (17b cut M1b.1; doc 17 §5, D-UM-16),
///    right after item 3c: every row of [`Arena::conditions`], in unit-id
///    order and with **no count prefix** (an arena with no slot — every
///    production match — adds no byte): the unit's `u32`, then per ARMED slot
///    its 1-based number `u8`, the weapon id's bytes, the target as `[0]`
///    (lock), `[1]` then the contact's `u64` (contact) or `[2]` (best hostile),
///    `min_permille` `u16`, the aspect mask `u8`, `min_rounds_left` `u32`,
///    `min_range_m` `u32` and `max_range_m` `u32`. A slot decides a shot at a
///    later millisecond with no ledger line of its own until it fires.
///
///    **Item 3e, the held-shot rows** (17b cut M1b.1), right after 3d: every
///    entry of [`Arena::held_said`] in key order, per row with no count prefix
///    — the unit's `u32`, the slot `u8`, the contact's `u64` and the stamp
///    `i64` — because a stamp decides whether a later held sentence is said.
///
///    **[`Arena::odds_floor_permille`] is NOT folded** (17b cut M1b.1): it is a
///    per-match constant captured at check-out, carried by the snapshot, and
///    read by nothing a production match runs before 17b cut M4 — folding it
///    would move the production hash of every match for a value no production
///    match reads. Two arenas differing only in the floor hash equal (the
///    in-source probe pins it).
/// 4. **The shadow contact store** (16b cut S.1), in its own iteration order —
///    viewer object number ascending, then contact id ascending, which is what
///    the two `BTreeMap`s give: per contact the viewer's `u32`, the contact id's
///    `u64`, the target's `u32`, the class byte (`0` unknown, `1` classified red,
///    `2` classified blue), `first_seen_ms` and `last_seen_ms` as `i64`; then the
///    plot's five integers, the own sighting (a presence byte, its stamp, its own
///    plot and **its reading**) and **the reading the platform holds** — a reading
///    being a presence byte, the look's `i64` stamp, the packed bands as `u32` and
///    the packed section count as `u8` (16b cut Q.2); then **whether this hold was
///    opened under a provisional number** as one byte (16b cut D.4).
/// 5. **The designator book** (16b cut Q.1; a digest since 16b cut M.3, 16c
///    `T-27`): the book's running digest
///    ([`crate::arena::state::DesignatorBook::digest`]) as `u64`, then the count
///    of issued numbers as `u64`. Until M.3 this walked both of the book's maps
///    row by row in every substep hash — a monotonic table, so the walk only ever
///    grew. The digest is an FNV-1a 64 fold of every *mint* in the order it was
///    made (the viewer's `u32`, the target's `u32`, the number's `u16`), so it
///    is a function of the whole sequence of mints: two books that minted the
///    same numbers for the same pairs in the same order agree, and (short of a
///    64-bit collision) any two that did not disagree. That tells apart
///    everything the walk could — the book is append-only, a re-assignment of a
///    held pair mints nothing, and the `next` counters are themselves a function
///    of the mints — and one thing more, the interleaving of two viewers' mints,
///    which is itself a function of arena time (the passes that mint run in
///    `(t_ms, seq)` order), so no replay the walk called equal is told apart
///    here by anything but the order the arena made its numbers in. The count
///    beside it is a cheap second witness.
/// 6. **The held losses** (16b cut M.2), in the map's own key order: the count of
///    entries as `u64`, then per entry the viewer's `u32`, the contact id's `u64`
///    and the deadline as `i64`. The designator in the value is not folded.
/// 7. The tempo byte (`0` quiet, `1` alert, `2` engaged).
/// 8. `now_ms`, `i64`.
/// 9. The due-event count, `u64`.
/// 10. **The in-flight shot count** (16b cut W.1), `u64`.
/// 11. The ledger's event sequence, `u64`.
/// 12. **The mode and the Hold objective** (16b cut O.1; 16c N11): the mode's
///     code byte ([`crate::arena::state::MatchMode::code`]: `0` skirmish, `1`
///     hold, `2` range — 16b cut O.3's arm rides this one byte and adds none:
///     a range carries no objective, so its hold is `[0]`); then the hold as a
///     presence byte, the spot's `x` and `y` as `u16`,
///     `radius_cells` as `u16` and `hold_ms` as `u32`, then `held_by` as a
///     presence byte and the team byte (`0` red, `1` blue) in one call, then the
///     `i64` arena millisecond it opened at — `[0]` alone for no hold, and `[0]`
///     alone for a hold nobody holds.
///
/// **Why the Hold clock is in it** (16b cut O.1), under cut D.4's one rule —
/// *persistent state that decides an end is hashed*. `held_by` moves with **no
/// ledger line of its own** (opening, dropping and re-opening the clock journal
/// nothing: it reaches no pilot), and it decides the match: a replay that saw
/// the sole occupant one substep earlier would win one substep earlier, and
/// until then nothing else in here would differ. Folding it catches that at the
/// substep it happened. The `HoldSettles` event it schedules is folded by the
/// due count (item 9). The motor rows' `hold_retry_ms` (item 3b; the reflex
/// machines', item 3, until 17b cut M4.6) is the same argument read for a
/// controller: a decision with no line of its own that decides a later goal.
///
/// **Why the contacts are in it** (16b cut S.1). Doc 16 §3.1 makes the arena the
/// single writer for *unit poses, unit condition cells, contacts, orders,
/// scheduled impacts, the match ledger*, and the hash exists to catch a tempo that
/// changed an outcome. Detection, classification and loss are journalled, so the
/// ledger sequence already moves with each of them — but a contact's `last_seen_ms`
/// advances on every refresh with no event at all, and a replay that refreshed a
/// contact one pass earlier would otherwise compare equal. The store is folded in
/// its map order, which is deterministic by construction, so there is no iteration
/// order for a replay to diverge on.
///
/// **What is deliberately absent.** The *ordered* desired heading and speed are
/// not folded in: they are an input, and a pulse at one tempo may have applied a
/// helm order that a pulse at another has not yet — the replay compares what the
/// world *became*, not what it was told. The opaque `TacticalUnit` is not folded
/// in either; its truth is M32's and its every change is journalled there, so
/// hashing it would make this function a second, weaker copy of that journal.
/// (One cell of it is, since 16b cut M.3: the `heat` cell, item 1.18, because it
/// gates a refusal and moves every substep — see cut M.3's extensions below.)
///
/// **The staff tempo floor is not in it either** (16b cut I.3; 16c N10). It is
/// a view of how often the task wakes — `crate::arena::clock::Clock`'s
/// `floor_ms` and the floor at the previous wake — and the tempo decides no
/// outcome (D-16-2), so neither does a floor under it; every floored pulse is
/// quantised to the substep lattice so the positions a due event is applied
/// against never depend on it. **But its `tempo_floor` line moves the ledger's
/// sequence, item 11, and so every hash after it**, exactly as every line does:
/// a floored run hash-equals another run floored at the same arena moment, and
/// never an unfloored one. A staff `@arena/place` adds no hashed field either:
/// it moves a pose, which items 1.2 to 1.4 already fold, and its `placed` or
/// `place_refused` line moves item 11 like any other.
///
/// **Why the machines are in it** (16b cut N.1). The controller writes no ledger
/// event of its own — what leaves it is the orders a pilot's verbs would have
/// staged, and those journal what they always did — so a replay whose machine
/// chose a different waypoint would compare equal for as many substeps as it took
/// the divergence to reach a pose. Folding the machine in catches it at the
/// substep it happened, which is the whole reason the hash sequence and not only
/// the final state is compared. What is folded is the *decision* and not the
/// route's cells: the cells are a function of the goal and the ground, and hashing
/// them would hash the window once per unit.
///
/// **Cut W.1's extension**, which that last sentence promised: the lock, the two
/// weapon cells and the in-flight shot count. Each is a fact the arena is the single
/// writer of and each moves without an event of its own — a cycle that has run down,
/// a lock cleared because its contact expired — so a replay that fired half a pulse
/// earlier would otherwise compare equal. The shot *record* is not folded in beyond
/// its count: the shot's own fields are a function of the poses and the range at
/// acceptance, which are already in here, and folding a predicted point would hash
/// the same numbers twice.
///
/// **Cut X.6's extension**: `grade_pct`, for the same argument. It is
/// authoritative state with exactly one writer
/// ([`crate::arena::motion::integrate`]'s accepted crossing), it moves without a
/// ledger event, and a unit can sit in one cell for many substeps after the
/// crossing that set it — so a replay that took a ramp a substep earlier would
/// compare equal until the difference in speed had accumulated into a whole
/// centimetre of pose. The *limit* beside it is deliberately not folded: it is
/// read once at check-out and never moves, and every refusal it causes is a pose
/// that is already in here.
///
/// **Cut Q.1's extension**: the designator book. It is the same argument the
/// contact store makes, one step further. A number is issued when a platform's
/// own sensors first insert an `Unknown` contact — and that insert journals an
/// `EventKind::Detected` which is **byte for byte the shape a mesh arrival
/// journals**, so the ledger's sequence moves identically whether a number was
/// issued or not. A replay whose blip was found by its own sight where the
/// original took it off the mesh would otherwise compare equal, and the two runs
/// would then show the pilot different names for the same tank. Folding the book
/// catches it at the substep it happened, which is the argument cut F.3 made for
/// the plot.
///
/// **Cut Q.2's extension**: the two readings on every contact. They are the
/// contact store's own argument again — a close look is taken on a pass and
/// journals nothing, a share hands one over and journals nothing, and the band a
/// gunner watched come apart is written in beside a `section_destroyed` that is
/// about the *victim* and would be byte-identical either way. A replay that got
/// close a pass earlier, or whose share arrived from a different look, would
/// otherwise compare equal and then show the two pilots different `SCAN`s.
///
/// The *unit's own* [`crate::arena::state::UnitState::condition`] is deliberately
/// **not** folded, and that is the other half of the same argument: it is a pure
/// function of the `TacticalUnit` beside it, which M32 journals, so folding it
/// would make this function a second and weaker copy of that journal — the reason
/// the opaque unit itself is absent.
///
/// **Cut Q.6's extension**: the four motion carries. They are the *fractional
/// part* of three quantities this hash already folds — the heading, the speed and
/// the pose — so leaving them out would be folding a number and hiding its low
/// bits. Each is authoritative state with exactly one writer
/// ([`crate::arena::motion::integrate`]), each moves with no ledger event of its
/// own, and two replays whose carries differ are two replays that are about to
/// disagree about a whole centimetre — several substeps later, when the drift has
/// accumulated, which is precisely the divergence the per-substep sequence exists
/// to catch at the substep it happened. **16b cut M.4** (16c `T-16`) adds the two
/// sign bytes beside them, `turn_side` and `speed_rate`: a carry is now zeroed on
/// the substep that turns the other way or changes rate, so which way it was
/// banked decides the next substep's heading and speed.
///
/// **What a shot in flight carries is still not folded** beyond the count, and
/// that includes 16b cut M.4's `Resolution::beyond` (16c `B-52`): it is a
/// function of the range at acceptance and the weapon's authored bands, both
/// fixed when the shot was accepted, exactly as the rest of the recorded
/// resolution is. So is **16b cut AE.2**'s launch point (`from_x_cm`,
/// `from_y_cm`, `from_cell`, `from_height_cm`): the shooter's pose and eye at
/// acceptance, which this hash folded as the shooter's own pose at the substep
/// the shot was accepted in, and which nothing writes again — the impact reads
/// it and the shot leaves the table.
///
/// The *callsign* on each unit is deliberately **not** folded: it is a pure
/// function of the check-out message, resolved once by `Arena::new` and never
/// moved, so two replays of one custody message agree on it by construction —
/// the same argument `slope_limit_pct` makes.
///
/// Neither is the transient `lost_designators` table, and the reason is not that
/// it is empty here: the record below is taken **before** the staging scan that
/// drains it, so in a substep where a contact expired the table is holding that
/// row's name at this very line. It is not folded because its contents are a
/// function of state this hash already folds — the expiry that filled it is the
/// contact store's own change, and the name it carries is read off the book and
/// the callsign — and because it never survives the substep it was filled in, so
/// nothing it holds can reach a later one. Folding it would add no information
/// and would make the hash a function of *where* the drain sits.
///
/// **Cut D.2's `spent_hits` is absent on exactly that argument**, and it is the
/// one other table the argument really does fit. The shots in it are a function
/// of state this hash already folds — the struck section's two cells, and the
/// `hit` line the ledger sequence counts — and it never survives the substep
/// `crate::arena::damage::Arena::apply_hit` filled it in, because the staging
/// scan drains it. It carries the same caveat word for word: the record below is
/// taken *before* that scan, so in a substep where a round found a spent section
/// the table is holding that shot at this very line, and that costs nothing.
///
/// **Cut U.2's `jam_fold` is absent too, on a plainer argument** (16c `T-29`):
/// [`crate::arena::state::Arena::jam_fold`] is a memo of
/// [`crate::arena::Arena::jamming_at`], a pure function of the poses, the power
/// bytes and the wreck bytes item 1 already folds, and it is cleared at every
/// point inside a substep where one of those moves. Folding a cache would add no
/// information and would make the hash a function of which folds had been asked
/// for by the time the record was taken.
///
/// **Cut J.2's horizon index is absent on the same argument, and a plainer one
/// still**: it lives inside [`crate::arena::state::ArenaTerrain`] as a function
/// of the cells the definition digest already covers, built with them and never
/// written, so folding it would fold the terrain a second time.
///
/// **Cut Y.4's `at_release` and `pending_catchup` are absent on `last_frames`'
/// argument**: they are what a pilot will be told about a drop, written only by
/// a control that names a pilot and read only by the frame builder, so folding
/// them would make the hash a function of who was connected — and nothing that
/// steers reads them, so they cannot move a later substep either.
///
/// **Cut I.4's `watched` and `last_observer_s` are absent beside
/// `last_frames`**: a fact about who is watching and not about the match. They
/// gate the staff picture phase 5 builds at most once an arena second, are
/// written only by the task's `Watch` and by that gate, and are read by nothing
/// that steers, so folding them would make the hash a function of whether a
/// staffer was looking — and a watched match must hash-equal an unwatched one
/// (`crates/tacsim/tests/arena.rs`, `i4_a_watched_match_is_the_same_match`).
///
/// **Cut D.2's `spent_said` is the opposite case, and it IS folded.** It persists
/// for [`crate::arena::state::SPENT_NOTICE_MS`] across substeps and pulses and it
/// decides what a *later* substep says, so `lost_designators`' argument is simply
/// false of it: a replay whose first spent round landed one substep earlier would
/// suppress a different later one, and the two runs would then show the two
/// pilots different lines. Folding it catches that at the substep it happened,
/// which is the argument cut Q.1 made for the designator book. Two rules keep the
/// fold rate-independent, and both are `Arena::spent_said`'s: the decision and
/// the write happen whether or not anybody is seated and whether or not a frame
/// is ever sent, and entries are pruned only on touch, against the event's own
/// timestamp, never by a per-pulse sweep. Because the staging scan runs **after**
/// the substep's hash is recorded, a write at substep *k* is first folded at
/// *k + 1*.
///
/// **Cut D.4's extension**: two more, under the same one rule — *persistent state
/// that decides a notice is hashed*.
///
/// [`crate::arena::state::ShadowContact::numbered`] moves with **no
/// distinguishable ledger event**: a contact opened by this platform's own
/// sensors and a contact handed over by the mesh both journal an
/// `EventKind::Detected` of byte-identical shape, so a replay whose blip was
/// found by its own sight where the original took it off the mesh would compare
/// equal — and the two runs would then say the `identified` line in one and not
/// the other. That is cut Q.1's own argument for the designator book, one hold
/// further in.
///
/// [`crate::arena::state::UnitState::receiver`] persists between passes and
/// decides a notice, and it can move with no ledger line at all: a platform that
/// stops running passes (a wreck, a shot-off sight) writes `None` silently, which
/// is a deliberate silence and not an omission. Folding it catches that at the
/// pass's own timestamp. The `jammer` and `eccm` beside it stay out for the
/// reason they always did: those are facts about the unit's fit that nothing in a
/// match moves.
///
/// **Cut E.2's extension**, under the same one rule.
/// [`crate::arena::state::ShadowContact::known_dead`] is what the shooter's own
/// machine reads to drop a target it killed, and it moves with **no ledger line
/// that names the holder**: a `destroyed` event carries the victim and the
/// shooter, never the contact row the fact was written onto, so a replay that
/// credited one shooter's kill to another platform's hold would compare equal
/// and the two runs would then steer two different tanks. Folding it catches
/// that at the substep it happened. Because the staging scan runs *after* the
/// substep's hash is recorded, a kill at substep *k* is first folded at
/// *k + 1*, exactly as `spent_said`'s writes are.
///
/// [`crate::arena::state::UnitState::fatal_sections`] beside it is deliberately
/// **not** folded: it is laid down once by [`Arena::new`] off the check-out
/// record and never moves, which is `slope_limit_pct`'s and `jammer`'s argument
/// word for word.
///
/// **Cut E.3's extension**: [`crate::arena::state::UnitState::occupied_hold`], on
/// `grade_pct`'s argument exactly. It is authoritative state with one writer
/// ([`crate::arena::motion::integrate`]), it decides a *later* refusal — whether
/// the next crossing this unit is refused is a hold or doc 16 §3.4 step 4's stop
/// — and it moves with **no ledger line at all**, because a hold is by
/// construction the substep that journals nothing. A replay that opened its hold
/// one substep earlier would stop one substep earlier two seconds later, and
/// nothing else in here would differ until the difference had accumulated into a
/// whole centimetre of pose. The cell and the stamp both go in: a hold against a
/// different cell is a different hold, and a hold that started earlier matures
/// earlier.
///
/// **Cut M.2's extension** (16c `T-83`): [`Arena::held_losses`], under cut D.4's
/// one rule. It was kept out while a sweep at the end of every pulse said the
/// expired entries, because folding a table a pulse boundary moved would have
/// made the hash a function of the tempo; since the expiry is a
/// [`crate::arena::state::DueKind::LossSettles`] at the deadline's own
/// millisecond, nothing but an arena timestamp moves it, and it decides whether a
/// later substep says `lost` — a replay whose contact was re-detected a pass
/// later would otherwise compare equal until the two pilots were shown
/// different lines. It is written only in the staging scan (the `lost` arm
/// inserts, the `detected` arm cancels, the settle removes), which runs after
/// the substep's hash is recorded, so an entry written at substep *k* — and the
/// due event scheduled beside it, which the due count folds — is first folded at
/// *k + 1*, exactly as `spent_said`'s writes are. The designator in the value is
/// not folded: it is a name derived from the book and the callsign, both of
/// which are already in here.
///
/// **Cut M.3's extensions** (16c `T-04`, `T-06`), under cut D.4's one rule.
///
/// [`crate::arena::state::UnitState::held`] decides, since that cut, the
/// ledger's `blocked` line, the pilot's `Held:` notice and a machine's avoid
/// list — all three fire on its new edge — and it moves with no line of its own
/// when it clears (a reverse off the bank, a crossing into another cell, the
/// order coming off). A replay that cleared it a substep earlier would journal
/// a second stop where the original said nothing, so the cell and the cause
/// both go in: a different cell or a different cause is a different condition,
/// and a new edge. The per-substep `blocked` flag beside it stays out — it
/// journals nothing now, and it is what *this* substep did rather than state a
/// later one reads.
///
/// **Heat** gates firing (a platform at its ceiling refuses `FIRE`) and moves on
/// every substep with no ledger line at all — the dissipation is a rate over
/// arena time, carried in thousandths ([`crate::arena::heat`]'s head) — which is
/// cut Q.6's argument for the motion carries, word for word: a replay that shed
/// its heat a substep sooner would compare equal until the difference refused a
/// shot. The cell's value and the carry both go in, because the carry is the
/// cell's fractional part.
///
/// **What the staging scan writes after the record** (16c `T-09`, written down
/// by 16b cut M.3). The record below is taken before the substep's staging scan,
/// and the scan writes hashed state of its own — among it: `spent_said` and
/// `known_dead` (their paragraphs above); a gunner's observed band — when a
/// classified target's section is destroyed by the gunner's own round, the
/// scan's `section_destroyed` arm writes that section's band `destroyed` into the
/// reading the gunner's platform holds
/// ([`crate::arena::state::ShadowContact::seen_condition`], through
/// `Arena::observe_section_lost`); the lock a `lost` contact clears (the `lost`
/// arm sets the viewer's `lock` — per-unit item 10 — to none); and
/// `held_losses` (item 6), with the `LossSettles` due event the `lost` arm
/// schedules beside an entry, which the due count (item 9) folds — the M.2
/// paragraph above. Every one of them is first folded at *k + 1*, and each is
/// rate-independent for the reason `spent_said` is: the scan runs at the same
/// point of every substep at every cadence, off the journal entries of that
/// substep. The list is not a checklist: a write added to the scan is folded a
/// substep late by construction, whether or not it is named here.
///
/// **What a snapshot keeps of all this** (16b cut AF.1; 16c N16;
/// `crate::arena::snapshot`). What the snapshot carries is every field this
/// function folds and every field that steers; what it re-derives is a
/// function of the definition (whose digest the document records) and of
/// compiled code (the locomotion table, like the slope rule — the document
/// records its digest too), or a cache of state it carries (the index, the
/// jamming memo, the cheapest cell per row); what it resets is a view or
/// per-pulse scratch. The decode's digest and hash checks and
/// `crates/tacsim/tests/arena.rs`' every-pulse resume are the proofs; a field
/// added to the state must go in one of the three lists in `snapshot.rs`, whose
/// exhaustive destructuring will not compile until it does. No fold here moved
/// for it.
#[must_use]
pub fn state_hash(arena: &Arena) -> u64 {
    #[cfg(test)]
    STATE_HASHES_TAKEN.with(|taken| taken.set(taken.get().saturating_add(1)));
    let mut fold = Fold::new();
    let fold = &mut fold;
    for unit in &arena.units {
        fold.bytes(&unit.id.0.to_le_bytes());
        fold.bytes(&unit.pose.x_cm.to_le_bytes());
        fold.bytes(&unit.pose.y_cm.to_le_bytes());
        fold.bytes(&unit.pose.z_cm.to_le_bytes());
        fold.bytes(&unit.pose.heading_mdeg.to_le_bytes());
        fold.bytes(&unit.speed_cms.to_le_bytes());
        fold.bytes(&[unit.power.code()]);
        fold.bytes(&[
            unit.limits.slow_pct,
            // 16b cut M.4 (`B-47`): the two sides are two bytes.
            u8::from(unit.limits.no_turn_left),
            u8::from(unit.limits.no_turn_right),
            u8::from(unit.limits.motion_disabled),
            u8::from(unit.limits.wreck),
        ]);
        // **16b cut M.3** (`T-04`): what it is held against — its own item after
        // the limit bytes (five since 16b cut M.4).
        match unit.held {
            Some(held) => {
                fold.bytes(&[1_u8]);
                fold.bytes(&held.x.to_le_bytes());
                fold.bytes(&held.y.to_le_bytes());
                fold.bytes(&[held.cause.code()]);
            }
            None => fold.bytes(&[0_u8]),
        }
        // 16b cut W.1: the lock and the two weapon cells.
        fold.bytes(&unit.lock.map_or(0_u64, |contact| contact.0).to_le_bytes());
        fold.bytes(&(unit.ready_at_ms.len() as u64).to_le_bytes());
        for (weapon, ready_at_ms) in &unit.ready_at_ms {
            fold.bytes(weapon.as_bytes());
            fold.bytes(&ready_at_ms.to_le_bytes());
        }
        fold.bytes(&(unit.ammo.len() as u64).to_le_bytes());
        for (weapon, rounds) in &unit.ammo {
            fold.bytes(weapon.as_bytes());
            fold.bytes(&rounds.to_le_bytes());
        }
        // 16b cut N.1: who is flying it.
        fold.bytes(&[unit.controller.code()]);
        // 16b cut X.6: the slope it is standing on.
        fold.bytes(&unit.grade_pct.to_le_bytes());
        // **16b cut E.3**: and what it is standing *against* — see this function's
        // head for why a hold is hashed where the slope limit beside it is not.
        match unit.occupied_hold {
            Some(hold) => {
                fold.bytes(&[1_u8]);
                fold.bytes(&hold.cell.0.to_le_bytes());
                fold.bytes(&hold.cell.1.to_le_bytes());
                fold.bytes(&hold.since_ms.to_le_bytes());
            }
            None => fold.bytes(&[0_u8]),
        }
        // 16b cut Q.6: the fractions of a turn, a speed change and a centimetre
        // that the last substep did not spend.
        fold.bytes(&unit.carry.turn.to_le_bytes());
        fold.bytes(&unit.carry.speed.to_le_bytes());
        fold.bytes(&unit.carry.east.to_le_bytes());
        fold.bytes(&unit.carry.north.to_le_bytes());
        // 16b cut M.4 (`T-16`): which way each of the first two was banked.
        fold.bytes(&[unit.carry.turn_side, unit.carry.speed_rate]);
        // **16b cut D.4**: what this platform's own last pass ran under.
        match unit.receiver {
            Some(receiver) => {
                fold.bytes(&[1_u8, receiver.budget, receiver.budget_max]);
                fold.bytes(&receiver.reach_permille.to_le_bytes());
            }
            None => fold.bytes(&[0_u8]),
        }
        // **16b cut M.1** (`T-82`): the seat-order rate's window and count — they
        // persist between pulses and decide a `queue_full` refusal.
        fold.bytes(&unit.orders_window_s.to_le_bytes());
        fold.bytes(&unit.orders_in_window.to_le_bytes());
        // **16b cut M.3** (`T-06`): heat, which gates firing and moves every
        // substep with no line — the cell's value and the carry that is its
        // fractional part, under one presence byte.
        match (unit.heat_now(), unit.heat.as_ref()) {
            (Some(now), Some(heat)) => {
                fold.bytes(&[1_u8]);
                fold.bytes(&now.to_le_bytes());
                fold.bytes(&heat.carry_milli.to_le_bytes());
            }
            _ => fold.bytes(&[0_u8]),
        }
        // **16b cut O.2** (16c N12): the pod's switch, which decides what every
        // sight inside its bubble sees.
        fold.bytes(&[u8::from(unit.ecm_on)]);
        // **17b cut M5.2** (sub-item 20): the active profile, only when it is
        // not the first never switched — no byte at all for the default, so
        // a match that switches nothing hashes as it did.
        if !unit.card_profile.is_first() {
            fold.bytes(&[1_u8, unit.card_profile.index]);
            fold.bytes(
                &unit
                    .card_profile
                    .switched_ms
                    .unwrap_or(i64::MIN)
                    .to_le_bytes(),
            );
        }
    }
    // **16b cut D.2**: the spent-section window, in the map's own key order.
    fold.bytes(&(arena.spent_said.len() as u64).to_le_bytes());
    for ((told, target, section), at_ms) in &arena.spent_said {
        fold.bytes(&told.0.to_le_bytes());
        fold.bytes(&target.0.to_le_bytes());
        fold.bytes(section.as_bytes());
        fold.bytes(&at_ms.to_le_bytes());
    }
    // (Item 3, 16b cut N.1's reflex machines — each row with its hull's motor
    // row folded inside it — was deleted with the reflex controller in 17b cut
    // M4.6. It folded per row with no count prefix, so an arena with no reflex
    // row, every match since 17b cut M4.3, folded no byte there.)
    //
    // **17b cut M1a.1** (item 3b): every motor row — a pilot's `DRIVE`, a
    // mind's — in unit-id order and with no count prefix, so a table that holds
    // none folds no byte here. Until 17b cut M4.6 it skipped a hull with a
    // reflex row (item 3 folded that hull's motor row); with no reflex row
    // anywhere the guard skipped nothing, and it went with item 3, the fold
    // unchanged.
    for (unit, motor) in arena.motors.rows() {
        fold.bytes(&unit.0.to_le_bytes());
        // The original requested goal and its retained completion decide later
        // thinks even after the active fanned goal has been cleared.
        let (rx, ry) = motor.requested_goal.unwrap_or((u16::MAX, u16::MAX));
        fold.bytes(&rx.to_le_bytes());
        fold.bytes(&ry.to_le_bytes());
        fold.bytes(&[match motor.outcome {
            None => 0,
            Some(crate::arena::perception::DriveOutcome::Arrived) => 1,
            Some(crate::arena::perception::DriveOutcome::NoRoute) => 2,
        }]);
        fold.bytes(&[u8::from(motor.fan.is_some())]);
        if let Some(fan) = motor.fan {
            for coordinate in [fan.origin.0, fan.origin.1, fan.assigned.0, fan.assigned.1] {
                fold.bytes(&coordinate.to_le_bytes());
            }
        }
        let (gx, gy) = motor.goal.unwrap_or((u16::MAX, u16::MAX));
        fold.bytes(&gx.to_le_bytes());
        fold.bytes(&gy.to_le_bytes());
        fold.bytes(&motor.waypoint.unwrap_or(u16::MAX).to_le_bytes());
        fold.bytes(&(motor.route.len() as u64).to_le_bytes());
        // **16b cut Q.7**: the cells it was stopped at, with the stamps that
        // age them out.
        fold_avoid(fold, &motor.avoid);
        // **16b cut O.1**: when a Hold spot no route reached may be tried
        // again — a decision with no line of its own, written and read at
        // arena timestamps. Kept folded after 17b cut M4.6 took its only
        // production writer's only passer (the reflex's decision): dropping
        // the byte would move the hash of every match with a motor row.
        fold_stamp(fold, motor.hold_retry_ms);
        // **16b cut AI.1** (16c `B-84`): the cell the last search started
        // from, and the stop the row remembers. Neither has a line of its
        // own, and each gates a later search.
        fold_search(fold, motor);
    }
    // **17b cut M1a.5** (item 3c): the unit minds, per row with no count
    // prefix — an arena with none folds no byte here (every match before 17b
    // cut M4.3; every production match since folds a row per mind).
    for (unit, mind) in &arena.minds {
        fold_mind(fold, *unit, mind);
    }
    // **17b cut M1b.1** (item 3d): every hull's condition slots, per row with
    // no count prefix — an arena with none (every production match) folds no
    // byte here.
    for (unit, slots) in arena.conditions.rows() {
        fold.bytes(&unit.0.to_le_bytes());
        for (at, slot) in slots.iter().enumerate() {
            let Some(rule) = slot else {
                continue;
            };
            let number = u8::try_from(at.saturating_add(1)).unwrap_or(u8::MAX);
            fold.bytes(&[number]);
            // M5rules: the complete typed rule, including arbitration and
            // replacement identity, determines future behavior. The shared
            // field serializer keeps new predicates in the deterministic fold.
            fold_value(fold, rule);
        }
    }
    // **17b cut M1b.1** (item 3e): when each slot last said why it held, per row
    // with no count prefix — empty in every production match.
    for ((unit, slot, contact), at_ms) in &arena.held_said {
        fold.bytes(&unit.0.to_le_bytes());
        fold.bytes(&[*slot]);
        fold.bytes(&contact.0.to_le_bytes());
        fold.bytes(&at_ms.to_le_bytes());
    }
    // **17b cut M1b.1**: `arena.odds_floor_permille` is deliberately NOT folded
    // (this function's head says why).
    for (viewer, id, held) in arena.contacts.all() {
        fold.bytes(&viewer.0.to_le_bytes());
        fold.bytes(&id.0.to_le_bytes());
        fold.bytes(&held.target.0.to_le_bytes());
        fold.bytes(&[held.class.code()]);
        fold.bytes(&held.first_seen_ms.to_le_bytes());
        fold.bytes(&held.last_seen_ms.to_le_bytes());
        // **16b cut F.3**: the plot and the provenance are authoritative state,
        // folded exactly as the stamps beside them are. A replay whose share
        // arrived from a different sighting, or whose plot is a metre away, must
        // not compare equal — the plot is what every reader of a contact's
        // position now reads, and the own sighting is what decides whether the
        // mesh may carry it a hop further.
        fold_plot(fold, &held.seen_at);
        match held.own_seen {
            Some(own) => {
                fold.bytes(&[1_u8]);
                fold.bytes(&own.at_ms.to_le_bytes());
                fold_plot(fold, &own.plot);
                // **16b cut Q.2**: what this platform's OWN sensors read, which
                // is what it may put on the mesh.
                fold_reading(fold, own.condition);
            }
            None => fold.bytes(&[0_u8]),
        }
        // **16b cut Q.2**: and what it holds, which may be a teammate's. Both are
        // knowledge that moves with no event of its own — a close look taken one
        // pass earlier, a share that arrived, a band the gunner watched go — so a
        // replay that read the same hull a pass sooner must not compare equal.
        fold_reading(fold, held.seen_condition);
        // **16b cut D.4**: whether this hold was opened under a number, which is
        // what decides the `identified` line and which moves with no
        // distinguishable event of its own — see this function's head.
        //
        // **16b cut E.2** (`B-45`): and whether this platform watched the target
        // die, beside it and for the same reason — the `destroyed` line that
        // moves it names the victim and the shooter and never the **holder**,
        // so the fold is the only thing that tells two replays apart.
        fold.bytes(&[u8::from(held.numbered), u8::from(held.known_dead)]);
    }
    // **16b cut Q.1**: the provisional numbers — since **16b cut M.3** (`T-27`)
    // as the book's running digest of every mint and the issued count, in place
    // of a walk of both maps that grew with the match (this function's head).
    fold.bytes(&arena.designators.digest().to_le_bytes());
    fold.bytes(&(arena.designators.issued_len() as u64).to_le_bytes());
    // **16b cut M.2** (16c `T-83`): the held losses, in the map's own key order —
    // the deadline decides whether a later substep says `lost`. The designator
    // beside it is not folded: it is read off the book and the callsign, both
    // already in here.
    fold.bytes(&(arena.held_losses.len() as u64).to_le_bytes());
    for ((viewer, contact), (deadline, _)) in &arena.held_losses {
        fold.bytes(&viewer.0.to_le_bytes());
        fold.bytes(&contact.0.to_le_bytes());
        fold.bytes(&deadline.to_le_bytes());
    }
    fold.bytes(&[arena.clock.tempo().code()]);
    fold.bytes(&arena.now_ms.to_le_bytes());
    fold.bytes(&(arena.due.len() as u64).to_le_bytes());
    fold.bytes(&(arena.shots.len() as u64).to_le_bytes());
    fold.bytes(&arena.ledger.seq().to_le_bytes());
    // **16b cut O.1** (item 12): the mode, and the Hold objective with its clock.
    fold.bytes(&[arena.mode.code()]);
    match arena.hold.as_ref() {
        Some(hold) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&hold.spot.0.to_le_bytes());
            fold.bytes(&hold.spot.1.to_le_bytes());
            fold.bytes(&hold.radius_cells.to_le_bytes());
            fold.bytes(&hold.hold_ms.to_le_bytes());
            match hold.held_by {
                Some((team, since_ms)) => {
                    fold.bytes(&[
                        1_u8,
                        match team {
                            crate::arena::state::Team::Red => 0_u8,
                            crate::arena::state::Team::Blue => 1_u8,
                        },
                    ]);
                    fold.bytes(&since_ms.to_le_bytes());
                }
                None => fold.bytes(&[0_u8]),
            }
        }
        None => fold.bytes(&[0_u8]),
    }
    fold.finish()
}

/// **One unit mind's row** (17b cut M1a.5; item 3c of [`state_hash`]).
fn fold_mind(fold: &mut Fold, unit: mush_core::ObjId, mind: &crate::arena::mind::MindState) {
    use crate::arena::perception::MindObjective;
    use crate::deliberation::order::RangeBand;
    fold.bytes(&unit.0.to_le_bytes());
    fold.bytes(&mind.think_seq.to_le_bytes());
    // **17b cut M3.6** (D-UM-23 as amended, "hashed whole"): the pending
    // think's presence, `think_seq` and `t_ms` (M2's bytes), then its ANSWER —
    // the three epochs in `Channel::ALL` order, the intents' count and each
    // intent's canonical bytes, the new memory's length and its FNV-1a 64
    // digest. This head's item 3c has the list.
    match mind.pending.as_ref() {
        Some(pending) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&pending.think_seq.to_le_bytes());
            fold.bytes(&pending.t_ms.to_le_bytes());
            for epoch in pending.epochs {
                fold.bytes(&epoch.to_le_bytes());
            }
            fold.bytes(&(pending.intents.len() as u64).to_le_bytes());
            for intent in &pending.intents {
                // Canonical bytes, as a notice's are below: a derive over plain
                // data, the same intent the same bytes on every run.
                fold_value(fold, intent);
            }
            fold.bytes(&(pending.memory.len() as u64).to_le_bytes());
            fold.bytes(&pending.memory);
        }
        None => fold.bytes(&[0_u8]),
    }
    fold.bytes(&(mind.memory.len() as u64).to_le_bytes());
    fold.bytes(&mind.memory);
    fold.bytes(&(mind.notices.len() as u64).to_le_bytes());
    for notice in &mind.notices {
        // Canonical bytes: a derive over closed enums, integers and words, so
        // the same notice is the same bytes on every run. A notice that does not
        // serialise cannot exist (every payload is plain data); it would fold
        // as no bytes rather than take the arena down.
        fold_value(fold, notice);
    }
    fold.bytes(&(mind.ranges.len() as u64).to_le_bytes());
    for (contact, range_m) in &mind.ranges {
        fold.bytes(&contact.0.to_le_bytes());
        fold.bytes(&range_m.to_le_bytes());
    }
    match mind.order.as_ref() {
        Some(order) => {
            fold.bytes(&[1_u8, order.objective.code()]);
            match order.objective {
                MindObjective::Advance { to } => {
                    fold.bytes(&to.0.to_le_bytes());
                    fold.bytes(&to.1.to_le_bytes());
                }
                MindObjective::Engage { target } => {
                    fold.bytes(&target.map_or(0, |id| id.0).to_le_bytes());
                }
                MindObjective::Screen { facing_mdeg } => {
                    fold.bytes(&facing_mdeg.to_le_bytes());
                }
                MindObjective::Support { of } => {
                    fold.bytes(&of.map_or(0, |id| id.0).to_le_bytes());
                    // **17b cut M5.2** (STOP E): the charge's unit, inside the
                    // `Support` arm only — a row holding any other order, or
                    // none, folds no new byte.
                    fold.bytes(&mind.order_charge.map_or(0, |unit| unit.0).to_le_bytes());
                }
                MindObjective::Hold | MindObjective::Withdraw => {}
            }
            let band = match order.max_range_band {
                None => 0_u8,
                Some(RangeBand::Close) => 1,
                Some(RangeBand::Effective) => 2,
                Some(RangeBand::Far) => 3,
            };
            fold.bytes(&[u8::from(order.hold_fire), band]);
            fold.bytes(&order.expires_ms.to_le_bytes());
            fold.bytes(order.squad.as_bytes());
        }
        None => fold.bytes(&[0_u8]),
    }
    fold.bytes(&[mind.strikes]);
    // **17b cut M3.6** (doc 17 §13's list): the drop — its presence, then the
    // cause's byte (strikes 1, core 2, abi 3) — and the seal's digest, after
    // `strikes` and before the card.
    match mind.dropped {
        None => fold.bytes(&[0_u8]),
        Some(cause) => fold.bytes(&[1_u8, drop_cause_code(cause)]),
    }
    fold.bytes(&mind.seal_digest);
    // **17b cut M1b.1**: the card, its fields in declaration order — an enum as
    // its code byte, a bool as one byte, an integer little-endian.
    fold_card(fold, &mind.card);
}

/// **A drop's cause as its one hash byte** (17b cut M3.6; item 3c): strikes
/// `1`, core `2`, abi `3` — `0` is the absent drop's presence byte, never a
/// cause.
const fn drop_cause_code(cause: crate::arena::mind::DropCause) -> u8 {
    use crate::arena::mind::DropCause;
    match cause {
        DropCause::Strikes => 1,
        DropCause::Core => 2,
        DropCause::Abi => 3,
    }
}

/// **One mind's card** (17b cut M1b.1; inside item 3c of [`state_hash`], after
/// `strikes`, the drop and the seal's digest).
fn fold_card(fold: &mut Fold, card: &crate::arena::card::StockCard) {
    fold.bytes(&[card.stance.code()]);
    fold.bytes(&card.engage_band_permille.to_le_bytes());
    fold.bytes(&card.close_m.to_le_bytes());
    fold.bytes(&[card.orbit_sense.code(), u8::from(card.turn_into_hits)]);
    fold.bytes(&card.turn_s.to_le_bytes());
    fold.bytes(&card.screen_bearing.to_le_bytes());
    fold.bytes(&card.screen_arc.to_le_bytes());
    fold.bytes(&[card.opening.code()]);
    fold.bytes(&card.min_odds_permille.to_le_bytes());
    fold.bytes(&[card.aspect.code()]);
    fold.bytes(&card.reserve_pct.to_le_bytes());
    fold.bytes(&card.reserve_odds_pct.to_le_bytes());
    fold.bytes(&[card.prefer.code()]);
    fold.bytes(&card.weight_range.to_le_bytes());
    fold.bytes(&card.weight_facet.to_le_bytes());
    fold.bytes(&card.weight_health.to_le_bytes());
    fold.bytes(&card.switch_m.to_le_bytes());
    fold.bytes(&card.withdraw_permille.to_le_bytes());
    fold.bytes(&card.stand_min_permille.to_le_bytes());
    fold.bytes(&card.rally_ms.to_le_bytes());
    fold.bytes(&card.search_ms.to_le_bytes());
    fold.bytes(&card.hunt_after_ms.to_le_bytes());
    fold.bytes(&[u8::from(card.stop_unseen)]);
}

/// **A motor row's avoided cells** (16b cut Q.7's encoding; one function since
/// 17b cut M1a.1 so items 3 and 3b could not fold them differently, item 3b's
/// alone since 17b cut M4.6): their count as
/// `u64`, then per entry the cell's two `u16` and the stop's `i64` stamp.
fn fold_avoid(fold: &mut Fold, avoid: &[((u16, u16), i64)]) {
    fold.bytes(&(avoid.len() as u64).to_le_bytes());
    for ((x, y), at_ms) in avoid {
        fold.bytes(&x.to_le_bytes());
        fold.bytes(&y.to_le_bytes());
        fold.bytes(&at_ms.to_le_bytes());
    }
}

/// **A motor row's search start and remembered stop** (16b cut AI.1's encoding;
/// one function since 17b cut M1a.1 so items 3 and 3b could not fold them
/// differently, item 3b's alone since 17b cut M4.6): `searched_from` as a
/// presence byte then the cell's two `u16`,
/// and `refused` as `[0]` alone for none, else `[1]`, the stand cell's and the
/// refused cell's two `u16` each, the cause's code byte, the goal's two `u16`
/// and the `i64` it lapses at.
fn fold_search(fold: &mut Fold, motor: &crate::arena::motor::MotorRow) {
    match motor.searched_from {
        Some((x, y)) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&x.to_le_bytes());
            fold.bytes(&y.to_le_bytes());
        }
        None => fold.bytes(&[0_u8]),
    }
    match motor.refused {
        Some(refusal) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&refusal.at.0.to_le_bytes());
            fold.bytes(&refusal.at.1.to_le_bytes());
            fold.bytes(&refusal.cell.0.to_le_bytes());
            fold.bytes(&refusal.cell.1.to_le_bytes());
            fold.bytes(&[refusal.cause.code()]);
            fold.bytes(&refusal.goal.0.to_le_bytes());
            fold.bytes(&refusal.goal.1.to_le_bytes());
            fold.bytes(&refusal.until_ms.to_le_bytes());
        }
        None => fold.bytes(&[0_u8]),
    }
}

/// **One contact's reading** (16b cut Q.2): a presence byte, then the look's own
/// stamp, the packed bands and how many sections are packed.
///
/// A contact carries two of them — what this platform's own sensors read and what
/// it holds, which may be a teammate's — and they are folded by one function so
/// they cannot fold differently, exactly as [`fold_plot`]'s two plots are.
fn fold_reading(fold: &mut Fold, reading: Option<crate::arena::condition::Reading>) {
    match reading {
        Some(reading) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&reading.at_ms.to_le_bytes());
            fold.bytes(&reading.condition.bands.to_le_bytes());
            fold.bytes(&[reading.condition.sections]);
        }
        None => fold.bytes(&[0_u8]),
    }
}

/// **One optional arena timestamp** (16b cut Q.7): a presence byte, then the
/// `i64`.
///
/// `Withdraw`'s two stamps (the reflex machines', item 3, until 17b cut M4.6)
/// were folded by one function so they could not fold differently, for the
/// reason [`fold_plot`] and [`fold_reading`] are one function each; item 3b's
/// `hold_retry_ms` reads it.
fn fold_stamp(fold: &mut Fold, at_ms: Option<i64>) {
    match at_ms {
        Some(at_ms) => {
            fold.bytes(&[1_u8]);
            fold.bytes(&at_ms.to_le_bytes());
        }
        None => fold.bytes(&[0_u8]),
    }
}

/// **One contact plot's five integers** (16b cut F.3), in field order.
///
/// A contact carries two of them — the plot every reader reads, and the plot of
/// this platform's own last sighting — and they are folded by one function so
/// they cannot fold differently.
fn fold_plot(fold: &mut Fold, plot: &crate::arena::state::SeenPose) {
    fold.bytes(&plot.x_cm.to_le_bytes());
    fold.bytes(&plot.y_cm.to_le_bytes());
    fold.bytes(&plot.z_cm.to_le_bytes());
    fold.bytes(&plot.heading_mdeg.to_le_bytes());
    fold.bytes(&plot.speed_cms.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::clock::{Tempo, TempoSignals};
    use crate::arena::state::{DueKind, Power, Team};
    use crate::arena::test_support::{arena_of, pose, tank};
    use crate::config::TempoConfig;
    use mush_core::ObjId;

    fn two_tanks() -> Arena {
        arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(6_500, 6_500)),
        ])
    }

    #[test]
    fn fnv_folds_bytes_in_order() {
        assert_eq!(fnv1a64_update(FNV_OFFSET_BASIS, b""), FNV_OFFSET_BASIS);
        // Order matters, which is what makes the length-prefix-free field list
        // above safe only because every field has a fixed width.
        assert_ne!(
            fnv1a64_update(FNV_OFFSET_BASIS, b"ab"),
            fnv1a64_update(FNV_OFFSET_BASIS, b"ba")
        );
        // Chaining is the same as one pass.
        let split = fnv1a64_update(fnv1a64_update(FNV_OFFSET_BASIS, b"ab"), b"cd");
        assert_eq!(split, fnv1a64_update(FNV_OFFSET_BASIS, b"abcd"));
    }

    #[test]
    fn the_hash_is_stable_for_one_state() {
        let arena = two_tanks();
        assert_eq!(state_hash(&arena), state_hash(&arena));
        // And identical for an independently built identical arena.
        assert_eq!(state_hash(&arena), state_hash(&two_tanks()));
    }

    #[test]
    fn the_hash_changes_when_a_pose_changes() {
        let mut arena = two_tanks();
        let before = state_hash(&arena);
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.pose.x_cm += 1;
        }
        let moved = state_hash(&arena);
        assert_ne!(before, moved);
        // One centimetre back is the same state again.
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.pose.x_cm -= 1;
        }
        assert_eq!(state_hash(&arena), before);
        // Each pose field separately.
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| unit.pose.y_cm += 1) as fn(&mut _),
            |unit| unit.pose.z_cm += 1,
            |unit| unit.pose.heading_mdeg += 1,
            |unit| unit.speed_cms += 1,
            |unit| unit.power = Power::Online,
            |unit| unit.limits.slow_pct = 50,
            |unit| unit.limits.no_turn_left = true,
            |unit| unit.limits.no_turn_right = true,
            |unit| unit.limits.motion_disabled = true,
            |unit| unit.limits.wreck = true,
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), before, "a change went unhashed");
        }
    }

    #[test]
    fn the_hash_folds_the_tempo_the_clock_the_schedule_and_the_ledger() {
        let base = state_hash(&two_tanks());

        let mut tempo = two_tanks();
        tempo.clock.evaluate(
            0,
            &TempoSignals {
                shot_fired: true,
                ..TempoSignals::quiet()
            },
            &TempoConfig::default(),
        );
        assert_eq!(tempo.clock.tempo(), Tempo::Engaged);
        assert_ne!(state_hash(&tempo), base);

        let mut later = two_tanks();
        later.now_ms = 1;
        assert_ne!(state_hash(&later), base);

        let mut scheduled = two_tanks();
        scheduled.due.schedule(1_000, DueKind::PositionSummary);
        assert_ne!(state_hash(&scheduled), base);

        let mut journalled = two_tanks();
        let _ = journalled.ledger.append(
            0,
            crate::arena::ledger::EventKind::Startup { unit: ObjId(4) },
            Vec::new(),
            0,
        );
        assert_ne!(state_hash(&journalled), base);
    }

    /// **16b cut W.1's extension**: the lock, the two weapon cells and the in-flight
    /// shot count are each a fact that moves without an event of its own, so each has
    /// to move the hash.
    #[test]
    fn the_hash_folds_the_lock_the_weapon_cells_and_the_shots_in_flight() {
        let base = state_hash(&two_tanks());
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| {
                unit.lock = Some(crate::arena::state::ContactId(7));
            }) as fn(&mut _),
            |unit| {
                unit.ready_at_ms
                    .insert(smol_str::SmolStr::new("cannon"), 1_000);
            },
            |unit| {
                unit.ammo.insert(smol_str::SmolStr::new("cannon"), 40);
            },
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), base, "a weapon cell went unhashed");
        }
        // Two weapons that swapped cycles are not one state, which is why the id's
        // bytes are folded beside the value.
        let mut swapped = two_tanks();
        if let Some(unit) = swapped.unit_mut(ObjId(9)) {
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("cannon"), 1_000);
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("machine_gun"), 2_000);
        }
        let mut other = two_tanks();
        if let Some(unit) = other.unit_mut(ObjId(9)) {
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("cannon"), 2_000);
            unit.ready_at_ms
                .insert(smol_str::SmolStr::new("machine_gun"), 1_000);
        }
        assert_ne!(state_hash(&swapped), state_hash(&other));
    }

    /// **16b cut X.6's extension**: the slope a unit entered its cell by is a
    /// fact with one writer that moves without an event, so it has to move the
    /// hash. The limit beside it never moves, so it deliberately does not.
    #[test]
    fn the_hash_folds_the_slope_a_unit_entered_its_cell_by() {
        let base = state_hash(&two_tanks());
        let mut climbed = two_tanks();
        if let Some(unit) = climbed.unit_mut(ObjId(9)) {
            unit.grade_pct = 30;
        }
        assert_ne!(state_hash(&climbed), base, "a grade went unhashed");
        // Back to the flat is the same state again.
        if let Some(unit) = climbed.unit_mut(ObjId(9)) {
            unit.grade_pct = 0;
        }
        assert_eq!(state_hash(&climbed), base);
        // The limit is a fact about the fit, not about the match.
        let mut limited = two_tanks();
        if let Some(unit) = limited.unit_mut(ObjId(9)) {
            unit.slope_limit_pct = Some(45);
        }
        assert_eq!(state_hash(&limited), base);
        // **16b cut AE.1**: and so is the row it pays, for the same reason.
        let mut walker = two_tanks();
        if let Some(unit) = walker.unit_mut(ObjId(9)) {
            unit.cost_row = crate::arena::state::CostRow::Legged;
        }
        assert_eq!(state_hash(&walker), base, "the cost row is a fit constant");
    }

    /// **16b cut E.3's extension**: a traffic hold is a fact with one writer that
    /// decides a later refusal and moves with no ledger line at all, so it has to
    /// move the hash — and the cell and the stamp each on their own, because a
    /// hold against a different cell is a different hold and a hold that opened
    /// earlier matures earlier.
    #[test]
    fn the_hash_folds_the_cell_a_unit_is_held_against_and_since_when() {
        use crate::arena::state::OccupiedHold;

        let base = state_hash(&two_tanks());
        let mut held = two_tanks();
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (4, 3),
                since_ms: 2_000,
            });
        }
        let first = state_hash(&held);
        assert_ne!(first, base, "a hold went unhashed");
        // The cell alone.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (3, 4),
                since_ms: 2_000,
            });
        }
        assert_ne!(state_hash(&held), first, "the held cell went unhashed");
        // The stamp alone.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = Some(OccupiedHold {
                cell: (4, 3),
                since_ms: 2_250,
            });
        }
        assert_ne!(state_hash(&held), first, "the hold's stamp went unhashed");
        // And letting it go is the state it was in before.
        if let Some(unit) = held.unit_mut(ObjId(9)) {
            unit.occupied_hold = None;
        }
        assert_eq!(state_hash(&held), base);
    }

    /// **16b cut Q.1's extension**: a provisional number moves without a
    /// distinguishable ledger event — a mesh `detected` and a sensor `detected`
    /// are one shape — so the book has to move the hash. The unit's own callsign
    /// does not: it is a function of the check-out and never moves.
    #[test]
    fn the_hash_folds_the_designator_book_and_not_the_callsigns() {
        let base = state_hash(&two_tanks());
        let mut issued = two_tanks();
        assert_eq!(issued.designators.assign(ObjId(4), ObjId(9)), 1);
        assert_ne!(state_hash(&issued), base, "a number went unhashed");
        // Two viewers that issued the same number for different targets are not
        // one state.
        let mut other = two_tanks();
        assert_eq!(other.designators.assign(ObjId(9), ObjId(4)), 1);
        assert_ne!(state_hash(&other), state_hash(&issued));
        // The callsign is a fact about the check-out, not about the match.
        let mut renamed = two_tanks();
        if let Some(unit) = renamed.unit_mut(ObjId(9)) {
            unit.callsign = crate::arena::state::Designator::Slot {
                team: Team::Blue,
                slot: 9,
            };
        }
        assert_eq!(state_hash(&renamed), base);
    }

    /// **16b cut Q.6's extension**: each of the four motion carries is the
    /// fractional part of a quantity already in the hash, so each has to move it
    /// on its own — a replay whose carries differ is a replay that is about to
    /// disagree about a whole centimetre.
    #[test]
    fn the_hash_folds_each_of_the_four_motion_carries() {
        let base = state_hash(&two_tanks());
        for mutate in [
            (|unit: &mut crate::arena::state::UnitState| unit.carry.turn = 1) as fn(&mut _),
            |unit| unit.carry.speed = 1,
            |unit| unit.carry.east = 1,
            |unit| unit.carry.north = 1,
            // 16b cut M.4 (`T-16`): the two sign bytes, each on its own.
            |unit| unit.carry.turn_side = 1,
            |unit| unit.carry.speed_rate = 2,
        ] {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                mutate(unit);
            }
            assert_ne!(state_hash(&probe), base, "a carry went unhashed");
        }
        // And the two sign bytes are not one number either: a left turn's
        // carry and an accelerating one are different states.
        let mut left = two_tanks();
        if let Some(unit) = left.unit_mut(ObjId(9)) {
            unit.carry.turn_side = 1;
        }
        let mut accel = two_tanks();
        if let Some(unit) = accel.unit_mut(ObjId(9)) {
            unit.carry.speed_rate = 1;
        }
        assert_ne!(state_hash(&left), state_hash(&accel));
        // The four are separate fields and not one number: a carry moved from one
        // to another is a different state.
        let mut east = two_tanks();
        if let Some(unit) = east.unit_mut(ObjId(9)) {
            unit.carry.east = 7;
        }
        let mut north = two_tanks();
        if let Some(unit) = north.unit_mut(ObjId(9)) {
            unit.carry.north = 7;
        }
        assert_ne!(state_hash(&east), state_hash(&north));
        // And back to nothing carried is the same state again.
        if let Some(unit) = east.unit_mut(ObjId(9)) {
            unit.carry.east = 0;
        }
        assert_eq!(state_hash(&east), base);
    }

    /// **16b cut D.2's extension**: the spent-section window is persistent state
    /// that decides what a later substep says, so it has to move the hash — and
    /// the scratch beside it is not, so it must not.
    #[test]
    fn the_hash_folds_the_spent_section_window_and_not_the_spent_shot_scratch() {
        let base = state_hash(&two_tanks());
        let mut said = two_tanks();
        said.spent_said.insert(
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("turret")),
            5_000,
        );
        assert_ne!(
            state_hash(&said),
            base,
            "one window entry alone went unhashed"
        );
        // The stamp is part of it: two runs that said the same line at different
        // arena times are about to suppress different later ones.
        let mut later = two_tanks();
        later.spent_said.insert(
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("turret")),
            5_001,
        );
        assert_ne!(state_hash(&later), state_hash(&said));
        // And so is each of the three key parts.
        for key in [
            (ObjId(9), ObjId(9), smol_str::SmolStr::new("turret")),
            (ObjId(4), ObjId(4), smol_str::SmolStr::new("turret")),
            (ObjId(4), ObjId(9), smol_str::SmolStr::new("hull")),
        ] {
            let mut probe = two_tanks();
            probe.spent_said.insert(key, 5_000);
            assert_ne!(state_hash(&probe), state_hash(&said), "a key part is lost");
        }
        // Taking it out again is the same state.
        said.spent_said.clear();
        assert_eq!(state_hash(&said), base);
        // The scratch is deliberately absent: it is a function of state this hash
        // already folds and it never survives its own substep.
        let mut scratch = two_tanks();
        scratch.spent_hits.insert(7);
        assert_eq!(state_hash(&scratch), base, "the scratch was folded");
    }

    /// **16b cut D.4's extension**: the two facts that move with no
    /// distinguishable ledger event of their own — whether a hold was opened
    /// under a number, and what the platform's own receiver is running under.
    #[test]
    fn the_hash_folds_whether_a_hold_was_numbered_and_the_platforms_own_receiver() {
        use crate::arena::perception::Receiver;
        use crate::arena::state::{Classification, ContactId, SeenPose, ShadowContact};

        let base = state_hash(&two_tanks());

        // `numbered` alone: two arenas identical but for the one byte. A mesh
        // arrival and a contact this platform's own sight opened journal the same
        // `detected` line, so this is the only thing that tells them apart.
        let hold = |numbered: bool| {
            let mut probe = two_tanks();
            probe.contacts.insert(
                ObjId(4),
                ContactId(7),
                ShadowContact {
                    numbered,
                    ..ShadowContact::sighted(
                        ObjId(9),
                        Classification::Unknown,
                        1_000,
                        SeenPose::default(),
                    )
                },
            );
            state_hash(&probe)
        };
        assert_ne!(hold(false), base, "the contact itself went unhashed");
        assert_ne!(hold(true), hold(false), "`numbered` alone went unhashed");

        // `receiver` alone, and each of its three integers on its own: two
        // stacked pods and one pod are not one state, and neither is a filter
        // that recovered half the reach.
        let receiver = Receiver {
            budget: 15,
            budget_max: 40,
            reach_permille: 600,
        };
        let with = |receiver: Option<Receiver>| {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                unit.receiver = receiver;
            }
            state_hash(&probe)
        };
        assert_ne!(with(Some(receiver)), base, "a receiver went unhashed");
        for moved in [
            Receiver {
                budget: 0,
                ..receiver
            },
            Receiver {
                budget_max: 100,
                ..receiver
            },
            Receiver {
                reach_permille: 601,
                ..receiver
            },
        ] {
            assert_ne!(
                with(Some(moved)),
                with(Some(receiver)),
                "a receiver field went unhashed"
            );
        }
        // And clean again is the same state: the fold is a function of the value
        // and not of how it got there.
        assert_eq!(with(None), base);
    }

    /// **16b cut E.2's extension** (`B-45`): whether the platform holding a
    /// contact watched its target die. The `destroyed` line that moves it names
    /// the victim and the shooter and never the holder, so nothing but this fold
    /// tells two replays apart — and the static mask beside it never moves, so
    /// it must not be folded at all.
    #[test]
    fn the_hash_folds_whether_a_hold_knows_its_target_is_dead_and_not_the_fatal_mask() {
        use crate::arena::state::{Classification, ContactId, SeenPose, ShadowContact};

        let base = state_hash(&two_tanks());
        let hold = |known_dead: bool| {
            let mut probe = two_tanks();
            probe.contacts.insert(
                ObjId(4),
                ContactId(7),
                ShadowContact {
                    known_dead,
                    ..ShadowContact::sighted(
                        ObjId(9),
                        Classification::Unknown,
                        1_000,
                        SeenPose::default(),
                    )
                },
            );
            state_hash(&probe)
        };
        assert_ne!(hold(false), base, "the contact itself went unhashed");
        assert_ne!(hold(true), hold(false), "`known_dead` alone went unhashed");

        // The mask is a fact about the unit's fit, not about the match.
        let mut fitted = two_tanks();
        if let Some(unit) = fitted.unit_mut(ObjId(9)) {
            unit.fatal_sections = 0b101;
        }
        assert_eq!(state_hash(&fitted), base, "the fatal mask was folded");
    }

    /// **16b cut M.3's extension** (16c `T-04`): what a unit is held against
    /// moves the hash — its presence, and each of the cell and the cause on its
    /// own, because a different cell or a different cause is a new edge — and
    /// letting it go is the state before. The per-substep flag beside it does not.
    #[test]
    fn m3_the_hash_folds_what_a_unit_is_held_against_cell_and_cause() {
        use crate::arena::motion::{BlockedCause, Held};

        let base = state_hash(&two_tanks());
        let held = |x: u16, y: u16, cause: BlockedCause| {
            let mut probe = two_tanks();
            if let Some(unit) = probe.unit_mut(ObjId(9)) {
                unit.held = Some(Held { x, y, cause });
            }
            state_hash(&probe)
        };
        let first = held(4, 3, BlockedCause::Impassable);
        assert_ne!(first, base, "a held unit went unhashed");
        assert_ne!(held(3, 3, BlockedCause::Impassable), first, "the column");
        assert_ne!(held(4, 4, BlockedCause::Impassable), first, "the row");
        for cause in [
            BlockedCause::Occupied,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ] {
            assert_ne!(
                held(4, 3, cause),
                first,
                "a different cause at the same cell went unhashed: {cause:?}"
            );
        }
        // The flag is not the condition, and is not folded.
        let mut flagged = two_tanks();
        if let Some(unit) = flagged.unit_mut(ObjId(9)) {
            unit.blocked = true;
        }
        assert_eq!(
            state_hash(&flagged),
            base,
            "the per-substep flag was folded"
        );
        // Letting go is the state it was in before.
        let mut released = two_tanks();
        if let Some(unit) = released.unit_mut(ObjId(9)) {
            unit.held = Some(Held {
                x: 4,
                y: 3,
                cause: BlockedCause::Impassable,
            });
            unit.held = None;
        }
        assert_eq!(state_hash(&released), base);
    }

    /// **16b cut M.3's extension** (16c `T-06`), the carry probe's shape: heat
    /// gates firing and moves every substep with no line, so the heat cell and
    /// its carry each move the hash on their own.
    #[test]
    fn m3_the_hash_folds_the_heat_cell_and_the_heat_carry() {
        use crate::arena::test_support::laser_tank;

        let hot = || {
            arena_of(vec![
                tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
                laser_tank(ObjId(9), Team::Blue, pose(6_500, 6_500)),
            ])
        };
        let base = state_hash(&hot());
        assert_eq!(state_hash(&hot()), base, "the fixture is stable");
        // The heat cell, through the one writer the arena uses for it: one
        // second's rise — a shot's worth — as `raise_heat` would apply it.
        let mut raised = hot();
        let addr = raised
            .unit(ObjId(9))
            .and_then(|unit| unit.heat.as_ref())
            .and_then(|heat| heat.addr(crate::definitions::unit::HEAT_CELL))
            .unwrap_or_else(|| panic!("the laser tank carries a heat cell"));
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell { addr, value: 7 }],
            None,
            None,
        )
        .unwrap_or_else(|_| panic!("a one-op plan"));
        let post = raised
            .unit(ObjId(9))
            .map(|unit| mush_tactics::apply(&unit.unit, &plan, 0))
            .and_then(Result::ok)
            .map(|(post, _)| post)
            .unwrap_or_else(|| panic!("the plan applies"));
        raised.set_unit(ObjId(9), post, false);
        assert_eq!(
            raised.unit(ObjId(9)).and_then(|unit| unit.heat_now()),
            Some(7)
        );
        assert_ne!(state_hash(&raised), base, "the heat cell went unhashed");
        // The carry alone: the fraction of a heat unit the last substep did not
        // shed.
        let mut carried = hot();
        if let Some(heat) = carried
            .unit_mut(ObjId(9))
            .and_then(|unit| unit.heat.as_mut())
        {
            heat.carry_milli = 1;
        }
        assert_ne!(state_hash(&carried), base, "the heat carry went unhashed");
        assert_ne!(state_hash(&carried), state_hash(&raised));
        // And back to cold with nothing carried is the same state again.
        if let Some(heat) = carried
            .unit_mut(ObjId(9))
            .and_then(|unit| unit.heat.as_mut())
        {
            heat.carry_milli = 0;
        }
        assert_eq!(state_hash(&carried), base);
    }

    /// **16b cut M.1's extension** (`T-82`): the seat-order rate's window and
    /// count each move the hash on their own — they decide a later `queue_full`.
    #[test]
    fn the_hash_folds_the_seat_order_window_and_its_count() {
        let base = state_hash(&two_tanks());
        let mut windowed = two_tanks();
        if let Some(unit) = windowed.unit_mut(ObjId(9)) {
            unit.orders_window_s = 3;
        }
        assert_ne!(state_hash(&windowed), base, "the window went unhashed");
        let mut counted = two_tanks();
        if let Some(unit) = counted.unit_mut(ObjId(9)) {
            unit.orders_in_window = 3;
        }
        assert_ne!(state_hash(&counted), base, "the count went unhashed");
        assert_ne!(state_hash(&counted), state_hash(&windowed));
        if let Some(unit) = counted.unit_mut(ObjId(9)) {
            unit.orders_in_window = 0;
        }
        assert_eq!(state_hash(&counted), base);
    }

    /// **16b cut M.2's extension** (`T-83`): a held loss moves the hash — its
    /// viewer, its contact and its deadline each on their own — its expiry moves
    /// it back, and the designator carried beside the deadline is not folded.
    #[test]
    fn m2_the_hash_folds_a_held_loss_and_its_expiry_moves_it_back() {
        use crate::arena::state::{ContactId, Designator};

        let base = state_hash(&two_tanks());
        let held = |viewer: u32, contact: u64, deadline: i64, n: u16| {
            let mut probe = two_tanks();
            probe.held_losses.insert(
                (ObjId(viewer), ContactId(contact)),
                (deadline, Designator::Provisional(n)),
            );
            probe
        };
        let first = state_hash(&held(4, 7, 2_004, 1));
        assert_ne!(first, base, "a held loss went unhashed");
        assert_ne!(state_hash(&held(9, 7, 2_004, 1)), first, "the viewer");
        assert_ne!(state_hash(&held(4, 8, 2_004, 1)), first, "the contact");
        assert_ne!(state_hash(&held(4, 7, 2_005, 1)), first, "the deadline");
        assert_eq!(
            state_hash(&held(4, 7, 2_004, 2)),
            first,
            "the designator was folded"
        );
        // The expiry takes it out again: the same state as before the loss.
        let mut expired = held(4, 7, 2_004, 1);
        expired.held_losses.remove(&(ObjId(4), ContactId(7)));
        assert_eq!(state_hash(&expired), base);
    }

    #[test]
    fn the_hash_does_not_fold_the_orders_only_their_effect() {
        // A pulse that has accepted a helm order but not yet integrated it is
        // the same world as one that has not: the replay compares what the
        // world became.
        let mut ordered = two_tanks();
        if let Some(unit) = ordered.unit_mut(ObjId(4)) {
            unit.desired_heading_mdeg = 90_000;
            unit.desired_speed_cms = 500;
        }
        assert_eq!(state_hash(&ordered), state_hash(&two_tanks()));
    }

    #[test]
    fn two_units_swapping_positions_is_not_one_state() {
        // The guard against a hash that folds a multiset rather than a sequence.
        let swapped = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(6_500, 6_500)),
            tank(ObjId(9), Team::Blue, pose(1_500, 1_500)),
        ]);
        assert_ne!(state_hash(&swapped), state_hash(&two_tanks()));
    }

    /// **16b cut O.1's extension** (item 12): the mode and the Hold clock move
    /// the hash — two arenas differing only in `held_by`, or only in its `since`,
    /// or only in the mode, are two states — and so does a machine's
    /// `hold_retry_ms`.
    ///
    /// 17b cut M4.5a: the hold retry is the hull's motor row's (item 3b folds
    /// it for every hull — every hull with no reflex row until 17b cut M4.6),
    /// laid by `MotorTable::ensure` and not by adopting the reflex machine.
    #[test]
    fn o1_the_hash_folds_the_mode_the_hold_clock_and_the_hold_retry() {
        use crate::arena::state::{HoldState, MatchMode};
        let hold = HoldState {
            spot: (4, 4),
            letter: Some('A'),
            radius_cells: 1,
            hold_ms: 10_000,
            held_by: None,
        };
        let with = |mode: MatchMode, held: Option<HoldState>| {
            let mut arena = two_tanks();
            arena.mode = mode;
            arena.hold = held;
            state_hash(&arena)
        };
        let base = state_hash(&two_tanks());
        assert_eq!(with(MatchMode::Skirmish, None), base);
        let open = with(MatchMode::Hold, Some(hold));
        assert_ne!(open, base, "the mode and the objective went unhashed");
        assert_ne!(with(MatchMode::Hold, None), base, "the mode alone");
        let red = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Red, 250)),
                ..hold
            }),
        );
        assert_ne!(red, open, "`held_by` went unhashed");
        let blue = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Blue, 250)),
                ..hold
            }),
        );
        assert_ne!(blue, red, "the holding side went unhashed");
        let later = with(
            MatchMode::Hold,
            Some(HoldState {
                held_by: Some((Team::Red, 500)),
                ..hold
            }),
        );
        assert_ne!(later, red, "the clock's `since` went unhashed");
        let wider = with(
            MatchMode::Hold,
            Some(HoldState {
                radius_cells: 2,
                ..hold
            }),
        );
        assert_ne!(wider, open, "the radius went unhashed");

        // A hull's motor row's hold retry.
        let mut machine = two_tanks();
        let _ = machine.motors.ensure(ObjId(4));
        let adopted = state_hash(&machine);
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.hold_retry_ms = Some(90_000);
        }
        assert_ne!(
            state_hash(&machine),
            adopted,
            "`hold_retry_ms` went unhashed"
        );
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.hold_retry_ms = None;
        }
        assert_eq!(state_hash(&machine), adopted);
    }

    /// **16b cut AI.1's extension** (item 3): the cell a machine's last search
    /// started from, and the stop it remembers, each move the hash — and every
    /// field of the stop does, so a fold that wrote only its presence would be
    /// caught — and resetting either is the same state again.
    ///
    /// 17b cut M4.5a: on the hull's motor row (item 3b, `fold_search`), laid
    /// by `MotorTable::ensure` and not by adopting the reflex machine.
    #[test]
    fn ai1_the_hash_folds_the_search_start_and_the_refusal_memory() {
        use crate::arena::motion::BlockedCause;
        use crate::arena::motor::Refusal;
        let mut machine = two_tanks();
        let _ = machine.motors.ensure(ObjId(4));
        let adopted = state_hash(&machine);

        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((1, 1));
        }
        let from = state_hash(&machine);
        assert_ne!(from, adopted, "`searched_from` went unhashed");
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = Some((1, 2));
        }
        assert_ne!(
            state_hash(&machine),
            from,
            "`searched_from`'s cell went unhashed"
        );
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.searched_from = None;
        }
        assert_eq!(state_hash(&machine), adopted);

        let base = Refusal {
            at: (1, 1),
            cell: (2, 2),
            cause: BlockedCause::Impassable,
            goal: (6, 4),
            until_ms: 31_000,
        };
        let mut seen: Vec<u64> = vec![adopted];
        for refusal in [
            base,
            Refusal { at: (1, 2), ..base },
            Refusal {
                cell: (2, 1),
                ..base
            },
            Refusal {
                cause: BlockedCause::Edge,
                ..base
            },
            Refusal {
                goal: (6, 5),
                ..base
            },
            Refusal {
                until_ms: 31_001,
                ..base
            },
        ] {
            if let Some(row) = machine.motors.row_mut(ObjId(4)) {
                row.refused = Some(refusal);
            }
            let hash = state_hash(&machine);
            assert!(
                !seen.contains(&hash),
                "`refused` {refusal:?} folded the same bytes as an earlier state"
            );
            seen.push(hash);
        }
        if let Some(row) = machine.motors.row_mut(ObjId(4)) {
            row.refused = None;
        }
        assert_eq!(state_hash(&machine), adopted);
    }

    /// **17b cut M1a.1 (`hash.rs` item 3b): a motor row is folded, field by
    /// field, and a table that holds none adds no byte.**
    ///
    /// A pilot's tank is given a motor row — the shape 17b cut M1a.4's `DRIVE`
    /// lays down — and each of the row's folded fields, set one at a time,
    /// moves the hash to a state no earlier probe reached; taking the row away
    /// is the first state again.
    ///
    /// 17b cut M4.6a: moved here from `crate::arena::reflex`'s tests with its
    /// assertions word for word — the only pin of item 3b's `goal`, `waypoint`,
    /// `route` and `avoid` probes, of `from_waypoint` left unfolded and of a
    /// release leaving no byte. 17b cut M4.6c deleted item 3 and the reflex
    /// controller: its reflex half (a reflex row whose motor row is missing
    /// folded as a fresh one) died with the reflex's module, and this test
    /// lost its precondition that the tank has no reflex row (`Arena.reflex`
    /// is gone) and its name,
    /// `m1a1_a_motor_row_of_a_unit_with_no_reflex_row_moves_the_hash_and_an_empty_table_adds_none`;
    /// every other assertion is as it was.
    #[test]
    fn m1a1_a_motor_row_moves_the_hash_field_by_field_and_an_empty_table_adds_none() {
        use crate::arena::motion::BlockedCause;
        use crate::arena::motor::Refusal;
        let build = || arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        let bare = build();
        assert_eq!(bare.motors.rows().count(), 0, "an empty table");
        let base = crate::arena::hash::state_hash(&bare);

        let refusal = Refusal {
            at: (1, 1),
            cell: (2, 1),
            cause: BlockedCause::Impassable,
            goal: (6, 4),
            until_ms: 31_000,
        };
        let mut seen: Vec<u64> = vec![base];
        for edit in 0_u8..=15 {
            let mut arena = build();
            let motor = arena.motors.ensure(ObjId(4));
            match edit {
                0 => {} // the row itself, every field at its default
                1 => motor.goal = Some((3, 4)),
                2 => motor.goal = Some((4, 3)),
                3 => motor.waypoint = Some(2),
                4 => motor.route = vec![(2, 2)],
                5 => motor.avoid = vec![((3, 4), 1_000)],
                6 => motor.hold_retry_ms = Some(9_000),
                7 => motor.searched_from = Some((1, 1)),
                8 => motor.refused = Some(refusal),
                9 => motor.requested_goal = Some((3, 4)),
                10 => motor.requested_goal = Some((4, 3)),
                11 => {
                    motor.requested_goal = Some((3, 4));
                    motor.outcome = Some(crate::arena::perception::DriveOutcome::Arrived);
                }
                12 => {
                    motor.requested_goal = Some((3, 4));
                    motor.outcome = Some(crate::arena::perception::DriveOutcome::NoRoute);
                }
                13 => motor.fan = crate::arena::motor::GoalFan::between((6, 3), (6, 6)),
                14 => motor.fan = crate::arena::motor::GoalFan::between((7, 3), (6, 6)),
                _ => motor.fan = crate::arena::motor::GoalFan::between((6, 3), (7, 6)),
            }
            let hash = crate::arena::hash::state_hash(&arena);
            assert!(
                !seen.contains(&hash),
                "probe {edit} of a machine-less motor row folded the bytes of an earlier state"
            );
            seen.push(hash);
            arena.motors.release(ObjId(4));
            assert_eq!(
                crate::arena::hash::state_hash(&arena),
                base,
                "probe {edit}: the row's release left a byte behind"
            );
        }
        // `from_waypoint` is deliberately not folded, as it was not on the
        // reflex machine (item 3, until 17b cut M4.6): a row differing only
        // there is one state.
        let mut arena = build();
        arena.motors.ensure(ObjId(4)).from_waypoint = Some(1);
        let mut other = build();
        let _ = other.motors.ensure(ObjId(4));
        assert_eq!(
            crate::arena::hash::state_hash(&arena),
            crate::arena::hash::state_hash(&other)
        );
    }

    /// **16b cut O.2** (16c N12), item 1.19: two arenas differing only in one
    /// unit's pod switch differ, and switching it back is the same state again.
    #[test]
    fn o2_the_hash_folds_the_pods_switch() {
        let base = state_hash(&two_tanks());
        let mut silent = two_tanks();
        if let Some(unit) = silent.unit_mut(ObjId(9)) {
            assert!(unit.ecm_on, "every unit checks out radiating");
            unit.ecm_on = false;
        }
        assert_ne!(state_hash(&silent), base, "the switch went unhashed");
        if let Some(unit) = silent.unit_mut(ObjId(9)) {
            unit.ecm_on = true;
        }
        assert_eq!(state_hash(&silent), base);
    }

    /// **17b cut M5.2: the active profile folds only off its default** (item
    /// 1's sub-item 20): a unit at [`crate::arena::card::ActiveProfile::FIRST`]
    /// folds no byte — the arena hashes as one that never had the field
    /// (pinned at `5f505c8`'s value, 17b cut M5.2's review) — and every other
    /// value moves the hash, each from the base and from
    /// every other: a switch to the second profile, a switch BACK to the
    /// first (its stamp kept), a later stamp, and a stamp-less second.
    #[test]
    fn m5_the_active_profile_folds_only_off_its_default() {
        use crate::arena::card::ActiveProfile;
        let base_arena = two_tanks();
        let base = state_hash(&base_arena);
        assert!(
            base_arena
                .units
                .iter()
                .all(|unit| unit.card_profile == ActiveProfile::FIRST),
            "every unit checks out on its first profile"
        );
        // The default folds nothing: the hash is the one the fold below
        // would give with sub-item 20 skipped — two units, no byte each.
        // 17b cut M5.2's review: pinned at the value `5f505c8` gives for the
        // same two tanks (read on a clean `5f505c8` worktree, before the
        // field existed), so a fold that wrote ANY byte for the default — a
        // constant `[0]` included — fails here, not only against itself.
        assert_eq!(
            base, 0x0588_c49e_80b6_962c,
            "the hash of two_tanks since hash v2 (16c T-104; 5f505c8's 0x15a4_d2a7_9200_fae0 before)"
        );
        let probe = |profile: ActiveProfile| {
            let mut arena = two_tanks();
            if let Some(unit) = arena.unit_mut(ObjId(9)) {
                unit.card_profile = profile;
            }
            state_hash(&arena)
        };
        assert_eq!(probe(ActiveProfile::FIRST), base);
        let mut seen = vec![("first, never switched", base)];
        for (what, profile) in [
            (
                "second, switched at 5 s",
                ActiveProfile {
                    index: 1,
                    switched_ms: Some(5_000),
                },
            ),
            (
                "first again, switched at 5 s",
                ActiveProfile {
                    index: 0,
                    switched_ms: Some(5_000),
                },
            ),
            (
                "second, switched at 15 s",
                ActiveProfile {
                    index: 1,
                    switched_ms: Some(15_000),
                },
            ),
            (
                "second, no stamp",
                ActiveProfile {
                    index: 1,
                    switched_ms: None,
                },
            ),
        ] {
            let hash = probe(profile);
            for (other, had) in &seen {
                assert_ne!(hash, *had, "`{what}` folds as `{other}` does");
            }
            seen.push((what, hash));
        }
    }

    /// **17b cut M5.2: STOP E's charge folds inside the `Support` arm only**
    /// (item 3c): two rows holding one `Support` and differing only in
    /// `order_charge` hash differently; a row holding any other order — or
    /// none — folds no byte for it, so the charge can only ever move the hash
    /// of an arena holding a `Support` (never a per-row presence byte).
    #[test]
    fn m5_the_support_charge_folds_inside_the_support_arm_only() {
        use crate::arena::mind::MindState;
        use crate::arena::perception::{FrameOrder, MindObjective};
        let with_row = |objective: Option<MindObjective>, charge: Option<ObjId>| {
            let mut arena = two_tanks();
            arena.minds.insert(
                ObjId(4),
                MindState {
                    order: objective.map(|objective| FrameOrder {
                        squad: "red-1".into(),
                        objective,
                        hold_fire: false,
                        max_range_band: None,
                        expires_ms: 30_000,
                    }),
                    order_charge: charge,
                    ..MindState::default()
                },
            );
            state_hash(&arena)
        };
        let support = Some(MindObjective::Support { of: None });
        assert_ne!(
            with_row(support, Some(ObjId(9))),
            with_row(support, None),
            "the charge of a `Support` is folded"
        );
        assert_ne!(
            with_row(support, Some(ObjId(9))),
            with_row(support, Some(ObjId(5))),
            "the charge's unit is folded"
        );
        for objective in [
            None,
            Some(MindObjective::Hold),
            Some(MindObjective::Engage { target: None }),
            Some(MindObjective::Withdraw),
        ] {
            assert_eq!(
                with_row(objective, Some(ObjId(9))),
                with_row(objective, None),
                "{objective:?}: a charge outside a `Support` folds no byte"
            );
        }
    }

    /// **17b cut M1b.1** (items 3d, 3e, the card inside 3c): every new field
    /// moves the hash, and an arena with none folds no byte for them — the
    /// floor folds nothing at all.
    #[test]
    fn m1b1_slots_held_rows_and_the_card_move_the_hash_and_the_floor_does_not() {
        use crate::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget};
        use crate::arena::state::ContactId;
        use smol_str::SmolStr;
        let base_arena = two_tanks();
        let base = state_hash(&base_arena);
        // The floor is a per-match constant: not folded.
        let mut floor = base_arena.clone();
        floor.odds_floor_permille = 50;
        assert_eq!(state_hash(&floor), base, "the floor folds nothing");
        // Item 3d: a slot, and each of its terms.
        let cond = FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::Lock,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        };
        let armed_with = |cond: FireCondition| {
            let mut arena = base_arena.clone();
            let _ = arena.conditions.arm(ObjId(4), cond, 2);
            state_hash(&arena)
        };
        let armed = armed_with(cond.clone());
        assert_ne!(armed, base, "a slot moves the hash");
        let variants = [
            FireCondition {
                weapon: SmolStr::new("laser"),
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::BestHostile,
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::Contact { id: ContactId(1) },
                ..cond.clone()
            },
            FireCondition {
                target: SlotTarget::Contact { id: ContactId(2) },
                ..cond.clone()
            },
            FireCondition {
                min_permille: 110,
                ..cond.clone()
            },
            FireCondition {
                aspects: 0b0110,
                ..cond.clone()
            },
            FireCondition {
                min_rounds_left: 1,
                ..cond.clone()
            },
            FireCondition {
                min_range_m: 1,
                ..cond.clone()
            },
            FireCondition {
                max_range_m: 1,
                ..cond.clone()
            },
        ];
        let mut seen = vec![base, armed];
        for variant in variants {
            let hash = armed_with(variant.clone());
            assert!(!seen.contains(&hash), "{variant:?} folds as another slot");
            seen.push(hash);
        }
        // The slot's number is folded: the same terms in slot 2.
        let mut second = base_arena.clone();
        let _ = second.conditions.arm(
            ObjId(4),
            FireCondition {
                weapon: SmolStr::new("other"),
                ..cond.clone()
            },
            2,
        );
        let _ = second.conditions.arm(ObjId(4), cond.clone(), 2);
        let _ = second.conditions.disarm(ObjId(4), Some("other"));
        assert_ne!(state_hash(&second), armed, "slot 2 is not slot 1");
        // Item 3e: each field of a held-shot row.
        let held_with = |key: (ObjId, u8, ContactId), at: i64| {
            let mut arena = base_arena.clone();
            arena.held_said.insert(key, at);
            state_hash(&arena)
        };
        let row = held_with((ObjId(4), 1, ContactId(7)), 1_000);
        assert_ne!(row, base);
        for other in [
            held_with((ObjId(9), 1, ContactId(7)), 1_000),
            held_with((ObjId(4), 2, ContactId(7)), 1_000),
            held_with((ObjId(4), 1, ContactId(8)), 1_000),
            held_with((ObjId(4), 1, ContactId(7)), 1_001),
        ] {
            assert_ne!(other, row);
        }
        // The card, inside a mind's row: each section's terms move it.
        let mut minded = base_arena.clone();
        minded
            .minds
            .insert(ObjId(4), crate::arena::mind::MindState::default());
        let card_base = state_hash(&minded);
        // Every one of the card's 24 fields, each edit hashing apart from the
        // base and from every other edit (a dropped or a doubled field folds
        // two cards as one).
        use crate::arena::card::{AspectWord, Opening, OrbitSense, Prefer, Stance};
        let edits: [fn(&mut crate::arena::card::StockCard); 24] = [
            |card| card.stance = Stance::Orbit,
            |card| card.engage_band_permille += 1,
            |card| card.close_m += 1,
            |card| card.orbit_sense = OrbitSense::Cw,
            |card| card.turn_into_hits = !card.turn_into_hits,
            |card| card.turn_s += 1,
            |card| card.screen_bearing += 1,
            |card| card.screen_arc += 1,
            |card| card.opening = Opening::Search,
            |card| card.min_odds_permille += 1,
            |card| card.aspect = AspectWord::Rear,
            |card| card.reserve_pct += 1,
            |card| card.reserve_odds_pct += 1,
            |card| card.prefer = Prefer::BestOdds,
            |card| card.weight_range += 1,
            |card| card.weight_facet += 1,
            |card| card.weight_health += 1,
            |card| card.switch_m += 1,
            |card| card.withdraw_permille += 1,
            |card| card.stand_min_permille += 1,
            |card| card.rally_ms += 1,
            |card| card.search_ms += 1,
            |card| card.hunt_after_ms += 1,
            |card| card.stop_unseen = !card.stop_unseen,
        ];
        let mut seen = vec![card_base];
        for (at, edit) in edits.into_iter().enumerate() {
            let mut edited = minded.clone();
            if let Some(state) = edited.minds.get_mut(&ObjId(4)) {
                edit(&mut state.card);
                assert_ne!(
                    state.card,
                    minded.minds[&ObjId(4)].card,
                    "edit {at} changed nothing"
                );
            }
            let hash = state_hash(&edited);
            assert!(
                !seen.contains(&hash),
                "card edit {at} folds as another card"
            );
            seen.push(hash);
        }
    }

    /// **17b cut M3.6: every part folded** (this file's head, item 3c; D-UM-23
    /// as amended, doc 17 §13's list) — 17b cut M3.2's "carried, not yet
    /// folded" probe inverted. Two arenas differing ONLY in one PART of a
    /// pending think's answer — an intent's presence, an intent's own bytes,
    /// the memory's length, the memory's bytes at one length, any one of the
    /// three `epochs` — or in a mind row's `dropped` (its presence and its
    /// cause) or `seal_digest` hash DIFFERENTLY, each from the base and every
    /// probe from every other (the pattern of the reflex's
    /// `each_new_row_field_moves_the_state_hash`, deleted with it in 17b cut
    /// M4.6). And two differing only in a unit's `seal` (every field of the
    /// `SealRef`, since 17b cut M4.6 its `doctrine` bit too) or in the
    /// arena's class table still hash equal: per-match constants, never
    /// folded — and (17b cut M4.6, the owner's STOP C) two differing only in a
    /// mind row's `order_issuer` hash equal: a record field, never folded. The
    /// presence, `think_seq` and `t_ms` of the pending think move it, as they
    /// did.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn m3_every_part_of_the_answer_the_drop_and_the_seal_digest_moves_the_hash() {
        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::intents::IntentKind;
        use crate::arena::mind::{
            DropCause, MindState, PendingThink, SealEngine, SealRef, SortieId,
        };
        use crate::arena::state::ContactId;
        let mut base = two_tanks();
        base.minds.insert(
            ObjId(4),
            MindState {
                pending: Some(PendingThink {
                    think_seq: 2,
                    t_ms: 2_000,
                    epochs: [0; 3],
                    intents: Vec::new(),
                    memory: Vec::new(),
                }),
                ..MindState::default()
            },
        );
        let before = state_hash(&base);
        let with_mind = |change: &dyn Fn(&mut MindState)| {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&ObjId(4)) {
                change(state);
            }
            assert_ne!(
                arena.minds.get(&ObjId(4)),
                base.minds.get(&ObjId(4)),
                "the probe changed the row"
            );
            state_hash(&arena)
        };
        let pending = |change: &dyn Fn(&mut PendingThink)| {
            with_mind(&|state: &mut MindState| {
                if let Some(pending) = state.pending.as_mut() {
                    change(pending);
                }
            })
        };
        // Folded since 17b cut M3.6, part by part: each probe moves the hash
        // off the base and off every other probe.
        let probes = [
            (
                "pending.intents (one)",
                pending(&|p| {
                    p.intents = vec![IntentKind::Lock {
                        contact: ContactId(7),
                    }];
                }),
            ),
            (
                "pending.intents (its bytes)",
                pending(&|p| {
                    p.intents = vec![IntentKind::Lock {
                        contact: ContactId(8),
                    }];
                }),
            ),
            (
                "pending.intents (two)",
                pending(&|p| {
                    p.intents = vec![
                        IntentKind::Lock {
                            contact: ContactId(7),
                        },
                        IntentKind::Lock {
                            contact: ContactId(7),
                        },
                    ];
                }),
            ),
            (
                "pending.memory (length)",
                pending(&|p| p.memory = vec![1, 2, 3]),
            ),
            (
                "pending.memory (bytes)",
                pending(&|p| p.memory = vec![1, 2, 4]),
            ),
            ("pending.epochs (helm)", pending(&|p| p.epochs = [1, 0, 0])),
            (
                "pending.epochs (sensors)",
                pending(&|p| p.epochs = [0, 1, 0]),
            ),
            ("pending.epochs (fire)", pending(&|p| p.epochs = [0, 0, 1])),
            ("pending.t_ms", pending(&|p| p.t_ms = 2_250)),
            ("pending.think_seq", pending(&|p| p.think_seq = 3)),
            ("pending's presence", with_mind(&|s| s.pending = None)),
            (
                "dropped (strikes)",
                with_mind(&|s| s.dropped = Some(DropCause::Strikes)),
            ),
            (
                "dropped (core)",
                with_mind(&|s| s.dropped = Some(DropCause::Core)),
            ),
            (
                "dropped (abi)",
                with_mind(&|s| s.dropped = Some(DropCause::Abi)),
            ),
            ("seal_digest", with_mind(&|s| s.seal_digest = [7; 32])),
            (
                "seal_digest (one byte)",
                with_mind(&|s| s.seal_digest[31] = 1),
            ),
        ];
        let mut seen: Vec<(&str, u64)> = vec![("base", before)];
        for (field, hash) in probes {
            for (other, had) in &seen {
                assert_ne!(hash, *had, "`{field}` folds as `{other}` does");
            }
            seen.push((field, hash));
        }

        // Never folded: the seal reference, every field of it. Since 17b cut
        // M3.3 the fixture hull carries its check-out seal (sortie, digest,
        // class I on its class I core), so each probe is held to differ from
        // the base unit's OWN seal — not from `SealRef::none()` — and every
        // probe writes a value the base does not already hold.
        let was = base.unit(ObjId(4)).map(|unit| unit.seal);
        assert_ne!(was, Some(SealRef::none()), "the fixture hull is sealed");
        let sealed = |change: &dyn Fn(&mut SealRef)| {
            let mut arena = base.clone();
            if let Some(unit) = arena.unit_mut(ObjId(4)) {
                change(&mut unit.seal);
                assert_ne!(Some(unit.seal), was, "the probe changed the seal");
            }
            state_hash(&arena)
        };
        for (field, hash) in [
            (
                "seal.sortie",
                sealed(&|s| s.sortie = SortieId::in_match(7, ObjId(4))),
            ),
            ("seal.digest", sealed(&|s| s.digest = [9; 32])),
            ("seal.core_class", sealed(&|s| s.core_class = CoreClass::II)),
            ("seal.class", sealed(&|s| s.class = CoreClass::Zero)),
            ("seal.engine", sealed(&|s| s.engine = SealEngine::Scripted)),
            ("seal.governed", sealed(&|s| s.governed = false)),
            ("seal.closed", sealed(&|s| s.closed = true)),
            // 17b cut M4.6: the doctrine bit (the fixture's seal is Standard
            // Doctrine's, so the base reads `true`).
            ("seal.doctrine", sealed(&|s| s.doctrine = false)),
            // 17b cut M5.2: the sealed set (the fixture's is the Line card
            // alone, so the base reads `LINE`).
            (
                "seal.profiles",
                sealed(&|s| {
                    s.profiles = crate::arena::card::ProfileSet {
                        len: 2,
                        codes: [2, 0, 0, 0],
                    };
                }),
            ),
        ] {
            assert_eq!(hash, before, "`{field}` is folded");
        }
        // Never folded: the class table.
        let mut classes = base.clone();
        let stock = ClassTable::STOCK;
        let mut wire = serde_json::to_value(stock).expect("the table encodes");
        wire["II"]["slots"] = serde_json::json!(5);
        classes.classes = serde_json::from_value(wire).expect("a lawful table");
        assert_ne!(classes.classes, stock, "the probe changed the table");
        assert_eq!(state_hash(&classes), before, "the class table is folded");
        // Never folded (17b cut M4.6, the owner's STOP C ruling): the issuer
        // beside a mind's order. Two rows holding one order, differing only in
        // who issued it, are one state — and the order itself is folded, so
        // the probe is not vacuous.
        let ordered = |issuer: Option<ObjId>| {
            let mut arena = base.clone();
            if let Some(state) = arena.minds.get_mut(&ObjId(4)) {
                state.order = Some(crate::arena::perception::FrameOrder {
                    squad: "red-1".into(),
                    objective: crate::arena::perception::MindObjective::Hold,
                    hold_fire: false,
                    max_range_band: None,
                    expires_ms: 30_000,
                });
                state.order_issuer = issuer;
            }
            arena
        };
        let by_commander = ordered(None);
        assert_ne!(state_hash(&by_commander), before, "the order is folded");
        let by_pilot = ordered(Some(ObjId(9)));
        assert_ne!(
            by_pilot.minds.get(&ObjId(4)),
            by_commander.minds.get(&ObjId(4)),
            "the probe changed the row"
        );
        assert_eq!(
            state_hash(&by_pilot),
            state_hash(&by_commander),
            "`order_issuer` is folded"
        );
    }

    /// [`t104_the_fold_s_vectors`]'s pins.
    const FOLD_EMPTY: u64 = 0xf937_6fe9_0135_a10e;
    const FOLD_A: u64 = 0x4681_e5df_70d4_9b0a;
    const FOLD_WORD: u64 = 0x2e0f_7611_7070_06f3;
    const FOLD_WORD_AND_BYTE: u64 = 0xdf88_2b87_20a5_a1fb;
    const FOLD_ALL_BYTES: u64 = 0x7140_4dd0_7980_1636;

    // ---- 16c T-104, hash v2: the fold and the field fold ----------------

    fn folded(bytes: &[u8]) -> u64 {
        let mut fold = Fold::new();
        fold.bytes(bytes);
        fold.finish()
    }

    /// **A call is one message**: the hash is a function of the sequence of
    /// messages the callers' fixed field order makes — two `u32` fields are
    /// not one `u64`, a byte is not a zero-padded word, nine bytes are not
    /// eight and one, an empty message counts — and the same messages in
    /// the same order fold the same.
    #[test]
    fn t104_a_call_is_one_message() {
        let mut two = Fold::new();
        two.bytes(&1_u32.to_le_bytes());
        two.bytes(&2_u32.to_le_bytes());
        let mut one = Fold::new();
        one.bytes(&[1, 0, 0, 0, 2, 0, 0, 0]);
        assert_ne!(two.finish(), one.finish(), "two words, not one");
        assert_ne!(folded(&[7]), folded(&[7, 0]), "a byte, not a padded word");
        assert_ne!(folded(b""), folded(&[0]), "empty, not a zero byte");
        let mut split = Fold::new();
        split.bytes(b"abcdefgh");
        split.bytes(b"i");
        assert_ne!(
            split.finish(),
            folded(b"abcdefghi"),
            "nine, not eight and one"
        );
        let mut with_empty = Fold::new();
        with_empty.bytes(b"ab");
        with_empty.bytes(b"");
        assert_ne!(
            with_empty.finish(),
            folded(b"ab"),
            "an empty message counts"
        );
        let mut again = Fold::new();
        again.bytes(&1_u32.to_le_bytes());
        again.bytes(&2_u32.to_le_bytes());
        assert_eq!(again.finish(), two.finish());
        assert_ne!(
            Fold::new().finish(),
            folded(b""),
            "no message is not an empty one"
        );
    }

    /// **Every bit and every length counts**: one flipped bit anywhere in a
    /// 64-byte stream moves the hash, a trailing zero byte moves it, and the
    /// empty stream is not a zero-byte stream.
    #[test]
    fn t104_the_fold_sees_every_bit_and_the_length() {
        let base: Vec<u8> = (0..64_u8).collect();
        let whole = folded(&base);
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(whole);
        for byte in 0..base.len() {
            for bit in 0..8 {
                let mut flipped = base.clone();
                flipped[byte] ^= 1 << bit;
                let hash = folded(&flipped);
                assert_ne!(hash, whole, "byte {byte} bit {bit}");
                assert!(seen.insert(hash), "byte {byte} bit {bit} collided");
            }
        }
        let word = [0_u8, 1, 2, 3, 4, 5, 6, 7];
        let word_hash = folded(&word);
        for byte in 0..8 {
            for bit in 0..8 {
                let mut flipped = word;
                flipped[byte] ^= 1 << bit;
                assert_ne!(folded(&flipped), word_hash, "word byte {byte} bit {bit}");
            }
        }
        let mut longer = base.clone();
        longer.push(0);
        assert_ne!(folded(&longer), whole, "a trailing zero byte");
        assert_ne!(folded(b""), folded(&[0]), "empty against one zero");
        assert_ne!(
            folded(&[0; 8]),
            folded(&[0; 16]),
            "one zero word against two"
        );
        assert_ne!(folded(&[0; 9]), folded(&[0; 10]), "nine zeros against ten");
    }

    /// **The fold's vectors**, written out so a toolchain cannot move the
    /// mix without a diff here: the empty stream, one byte, one word, a word
    /// and a byte, and the 256 byte values in order.
    #[test]
    fn t104_the_fold_s_vectors() {
        let all: Vec<u8> = (0..=255_u8).collect();
        let vectors = [
            (&b""[..], FOLD_EMPTY),
            (&b"a"[..], FOLD_A),
            (&b"tacsim!!"[..], FOLD_WORD),
            (&b"tacsim!!x"[..], FOLD_WORD_AND_BYTE),
            (all.as_slice(), FOLD_ALL_BYTES),
        ];
        let read: Vec<u64> = vectors.iter().map(|(stream, _)| folded(stream)).collect();
        for ((stream, _), hash) in vectors.iter().zip(&read) {
            println!("t104_fold_vector: {stream:?} -> {hash:#018x}");
        }
        for ((stream, pinned), hash) in vectors.iter().zip(&read) {
            assert_eq!(hash, pinned, "{stream:?}");
        }
    }

    /// **The field fold is serde's walk**: every field and every variant
    /// counts — a notice with one field changed folds differently, an
    /// option's presence folds, an empty and a one-element sequence differ —
    /// and on the frame module's notices it tells apart every pair the
    /// canonical JSON told apart.
    #[test]
    fn t104_the_field_fold_counts_every_field() {
        use crate::arena::perception::FrameNotice;
        use smol_str::SmolStr;
        fn fv<T: serde::Serialize + ?Sized>(value: &T) -> u64 {
            let mut fold = Fold::new();
            fold_value(&mut fold, value);
            fold.finish()
        }
        assert_ne!(fv(&Some(1_u8)), fv(&None::<u8>));
        assert_ne!(fv(&Vec::<u8>::new()), fv(&vec![0_u8]));
        assert_ne!(fv(&(1_u8, 2_u8)), fv(&(2_u8, 1_u8)));
        assert_ne!(fv("ab"), fv("ba"));
        assert_eq!(
            fv(&[1_u8, 2, 3][..]),
            fv(&vec![1_u8, 2, 3]),
            "a slice is a sequence"
        );
        assert_ne!(
            fv(&[1_u8, 2, 3]),
            fv(&vec![1_u8, 2, 3]),
            "an array is a tuple: no length"
        );
        let s = SmolStr::new_static;
        let struck = |armor_left: u32, facet: &'static str| FrameNotice::Struck {
            section: s("hull"),
            armor_section: s("hull"),
            facet: s(facet),
            armor_left,
            armor_max: 40,
            internals_left: 12,
            internals_max: 12,
        };
        let notices: Vec<FrameNotice> = vec![
            FrameNotice::Miss {
                contact: None,
                reason: s("deflected"),
                launch_cell: None,
            },
            FrameNotice::Miss {
                contact: None,
                reason: s("wide"),
                launch_cell: None,
            },
            FrameNotice::Miss {
                contact: None,
                reason: s("deflected"),
                launch_cell: None,
            },
            FrameNotice::Miss {
                contact: None,
                reason: s("deflected"),
                launch_cell: Some((1, 2)),
            },
            FrameNotice::Miss {
                contact: None,
                reason: s("deflected"),
                launch_cell: Some((2, 1)),
            },
            FrameNotice::Kill { contact: None },
            FrameNotice::Hit {
                contact: None,
                section: None,
                spent: false,
            },
            FrameNotice::Hit {
                contact: None,
                section: None,
                spent: true,
            },
            FrameNotice::Hit {
                contact: None,
                section: Some(s("hull")),
                spent: false,
            },
            FrameNotice::StruckSpent { section: s("hull") },
            FrameNotice::StruckSpent {
                section: s("turret"),
            },
            FrameNotice::SectionLost {
                section: s("hull"),
                consequences: Vec::new(),
            },
            FrameNotice::SectionLost {
                section: s("hull"),
                consequences: vec![s("destroy_unit")],
            },
            struck(39, "front"),
            struck(38, "front"),
            struck(39, "left"),
        ];
        assert!(notices.len() >= 4, "the corpus");
        let mut by_json: std::collections::BTreeMap<Vec<u8>, u64> =
            std::collections::BTreeMap::new();
        for notice in &notices {
            let json = serde_json::to_vec(notice).expect("a notice serialises");
            let hash = fv(notice);
            if let Some(seen) = by_json.insert(json, hash) {
                assert_eq!(seen, hash, "equal JSON, equal fold: {notice:?}");
            }
        }
        let distinct: std::collections::BTreeSet<u64> = by_json.values().copied().collect();
        assert_eq!(
            distinct.len(),
            by_json.len(),
            "every pair the JSON told apart, the fold tells apart"
        );
    }
}
