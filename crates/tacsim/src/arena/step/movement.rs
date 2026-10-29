//! Substep lattice timing, placement resolution, and mind landing/think phase execution.

use super::*;

impl Arena {
    // The staff place (16b cut I.3; 16c N10)
    // -----------------------------------------------------------------------
    //
    // Every `Place` the arena receives leaves one line: `place_refused` at
    // phase 2 (`unknown`, `epoch`, `channel`), `placed` or `place_refused` here
    // at phase 4, or `place_refused` with `stopped` in the pulse a `Stop` ends
    // the match in (`Arena::stopped`). The one exception is a place queued at
    // phase 2 of the pulse whose own substep ends the match — a kill or the
    // duration bound — which phase 4 never drains, because no line may follow
    // `match_ended` (16b cut W.2).

    /// **Put one unit on a cell, for staff** (16b cut I.3; 16c N10): the
    /// phase-4 resolution of an [`IntentKind::Place`], at the pulse's end and
    /// under phase 4's one hash like every action there.
    ///
    /// The tests, in this order, the first that fails naming the reason:
    ///
    /// 1. the unit exists — else `unknown`. Unreachable past phase 2's own gate,
    ///    and journalled all the same, so *every `Place` the arena receives
    ///    leaves a line* holds by one rule;
    /// 2. the cell is on the window, passable and has a way out **for the
    ///    placed unit's own cost row**
    ///    ([`crate::arena::state::ArenaTerrain::has_way_out_for`] on
    ///    `UnitState::cost_row`, 16b cut AE.1 — the rule an opening cell is
    ///    held to; a car onto rubble is refused though a tank would stand
    ///    there) — else `impassable`;
    /// 3. nobody else stands in it ([`crate::arena::index::SpatialIndex::occupant`]
    ///    is empty or the unit itself) — else `occupied`: doc 16 §3.4 step 5's
    ///    one unit per cell;
    /// 4. **no round is in flight to or from the unit** — no [`Arena::shots`]
    ///    row whose `target` or `shooter` it is — else `in_flight`. An impact
    ///    traces its line from the SHOOTER's pose when the round arrives
    ///    (`damage::resolve_impact`) and teaches its machine `no line` on a
    ///    blocked one, so a placed shooter would decide its own round's outcome,
    ///    and a placed target would dodge (or meet) a round aimed elsewhere.
    ///
    /// On success the pose goes to the cell's centre at the cell's `elev_cm`,
    /// heading kept; speed, desired speed and the motion carries go to nought;
    /// the hold, the traffic hold, `blocked` and `grade_pct` are cleared; the
    /// index moves the unit (rebuilt whole when it stood off the window); and a
    /// machine forgets its route ([`crate::arena::motor::MotorTable::forget_route`])
    /// so it re-plans at its next tick. **No contact is dropped**: a contact id
    /// and its designator are drawn once per (viewer, target) for the whole
    /// match (`draw_contact_id`, `DesignatorBook::assign`), so a drop could mint
    /// no new name, and the loss path is split between the sensor pass and the
    /// staging scan; every other platform's contact on the unit is left as it
    /// is, and the next sensor pass moves the track to where the unit now is —
    /// which is the truth of that sensor. Then one `placed` line, `from_x` and
    /// `from_y` `u16::MAX` for a unit that stood off the window.
    ///
    /// **Every outcome is journalled and none is staged** (16c N10: *journalled
    /// always, announced never*): the pilot's next frame carries the new cell
    /// and nothing says why. The hash is asked before anything is written,
    /// `Arena::event_hash`'s rule.
    ///
    /// # Errors
    ///
    /// The reason word, after its `place_refused` line is journalled.
    pub(crate) fn resolve_place(
        &mut self,
        unit: ObjId,
        x: u16,
        y: u16,
        at_ms: i64,
    ) -> Result<(), SmolStr> {
        let refused = self.place_refusal(unit, x, y);
        let hash = self.event_hash();
        if let Some(reason) = refused {
            let reason = SmolStr::new_static(reason);
            let _ = self.journal(
                at_ms,
                EventKind::PlaceRefused {
                    unit,
                    x,
                    y,
                    reason: reason.clone(),
                },
                hash,
            );
            return Err(reason);
        }
        let terrain = &self.definition.terrain;
        let cell_cm = i64::from(terrain.cell_cm());
        let elev_cm = terrain.get(x, y).map_or(0, |cell| i32::from(cell.elev_cm));
        let from = self
            .unit(unit)
            .and_then(|state| self.definition.terrain.cell_of(&state.pose));
        if let Some(state) = self.unit_mut(unit) {
            state.pose.x_cm = i64::from(x)
                .saturating_mul(cell_cm)
                .saturating_add(cell_cm / 2);
            state.pose.y_cm = i64::from(y)
                .saturating_mul(cell_cm)
                .saturating_add(cell_cm / 2);
            state.pose.z_cm = elev_cm;
            state.speed_cms = 0;
            state.desired_speed_cms = 0;
            state.carry = crate::arena::state::MotionCarry::default();
            state.held = None;
            state.occupied_hold = None;
            state.blocked = false;
            state.grade_pct = 0;
        }
        match from {
            Some(from) => self.index.move_unit(unit, from, (x, y)),
            None => self.index.rebuild(&self.definition.terrain, &self.units),
        }
        // PLACE invalidates both a former completion and the private formation
        // assignment. Resume the principal's request with a fresh fan from the
        // new pose, without touching the mind's memory.
        let restart = self
            .motors
            .row(unit)
            .and_then(|motor| motor.requested_goal.or(motor.goal));
        self.motors.forget_route(unit);
        if let Some(requested) = restart {
            let goal = self.fan_goal(unit, requested);
            let motor = self.motors.ensure(unit);
            motor.goal = Some(goal);
            motor.fan = crate::arena::motor::GoalFan::between(requested, goal);
        }
        if self
            .motors
            .row(unit)
            .is_some_and(|motor| motor.goal.is_some())
        {
            self.engage_motor(unit);
        }
        let (from_x, from_y) = from.unwrap_or((u16::MAX, u16::MAX));
        let _ = self.journal(
            at_ms,
            EventKind::Placed {
                unit,
                x,
                y,
                from_x,
                from_y,
            },
            hash,
        );
        Ok(())
    }

    /// **Why a place would be refused**, or `None` (16b cut I.3): the four
    /// tests of [`Arena::resolve_place`], in its order, as their wire words.
    pub(super) fn place_refusal(&self, unit: ObjId, x: u16, y: u16) -> Option<&'static str> {
        let Some(state) = self.unit(unit) else {
            return Some("unknown");
        };
        // **16b cut AE.1** (16c `B-58`): passable, with a way out, **to the
        // placed unit's own row** — a car put down on rubble is `impassable`,
        // whatever a tank could have done there.
        let terrain = &self.definition.terrain;
        if !terrain.has_way_out_for(state.cost_row, x, y) {
            return Some("impassable");
        }
        if self
            .index
            .occupant(x, y)
            .is_some_and(|holder| holder != unit)
        {
            return Some("occupied");
        }
        if self
            .shots
            .values()
            .any(|shot| shot.target == unit || shot.shooter == unit)
        {
            return Some("in_flight");
        }
        None
    }

    /// **Run one millisecond's held landings, the first of its two mind
    /// phases** (17b cut M1b.0; doc 17 §6, D-UM-20 as amended 2026-10-04,
    /// D-UM-21): every held `MindLand`, by unit then `think_seq`, drained from
    /// `held`; the held thinks stay in it.
    ///
    /// The order of a millisecond is **every landing, then the heap's ordinary
    /// events at that millisecond, then the thinks** — so an impact a landing's
    /// beam schedules at its own millisecond is applied before any think of
    /// that millisecond takes its view. The due loop's head runs this, goes
    /// round to pop whatever the landings scheduled at their millisecond (a
    /// beam's zero-flight `Impact`; a goal's `MotorTick` when the landing falls
    /// on the substep's end, `Arena::engage_motor`'s `grid_at` reading
    /// `now_ms`; a `StartupComplete` under a `startup_ms` of nought), and runs
    /// [`Arena::run_mind_thinks`] once the heap's head has left it. Every
    /// landing runs before any impact of theirs, so two minds' beams at one
    /// millisecond both fire and both land: the simultaneity two reflex ticks
    /// at one millisecond had. (A landing on a hull already a wreck is not
    /// refused but dropped — `Arena::mind_land` returns before applying it —
    /// so a per-landing order would lose the second shot silently.) A reflex
    /// tick at that millisecond was NOT mutual with them — the asymmetry the
    /// loop head declares, retired with the reflex controller in 17b cut M4.6.
    ///
    /// The lazy action hash is opened as the reflex tick's arm opened it (16b
    /// cut M.3: opened, not taken), so a landing's lines share the substep's
    /// one hash.
    pub(super) fn run_mind_landings(
        &mut self,
        held: &mut Vec<DueEvent>,
        reflex: &mut MotorCounters,
    ) {
        let mut landings: Vec<(ObjId, u64, i64)> = Vec::new();
        held.retain(|event| match event.kind {
            DueKind::MindLand { unit, think_seq } => {
                landings.push((unit, think_seq, event.t_ms));
                false
            }
            _ => true,
        });
        landings.sort_unstable();
        for (unit, think_seq, t_ms) in landings {
            if self.action_hash == ActionHash::Outside {
                self.action_hash = ActionHash::Pending;
            }
            self.mind_land(unit, think_seq, t_ms, reflex);
        }
    }

    /// **Run one millisecond's held thinks, the second of its two mind
    /// phases** (17b cut M1b.0; doc 17 §6, D-UM-20 as amended 2026-10-04):
    /// every held `MindThink`, by unit then `think_seq`, drained from `held` —
    /// reached only when no landing is held and the heap's head has left the
    /// millisecond, so every landing of the millisecond and everything those
    /// landings scheduled at it has been applied, and each think's view is the
    /// state then ([`Arena::run_mind_landings`] has the order). **No action
    /// hash is opened for a think** — kept so by 17b cut M3.4, whose
    /// `mind_fault` and `mind_dropped` lines (`Arena::mind_think`) fill no
    /// lazy slot either: they carry the substep's hash once an earlier line
    /// has TAKEN it, else a fresh state hash that stores nothing — under
    /// `Outside`, and under a `Pending` slot a landing (or, until 17b cut M4.6,
    /// a reflex tick) earlier in the substep opened and journalled nothing under
    /// (`Arena::think_line_hash`; [`Arena::event_hash`] would fill that slot
    /// with the think's moment). So a think, clean or faulted, moves no later
    /// line's hash in its substep (opening one before every think would make
    /// the substep's later lines share one hash where each had its own).
    pub(super) fn run_mind_thinks(&mut self, held: &mut Vec<DueEvent>, reflex: &mut MotorCounters) {
        let mut thinks: Vec<(ObjId, u64, i64)> = held
            .drain(..)
            .filter_map(|event| match event.kind {
                DueKind::MindThink { unit, think_seq } => Some((unit, think_seq, event.t_ms)),
                _ => None,
            })
            .collect();
        thinks.sort_unstable();
        for (unit, think_seq, t_ms) in thinks {
            // 17b cut M4.1: collected here, written at the next substep's
            // head (`Arena::settle_inline`) or by the task's delivery.
            self.collect_think(unit, think_seq, t_ms, reflex);
        }
    }

    /// **The substep length**, arena milliseconds (doc 16 §3.1 "Rate
    /// independence"): `substep_ms` over this match's tempo block, its cell and
    /// its true speed ceiling.
    ///
    /// **16b cut U.1**: one number with three readers — the partition in
    /// [`Arena::step`], the lattice `Arena::next_due_ms` wakes the task on, and
    /// the stopping look-ahead in [`Arena::hold_routeless`] — where the first and
    /// the last each computed the product for themselves.
    ///
    /// **It is read live, and it is not frozen in general.** The tempo block and
    /// the cell are fixed at [`Arena::new`], but [`Arena::max_speed_cms`] is
    /// recomputed on every call over the units that are neither a wreck nor
    /// motion-disabled, and both of those change mid-match. The ceiling can only
    /// fall, so the substep can only lengthen, up to `tempo.substep_max_ms`. On
    /// the shipped content the answer is constant because that cap binds for
    /// every fielded unit (250 ms; 16c Q24); on a config whose fastest unit cuts
    /// a shorter substep, losing that unit lengthens it from the next pulse, and
    /// both the step's partition and `Arena::next_due_ms`'s lattice read the
    /// new figure there. Freezing the ceiling at check-out would be a behaviour
    /// change and is not this cut's.
    ///
    /// **Public since 17b cut M1a.6**: the twin lane (`tests/twin.rs`) steps a
    /// match one substep a pulse, so the arena it reads after a pulse that ran
    /// a think is the state that think saw.
    #[must_use]
    pub fn substep_len_ms(&self) -> i64 {
        i64::from(substep_ms(
            &self.arena_cfg.tempo,
            self.cell_cm(),
            self.max_speed_cms(),
        ))
    }

    /// **Resume a decoded arena on this process's wall** (16b cut AF.1; doc 16
    /// §3.1 "Clock and lag"; 16c N16), answering the first deadline the task
    /// should wake at, in the caller's own wall milliseconds — exactly what a
    /// pulse's [`StepOutput::next_due_ms`] would have answered at that wall.
    ///
    /// The clock is re-anchored ([`crate::arena::clock::Clock::reanchor`]):
    /// **arena time does not advance across the outage**, the first resumed
    /// pulse measures its `dt` from `wall_now_ms`, no `lagged` line is journalled
    /// for the gap, and a staff floor the old process had set is gone (16c
    /// `Q-AF1`). Everything that decides *when* something happens next — the
    /// due heap, the shots in flight, the staggered sensor passes and reflex
    /// ticks, the wake lattice itself — is carried integers in arena time, so
    /// the deadline answered here is the one the match would have woken at, and
    /// the cross-tempo property holds across a resume as it holds across a
    /// pulse (`crates/tacsim/tests/arena.rs`'s `af1_*` tests; a round carried
    /// in flight across a resume, at two cadences, is
    /// `crates/tacsim/tests/shipped_scale.rs`'
    /// `af1_the_shipped_centre_encodes_under_the_expectation`).
    pub fn resume_at(&mut self, wall_now_ms: i64) -> i64 {
        self.clock.reanchor(wall_now_ms);
        self.next_due_ms(wall_now_ms)
    }

    /// **When the task should step again**, in the wall milliseconds
    /// `wall_now_ms` was given in (doc 16 §3.1 phase 5; 16c Q25).
    ///
    /// The rule, in **absolute arena time**: the earlier of the next multiple of
    /// the tempo's period strictly after `now_ms`, and — when anything is due —
    /// the first multiple of the substep length at or after the next due event
    /// (never `now_ms` itself); answered as `wall_now_ms + (target − now_ms)`, so
    /// never sooner than a millisecond away.
    ///
    /// **Why the targets are absolute** (16b cut U.1; 16c `T-22`). A due event
    /// inside a substep is applied in that substep, at its own timestamp,
    /// against the positions at the substep's **end** (§3.1 phase 3). Woken at
    /// the due event itself — 16c's first sketch — the pulse would end there, and
    /// the next substep would be cut from that wake: the position the event is
    /// applied against would be a function of when the task woke, which is
    /// exactly what D-16-2 forbids. So the wake is the substep boundary the event
    /// already belongs to. And the target is a lattice point in arena time, not
    /// an interval from the last wake: a rule of `wall + period` or `wall +
    /// (due − now)` measures from wherever the last wake landed, so after one
    /// late wake it would never return to the lattice; this one pulls a late
    /// wake back (from `now = 253` with a due at 500 on a 250 ms lattice, it asks
    /// for 247 ms). The period is a multiple of the substep for every shipped
    /// configuration (asserted for every fielded unit since 16b cut M.2, 16c
    /// Q24), so the period multiples are lattice points too, and a task that
    /// wakes on time wakes only on the lattice.
    ///
    /// In production arena time is wall time, so a wake is the lattice plus the
    /// OS timer's overshoot (wall jitter, §3.1); a late wake's pulse ends off the
    /// lattice, and the next one returns to it.
    ///
    /// The lattice is the **current** [`Arena::substep_len_ms`], which is
    /// constant on the shipped content and can lengthen mid-match elsewhere (see
    /// there); the partition inside a pulse is still cut from that pulse's start.
    ///
    /// **16b cut I.3: under a staff floor the answer is a WALL interval** —
    /// `wall_now_ms + max(period, floor)` ([`crate::arena::clock::Clock::wake_period_ms`]),
    /// the due-event branch not consulted. A floored arena is behind the wall on
    /// purpose, so the rule above — `wall + (target − now)` with the target in
    /// arena time — would wake the task after 5,000, then 3,000, then 1,000 wall
    /// ms as the arena caught up with lattice points it can only reach a clamp
    /// at a time. The due events are still applied at their own timestamps,
    /// inside the catch-up substeps, as always; and the lattice is kept by
    /// [`Arena::step`] quantising every floored pulse instead (see there). With
    /// no floor the rule is unchanged.
    pub(super) fn next_due_ms(&self, wall_now_ms: i64) -> i64 {
        if self.clock.floor_ms().is_some() {
            return wall_now_ms
                .saturating_add(i64::from(self.clock.wake_period_ms(&self.arena_cfg.tempo)));
        }
        let now = self.now_ms;
        let period = i64::from(self.clock.tempo().period_ms(&self.arena_cfg.tempo));
        let substep = self.substep_len_ms();
        let after_now = now.saturating_add(1);
        let target_period = ceil_to(after_now, period);
        let target = match self.due.next_t_ms() {
            Some(due) => target_period.min(ceil_to(due.max(after_now), substep)),
            None => target_period,
        };
        wall_now_ms.saturating_add(target.saturating_sub(now))
    }
}

/// **`t` rounded up to a multiple of `m`** (16b cut U.1), saturating: the least
/// multiple of `m` at or after `t`. An `m` of nought or below answers `t`, so a
/// degenerate period or substep asks for the moment itself rather than for
/// nothing. Integer arithmetic only; `t` is arena milliseconds and never
/// negative where [`Arena::next_due_ms`] calls it.
pub(super) fn ceil_to(t: i64, m: i64) -> i64 {
    if m <= 0 {
        return t;
    }
    t.saturating_add(m - 1).div_euclid(m).saturating_mul(m)
}

/// **The reason word of a staff place that reached the pulse a `Stop` ends the
/// match in** (16b cut I.3): `place_refused` with `stopped`, journalled by
/// [`Arena::stopped`] before the stop's own line.
pub(super) const PLACE_STOPPED: &str = "stopped";

/// **`t` rounded down to a multiple of `m`** (16b cut I.3), [`ceil_to`]'s other
/// half: the greatest multiple of `m` at or below `t`. An `m` of nought or below
/// answers `t`, so a degenerate substep quantises nothing rather than stopping
/// time. Integer arithmetic only; [`Arena::step`] calls it with `t` the arena
/// millisecond a floored pulse would end at.
pub(super) fn floor_to(t: i64, m: i64) -> i64 {
    if m <= 0 {
        return t;
    }
    t.div_euclid(m).saturating_mul(m)
}
