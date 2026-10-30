//! Sensor pass notice projection, contact loss hysteresis, and scope/scan-table assembly.

use super::*;

impl Arena {
    /// Settle every held loss in `due` in `(t_ms, kind, viewer, contact)` order
    /// and leave it empty (16b cut M.2) — the one ordering both of the staging
    /// scan's sources of expiries are put through.
    pub(super) fn settle_held_losses(&mut self, due: &mut Vec<(i64, u8, ObjId, ContactId)>) {
        due.sort_unstable();
        for (at, _, viewer, contact) in due.drain(..) {
            self.settle_held_loss(viewer, contact, at);
        }
    }

    /// **Say one held loss whose hysteresis ran out** (16b cut M.2, 16c `T-83`;
    /// doc 16 §3.6 "What a pilot sees"), for the [`DueKind::LossSettles`] that
    /// came due at `at_ms`.
    ///
    /// Only when the entry is still there **with that deadline**: a loss the
    /// platform re-detected at or before it was taken out by the `detected` arm
    /// of `Arena::stage_pilot_notice`, and one lost again since carries a later
    /// deadline and its own event, so either way this event fires into nothing.
    /// A held loss whose platform has left the match is dropped rather than said:
    /// there is nobody it is a fact about any more (the rule the pulse-end sweep
    /// this replaced had).
    pub(super) fn settle_held_loss(&mut self, viewer: ObjId, contact: ContactId, at_ms: i64) {
        let Some((at, designator)) = self.held_losses.get(&(viewer, contact)).copied() else {
            return;
        };
        if at != at_ms {
            return;
        }
        self.held_losses.remove(&(viewer, contact));
        if self.unit(viewer).is_none() {
            return;
        }
        // 16b cut Q.1: the designator travels with the deadline, because the
        // contact row the name could have been read off went at the expiry.
        self.stage_notice(
            viewer,
            FrameNotice::Contact {
                kind: NoticeKind::Lost,
                contact: Named {
                    id: contact,
                    designator,
                },
                class: Classification::Unknown,
                was: None,
            },
        );
    }

    /// [`FrameNotice`]'s hysteresis (16b cut S.2; doc 16 §3.6 "What a pilot sees").
    ///
    /// Called with every event the pass journalled, so the pilot's lines are a
    /// projection of the black box's own record and the two cannot disagree. Three
    /// arms and one rule each:
    ///
    /// * `detected` — an acquisition, **unless** a held loss for the same
    ///   `(platform, contact)` is still inside its window, in which case the
    ///   contact merely flickered: the held loss is cancelled and nothing is said
    ///   — unless the row came back under another name, which is said as a
    ///   regression (16b cut D.4) or, for a mesh insert that arrived told apart,
    ///   as an identification (16b cut M.2).
    /// * `classified` — always said; being told apart is not something that
    ///   flickers (the pass keeps a classification across a gap by design).
    /// * `lost` — held for one further pass rather than said, which is the whole
    ///   hysteresis; a `DueKind::LossSettles` at the deadline is what eventually
    ///   says it (16b cut M.2 — a sweep at the end of every pulse said it until
    ///   then, which made the end of the window a pulse boundary).
    ///
    /// Answers `Some((deadline, viewer, contact))` for a `lost` whose window
    /// already closes at or before `now_ms` — only a pass shorter than the
    /// substep does that — so the staging scan settles it itself rather than a
    /// due event in the past; `None` for everything else.
    pub(super) fn stage_pilot_notice(
        &mut self,
        t_ms: i64,
        kind: &EventKind,
        in_flight: &mut Option<(ObjId, ObjId)>,
        launch_cell: Option<(u16, u16)>,
    ) -> Option<(i64, ObjId, ContactId)> {
        let window_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        match *kind {
            EventKind::Detected {
                viewer, contact, ..
            } => {
                if let Some((_, was)) = self.held_losses.remove(&(viewer, contact)) {
                    // **16b cut D.4** (`B-20`; 16c §5 Q16). A loss cancelled
                    // inside the hysteresis window is still silent when the row
                    // comes back under the **same** name — an `X2` that flickered
                    // behind a treeline, or any contact under
                    // `designate_on_detection` — which is cut Q.1's rule intact.
                    //
                    // It is **not** silent when the name moved. A contact that
                    // had been told apart comes back `Unknown`, so its row reads
                    // `X2` where it read `BA`, and `LOCK BA` answers
                    // `No such contact.` for three seconds with nothing to say
                    // why. That is a regression the pilot has to be told about,
                    // and it is one line.
                    //
                    // The old name is quoted and never learned: `resolve_designator`
                    // is untouched, because accepting `BA` for this row would say
                    // BA is still out there — which is exactly what doc 16 §3.6's
                    // one byte-identical refusal exists to hide (16c §6).
                    //
                    // The test is **the row's own class** and not the name alone,
                    // because the name can move the other way: `Arena::share_pass`
                    // journals a `detected` for a mesh insert and that row arrives
                    // `Classified`, so a hold lost as `X2` and handed back over the
                    // mesh inside the window reads `BA` here. That is a contact
                    // becoming *more* identified, and saying
                    // `X2 is unidentified again: BA.` about it would be the
                    // sentence backwards. A regression is a row that is `Unknown`
                    // now, which also makes the notice's own `class` true by
                    // construction.
                    let named = self.name_of(viewer, contact);
                    let regressed = self
                        .contacts
                        .resolve(viewer, contact)
                        .is_some_and(|held| held.class == Classification::Unknown);
                    // **16b cut M.2** (16c `B-74`) says the third case, which
                    // D.4's rule left silent: the name moved and the row is NOT
                    // `Unknown` — a hold lost as `X1` and handed back over the
                    // mesh inside the window, already told apart, so it reads
                    // `BA`. Before D.4 the pilot read `Contact X1 identified: BA,
                    // blue.` there and after it nothing, so a row renamed itself
                    // under the pilot's eyes. It is the `classified` notice with
                    // the old name, exactly as a share that classifies a
                    // still-held row says it, with the class read off the same
                    // `resolve`. No new state: the name and the class were both
                    // already in hand.
                    //
                    // **Only a mesh insert that arrived told apart**, which is
                    // 16c `B-74`'s own case, read off the row the scan sees at
                    // the end of the substep: `numbered` false (the row came over
                    // the mesh, not from this platform's own pass) and
                    // `Classified`. Anything else stays silent here, because
                    // anything else is said elsewhere or is not true: a row this
                    // platform's own pass re-opened (`numbered`) and a share then
                    // classified later in the same substep is said by that
                    // share's own `classified` line, with this very `was`, and
                    // saying it here too would be the sentence twice; and a row
                    // that is already gone again by the scan (a stale mesh copy
                    // the viewer's own pass expired) has no class to announce and
                    // would read `identified: X0, unknown`.
                    let identified = self
                        .contacts
                        .resolve(viewer, contact)
                        .filter(|held| {
                            !held.numbered
                                && matches!(held.class, Classification::Classified { .. })
                        })
                        .map(|held| held.class);
                    if regressed && named.designator != was {
                        self.stage_notice(
                            viewer,
                            FrameNotice::Contact {
                                kind: NoticeKind::Unidentified,
                                contact: named,
                                class: Classification::Unknown,
                                was: Some(was),
                            },
                        );
                    } else if let Some(class) = identified
                        && named.designator != was
                    {
                        self.stage_notice(
                            viewer,
                            FrameNotice::Contact {
                                kind: NoticeKind::Classified,
                                contact: named,
                                class,
                                was: Some(was),
                            },
                        );
                    }
                    return None;
                }
                self.stage_notice(
                    viewer,
                    FrameNotice::Contact {
                        kind: NoticeKind::Acquired,
                        contact: self.name_of(viewer, contact),
                        // **16b cut D.4** (`T-19`): the class the row actually
                        // carries, which is `Classified` for a mesh arrival —
                        // the acquisition and the classification are one moment
                        // there, and the notice used to say `Unknown` about a row
                        // the same substep had already told apart. `notice_line`
                        // does not read it; the notice object's consumers may.
                        class: self
                            .contacts
                            .resolve(viewer, contact)
                            .map_or(Classification::Unknown, |held| held.class),
                        was: None,
                    },
                );
            }
            EventKind::Classified {
                viewer,
                contact,
                target,
                team,
            } => {
                // **16b cut Q.1**: what the row was called a moment ago, so the
                // rename is one sentence and not two rows a pilot has to match
                // up. `None` for a contact that was never provisional — a mesh
                // arrival, or a match run with `designate_on_detection` on — and
                // the renderer then says nothing at all, because the acquisition
                // already read `New contact RB: …`.
                //
                // **16b cut D.4** (`B-21`) fixed where "never provisional" is
                // read from. It was the **book**, which is permanent, so a target
                // this platform numbered in an EARLIER hold and now received over
                // the mesh — already classified — printed `New contact BA: …` and
                // at once `Contact X2 identified: BA, blue.`, a rename of a row
                // the pilot is not looking at. It is read off **this hold**
                // (`ShadowContact::numbered`) now: a mesh insert says nothing
                // after its acquisition, and a share that classifies a row this
                // platform's own sensors opened still says the rename.
                let was = if self.sensors_cfg.designate_on_detection {
                    None
                } else if self
                    .contacts
                    .resolve(viewer, contact)
                    .is_some_and(|held| held.numbered)
                {
                    self.designators
                        .provisional(viewer, target)
                        .map(crate::arena::state::Designator::Provisional)
                } else {
                    None
                };
                self.stage_notice(
                    viewer,
                    FrameNotice::Contact {
                        kind: NoticeKind::Classified,
                        contact: self.name_of(viewer, contact),
                        class: Classification::Classified { party: team },
                        was,
                    },
                );
            }
            // **16b cut Q.1**: the far end of `STARTUP`, said to the platform's
            // own pilots and to nobody else.
            EventKind::PowerOnline { unit } => {
                self.stage_notice(unit, FrameNotice::PowerOnline);
            }
            // **16b cut D.4** (`B-22`): the platform's own receiver, said to its
            // own pilots and to nobody else. One sentence per transition.
            EventKind::Receiver { unit, degraded } => {
                self.stage_notice(unit, FrameNotice::Receiver { degraded });
            }
            // **16b cut F.2**, the four combat arms. Each is produced where its
            // event is produced, at that event's own arena timestamp, so the list
            // a scripted run gives a pilot is the same at every cadence.
            EventKind::Impact { unit, target, .. } => *in_flight = Some((unit, target)),
            EventKind::Hit {
                unit,
                target,
                shot_seq,
                ref section,
                ref facet,
                armor_left,
                internals_left,
                ..
            } => {
                // **16b cut D.2** (`B-15`): whether this round found a section
                // that was already destroyed. The scratch is filled by
                // `apply_hit` in this very substep and is drained here, so it
                // never survives the substep it was filled in
                // (`Arena::spent_hits`).
                let spent = self.spent_hits.remove(&shot_seq);
                if spent {
                    // Pruned **on touch** and against the event's own timestamp,
                    // never by a per-pulse sweep: a sweep would make the table a
                    // function of the cadence, which the cross-tempo property
                    // refuses.
                    self.spent_said
                        .retain(|_, at| t_ms.saturating_sub(*at) < SPENT_NOTICE_MS);
                }
                // The shooter sees the round land, and nothing about what it did:
                // a gunner reads the strike, not the target's internals.
                //
                // **16b cut Q.2** gave it the one word that is not the target's
                // internals: which section the round found. The gate is
                // `classified` and not merely `held`, because a section id is a
                // chassis vocabulary — see `FrameNotice::Hit`'s `section`.
                let held = self.named_target(unit, target);
                let classified = held.is_some() && self.target_classified(unit, target);
                let named_section = classified.then(|| section.clone());
                // **16b cut D.2**: the gunner who has told the target apart can
                // tell a spent section from a live one, so his line says so — and
                // is therefore rate-limited. The gunner who cannot is told
                // exactly what he was told before this cut, every round, with no
                // window and no entry: see `Arena::spent_said`.
                if !(spent && classified) {
                    self.stage_notice(
                        unit,
                        FrameNotice::Hit {
                            contact: held,
                            section: named_section,
                            spent: false,
                        },
                    );
                } else if self.say_spent(unit, target, section, t_ms) {
                    self.stage_notice(
                        unit,
                        FrameNotice::Hit {
                            contact: held,
                            section: named_section,
                            spent: true,
                        },
                    );
                }
                // The victim reads their own tank. **Nothing about the shooter** —
                // no id, no bearing, no team, no weapon: where a round came from
                // is a thing only a sensor may tell.
                //
                // **16b cut D.2**: a round on a section that is already gone has
                // no numbers left to report — both cells are nought — so it is
                // one short sentence under the same window, keyed on the victim's
                // own tank.
                if spent {
                    if self.say_spent(target, target, section, t_ms) {
                        self.stage_notice(
                            target,
                            FrameNotice::StruckSpent {
                                section: section.clone(),
                            },
                        );
                    }
                } else {
                    let (armor_section, armor_max, internals_max) =
                        self.struck_maxima(target, section, facet);
                    self.stage_notice(
                        target,
                        FrameNotice::Struck {
                            section: section.clone(),
                            armor_section,
                            facet: facet.clone(),
                            armor_left,
                            armor_max,
                            internals_left,
                            internals_max,
                        },
                    );
                }
                // **16b cut D.4** (`T-08`): the pair has resolved, so the scratch
                // is empty again. Nothing after this line in this substep may be
                // credited to this round unless its own event says so.
                *in_flight = None;
            }
            // **16b cut D.2** (`B-03`): the victim's own two lines, off the same
            // journalled events the gunner's kill line comes off, and staged
            // after it so the gunner's is untouched.
            EventKind::UnitDisabled { unit } => {
                self.stage_notice(unit, FrameNotice::OwnDisabled);
            }
            EventKind::Miss { ref reason, .. } => {
                if let Some((shooter, target)) = *in_flight {
                    let held = self.named_target(shooter, target);
                    // **16b cut AE.2**: the reason is the platform's only while
                    // it holds the target. `moved`, `blocked`, `cover`, `wreck`
                    // are facts about where the target is and what stands round
                    // it — a position and cover check the sensors have not
                    // earned once the contact is gone, which a round with a
                    // flight time makes an everyday case. So a miss on a target
                    // the shooter no longer holds carries no reason at all, and
                    // renders `Miss.` (`crate::match_::cockpit::notice_line`).
                    // `Hit.` on an unheld target stays: a round striking metal
                    // is the shooter's own physics, paid for with a round and a
                    // cycle, and it says nothing of where or why.
                    let reason = if held.is_some() {
                        reason.clone()
                    } else {
                        SmolStr::new_static("")
                    };
                    self.stage_notice(
                        shooter,
                        FrameNotice::Miss {
                            contact: held,
                            reason,
                            launch_cell,
                        },
                    );
                }
                // 16b cut D.4 (`T-08`): the pair has resolved, as on the `Hit`
                // arm above. This is the one arm that still reads the scratch.
                *in_flight = None;
            }
            EventKind::SectionDestroyed {
                unit,
                ref section,
                by,
                ..
            } => {
                // The consequences that **fired**, not the authored list the
                // ledger line carries: `disable_motion_when_all_of` is written
                // down on each of a biped's legs and does nothing until the
                // second one goes (16b cut X.2), and a pilot told their tank
                // cannot move while it is still walking would act on it.
                let consequences = self.fired_consequences(unit, section);
                self.stage_notice(
                    unit,
                    FrameNotice::SectionLost {
                        section: section.clone(),
                        consequences,
                    },
                );
                // **16b cut Q.2**: and the gunner's half of it, under the same
                // held-and-classified gate the section word on a hit is under,
                // and only when a round is what did it — a section lost to
                // anything else is nobody's shot.
                //
                // **16b cut D.4** (`T-08`): off the event's own `by` and not off
                // the substep's in-flight scratch, which by this line may be a
                // round that has already resolved. The write below is hashed
                // state, so a stale read of it is a state divergence and not only
                // a wrong line.
                if let Some(shooter) = by
                    && self.target_classified(shooter, unit)
                    && let Some(named) = self.named_target(shooter, unit)
                {
                    self.stage_notice(
                        shooter,
                        FrameNotice::TargetSectionLost {
                            contact: named,
                            section: section.clone(),
                        },
                    );
                    // The gunner's own eyes: a section this platform watched come
                    // apart is `destroyed` on its own reading from now on, so
                    // `SCAN` cannot contradict the line the pilot was just sent.
                    self.observe_section_lost(shooter, unit, section);
                }
            }
            // The kill line the shooter is owed, and only when a round is what did
            // it: a unit destroyed by anything else is nobody's shot.
            //
            // **16b cut D.4** (`T-08`): off the event's own `by`, for the reason
            // the section arm above reads it.
            EventKind::UnitDestroyed { unit, by } => {
                if let Some(shooter) = by {
                    let held = self.named_target(shooter, unit);
                    self.stage_notice(shooter, FrameNotice::Kill { contact: held });
                    // **16b cut E.2** (`B-45`; 16c §5's Q11): the shooter's own
                    // eyes, the way the section arm above writes the band its
                    // gunner watched go. The kill is knowledge this platform
                    // earned — it is the very fact the `Kill` line it was just
                    // sent carries — so its hold records it and
                    // `Arena::contact_reads_dead` lets its machine drop the
                    // target on the next tick. It is written **only** on the
                    // hold the shooter already has: a platform that killed
                    // something it never held learns nothing, because there is
                    // no row for the fact to live on.
                    //
                    // Nobody else's hold moves. A teammate that watched through
                    // its own sight has the destroyed band from the close look;
                    // one that did not keeps its track until the track expires,
                    // which is doc 16 §1's observer rule and not a courtesy.
                    if let Some(id) = self.contacts.id_of(shooter, unit)
                        && let Some(held) = self.contacts.get_mut(shooter, id)
                    {
                        held.known_dead = true;
                    }
                }
                // **16b cut D.2** (`B-03`), after the gunner's line: the pilot in
                // the wreck, who until this cut read nothing at all.
                self.stage_notice(unit, FrameNotice::OwnDestroyed);
            }
            EventKind::Lost {
                viewer,
                contact,
                target,
            } => {
                // **16b cut Q.1**: the row is already gone, so the name it read
                // as comes from the side table `sensor_pass` filled at the
                // expiry, and the entry is taken out here — this scan is what
                // drains it, so nothing the expiry put there survives the
                // substep. (It is *not* empty when this substep's hash is taken
                // a few lines above; it does not have to be, because the table
                // is not folded — see `Arena::lost_designators`.) The fallback
                // is the book's own number, which is what a `lost` produced by
                // anything that did not go through the expiry would honestly be
                // called.
                let named = Named {
                    id: contact,
                    designator: self.lost_designators.remove(&(viewer, contact)).unwrap_or(
                        crate::arena::state::Designator::Provisional(
                            self.designators.provisional(viewer, target).unwrap_or(0),
                        ),
                    ),
                };
                // **16b cut W.1**: a lock is *a lock on a contact the platform
                // holds* (doc 16 §3.5), so a contact that stops being held is a lock
                // that stops being one. This is the one place it can be done: the
                // `lost` event is what "this platform no longer holds this contact"
                // *is*, whatever produced it — the sensor pass's time-to-live now, a
                // wreck's cleared store later.
                //
                // Said at once, with none of the hysteresis below: see
                // `NoticeKind::LockLost`.
                if self
                    .unit(viewer)
                    .is_some_and(|state| state.lock == Some(contact))
                {
                    if let Some(state) = self.unit_mut(viewer) {
                        state.lock = None;
                    }
                    self.stage_notice(
                        viewer,
                        FrameNotice::Contact {
                            kind: NoticeKind::LockLost,
                            contact: named,
                            class: Classification::Unknown,
                            was: None,
                        },
                    );
                }
                // **16b cut M.2** (16c `T-83`): the end of the hysteresis is a due
                // event at the very deadline written into the table, so it is
                // said at an arena millisecond and not at whichever pulse
                // boundary came next. This runs in the staging scan, after the
                // substep's hash, so the entry and its event are first folded at
                // *k + 1*.
                //
                // A deadline at or before `now_ms` — a window shorter than the
                // substep it opened in — is not scheduled into the past: it is
                // handed back to the staging scan, which settles it at its place
                // in this same scan (see `Arena::substep`).
                let deadline = t_ms.saturating_add(window_ms);
                self.held_losses
                    .insert((viewer, contact), (deadline, named.designator));
                if deadline <= self.now_ms {
                    return Some((deadline, viewer, contact));
                }
                self.due
                    .schedule(deadline, DueKind::LossSettles { viewer, contact });
            }
            _ => {}
        }
        None
    }

    /// Put one notice in the platform's queue for this pulse.
    ///
    /// `pub(crate)` since 16b cut O.2, for
    /// [`crate::arena::intents::resolve_emcon`], which stages its own notice as
    /// it journals its own line.
    ///
    /// **17b cut M1a.5** (doc 17 §3, D-UM-8): a hull a mind flies keeps its own
    /// copy, in its bounded buffer (`crate::arena::mind::MindState::push_notice`,
    /// the oldest dropped past sixteen), drained into its next view. Every
    /// other hull costs one map miss.
    ///
    /// **17b cut M3.5** (doc 17 §12 "Release and possess"): only while the
    /// hull IS the mind's ([`Controller::Mind`]). A possess keeps the mind's
    /// row dormant (`Arena::suspend_mind`), and a dormant row is not pushed
    /// to: the pilot's notices — the refusals of their own verbs, their
    /// `drive_arrived`, the hits they take — never enter the mind's bounded
    /// buffer and are never drained by the first think after a release.
    pub(crate) fn stage_notice(&mut self, platform: ObjId, notice: FrameNotice) {
        // The map miss first: every hull with no row pays only that.
        let minded = self.minds.contains_key(&platform)
            && self
                .unit(platform)
                .is_some_and(|unit| unit.controller == Controller::Mind);
        if minded && let Some(state) = self.minds.get_mut(&platform) {
            state.push_notice(notice.clone());
        }
        self.pending_notices
            .entry(platform)
            .or_default()
            .push(notice);
    }

    /// **The pair one contact id is named by right now** (16b cut Q.1), for a
    /// notice staged while the row is still held.
    ///
    /// A row the platform no longer holds cannot happen on the arms that call it
    /// — a `detected` or a `classified` is staged in the same substep the store
    /// was written in — and answers `X0`, a name nothing resolves, rather than
    /// taking the arena down.
    pub(crate) fn name_of(&self, viewer: ObjId, contact: ContactId) -> Named {
        Named {
            id: contact,
            designator: self
                .contacts
                .get(viewer, contact)
                .map_or(crate::arena::state::Designator::Provisional(0), |held| {
                    self.designator_for(viewer, held)
                }),
        }
    }

    /// **The contact one platform holds a target under, named** (16b cut Q.1), or
    /// `None` when it holds none — which is what makes a shooter's line read
    /// `Hit.` rather than `Hit on BA.`
    pub(super) fn named_target(&self, viewer: ObjId, target: ObjId) -> Option<Named> {
        let id = self.contacts.id_of(viewer, target)?;
        Some(self.name_of(viewer, id))
    }

    /// **Whether this platform has told that target apart, right now** (16b cut
    /// Q.2) — the gate on every word a gunner is told about the target's hull.
    ///
    /// Read off the live contact and not off a flag, for the reason
    /// `Arena::designator_for` is: a contact is not a durable object, so a target
    /// that was classified, lost and re-detected is `Unknown` again and the
    /// section words stop until it has been told apart a second time.
    pub(super) fn target_classified(&self, viewer: ObjId, target: ObjId) -> bool {
        self.contacts
            .id_of(viewer, target)
            .and_then(|id| self.contacts.get(viewer, id))
            .is_some_and(|held| matches!(held.class, Classification::Classified { .. }))
    }

    /// **Write what the gunner watched happen into the reading they hold** (16b
    /// cut Q.2): that section's band becomes
    /// [`crate::arena::condition::ConditionBand::Destroyed`].
    ///
    /// The **stamp is untouched**, and so is `own_seen.condition`. The stamp is
    /// the age of the *look*, and this is not a look — it is one fact observed
    /// through a gunsight, folded into a picture that is otherwise as old as it
    /// says it is. Leaving `own_seen` alone is what keeps it off the mesh: what a
    /// teammate is handed is what this platform's sensors read, not what its
    /// gunner inferred from a fire.
    ///
    /// A platform with no reading is left with none: a gunner who has never been
    /// close enough to read the hull has not earned a picture by shooting at it.
    ///
    /// The index is the section's position in the **target's** own section order,
    /// which is the order every band on that contact is in.
    pub(super) fn observe_section_lost(&mut self, viewer: ObjId, target: ObjId, section: &str) {
        let Some(index) = self.unit(target).and_then(|state| {
            state
                .unit
                .sections()
                .position(|(id, _)| id.as_str() == section)
        }) else {
            return;
        };
        let Some(id) = self.contacts.id_of(viewer, target) else {
            return;
        };
        if let Some(held) = self.contacts.get_mut(viewer, id)
            && let Some(reading) = held.seen_condition.as_mut()
        {
            reading.condition = reading
                .condition
                .with_band(index, crate::arena::condition::ConditionBand::Destroyed);
        }
    }

    /// **What one platform holds, as its pilots' frame carries it** (16b cut S.1;
    /// doc 16 §3.6 "What a pilot sees"), in contact-id order.
    ///
    /// **16b cut F.3 made it a plot rather than a tether.** Bearing, range and
    /// cell are taken from the pose the target was in **at the sighting that last
    /// refreshed this contact** — this platform's own, or the sharer's own as the
    /// share carried it — and never from the target's live pose. Cut S.1 read the
    /// target live and justified it with "the contact was refreshed within the
    /// last `sensors.pass_ms` or it would not be here", which was false by a factor
    /// of `lost_after_passes`: a tank that ducked behind a building was tracked
    /// through it, cell by cell, for the whole time to live, by pilots and by the
    /// reflex controller alike.
    ///
    /// `age_ms` is the age of **that sighting** (`now − last_seen_ms`) and not the
    /// hold time since first detection, because what a pilot needs off this column
    /// is whether the plot in front of them is a live track or a dying one. While
    /// a target is in continuous view it is at most one `sensors.pass_ms` old.
    ///
    /// A contact whose target has left the match is dropped rather than reported:
    /// there is no unit for it to be a fact about.
    #[must_use]
    pub fn frame_contacts(&self, platform: ObjId) -> Vec<FrameContact> {
        self.frame_scope(platform).0
    }

    /// **The section vocabularies one platform's readings are in** (16b cut
    /// Q.2), the companion of [`Arena::frame_contacts`] and the other half of
    /// [`Frame::scan_tables`].
    ///
    /// The two are built by one walk (`Arena::frame_scope`); this is the
    /// caller that wants only the tables, exactly as `frame_contacts` is the one
    /// that wants only the rows.
    #[must_use]
    pub fn frame_scan_tables(&self, platform: ObjId) -> Vec<Vec<SmolStr>> {
        self.frame_scope(platform).1
    }

    /// **The scope and the section vocabularies its readings are in** (16b cut
    /// Q.2), built together because a [`FrameCondition`]'s `table` is an index
    /// into the list beside it and the two cannot be built apart without one of
    /// them being able to name a row the other does not have.
    ///
    /// A table is added by the **first** row that carries a reading in that
    /// chassis's vocabulary, and a second row on the same chassis re-uses the
    /// index — so a frame holding four stock tanks carries one table. A row with
    /// no reading contributes nothing, which is the whole of the rule
    /// [`Frame::scan_tables`] states.
    ///
    /// **17b cut M1a.3** (doc 17 §3, D-UM-5) adds three things per row, each
    /// read off the plot, the hold or the target chassis's static content and
    /// never off the target's live pose or wreck flag:
    ///
    /// * `aspect` — [`facet_band_seen`] off the row's plot and the platform's
    ///   own pose, on every hull;
    /// * `reads_dead` — on a hull **with a weapons station** only (`None`
    ///   without one): the own kill, or a held reading whose destroyed sections
    ///   meet the target chassis's fatal mask
    ///   ([`crate::arena::state::fatal_section_mask`] over its definition and
    ///   profile). Value-identical to [`Arena::contact_reads_dead`] wherever a
    ///   reading is held, and deliberately not a call of it;
    /// * `fatal` — that mask's section words, beside a held reading only.
    pub(super) fn frame_scope(&self, platform: ObjId) -> (Vec<FrameContact>, Vec<Vec<SmolStr>>) {
        let Some(from) = self.unit(platform) else {
            return (Vec::new(), Vec::new());
        };
        // §22's italic line: `reads_dead` is built only for a hull with a
        // weapons station.
        let armed = !from.weapons.is_empty();
        let mut tables: Vec<Vec<SmolStr>> = Vec::new();
        let mut rows: Vec<FrameContact> = Vec::new();
        for (id, held) in self.contacts.of(platform) {
            // A contact whose target has left the match is dropped: only the
            // unit's presence is read here, never its pose.
            let Some(at) = self.unit_at(held.target) else {
                continue;
            };
            let Some(target) = self.units.get(at) else {
                continue;
            };
            // **16c T-105**: the target chassis's static words — its body
            // mask, those sections' names and its section vocabulary — read
            // off the arena's table instead of recomputed for every row of
            // every frame. The table is the same two functions over the same
            // two inputs, laid down once per unit, so the frame's bytes are
            // unchanged; a row the table lacks (it never does: the table is
            // built over the unit table, which never moves) is computed here
            // as every row was before.
            let computed;
            let words = match self.chassis.get(at) {
                Some(words) => words,
                None => {
                    computed = crate::arena::state::ChassisWords::of(&target.unit, &target.profile);
                    &computed
                }
            };
            // **16b cut Q.2**: the reading, and the chassis vocabulary it is in —
            // resolved only for a row that has one, so a row with no reading
            // cannot put a section id on the frame at all.
            let condition = held.seen_condition.map(|reading| {
                let table = match tables.iter().position(|held| *held == words.section_names) {
                    Some(at) => at,
                    None => {
                        tables.push(words.section_names.clone());
                        tables.len().saturating_sub(1)
                    }
                };
                FrameCondition {
                    table: u8::try_from(table).unwrap_or(u8::MAX),
                    bands: reading.condition,
                    age_ms: self.now_ms.saturating_sub(reading.at_ms),
                }
            });
            // **17b cut M1a.3** (departure (j)): the target chassis's fatal
            // sections, off its definition and profile — the content its
            // `fatal_sections` is built from, never `limits.wreck`.
            let fatal_mask = words.fatal_mask;
            let reads_dead = armed.then(|| {
                held.known_dead
                    || held
                        .seen_condition
                        .is_some_and(|reading| reading.condition.destroyed_mask() & fatal_mask != 0)
            });
            // Beside a held reading only: the row already carries the chassis's
            // vocabulary there, so the words name nothing new.
            let fatal = held.seen_condition.map(|_| words.fatal_words.clone());
            rows.push(FrameContact {
                id: *id,
                // **16b cut Q.1**: read off the live contact, so a row that
                // has been re-acquired and not yet re-classified is `X<n>`
                // again, and the alias is what keeps a lock taken under that
                // number typable after the rename.
                designator: self.designator_for(platform, held),
                alias: self.alias_for(platform, held),
                class: held.class,
                moving: held.seen_at.speed_cms != 0,
                bearing_mdeg: held.seen_at.bearing_from(&from.pose),
                range_cm: held.seen_at.range_from(&from.pose),
                age_ms: self.now_ms.saturating_sub(held.last_seen_ms),
                cell: held.seen_at.cell(&self.definition.terrain),
                condition,
                // 17b cut M1a.3: the face the plot was showing to where this
                // platform stands now, and (doc 22 D-22-9) its eight-way
                // sprite facing.
                aspect: Aspect::from_band(facet_band_seen(&held.seen_at, from)),
                facing: ContactFacing::from_band(facing_band_seen(&held.seen_at, from)),
                reads_dead,
                fatal,
                // Doc 22 D-22-5: the type with the team, never before it.
                unit_type: held.class.party().map(|_| words.unit_type.clone()),
            });
        }
        (rows, tables)
    }
}
