//! # `mush-propagation` — E7 cut 7: one physical context, two resolvers
//!
//! **What this crate is.** The one layer that answers *what was seen* and *what
//! arrived*, over a single shared physical context (Q4A, D-E7-14, doc 10 §16
//! item 4's R4: **one propagation layer**). Detection and delivery share
//! geometry, medium, range, occlusion, interference and time-of-flight and
//! answer different questions, which is what lets a jammed unit still take
//! orders and a courier arrive without ever having been detected.
//!
//! **What this crate is NOT, and never will be** (08i answer 1, D-E7-36, 09a
//! §3): a second contact authority. `ContactState` is `mush-core`'s record,
//! `ContactGuardValidator` is its validator, and `mush-tactics` registers both
//! along with the six migrated lifecycle actions
//! (`contact_observe/refresh/share/unshare/loss/reacquire`). This crate:
//!
//! - registers **no** contact family and **no** `AuthorityGuardKind::Contact`
//!   validator (probed);
//! - holds **no** `StateFamilyHandle` on the contact family, so
//!   `ActionTxn::checked_key` never even has to refuse it;
//! - supplies **bounded, deterministic observation intents** through
//!   `mush_core::SensorPolicy`, which the **tactics-owned** sensor schedule
//!   sources read (see `mush_tactics::sensor` for why the ingress cannot be
//!   registered here: the kernel's owner fence);
//! - owns the **false-contact phantoms** as `SubjectKey::Scoped` subjects that
//!   resolve to no unit through any projection;
//! - owns the **delivery envelopes**, `propagation.envelope`, and the coarse
//!   pass that advances them;
//! - keeps **provenance and confidence** as a side projection with **no
//!   lifecycle authority** (OPEN 2, flagged for K review).
//!
//! **The edge set** is `mush-propagation -> { mush-core, mush-sim }` and
//! nothing else — in particular **no `mush-tactics` edge**: the policy seam is
//! a `mush-core` trait installed at the composition root, and the one action
//! this crate stages by name is resolved through the definition registry, which
//! is keyed by `(owner, kind, id)` strings.
//!
//! **What cut 7 hands cut 8** (10b §3.8 owns the pulse): the per-tick
//! [`pass::detection_pass`] and [`pass::delivery_pass`] functions the pulse
//! driver calls, and the `PerceivedRef::Contact` entries tactical intents
//! target. Cut 7 builds no `DomainClock::Pulse`, no `DomainTickDriver`, no
//! `DomainClockProvider`, no domain-tick schedule source and no `ActionWindow`.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::sync::Arc;
use std::time::Duration;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence};
use mush_core::{Ctx, ModuleOp, ModuleWrite, SceneId};
use smol_str::SmolStr;

pub mod access;
pub mod admission;
pub mod billing;
pub mod commands;
pub mod commerce;
pub mod context;
pub mod dispatch;
pub mod panel;
pub mod pass;
pub mod policy;
pub mod record;
pub mod routing;
pub mod schedule;
pub mod service;

pub use commands::{
    ContactCells, NO_SUCH_CONTACT, NO_SUCH_ENVELOPE, UNAVAILABLE, contact_cells, render_contact,
    render_envelope,
};
pub use context::{
    ADJACENT_RANGE, DeliveryOutcome, DetectionOutcome, Geometry, Interference, JAMMING_BLOCKS_AT,
    MAX_CONTEXT_RECORDS, Medium, PropagationContext, domain_of_place, place_of, resolve_delivery,
    resolve_detection,
};
pub use panel::{CONTACTS_FEED, ContactsQuery, MAX_CONTACT_VIEW_ROWS};
pub use pass::{
    CLOCK_GENERATION_REASON, CONTACT_TTL_SECS, DEFAULT_SIGNATURE, DeliveryReport, DetectionReport,
    MAX_ARM_DOCUMENT_BYTES, MAX_DETECTION_VISITS, MAX_ENVELOPES_PER_PASS, MAX_OBSERVERS_PER_PASS,
    MAX_REAP_PER_PASS, MAX_SUBJECTS_PER_OBSERVER, REFRESH_AFTER_SECS, Reaped, active_order,
    active_order_from, delivery_pass, detection_pass, observers, observers_from, standing_orders,
};
pub use policy::PropagationSensorPolicy;
pub use record::{
    ALLOCATOR_KEY, ContactProvenance, ENVELOPE_TTL_DAYS, ENVELOPE_TTL_SECS, Envelope,
    EnvelopeState, FalseCause, HopRef, MAX_ENVELOPE_HOPS, MAX_ENVELOPE_PAYLOAD_BYTES,
    MAX_ENVELOPES, MAX_HELD_DOCUMENTS, MAX_PHANTOM_ID_BYTES, MAX_PHANTOMS, MAX_PROVENANCE,
    MAX_SIGHTINGS, PHANTOM_CLASS, Phantom, PropagationStore, Provenance, RecordError, Sighting,
    envelope_key, owner, phantom_key, provenance_key, sighting_key,
};

/// The module id and persistence namespace.
pub const NS: &str = "propagation";

/// The coarse delivery cadence, in seconds.
///
/// **RESOLUTION A's shape, and why.** 10b §3.7's OPEN 3 default is a
/// propagation-owned `reg.schedule_source` firing an `envelope_advance` action.
/// That is buildable, and it is not what §4.1's cut-7 row or §3.7's B7
/// classification describe: a `schedule_source`-fired action runs inside
/// `prepare_authoritative` and can write **only typed `state_family` records**,
/// which would make the envelope an **authoritative** family, `+order` an
/// `Authoritative` handler, and §4.1's "exempt — workflow, legacy-post-image"
/// row wrong. Cut 6 met the identical fork and the owner's RESOLUTION A settled
/// it the same way: a `Coarse` `reg.tick` on the world's IC clock, pulling
/// rather than subscribing. See §3.7's "As built" deviation (a).
pub const DELIVERY_TICK_SECS: u64 = 1;
/// The registered delivery tick tag. Disjoint from `sim.journey`,
/// `route.sweep` and every schedule-source id on the tree.
pub const DELIVERY_TICK_TAG: &str = "propagation.delivery";

/// The coarse **detection** cadence, in seconds.
///
/// **E7 cut 8 independent verify fold D8(a).** §3.8's deviation (f) deferred
/// this, and the deferral left cut 8's whole surface unreachable: `@prop/pass`
/// (staff) was the ONLY producer of a contact anywhere in the tree —
/// `SensorPolicy::due` has exactly one caller, `SensorScheduleAdapter::re_derived`,
/// which runs only for a `SensorJob` that is already armed — so on a running
/// server no contact ever formed, nothing on the tactical surface had a contact
/// to name, and it could not be reached end to end even before A2. This tick
/// walks the
/// **promoted** domains and runs cut 7's landed `detection_pass` per domain,
/// arming through the tactics-owned ingress exactly as `@prop/pass` does. It
/// adds no crate edge (`mush-propagation -> mush-sim` already exists) and it
/// bounds detection cost for the first time.
/// **Why 2 s and not 1 s.** `Ctx::submit_authoritative` is one per ACTOR per
/// deferred drain, and every `reg.tick` handler runs as `GOD`; a drain that
/// carries two submissions from one actor **fans out and voids both** (cut 2's
/// fold C4, applied across handlers rather than within one). `sim.domain_pulse`
/// fires every second and submits only while a domain still needs arming or
/// disarming, so a 2 s detection cadence always leaves the pulse an unshared
/// fire to get its cursor armed in, and thereafter the detection tick has every
/// even fire to itself. A detection fire that loses the slot simply arms nothing
/// and the next one repeats it — the intents are re-derived from the sighting
/// book each time, so nothing is lost.
pub const DETECTION_TICK_SECS: u64 = 2;
/// The registered detection tick tag. Disjoint from every other tick tag and
/// schedule-source id on the tree.
pub const DETECTION_TICK_TAG: &str = "propagation.detection";
/// Promoted domains one detection fire may visit. Bounded on records
/// **visited** (D-LBK-5); `MAX_ACTIVE_DOMAINS` is 8, so this never binds.
pub const MAX_DOMAINS_PER_DETECTION_FIRE: usize = 8;

/// The propagation module.
#[derive(Debug, Default)]
pub struct PropagationModule {
    metric: Option<Arc<dyn routing::MetricProvider>>,
    sources: Option<Arc<dyn dispatch::Sources>>,
    service: Option<Arc<dyn service::Integration>>,
}

impl PropagationModule {
    /// A module with default configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            metric: None,
            sources: None,
            service: None,
        }
    }

    #[must_use]
    pub fn with_service(mut self, service: Arc<dyn service::Integration>) -> Self {
        self.service = Some(service);
        self
    }

    #[must_use]
    pub fn with_metric(mut self, metric: Arc<dyn routing::MetricProvider>) -> Self {
        self.metric = Some(metric);
        self
    }

    #[must_use]
    pub fn with_dispatch_sources(mut self, sources: Arc<dyn dispatch::Sources>) -> Self {
        self.sources = Some(sources);
        self
    }
}

impl GameModule for PropagationModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // K8.5: the module's lane is declared BEFORE any command registers.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(record::PropagationStore::default());
        if let Some(sources) = &self.sources {
            reg.state(dispatch::Services(Arc::clone(sources)));
        }
        if let Some(metric) = &self.metric {
            reg.state(routing::RoutingServices(
                Arc::clone(metric),
                self.service.clone(),
            ));
        }
        // The coarse delivery pass. It writes only propagation's own exempt
        // documents; it never touches a contact, and it stages no action.
        reg.tick(
            DELIVERY_TICK_TAG,
            TickCadence::Coarse(Duration::from_secs(DELIVERY_TICK_SECS)),
            Arc::new(delivery_tick),
        )?;
        // **The detection pass, per promoted domain** (fold D8(a)). Same shape
        // as the delivery tick: a `Coarse` `reg.tick` that reads `SpatialState`,
        // `ContactLedgerState` and the `PropagationStore` off `ctx.w`, writes
        // only propagation's own exempt documents, and stages exactly ONE
        // `Ctx::submit_authoritative` of the tactics-owned `sensor_arm` action
        // per fire (cut 2's fold C4).
        reg.tick(
            DETECTION_TICK_TAG,
            TickCadence::Coarse(Duration::from_secs(DETECTION_TICK_SECS)),
            Arc::new(detection_tick),
        )?;
        commands::register(reg)?;
        if self.service.is_some() {
            reg.persist_delegation(
                NS,
                "tacsim",
                "R10 typed service decisions persist the game-owned Terms case in the same commit",
            )?;
            reg.persist_delegation(
                NS,
                "npc",
                "R3 permissioned relay evidence is filed through the knowledge owner's public API",
            )?;
        }
        service::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        mut data: ModuleBootData,
    ) -> Result<(), ModError> {
        data.normalize();
        let mut store = record::PropagationStore::default();
        let mut held = 0_usize;
        let mut dropped = 0_usize;
        let mut transport_loaded = false;
        for (key, value) in data.kv {
            if key.as_str() == routing::STATE_KEY {
                let transport: routing::TransportState = serde_json::from_value(value)
                    .map_err(|e| ModError::msg(format!("relay transport held: {e}")))?;
                transport.validate().map_err(ModError::msg)?;
                store.transport = transport;
                transport_loaded = true;
                continue;
            }
            let decoded = if key.starts_with("envelope:") {
                serde_json::from_value::<record::Envelope>(value.clone())
                    .map(|record| store.put_envelope(record).map_err(|e| e.to_string()))
                    .map_err(|error| error.to_string())
            } else if key.starts_with("phantom:") {
                serde_json::from_value::<record::Phantom>(value.clone())
                    .map(|record| store.put_phantom(record).map_err(|e| e.to_string()))
                    .map_err(|error| error.to_string())
            } else if key.starts_with("provenance:") {
                serde_json::from_value::<record::ContactProvenance>(value.clone())
                    .map(|record| store.put_provenance(record).map_err(|e| e.to_string()))
                    .map_err(|error| error.to_string())
            } else if key.starts_with("sighting:") {
                serde_json::from_value::<record::Sighting>(value.clone())
                    .map(|record| store.put_sighting(record).map_err(|e| e.to_string()))
                    .map_err(|error| error.to_string())
            } else if key.as_str() == record::ALLOCATOR_KEY {
                store.install_allocator(&value);
                Ok(Ok(()))
            } else {
                Ok(Ok(()))
            };
            let failed = match decoded {
                Ok(Ok(())) => None,
                Ok(Err(error)) | Err(error) => Some(error),
            };
            if let Some(error) = failed {
                if value.get("dispatch").is_some_and(|v| !v.is_null())
                    || value
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|k| matches!(k, "mail" | "channel"))
                {
                    return Err(ModError::msg(format!(
                        "message handoff {key} is unreadable: {error}"
                    )));
                }
                // **Skipped loudly, and HELD — never deleted** (cut 6's R3: a
                // document that will not decode is the only copy of the fact it
                // carries, and a boot that deletes it destroys it). The RAM copy
                // is bounded (fold B7); past the cap the document is still in
                // the store, it is simply not mirrored, and the count is paged.
                held = held.saturating_add(1);
                tracing::warn!(
                    key = %key,
                    error = %error,
                    "propagation: a record would not load; it is held, not deleted"
                );
                if !store.hold(key.clone(), value) {
                    dropped = dropped.saturating_add(1);
                }
                debug_assert!(dropped <= held);
            }
        }
        if !transport_loaded && store.envelopes().any(|e| e.transit().is_some()) {
            return Err(ModError::msg(
                "relay_transport missing beside routed envelopes; restore the committed capacity record",
            ));
        }
        if store.transport.service.is_none()
            && store
                .envelopes()
                .any(|e| e.transit().is_some_and(|r| r.service.is_some()))
        {
            return Err(ModError::msg(
                "relay service policy missing beside class tickets; restore committed transport state",
            ));
        }
        if store.transport.service.is_none()
            && let Some(api) = &self.service
        {
            let config = api.seed().map_err(ModError::msg)?;
            config.validate().map_err(ModError::msg)?;
            store.transport.service = Some(service::State::new(config));
            ctx.persist(ModuleOp {
                ns: NS.into(),
                op: ModuleWrite::JsonSet {
                    key: routing::STATE_KEY.into(),
                    value: store.transport.to_value(),
                },
            });
        }
        if let Some(state) = &store.transport.service {
            schedule::validate_bindings(ctx.w, state).map_err(ModError::msg)?;
        }
        store.reconcile_allocators();
        let dispatches: Vec<_> = store.envelopes().filter_map(|e| e.dispatch_ref()).collect();
        let keys: std::collections::BTreeSet<_> = dispatches.iter().map(|d| &d.key).collect();
        if dispatches.len() > dispatch::MAX_DISPATCHES || keys.len() != dispatches.len() {
            return Err(ModError::msg("invalid or duplicate dispatch handoffs"));
        }
        if held > 0 {
            // **Paged, not logged** (E4c V6): `@prop/held` is the verb that can
            // actually show them, and it names the key and the decode error.
            store.page(format!(
                "propagation: boot held {held} unreadable record(s) ({} past the RAM cap of {}); \
                 run @prop/held to inspect them.",
                dropped,
                record::MAX_HELD_DOCUMENTS
            ));
            tracing::warn!(
                held,
                dropped,
                "propagation: boot held unreadable records; run @prop/held to inspect"
            );
        }
        if let Some(slot) = ctx.w.module_state_mut::<record::PropagationStore>() {
            *slot = store;
        }
        Ok(())
    }
}

/// One coarse delivery fire.
///
/// **Panic-safe and bounded**, with the bound **paged rather than logged** (E4c
/// V6): a pass that stops at its visit bound records the fact in the store's
/// report and `@prop/pass` prints it, so the operator is told rather than a log
/// line being written into the dark.
fn delivery_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    dispatch::reconcile(ctx);
    commerce::sweep(ctx);
    billing::maintain(ctx);
    let at = ctx.w.world_ic_now();
    let Some(state) = ctx.w.module_state::<mush_sim::SpatialState>().cloned() else {
        return;
    };
    let ledger = ctx
        .w
        .module_state::<mush_core::ContactLedgerState>()
        .cloned()
        .unwrap_or_default();
    let Some(mut book) = ctx.w.module_state::<record::PropagationStore>().cloned() else {
        return;
    };
    let old_allocator = book.allocator();
    // **Re-anchor propagation's OWN records first** (fold D5(a)(ii)). A world
    // rate change opens a new `WorldIc` generation whose IC scalar is continuous
    // with the old one; without this an `Envelope::age` answered 0 for ever and
    // a `Phantom` never expired, so no envelope aged out and no holder of a
    // false contact ever received its `Loss`.
    let reanchored = reanchor(&mut book, &at);
    let metric = routing::snapshot_in(ctx, &at);
    let report =
        pass::delivery_pass_with_metric(&mut book, &state, ctx.w, &ledger, &at, None, metric);
    if !commerce::settle_pass(ctx, &mut book) {
        return;
    }
    let allocator = book.allocator();
    if allocator != old_allocator {
        ctx.persist(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::JsonSet {
                key: record::ALLOCATOR_KEY.into(),
                value: allocator,
            },
        });
    }
    if !report.routed.is_empty() {
        ctx.persist(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::JsonSet {
                key: routing::STATE_KEY.into(),
                value: book.transport.to_value(),
            },
        });
    }
    // The report is NEVER dropped: the bound, the anomalies and the compaction
    // are paged into the store's own notice queue, which `@prop/pass` and
    // `@prop/held` drain (fold B3).
    if report.bound_hit {
        book.page(format!(
            "The delivery tick stopped at its bound of {} envelopes and resumes at #{}.",
            pass::MAX_ENVELOPES_PER_PASS,
            report.resume.unwrap_or(0)
        ));
    }
    for anomaly in &report.anomalies {
        book.page(anomaly.clone());
        tracing::warn!(detail = %anomaly, "propagation: delivery pass anomaly");
    }
    if reanchored > 0 {
        book.page(format!(
            "A world time-rate change re-anchored {reanchored} propagation record(s); contacts \
             stranded on the older generation are retired by the next detection pass."
        ));
    }
    if report.is_empty() && report.anomalies.is_empty() && reanchored == 0 {
        if let Some(slot) = ctx.w.module_state_mut::<record::PropagationStore>() {
            *slot = book;
        }
        return;
    }
    let touched = report.touched();
    let envelopes: Vec<record::Envelope> = touched
        .iter()
        .filter_map(|id| book.envelope(*id).cloned())
        .collect();
    let rewritten: Vec<record::Envelope> = if reanchored > 0 {
        book.envelopes()
            .filter(|envelope| !touched.contains(&envelope.id()))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let phantoms: Vec<record::Phantom> = if reanchored > 0 {
        book.phantoms().cloned().collect()
    } else {
        Vec::new()
    };
    let reaped = report.reaped.clone();
    if let Some(slot) = ctx.w.module_state_mut::<record::PropagationStore>() {
        *slot = book;
    }
    for envelope in envelopes.into_iter().chain(rewritten) {
        let Ok(value) = serde_json::to_value(&envelope) else {
            tracing::error!(
                envelope = envelope.id(),
                "propagation: an envelope post-image would not serialize; run @prop/pass"
            );
            continue;
        };
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::JsonSet {
                key: record::envelope_key(envelope.id()),
                value,
            },
        });
    }
    for phantom in phantoms {
        let Ok(value) = serde_json::to_value(&phantom) else {
            continue;
        };
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::JsonSet {
                key: record::phantom_key(phantom.id()),
                value,
            },
        });
    }
    // The compaction half: a drop is a `ModuleWrite::Del`, beside the post-images.
    for id in reaped.envelopes {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: record::envelope_key(id),
            },
        });
    }
    for contact in reaped.provenance {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: record::provenance_key(contact),
            },
        });
    }
    for id in reaped.phantoms {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: record::phantom_key(&id),
            },
        });
    }
}

/// **One coarse detection fire, over the promoted domains** (fold D8(a)).
///
/// It walks `SpatialState.domains` for `clock.is_pulsing()`, runs
/// [`pass::detection_pass`] scoped to each promoted domain — the `domain` and
/// `arm_bound` parameters cut 7's own fold landed, precisely so a per-domain
/// pulse could do per-domain work — and arms the result through the
/// **tactics-owned** `sensor_arm` ingress by definition-registry lookup, the
/// same way `@prop/pass` does. It registers nothing of tactics', writes no
/// contact, and stages exactly **one** `Ctx::submit_authoritative` per fire.
///
/// The report is never dropped: `bound_hit`, `resume`, `stranded` and every
/// refusal are **paged** into the store's own notice queue, which `@prop/pass`
/// and `@prop/held` drain (E4c V6).
fn detection_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let at = ctx.w.world_ic_now();
    let Some(state) = ctx.w.module_state::<mush_sim::SpatialState>().cloned() else {
        return;
    };
    let promoted: Vec<mush_sim::DomainId> = state
        .domains
        .iter()
        .filter(|(_, record)| record.clock.is_pulsing())
        .map(|(id, _)| *id)
        .take(MAX_DOMAINS_PER_DETECTION_FIRE)
        .collect();
    if promoted.is_empty() {
        return;
    }
    let visited = promoted.len();
    let bound_hit_domains = state
        .domains
        .values()
        .filter(|record| record.clock.is_pulsing())
        .count()
        > visited;
    let ledger = ctx
        .w
        .module_state::<mush_core::ContactLedgerState>()
        .cloned()
        .unwrap_or_default();
    let Some(mut book) = ctx.w.module_state::<record::PropagationStore>().cloned() else {
        return;
    };
    let arm_bound = commands::max_intents_per_arm();
    let mut intents: Vec<mush_core::ObservationIntent> = Vec::new();
    let mut sightings: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    let mut stranded = 0_usize;
    for domain in promoted {
        let report = pass::detection_pass(
            &mut book,
            &state,
            ctx.w,
            &ledger,
            &at,
            Some(domain),
            arm_bound,
        );
        sightings.extend(report.sightings.iter().cloned());
        dropped.extend(report.dropped_sightings.iter().cloned());
        stranded = stranded.saturating_add(report.stranded);
        for refusal in &report.refused {
            notices.push(refusal.clone());
        }
        if report.bound_hit {
            notices.push(format!(
                "The detection pass for domain #{domain} stopped at its bound of {arm_bound} \
                 observation(s){}; the next fire resumes there.",
                report
                    .resume
                    .map(|(observer, subject)| format!(
                        " and resumes at observer #{}, subject #{}",
                        observer.0, subject.0
                    ))
                    .unwrap_or_default()
            ));
        }
        intents.extend(report.intents);
    }
    if bound_hit_domains {
        notices.push(format!(
            "The detection tick stopped at its bound of {MAX_DOMAINS_PER_DETECTION_FIRE} promoted \
             domain(s); the next fire covers the rest."
        ));
    }
    if stranded > 0 {
        notices.push(format!(
            "{stranded} contact(s) were stranded on an older world time generation and are being \
             retired; their holders will see them go lost."
        ));
    }
    // **C4:** one submission per fire. A pass with more due work than one arm
    // carries says so and the next fire takes the rest.
    intents.sort_by(|left, right| left.key().as_str().cmp(right.key().as_str()));
    intents.dedup_by(|left, right| left.key() == right.key());
    let armed = intents.len().min(arm_bound);
    if intents.len() > armed {
        notices.push(format!(
            "{} observation(s) were due past the per-arm bound of {arm_bound}; the next detection \
             fire arms them.",
            intents.len() - armed
        ));
    }
    let batch: Vec<mush_core::ObservationIntent> = intents.into_iter().take(armed).collect();
    let written: Vec<record::Sighting> = sightings
        .iter()
        .filter_map(|key| book.sighting(key).cloned())
        .collect();
    for notice in &notices {
        book.page(notice.clone());
    }
    if let Some(slot) = ctx.w.module_state_mut::<record::PropagationStore>() {
        *slot = book;
    }
    for sighting in &written {
        if let Ok(value) = serde_json::to_value(sighting) {
            ctx.persist(ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::JsonSet {
                    key: record::sighting_key(&sighting.key()),
                    value,
                },
            });
        }
    }
    for key in &dropped {
        ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::Del {
                key: record::sighting_key(key),
            },
        });
    }
    if batch.is_empty() {
        return;
    }
    let Some(action) = commands::sensor_arm_action_ref(ctx.reg) else {
        return;
    };
    let (Ok(document), Ok(key)) = (
        serde_json::to_string(&batch),
        mush_core::IdempotencyKeyText::parse_canonical(format!("prop/detect/{:x}", at.at)),
    ) else {
        return;
    };
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "intents": document }),
        key,
    );
}

/// Move propagation's own records onto the current clock generation, keeping
/// their IC scalars, and answer how many moved (fold D5).
///
/// `ContactState` is **not** re-anchored here and cannot be: it is K's record
/// behind an owner-fenced family. The stranded contacts are retired by
/// `detection_pass`'s `clock_generation` `Loss` instead, which the kernel now
/// admits because loss is terminal.
fn reanchor(book: &mut record::PropagationStore, at: &mush_core::ClockStamp) -> usize {
    let stale_envelopes: Vec<record::Envelope> = book
        .envelopes()
        .filter(|envelope| envelope.issued().clock != at.clock)
        .take(pass::MAX_REAP_PER_PASS)
        .map(|envelope| envelope.reanchored(&at.clock))
        .collect();
    let stale_phantoms: Vec<record::Phantom> = book
        .phantoms()
        .filter(|phantom| phantom.created().clock != at.clock)
        .take(pass::MAX_REAP_PER_PASS)
        .map(|phantom| phantom.reanchored(&at.clock))
        .collect();
    let mut moved = 0_usize;
    for envelope in stale_envelopes {
        if book.put_envelope(envelope).is_ok() {
            moved = moved.saturating_add(1);
        }
    }
    for phantom in stale_phantoms {
        if book.put_phantom(phantom).is_ok() {
            moved = moved.saturating_add(1);
        }
    }
    moved
}
