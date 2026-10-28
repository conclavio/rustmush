//! Discrete action queue resolution (Phase 4), zero-flight drain, and combat refusal/damage notices.

use super::*;

impl Arena {
    pub(super) fn pulse_actions(&mut self, reflex: &mut MotorCounters) {
        // ---- phase 4: the queued discrete actions ----
        //
        // Doc 16 §3.1: *resolve newly queued discrete actions that became legal this
        // pulse*. Nothing is re-queued — an action is resolved or refused in the
        // pulse it was drained in, so a bound another order needs is never held by
        // one that cannot be resolved — and a refusal is the pilot's business, not
        // the ledger's: **16b cut F.2** stages a `FrameNotice::Refused` here and
        // `crate::match_::cockpit::refusal_line` is what turns it into a
        // sentence, so a refused order is said and still never written to the
        // black box.
        //
        // One hash for the whole phase, taken once: the phase runs at one arena
        // timestamp with no time advancing inside it, so every action it resolves
        // happened at one moment and belongs to one hash — the same rule the substep
        // above follows, and see `Arena::action_hash` for what it costs not to.
        // A match that ended inside phase 3 resolves nothing here: a shot accepted
        // after `match_ended` would be a line in the black box about a match that was
        // already over (16b cut W.2).
        //
        // **16b cut M.3** (16c `T-23`): the slot is `Pending`, not a hash — the
        // first resolver that journals takes it (`Arena::event_hash`), so a pulse
        // with nothing queued, or an ended match, takes no `state_hash` here at
        // all. It used to be taken unconditionally, every pulse, before the
        // `ended` guard.
        self.action_hash = ActionHash::Pending;
        // **16b cut M.2** (16c `T-02`; doc 16 §11's D-16-2 as amended): a pilot's
        // discrete action resolves at the end of the pulse that accepted it, so
        // its arena millisecond is this one — which in production is a wall-clock
        // instant, since arena time is elapsed wall time and a pulse ends on the
        // substep lattice (the earlier of the next period multiple and the first
        // boundary at or after the next due event, 16b cut U.1) plus the timer's
        // overshoot. Everything the action
        // schedules from here is a due event and rate-independent. The resolvers
        // take the millisecond as an argument because a machine's actions resolve
        // at its own millisecond instead (a mind's landing; the reflex's tick
        // until 17b cut M4.6).
        let at_ms = self.now_ms;
        let ids: Vec<ObjId> = if self.ended.is_some() {
            Vec::new()
        } else {
            self.units.iter().map(|unit| unit.id).collect()
        };
        for id in ids {
            for order in drain_pending(self, id) {
                // **16b cut D.2**: the order's own word, through the one accessor
                // phase 2 reads. This phase passed the literals `"startup"`,
                // `"lock"` and `"fire"`, which was a second spelling of a
                // vocabulary `IntentKind::as_str` already owns — and the bridge's
                // `Starting` hint matches on that accessor's answer.
                let word = order.kind.as_str();
                match order.kind {
                    // **16b cut I.3** (16c N10): the staff place, resolved at the
                    // pulse's end under the phase's one hash like every action
                    // here. `resolve_place` journals its own outcome, `placed` or
                    // `place_refused`, and nothing is ever staged to a pilot —
                    // a refusal naming a round in flight would tell the pilot of
                    // an impact their sensors never saw.
                    IntentKind::Place { x, y } => {
                        let _ = self.resolve_place(id, x, y, at_ms);
                    }
                    // Doc 16 §3.3 "Startup".
                    IntentKind::Startup => {
                        if let Err(refusal) = resolve_startup(self, id, at_ms) {
                            self.stage_refusal(id, word, refusal, None);
                        }
                    }
                    // **16b cut O.2** (16c N12): the pod's switch, in `Startup`'s
                    // shape. `resolve_emcon` journals and stages its own flip; a
                    // no-op journals and stages nothing.
                    IntentKind::Emcon { on } => {
                        if let Err(refusal) = resolve_emcon(self, id, on, at_ms) {
                            self.stage_refusal(id, word, refusal, None);
                        }
                    }
                    // **16b cut W.1** (doc 16 §3.7, §3.5): the lock, and the shot.
                    IntentKind::Lock { contact } => match self.resolve_lock(id, contact, at_ms) {
                        // **16b cut F.2** (doc 16 §3.5 step 5): the percentage
                        // *on `LOCK` before any shot*. The lock resolves here, at
                        // its own arena timestamp, so this is where it is said —
                        // the cockpit could only ever say it against the frame it
                        // held *before* the order, which is why a first `LOCK`
                        // was silent.
                        // 16b cut Q.1: named off the live contact at the moment
                        // the lock resolved.
                        Ok(()) => {
                            let named = self.name_of(id, contact);
                            self.stage_notice(id, FrameNotice::Locked { contact: named });
                            // **17b cut M1b.1** (doc 17 §5, D-UM-16): a new lock
                            // is a new target for the hull's slots, read at the
                            // lock's own millisecond and in the typed order — a
                            // `LOCK` then an arming of one pulse lands the lock,
                            // then the arming. A hull with no slot returns at once.
                            let _ = self.evaluate_conditions(
                                id,
                                at_ms,
                                ConditionTrigger::Lock,
                                None,
                                reflex,
                            );
                        }
                        Err(refusal) => self.stage_refusal(id, word, refusal, None),
                    },
                    // A seat's typed `FIRE` — the human trigger, never gated
                    // (`Arena::resolve_fire` is `FiredBy::Seat`).
                    IntentKind::Fire { ref weapon, .. } => {
                        if let Err(refusal) = self.resolve_fire(id, weapon, at_ms) {
                            self.stage_refusal(id, word, refusal, Some(weapon.clone()));
                        }
                    }
                    // **17b cut M1b.1** (doc 17 §4 D-UM-10, §5 D-UM-16, D-UM-17):
                    // a seat's arming lands at the pulse's end in arrival order,
                    // through the one resolver a mind's goes through; an armed
                    // slot is evaluated at once (`ConditionTrigger::Arming`), so
                    // a slot armed on a ready gun and a warranted lock fires at
                    // this millisecond. Every refusal — `NoWeapon`, `Unlawful`,
                    // `NoSlot` — is staged here with the arming's weapon, so it
                    // reads `FIRE cannon refused. …` as every `FIRE` refusal does.
                    IntentKind::Condition(ref arming) => {
                        match self.resolve_manual_arming(id, arming, at_ms) {
                            Ok(_) => {
                                let _ = self.evaluate_conditions(
                                    id,
                                    at_ms,
                                    ConditionTrigger::Arming,
                                    None,
                                    reflex,
                                );
                            }
                            Err(refusal) => {
                                let notice =
                                    self.arming_refusal_notice(id, arming, refusal, self.now_ms);
                                self.stage_notice(id, notice);
                            }
                        }
                    }
                    // Continuous orders never queue (phase 2 applies them), and
                    // neither does a squad's standing order (16b cut N.2): it is
                    // written onto the machine at apply (a mind's row; the reflex
                    // row until 17b cut M4.6) and read by its next think. **17b
                    // cut M1a.4**: nor does a `DRIVE`'s
                    // goal, which phase 2 writes onto the motor row.
                    IntentKind::Heading { .. }
                    | IntentKind::Speed { .. }
                    | IntentKind::Order(_)
                    | IntentKind::DetachmentCommand(_)
                    | IntentKind::ReleaseOrder { .. }
                    | IntentKind::Goal(_)
                    // 17b cut M5.2: nor a profile switch, which phase 2 writes
                    // onto the unit (and a standing mind row's card).
                    | IntentKind::Profile { .. } => {}
                }
            }
        }
        self.action_hash = ActionHash::Outside;
    }

    pub(super) fn pulse_drain(&mut self, work: &mut PulseWork) {
        // ---- the phase-4 drain (16b cut M.2, 16c `B-51`) ----
        //
        // A pilot's beam flies in no time, so phase 4 schedules its impact at
        // `now_ms` — and phase 4 drains nothing. The round then resolved in the
        // NEXT pulse's first substep, against positions later than the fire
        // timestamp it is stamped with, and `next_due_ms` answered the wall
        // reading itself, so the task's timer slept for nothing. One zero-length
        // substep here pops exactly what phase 4 left due at `now_ms`.
        //
        // **The `ran > 0` guard is the rule, not a precaution.** After at least
        // one substep, every event at or before `now_ms` that was in the heap
        // before phase 4 has already been popped by the last substep's due loop,
        // and the substep's staging scan schedules nothing at or before `now_ms`
        // (a held loss whose window closes inside the substep is settled by the
        // scan itself, never scheduled into the past), so anything still due now
        // was scheduled BY phase 4 — a beam's zero-flight impact — and the drain
        // is exactly "phase 4's own zero-flight events". (A `WeaponReady` cannot
        // be one with shipped content: `cycle_ms` is floored at 1,000 by W14,
        // longer than any substep.) The opening pulse runs no substep and drains
        // nothing: its `t = 0` events (a `SensorPass` or — until 17b cut M4.6 —
        // a reflex tick for a unit whose stagger is nought — `stagger_ms`, 16b
        // cut U.1 — and a
        // `MatchDeadline` of `0`) pop in the next pulse's first substep exactly
        // as they always did, so the opening pulse's hash sequence, its
        // `substeps == 0` and phase 5's duration backstop are untouched. A
        // zero-length substep is well-defined (`Arena::substep` skips motion at
        // `dt 0`, `dissipate_heat(0)` sheds nothing, the index is rebuilt once,
        // the due loop pops what phase 4 scheduled, elimination settles, one hash
        // is recorded).
        //
        // **The drain runs only on a pulse that ran a substep** — or, since 16b
        // cut I.3, on a floored one (below). Under the scripted zero-`dt`
        // drivers, which are unfloored, a beat's pulse runs none, so there the
        // drain never runs: a beam fired on a beat still lands in the next
        // pulse's first substep, the same at every cadence, which is why those
        // replays agree and why they do not prove the drain. It is proved on a
        // driver whose pulse ends on the beat, the production shape
        // (`tests/extensions.rs`'s beam replay and `tests/weapons.rs`'s
        // `a_beam_is_scheduled_at_now`).
        //
        // **16b cut I.3: and on a quantised pulse of no time, past the opening.**
        // Under a staff floor a production pulse can run no substep — the
        // pull-in on a set, a lift or an abort, or any wake inside a substep,
        // cut back to the lattice point it started on. A beam `FIRE` such a
        // pulse resolves at `now_ms` would otherwise wait out the floor and land
        // in the next wake's first substep, against positions a substep later
        // than the same `FIRE` resolved at the same millisecond by a pulse that
        // ran one: the outcome would turn on the wall moment staff typed the
        // verb, which is what the quantisation exists to exclude. The rule above
        // holds for it by induction: once arena time has moved (`now_ms > 0`),
        // the pulse before this one either ran a substep ending at `now_ms` or
        // was itself a floored pulse of no time and drained — an unfloored
        // production pulse always runs one, `arena_task`'s `deadline` giving it
        // at least a millisecond — so anything due now was scheduled by this
        // phase 4. At `now_ms == 0` nothing has run and the `t = 0` events are
        // the next substep's, exactly as after the opening pulse, so a floored
        // pulse of no time there drains nothing (`tests/weapons.rs`'s
        // `i3_a_beam_fired_in_a_floored_pulse_of_no_time_lands_in_it`).
        let drains = work.ran > 0 || (work.floored && self.now_ms > 0);
        if drains && self.ended.is_none() && self.due.peek_due(self.now_ms).is_some() {
            work.ledger_full |= self.substep(0, &mut work.perception, &mut work.reflex);
            work.ran = work.ran.saturating_add(1);
        }
    }

    /// **Whose armour the round passed, and what that facet and this section hold
    /// when they are whole** (16b cut F.2), so the victim's line can say *of* as
    /// well as *left*.
    ///
    /// The armour is **not** always the struck section's. Doc 16 §3.3: a section
    /// with no armour of its own is *struck through* another's — the engine
    /// through `hull.rear` — and `crate::arena::damage::Arena::plan_hit` spends
    /// the round on that other section's cell, so the `armor_left` the ledger
    /// carries is that cell's and its maximum has to be read from the same place.
    /// Reading it at `engine/rear/armor`, a cell the stock tank does not declare,
    /// answered nought, and a pilot shot in the back read `110 of 0 armour left`.
    /// So the pair is resolved through the profile's own `struck_through`, exactly
    /// as the plan was, and the section it names is returned with it for the
    /// sentence to be honest about.
    ///
    /// The internals are always the **struck** section's: that is where the
    /// overflow went, whoever's armour it passed first.
    ///
    /// Both maxima are read off the cells' own `max`, which is content and does
    /// not move, so they are facts about the tank rather than about the round.
    ///
    /// **16c T-110a**: the two cell ids are the once-built ones
    /// ([`crate::definitions::unit::armor_cell`],
    /// [`crate::definitions::unit::internals_cell`]) where each was a
    /// `CellId::new` a round; the section and facet ids are still built here.
    pub(super) fn struck_maxima(
        &self,
        unit: ObjId,
        section: &str,
        facet: &str,
    ) -> (SmolStr, u32, u32) {
        let Some(state) = self.unit(unit) else {
            return (SmolStr::new(section), 0, 0);
        };
        let armor_section = state
            .profile
            .section(section)
            .and_then(|profile| profile.struck_through.as_ref())
            .map_or_else(
                || SmolStr::new(section),
                |(through, _)| SmolStr::new(through.as_str()),
            );
        let armor = mush_tactics::SectionId::new(armor_section.as_str())
            .ok()
            .zip(mush_tactics::FacetId::new(facet).ok())
            .and_then(|(section, facet)| {
                armor_cell().map(|cell| mush_tactics::CellAddr::Facet {
                    section,
                    facet,
                    cell: cell.clone(),
                })
            })
            .and_then(|addr| state.unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.max.max(0)).unwrap_or(u32::MAX));
        let internals = mush_tactics::SectionId::new(section)
            .ok()
            .and_then(|section| {
                internals_cell().map(|cell| mush_tactics::CellAddr::Section {
                    section,
                    cell: cell.clone(),
                })
            })
            .and_then(|addr| state.unit.cell(&addr))
            .map_or(0, |cell| u32::try_from(cell.max.max(0)).unwrap_or(u32::MAX));
        (armor_section, armor, internals)
    }

    /// **Whether this reader may be told about a spent section now** (16b cut
    /// D.2, `B-15`), and the write that goes with a `true`.
    ///
    /// The key is `(the platform told, the target, the section)`: the victim's
    /// own key is `(victim, victim, section)`, because his tank is both. A `true`
    /// writes the entry, so the next round inside
    /// [`SPENT_NOTICE_MS`] of arena time is
    /// silent.
    ///
    /// It runs **whether or not anybody is seated on the platform and whether or
    /// not a frame is ever sent**: the staging scan runs per substep at the
    /// event's own timestamp, so the table is a function of arena time alone and
    /// the fold below it is rate-independent. The pruning is the caller's, on
    /// touch, for the same reason.
    pub(super) fn say_spent(
        &mut self,
        told: ObjId,
        target: ObjId,
        section: &SmolStr,
        t_ms: i64,
    ) -> bool {
        let key = (told, target, section.clone());
        if let Some(at) = self.spent_said.get(&key)
            && t_ms.saturating_sub(*at) < SPENT_NOTICE_MS
        {
            return false;
        }
        self.spent_said.insert(key, t_ms);
        true
    }

    /// **Whether pulse phase 2 says a refusal out loud** (16b cut D.2, `B-03`).
    ///
    /// Five of the twenty, and no others (four until 17b cut M5.2 added the
    /// fifth):
    ///
    /// * [`Refusal::Wreck`] — the pilot is typing into a dead tank and nothing
    ///   else was telling them so;
    /// * [`Refusal::MotionDisabled`] — the engine is gone and `HEADING` does
    ///   nothing;
    /// * [`Refusal::PowerNotOnline`] — doc 16 §3.3's cold tank, refused at apply
    ///   when the cockpit's own mirror was a pulse stale;
    /// * [`Refusal::QueueFull`] — the ninth order of a burst was **dropped**, and
    ///   silently, which is the defect `B-03` names. The bound it is the far
    ///   side of is a seat's discrete orders a unit per **arena second**
    ///   (`apply_intent`, 16b cut M.1, `T-82`), the same at every tempo; before
    ///   that cut it was a queue drained once a pulse, which made how many orders
    ///   a pilot might burst a function of the tempo;
    /// * [`Refusal::Profile`] (**17b cut M5.2**; doc 17 D-UM-44) — a seat's
    ///   `MIND PROFILE` the arena could not take: a hull with no seal, an index
    ///   past the sealed set, the profile already active, or a switch inside
    ///   ten seconds of the last. The cockpit's local pre-checks (17b cut M5.4)
    ///   catch most of these first; this is the arena's word when one gets this
    ///   far.
    ///
    /// The other five have nobody to tell. [`Refusal::Epoch`] is doc 16 §3.1's
    /// `FIRE` then `EXIT` burst working exactly as written: the pilot who left is
    /// told nothing about the shot they did not take. [`Refusal::Channel`],
    /// [`Refusal::Unknown`], [`Refusal::Linkdead`] and [`Refusal::NotMachine`] are
    /// answers to something no seated pilot's cockpit can produce — a wrong
    /// channel is a routing fault, an absent unit has no pilot, a linkdead seat
    /// has nobody at it, and an `ORDER` is a squad's.
    ///
    /// An [`IntentKind::Order`] is never staged whatever it was refused for: it
    /// is a commander's doctrine and not a pilot's order, and a tank's pilots are
    /// not the audience for it. **16b cut Z.2**: nor is an
    /// [`IntentKind::ReleaseOrder`], for the same reason — a release hands a
    /// squad's doctrine back, and the tank it lands on is not the pilot who gave
    /// it.
    ///
    /// **16b cut I.3**: nor is an [`IntentKind::Place`] — a staff act the pilot
    /// is told nothing of (16c N10: *journalled always, announced never*); its
    /// phase-2 refusal is a `place_refused` line on the tape instead.
    pub(super) const fn phase_two_tells(kind: &IntentKind, refusal: Refusal) -> bool {
        if matches!(
            kind,
            IntentKind::Order(_)
                | IntentKind::ReleaseOrder { .. }
                | IntentKind::Place { .. }
                | IntentKind::DetachmentCommand(_)
        ) {
            return false;
        }
        matches!(
            refusal,
            Refusal::Wreck
                | Refusal::MotionDisabled
                | Refusal::QueueFull
                | Refusal::PowerNotOnline
                | Refusal::Profile
        )
    }

    /// **Stage the line one refused order owes its pilot** (16b cut F.2; doc 16
    /// §5: *`Notice` events for transitions and refusals*).
    ///
    /// The recorded open point K8, in its broad form: pulse phase 4 threw every
    /// refusal away — `let _ = self.resolve_fire(…)` — and the only refusals a
    /// pilot ever saw were the cockpit's own pre-checks against a frame up to one
    /// pulse old. So a second `FIRE` inside one pulse printed `Firing cannon.` and
    /// spent nothing, and a lock that expired between the frame and the apply was
    /// silent.
    ///
    /// The numbers are read **here**, at the refusal, because that is the only
    /// moment they are true; `crate::match_::cockpit` turns the pair into the same
    /// sentence its own pre-check prints for the same condition, which is what
    /// keeps the early answer and the late one from drifting apart.
    ///
    /// Two silences are deliberate. An intent **purged** by an epoch raise never
    /// reaches this phase at all (`apply_control` drops it), which is doc 16
    /// §3.1's `FIRE` then `EXIT` burst working exactly as written — the pilot who
    /// left is told nothing about the shot they did not take. And the reflex
    /// controller resolves its own shots inside a substep and never through this
    /// phase, so an AI refusal reaches nobody.
    ///
    /// Pulse **phase 2** stages through here too since 16b cut D.2 (`B-03`), for
    /// the five refusals [`Arena::phase_two_tells`] admits (four until 17b cut
    /// M5.2); the order word is
    /// [`IntentKind::as_str`] in both phases, so one order cannot be spelled two
    /// ways.
    pub(super) fn stage_refusal(
        &mut self,
        unit: ObjId,
        order: &'static str,
        refusal: Refusal,
        weapon: Option<SmolStr>,
    ) {
        let notice = self.refusal_notice(unit, order, refusal, weapon, self.now_ms);
        self.stage_notice(unit, notice);
    }

    /// **The notice one refused order is told as** (16b cut F.2's shape,
    /// factored out by 17b cut M1a.5): the order's word, the refusal, the
    /// weapon it named and the numbers its sentence carries, read at `at_ms`.
    /// What [`Arena::stage_refusal`] stages to a pilot and what a mind's
    /// landing appends to its own buffer (D-UM-8), so the two are one shape.
    ///
    /// `at_ms` is **the millisecond the order was refused at**, because that
    /// is the only moment the numbers are true: `now_ms` for phase 2 and phase
    /// 4, which refuse at the pulse's end, and the landing's own millisecond
    /// for a mind, whose `resolve_fire` refused `Cycling` there — read at the
    /// substep's end instead, a gun ready inside the substep would be told as
    /// cycling for `0` seconds.
    pub(crate) fn refusal_notice(
        &self,
        unit: ObjId,
        order: &'static str,
        refusal: Refusal,
        weapon: Option<SmolStr>,
        at_ms: i64,
    ) -> FrameNotice {
        let detail = match refusal {
            Refusal::Cycling => {
                let secs = weapon
                    .as_ref()
                    .and_then(|weapon| {
                        self.unit(unit)
                            .map(|state| state.weapon_ready_in_ms(weapon, at_ms))
                    })
                    .unwrap_or(0);
                RefusalDetail::Cycling {
                    secs: secs.saturating_add(999) / 1_000,
                }
            }
            Refusal::Overheated => match self
                .unit(unit)
                .and_then(|state| state.heat_now().zip(state.heat_max()))
            {
                Some((heat, max)) => RefusalDetail::Overheated { heat, max },
                None => RefusalDetail::None,
            },
            // **17b cut M1b.1** (doc 17 §5, D-UM-17 as amended): the floor the
            // arena captured at check-out, so the sentence names the floor the
            // condition was held to. Filled here for EVERY `Unlawful`; an
            // arming's refusal goes through `Arena::arming_refusal_notice`,
            // which clears it back to `None` when the odds term met the floor
            // (the refusal was for the target kind, a `Contact`), so that
            // `refusal_line` says the fallback sentence that names no floor.
            // An arming's landing refusal is staged through that function,
            // never through `stage_refusal` directly (phase 2 tells an arming
            // only `wreck` and `queue_full`, which carry no floor).
            Refusal::Unlawful => RefusalDetail::Floor {
                permille: self.odds_floor_permille,
            },
            _ => RefusalDetail::None,
        };
        FrameNotice::Refused {
            order: SmolStr::new_static(order),
            refusal,
            weapon,
            detail,
        }
    }
}
