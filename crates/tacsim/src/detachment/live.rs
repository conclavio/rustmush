//! Opt-in OOC command hosting. IC personnel, grants and durable missions belong
//! to DT.2/DT.3; practice permission is confined to a checked-out six-unit group.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_core::{Ctx, ModError, ObjId, Registrar, TickCadence, World};
use mush_tactics::detachment::orders::{
    DeliveryOutcome, Directive, IssuedOrder, OrderBook, OrderEnvelope, OrderError, OrderRequest,
    OrderTarget, Response,
};
use mush_tactics::detachment::roster::{
    Connection, Controller, DetachmentId, DetachmentRoster, LeadershipPolicy, Member, MemberFacts,
    RosterError, Slot,
};
use serde_json::{Value, json};

use super::PracticeObjective;
use crate::arena::state::PartyId;
use crate::arena_bridge::ArenaRegistry;
use crate::arena_task::MatchId;
use crate::deliberation::SquadId;
use crate::deliberation::order::Objective;
use crate::match_::{MatchBook, MatchRecord, MatchState};

const GROUP_SIZE: usize = 6;
const NOTICE: &str = "sortie";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    match_id: MatchId,
    team: PartyId,
    index: u16,
}

impl GroupKey {
    fn id(self) -> Result<DetachmentId, LiveError> {
        let side = if self.team == PartyId::BLUE {
            65_536
        } else {
            0
        };
        self.match_id
            .0
            .checked_mul(131_072)
            .and_then(|base| base.checked_add(side + u64::from(self.index)))
            .map(DetachmentId)
            .ok_or(LiveError::Capacity)
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    key: GroupKey,
    members: Vec<Member>,
    callsigns: BTreeMap<ObjId, String>,
    squads: BTreeMap<ObjId, SquadId>,
    facts: Vec<MemberFacts>,
    live_players: BTreeSet<ObjId>,
    window: (u16, u16),
}

#[derive(Clone, Debug)]
struct Group {
    key: GroupKey,
    roster: DetachmentRoster,
    orders: OrderBook<PracticeObjective>,
    callsigns: BTreeMap<ObjId, String>,
    squads: BTreeMap<ObjId, SquadId>,
    live_players: BTreeSet<ObjId>,
    window: (u16, u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Authority {
    detachment: DetachmentId,
    revision: u64,
    term: u64,
    epoch: u64,
    confirmed: bool,
}

/// RAM only, registered with the world's normal module-state owner. A fresh
/// process requires explicit practice opt-in and mints a new admission revision.
#[derive(Clone, Debug)]
pub struct PracticeHost {
    origin: Instant,
    now_ms: u64,
    // Minted at composition: an OOC admission nonce, not gameplay randomness.
    next_roster_revision: u64,
    /// A failed world refresh must not leave old authority open for admission.
    refresh_valid: bool,
    next_authority_epoch: u64,
    groups: BTreeMap<GroupKey, Group>,
    authority: BTreeMap<(MatchId, ObjId), Authority>,
}

impl Default for PracticeHost {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            now_ms: 0,
            next_roster_revision: (rand::random::<u64>() >> 1).max(1),
            refresh_valid: false,
            next_authority_epoch: 1,
            groups: BTreeMap::new(),
            authority: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("Practice detachments are unavailable.")]
    Unavailable,
    #[error("Take a live station in the running practice match first.")]
    NotSeated,
    #[error("Enable your practice group with +sortie/practice first.")]
    NotEnabled,
    #[error("That detachment selection is stale; read +sortie/roster again.")]
    Stale,
    #[error("The practice detachment cannot admit more identifiers.")]
    Capacity,
    #[error(transparent)]
    Roster(#[from] RosterError),
    #[error(transparent)]
    Order(#[from] OrderError),
    #[error(transparent)]
    Objective(#[from] super::ObjectiveError),
}

/// The client/typed command must return the exact projected fences.
#[derive(Clone, Debug)]
pub struct LiveOrderRequest {
    pub detachment: u64,
    pub revision: u64,
    pub term: u64,
    pub target: OrderTarget,
    pub directive: Directive<PracticeObjective>,
    pub expires_after_ms: Option<u64>,
}

/// An admitted roster and its real practice window for the game's objective parser.
#[derive(Clone, Debug)]
pub struct CommandContext {
    pub match_id: MatchId,
    pub roster: DetachmentRoster,
    pub window: (u16, u16),
}

/// Offers remain queued until their corresponding confirmation. Authority offers
/// precede controller messages; a message is exposed only after its authority
/// offer was accepted. Neither queue admission nor delivery claims execution.
#[derive(Clone, Debug)]
pub enum LiveDispatch {
    Authority {
        match_id: MatchId,
        unit: ObjId,
        epoch: u64,
        detachment: DetachmentId,
        revision: u64,
        term: u64,
    },
    ControllerOrder {
        match_id: MatchId,
        authority_epoch: u64,
        squad: SquadId,
        /// Remaining host duration; the bridge anchors it to the arena clock.
        expires_after_ms: Option<u64>,
        order: OrderEnvelope<PracticeObjective>,
    },
}

pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(PracticeHost::default());
    reg.tick(
        "detachment-command",
        TickCadence::Coarse(Duration::from_secs(1)),
        Arc::new(
            |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                sync(ctx);
                crate::arena_bridge::flush_detachment_dispatches(ctx);
            },
        ),
    )?;
    Ok(())
}

fn host_now(w: &World) -> Option<u64> {
    let host = w.module_state::<PracticeHost>()?;
    Some(
        u64::try_from(host.origin.elapsed().as_millis())
            .unwrap_or(u64::MAX)
            .max(host.now_ms),
    )
}

fn member_rows(record: &MatchRecord) -> Result<Vec<(GroupKey, Vec<Member>)>, LiveError> {
    let mut groups = Vec::new();
    for team in &record.teams {
        groups.extend(team_member_rows(record.id, team)?);
    }
    Ok(groups)
}

fn team_member_rows(
    match_id: MatchId,
    team: &crate::match_::TeamRoster,
) -> Result<Vec<(GroupKey, Vec<Member>)>, LiveError> {
    let mut groups = Vec::new();
    // Doc 32 cut CF.0: the roster is in pod-slot order already
    // (`match_::slot_order`); a drawn callsign says nothing about a slot, so
    // the members are numbered in the roster's own order.
    let units = team.units.clone();
    for (index, chunk) in units.chunks(GROUP_SIZE).enumerate() {
        let key = GroupKey {
            match_id,
            team: team.team,
            index: u16::try_from(index + 1).map_err(|_| LiveError::Capacity)?,
        };
        let mut members = Vec::with_capacity(chunk.len());
        for (slot, unit) in chunk.iter().enumerate() {
            let controller = team.pilots.iter().find(|(_, held)| held == unit).map_or(
                Controller::Automated,
                |(character, _)| Controller::Player {
                    character: *character,
                },
            );
            members.push(Member {
                slot: Slot::new(u16::try_from(slot + 1).map_err(|_| LiveError::Capacity)?)?,
                unit: *unit,
                controller,
            });
        }
        groups.push((key, members));
    }
    Ok(groups)
}

fn snapshots(w: &World) -> Result<Vec<Snapshot>, LiveError> {
    let book = w
        .module_state::<MatchBook>()
        .ok_or(LiveError::Unavailable)?;
    let registry = w
        .module_state::<ArenaRegistry>()
        .ok_or(LiveError::Unavailable)?;
    let mut snapshots = Vec::new();
    for (_, record) in book.iter().filter(|(_, record)| record.state.is_live()) {
        let Some(arena) = registry.arena_of(record.id) else {
            continue;
        };
        for (key, members) in member_rows(record)? {
            let mut live_players = BTreeSet::new();
            let mut facts = Vec::with_capacity(members.len());
            for member in &members {
                let fact = match member.controller {
                    Controller::Player { character } => {
                        let seated =
                            registry.seat_of_pilot(character) == Some((record.id, member.unit));
                        let frame = registry.last_frame(character).filter(|held| {
                            held.match_id == record.id && held.frame.unit == member.unit
                        });
                        let unit_disabled = frame.is_none_or(|held| {
                            held.frame.wreck
                                || (held.frame.immobile && held.frame.weapons.is_empty())
                        });
                        let connection = if w.has_live_conns(character) {
                            if seated {
                                live_players.insert(character);
                            }
                            Connection::Connected
                        } else if w.is_connected(character) {
                            // The session manager owns the configured grace. Its
                            // Disconnected hook/world flag is authoritative; the
                            // world API exposes no deadline to copy or guess.
                            Connection::Linkdead {
                                grace_until_ms: u64::MAX,
                            }
                        } else {
                            Connection::Disconnected
                        };
                        MemberFacts {
                            slot: member.slot,
                            capable: w.get(character).is_some(),
                            controls_unit: seated,
                            unit_disabled,
                            connection,
                            command_authorized: record.state == MatchState::Running,
                        }
                    }
                    _ => MemberFacts {
                        slot: member.slot,
                        capable: w.get(member.unit).is_some(),
                        controls_unit: true,
                        unit_disabled: false,
                        connection: Connection::Connected,
                        command_authorized: record.state == MatchState::Running,
                    },
                };
                facts.push(fact);
            }
            snapshots.push(Snapshot {
                key,
                callsigns: members
                    .iter()
                    .map(|member| (member.unit, record.callsign_word(member.unit).to_string()))
                    .collect(),
                squads: members
                    .iter()
                    .map(|member| {
                        let squad = record
                            .squads
                            .iter()
                            .find(|squad| squad.holds(member.unit))
                            .map_or(
                                SquadId {
                                    team: key.team,
                                    index: key.index,
                                },
                                |squad| squad.id,
                            );
                        (member.unit, squad)
                    })
                    .collect(),
                members,
                facts,
                live_players,
                window: (arena.terrain.width(), arena.terrain.height()),
            });
        }
    }
    Ok(snapshots)
}

impl PracticeHost {
    fn activate(&mut self, snapshot: &Snapshot, revision: u64) -> Result<bool, LiveError> {
        if self.groups.contains_key(&snapshot.key) {
            return Ok(false);
        }
        let id = snapshot.key.id()?;
        let roster = DetachmentRoster::new(
            id,
            revision,
            snapshot.members.clone(),
            None,
            LeadershipPolicy::default(),
        )?;
        self.groups.insert(
            snapshot.key,
            Group {
                key: snapshot.key,
                orders: OrderBook::new(id),
                roster,
                callsigns: snapshot.callsigns.clone(),
                squads: snapshot.squads.clone(),
                live_players: BTreeSet::new(),
                window: snapshot.window,
            },
        );
        self.refresh_valid = false;
        Ok(true)
    }

    fn reconcile(
        &mut self,
        snapshots: &[Snapshot],
        now_ms: u64,
    ) -> Result<Vec<(ObjId, Option<ObjId>)>, LiveError> {
        self.refresh_valid = false;
        let mut prepared = self.clone();
        let notices = prepared.reconcile_inner(snapshots, now_ms)?;
        prepared.refresh_valid = true;
        *self = prepared;
        Ok(notices)
    }

    fn reconcile_inner(
        &mut self,
        snapshots: &[Snapshot],
        now_ms: u64,
    ) -> Result<Vec<(ObjId, Option<ObjId>)>, LiveError> {
        if now_ms < self.now_ms {
            return Err(RosterError::BackwardTime.into());
        }
        let keys: BTreeSet<_> = snapshots.iter().map(|snapshot| snapshot.key).collect();
        self.groups.retain(|key, _| keys.contains(key));
        let mut notices = Vec::new();
        for snapshot in snapshots {
            let Some(group) = self.groups.get_mut(&snapshot.key) else {
                continue;
            };
            // Live membership is frozen. A host mismatch is a failed admission,
            // never an implicit assignment edit that reuses consent or orders.
            if group.roster.members() != snapshot.members {
                return Err(LiveError::Stale);
            }
            let change = group.roster.reconcile(now_ms, &snapshot.facts)?;
            group.live_players.clone_from(&snapshot.live_players);
            group.callsigns.clone_from(&snapshot.callsigns);
            group.squads.clone_from(&snapshot.squads);
            if let Some(change) = change {
                notices.extend(
                    group
                        .live_players
                        .iter()
                        .map(|actor| (*actor, change.current.map(|leader| leader.character))),
                );
            }
            for member in group
                .roster
                .members()
                .iter()
                .filter(|member| member.controller == Controller::Automated)
            {
                let key = (group.key.match_id, member.unit);
                let desired = (
                    group.roster.id(),
                    group.roster.revision(),
                    group.roster.term(),
                );
                if self
                    .authority
                    .get(&key)
                    .is_some_and(|old| (old.detachment, old.revision, old.term) == desired)
                {
                    continue;
                }
                let epoch = self.next_authority_epoch;
                self.next_authority_epoch = epoch.checked_add(1).ok_or(LiveError::Capacity)?;
                self.authority.insert(
                    key,
                    Authority {
                        detachment: desired.0,
                        revision: desired.1,
                        term: desired.2,
                        epoch,
                        confirmed: false,
                    },
                );
            }
        }
        let active: BTreeSet<_> = self
            .groups
            .values()
            .map(|group| group.roster.id())
            .collect();
        self.authority
            .retain(|_, authority| active.contains(&authority.detachment));
        self.now_ms = now_ms;
        Ok(notices)
    }

    fn group_of(&self, actor: ObjId) -> Option<&Group> {
        if !self.refresh_valid {
            return None;
        }
        self.groups.values().find(|group| {
            group
                .roster
                .members()
                .iter()
                .any(|member| member.controller == Controller::Player { character: actor })
        })
    }

    fn group_by_id_mut(&mut self, id: DetachmentId) -> Option<&mut Group> {
        self.groups
            .values_mut()
            .find(|group| group.roster.id() == id)
    }
}

/// Explicit opt-in. A matching later invocation leaves the roster/history intact.
pub fn enable(ctx: &mut Ctx<'_>) -> Result<(), LiveError> {
    let registry = ctx
        .w
        .module_state::<ArenaRegistry>()
        .ok_or(LiveError::Unavailable)?;
    let (match_id, unit) = registry
        .seat_of_pilot(ctx.actor)
        .ok_or(LiveError::NotSeated)?;
    if !ctx.w.has_live_conns(ctx.actor) {
        return Err(LiveError::NotSeated);
    }
    if ctx
        .w
        .module_state::<MatchBook>()
        .and_then(|book| book.get(match_id))
        .is_none_or(|record| record.state != MatchState::Running)
    {
        return Err(LiveError::NotSeated);
    }
    let candidates = snapshots(ctx.w)?;
    let snapshot = candidates
        .iter()
        .find(|snapshot| {
            snapshot.key.match_id == match_id
                && snapshot.members.iter().any(|member| {
                    member.unit == unit
                        && member.controller
                            == Controller::Player {
                                character: ctx.actor,
                            }
                })
        })
        .ok_or(LiveError::NotSeated)?;
    let host = ctx
        .w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?;
    let revision = host.next_roster_revision;
    let next_revision = revision.checked_add(1).ok_or(LiveError::Capacity)?;
    let created = host.activate(snapshot, revision)?;
    if created {
        host.next_roster_revision = next_revision;
    }
    sync(ctx);
    if created {
        ctx.out.sys_to_actor(ctx.actor, NOTICE,
            "Practice detachment enabled. Its command history starts fresh; read +sortie/roster for the current slots and order controls.");
    }
    Ok(())
}

/// Reconcile the opt-in groups against current world/arena observations.
pub fn sync(ctx: &mut Ctx<'_>) {
    if ctx
        .w
        .module_state::<PracticeHost>()
        .is_none_or(|host| host.groups.is_empty())
    {
        return;
    }
    let Some(now_ms) = host_now(ctx.w) else {
        return;
    };
    if let Err(error) = sync_at(ctx, now_ms) {
        tracing::warn!(%error, "practice detachment synchronization refused");
    }
}

/// The deterministic seam used by host tests. Production uses the monotonic
/// elapsed clock above, independently of campaign acceleration and arena tempo.
pub fn sync_at(ctx: &mut Ctx<'_>, now_ms: u64) -> Result<(), LiveError> {
    // Invalidate before collecting facts too: a failed source read must close
    // contexts and dispatches just as a refused roster reconciliation does.
    ctx.w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?
        .refresh_valid = false;
    let snapshots = snapshots(ctx.w)?;
    let notices = ctx
        .w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?
        .reconcile(&snapshots, now_ms)?;
    for (actor, leader) in notices {
        let line = leader.map_or_else(
            || "No active detachment leader; standing orders continue.".to_owned(),
            |leader| {
                format!(
                    "Detachment command: {}.",
                    mush_core::output::display_name(ctx.w, leader)
                )
            },
        );
        ctx.out.sys_to_actor(actor, NOTICE, line);
    }
    deliver_players(ctx);
    Ok(())
}

#[must_use]
pub fn command_context(w: &World, actor: ObjId) -> Option<CommandContext> {
    let host = w.module_state::<PracticeHost>()?;
    let group = host.group_of(actor)?;
    let member = group
        .roster
        .members()
        .iter()
        .find(|member| member.controller == Controller::Player { character: actor })?;
    if !w.has_live_conns(actor)
        || w.module_state::<ArenaRegistry>()?.seat_of_pilot(actor)
            != Some((group.key.match_id, member.unit))
        || w.module_state::<MatchBook>()?
            .get(group.key.match_id)?
            .state
            != MatchState::Running
    {
        return None;
    }
    Some(CommandContext {
        match_id: group.key.match_id,
        roster: group.roster.clone(),
        window: group.window,
    })
}

pub fn issue(ctx: &mut Ctx<'_>, request: LiveOrderRequest) -> Result<IssuedOrder, LiveError> {
    sync(ctx);
    let context = command_context(ctx.w, ctx.actor).ok_or(LiveError::NotEnabled)?;
    let detachment = DetachmentId(request.detachment);
    if context.roster.id() != detachment {
        return Err(LiveError::Stale);
    }
    if let Directive::Objective(payload) = &request.directive {
        PracticeObjective::new(
            &context.roster,
            payload.objective(),
            payload.constraints(),
            context.window,
        )?;
    }
    let host = ctx
        .w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?;
    let now_ms = host.now_ms;
    let expires_at_ms = request
        .expires_after_ms
        .map(|duration| now_ms.checked_add(duration).ok_or(LiveError::Capacity))
        .transpose()?;
    let group = host.group_by_id_mut(detachment).ok_or(LiveError::Stale)?;
    let issued = group.orders.issue(
        &group.roster,
        now_ms,
        OrderRequest {
            issuer: ctx.actor,
            roster_revision: request.revision,
            command_term: request.term,
            target: request.target,
            directive: request.directive,
            expires_at_ms,
        },
        |_| true,
    )?;
    deliver_players(ctx);
    Ok(issued)
}

pub fn respond(
    ctx: &mut Ctx<'_>,
    detachment: u64,
    revision: u64,
    sequence: u64,
    response: Response,
) -> Result<(), LiveError> {
    sync(ctx);
    let context = command_context(ctx.w, ctx.actor).ok_or(LiveError::NotEnabled)?;
    let detachment = DetachmentId(detachment);
    if context.roster.id() != detachment || context.roster.revision() != revision {
        return Err(LiveError::Stale);
    }
    let slot = context
        .roster
        .members()
        .iter()
        .find(|member| {
            member.controller
                == Controller::Player {
                    character: ctx.actor,
                }
        })
        .map(|member| member.slot)
        .ok_or(LiveError::NotSeated)?;
    let host = ctx
        .w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?;
    let now_ms = host.now_ms;
    host.group_by_id_mut(detachment)
        .ok_or(LiveError::Stale)?
        .orders
        .respond(slot, sequence, ctx.actor, response, now_ms)?;
    Ok(())
}

fn deliver_players(ctx: &mut Ctx<'_>) {
    let Some(host) = ctx.w.module_state_mut::<PracticeHost>() else {
        return;
    };
    if !host.refresh_valid {
        return;
    }
    let mut notices = Vec::new();
    for group in host.groups.values_mut() {
        for member in group.roster.members() {
            let Controller::Player { character } = member.controller else {
                continue;
            };
            if !group.live_players.contains(&character) {
                continue;
            }
            let Some(pending) = group.orders.pending(member.slot).cloned() else {
                continue;
            };
            if group.orders.deliver(
                &group.roster,
                member.slot,
                pending.sequence,
                host.now_ms,
                |_| true,
            ) == Ok(DeliveryOutcome::Delivered)
            {
                notices.push((character, pending, group.callsigns.clone()));
            }
        }
    }
    for (recipient, order, callsigns) in notices {
        let label = directive_words(&order.directive, &callsigns);
        let line = format!(
            "Order from {}: {label}.",
            mush_core::output::display_name(ctx.w, order.issuer)
        );
        ctx.out.sys_to_actor(recipient, NOTICE, line);
    }
}

/// Repeated calls expose only unconfirmed authority and still-pending messages.
/// After confirming controls, the bridge may call again in the same world action
/// to offer newly unblocked commands without waiting for the periodic tick.
#[must_use]
pub fn drain_dispatches(w: &World) -> Vec<LiveDispatch> {
    let Some(host) = w.module_state::<PracticeHost>() else {
        return Vec::new();
    };
    dispatches(host)
}

fn dispatches(host: &PracticeHost) -> Vec<LiveDispatch> {
    if !host.refresh_valid {
        return Vec::new();
    }
    let mut events = Vec::new();
    for ((match_id, unit), authority) in &host.authority {
        if !authority.confirmed {
            events.push(LiveDispatch::Authority {
                match_id: *match_id,
                unit: *unit,
                epoch: authority.epoch,
                detachment: authority.detachment,
                revision: authority.revision,
                term: authority.term,
            });
        }
    }
    for group in host.groups.values() {
        for member in group
            .roster
            .members()
            .iter()
            .filter(|member| member.controller == Controller::Automated)
        {
            let Some(authority) = host
                .authority
                .get(&(group.key.match_id, member.unit))
                .filter(|authority| authority.confirmed)
            else {
                continue;
            };
            let Some(order) = group.orders.pending(member.slot) else {
                continue;
            };
            if group
                .roster
                .validate_command_at(
                    order.issuer,
                    order.roster_revision,
                    order.command_term,
                    host.now_ms,
                )
                .is_err()
                || order.expires_at_ms.is_some_and(|end| host.now_ms >= end)
            {
                continue;
            }
            let Some(squad) = group.squads.get(&member.unit) else {
                continue;
            };
            events.push(LiveDispatch::ControllerOrder {
                match_id: group.key.match_id,
                authority_epoch: authority.epoch,
                squad: *squad,
                expires_after_ms: order
                    .expires_at_ms
                    .map(|deadline| deadline.saturating_sub(host.now_ms)),
                order: order.clone(),
            });
        }
    }
    events
}

pub fn confirm_authority(w: &mut World, match_id: MatchId, unit: ObjId, epoch: u64) {
    if let Some(authority) = w
        .module_state_mut::<PracticeHost>()
        .and_then(|host| host.authority.get_mut(&(match_id, unit)))
        && authority.epoch == epoch
    {
        authority.confirmed = true;
    }
}

/// A practice-owned automated recipient may no longer accept legacy squad
/// orders. Players remain advisory recipients regardless of their hull's current
/// machine fallback, so their physical station path is never claimed here.
#[must_use]
pub fn owns_unit(w: &World, match_id: MatchId, unit: ObjId) -> bool {
    w.module_state::<PracticeHost>()
        .is_some_and(|host| host.authority.contains_key(&(match_id, unit)))
}

pub fn confirm_delivery(
    ctx: &mut Ctx<'_>,
    detachment: DetachmentId,
    slot: Slot,
    sequence: u64,
) -> Result<DeliveryOutcome, LiveError> {
    let host = ctx
        .w
        .module_state_mut::<PracticeHost>()
        .ok_or(LiveError::Unavailable)?;
    if !host.refresh_valid {
        return Err(LiveError::Stale);
    }
    let now_ms = host.now_ms;
    let group = host.group_by_id_mut(detachment).ok_or(LiveError::Stale)?;
    if group
        .roster
        .member(slot)
        .is_none_or(|member| member.controller != Controller::Automated)
    {
        return Err(OrderError::WrongRecipient.into());
    }
    Ok(group
        .orders
        .deliver(&group.roster, slot, sequence, now_ms, |_| true)?)
}

fn directive_words(
    directive: &Directive<PracticeObjective>,
    callsigns: &BTreeMap<ObjId, String>,
) -> String {
    match directive {
        Directive::Cancel => "CANCEL".to_owned(),
        Directive::Objective(payload) => match payload.objective() {
            Objective::Hold => "HOLD".to_owned(),
            Objective::Advance { to } => format!("ADVANCE {},{}", to.0, to.1),
            Objective::Engage { .. } => "ENGAGE".to_owned(),
            Objective::Withdraw => "WITHDRAW".to_owned(),
            Objective::Screen { facing_mdeg } => format!("SCREEN {} degrees", facing_mdeg / 1_000),
            Objective::Support { of } => format!(
                "SUPPORT {}",
                callsigns.get(&of).map_or("member", String::as_str)
            ),
        },
    }
}

fn plain_label(value: &str, max: usize) -> String {
    let clean: String = value
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}'))
        .take(max)
        .collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "unknown".to_owned()
    } else {
        clean.to_owned()
    }
}

/// A member-only UI projection, with strings preserving all 64-bit fences in JS.
#[must_use]
pub fn projection(w: &World, actor: ObjId) -> Option<Value> {
    let context = command_context(w, actor)?;
    let host = w.module_state::<PracticeHost>()?;
    let group = host.group_of(actor)?;
    let prefix = format!(
        "DT:{}:{}:{}",
        group.roster.id().0,
        group.roster.revision(),
        group.roster.term()
    );
    let slots: Vec<_> = group.roster.members().iter().map(|member| {
        let (kind, pilot) = match member.controller {
            Controller::Player { character } => ("player", Some(plain_label(&mush_core::output::display_name(w, character), 96))),
            Controller::Npc { character } => ("npc", Some(plain_label(&mush_core::output::display_name(w, character), 96))),
            Controller::Automated => ("automated", None),
        };
        json!({"slot":member.slot.get(), "callsign":plain_label(group.callsigns.get(&member.unit).map_or("unknown", String::as_str), 32), "pilot":pilot,
            "controller":kind, "order_prefill":format!("ORDER {prefix} {} ",member.slot.get())})
    }).collect();
    let own_slot = context
        .roster
        .members()
        .iter()
        .find(|member| member.controller == Controller::Player { character: actor })?
        .slot;
    let own = group.orders.delivered(own_slot).filter(|_| {
        group
            .orders
            .standing(&group.roster, own_slot, host.now_ms)
            .is_some()
    });
    let my_order = own.map(|delivered| json!({
        "sequence":delivered.order.sequence.to_string(),
        "from":plain_label(&mush_core::output::display_name(w, delivered.order.issuer), 96),
        "objective":plain_label(&directive_words(&delivered.order.directive, &group.callsigns), 256),
        "response":delivered.acknowledgement.map(|ack| match ack.response { Response::Wilco=>"wilco", Response::Negative=>"negative" }),
    }));
    let response_suffix = own.map(|delivered| {
        format!(
            "DT:{}:{} {}",
            group.roster.id().0,
            group.roster.revision(),
            delivered.order.sequence
        )
    });
    Some(json!({
        "id":group.roster.id().0.to_string(), "revision":group.roster.revision().to_string(), "term":group.roster.term().to_string(),
        "leader_slot":group.roster.leader().map(|leader| leader.slot.get()), "slots":slots,
        "all_order_prefill":format!("ORDER {prefix} ALL "), "my_order":my_order,
        "wilco_command":response_suffix.as_ref().map(|suffix| format!("WILCO {suffix}")),
        "negative_command":response_suffix.map(|suffix| format!("NEGATIVE {suffix}")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deliberation::Constraints;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn projected_labels_are_nonempty_bounded_and_plain() -> TestResult {
        check_eq!(
            plain_label("\u{1b}[31mAster\n\u{2028}\u{2029}", 96),
            "[31mAster"
        );
        check_eq!(plain_label(" \n\t\u{2028}", 96), "unknown");
        let name = "🛠".repeat(120);
        let label = plain_label(&name, 96);
        check_eq!(label.chars().count(), 96);
        check_eq!(label, "🛠".repeat(96));
        Ok(())
    }

    fn snapshot() -> Result<Snapshot, LiveError> {
        let key = GroupKey {
            match_id: MatchId(8),
            team: PartyId::RED,
            index: 1,
        };
        let members = vec![
            Member {
                slot: Slot::new(1)?,
                unit: ObjId(101),
                controller: Controller::Player {
                    character: ObjId(10),
                },
            },
            Member {
                slot: Slot::new(2)?,
                unit: ObjId(102),
                controller: Controller::Automated,
            },
            Member {
                slot: Slot::new(3)?,
                unit: ObjId(103),
                controller: Controller::Player {
                    character: ObjId(30),
                },
            },
        ];
        Ok(Snapshot {
            key,
            callsigns: members
                .iter()
                .map(|member| (member.unit, format!("R{}", member.slot.get())))
                .collect(),
            squads: members
                .iter()
                .map(|member| {
                    (
                        member.unit,
                        SquadId {
                            team: PartyId::RED,
                            index: 1,
                        },
                    )
                })
                .collect(),
            facts: members
                .iter()
                .map(|member| MemberFacts {
                    slot: member.slot,
                    capable: true,
                    controls_unit: true,
                    unit_disabled: false,
                    connection: Connection::Connected,
                    command_authorized: true,
                })
                .collect(),
            members,
            live_players: [ObjId(10), ObjId(30)].into_iter().collect(),
            window: (256, 256),
        })
    }

    fn issue_hold(host: &mut PracticeHost, key: GroupKey) -> Result<IssuedOrder, LiveError> {
        let now_ms = host.now_ms;
        let group = host.groups.get_mut(&key).ok_or(LiveError::NotEnabled)?;
        let objective = PracticeObjective::new(
            &group.roster,
            Objective::Hold,
            Constraints::default(),
            group.window,
        )
        .map_err(|_| LiveError::Unavailable)?;
        Ok(group.orders.issue(
            &group.roster,
            now_ms,
            OrderRequest {
                issuer: ObjId(10),
                roster_revision: group.roster.revision(),
                command_term: group.roster.term(),
                target: OrderTarget::Slot(Slot::new(2)?),
                directive: Directive::Objective(objective),
                expires_at_ms: None,
            },
            |_| true,
        )?)
    }

    #[test]
    fn practice_groups_preserve_every_unit_and_original_player_assignment() -> TestResult {
        // Doc 32 cut CF.0: the roster arrives in pod-slot order
        // (`match_::slot_order`) and the members keep it.
        let units: Vec<_> = (1..=13).map(ObjId).collect();
        let team = crate::match_::TeamRoster {
            team: PartyId::RED,
            units: units.clone(),
            pilots: vec![(ObjId(70), ObjId(7))],
        };
        let groups = team_member_rows(MatchId(8), &team)?;
        check_eq!(
            groups
                .iter()
                .map(|(_, rows)| rows.len())
                .collect::<Vec<_>>(),
            vec![6, 6, 1]
        );
        let flattened: Vec<_> = groups
            .iter()
            .flat_map(|(_, rows)| rows.iter().map(|member| member.unit))
            .collect();
        check_eq!(flattened, (1..=13).map(ObjId).collect::<Vec<_>>());
        let player = groups
            .iter()
            .flat_map(|(_, members)| members)
            .find(|member| member.unit == ObjId(7))
            .ok_or(LiveError::NotSeated)?;
        check_eq!(player.slot.get(), 1);
        check_eq!(
            player.controller,
            Controller::Player {
                character: ObjId(70)
            }
        );
        check!(
            groups
                .iter()
                .flat_map(|(_, members)| members)
                .all(|member| !matches!(member.controller, Controller::Npc { .. }))
        );
        Ok(())
    }

    #[test]
    fn opt_in_is_required_and_repeated_enable_preserves_history_revision() -> TestResult {
        let snapshot = snapshot()?;
        let mut host = PracticeHost::default();
        host.reconcile(std::slice::from_ref(&snapshot), 0)?;
        check!(host.groups.is_empty());
        check!(dispatches(&host).is_empty());
        check!(host.activate(&snapshot, 11)?);
        host.reconcile(std::slice::from_ref(&snapshot), 1)?;
        check!(!host.activate(&snapshot, 99)?);
        let group = host
            .groups
            .get(&snapshot.key)
            .ok_or(LiveError::NotEnabled)?;
        check_eq!(group.roster.revision(), 11);
        check_eq!(
            group.roster.leader().map(|leader| leader.character),
            Some(ObjId(10))
        );
        check_eq!(host.authority.len(), 1);
        check!(!host.authority.contains_key(&(MatchId(8), ObjId(101))));
        Ok(())
    }

    #[test]
    fn rejected_authority_and_controller_offers_remain_pending_without_delivery() -> TestResult {
        let snapshot = snapshot()?;
        let mut host = PracticeHost::default();
        host.activate(&snapshot, 11)?;
        host.reconcile(std::slice::from_ref(&snapshot), 0)?;
        let issued = issue_hold(&mut host, snapshot.key)?;
        let offers = dispatches(&host);
        check_eq!(offers.len(), 1);
        check!(matches!(
            offers.first(),
            Some(LiveDispatch::Authority { .. })
        ));
        check_eq!(dispatches(&host).len(), 1);
        let group = host
            .groups
            .get(&snapshot.key)
            .ok_or(LiveError::NotEnabled)?;
        check!(group.orders.delivered(Slot::new(2)?).is_none());
        let authority = host
            .authority
            .get_mut(&(MatchId(8), ObjId(102)))
            .ok_or(LiveError::Unavailable)?;
        authority.confirmed = true;
        let offers = dispatches(&host);
        check_eq!(offers.len(), 1);
        check!(
            matches!(offers.first(), Some(LiveDispatch::ControllerOrder { order, .. }) if order.sequence == issued.sequence)
        );
        check_eq!(dispatches(&host).len(), 1);
        let group = host
            .groups
            .get_mut(&snapshot.key)
            .ok_or(LiveError::NotEnabled)?;
        check!(group.orders.delivered(Slot::new(2)?).is_none());
        group
            .orders
            .deliver(&group.roster, Slot::new(2)?, issued.sequence, 0, |_| true)?;
        check!(dispatches(&host).is_empty());
        Ok(())
    }

    #[test]
    fn refused_refresh_closes_admission_and_dispatch_until_reconciled() -> TestResult {
        let snapshot = snapshot()?;
        let mut host = PracticeHost::default();
        host.activate(&snapshot, 11)?;
        host.reconcile(std::slice::from_ref(&snapshot), 0)?;
        issue_hold(&mut host, snapshot.key)?;
        if let Some(authority) = host.authority.get_mut(&(MatchId(8), ObjId(102))) {
            authority.confirmed = true;
        }
        check!(host.group_of(ObjId(10)).is_some());
        check!(matches!(
            dispatches(&host).first(),
            Some(LiveDispatch::ControllerOrder { .. })
        ));
        let mut changed = snapshot.clone();
        if let Some(member) = changed.members.first_mut() {
            member.unit = ObjId(999);
        }
        check!(matches!(
            host.reconcile(&[changed], 10),
            Err(LiveError::Stale)
        ));
        check!(!host.refresh_valid);
        check!(
            host.group_of(ObjId(10)).is_none(),
            "command contexts and projections must close"
        );
        check!(
            dispatches(&host).is_empty(),
            "a frozen host clock must not deliver pending orders"
        );
        check_eq!(
            host.now_ms,
            0,
            "the refused snapshot did not partially commit"
        );
        host.reconcile(std::slice::from_ref(&snapshot), 11)?;
        check!(host.refresh_valid);
        check!(host.group_of(ObjId(10)).is_some());
        check!(matches!(
            dispatches(&host).first(),
            Some(LiveDispatch::ControllerOrder { .. })
        ));
        Ok(())
    }

    #[test]
    fn session_expiry_and_return_reconcile_without_converting_player_to_machine() -> TestResult {
        let mut snapshot = snapshot()?;
        let mut host = PracticeHost::default();
        host.activate(&snapshot, 11)?;
        host.reconcile(std::slice::from_ref(&snapshot), 0)?;
        if let Some(fact) = snapshot.facts.first_mut() {
            fact.connection = Connection::Linkdead {
                grace_until_ms: u64::MAX,
            };
        }
        snapshot.live_players.remove(&ObjId(10));
        host.reconcile(std::slice::from_ref(&snapshot), 100)?;
        check_eq!(
            host.groups
                .get(&snapshot.key)
                .map(|group| group.roster.term()),
            Some(1)
        );
        if let Some(fact) = snapshot.facts.first_mut() {
            fact.connection = Connection::Disconnected;
            fact.controls_unit = false;
        }
        host.reconcile(std::slice::from_ref(&snapshot), 300_000)?;
        let group = host
            .groups
            .get(&snapshot.key)
            .ok_or(LiveError::NotEnabled)?;
        check_eq!(
            group.roster.leader().map(|leader| leader.character),
            Some(ObjId(30))
        );
        check_eq!(
            group
                .roster
                .member(Slot::new(1)?)
                .map(|member| member.controller),
            Some(Controller::Player {
                character: ObjId(10)
            })
        );
        if let Some(fact) = snapshot.facts.first_mut() {
            fact.connection = Connection::Connected;
            fact.controls_unit = true;
        }
        snapshot.live_players.insert(ObjId(10));
        host.reconcile(std::slice::from_ref(&snapshot), 301_000)?;
        check_eq!(
            host.groups
                .get(&snapshot.key)
                .map(|group| group.roster.term()),
            Some(2)
        );
        host.reconcile(std::slice::from_ref(&snapshot), 304_000)?;
        check_eq!(
            host.groups
                .get(&snapshot.key)
                .map(|group| group.roster.term()),
            Some(3)
        );
        check_eq!(host.authority.len(), 1);
        Ok(())
    }

    #[test]
    fn succession_replaces_authority_before_old_pending_orders_can_escape() -> TestResult {
        let mut snapshot = snapshot()?;
        let mut host = PracticeHost::default();
        host.activate(&snapshot, 11)?;
        host.reconcile(std::slice::from_ref(&snapshot), 0)?;
        issue_hold(&mut host, snapshot.key)?;
        if let Some(authority) = host.authority.get_mut(&(MatchId(8), ObjId(102))) {
            authority.confirmed = true;
        }
        check!(matches!(
            dispatches(&host).first(),
            Some(LiveDispatch::ControllerOrder { .. })
        ));
        if let Some(fact) = snapshot.facts.first_mut() {
            fact.unit_disabled = true;
        }
        host.reconcile(std::slice::from_ref(&snapshot), 1)?;
        check!(matches!(
            dispatches(&host).first(),
            Some(LiveDispatch::Authority { term: 2, .. })
        ));
        if let Some(authority) = host.authority.get_mut(&(MatchId(8), ObjId(102))) {
            authority.confirmed = true;
        }
        check!(dispatches(&host).is_empty());
        Ok(())
    }

    #[test]
    fn terminal_matches_prune_host_state_and_new_host_revision_fences_old_requests() -> TestResult {
        let snapshot = snapshot()?;
        let mut old = PracticeHost::default();
        old.activate(&snapshot, 11)?;
        old.reconcile(std::slice::from_ref(&snapshot), 0)?;
        old.reconcile(&[], 1)?;
        check!(old.groups.is_empty());
        check!(old.authority.is_empty());
        let mut resumed = PracticeHost::default();
        resumed.activate(&snapshot, 12)?;
        resumed.reconcile(std::slice::from_ref(&snapshot), 0)?;
        let group = resumed
            .groups
            .get(&snapshot.key)
            .ok_or(LiveError::NotEnabled)?;
        check_eq!(
            group.roster.validate_command_at(ObjId(10), 11, 1, 0),
            Err(RosterError::StaleRevision)
        );
        Ok(())
    }
}
