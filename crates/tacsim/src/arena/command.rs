//! **What the arena tells the world about its squads** (16b cut N.2, N3; doc 16
//! §6 "Deliberation": *event-driven with a floor of `command.deliberate_min_ms`*).
//!
//! Deliberation is the world's: the request goes through the world task's own LLM
//! re-entry seam, the reply comes back as an ordinary [`crate::arena::Intent`], and the arena
//! never waits for either (doc 16 §6 "Provider failure": *the pipeline's retries,
//! the breaker and the fallback never block a pulse*). But the **trigger** is the
//! arena's, because doc 16 §6 puts it on *the commander's contact set*, and a
//! contact set is a thing only the arena can see.
//!
//! So this module is exactly one thing: a bounded, allocation-light report the
//! pulse leaves on [`crate::arena::step::StepOutput`] saying *this squad has
//! something new to decide about*. It reads, it never writes anything a pilot
//! could observe, and it is the whole of the arena's part in deliberation.
//!
//! # The floor lives here, and not at the world
//!
//! `[tacsim.arena].deliberate_min_ms` is enforced **in arena time, in this
//! module**, for D-16-2's reason once more: a floor counted at the world would be
//! a floor in wall time, and two runs of the same match at two tempos would make
//! a different number of requests. Counted here, the same scripted match asks the
//! same number of times at 5,000, 1,000 and 500 ms, and a world that drops a
//! signal loses nothing — the next change re-signals, because the arena remembers
//! what it last reported and not what the world did with it.
//!
//! # What crosses, and what may not
//!
//! A signal carries **counts, bands and opaque contact ids** — the same things
//! `CONTACTS` prints for a pilot (doc 16 §3.6 "What a pilot sees"). It never
//! carries a target's object id, a target's pose, or anything about an *other*
//! unit the commander does not hold. That is not politeness: the signal becomes
//! the prompt a model reads, and doc 16 §3.6's observer rule does not stop being
//! true because the observer is a provider.
//!
//! **The one other thing it carries is the squad itself** (16b cut Z.1, 16c N6;
//! [`SignalMember`]): each member of a reported squad as it knows itself — its
//! reflex state, its health, and its range and bearing from the commander, read
//! off its own pose whether or not the commander holds it as a contact. That is
//! the squad's own knowledge relayed to its commander (doc 16 §3.6 read for
//! machines: a squad acts on what its own units hold), never the other side's.
//! A member's object id rides the signal for the reply's sake and is never
//! printed: the prompt names a member by its ordinal
//! ([`crate::deliberation::request::user_prompt`]).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;

use crate::arena::reflex::ReflexState;
use crate::arena::state::{Arena, Classification, ContactId, Team};
use crate::deliberation::squad::{SquadId, SquadRoster};

/// The most contacts one signal carries, nearest first.
///
/// Twelve, because a commander with more than twelve contacts is a commander
/// whose order will be about the shape of the fight and not about one tank, and
/// because the prompt this becomes has to stay bounded whatever the field does.
pub const MAX_SIGNAL_CONTACTS: usize = 12;

/// **What the commander holds, reduced to three numbers** — doc 16 §6's *changes
/// class or count*.
///
/// Class and count and nothing else: a digest that moved because a contact's
/// bearing changed by a degree would ask for a new order every pass, which is the
/// floor doing all the work and the event-driven trigger doing none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SquadDigest {
    /// Contacts held but not told apart.
    pub unknown: u16,
    /// Contacts resolved to the other side.
    pub hostile: u16,
    /// Contacts resolved to this side.
    pub friendly: u16,
}

/// One contact as a signal carries it — the opaque id and the two numbers the
/// cockpit already shows a pilot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalContact {
    /// The opaque arena id, the only name an order may use for it.
    pub id: ContactId,
    /// What the commander knows of it.
    pub class: Classification,
    /// Bearing from the commander, millidegrees clockwise from north.
    pub bearing_mdeg: u32,
    /// Range from the commander, centimetres. The *world* turns this into one of
    /// the cockpit's four band words before it reaches a prompt.
    pub range_cm: u64,
}

/// **One member of a reported squad, as its own facts** (16b cut Z.1, 16c N6).
///
/// The squad's picture is the union of what its units hold **of themselves** —
/// doc 16 §3.6 read for machines: a squad acts on what its own units hold, and a
/// unit always holds its own state, its own hull and its own place. So a member
/// row is never a contact, never a plot and never anything about the other side;
/// it is the commander's own squadmate as that squadmate knows itself. The range
/// and bearing are from the commander's pose to the member's own pose, which is
/// the squad's radio net and not a sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalMember {
    /// The member's object id. It rides the seam so a later order may name a
    /// member (cut Z.2's `support_member`) and **never reaches a prompt**, as
    /// [`SquadSignal::commander`] does not.
    pub unit: ObjId,
    /// `1` for the commander, then roster order.
    pub ordinal: u16,
    /// The reflex controller's state, [`ReflexState::Startup`] while the
    /// controller has no row for it yet.
    pub state: ReflexState,
    /// Its health, per mille of its authored totals (`reflex_health_permille`,
    /// the number the squad's mean is taken over).
    pub health_permille: u32,
    /// Centimetres from the commander's pose to this member's own; `0` for the
    /// commander.
    pub range_cm: u64,
    /// Millidegrees clockwise from north, from the commander to this member; `0`
    /// for the commander.
    pub bearing_mdeg: u32,
}

/// **Why this squad is being reported** (doc 16 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalReason {
    /// The squad has never had an order.
    NoOrder,
    /// The commander's contact set changed class or count.
    ContactsChanged,
    /// An order ran out its time to live and the squad is on doctrine.
    OrderExpired,
}

impl SignalReason {
    /// The wire word — what the prompt and the staff notice name it by.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SignalReason::NoOrder => "no_order",
            SignalReason::ContactsChanged => "contacts_changed",
            SignalReason::OrderExpired => "order_expired",
        }
    }
}

/// **One squad, as the world is told about it.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SquadSignal {
    /// Which squad.
    pub squad: SquadId,
    /// Its commander.
    pub commander: ObjId,
    /// Its side.
    pub team: Team,
    /// Why now.
    pub reason: SignalReason,
    /// Arena milliseconds at the signal.
    pub at_ms: i64,
    /// What the commander holds, in three numbers.
    pub digest: SquadDigest,
    /// The commander's contacts, nearest first, at most [`MAX_SIGNAL_CONTACTS`].
    pub contacts: Vec<SignalContact>,
    /// **The authored band edges of the weapon the commander's bare `FIRE` would
    /// fire**, centimetres, outward (16b cut F.3) — what turns a contact's range
    /// into a band, and the same list the pilot's own `CONTACTS` band is read
    /// against. It replaces cut N.2's `reach_cm`, which banded in thirds of the
    /// longest reach and so answered `close` for everything a sight can see.
    pub bands_cm: Vec<u32>,
    /// **The commander's own nominal sight**, centimetres (16b cut E.1;
    /// [`crate::arena::state::UnitState::sight_cm`]), `0` for a hull with no
    /// sensor.
    ///
    /// It is here because the band word is thirds of the *lesser* of the sight
    /// and [`SquadSignal::bands_cm`]'s last edge, and cut D.1 had left the
    /// commander's brief reading the gun alone — so a lead's `close` and a
    /// pilot's `close` were two distances (16c `T-81`). The brief prints the word
    /// and not the number: what a deliberating mind is owed is the vocabulary a
    /// pilot uses, and one more centimetre figure in the prompt is a number it
    /// would have to do arithmetic on.
    pub sight_cm: u32,
    /// **How long an order stands when the model does not say** (16b cut E.2;
    /// 16c `T-56`), milliseconds.
    ///
    /// `[tacsim.arena].deliberate_min_ms` clamped into
    /// `[crate::deliberation::order::MIN_TTL_MS,
    /// crate::deliberation::order::MAX_TTL_MS]` — the floor under two requests
    /// for one squad, which is exactly how long an order has to cover for the
    /// squad not to be on doctrine while it waits for the next one. Before this
    /// cut an omitted `ttl_ms` became `MIN_TTL_MS`, a thousand milliseconds:
    /// dead at the next decide tick, with the commander's whole round trip spent
    /// on an order nobody carried out.
    ///
    /// It rides the signal, and [`crate::deliberation::request::SquadBrief`]
    /// copies it, so the default stays with the request that produced it across
    /// the world's async hop. **The prompt never prints it**: the model is not
    /// told what its silence means, the forced tool still requires `ttl_ms`, and
    /// only the JSON-in-text fallback can omit one — the same posture
    /// `SquadBrief::commander` has.
    pub default_ttl_ms: u32,
    /// The squad's mean health, per mille of the authored totals.
    pub health_permille: u32,
    /// How many of the squad's machines are in each state, in
    /// [`ReflexState::ALL`] order.
    pub states: [u32; 6],
    /// The commander's cell, so an `advance` order can be written relative to
    /// somewhere.
    pub cell: Option<(u16, u16)>,
    /// The window's `(columns, rows)`, so an `advance` order can be bounded.
    pub window: (u16, u16),
    /// **The squad's own units, as their own facts** (16b cut Z.1, 16c N6): one
    /// row per roster unit the arena still holds, in roster order, the commander
    /// first at ordinal `1`. Filled only for a squad that is being reported.
    pub members: Vec<SignalMember>,
}

/// What the arena remembers about one squad between signals.
///
/// A **view**, exactly as `Arena::last_frames` is (`Arena::held_losses` was the
/// example here until 16b cut M.2 folded it): it is not folded into the state
/// hash, because it is a fact about what the world has been told and not about the
/// match. Folding it would make the hash a function of the request cadence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SquadWatch {
    /// The digest last reported.
    digest: SquadDigest,
    /// The arena millisecond it was last reported at; `None` before the first.
    last_ms: Option<i64>,
}

impl Arena {
    /// The squad a unit belongs to, if any.
    #[must_use]
    pub fn squad_of(&self, unit: ObjId) -> Option<&SquadRoster> {
        self.squads.iter().find(|squad| squad.holds(unit))
    }

    /// Note that a unit's standing order expired — called by the reflex tick, so
    /// the next signal says why it is asking.
    pub(crate) fn note_order_expired(&mut self, squad: SquadId) {
        self.squads_expired.insert(squad);
    }

    /// **The squads with something new to decide about** — taken once per pulse by
    /// [`crate::arena::Arena::step`] and reported on its output.
    ///
    /// The cost is one pass over the squads, and for each of them **at most one
    /// counting pass** over the contacts its commander already holds — the same
    /// bounded read [`Arena::hostile_in_weapons_range`] makes, never a sweep over
    /// pairs of units. A squad inside the floor costs one comparison and stops, and
    /// a squad whose digest has not moved allocates nothing: the contact *list* is
    /// built only for a squad that is actually being reported, which at
    /// `deliberate_min_ms` of 30,000 is a handful of squads a minute out of the
    /// harness's sixty-four.
    pub(crate) fn take_deliberation_signals(&mut self) -> Vec<SquadSignal> {
        if self.squads.is_empty() {
            return Vec::new();
        }
        let floor = i64::from(self.arena_cfg.deliberate_min_ms);
        let now = self.now_ms;
        let window = (
            self.definition.terrain.width(),
            self.definition.terrain.height(),
        );
        let mut expired = std::mem::take(&mut self.squads_expired);
        let mut watch = std::mem::take(&mut self.squad_watch);
        let mut reported: Vec<SquadId> = Vec::new();
        let mut out: Vec<SquadSignal> = Vec::new();
        for squad in &self.squads {
            // A commander that is gone or a wreck commands nothing: doc 16 §6's
            // fallback is the reflex doctrine, and a decapitated squad is already
            // on it.
            let Some(commander) = self.unit(squad.commander) else {
                continue;
            };
            if commander.limits.wreck || commander.disabled {
                continue;
            }
            let row = watch.entry(squad.id).or_default();
            // **The floor first**, because it is one comparison and it is what a
            // squad in a firefight fails nine times out of ten. An expiry gets no
            // pass through it — a squad whose commander is unreachable would
            // otherwise ask once per expiry for ever — but it is **kept**: the flag
            // stays in the set until the request it asked for is actually made, so
            // an order whose time to live was shorter than the floor still gets its
            // squad a new one, one floor later, rather than silently leaving it on
            // doctrine until something else happens to change.
            if let Some(last) = row.last_ms
                && now.saturating_sub(last) < floor
            {
                continue;
            }
            // Then the digest, which is three counters and no allocation.
            let mut digest = SquadDigest::default();
            for (_, held) in self.contacts.of(squad.commander) {
                match held.class {
                    Classification::Unknown => digest.unknown = digest.unknown.saturating_add(1),
                    Classification::Classified { team } if team == commander.team => {
                        digest.friendly = digest.friendly.saturating_add(1);
                    }
                    Classification::Classified { .. } => {
                        digest.hostile = digest.hostile.saturating_add(1);
                    }
                }
            }
            let reason = if row.last_ms.is_none() {
                SignalReason::NoOrder
            } else if expired.contains(&squad.id) {
                SignalReason::OrderExpired
            } else if row.digest != digest {
                SignalReason::ContactsChanged
            } else {
                continue;
            };
            row.digest = digest;
            row.last_ms = Some(now);
            reported.push(squad.id);
            // Only now, for a squad that is actually being reported, the list the
            // prompt is written from.
            let mut contacts: Vec<SignalContact> = self
                .contacts
                .of(squad.commander)
                .map(|(id, held)| {
                    // **16b cut F.3**: the commander's brief is written from the
                    // commander's **plot** — where its platform last saw the
                    // target — for the same reason the pilot's contact row is.
                    // A deliberating mind that was handed a target's live pose
                    // would be told something no platform on the field knows.
                    let (range_cm, bearing_mdeg) =
                        self.unit(held.target).map_or((u64::MAX, 0), |_| {
                            (
                                held.seen_at.range_from(&commander.pose),
                                held.seen_at.bearing_from(&commander.pose),
                            )
                        });
                    SignalContact {
                        id: *id,
                        class: held.class,
                        bearing_mdeg,
                        range_cm,
                    }
                })
                .collect();
            // Nearest first, and then by id so two contacts at one range are always
            // reported in the same order (nothing here draws).
            contacts.sort_by_key(|contact| (contact.range_cm, contact.id));
            contacts.truncate(MAX_SIGNAL_CONTACTS);
            let mut states = [0_u32; 6];
            let mut health_total: u64 = 0;
            let mut health_count: u64 = 0;
            for unit in &squad.units {
                if let Some(machine) = self.reflex.unit(*unit)
                    && let Some(slot) = states.get_mut(usize::from(machine.state.code()))
                {
                    *slot = slot.saturating_add(1);
                }
                if self.unit(*unit).is_some() {
                    health_total =
                        health_total.saturating_add(u64::from(self.reflex_health_permille(*unit)));
                    health_count = health_count.saturating_add(1);
                }
            }
            // **16b cut Z.1** (16c N6): the squad's own picture — every roster
            // unit the arena still holds, as it knows itself. Built here, beside
            // the contact list, so a squad that is not being reported allocates
            // nothing for it. A member's own pose is read directly: it is the
            // member's own knowledge, relayed over the squad's net, and no
            // sensor of either side is consulted.
            let mut members: Vec<SignalMember> = Vec::with_capacity(squad.units.len());
            for unit in &squad.units {
                let Some(member) = self.unit(*unit) else {
                    continue;
                };
                let Ok(ordinal) = u16::try_from(members.len().saturating_add(1)) else {
                    break;
                };
                let (range_cm, bearing_mdeg) = if *unit == squad.commander {
                    (0, 0)
                } else {
                    let own = crate::arena::state::SeenPose::of(member);
                    (
                        own.range_from(&commander.pose),
                        own.bearing_from(&commander.pose),
                    )
                };
                members.push(SignalMember {
                    unit: *unit,
                    ordinal,
                    state: self
                        .reflex
                        .unit(*unit)
                        .map_or(ReflexState::Startup, |machine| machine.state),
                    health_permille: self.reflex_health_permille(*unit),
                    range_cm,
                    bearing_mdeg,
                });
            }
            out.push(SquadSignal {
                squad: squad.id,
                commander: squad.commander,
                team: commander.team,
                reason,
                at_ms: now,
                digest,
                contacts,
                bands_cm: commander.fire_bands_cm(),
                // 16b cut E.1: the same product the commander's own frame carries,
                // so the brief's band word is the word its pilots would read.
                sight_cm: commander.sight_cm(self.definition.terrain.cell_cm()),
                // **16b cut E.2** (16c `T-56`): the floor this module enforces,
                // read here where it is already in hand, so an order that names
                // no time to live stands exactly as long as the squad will wait
                // for its next one.
                default_ttl_ms: self.arena_cfg.deliberate_min_ms.clamp(
                    crate::deliberation::order::MIN_TTL_MS,
                    crate::deliberation::order::MAX_TTL_MS,
                ),
                health_permille: u32::try_from(health_total / health_count.max(1)).unwrap_or(1_000),
                states,
                cell: self.definition.terrain.cell_of(&commander.pose),
                window,
                members,
            });
        }
        self.squad_watch = watch;
        // Whatever was asked for is answered; whatever the floor held back is still
        // owed, and waits for the next pulse past it.
        for squad in reported {
            expired.remove(&squad);
        }
        self.squads_expired = expired;
        out
    }
}

/// Every machine state in one place, so a caller can render a distribution
/// without importing the enum.
#[must_use]
pub fn state_words() -> [&'static str; 6] {
    let mut out = [""; 6];
    for (at, state) in ReflexState::ALL.iter().enumerate() {
        if let Some(slot) = out.get_mut(at) {
            *slot = state.as_str();
        }
    }
    out
}

/// The watch table, for [`Arena`]'s own field.
pub(crate) type SquadWatchTable = BTreeMap<SquadId, SquadWatch>;

/// The set of squads whose order expired since the last signal.
pub(crate) type ExpiredSquads = BTreeSet<SquadId>;
