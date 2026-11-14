//! **Saving an open bubble costs what changed** (doc 47 unit CO.12).
//!
//! An open bubble's record ([`ActiveInterdictionBubble`]) is mostly copies of the bubble: the
//! live one, the tape's opening snapshot, and up to [`super::tick::MAX_BUBBLE_SNAPSHOTS`]
//! snapshots, one pushed by every step. Before this unit the space tick copied the whole
//! book and wrote every stepped bubble's whole record, about 63 KB of JSON, every second.
//! Here a record is saved in two kinds of document:
//!
//! - **the record without its snapshots** at `tacsim:space:bubble:{domain}` (the key it had):
//!   a [`PersistedBubbleDoc`] whose `bubble` has no `snapshots` and whose `tape` has no
//!   `initial_snapshot`, and whose `parts` ([`BubbleParts`]) name the slots they are kept in;
//! - **each snapshot once** at `tacsim:space:bubble_snapshot:{domain}:{slot}`
//!   ([`PersistedBubbleSnapshotDoc`]). A snapshot never changes once it is taken, so a slot
//!   is written once, when its snapshot first appears in the record, and deleted when the
//!   record no longer holds it. The tape's opening snapshot is one of them; when it is the
//!   same as one of the ring's it shares that slot.
//!
//! A step therefore writes the record (one bubble's worth and the tape's entries) and the
//! one snapshot it pushed, and deletes the one the ring let go. A slot is never reused while
//! the record before the write still names it, so every write of a batch lands on a slot
//! nobody reads until the record that names it lands too.
//!
//! **Reading.** A record whose document has `parts` waits for its snapshots
//! (`stash_bubble_core`, `stash_bubble_snapshot`); [`finish_space_kv_load`] puts each back
//! where it was and decodes the whole record as before. A record with a snapshot missing or
//! unreadable holds its domain for staff, as an unreadable record always has. A record
//! written before this unit, the whole record under the one key with no `parts`, reads as
//! it always did; its first save afterwards writes its snapshots apart and the record
//! without them.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use mush_core::ModuleWrite;
use mush_sim::DomainId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bubble::{BubbleInputTape, BubbleSnapshot};
use super::tick::{
    ActiveInterdictionBubble, PersistedBubbleDoc, SPACE_STATE_FORMAT, space_bubble_key,
};
use crate::store::TacsimStore;

/// Per-domain bubble snapshot store key prefix
/// (`tacsim:space:bubble_snapshot:{domain}:{slot}`, doc 47 unit CO.12).
pub const SPACE_BUBBLE_SNAPSHOT_PREFIX: &str = "tacsim:space:bubble_snapshot:";

/// Format the store key of one of a bubble's snapshots
/// (`tacsim:space:bubble_snapshot:{domain}:{slot}`).
#[must_use]
pub fn space_bubble_snapshot_key(domain: DomainId, slot: u16) -> String {
    format!("{SPACE_BUBBLE_SNAPSHOT_PREFIX}{}:{slot}", domain.0)
}

/// **Where a saved record's snapshots are** (doc 47 unit CO.12): the slot of the tape's
/// opening snapshot, and the slot of each of the ring's, in the ring's order. Two that are
/// the same snapshot may name the same slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleParts {
    /// The slot of `tape.initial_snapshot`.
    pub initial: u16,
    /// The slot of each of `snapshots`, in order.
    pub snapshots: Vec<u16>,
}

impl BubbleParts {
    /// Every slot named, once each.
    fn slots(&self) -> BTreeSet<u16> {
        std::iter::once(self.initial)
            .chain(self.snapshots.iter().copied())
            .collect()
    }
}

/// One of a bubble's snapshots (`tacsim:space:bubble_snapshot:{domain}:{slot}`, doc 47
/// unit CO.12).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedBubbleSnapshotDoc {
    /// Schema format version ([`SPACE_STATE_FORMAT`]).
    pub format: u16,
    /// Domain identity matching the key.
    pub domain: DomainId,
    /// Slot matching the key.
    pub slot: u16,
    /// The snapshot.
    pub snapshot: BubbleSnapshot,
}

/// The bubble document as it is written: [`PersistedBubbleDoc`] by reference.
#[derive(Serialize)]
struct PersistedBubbleDocRef<'a> {
    format: u16,
    domain: DomainId,
    bubble: &'a ActiveInterdictionBubble,
    parts: &'a BubbleParts,
}

/// The snapshot document as it is written.
#[derive(Serialize)]
struct PersistedBubbleSnapshotDocRef<'a> {
    format: u16,
    domain: DomainId,
    slot: u16,
    snapshot: &'a BubbleSnapshot,
}

thread_local! {
    /// Set while a record is encoded without its snapshots ([`encode_apart`]).
    static APART: Cell<bool> = const { Cell::new(false) };
}

/// **Whether the record being encoded leaves its snapshots out** (the `skip_serializing_if`
/// of `ActiveInterdictionBubble::snapshots` and `BubbleInputTape::initial_snapshot`): only
/// inside [`encode_apart`], so every other encoding of a record is whole, as before.
pub(crate) fn kept_apart<T>(_: &T) -> bool {
    APART.with(Cell::get)
}

/// Clears [`APART`] when an encoding ends, panic or not.
struct ApartGuard;

impl Drop for ApartGuard {
    fn drop(&mut self) {
        APART.with(|apart| apart.set(false));
    }
}

/// Encode `value` with every record in it leaving its snapshots out.
fn encode_apart<T: Serialize>(value: &T) -> serde_json::Result<serde_json::Value> {
    APART.with(|apart| apart.set(true));
    let _guard = ApartGuard;
    serde_json::to_value(value)
}

/// Whether two snapshots are the same: the hash first, which differs for nearly every two
/// steps, and the whole of them when it does not.
fn same_snapshot(a: &BubbleSnapshot, b: &BubbleSnapshot) -> bool {
    a.state_hash == b.state_hash && a == b
}

/// **Whether two records are the same but for their snapshots.** Every field is named, so a
/// field added to the record is a compile error here until it is compared.
fn same_core(a: &ActiveInterdictionBubble, b: &ActiveInterdictionBubble) -> bool {
    let ActiveInterdictionBubble {
        hull,
        interdictor_hull,
        domain,
        carrier_ship_id,
        interdictor_ship_id,
        leg_cursor,
        opened_ic_s,
        steps,
        cleared,
        bubble,
        tape,
        snapshots: _,
        picket,
        approaching_derelict,
        holding_for_salvage,
        last_stepped_wall_s,
        last_synced_propellant_kg,
        carriers,
        ship_hulls,
        interdictor_memory,
        interdictor_think_seq,
        interdictor_seal,
        interdictor_contacts,
        stood_down,
        thinks_owed,
    } = a;
    let BubbleInputTape {
        initial_snapshot: _,
        entries,
    } = tape;
    // What a step moves first, so a stepped record differs at once.
    *steps == b.steps
        && *last_stepped_wall_s == b.last_stepped_wall_s
        && *thinks_owed == b.thinks_owed
        && *interdictor_think_seq == b.interdictor_think_seq
        && *hull == b.hull
        && *interdictor_hull == b.interdictor_hull
        && *domain == b.domain
        && *leg_cursor == b.leg_cursor
        && *opened_ic_s == b.opened_ic_s
        && *cleared == b.cleared
        && *approaching_derelict == b.approaching_derelict
        && *holding_for_salvage == b.holding_for_salvage
        && *last_synced_propellant_kg == b.last_synced_propellant_kg
        && *carrier_ship_id == b.carrier_ship_id
        && *interdictor_ship_id == b.interdictor_ship_id
        && *entries == b.tape.entries
        && *carriers == b.carriers
        && *ship_hulls == b.ship_hulls
        && *interdictor_memory == b.interdictor_memory
        && *interdictor_seal == b.interdictor_seal
        && *interdictor_contacts == b.interdictor_contacts
        && *stood_down == b.stood_down
        && *picket == b.picket
        && *bubble == b.bubble
}

/// **Copy every field of `src` but its snapshots into `dst`.** Every field is named, as in
/// [`same_core`].
fn copy_core(dst: &mut ActiveInterdictionBubble, src: &ActiveInterdictionBubble) {
    let ActiveInterdictionBubble {
        hull,
        interdictor_hull,
        domain,
        carrier_ship_id,
        interdictor_ship_id,
        leg_cursor,
        opened_ic_s,
        steps,
        cleared,
        bubble,
        tape,
        snapshots: _,
        picket,
        approaching_derelict,
        holding_for_salvage,
        last_stepped_wall_s,
        last_synced_propellant_kg,
        carriers,
        ship_hulls,
        interdictor_memory,
        interdictor_think_seq,
        interdictor_seal,
        interdictor_contacts,
        stood_down,
        thinks_owed,
    } = src;
    let BubbleInputTape {
        initial_snapshot: _,
        entries,
    } = tape;
    dst.hull = *hull;
    dst.interdictor_hull = *interdictor_hull;
    dst.domain = *domain;
    dst.carrier_ship_id.clone_from(carrier_ship_id);
    dst.interdictor_ship_id.clone_from(interdictor_ship_id);
    dst.leg_cursor = *leg_cursor;
    dst.opened_ic_s = *opened_ic_s;
    dst.steps = *steps;
    dst.cleared = *cleared;
    dst.bubble.clone_from(bubble);
    dst.tape.entries.clone_from(entries);
    dst.picket.clone_from(picket);
    dst.approaching_derelict = *approaching_derelict;
    dst.holding_for_salvage = *holding_for_salvage;
    dst.last_stepped_wall_s = *last_stepped_wall_s;
    dst.last_synced_propellant_kg = *last_synced_propellant_kg;
    dst.carriers.clone_from(carriers);
    dst.ship_hulls.clone_from(ship_hulls);
    dst.interdictor_memory.clone_from(interdictor_memory);
    dst.interdictor_think_seq = *interdictor_think_seq;
    dst.interdictor_seal.clone_from(interdictor_seal);
    dst.interdictor_contacts.clone_from(interdictor_contacts);
    dst.stood_down.clone_from(stood_down);
    dst.thinks_owed = *thinks_owed;
}

/// Where one wanted snapshot comes from: a slot already written, or none yet.
#[derive(Clone, Copy)]
enum Source {
    /// The saved record's opening snapshot, in this slot.
    Initial(u16),
    /// The saved record's ring snapshot at this index, in this slot.
    Ring(usize, u16),
    /// Not saved: written to this slot.
    New(u16),
}

impl Source {
    fn slot(self) -> u16 {
        match self {
            Source::Initial(slot) | Source::Ring(_, slot) | Source::New(slot) => slot,
        }
    }
}

/// **Save every open bubble that changed** (doc 47 unit CO.12): diff `live` against the
/// saved copies in `cache` (the last save's, whole) and the slots in `parts`, push the writes
/// to `out`, and bring `cache` and `parts` up to `live`.
pub(crate) fn save_bubbles(
    live: &BTreeMap<DomainId, ActiveInterdictionBubble>,
    cache: &mut BTreeMap<DomainId, ActiveInterdictionBubble>,
    parts: &mut BTreeMap<DomainId, BubbleParts>,
    out: &mut mush_core::Outbox,
) {
    // Bubbles that closed: the record and every slot it named.
    let closed: Vec<DomainId> = cache
        .keys()
        .filter(|domain| !live.contains_key(domain))
        .copied()
        .collect();
    for domain in closed {
        cache.remove(&domain);
        crate::store::push_op(
            out,
            ModuleWrite::Del {
                key: SmolStr::new(space_bubble_key(domain)),
            },
        );
        if let Some(had) = parts.remove(&domain) {
            for slot in had.slots() {
                delete_snapshot(out, domain, slot);
            }
        }
    }
    // A bubble the cache never held has no slots of its own.
    parts.retain(|domain, _| cache.contains_key(domain) || live.contains_key(domain));

    for (&domain, active) in live {
        let had = parts.get(&domain);
        let saved = cache.get(&domain);
        let plan = plan_parts(active, saved, had);
        let now = BubbleParts {
            initial: plan.initial.slot(),
            snapshots: plan.ring.iter().map(|source| source.slot()).collect(),
        };
        let core_changed = saved.is_none_or(|saved| !same_core(active, saved));
        let parts_changed = had != Some(&now);
        if !core_changed && !parts_changed {
            continue;
        }

        // 1. The snapshots not saved yet, each once.
        let mut written = BTreeSet::new();
        let wanted = std::iter::once((plan.initial, &active.tape.initial_snapshot))
            .chain(plan.ring.iter().copied().zip(active.snapshots.iter()));
        for (source, snapshot) in wanted {
            if let Source::New(slot) = source
                && written.insert(slot)
            {
                let doc = PersistedBubbleSnapshotDocRef {
                    format: SPACE_STATE_FORMAT,
                    domain,
                    slot,
                    snapshot,
                };
                match serde_json::to_value(&doc) {
                    Ok(value) => crate::store::push_op(
                        out,
                        ModuleWrite::JsonSet {
                            key: SmolStr::new(space_bubble_snapshot_key(domain, slot)),
                            value,
                        },
                    ),
                    Err(error) => {
                        tracing::error!(%error, domain = domain.0, "tacsim: a bubble snapshot did not encode");
                        return;
                    }
                }
            }
        }

        // 2. The record without its snapshots, naming their slots.
        let doc = PersistedBubbleDocRef {
            format: SPACE_STATE_FORMAT,
            domain,
            bubble: active,
            parts: &now,
        };
        match encode_apart(&doc) {
            Ok(value) => crate::store::push_op(
                out,
                ModuleWrite::JsonSet {
                    key: SmolStr::new(space_bubble_key(domain)),
                    value,
                },
            ),
            Err(error) => {
                tracing::error!(%error, domain = domain.0, "tacsim: a bubble record did not encode");
                return;
            }
        }

        // 3. The slots the record let go.
        if let Some(had) = had {
            let kept = now.slots();
            for slot in had.slots() {
                if !kept.contains(&slot) {
                    delete_snapshot(out, domain, slot);
                }
            }
        }

        // 4. The cache, brought up to the record as it now stands.
        match cache.get_mut(&domain) {
            Some(saved) => {
                if core_changed {
                    copy_core(saved, active);
                }
                refresh_snapshots(saved, active, &plan);
            }
            None => {
                cache.insert(domain, active.clone());
            }
        }
        parts.insert(domain, now);
    }
}

/// Delete one snapshot slot.
fn delete_snapshot(out: &mut mush_core::Outbox, domain: DomainId, slot: u16) {
    crate::store::push_op(
        out,
        ModuleWrite::Del {
            key: SmolStr::new(space_bubble_snapshot_key(domain, slot)),
        },
    );
}

/// Where each of a live record's snapshots is to be kept.
struct Plan {
    initial: Source,
    ring: Vec<Source>,
}

/// **Match a live record's snapshots to the slots already written.** A snapshot equal to one
/// the saved record holds keeps that one's slot; any other gets a slot that neither the saved
/// record nor this plan names, and two new ones that are equal share it.
fn plan_parts<'a>(
    live: &'a ActiveInterdictionBubble,
    saved: Option<&'a ActiveInterdictionBubble>,
    had: Option<&BubbleParts>,
) -> Plan {
    // What is already written, with where the cache holds it.
    let mut pool: Vec<(Source, &'a BubbleSnapshot)> = Vec::new();
    if let (Some(saved), Some(had)) = (saved, had)
        && had.snapshots.len() == saved.snapshots.len()
    {
        pool.push((Source::Initial(had.initial), &saved.tape.initial_snapshot));
        for (index, (&slot, snapshot)) in had.snapshots.iter().zip(&saved.snapshots).enumerate() {
            pool.push((Source::Ring(index, slot), snapshot));
        }
    }
    let mut taken: BTreeSet<u16> = had.map(BubbleParts::slots).unwrap_or_default();
    let mut fresh: Vec<(u16, &BubbleSnapshot)> = Vec::new();
    let mut place = |snapshot: &'a BubbleSnapshot| -> Source {
        if let Some((source, _)) = pool.iter().find(|(_, held)| same_snapshot(held, snapshot)) {
            return *source;
        }
        if let Some((slot, _)) = fresh.iter().find(|(_, new)| same_snapshot(new, snapshot)) {
            return Source::New(*slot);
        }
        let slot = (0..=u16::MAX)
            .find(|slot| !taken.contains(slot))
            .unwrap_or(u16::MAX);
        taken.insert(slot);
        fresh.push((slot, snapshot));
        Source::New(slot)
    };
    let initial = place(&live.tape.initial_snapshot);
    let ring = live.snapshots.iter().map(&mut place).collect();
    Plan { initial, ring }
}

/// Bring the cache's snapshots up to `live`'s, moving those it already holds rather than
/// copying them.
fn refresh_snapshots(
    saved: &mut ActiveInterdictionBubble,
    live: &ActiveInterdictionBubble,
    plan: &Plan,
) {
    let mut old: Vec<Option<BubbleSnapshot>> = std::mem::take(&mut saved.snapshots)
        .into_iter()
        .map(Some)
        .collect();
    let mut ring = Vec::with_capacity(live.snapshots.len());
    for (source, snapshot) in plan.ring.iter().zip(&live.snapshots) {
        let kept = match *source {
            Source::Ring(index, _) => old.get_mut(index).and_then(Option::take),
            Source::Initial(_) | Source::New(_) => None,
        };
        ring.push(kept.unwrap_or_else(|| snapshot.clone()));
    }
    saved.snapshots = ring;
    if !matches!(plan.initial, Source::Initial(_)) {
        saved
            .tape
            .initial_snapshot
            .clone_from(&live.tape.initial_snapshot);
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Split bubble documents read at boot and waiting on their snapshots ([`finish_space_kv_load`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct PendingBubbleLoad {
    /// Each record that names its parts, by domain: its key and its document.
    cores: BTreeMap<DomainId, (String, serde_json::Value)>,
    /// Each snapshot read, by domain and slot: its `snapshot`.
    snapshots: BTreeMap<(DomainId, u16), serde_json::Value>,
    /// Domains a snapshot document of which did not read.
    unreadable: BTreeSet<DomainId>,
}

/// Whether a bubble document names its parts (written since doc 47 unit CO.12).
pub(crate) fn names_its_parts(value: &serde_json::Value) -> bool {
    value.get("parts").is_some()
}

/// Hold a bubble document that names its parts until they are read.
pub(crate) fn stash_bubble_core(
    store: &mut TacsimStore,
    domain: DomainId,
    key: &str,
    value: &serde_json::Value,
) {
    store
        .space_bubble_load
        .cores
        .insert(domain, (key.to_owned(), value.clone()));
}

/// Read one `tacsim:space:bubble_snapshot:{domain}:{slot}` document. Answers `false` when
/// the key itself does not read (the caller holds the space state, as for any such key).
pub(crate) fn stash_bubble_snapshot(
    store: &mut TacsimStore,
    key: &str,
    value: &serde_json::Value,
) -> bool {
    let Some(rest) = key.strip_prefix(SPACE_BUBBLE_SNAPSHOT_PREFIX) else {
        return false;
    };
    let Some((domain, slot)) = rest
        .split_once(':')
        .and_then(|(d, s)| Some((DomainId(d.parse().ok()?), s.parse::<u16>().ok()?)))
    else {
        return false;
    };
    let readable = value.get("format").and_then(serde_json::Value::as_u64)
        == Some(u64::from(SPACE_STATE_FORMAT))
        && value.get("domain").and_then(serde_json::Value::as_u64) == Some(domain.0)
        && value.get("slot").and_then(serde_json::Value::as_u64) == Some(u64::from(slot));
    match value.get("snapshot") {
        Some(snapshot) if readable => {
            store
                .space_bubble_load
                .snapshots
                .insert((domain, slot), snapshot.clone());
        }
        _ => {
            store.space_bubble_load.unreadable.insert(domain);
        }
    }
    true
}

/// **Put every split record read at boot back together** (doc 47 unit CO.12): each record's
/// snapshots back in their places, the whole record decoded and checked as a record written
/// whole is, and its slots kept for its next save. A record with a snapshot missing or
/// unreadable holds its domain for staff. Called once after every `tacsim:space:*` key has
/// gone through `load_space_kv_entry`.
pub fn finish_space_kv_load(store: &mut TacsimStore) {
    let pending = std::mem::take(&mut store.space_bubble_load);
    if pending.cores.is_empty() && pending.snapshots.is_empty() {
        return;
    }
    let Some(mut state) = store.space_state.take() else {
        // The space state is held for repair already; nothing more is read into it.
        return;
    };
    let mut used = BTreeSet::new();
    for (domain, (key, mut value)) in pending.cores {
        match reassemble(domain, &mut value, &pending.snapshots, &pending.unreadable) {
            Some(parts) => {
                for slot in parts.slots() {
                    used.insert((domain, slot));
                }
                match serde_json::from_value::<PersistedBubbleDoc>(value) {
                    Ok(doc)
                        if doc.format == SPACE_STATE_FORMAT
                            && doc.domain == domain
                            && doc.bubble.domain == domain =>
                    {
                        state.book.by_domain.insert(domain, doc.bubble);
                        store.space_bubble_parts.insert(domain, parts);
                    }
                    _ => {
                        state.book.quarantined_bubbles.insert(domain);
                        tracing::warn!(%key, domain = domain.0, "tacsim: space bubble document is unreadable; domain quarantined");
                    }
                }
            }
            None => {
                state.book.quarantined_bubbles.insert(domain);
                tracing::warn!(%key, domain = domain.0, "tacsim: a space bubble's snapshot is missing or unreadable; domain quarantined");
            }
        }
    }
    let stray = pending
        .snapshots
        .keys()
        .filter(|at| !used.contains(at))
        .count();
    if stray > 0 {
        tracing::warn!(
            stray,
            "tacsim: bubble snapshot documents no record names; left as they are"
        );
    }
    store.space_state = Some(state);
}

/// Put a split record's snapshots back into its document; answers its parts, or `None` when
/// one is missing or unreadable.
fn reassemble(
    domain: DomainId,
    value: &mut serde_json::Value,
    snapshots: &BTreeMap<(DomainId, u16), serde_json::Value>,
    unreadable: &BTreeSet<DomainId>,
) -> Option<BubbleParts> {
    if unreadable.contains(&domain) {
        return None;
    }
    let parts: BubbleParts = serde_json::from_value(value.get("parts")?.clone()).ok()?;
    let part = |slot: u16| snapshots.get(&(domain, slot)).cloned();
    let initial = part(parts.initial)?;
    let ring = parts
        .snapshots
        .iter()
        .map(|&slot| part(slot))
        .collect::<Option<Vec<_>>>()?;
    let bubble = value.get_mut("bubble")?.as_object_mut()?;
    bubble.insert("snapshots".to_owned(), serde_json::Value::Array(ring));
    bubble
        .get_mut("tape")?
        .as_object_mut()?
        .insert("initial_snapshot".to_owned(), initial);
    Some(parts)
}

/// Forget every slot and pending read (the space state was reset or reread).
pub(crate) fn forget(store: &mut TacsimStore) {
    store.space_bubble_parts.clear();
    store.space_bubble_load = PendingBubbleLoad::default();
}

/// The saved bubbles and their slots, taken out of `store` for a save
/// (`persist_space_state`).
pub(crate) fn take_parts(store: &mut TacsimStore) -> BTreeMap<DomainId, BubbleParts> {
    std::mem::take(&mut store.space_bubble_parts)
}
