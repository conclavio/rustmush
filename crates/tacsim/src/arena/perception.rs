//! **The sensor pass and the contact share** (16b cut S.1; doc 16 §3.6
//! "Perception", §3.2 "Line of sight" and "The spatial index", §3.1 "Rate
//! independence"; D-16-10, W1, W3, W4, W5, W6).
//!
//! One unit's pass is a [`DueKind::SensorPass`] every `sensors.pass_ms` of arena
//! time, **staggered by unit id**, applied at its own timestamp against the
//! positions the substep boundary left (doc 16 §3.1's rate-independence rule). It
//! is doc 16 §3.6's five steps and nothing else — no emitter term (W3), no
//! cross-observer cache (§3.2), no state outside the shadow contact store.
//!
//! # The five steps, and the one place they are not in the doc's order
//!
//! 1. **Candidates** from the spatial index, inside
//!    `max_range_cells × cell_cm × sensors.max_signature_factor` of the observer,
//!    excluding wrecks unless the sensor `sees_wrecks` (doc 16 §3.6 step 1, W1).
//! 2. **Line of sight** from the observer's `Eye` to the candidate's, each at its
//!    unit's `sensor_height_cm`, accepted as `Clear` or as `Partial` under the
//!    sensor's `max_attenuation` (step 2).
//! 3. **The range test**, `range_cm ≤ max_range_cm × signature_factor / 1000`,
//!    the factor read off the sensor definition's signature table by the ground
//!    the target stands on and whether it is moving (step 3).
//! 4. **The contact**: a passing candidate becomes or refreshes a shadow contact
//!    with `class = unknown`, and after `classify_after_ms` of detection with no
//!    gap longer than `pass_ms` it becomes `classified`; a contact not refreshed
//!    within `pass_ms × lost_after_passes` is lost (step 4, W5).
//! 5. **The share**: a unit that ends its pass holding anything classified
//!    schedules a [`DueKind::Share`], which hands its classified contacts to
//!    teammates inside `share_range_cells` with line of sight (step 5, W6).
//!
//! # The mesh is continuous, and what that costs
//!
//! The share runs on **every** pass of a unit that has something to say, not only
//! on the pass that classified it. It has to: a share *refreshes* the copy it hands
//! over, and the receiver's own pass expires what nothing refreshed
//! (`pass_ms × lost_after_passes`), so a mesh that only spoke on the transition
//! would hand a teammate a contact and then take it away three passes later while
//! the sharer was still looking straight at it. W6's *shared contacts fan out only
//! within comms range* is a continuous datalink, and this is that.
//!
//! The price is one `(sharer, teammate)` pair — one range test and one trace — per
//! sharer per pass, which is why `comms.share_range_cells` is a *short* reach and
//! why `[tacsim.harness].max_share_ops_per_pulse` exists at all. There is no
//! cheaper continuous mesh: a receiver that pulled instead of the sharer pushing
//! would walk the same pairs.
//!
//! **Step 3 runs before step 2**, and that is the only departure from doc 16's
//! numbering. The two are a conjunction — a candidate must pass both — so their
//! order cannot change any answer, and the range test is integer arithmetic over
//! one cell read while the trace walks up to ninety cells. Doc 16 §2 W4 bounds
//! *traces and cell probes per pulse*; running the cheap half of the conjunction
//! first is how that bound is met without weakening it.
//!
//! # Why nothing here is cached and nothing here is shared
//!
//! Doc 16 §3.2: *It is directional; there is no cross-observer cache.* A cache
//! keyed on a cell pair would make one observer's pass depend on which observer
//! ran first, which is exactly the kind of order dependence that the cross-tempo
//! property (A7) exists to refuse. So the pass is a pure function of the arena's
//! state at the event's timestamp, every observer pays for its own traces, and
//! the counters below are what makes the cost visible rather than assumed.
//!
//! # Power
//!
//! The pass does **not** require power. Doc 16 §3.3's `STARTUP` gate is written
//! against *motion and fire intents*; optics are optics, and a cold tank with an
//! intact sight can be looked out of. A wreck is different: it observes nothing,
//! because the `destroy_unit` consequence revokes its capabilities (doc 16 §3.3).

use mush_atlas::TerrainClass;
use mush_atlas::trace::{Eye, Sight, trace_cells};
use mush_core::ObjId;

use crate::arena::index::range_cm;
use crate::arena::ledger::EventKind;
use crate::arena::rng::{DrawLabel, draw, draw_contact_id};
use crate::arena::state::{
    Arena, Classification, ContactId, DueKind, ShadowContact, Team, UnitState,
};

/// The factor of a target that hides nothing, per mille.
pub const FULL_SIGNATURE_PERMILLE: u32 = 1_000;

/// **One row of a sensor's signature table**, as the arena carries it (16b cut
/// S.1; doc 16 §3.6 step 3).
///
/// The arena's own copy of `crate::definitions::sensor::SignatureRow`: the
/// content decoder's type is `serde`-shaped and lives beside the pack loader,
/// and the arena holds neither. Cut T's check-out copies the rows across
/// ([`crate::match_::arena_def::sensor_profile`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignatureFactor {
    /// The ground the target stands on.
    pub class: TerrainClass,
    /// The factor while it is under way, per mille.
    pub moving_permille: u32,
    /// The factor while it is stopped, per mille.
    pub stopped_permille: u32,
}

/// **What a unit sees with**, as the arena carries it (16b cut S.1; doc 16 §3.6).
///
/// Every field is an integer (D-16-4) and every one is the decoded form of a
/// `sensor` definition row. A unit with `None` for this runs no pass at all,
/// which is what an unmodelled hull and the unit tests of cut A have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorProfile {
    /// The candidate radius at signature factor 1, in cells (doc 16 §3.6 step 1).
    pub max_range_cells: u16,
    /// The most `Partial` attenuation a line of sight may carry and still be a
    /// detection (step 2). The atlas prices light woods at 25 and heavy at 60.
    pub max_attenuation: u8,
    /// How long a contact must be held, with no gap longer than `pass_ms`, before
    /// it classifies (step 4).
    pub classify_after_ms: u32,
    /// Whether a wreck is a candidate (step 1).
    pub sees_wrecks: bool,
    /// The comms reach a classified contact is shared along, in cells (step 5,
    /// W6). Zero shares nothing.
    pub share_range_cells: u16,
    /// The signature table, by the ground the target stands on. A class it does
    /// not name is [`FULL_SIGNATURE_PERMILLE`] in both postures.
    pub signature: Vec<SignatureFactor>,
}

impl SensorProfile {
    /// **The signature factor of a target**, per mille (doc 16 §3.6 step 3).
    ///
    /// The row for `class`, read at `moving` or at stopped; a class the table does
    /// not name is a target that hides nothing. The answer is never above
    /// [`FULL_SIGNATURE_PERMILLE`], which the definition's own `check` already
    /// refuses — asserted here too, because a factor above 1 would *lengthen* a
    /// sensor's range and that is `sensors.max_signature_factor`'s job.
    #[must_use]
    pub fn signature_permille(&self, class: TerrainClass, moving: bool) -> u32 {
        let row = self.signature.iter().find(|row| row.class == class);
        let permille = match row {
            Some(row) if moving => row.moving_permille,
            Some(row) => row.stopped_permille,
            None => FULL_SIGNATURE_PERMILLE,
        };
        permille.min(FULL_SIGNATURE_PERMILLE)
    }
}

/// **What one pulse's perception cost** (16b cut S.1; doc 16 §2 W4 and W6: *traces
/// and cell probes per pulse under `harness.max_traces_per_pulse`*, *share
/// operations per pulse bounded*).
///
/// Counted by the arena and reported on [`crate::arena::step::StepOutput`], so the
/// harness asserts a measurement rather than an estimate. They are **counters, not
/// throttles**: an engine that stopped tracing halfway through a pulse would make
/// the outcome a function of the tempo, which D-16-2 forbids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerceptionCounters {
    /// Sensor passes applied.
    pub passes: u32,
    /// Candidates the spatial index offered, after the wreck rule.
    pub candidates: u32,
    /// Line-of-sight traces walked — detection's and the share mesh's together.
    pub traces: u32,
    /// Cells read by those traces, hit or miss.
    pub cell_probes: u64,
    /// `(sharer, teammate)` pairs considered.
    pub share_ops: u32,
    /// Contacts newly held.
    pub detected: u32,
    /// Contacts newly told apart.
    pub classified: u32,
    /// Contacts lost to the time-to-live.
    pub lost: u32,
}

impl PerceptionCounters {
    /// Add another pulse's counts — the harness's running total.
    pub fn absorb(&mut self, other: &PerceptionCounters) {
        self.passes = self.passes.saturating_add(other.passes);
        self.candidates = self.candidates.saturating_add(other.candidates);
        self.traces = self.traces.saturating_add(other.traces);
        self.cell_probes = self.cell_probes.saturating_add(other.cell_probes);
        self.share_ops = self.share_ops.saturating_add(other.share_ops);
        self.detected = self.detected.saturating_add(other.detected);
        self.classified = self.classified.saturating_add(other.classified);
        self.lost = self.lost.saturating_add(other.lost);
    }
}

/// One journalled perception event: its arena timestamp, what happened, and the
/// draw where one was made.
pub type Notice = (i64, EventKind, Option<(String, u64)>);

/// A candidate that passed every test of the pass.
struct Detection {
    target: ObjId,
    team: Team,
}

impl Arena {
    /// **The time-to-live of a contact**, arena milliseconds (doc 16 §3.6 step 4):
    /// `sensors.pass_ms × sensors.lost_after_passes`, each read as at least one so
    /// that a misconfigured zero cannot make every contact expire the instant it
    /// is minted.
    #[must_use]
    pub fn contact_ttl_ms(&self) -> i64 {
        let pass = i64::from(self.sensors_cfg.pass_ms.max(1));
        let passes = i64::from(self.sensors_cfg.lost_after_passes.max(1));
        pass.saturating_mul(passes)
    }

    /// The observer's eye and the target's, as the atlas trace wants them: each
    /// unit's cell and its own `sensor_height_cm` (doc 16 §3.2 "Line of sight").
    fn eye_of(&self, unit: &UnitState) -> Option<Eye> {
        let cell = self.definition.terrain.cell_of(&unit.pose)?;
        Some(Eye {
            cell,
            height_cm: i32::try_from(unit.sensor_height_cm).unwrap_or(i32::MAX),
        })
    }

    /// **One unit's sensor pass** (doc 16 §3.6), applied at `t_ms` against the
    /// positions the substep boundary committed.
    ///
    /// Answers whether the observer **now holds anything classified**, which is
    /// what makes the caller schedule a [`DueKind::Share`]; everything else it did
    /// is in `journal` and in `counters`.
    ///
    /// The next pass is not scheduled here — [`Arena::step`] does that from the
    /// event's own timestamp, so the cadence is a function of arena time alone.
    pub fn sensor_pass(
        &mut self,
        observer: ObjId,
        pass_seq: u64,
        t_ms: i64,
        counters: &mut PerceptionCounters,
        journal: &mut Vec<Notice>,
    ) -> bool {
        // ---- the read half: who is detected, decided against one snapshot ----
        let Some(found) = self.detections(observer, counters) else {
            return false;
        };
        counters.passes = counters.passes.saturating_add(1);

        // ---- the write half: the store, then the loss band ----
        let gap_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        let classify_after_ms = self
            .unit(observer)
            .and_then(|unit| unit.sensor.as_ref())
            .map_or(i64::MAX, |sensor| i64::from(sensor.classify_after_ms));
        let seed = self.seed;
        for Detection { target, team } in found {
            let id = draw_contact_id(seed, observer, target);
            match self.contacts.get_mut(observer, id) {
                Some(held) => {
                    // Doc 16 §3.6 step 4: *no gap longer than `pass_ms`*. A gap
                    // restarts the continuity clock of a contact that has not
                    // classified yet; one that has stays classified, because
                    // telling a tank apart is not something a viewer forgets while
                    // still holding it.
                    if held.class == Classification::Unknown
                        && t_ms.saturating_sub(held.last_seen_ms) > gap_ms
                    {
                        held.first_seen_ms = t_ms;
                    }
                    held.last_seen_ms = t_ms;
                    if held.class == Classification::Unknown
                        && t_ms.saturating_sub(held.first_seen_ms) >= classify_after_ms
                    {
                        held.class = Classification::Classified { team };
                        // The draw doc 16 §3.5 pins for this transition. It
                        // decides **nothing** — classification is the elapsed-time
                        // rule above — and it is recorded so that the label stays
                        // in the ledger's pinned vocabulary and a later rule that
                        // wants a roll here finds the stream already keyed.
                        let label = DrawLabel::Classify {
                            observer,
                            target,
                            pass_seq,
                        };
                        let value = draw(seed, &label);
                        journal.push((
                            t_ms,
                            EventKind::Classified {
                                viewer: observer,
                                contact: id,
                                target,
                                team,
                            },
                            Some((label.encode(), value)),
                        ));
                        counters.classified = counters.classified.saturating_add(1);
                    }
                }
                None => {
                    self.contacts.insert(
                        observer,
                        id,
                        ShadowContact {
                            target,
                            class: Classification::Unknown,
                            first_seen_ms: t_ms,
                            last_seen_ms: t_ms,
                        },
                    );
                    journal.push((
                        t_ms,
                        EventKind::Detected {
                            viewer: observer,
                            contact: id,
                            target,
                        },
                        None,
                    ));
                    counters.detected = counters.detected.saturating_add(1);
                }
            }
        }

        // Doc 16 §3.6 step 4's loss. The cutoff is `t - ttl + 1` because
        // `ShadowContacts::expire` drops what was last seen *before* it: a contact
        // refreshed exactly `ttl` ago has gone `lost_after_passes` passes without
        // a refresh and is lost, and one refreshed more recently is not.
        let cutoff = t_ms.saturating_sub(self.contact_ttl_ms()).saturating_add(1);
        for (id, lost) in self.contacts.expire(observer, cutoff) {
            journal.push((
                t_ms,
                EventKind::Lost {
                    viewer: observer,
                    contact: id,
                    target: lost.target,
                },
                None,
            ));
            counters.lost = counters.lost.saturating_add(1);
        }
        // Whether the mesh has anything to carry, read after the losses: a unit
        // whose last classified contact just expired says nothing this pass.
        self.contacts
            .of(observer)
            .any(|(_, held)| held.class != Classification::Unknown)
    }

    /// Steps 1 to 3 of doc 16 §3.6, over one snapshot of the units and the index.
    ///
    /// `None` when the observer runs no pass at all: it is not in this match, it is
    /// a wreck, it carries no sensor, or its pose is off the window.
    fn detections(
        &self,
        observer: ObjId,
        counters: &mut PerceptionCounters,
    ) -> Option<Vec<Detection>> {
        let watcher = self.unit(observer)?;
        if watcher.limits.wreck {
            return None;
        }
        let sensor = watcher.sensor.as_ref()?;
        let from = self.eye_of(watcher)?;
        let cell_cm = u64::from(self.definition.terrain.cell_cm());
        let max_range_cm = u64::from(sensor.max_range_cells).saturating_mul(cell_cm);
        // Step 1's cull radius: the sensor's reach at the *widest* signature factor
        // any target could present (W1). Every later test only ever shortens it, so
        // a candidate outside this circle cannot be detected by any of them.
        let cull_cm = max_range_cm
            .saturating_mul(u64::from(
                self.sensors_cfg.max_signature_factor_permille.max(1),
            ))
            .saturating_div(1_000);
        let cull = u32::try_from(cull_cm).unwrap_or(u32::MAX);
        let mut found: Vec<Detection> = Vec::new();
        for id in self.index.in_range(&watcher.pose, cull, &self.units) {
            if id == observer {
                continue;
            }
            let Some(target) = self.unit(id) else {
                continue;
            };
            if target.limits.wreck && !sensor.sees_wrecks {
                continue;
            }
            counters.candidates = counters.candidates.saturating_add(1);
            let Some(to) = self.eye_of(target) else {
                continue;
            };
            // Step 3 before step 2: see this module's head. One cell read and
            // integer arithmetic, and it is a conjunct of the same test.
            let class = self
                .definition
                .terrain
                .get(to.cell.0, to.cell.1)
                .map_or(TerrainClass::Void, |cell| cell.class);
            let factor = sensor.signature_permille(class, target.speed_cms != 0);
            let reach_cm = max_range_cm
                .saturating_mul(u64::from(factor))
                .saturating_div(1_000);
            if range_cm(&watcher.pose, &target.pose) > reach_cm {
                continue;
            }
            // Step 2: the atlas trace over the arena's own cell grid.
            let trace = trace_cells(&self.definition.terrain, from, to);
            counters.traces = counters.traces.saturating_add(1);
            counters.cell_probes = counters
                .cell_probes
                .saturating_add(u64::from(trace.visited));
            if !sees(&trace.sight, sensor.max_attenuation) {
                continue;
            }
            found.push(Detection {
                target: id,
                team: target.team,
            });
        }
        Some(found)
    }

    /// **One unit's contact share** (doc 16 §3.6 step 5, W6): its classified
    /// contacts to every teammate inside `share_range_cells` with line of sight.
    ///
    /// The mesh is line of sight and the same attenuation rule detection uses (W6:
    /// *tactical comms are LOS mesh jammed by the same ECM*), so a teammate behind
    /// a wall is told nothing. A share **refreshes** what it hands over, which is
    /// what keeps a contact the mesh is still reporting from expiring in the
    /// teammate's own store.
    pub fn share_pass(
        &mut self,
        sharer: ObjId,
        t_ms: i64,
        counters: &mut PerceptionCounters,
        journal: &mut Vec<Notice>,
    ) {
        let Some(sharing) = self.unit(sharer) else {
            return;
        };
        if sharing.limits.wreck {
            return;
        }
        let Some(sensor) = sharing.sensor.as_ref() else {
            return;
        };
        if sensor.share_range_cells == 0 {
            return;
        }
        let Some(from) = self.eye_of(sharing) else {
            return;
        };
        let attenuation = sensor.max_attenuation;
        let cell_cm = u64::from(self.definition.terrain.cell_cm());
        let reach = u32::try_from(u64::from(sensor.share_range_cells).saturating_mul(cell_cm))
            .unwrap_or(u32::MAX);
        // What there is to say. Only classified contacts travel: an `unknown`
        // return is a smudge on one platform's own scope and means nothing to
        // anybody else.
        let mine: Vec<(ObjId, Team)> = self
            .contacts
            .of(sharer)
            .filter_map(|(_, held)| match held.class {
                Classification::Classified { team } => Some((held.target, team)),
                Classification::Unknown => None,
            })
            .collect();
        if mine.is_empty() {
            return;
        }
        let side = sharing.team;
        // Who to say it to, decided against the same snapshot.
        let mut mates: Vec<ObjId> = Vec::new();
        for id in self.index.in_range(&sharing.pose, reach, &self.units) {
            if id == sharer {
                continue;
            }
            let Some(mate) = self.unit(id) else {
                continue;
            };
            if mate.team != side || mate.limits.wreck {
                continue;
            }
            let Some(to) = self.eye_of(mate) else {
                continue;
            };
            counters.share_ops = counters.share_ops.saturating_add(1);
            let trace = trace_cells(&self.definition.terrain, from, to);
            counters.traces = counters.traces.saturating_add(1);
            counters.cell_probes = counters
                .cell_probes
                .saturating_add(u64::from(trace.visited));
            if !sees(&trace.sight, attenuation) {
                continue;
            }
            mates.push(id);
        }
        let seed = self.seed;
        for mate in mates {
            for (target, team) in &mine {
                // A unit is never handed a contact on itself: doc 16 §3.6's
                // contacts are what a platform holds on *something else*.
                if *target == mate {
                    continue;
                }
                let id = draw_contact_id(seed, mate, *target);
                match self.contacts.get_mut(mate, id) {
                    Some(held) => {
                        held.last_seen_ms = held.last_seen_ms.max(t_ms);
                        if held.class == Classification::Unknown {
                            held.class = Classification::Classified { team: *team };
                            journal.push((
                                t_ms,
                                EventKind::Classified {
                                    viewer: mate,
                                    contact: id,
                                    target: *target,
                                    team: *team,
                                },
                                // No draw: the classification was the sharer's,
                                // and its label is already in the ledger against
                                // that pass.
                                None,
                            ));
                            counters.classified = counters.classified.saturating_add(1);
                        }
                    }
                    None => {
                        self.contacts.insert(
                            mate,
                            id,
                            ShadowContact {
                                target: *target,
                                class: Classification::Classified { team: *team },
                                first_seen_ms: t_ms,
                                last_seen_ms: t_ms,
                            },
                        );
                        // Two transitions in one moment, both true of the teammate:
                        // it now holds the contact, and it holds it classified.
                        journal.push((
                            t_ms,
                            EventKind::Detected {
                                viewer: mate,
                                contact: id,
                                target: *target,
                            },
                            None,
                        ));
                        journal.push((
                            t_ms,
                            EventKind::Classified {
                                viewer: mate,
                                contact: id,
                                target: *target,
                                team: *team,
                            },
                            None,
                        ));
                        counters.detected = counters.detected.saturating_add(1);
                        counters.classified = counters.classified.saturating_add(1);
                    }
                }
            }
        }
    }

    /// **Doc 16 §3.1's `Alert` trigger**: some unit holds a contact whose Euclidean
    /// range is under `tempo.alert_range_cm` (which [`Arena::new`] derived when the
    /// key is zero).
    ///
    /// A derived range of zero — a match in which no unit carries a weapon — is
    /// never under itself, so nothing raises `Alert` on a contact alone. That is
    /// the honest answer: there is nothing to shoot with.
    #[must_use]
    pub fn contact_within_alert_range(&self) -> bool {
        let reach = u64::from(self.alert_range_cm);
        if reach == 0 {
            return false;
        }
        self.units.iter().any(|viewer| {
            self.contacts.of(viewer.id).any(|(_, held)| {
                self.unit(held.target)
                    .is_some_and(|target| range_cm(&viewer.pose, &target.pose) < reach)
            })
        })
    }

    /// The first [`DueKind::SensorPass`] of every unit that carries a sensor, at
    /// **`id % pass_ms`** of arena time (doc 16 §3.6: *staggered by unit id*).
    ///
    /// The stagger is the object number modulo the cadence, so it needs no state,
    /// it is the same on a replay, and two units never collide unless their ids do
    /// modulo `pass_ms`. It spreads a match's passes across the cadence rather than
    /// bunching every unit's on one substep boundary, which is what keeps the
    /// worst pulse's trace count near the average (doc 16 §2 W4).
    pub fn schedule_first_passes(&mut self) {
        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
        let ids: Vec<ObjId> = self
            .units
            .iter()
            .filter(|unit| unit.sensor.is_some())
            .map(|unit| unit.id)
            .collect();
        for id in ids {
            let offset = i64::from(id.0) % cadence;
            self.due.schedule(
                offset,
                DueKind::SensorPass {
                    unit: id,
                    pass_seq: 1,
                },
            );
        }
    }
}

/// Whether a line of sight is a detection at `max_attenuation` (doc 16 §3.6 step
/// 2): `Clear`, or `Partial` at or under the sensor's ceiling. Anything blocked is
/// nothing seen.
#[must_use]
pub fn sees(sight: &Sight, max_attenuation: u8) -> bool {
    match sight {
        Sight::Clear => true,
        Sight::Partial(pct) => *pct <= max_attenuation,
        Sight::BlockedBy(_) => false,
    }
}

/// **The one refusal for a contact id a platform does not hold** (doc 16 §3.6:
/// *unresolvable, unowned or expired* are one answer).
///
/// It is [`crate::arena::state::NO_SUCH_CONTACT`] itself, not a copy: the line a
/// pilot reads is K.2's and there is exactly one of it in the tree, so the three
/// cases cannot drift into three sentences.
#[must_use]
pub const fn contact_refusal() -> &'static str {
    crate::arena::state::NO_SUCH_CONTACT
}

/// The bearing from `from` to `to`, millidegrees clockwise from north, in
/// `0..360_000` (doc 16 §3.6 "What a pilot sees": *bearing*).
///
/// Floating point inside, quantised on the way out — doc 16 §3.1's determinism
/// rule exactly: *floating point is allowed inside a computation as long as every
/// stored result is quantised*. Two poses in one place answer `0`.
#[must_use]
pub fn bearing_mdeg(from: &mush_sim::data::Pose, to: &mush_sim::data::Pose) -> u32 {
    bearing_of(
        to.x_cm.saturating_sub(from.x_cm),
        to.y_cm.saturating_sub(from.y_cm),
    )
}

/// The same bearing, from a displacement rather than from two poses (16b cut
/// N.1).
///
/// [`bearing_mdeg`] is this function over two [`mush_sim::data::Pose`]s; the
/// reflex controller steers at *cells*, which have no pose and no opaque posture
/// blob, so it needs the arithmetic without one. One implementation, two callers,
/// and the quantisation lives in exactly one place.
#[must_use]
pub fn bearing_of(dx: i64, dy: i64) -> u32 {
    if dx == 0 && dy == 0 {
        return 0;
    }
    #[allow(clippy::cast_precision_loss)]
    let radians = (dx as f64).atan2(dy as f64);
    let degrees = radians.to_degrees();
    let mdeg = (degrees * 1_000.0).round();
    #[allow(clippy::cast_possible_truncation)]
    let whole = mdeg as i64;
    let wrapped = whole.rem_euclid(360_000);
    u32::try_from(wrapped).unwrap_or(0)
}

/// One contact as a pilot's frame carries it (doc 16 §3.6 "What a pilot sees";
/// S.2 renders it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameContact {
    /// The opaque id, the only name a pilot may use for it.
    pub id: ContactId,
    /// `unknown`, or the side the platform has resolved.
    pub class: Classification,
    /// Bearing from the platform, millidegrees clockwise from north.
    pub bearing_mdeg: u32,
    /// Range from the platform, centimetres (doc 16 §3.2's one range function).
    ///
    /// **Not rendered to a pilot** (16b cut S.2): doc 16 §3.6 gives `CONTACTS` a
    /// *range band* and not a range, so the cockpit turns this into one of
    /// `crate::match_::cockpit::range_band`'s four words and the exact figure
    /// never leaves the arena's own frame type.
    pub range_cm: u64,
    /// How long the platform has held it, arena milliseconds.
    pub age_ms: i64,
    /// **The cell the target was last seen in** (16b cut S.2), for the marker
    /// `VIEW` draws. `None` for a target that has left the window.
    ///
    /// A cell and not a pose: the map's resolution is what the map may show, so a
    /// marker carries exactly the precision the window already prints and no more.
    pub cell: Option<(u16, u16)>,
}

/// **Which transition a pilot is being told about** (16b cut S.2; doc 16 §3.6:
/// *notices fire once per transition (`acquired`, `classified`, `lost`)*).
///
/// The three words are the ledger's own `detected`, `classified` and `lost` read
/// as a pilot hears them — `acquired` rather than `detected`, because what the
/// pilot is told is that their platform now holds something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// The platform now holds a contact it did not hold before.
    Acquired,
    /// A contact it holds has been told apart.
    Classified,
    /// A contact it held has gone, and stayed gone (see [`FrameNotice`]).
    Lost,
    /// **The platform's lock broke** because the contact it was on stopped being
    /// held (16b cut W.1; doc 16 §3.5: a shot needs *a lock on a contact the platform
    /// holds*).
    ///
    /// Said **at once**, with none of [`FrameNotice`]'s hysteresis, and that is the
    /// one place the two rules part: a contact that flickers behind a treeline costs a
    /// pilot no `lost` line, but a *lock* that breaks has to be said the moment it
    /// breaks, because the pilot's next `FIRE` is going to be refused and the reason
    /// is this. The lock does not come back when the contact does — the platform
    /// re-acquires the contact and the pilot locks it again.
    LockLost,
}

impl NoticeKind {
    /// The wire word, which is the ledger's word for the same transition.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            NoticeKind::Acquired => "acquired",
            NoticeKind::Classified => "classified",
            NoticeKind::Lost => "lost",
            NoticeKind::LockLost => "lock_lost",
        }
    }
}

/// **One thing a pilot is told once** (16b cut S.2; doc 16 §3.6 "What a pilot
/// sees": *notices fire once per transition with hysteresis*).
///
/// It rides [`crate::arena::step::Frame::notices`] and the bridge turns it into a
/// line. It carries the opaque id and the class and **nothing about the target** —
/// no object number, no name, no pose — because a notice is the first thing a
/// pilot is ever told about a contact and the observer rule is strictest exactly
/// there.
///
/// # The hysteresis rule, in one sentence
///
/// A `lost` notice is **held for one further pass** (`sensors.pass_ms`) before it
/// is said: if the same platform re-acquires the same contact inside that window
/// the held notice is cancelled and the re-acquisition is **not** announced, so a
/// contact that flickers behind a treeline costs the pilot no lines at all. A loss
/// that outlives the window is announced, one pass late. The ledger's `lost` event
/// is unaffected — it is written at the moment the contact expired, which is what
/// makes the black box the record and the notice only the telling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameNotice {
    /// Which transition.
    pub kind: NoticeKind,
    /// The contact it is about, by its opaque id.
    pub contact: ContactId,
    /// What the platform knows of it at the moment of the notice.
    pub class: Classification,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::TerrainCell;
    use crate::arena::test_support::{arena_of, arena_painted, pose, tank, visual};

    #[test]
    fn the_signature_table_reads_a_row_by_ground_and_posture() {
        let sensor = visual();
        // The authored rows.
        assert_eq!(
            sensor.signature_permille(TerrainClass::WoodsHeavy, false),
            500
        );
        assert_eq!(
            sensor.signature_permille(TerrainClass::WoodsHeavy, true),
            800
        );
        assert_eq!(sensor.signature_permille(TerrainClass::Road, true), 1_000);
        assert_eq!(sensor.signature_permille(TerrainClass::Road, false), 1_000);
        // A class the table does not name hides nothing, in either posture.
        for moving in [true, false] {
            assert_eq!(
                sensor.signature_permille(TerrainClass::Water, moving),
                FULL_SIGNATURE_PERMILLE
            );
        }
        // And a row that somehow lengthened the range is clamped, whatever the
        // definition's own `check` did or did not catch.
        let wild = SensorProfile {
            signature: vec![SignatureFactor {
                class: TerrainClass::Ice,
                moving_permille: 9_000,
                stopped_permille: 9_000,
            }],
            ..visual()
        };
        assert_eq!(
            wild.signature_permille(TerrainClass::Ice, true),
            FULL_SIGNATURE_PERMILLE
        );
    }

    #[test]
    fn a_bearing_is_millidegrees_clockwise_from_north() {
        let origin = pose(1_000, 1_000);
        assert_eq!(bearing_mdeg(&origin, &pose(1_000, 2_000)), 0);
        assert_eq!(bearing_mdeg(&origin, &pose(2_000, 1_000)), 90_000);
        assert_eq!(bearing_mdeg(&origin, &pose(1_000, 0)), 180_000);
        assert_eq!(bearing_mdeg(&origin, &pose(0, 1_000)), 270_000);
        assert_eq!(bearing_mdeg(&origin, &pose(2_000, 2_000)), 45_000);
        assert_eq!(bearing_mdeg(&origin, &origin), 0);
    }

    #[test]
    fn the_time_to_live_is_the_cadence_times_the_passes() {
        let arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        // The shipped cadence and band: 1,000 ms, three passes.
        assert_eq!(arena.contact_ttl_ms(), 3_000);
    }

    #[test]
    fn a_blocked_line_is_never_a_detection_and_attenuation_has_a_ceiling() {
        assert!(sees(&Sight::Clear, 0));
        assert!(sees(&Sight::Partial(25), 40));
        assert!(sees(&Sight::Partial(40), 40));
        assert!(!sees(&Sight::Partial(41), 40));
        assert!(!sees(&Sight::BlockedBy(mush_atlas::Blocker::Edge), 100));
    }

    #[test]
    fn the_refusal_is_the_cockpits_own_line() {
        assert_eq!(contact_refusal(), "No such contact.");
        assert_eq!(contact_refusal(), crate::match_::cockpit::NO_SUCH_CONTACT);
    }

    #[test]
    fn a_wreck_observes_nothing_and_an_unsensored_unit_runs_no_pass() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        // No sensor at all: `test_support`'s tank carries none.
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        assert!(!arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal));
        assert_eq!(counters, PerceptionCounters::default());
        assert!(journal.is_empty());

        // With a sensor it sees; as a wreck it does not.
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(visual());
            unit.sensor_height_cm = 150;
        }
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.sensor_height_cm = 150;
        }
        assert!(!arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal));
        assert_eq!(counters.passes, 1);
        assert_eq!(counters.detected, 1);
        assert_eq!(arena.contacts.len(), 1);

        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.limits.wreck = true;
        }
        let mut after = PerceptionCounters::default();
        assert!(!arena.sensor_pass(ObjId(4), 2, 1_000, &mut after, &mut journal));
        assert_eq!(after, PerceptionCounters::default());
    }

    #[test]
    fn a_wreck_is_no_candidate_unless_the_sensor_sees_wrecks() {
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        for id in [ObjId(4), ObjId(9)] {
            if let Some(unit) = arena.unit_mut(id) {
                unit.sensor_height_cm = 150;
            }
        }
        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(visual());
        }
        if let Some(unit) = arena.unit_mut(ObjId(9)) {
            unit.limits.wreck = true;
        }
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal);
        assert_eq!(counters.candidates, 0, "a wreck was offered as a candidate");
        assert!(arena.contacts.is_empty());

        if let Some(unit) = arena.unit_mut(ObjId(4)) {
            unit.sensor = Some(SensorProfile {
                sees_wrecks: true,
                ..visual()
            });
        }
        let mut seeing = PerceptionCounters::default();
        arena.sensor_pass(ObjId(4), 2, 1_000, &mut seeing, &mut journal);
        assert_eq!(seeing.candidates, 1);
        assert_eq!(seeing.detected, 1);
    }

    #[test]
    fn the_stagger_is_the_object_number_modulo_the_cadence() {
        // Three ids whose remainders differ, so the schedule spreads.
        let mut arena = arena_of(vec![
            tank(ObjId(4), Team::Red, pose(1_500, 1_500)),
            tank(ObjId(9), Team::Blue, pose(2_500, 1_500)),
        ]);
        for id in [ObjId(4), ObjId(9)] {
            if let Some(unit) = arena.unit_mut(id) {
                unit.sensor = Some(visual());
                unit.sensor_height_cm = 150;
            }
        }
        arena.schedule_first_passes();
        let scheduled: Vec<(i64, ObjId)> = arena
            .due
            .iter()
            .filter_map(|event| match event.kind {
                DueKind::SensorPass { unit, pass_seq: 1 } => Some((event.t_ms, unit)),
                _ => None,
            })
            .collect();
        assert_eq!(scheduled, vec![(4, ObjId(4)), (9, ObjId(9))]);
    }

    /// The signature table, end to end through one pass on the 8 by 8 window: the
    /// target sits three cells out, and a sensor whose reach is exactly three cells
    /// holds it on a road and loses it stopped in heavy woods, because the row
    /// halves the reach. The wider properties — half range, the gap rule, the TTL,
    /// the share — are `crates/tacsim/tests/perception.rs`', which has a window
    /// big enough to put a target at half of the shipped sensor's reach.
    #[test]
    fn a_signature_row_shortens_the_reach_and_a_road_does_not() {
        for (class, held) in [
            (TerrainClass::Road, true),
            (TerrainClass::WoodsHeavy, false),
        ] {
            let mut arena = arena_painted(
                vec![
                    tank(ObjId(4), Team::Red, pose(500, 500)),
                    tank(ObjId(9), Team::Blue, pose(3_500, 500)),
                ],
                // The cell the target stands in, and nothing else: the trace runs
                // over open ground either way, so the only thing that differs is
                // the signature row.
                &[(
                    3,
                    0,
                    TerrainCell {
                        class,
                        elev_cm: 0,
                        move_cost: 150,
                    },
                )],
            );
            if let Some(unit) = arena.unit_mut(ObjId(4)) {
                unit.sensor = Some(SensorProfile {
                    // Three cells of reach against a target 3,000 cm away: full
                    // signature holds it, and the heavy-woods row's 500 does not.
                    max_range_cells: 3,
                    ..visual()
                });
            }
            let mut counters = PerceptionCounters::default();
            let mut journal: Vec<Notice> = Vec::new();
            arena.sensor_pass(ObjId(4), 1, 0, &mut counters, &mut journal);
            assert_eq!(counters.candidates, 1, "{class:?}: no candidate at all");
            assert_eq!(
                arena.contacts.len(),
                usize::from(held),
                "{class:?} answered the wrong way"
            );
        }
    }
}
