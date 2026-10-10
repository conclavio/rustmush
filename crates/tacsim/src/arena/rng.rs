//! **The arena's randomness** (16b cut A.1; doc 16 §3.5 "Draw labels", §3.6;
//! doc 09 C5).
//!
//! Every random value a match uses is `draw(match_seed, label)` — a keyed hash of
//! the match seed and the canonical bytes of an **enumerated** label. There is no
//! stream and no generator state, so:
//!
//! - **Adding a draw can never shift another.** A new label is a new key; it does
//!   not consume a position that some other resolution was going to read. That is
//!   what makes cut W's hit draw, cut S's classification draw and cut A's contact
//!   id independent, and it is why doc 16 §3.1's rate-independence property can
//!   hold at all: a draw's value does not depend on how many pulses have run.
//! - **A replay is exact.** Same seed, same label, same value, on every platform
//!   and every compiler version — because the mix below is written out here
//!   rather than borrowed. `std`'s `DefaultHasher` is explicitly *not* stable
//!   across releases and is therefore not usable for authoritative state; `rand`
//!   is a stream and has the position problem.
//!
//! The label set is pinned by `crates/mush-contract-tests/tests/arena_library.rs`,
//! and the arena is exempt from `provider_rng_policy` by construction: it
//! implements no action provider, so there is no `PrepareCtx` for it to draw
//! from (doc 16 §3.5).

use mush_core::ObjId;

use crate::arena::hash::{FNV_OFFSET_BASIS, fnv1a64_update};
use crate::arena::state::ContactId;

/// **Every draw a match may make** (doc 16 §3.5 "Draw labels", §3.6).
///
/// One variant per reason, each carrying exactly the arguments that make it
/// unique within a match. Two draws with the same label are the same value **on
/// purpose**: doc 16 §3.5's re-validation at impact re-resolves a target that
/// changed facet band *with the same `hit` draw*, which only works because the
/// label and not a stream position is the identity of the roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrawLabel {
    /// Whether one shot hits (cut W.2).
    Hit {
        /// The shot's sequence number.
        shot_seq: u64,
    },
    /// Which section a hit strikes, through the facet table (cut W.2).
    Facet {
        /// The shot's sequence number.
        shot_seq: u64,
    },
    /// How many rounds of a burst land (cut W.2).
    Burst {
        /// The shot's sequence number.
        shot_seq: u64,
    },
    /// Whether an observer tells a target apart on one pass (cut S).
    Classify {
        /// The observing unit.
        observer: ObjId,
        /// The observed unit.
        target: ObjId,
        /// The pass's sequence number.
        pass_seq: u64,
    },
    /// The opaque id a viewer platform holds a target under (doc 16 §3.6).
    Contact {
        /// The viewer platform.
        viewer: ObjId,
        /// The held unit.
        target: ObjId,
    },
    /// **A unit mind's `n`-th draw of one think** (17b cut M2.2; doc 17 §7,
    /// D-UM-29): what a scripted mind's `rand(k)` is keyed on — the hull, the
    /// think and the draw's place in it, so a mind's draws are a function of
    /// the match seed and its own thinks and never shift another draw. Pinned
    /// here in 17b cut M2.2; nothing draws it until 17b cut M2.3 registers
    /// `rand(k)`.
    Mind {
        /// The hull thinking — its own object number (the view's `unit`).
        unit: u32,
        /// The think's sequence number.
        think_seq: u64,
        /// The draw's place in the think, from 0.
        n: u32,
    },
}

impl DrawLabel {
    /// **The pinned label kinds**, in code order. A contract test asserts this
    /// exact array, so a seventh reason to draw is a reviewed diff (the sixth,
    /// `mind`, was 17b cut M2.2's; D-UM-29).
    pub const LABEL_KINDS: [&'static str; 6] =
        ["hit", "facet", "burst", "classify", "contact", "mind"];

    /// This label's kind word — the first colon-separated field of
    /// [`DrawLabel::encode`].
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            DrawLabel::Hit { .. } => "hit",
            DrawLabel::Facet { .. } => "facet",
            DrawLabel::Burst { .. } => "burst",
            DrawLabel::Classify { .. } => "classify",
            DrawLabel::Contact { .. } => "contact",
            DrawLabel::Mind { .. } => "mind",
        }
    }

    /// **The canonical encoding** the draw is keyed on, and the string the ledger
    /// records beside the value:
    ///
    /// ```text
    /// hit:<shot_seq>
    /// facet:<shot_seq>
    /// burst:<shot_seq>
    /// classify:<observer>:<target>:<pass_seq>
    /// contact:<viewer>:<target>
    /// mind:<unit>:<think_seq>:<n>
    /// ```
    ///
    /// Object ids are their **decimal numbers**, not their `#42` display form:
    /// the label is a key, and a key with a sigil in it invites the sigil to
    /// change. The kind words are disjoint prefixes and every field is decimal
    /// digits, so no two distinct labels encode alike.
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            DrawLabel::Hit { shot_seq } => format!("hit:{shot_seq}"),
            DrawLabel::Facet { shot_seq } => format!("facet:{shot_seq}"),
            DrawLabel::Burst { shot_seq } => format!("burst:{shot_seq}"),
            DrawLabel::Classify {
                observer,
                target,
                pass_seq,
            } => format!("classify:{}:{}:{pass_seq}", observer.0, target.0),
            DrawLabel::Contact { viewer, target } => {
                format!("contact:{}:{}", viewer.0, target.0)
            }
            DrawLabel::Mind { unit, think_seq, n } => format!("mind:{unit}:{think_seq}:{n}"),
        }
    }

    /// **The exact inverse of [`DrawLabel::encode`]** (16b cut M.3; 16c `T-07`,
    /// `T-12`): the label a ledger line's `draws` entry names, back as the value
    /// [`draw`] is keyed on, so a reader holding the match seed can re-derive the
    /// draw — which section a round found, whether a pass told a target apart.
    ///
    /// Split on `:`, the kind word first, every field decimal. Anything else is
    /// `None`: an unknown kind, a wrong field count, a field that is not decimal
    /// digits or does not fit its type — and any spelling `encode` would not
    /// have written (a leading `+` or zero), because the check is that encoding
    /// the answer gives the text back, byte for byte. So `decode(s) == Some(l)`
    /// exactly when `l.encode() == s`.
    ///
    /// It is the one place a pass sequence can be read back from the record:
    /// `EventKind::Classified` carries the viewer, the contact, the target and the
    /// side, and the `pass_seq` its draw was keyed on is written only in the
    /// label string beside it.
    #[must_use]
    pub fn decode(text: &str) -> Option<DrawLabel> {
        let mut fields = text.split(':');
        let kind = fields.next()?;
        let rest: Vec<&str> = fields.collect();
        let seq = |at: usize| -> Option<u64> { decimal(rest.get(at)?) };
        let id = |at: usize| -> Option<ObjId> { u32::try_from(seq(at)?).ok().map(ObjId) };
        let label = match (kind, rest.len()) {
            ("hit", 1) => DrawLabel::Hit { shot_seq: seq(0)? },
            ("facet", 1) => DrawLabel::Facet { shot_seq: seq(0)? },
            ("burst", 1) => DrawLabel::Burst { shot_seq: seq(0)? },
            ("classify", 3) => DrawLabel::Classify {
                observer: id(0)?,
                target: id(1)?,
                pass_seq: seq(2)?,
            },
            ("contact", 2) => DrawLabel::Contact {
                viewer: id(0)?,
                target: id(1)?,
            },
            ("mind", 3) => DrawLabel::Mind {
                unit: u32::try_from(seq(0)?).ok()?,
                think_seq: seq(1)?,
                n: u32::try_from(seq(2)?).ok()?,
            },
            _ => return None,
        };
        (label.encode() == text).then_some(label)
    }
}

/// One decimal field of an encoded label: ASCII digits only, parsed as `u64`.
/// The canonical-spelling check is [`DrawLabel::decode`]'s round trip.
fn decimal(field: &str) -> Option<u64> {
    if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

/// **The pinned label kinds**, as a module-level constant for callers that do not
/// want the type. The same array as [`DrawLabel::LABEL_KINDS`].
pub const LABEL_KINDS: [&str; 6] = DrawLabel::LABEL_KINDS;

/// **`draw(seed, label)`** — the keyed 64-bit value doc 16 §3.5 asks for.
///
/// FNV-1a 64 over `seed.to_le_bytes()` followed by the label's encoded bytes,
/// finished through [`splitmix64`]. Both halves are written out in this file:
/// FNV-1a's avalanche is poor in the high bits, which matters because callers
/// take `% 1000` and slice ranges out of the result, and splitmix64's finalizer
/// fixes that without introducing any state. Neither depends on the standard
/// library's hashing, the platform's endianness or the compiler's version, so a
/// ledger replayed on another machine draws the same numbers.
#[must_use]
pub fn draw(seed: u64, label: &DrawLabel) -> u64 {
    let mut state = fnv1a64_update(FNV_OFFSET_BASIS, &seed.to_le_bytes());
    state = fnv1a64_update(state, label.encode().as_bytes());
    splitmix64(state)
}

/// A per-mille draw in `0..1000` — the form doc 16 §3.5's no-dice resolution
/// compares against `p_permille` (D-16-9).
#[must_use]
pub fn draw_permille(seed: u64, label: &DrawLabel) -> u16 {
    u16::try_from(draw(seed, label) % 1_000).unwrap_or(0)
}

/// **The opaque contact id** a viewer platform holds a target under (doc 16
/// §3.6): `draw(seed, contact:<viewer>:<target>)`, and **never zero**.
///
/// Zero is reserved so that a caller can tell "no contact" from a minted one
/// without an `Option` inside a packed frame. A draw of exactly zero is replaced
/// by its [`splitmix64`] successor, and a successor of zero (which the mix does
/// not produce for this input, but which is cheaper to exclude than to argue
/// about) by `1`.
#[must_use]
pub fn draw_contact_id(seed: u64, viewer: ObjId, target: ObjId) -> ContactId {
    let mut value = draw(seed, &DrawLabel::Contact { viewer, target });
    if value == 0 {
        value = splitmix64(value);
    }
    if value == 0 {
        value = 1;
    }
    ContactId(value)
}

/// **splitmix64's finalizer**, written out: the gamma step and the two
/// multiply-xorshift rounds, in the canonical constants.
///
/// It is public because [`draw_contact_id`]'s "never zero" rule names it and a
/// rule a test cannot reach is not a rule.
#[must_use]
pub const fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// **Where in its cadence a unit's recurring event first falls** (16b cut U.1;
/// 16c `T-25`): `splitmix64(seed ^ id) % cadence`, in `0..cadence` arena
/// milliseconds. A `cadence_ms` below 1 is read as 1, so the answer is `0`.
///
/// It is what `Arena::schedule_first_passes` offsets each unit's first sensor
/// pass by and what `Arena::grid_at` offsets each hull's motor and think grid
/// by (`Arena::schedule_reflex_tick`'s decide grid until 17b cut M4.6 deleted
/// the reflex controller; doc 16 §3.6 *staggered by unit id*, read as *keyed
/// on it*). The
/// stagger was `id % cadence`, and a match's object numbers are contiguous, so
/// every first pass fell inside the first `units` milliseconds of each second —
/// 300 units put all 300 passes in the first 300 ms, and the worst pulse
/// carried every pass while the rest of the second carried none. Mixed through
/// [`splitmix64`] the offsets spread over the whole cadence.
///
/// **Not a draw.** It takes no [`DrawLabel`], so the pinned label set does not
/// move, and it is not in the ledger: an offset is a function of the match seed
/// and the unit's number and nothing else — no state, no stream, the same on a
/// replay and the same at every tempo, because the cadence it is taken against
/// is a configuration value and never the tempo's period.
#[must_use]
pub fn stagger_ms(seed: u64, id: ObjId, cadence_ms: i64) -> i64 {
    let cadence = cadence_ms.max(1);
    i64::try_from(splitmix64(seed ^ u64::from(id.0)) % u64::try_from(cadence).unwrap_or(1))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One label of every kind, in [`DrawLabel::LABEL_KINDS`] order.
    fn one_of_each() -> [DrawLabel; 6] {
        [
            DrawLabel::Hit { shot_seq: 1 },
            DrawLabel::Facet { shot_seq: 1 },
            DrawLabel::Burst { shot_seq: 1 },
            DrawLabel::Classify {
                observer: ObjId(4),
                target: ObjId(9),
                pass_seq: 3,
            },
            DrawLabel::Contact {
                viewer: ObjId(4),
                target: ObjId(9),
            },
            DrawLabel::Mind {
                unit: 4,
                think_seq: 2,
                n: 0,
            },
        ]
    }

    #[test]
    fn the_label_kinds_are_pinned_and_every_variant_declares_one() {
        // 17b cut M2.2 (D-UM-29): five -> six, `mind` last.
        assert_eq!(
            LABEL_KINDS,
            ["hit", "facet", "burst", "classify", "contact", "mind"]
        );
        assert_eq!(LABEL_KINDS, DrawLabel::LABEL_KINDS);
        for (label, kind) in one_of_each().iter().zip(LABEL_KINDS) {
            assert_eq!(label.kind(), kind);
            assert!(label.encode().starts_with(kind));
        }
    }

    #[test]
    fn the_encodings_are_the_pinned_strings() {
        assert_eq!(DrawLabel::Hit { shot_seq: 7 }.encode(), "hit:7");
        assert_eq!(DrawLabel::Facet { shot_seq: 7 }.encode(), "facet:7");
        assert_eq!(DrawLabel::Burst { shot_seq: 7 }.encode(), "burst:7");
        assert_eq!(
            DrawLabel::Classify {
                observer: ObjId(4),
                target: ObjId(9),
                pass_seq: 3
            }
            .encode(),
            "classify:4:9:3"
        );
        assert_eq!(
            DrawLabel::Contact {
                viewer: ObjId(4),
                target: ObjId(9)
            }
            .encode(),
            "contact:4:9"
        );
        assert_eq!(
            DrawLabel::Mind {
                unit: 4,
                think_seq: 2,
                n: 7
            }
            .encode(),
            "mind:4:2:7"
        );
    }

    /// **16b cut M.3** (16c `T-07`): `decode` is the exact inverse of `encode` —
    /// one label of every kind comes back as itself, with a field at its type's
    /// ceiling as well as a small one — and anything `encode` would not have
    /// written is `None`.
    #[test]
    fn m3_decode_is_the_exact_inverse_of_encode() {
        let ceiling = [
            DrawLabel::Hit { shot_seq: u64::MAX },
            DrawLabel::Facet { shot_seq: 0 },
            DrawLabel::Burst { shot_seq: 12 },
            DrawLabel::Classify {
                observer: ObjId(u32::MAX),
                target: ObjId(9),
                pass_seq: u64::MAX,
            },
            DrawLabel::Contact {
                viewer: ObjId(0),
                target: ObjId(u32::MAX),
            },
            DrawLabel::Mind {
                unit: u32::MAX,
                think_seq: u64::MAX,
                n: u32::MAX,
            },
        ];
        for label in one_of_each().into_iter().chain(ceiling) {
            assert_eq!(
                DrawLabel::decode(&label.encode()),
                Some(label),
                "{label:?} did not round-trip"
            );
        }
        for foreign in [
            "",
            "hit",
            "hit:",
            "hit:x",
            "hit:1:2",
            "hit:+1",
            "hit:01",
            "hit:-1",
            "hit: 1",
            "Hit:1",
            "dice:1",
            "facet:18446744073709551616",
            "classify:4:9",
            "classify:4:9:3:1",
            "classify:4294967296:9:3",
            "contact:4",
            "contact:4:9:1",
            "contact:#4:9",
            "mind:4:2",
            "mind:4:2:7:1",
            "mind:4294967296:2:7",
            "mind:4:2:4294967296",
            "mind:4:02:7",
        ] {
            assert_eq!(DrawLabel::decode(foreign), None, "{foreign:?} decoded");
        }
    }

    #[test]
    fn the_encodings_do_not_collide_across_kinds_or_arguments() {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for label in one_of_each() {
            assert!(seen.insert(label.encode()), "{label:?} collided");
        }
        // The three shot-keyed kinds share an argument and must still differ.
        assert_ne!(
            DrawLabel::Hit { shot_seq: 7 }.encode(),
            DrawLabel::Facet { shot_seq: 7 }.encode()
        );
        // And the two-id kinds are order-sensitive.
        assert_ne!(
            DrawLabel::Contact {
                viewer: ObjId(4),
                target: ObjId(9)
            }
            .encode(),
            DrawLabel::Contact {
                viewer: ObjId(9),
                target: ObjId(4)
            }
            .encode()
        );
        // Multi-field labels cannot be confused by concatenation: 4:9:3 and
        // 49:3 are different strings, which is why the separator is mandatory.
        assert_ne!(
            DrawLabel::Classify {
                observer: ObjId(4),
                target: ObjId(9),
                pass_seq: 3
            }
            .encode(),
            DrawLabel::Classify {
                observer: ObjId(49),
                target: ObjId(3),
                pass_seq: 3
            }
            .encode()
        );
    }

    /// **The pinned draw values.** These two literals are the whole
    /// cross-platform, cross-version stability claim: if a toolchain or an
    /// architecture changes what `draw` answers, this test fails and a replayed
    /// ledger is known to be untrustworthy rather than quietly wrong.
    #[test]
    fn draw_is_stable_at_pinned_values() {
        assert_eq!(
            draw(2350, &DrawLabel::Hit { shot_seq: 1 }),
            12_072_725_193_963_834_385
        );
        assert_eq!(
            draw(
                2350,
                &DrawLabel::Contact {
                    viewer: ObjId(4),
                    target: ObjId(9)
                }
            ),
            5_852_833_454_602_713_067
        );
        // And the mix itself, so a failure says which half moved.
        assert_eq!(splitmix64(0), 16_294_208_416_658_607_535);
        assert_eq!(
            fnv1a64_update(FNV_OFFSET_BASIS, b"hit:1"),
            14_058_921_558_142_482_227
        );
    }

    #[test]
    fn draw_is_a_function_of_the_seed_and_the_label_only() {
        let label = DrawLabel::Hit { shot_seq: 1 };
        // Repeatable.
        assert_eq!(draw(2350, &label), draw(2350, &label));
        // Keyed on the seed.
        assert_ne!(draw(2350, &label), draw(2351, &label));
        // Keyed on the label, including on a one-digit argument change.
        assert_ne!(
            draw(2350, &label),
            draw(2350, &DrawLabel::Hit { shot_seq: 2 })
        );
        assert_ne!(
            draw(2350, &label),
            draw(2350, &DrawLabel::Facet { shot_seq: 1 })
        );
    }

    #[test]
    fn adding_a_label_shifts_no_existing_draw() {
        // The property doc 16 §3.5 states: a draw is a keyed hash, so drawing a
        // hundred other labels between two reads of one label changes nothing.
        let label = DrawLabel::Hit { shot_seq: 1 };
        let before = draw(2350, &label);
        for seq in 0..100_u64 {
            let _ = draw(2350, &DrawLabel::Burst { shot_seq: seq });
            let _ = draw(
                2350,
                &DrawLabel::Classify {
                    observer: ObjId(4),
                    target: ObjId(9),
                    pass_seq: seq,
                },
            );
        }
        assert_eq!(draw(2350, &label), before);
    }

    #[test]
    fn the_permille_draw_is_in_range_and_spreads() {
        let mut buckets = [0_usize; 10];
        for seq in 0..1_000_u64 {
            let value = draw_permille(2350, &DrawLabel::Hit { shot_seq: seq });
            assert!(value < 1_000, "{value} is not a per-mille");
            let bucket = usize::from(value / 100);
            if let Some(slot) = buckets.get_mut(bucket) {
                *slot += 1;
            }
        }
        // Not a statistical test: only that no decile is empty, which a broken
        // mix (a constant, or a low-bit-only hash) would fail outright.
        assert!(buckets.iter().all(|count| *count > 0), "{buckets:?}");
    }

    /// **16b cut U.1** (16c `T-25`): the stagger is inside its cadence, keyed on
    /// the seed and the id, a nought or negative cadence answers nought, and
    /// three hundred contiguous ids no longer bunch into the first three hundred
    /// milliseconds of a second.
    #[test]
    fn the_stagger_is_inside_its_cadence_and_spreads_contiguous_ids() {
        assert_eq!(stagger_ms(2350, ObjId(4), 0), 0);
        assert_eq!(stagger_ms(2350, ObjId(4), -5), 0);
        assert_eq!(stagger_ms(2350, ObjId(4), 1), 0);
        assert_eq!(
            stagger_ms(2350, ObjId(4), 1_000),
            i64::try_from(splitmix64(2350 ^ 4) % 1_000).unwrap_or(-1)
        );
        assert_ne!(
            (0..24)
                .map(|n| stagger_ms(2350, ObjId(n), 1_000))
                .collect::<Vec<_>>(),
            (0..24)
                .map(|n| stagger_ms(2351, ObjId(n), 1_000))
                .collect::<Vec<_>>()
        );
        let offsets: Vec<i64> = (100_000..100_300)
            .map(|n| stagger_ms(2350, ObjId(n), 1_000))
            .collect();
        assert!(offsets.iter().all(|at| (0..1_000).contains(at)));
        assert!(
            offsets.iter().filter(|at| **at >= 300).count() > 150,
            "the offsets bunch under 300 ms"
        );
    }

    #[test]
    fn contact_ids_are_never_zero_and_are_per_pair() {
        let first = draw_contact_id(2350, ObjId(4), ObjId(9));
        assert_ne!(first.0, 0);
        assert_eq!(first, draw_contact_id(2350, ObjId(4), ObjId(9)));
        // Per viewer, per target, and per match.
        assert_ne!(first, draw_contact_id(2350, ObjId(4), ObjId(10)));
        assert_ne!(first, draw_contact_id(2350, ObjId(5), ObjId(9)));
        assert_ne!(first, draw_contact_id(2351, ObjId(4), ObjId(9)));
        // The zero rule's escape hatch is itself non-zero.
        assert_ne!(splitmix64(0), 0);
        // Over a 12 v 12 match's worth of pairs, no id is zero and none repeats.
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        for viewer in 1..=24_u32 {
            for target in 1..=24_u32 {
                let id = draw_contact_id(2350, ObjId(viewer), ObjId(target));
                assert_ne!(id.0, 0);
                assert!(seen.insert(id.0), "#{viewer} / #{target} collided");
            }
        }
        assert_eq!(seen.len(), 576);
    }
}
