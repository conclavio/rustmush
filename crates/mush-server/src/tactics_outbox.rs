//! # The tactics boundary-fact outbox adapter — E7 cut 8 B4 (10b §3.8 OPEN 4)
//!
//! `crates/mush-server/src/durable_outbox.rs` defines the
//! [`DurableOutboxAdapter`] trait and `spawn_persist_with_gateway` wires an
//! **empty** `DurableOutboxRegistry`: before this cut the tree had no production
//! adapter at all (08i §2 row 12). This is the first.
//!
//! ## What crosses the boundary, and what may not
//!
//! **T2 (D-E7-17): no economic transaction occurs inside a combat pulse.** A
//! domain tick mutates combat state and emits **typed boundary facts only** —
//! sortie started, unit disabled/abandoned, docking completed, damage
//! controlled. Those facts leave the authoritative bundle through this adapter,
//! bounded by `MAX_DURABLE_OUTBOX_PAYLOAD_BYTES` (256 KiB), and the economy
//! consumes them **afterwards**, on the far side of the commit.
//!
//! The facts are `mush-tactics`' even though `mush-sim` owns the pulse that
//! emits them, so the adapter is stamped with `mush-tactics`' `OwnerId` — the
//! `DurableOutboxIntent`'s `(owner, adapter)` pair is what routes it, and the
//! kernel puts no constraint on the emitting provider's own owner.
//!
//! ## At-least-once, by design
//!
//! The store persists an attempt **before** this adapter is called, so a crash
//! after delivery but before the terminal acknowledgement is an intentional
//! retry with the **same stable item id**. Every consumer must therefore be
//! idempotent on [`BoundaryFactWire::id`], which is
//! `{kind}/{domain}/{generation}/{tick}/{unit}/{ordinal}` — stable across
//! replays because the tick that produced it is stable across replays.
//!
//! **E7 cut 8 independent verify fold B8.** The id used to be
//! `{kind}/{domain}/{tick}/{unit}`, which was neither unique inside one tick
//! (two crew can both resolve the same kind on one unit, and the second fact was
//! silently dropped here as a duplicate) nor unique across a demote →
//! re-promote (which restarts the tick counter near 0 and therefore re-minted
//! ids an idempotent consumer had already recorded). The generation and the
//! ordinal close both.
//!
//! **The dedupe below is an inspection-ring convenience, NOT a delivery
//! guarantee.** It scans the in-process `VecDeque` (`MAX_RETAINED_BOUNDARY_FACTS`
//! 256, empty at boot), so it holds only inside one process and only for the
//! most recent 256 facts. Cut 9's sink must deduplicate **durably** on the
//! stable id.
//!
//! **Every fact cut 8 emits is `provisional`** (fold D6(b)): the resolution
//! behind it is a `draw % 2` placeholder with no unit post-image, so a consumer
//! must opt in rather than settle off it. The flag is removed when a damage
//! model lands.
//!
//! ## Cut 8's sink
//!
//! Cut 9 (`mush-campaign`) is the first real consumer; until it exists the
//! adapter **validates, bounds and records** each fact into a bounded ring the
//! server can inspect, and refuses an undecodable payload **permanently** rather
//! than retrying it forever. Retryability is explicit, never guessed from text.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use mush_core::MAX_DURABLE_OUTBOX_PAYLOAD_BYTES;
use mush_core::{CanonicalId, ObjId, OwnerId, canonical_json_bytes_with_limit};
use mush_store::AuthoritativeOutboxRecord;
use serde::{Deserialize, Serialize};

use crate::durable_outbox::{DurableDeliveryError, DurableOutboxAdapter};

/// The owner every tactical boundary fact is stamped with.
pub const BOUNDARY_OWNER: &str = "tactics";
/// The adapter id the pulse routes its facts to.
pub const BOUNDARY_ADAPTER: &str = "tactics.boundary";
/// Facts the inspection ring retains. Bounded on records **held**, so an
/// unconsumed stream cannot grow the process.
pub const MAX_RETAINED_BOUNDARY_FACTS: usize = 256;

/// The closed vocabulary the pulse emits — `mush_sim::tick::boundary_kind`'s
/// whole range, restated here rather than imported so the adapter's refusal is a
/// property of the adapter (fold A10).
pub const BOUNDARY_KINDS: [&str; 6] = [
    "tactics.damage_controlled",
    "tactics.docked",
    "tactics.fired",
    "tactics.order_set",
    "tactics.sortie_started",
    "tactics.unit_abandoned",
];

/// The wire one boundary fact crosses on.
///
/// A strict wire (`deny_unknown_fields`) that re-runs its own invariants, so a
/// payload that could never have been emitted is refused **permanently** rather
/// than retried until the end of time (§4 rule 1 applied to the outbox).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryFactWire {
    /// `{kind}/{domain}/{generation}/{tick}/{unit}/{ordinal}` — stable across
    /// replays, unique inside a tick, and never replayed across a re-promotion.
    pub id: String,
    /// The typed fact kind (`tactics.sortie_started`, …).
    pub kind: String,
    /// The domain it happened in.
    pub domain: u64,
    /// The domain clock generation it happened on.
    #[serde(default)]
    pub generation: u64,
    /// The tick it happened on.
    pub tick: i64,
    /// The unit it happened to.
    pub unit: ObjId,
    /// Its position in the emitting tick's fact list.
    #[serde(default)]
    pub ordinal: u16,
    /// Whether the resolution behind it is a placeholder (fold D6(b)). A
    /// consumer that settles must refuse a provisional fact.
    #[serde(default)]
    pub provisional: bool,
}

impl BoundaryFactWire {
    /// Re-derive the stable id from the fact's own contents.
    ///
    /// Kept byte-identical to `mush_sim::tick::BoundaryFact::stable_id`, so the
    /// forged-id refusal below still holds.
    #[must_use]
    pub fn derived_id(&self) -> String {
        format!(
            "{}/{}/{}/{}/{}/{}",
            self.kind, self.domain, self.generation, self.tick, self.unit.0, self.ordinal
        )
    }

    /// Whether the carried id agrees with the contents. A disagreement is a
    /// **permanent** refusal: a consumer keyed on a forged id would deduplicate
    /// against the wrong fact.
    #[must_use]
    pub fn id_is_consistent(&self) -> bool {
        self.id == self.derived_id()
    }
}

/// The first production [`DurableOutboxAdapter`] on the tree.
#[derive(Clone)]
pub struct TacticsBoundaryOutbox {
    owner: OwnerId,
    adapter: CanonicalId,
    delivered: Arc<Mutex<VecDeque<BoundaryFactWire>>>,
}

impl std::fmt::Debug for TacticsBoundaryOutbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TacticsBoundaryOutbox")
            .field("owner", &self.owner)
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

impl TacticsBoundaryOutbox {
    /// Build the adapter, or `None` when the static identities will not parse
    /// (which cannot happen; the constructor is total in practice and returns
    /// an `Option` so composition never panics).
    #[must_use]
    pub fn new() -> Option<TacticsBoundaryOutbox> {
        Some(TacticsBoundaryOutbox {
            owner: OwnerId::new(BOUNDARY_OWNER).ok()?,
            adapter: CanonicalId::new(BOUNDARY_ADAPTER).ok()?,
            delivered: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    /// The facts delivered so far, oldest first. Bounded by
    /// [`MAX_RETAINED_BOUNDARY_FACTS`].
    #[must_use]
    pub fn delivered(&self) -> Vec<BoundaryFactWire> {
        self.delivered
            .lock()
            .map(|held| held.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Validate and record one payload. Split out so a test can drive it
    /// without a store.
    ///
    /// # Errors
    ///
    /// [`DurableDeliveryError::Permanent`] for an oversized, undecodable or
    /// self-inconsistent payload — none of which a retry could ever fix.
    pub fn accept(&self, payload: &serde_json::Value) -> Result<(), DurableDeliveryError> {
        canonical_json_bytes_with_limit(payload, MAX_DURABLE_OUTBOX_PAYLOAD_BYTES).map_err(
            |_| {
                DurableDeliveryError::Permanent(
                    "a tactical boundary fact exceeds its hard byte limit".to_owned(),
                )
            },
        )?;
        let fact: BoundaryFactWire = serde_json::from_value(payload.clone()).map_err(|_| {
            DurableDeliveryError::Permanent(
                "a tactical boundary fact has invalid wire data".to_owned(),
            )
        })?;
        if !fact.id_is_consistent() {
            return Err(DurableDeliveryError::Permanent(
                "a tactical boundary fact's stable id disagrees with its contents".to_owned(),
            ));
        }
        // **E7 cut 8 independent verify fold A10, the half E owns.** The kernel
        // puts NO constraint on which owner a `DurableOutboxIntent` names:
        // `DurableOutboxIntent::new` accepts any `OwnerId`, the payload hash
        // proves only self-consistency, and `with_durable_outbox` simply takes
        // the vector — so any registered provider in any module can stamp an
        // intent `tactics`/`tactics.boundary` and reach this adapter. The
        // adapter cannot ask who emitted it; what it CAN do is refuse anything
        // outside the closed vocabulary the pulse actually emits, which turns a
        // free-form forgery into a forgery that must impersonate one of five
        // exact kinds. **The residue is a K item** (recorded in 08i): the kernel
        // should record the emitting owner beside the routed one, or make
        // cross-owner emission an explicit registered capability.
        if !BOUNDARY_KINDS.contains(&fact.kind.as_str()) {
            return Err(DurableDeliveryError::Permanent(format!(
                "{:?} is not a tactical boundary fact kind this adapter carries",
                fact.kind
            )));
        }
        if fact.generation == 0 {
            return Err(DurableDeliveryError::Permanent(
                "a tactical boundary fact names the domain clock generation it happened on"
                    .to_owned(),
            ));
        }
        // At-least-once: the same stable id may arrive twice after a crash, and
        // recording it twice must not change what a consumer sees.
        if let Ok(mut held) = self.delivered.lock() {
            if held.iter().any(|existing| existing.id == fact.id) {
                return Ok(());
            }
            if held.len() >= MAX_RETAINED_BOUNDARY_FACTS {
                held.pop_front();
            }
            tracing::info!(
                id = %fact.id,
                kind = %fact.kind,
                domain = fact.domain,
                tick = fact.tick,
                "tactics: a combat boundary fact crossed the T2 seam"
            );
            held.push_back(fact);
        }
        Ok(())
    }
}

impl DurableOutboxAdapter for TacticsBoundaryOutbox {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn adapter(&self) -> &CanonicalId {
        &self.adapter
    }

    fn deliver<'a>(
        &'a self,
        item: &'a AuthoritativeOutboxRecord,
    ) -> BoxFuture<'a, Result<(), DurableDeliveryError>> {
        Box::pin(async move {
            // Routing is by `(owner, adapter)`; a misrouted item is a
            // **permanent** refusal, never a retry (the decision-wake adapter's
            // own rule).
            if item.intent().owner() != &self.owner || item.intent().adapter() != &self.adapter {
                return Err(DurableDeliveryError::Permanent(
                    "a tactical boundary fact was routed to the wrong adapter".to_owned(),
                ));
            }
            self.accept(item.intent().payload())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(tick: i64) -> serde_json::Value {
        fact_at(tick, 2, 0)
    }

    /// The E7c8 fold's corrected wire (B8/D6(b)): the id carries the clock
    /// generation and the fact's ordinal inside its tick, and every fact cut 8
    /// emits is `provisional`.
    fn fact_at(tick: i64, generation: u64, ordinal: u16) -> serde_json::Value {
        json!({
            "id": format!("tactics.sortie_started/3/{generation}/{tick}/11/{ordinal}"),
            "kind": "tactics.sortie_started",
            "domain": 3,
            "generation": generation,
            "tick": tick,
            "unit": 11,
            "ordinal": ordinal,
            "provisional": true,
        })
    }

    /// The adapter is stamped with `mush-tactics`' owner even though `mush-sim`
    /// owns the pulse that emits the fact: routing is by `(owner, adapter)`, and
    /// the facts are tactics'.
    #[test]
    fn e7c8_b4_the_boundary_outbox_is_owned_by_tactics() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        assert_eq!(outbox.owner().as_str(), "tactics");
        assert_eq!(outbox.adapter().as_str(), "tactics.boundary");
    }

    /// A fact crosses once and is **observable** on the far side; the same
    /// stable id arriving again — which is exactly what an at-least-once retry
    /// after a crash produces — changes nothing a consumer can see.
    #[test]
    fn e7c8_b4_a_boundary_fact_crosses_once_and_a_retry_changes_nothing() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        outbox.accept(&fact(4)).expect("a coherent fact delivers");
        outbox.accept(&fact(4)).expect("the retry is accepted");
        outbox.accept(&fact(5)).expect("a second tick delivers");
        let seen = outbox.delivered();
        assert_eq!(seen.len(), 2, "a retry must not duplicate the fact");
        assert_eq!(seen[0].tick, 4);
        assert_eq!(seen[1].tick, 5);
    }

    /// Every permanently-undeliverable arm refuses **permanently**, so nothing
    /// retries forever: unknown fields, a missing field, and a forged stable id.
    #[test]
    fn e7c8_b4_undecodable_unknown_field_and_forged_id_all_refuse_permanently() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        for bad in [
            json!({ "id": "x", "kind": "k", "domain": 1, "tick": 1, "unit": 1, "extra": true }),
            json!({ "kind": "k", "domain": 1, "tick": 1, "unit": 1 }),
            json!({
                "id": "tactics.fired/9/9/9/9/9",
                "kind": "tactics.sortie_started",
                "domain": 3,
                "generation": 2,
                "tick": 4,
                "unit": 11,
                "ordinal": 0,
            }),
            // **Fold A10:** a kind outside the closed vocabulary the pulse
            // emits refuses permanently, so a foreign owner's provider that
            // routes to `tactics.boundary` can at best impersonate one of the
            // six kinds rather than invent a settlement input.
            json!({
                "id": "tactics.captured/3/2/4/11/0",
                "kind": "tactics.captured",
                "domain": 3,
                "generation": 2,
                "tick": 4,
                "unit": 11,
                "ordinal": 0,
            }),
            // **Fold B8:** a fact with no generation cannot be deduplicated
            // across a re-promotion, so it is refused rather than recorded.
            json!({
                "id": "tactics.fired/3/0/4/11/0",
                "kind": "tactics.fired",
                "domain": 3,
                "generation": 0,
                "tick": 4,
                "unit": 11,
                "ordinal": 0,
            }),
        ] {
            let error = outbox.accept(&bad).expect_err("a bad payload refuses");
            assert!(
                matches!(error, DurableDeliveryError::Permanent(_)),
                "a payload a retry cannot fix must be Permanent, got {error:?}"
            );
        }
        assert!(outbox.delivered().is_empty());
    }

    // ---- E7 cut 8 independent verify fold ----

    /// **B8 (MEDIUM).** Two effective results of the same kind on ONE unit in
    /// ONE tick deliver as TWO distinct facts. Before the fold both carried the
    /// id `{kind}/{domain}/{tick}/{unit}` and the second was silently dropped
    /// here as a duplicate.
    #[test]
    fn vf_b8a_two_results_of_one_kind_on_one_unit_in_one_tick_deliver_two_distinct_facts() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        outbox
            .accept(&fact_at(5, 2, 0))
            .expect("the first delivers");
        outbox
            .accept(&fact_at(5, 2, 1))
            .expect("the second delivers");
        let seen = outbox.delivered();
        assert_eq!(seen.len(), 2, "two facts in one tick must both cross");
        assert_ne!(seen[0].id, seen[1].id);
    }

    /// **B8, the generation half.** A fact at tick 5 of two generations is two
    /// facts: a demote → re-promote restarts the tick counter near 0, so without
    /// the generation the ids an idempotent consumer had already recorded came
    /// round again.
    #[test]
    fn vf_b8b_a_fact_at_one_tick_of_two_generations_is_two_facts() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        outbox
            .accept(&fact_at(5, 2, 0))
            .expect("generation 2 delivers");
        outbox
            .accept(&fact_at(5, 9, 0))
            .expect("generation 9 delivers");
        assert_eq!(outbox.delivered().len(), 2);
    }

    /// **B8, the honest half.** The dedupe is an inspection-ring convenience,
    /// **not** a delivery guarantee: a retry that arrives after
    /// `MAX_RETAINED_BOUNDARY_FACTS` newer facts is recorded again, because the
    /// ring has forgotten it. Cut 9's sink must deduplicate durably on the
    /// stable id, and the module header says so.
    #[test]
    fn vf_b8c_a_retry_after_the_ring_has_turned_is_recorded_again_and_the_header_says_so() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        outbox.accept(&fact_at(0, 2, 0)).expect("the original");
        for tick in 1..=(MAX_RETAINED_BOUNDARY_FACTS as i64) {
            outbox.accept(&fact_at(tick, 2, 0)).expect("delivers");
        }
        let before = outbox.delivered().len();
        outbox.accept(&fact_at(0, 2, 0)).expect("the late retry");
        assert_eq!(
            outbox.delivered().len(),
            before,
            "the ring is capped, so the late retry replaced the oldest rather than              growing the process"
        );
        assert!(
            outbox.delivered().iter().any(|fact| fact.tick == 0),
            "the late retry WAS recorded again: in-process dedupe is not a delivery              guarantee, which is why cut 9 must dedupe durably"
        );
    }

    /// **D6(b).** Every fact cut 8 emits is `provisional`, so a consumer that
    /// settles must refuse it until a damage model lands.
    #[test]
    fn vf_d6_every_cut_eight_boundary_fact_is_marked_provisional() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        outbox.accept(&fact(7)).expect("delivers");
        assert!(
            outbox.delivered().iter().all(|fact| fact.provisional),
            "cut 8's resolution is a placeholder; the wire must say so"
        );
    }

    /// The ring is bounded on records held: an unconsumed stream drops the
    /// oldest rather than growing the process.
    #[test]
    fn e7c8_b4_the_inspection_ring_is_bounded_and_drops_the_oldest() {
        let outbox = TacticsBoundaryOutbox::new().expect("static identities");
        for tick in 0..(MAX_RETAINED_BOUNDARY_FACTS as i64 + 8) {
            outbox.accept(&fact(tick)).expect("delivers");
        }
        let seen = outbox.delivered();
        assert_eq!(seen.len(), MAX_RETAINED_BOUNDARY_FACTS);
        assert_eq!(seen[0].tick, 8);
    }
}
