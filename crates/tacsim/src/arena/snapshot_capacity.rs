//! Recovery capacity at the immutable opening, before checkout publication.
//!
//! The harness's measured threshold is not an admission rule. This module
//! bounds the complete storage document and exposes each reserved contribution.
//! Full source and custody seals live outside Snapshot: their authoring/catalog
//! limits and the complete TapeHeader byte cap remain independently enforced.
use std::io::{self, Write};

use serde::Serialize;

use super::snapshot::{Snapshot, SnapshotError};
use super::state::Arena;

/// Deliberately below Redis's default 512 MiB RESP argument ceiling. The ordinary
/// writer sends one JSON.SET argument per document, not the 16 MiB gateway IPC.
pub const SNAPSHOT_MAX_BYTES: u64 = 480 * 1024 * 1024;
/// Smallest configured production ceiling.
pub const SNAPSHOT_MIN_BYTES: u64 = 16 * 1024 * 1024;

/// Auditable accounting, in bytes of the *outer encoded document*. These are
/// ceilings, not predicted allocations, and must not be reported as measured RAM.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct SnapshotBudget {
    pub opening: u64,
    pub catalog_growth: u64,
    pub scalar_growth: u64,
    pub memory: u64,
    pub routes: u64,
    pub hull_tables: u64,
    pub contact_tables: u64,
    pub spent_sections: u64,
    pub projectiles: u64,
    pub objective_events: u64,
    pub ledger_tail: u64,
    pub total: u64,
    pub limit: u64,
}

/// The fixed typed hull envelope covers two six-row rule representations, the
/// sixteen bounded station intents of a pending answer, sixteen notices, motor
/// bookkeeping/avoid cells, order/catchup, epochs and scheduled per-hull work.
/// Memory, path cells, physical-unit scalar growth and per-contact entries are
/// separate below. Rule symbols are <=64 ASCII bytes and conjunctions <=4.
const HULL_TABLE_BYTES: u64 = 160 * 1024;
/// One viewer/target pair: shadow plot + own sighting + two packed readings,
/// designator book, range memory, release contact, held-loss row and two stale
/// one-pass LossSettles events. Each held-shot slot adds its own compact key.
const CONTACT_PAIR_BYTES: u64 = 1344;
const HELD_SLOT_PAIR_BYTES: u64 = 64;
const PROJECTILE_BYTES: u64 = 4096;
const DUE_EVENT_BYTES: u64 = 256;

fn capacity(limit: u64, required_at_least: u64) -> SnapshotError {
    SnapshotError::Capacity {
        required_at_least,
        limit,
    }
}

fn sum(a: u64, b: u64, limit: u64) -> Result<u64, SnapshotError> {
    let value = a.checked_add(b).ok_or_else(|| capacity(limit, u64::MAX))?;
    if value > limit {
        return Err(capacity(limit, value));
    }
    Ok(value)
}

fn product(a: u64, b: u64, limit: u64) -> Result<u64, SnapshotError> {
    let value = a.checked_mul(b).ok_or_else(|| capacity(limit, u64::MAX))?;
    if value > limit {
        return Err(capacity(limit, value));
    }
    Ok(value)
}

/// Canonical memory is printable ASCII, <=24-byte keys and <=32 i64 values per
/// array. Every key may need two-byte escaping; the largest value is the array.
/// This is the raw memory encoding before Snapshot's decimal-byte-array layer.
pub fn memory_bytes(keys: usize) -> u64 {
    let shape = super::mind::STOCK_MEMORY_LIMITS;
    let array = 2 + (shape.array_len as u64).saturating_mul(21);
    let text = 2 + (shape.string_bytes as u64).saturating_mul(2);
    let pair = 4 + (shape.key_chars as u64).saturating_mul(2) + array.max(text).max(20);
    2_u64.saturating_add((keys as u64).saturating_mul(pair))
}

fn decimal_digits(mut value: u16) -> u64 {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

/// Count numeric widening in the actual physical-unit representation. Content
/// and opaque rules are measured exactly at the opening; mutation does not add
/// new sections/components/cells. This walk holds only one unit's value at once.
fn numeric_growth(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::Number(_) => 20,
        serde_json::Value::Array(items) => items.iter().map(numeric_growth).sum(),
        serde_json::Value::Object(items) => items.values().map(numeric_growth).sum(),
        _ => 0,
    }
}

/// Reserve an opening for the lifetime of its captured content and class limits.
/// No source is compiled and no world state is consulted or published here.
pub fn admit(arena: &Arena) -> Result<SnapshotBudget, SnapshotError> {
    let limit = arena.arena_cfg.snapshot_max_bytes;
    if !(SNAPSHOT_MIN_BYTES..=SNAPSHOT_MAX_BYTES).contains(&limit) {
        return Err(SnapshotError::Malformed(
            "snapshot_max_bytes must be 16 to 480 MiB".into(),
        ));
    }
    let opening = Snapshot::encode(arena)?;
    let opening = json_len(&opening.document, limit)?;
    let count = arena.units.len() as u64;
    let width = arena.definition.terrain.width();
    let height = arena.definition.terrain.height();
    let cells = product(u64::from(width), u64::from(height), u64::MAX)?;
    let route_cells = cells.min(u64::from(arena.arena_cfg.reflex.path_max_cells));
    let route_cell_bytes =
        4 + decimal_digits(width.saturating_sub(1)) + decimal_digits(height.saturating_sub(1));
    let routes = product(product(count, route_cells, limit)?, route_cell_bytes, limit)?;
    let mut budget = SnapshotBudget {
        opening,
        // Full catalog, not one definition per hull. Existing payload is already
        // counted above; this conservative extra allows later diagnostic cards
        // to fill the same immutable 2 MiB catalog without exceeding admission.
        catalog_growth: 2 * super::runtime_card::MAX_CARD_CATALOG_BYTES as u64,
        scalar_growth: 0,
        memory: 0,
        routes,
        hull_tables: product(count, HULL_TABLE_BYTES, limit)?,
        contact_tables: 0,
        spent_sections: 0,
        projectiles: 0,
        objective_events: 0,
        // finish_pulse drains every closed chunk before Task awaits its batch.
        // Open text is <CHUNK_BYTES; settlement adds at most bounded per-hull
        // fault/drop/sortie lines. JSON text can expand fourfold through the two
        // string layers. The exact opening already includes its initial chunks.
        ledger_tail: product(
            4,
            sum(
                super::ledger::CHUNK_BYTES as u64,
                product(count, 2048, limit)?,
                limit,
            )?,
            limit,
        )?,
        total: 0,
        limit,
    };
    let terrain_heights = arena
        .definition
        .terrain
        .cells()
        .iter()
        .map(|cell| i32::from(cell.elev_cm));
    let poses = arena.units.iter().map(|unit| unit.pose.z_cm);
    let heights = terrain_heights.chain(poses).collect::<Vec<_>>();
    let low = heights.iter().copied().min().unwrap_or(0);
    let high = heights.iter().copied().max().unwrap_or(0);
    let distance_cm = (u64::from(width) + u64::from(height))
        .saturating_mul(u64::from(arena.definition.terrain.cell_cm()))
        .saturating_add(i64::from(high).abs_diff(i64::from(low)));
    for own in &arena.units {
        // A damaged class can have a differently configured key count; preserve
        // the maximum of every reachable lower row, including captured ROM.
        let keys = arena
            .classes
            .rows()
            .iter()
            .filter(|row| row.class <= own.seal.class)
            .map(|row| row.memory_keys)
            .max()
            .unwrap_or(0);
        // Pilots can use the fitted core's complete slot row even when the
        // sealed executable is rated lower. Opening hardware can only degrade.
        let hardware_class = arena
            .core_of(own.id)
            .map_or(super::core::CoreClass::Zero, |core| core.class);
        let slots = arena
            .classes
            .rows()
            .iter()
            .filter(|row| row.class <= hardware_class)
            .map(|row| row.slots)
            .max()
            .unwrap_or(0) as u64;
        // Two memories: landed and pending. Vec<u8> needs <=4 encoded bytes per
        // raw byte, and its digits need no additional outer-string escaping.
        budget.memory = sum(
            budget.memory,
            sum(product(memory_bytes(keys), 8, limit)?, 4, limit)?,
            limit,
        )?;
        budget.hull_tables = sum(
            budget.hull_tables,
            product(own.weapons.len() as u64, 512, limit)?,
            limit,
        )?;
        let unit = serde_json::to_value(own)
            .map_err(|error| SnapshotError::Malformed(error.to_string()))?;
        budget.scalar_growth = sum(budget.scalar_growth, numeric_growth(&unit), limit)?;
        let pair_bytes = sum(
            CONTACT_PAIR_BYTES,
            product(slots, HELD_SLOT_PAIR_BYTES, limit)?,
            limit,
        )?;
        budget.contact_tables = sum(
            budget.contact_tables,
            product(count, pair_bytes, limit)?,
            limit,
        )?;
        // The stable key is (viewer,target,section), not number of impacts. IDs
        // are canonical ASCII. Use actual section names, including the victim's
        // self-key, rather than assuming all custom units have stock's sections.
        for (section, _) in own.unit.sections() {
            budget.spent_sections = sum(
                budget.spent_sections,
                product(count, 64 + section.as_str().len() as u64, limit)?,
                limit,
            )?;
        }
        // Destruction notices use closed consequence labels, but a custom
        // section may declare many of them. Account its actual largest list.
        let consequence_bytes = own
            .profile
            .sections
            .iter()
            .map(|section| {
                section
                    .consequences
                    .iter()
                    .map(|effect| effect.label().len() as u64 + 6)
                    .sum::<u64>()
            })
            .max()
            .unwrap_or(0);
        budget.hull_tables = sum(
            budget.hull_tables,
            product(consequence_bytes, super::mind::MIND_NOTICES as u64, limit)?,
            limit,
        )?;
        for weapon in &own.weapons {
            let flight = u64::try_from(weapon.flight_ms(distance_cm)).unwrap_or(u64::MAX);
            let window = flight.min(u64::from(arena.match_cfg.max_duration_ms));
            let cycles = window / u64::from(weapon.cycle_ms.max(1)) + 2;
            let rounds = u64::from(own.weapon_ammo(&weapon.id));
            let shots = rounds
                .div_ceil(u64::from(weapon.burst_rounds.max(1)))
                .min(cycles);
            budget.projectiles = sum(
                budget.projectiles,
                product(shots, PROJECTILE_BYTES + DUE_EVENT_BYTES, limit)?,
                limit,
            )?;
        }
    }
    if let Some(hold) = &arena.hold {
        // Hold can be re-opened once per native substep; old stamps remain due
        // until their finite hold window expires. Use MIN_SUBSTEP, not the usual
        // 250 ms maximum, so adaptive faster integration remains covered.
        let window = u64::from(hold.hold_ms).min(u64::from(arena.match_cfg.max_duration_ms));
        budget.objective_events = product(
            window / u64::from(super::clock::MIN_SUBSTEP_MS) + 2,
            DUE_EVENT_BYTES,
            limit,
        )?;
    }
    // Latest accepted groups can fill after admission. This complete bounded
    // record allowance is in addition to the actual opening document.
    budget.objective_events = sum(
        budget.objective_events,
        super::order_watch::SNAPSHOT_RESERVE,
        limit,
    )?;
    // One fixed authority/sequence row per hull, including removal tombstones;
    // double JSON escaping and maximum decimal identifiers fit within 512 bytes.
    budget.objective_events = sum(
        budget.objective_events,
        product(arena.units.len() as u64, 512, limit)?,
        limit,
    )?;
    for bytes in [
        budget.opening,
        budget.catalog_growth,
        budget.scalar_growth,
        budget.memory,
        budget.routes,
        budget.hull_tables,
        budget.contact_tables,
        budget.spent_sections,
        budget.projectiles,
        budget.objective_events,
        budget.ledger_tail,
    ] {
        budget.total = sum(budget.total, bytes, limit)?;
    }
    Ok(budget)
}

/// Sink stops *before* accepting bytes above the limit. Counting and encoding
/// share precisely the serializer that the store writes; no guessed escape cost.
struct Sink {
    limit: u64,
    written: u64,
    attempted: u64,
    bytes: Option<Vec<u8>>,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.attempted = self.written.saturating_add(bytes.len() as u64);
        if self.attempted > self.limit {
            return Err(io::Error::other("snapshot encoded byte limit"));
        }
        if let Some(out) = &mut self.bytes {
            let needed = usize::try_from(self.attempted)
                .map_err(|_| io::Error::other("snapshot allocation size"))?;
            if needed > out.capacity() {
                let ceiling = usize::try_from(self.limit).unwrap_or(usize::MAX);
                let target = out
                    .capacity()
                    .saturating_mul(2)
                    .max(64 * 1024)
                    .min(ceiling)
                    .max(needed);
                out.try_reserve_exact(target.saturating_sub(out.len()))
                    .map_err(|_| io::Error::other("snapshot allocation failed"))?;
            }
            out.extend_from_slice(bytes);
        }
        self.written = self.attempted;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize<T: Serialize + ?Sized>(
    value: &T,
    limit: u64,
    encode: bool,
) -> Result<Sink, SnapshotError> {
    let mut sink = Sink {
        limit,
        written: 0,
        attempted: 0,
        bytes: encode.then(Vec::new),
    };
    if let Err(error) = serde_json::to_writer(&mut sink, value) {
        return Err(if sink.attempted > limit {
            capacity(limit, sink.attempted)
        } else {
            SnapshotError::Malformed(error.to_string())
        });
    }
    Ok(sink)
}

pub(crate) fn json_len<T: Serialize + ?Sized>(value: &T, limit: u64) -> Result<u64, SnapshotError> {
    Ok(serialize(value, limit, false)?.written)
}

pub(crate) fn json_bytes<T: Serialize + ?Sized>(
    value: &T,
    limit: u64,
) -> Result<Vec<u8>, SnapshotError> {
    Ok(serialize(value, limit, true)?.bytes.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::core::ClassTable;
    use crate::arena::state::PartyId;
    use crate::arena::test_support::{arena_of, pose, tank};
    use mush_core::ObjId;

    #[test]
    fn m6_snapshot_writer_counts_actual_escaping_and_never_keeps_an_overflow() {
        let value = serde_json::json!({"state": "\"\\\n".repeat(1000)});
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            json_len(&value, bytes.len() as u64).unwrap(),
            bytes.len() as u64
        );
        assert_eq!(json_bytes(&value, bytes.len() as u64).unwrap(), bytes);
        assert!(matches!(
            json_bytes(&value, 32),
            Err(SnapshotError::Capacity { limit: 32, .. })
        ));
        let mut sink = Sink {
            limit: 3,
            written: 0,
            attempted: 0,
            bytes: Some(Vec::new()),
        };
        sink.write_all(b"ab").unwrap();
        assert!(sink.write_all(b"cd").is_err());
        assert_eq!(sink.bytes.unwrap(), b"ab");
    }

    #[test]
    fn m6_memory_reserve_covers_two_maximal_encoded_answers_each_class() {
        for profile in ClassTable::STOCK.rows() {
            let mut memory = mush_mind::memory::Memory::new();
            for at in 0..profile.memory_keys {
                memory.insert(
                    format!("{at:03}{}", "\"".repeat(21)),
                    mush_mind::memory::Value::Ints(vec![i64::MIN; 32]),
                );
            }
            let raw = mush_mind::memory::encode(&memory);
            assert!(raw.len() as u64 <= memory_bytes(profile.memory_keys));
            let encoded = serde_json::to_vec(&(&raw, &raw)).unwrap();
            assert!(encoded.len() as u64 <= 8 * memory_bytes(profile.memory_keys) + 8);
        }
    }

    #[test]
    fn m6_300_shared_stock_accounting_does_not_allocate_the_reserved_document() {
        let n = 300;
        let memory = n * (8 * memory_bytes(32) + 4);
        let routes = n * 65_536 * 10;
        let pairs = n * n * (CONTACT_PAIR_BYTES + 2 * HELD_SLOT_PAIR_BYTES);
        let hulls = n * HULL_TABLE_BYTES;
        let spent = n * n * 5 * 80;
        // 16 MiB actual opening allowance exceeds representative stock state;
        // all distinct definitions still share the separate 2 MiB catalog cap.
        let other = 16 * 1024 * 1024 + 2 * 2 * 1024 * 1024 + 8 * 1024 * 1024;
        assert!(memory + routes + pairs + hulls + spent + other < SNAPSHOT_MAX_BYTES);
        let all_iii = n * (8 * memory_bytes(128) + 4) + routes + pairs + hulls + spent + other;
        assert!(
            all_iii > SNAPSHOT_MAX_BYTES,
            "the extreme is explicitly outside this storage envelope"
        );
    }

    #[test]
    fn m6_snapshot_limit_applies_to_full_document_and_decode_before_state() {
        let arena = arena_of(vec![tank(ObjId(1), PartyId::RED, pose(2, 2))]);
        let snapshot = Snapshot::encode(&arena).unwrap();
        let size = json_len(&snapshot.document, SNAPSHOT_MAX_BYTES).unwrap();
        assert!(size > snapshot.bytes().len() as u64);
        assert!(Snapshot::encode_with_limit(&arena, size - 1).is_err());
        assert!(
            Snapshot::decode_with_limit(
                &snapshot.document,
                arena.definition.as_ref().clone(),
                size - 1
            )
            .is_err()
        );
        let mut invalid = snapshot.document;
        invalid["state"] = serde_json::Value::String("[".repeat(100));
        assert!(matches!(
            Snapshot::decode_with_limit(&invalid, arena.definition.as_ref().clone(), 32),
            Err(SnapshotError::Capacity { .. })
        ));
    }

    #[test]
    fn m6_capacity_math_refuses_overflow_without_allocation() {
        assert!(product(u64::MAX, 2, SNAPSHOT_MAX_BYTES).is_err());
        assert!(sum(u64::MAX, 1, SNAPSHOT_MAX_BYTES).is_err());
    }

    #[test]
    fn m6_slot_reserve_uses_fitted_hardware_while_memory_uses_sealed_class() {
        use crate::arena::core::CoreClass;
        use crate::arena::test_support::tactical_unit_cored;
        let id = ObjId(1);
        let mut arena = arena_of(vec![tank(id, PartyId::RED, pose(2, 2))]);
        assert_eq!(arena.unit(id).unwrap().seal.class, CoreClass::I);
        let lower = admit(&arena).unwrap();
        arena.unit_mut(id).unwrap().unit = tactical_unit_cored(id, CoreClass::III);
        let higher = admit(&arena).unwrap();
        assert_eq!(higher.memory, lower.memory);
        assert_eq!(
            higher.contact_tables - lower.contact_tables,
            4 * HELD_SLOT_PAIR_BYTES
        );
    }

    #[test]
    fn m6_contact_pair_envelope_covers_full_width_numbers_and_two_loss_events() {
        use super::super::condition::{Condition, Reading};
        use super::super::state::{
            Classification, ContactId, Designator, DueEvent, DueKind, SeenPose, ShadowContact,
        };
        let id = ObjId(u32::MAX);
        let contact_id = ContactId(u64::MAX);
        let plot = SeenPose {
            x_cm: i64::MIN,
            y_cm: i64::MIN,
            z_cm: i32::MIN,
            heading_mdeg: u32::MAX,
            speed_cms: i32::MIN,
        };
        let reading = Reading {
            at_ms: i64::MIN,
            condition: Condition {
                bands: u32::MAX,
                sections: u8::MAX,
            },
        };
        let mut contact = ShadowContact::sighted(
            id,
            Classification::Classified {
                party: PartyId::BLUE,
            },
            i64::MIN,
            plot,
        );
        contact.seen_condition = Some(reading);
        contact.own_seen.as_mut().unwrap().condition = Some(reading);
        let designator = Designator::Unit {
            party: PartyId::BLUE,
            letter: 'Z',
        };
        let loss = DueEvent {
            t_ms: i64::MIN,
            seq: u64::MAX,
            kind: DueKind::LossSettles {
                viewer: id,
                contact: contact_id,
            },
        };
        let pieces = [
            serde_json::json!((id, contact_id, contact)),
            serde_json::json!(((id, id), u16::MAX)),
            serde_json::json!((contact_id, u64::MAX)),
            serde_json::json!((contact_id, designator)),
            serde_json::json!(((id, contact_id), (i64::MIN, designator))),
            serde_json::json!(loss),
            serde_json::json!(loss),
        ];
        let escaped: u64 = pieces
            .iter()
            .map(|piece| {
                let inner = serde_json::to_string(piece).unwrap();
                json_len(&inner, SNAPSHOT_MAX_BYTES).unwrap() - 2
            })
            .sum();
        // Extra quotes for numeric map keys, separators and owning row braces.
        assert!(escaped + 32 <= CONTACT_PAIR_BYTES, "{escaped}");
    }

    #[test]
    fn m6_unique_large_four_profile_definitions_count_once_and_refuse_atomically() {
        use super::super::core::CoreClass;
        use super::super::runtime_card::{CardDefinition, ProfileDefinition};
        use super::super::schema::{CardField, CardSchema, CardSection, Widget};
        use super::super::state::Channel;
        let units = (0..24)
            .map(|index| {
                tank(
                    ObjId(index + 1),
                    PartyId::RED,
                    pose(
                        i64::from(index % 8) * 1000 + 500,
                        i64::from(index / 8) * 1000 + 500,
                    ),
                )
            })
            .collect();
        let mut arena = arena_of(units);
        let schema = CardSchema {
            sections: vec![CardSection {
                id: "settings".into(),
                label: "S".repeat(96).into(),
                station: Channel::Helm,
                fields: (0..64)
                    .map(|index| CardField {
                        id: format!("field_{index}").into(),
                        label: "L".repeat(96).into(),
                        widget: Widget::Toggle {
                            default: false,
                            help: "H".repeat(512).into(),
                        },
                    })
                    .collect(),
            }],
        };
        let before = admit(&arena).unwrap();
        let mut last = None;
        for index in 0..24 {
            let definition = CardDefinition {
                declared_class: CoreClass::III,
                schema: schema.clone(),
                profiles: (0..4)
                    .map(|profile| ProfileDefinition {
                        name: format!("revision_{index}_profile_{profile}").into(),
                        values: schema.defaults(),
                        rules: vec![],
                    })
                    .collect(),
            };
            definition.validate().unwrap();
            arena.cards.admit(definition.clone()).unwrap();
            last = Some(definition);
        }
        let after = admit(&arena).unwrap();
        assert!(after.opening > before.opening + 1024 * 1024);
        assert_eq!(after.memory, before.memory);
        assert_eq!(after.routes, before.routes);
        let catalog_bytes = arena.cards.encoded_bytes();
        let opening = after.opening;
        for _ in 0..300 {
            arena.cards.admit(last.as_ref().unwrap().clone()).unwrap();
        }
        assert_eq!(arena.cards.encoded_bytes(), catalog_bytes);
        assert_eq!(admit(&arena).unwrap().opening, opening);
        // A legal operator memory-row change can exceed a deliberately smaller
        // storage envelope, even though all immutable authored data is legal.
        arena.classes = serde_json::from_value(serde_json::json!({
            "I": { "memory_keys": 128 }
        }))
        .unwrap();
        arena.arena_cfg.snapshot_max_bytes = SNAPSHOT_MIN_BYTES;
        let bytes = arena.cards.encoded_bytes();
        let keys = arena.cards.len();
        assert!(matches!(admit(&arena), Err(SnapshotError::Capacity { .. })));
        assert_eq!(arena.cards.encoded_bytes(), bytes);
        assert_eq!(arena.cards.len(), keys);
    }
}
