//! Authoritative action providers (`SurfaceActProvider` and `SurfaceDoneProvider`).

use super::*;

fn selected_crew(
    parameters: &Map<String, Value>,
    tv: ObjId,
) -> Result<Option<Box<crate::detachment::sortie::SurfaceCrew>>, ActionError> {
    parameters
        .get("rigs")
        .and_then(|v| v.get(tv.0.to_string()))
        .and_then(|v| v.get("crew"))
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|_| invalid("invalid physical crew selection"))
        })
        .transpose()
        .map(Option::flatten)
}

// ---- tacsim_surface_act: the start ---------------------------------------------------------------

/// **`tacsim_surface_act`** — every act's start (the brief's 3.4; doc 20 cut SM.4a adds the scan
/// and the soundings, and retires SM.3b's test-world probe).
pub(crate) struct SurfaceActProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
    /// Doc 20 cut SM.7: the pocket cursors (`tacsim.pockets`) a run locks and draws.
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
}

/// **The start's fixed plan of a reading act** (doc 20 cut SM.4a): its plan, with the body's
/// site entries for the body caps; a refused one when the rig has no readable battery or
/// scanner (a scan or a sounding spends).
pub(super) fn reading_fixed(
    w: &World,
    plan: Option<Result<ActPlan, Refusal>>,
    has_battery: bool,
) -> FixedPlan {
    let (Some(plan), true) = (plan, has_battery) else {
        return FixedPlan::refused("tv_dark");
    };
    let body = plan.as_ref().ok().and_then(|plan| match &plan.params {
        ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => Some(body.clone()),
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => None,
    });
    let mut fixed = FixedPlan::of(plan);
    if let Some(body) = body {
        fixed.body_sites = chart::body_sites(w, &body);
    }
    fixed
}

impl ActionProvider for SurfaceActProvider {
    fn resolved_revision_keys(
        &self,
        resolved: &mush_core::ResolvedActionInvocation,
    ) -> Result<Vec<mush_core::RevisionKey>, ActionError> {
        let actor = resolved.invocation().actor();
        let parameters = parameters_of(resolved.invocation())?;
        let tv = tv_of(parameters)?;
        let crew = selected_crew(parameters, tv)?;
        if crew.is_some() {
            Ok(vec![
                crate::detachment::sortie::surface::revision_key(actor, tv)?,
                mush_org::unit_authority::operation::revision_key(
                    actor,
                    tv,
                    mush_org::unit_authority::UnitCapability::OperateCrew,
                )?,
            ])
        } else {
            Ok(vec![crate::detachment::sortie::availability_key(tv)?])
        }
    }
    fn resolved_authority_witnesses(
        &self,
        r: &mush_core::ResolvedActionInvocation,
        _engine: &mush_core::Engine,
    ) -> Result<Vec<mush_core::AuthorityWitnessRequest>, ActionError> {
        let params = parameters_of(r.invocation())?;
        let unit = tv_of(params)?;
        if selected_crew(params, unit)?.is_none() {
            return Ok(Vec::new());
        }
        Ok(vec![mush_core::AuthorityWitnessRequest::ControlLease {
            provider: mush_core::OwnerId::new("sim")
                .map_err(|_| invalid("control owner unavailable"))?,
            subject: unit,
            channel: mush_core::ChannelId::new("helm")
                .ok_or_else(|| invalid("helm unavailable"))?,
            holder: mush_core::PrincipalId::Object {
                id: r.invocation().actor(),
            },
        }])
    }
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters_of(invocation)?;
        crate::detachment::sortie::work::sequence(parameters)?;
        tv_of(parameters)?;
        ordinal_of(parameters)?;
        act_param(parameters)?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        let actor = query.actor();
        let mut rigs = operator_rigs(w, actor);
        if let Some(physical) = crate::detachment::sortie::surface::rig_in(engine, actor) {
            rigs.retain(|rig| rig.tv != physical.tv);
            rigs.push(physical);
        }
        // The lead's pre-commit round, item 3: the actor's leased rigs off the helm, offered only
        // their plan's variants below.
        let mut off_helm: Vec<RigAt> = leased_rigs(w, actor)
            .into_iter()
            .filter(|rig| rigs.iter().all(|at| at.tv != rig.tv))
            .collect();
        rigs.retain(|rig| {
            rig.crew.is_some() || crate::detachment::sortie::offer_available(engine, rig.tv)
        });
        off_helm.retain(|rig| crate::detachment::sortie::offer_available(engine, rig.tv));
        if rigs.is_empty() && off_helm.is_empty() {
            return Ok(None);
        }
        // Doc 20 cut SM.4a (the brief's reviewer finding F5): each site's composite is peeked
        // once for the whole projection and shared by every rig's scan plan — and since cut
        // SM.4b each site's home field too. The offer never wants a cold one (it cannot
        // mutate): a cold peek is refused `ground_cold`, as the command's is.
        let mut peeks = Peeks::default();
        let mut fixed_rigs: BTreeMap<String, FixedRig> = BTreeMap::new();
        for rig in &rigs {
            let battery_id = rig_battery_id(w, rig.tv);
            let scanner = rig_scanner_id(w, rig.tv).and_then(|id| scanner_of(&self.tools, &id));
            let battery = battery_id
                .as_ref()
                .and_then(|id| battery_of(&self.tools, id));
            let rates = rig_rates(w, rig.tv, battery);
            let mut plans: BTreeMap<String, FixedPlan> = BTreeMap::new();
            // A launch, a scan and a sounding need the battery (`needs_battery`: a launch
            // always, a reading act spends); a recover and a dump never do, so they plan whether
            // or not its row resolves (§4.1; the fixer round's item 3).
            let has_battery = battery_id.is_some();
            plans.insert(
                wire_word(ActKind::Launch, false).to_owned(),
                if has_battery {
                    FixedPlan::of(launch_plan(w, actor, rig))
                } else {
                    FixedPlan::refused("tv_dark")
                },
            );
            // Doc 20 cut SM.4b: a far recover plans its leg home the same way (a descent of the
            // warm home field, O(path), no search).
            plans.insert(
                wire_word(ActKind::Recover, false).to_owned(),
                FixedPlan::of(recover_plan(w, actor, rig, &mut peeks, rates)),
            );
            // Doc 20 cut SM.4b (the brief's decision P2): the drive's order, fixed.
            plans.insert(
                wire_word(ActKind::Drive, false).to_owned(),
                if has_battery {
                    drive_fixed(w, actor, rig)
                } else {
                    FixedPlan::refused("tv_dark")
                },
            );
            let mut dump = FixedPlan::of(dump_plan(w, actor, rig));
            if dump.ok {
                dump.goods = tv::hopper_goods_of(w, rig.tv)
                    .map(|(goods, _)| goods)
                    .unwrap_or_default();
                if dump.goods.is_empty() {
                    dump = FixedPlan::refused("tv_dark");
                }
            }
            plans.insert(wire_word(ActKind::Dump, false).to_owned(), dump);
            let scan = scanner
                .map(|scanner| scan_plan(w, actor, rig, scanner, &mut peeks, rates.as_ref()));
            plans.insert(
                wire_word(ActKind::Scan, false).to_owned(),
                reading_fixed(w, scan, has_battery),
            );
            for quiet in [false, true] {
                let sound = scanner.map(|scanner| {
                    sound_plan(w, actor, rig, scanner, quiet, &mut peeks, rates.as_ref())
                });
                plans.insert(
                    wire_word(ActKind::Sound, quiet).to_owned(),
                    reading_fixed(w, sound, has_battery),
                );
            }
            // Doc 20 cut SM.7 (the brief's 3.4, decision R10): the run's order, re-planned and
            // required equal; `mine_stop` is the prepare's to decide.
            let drill = rig_drill_id(w, rig.tv).and_then(|id| drill_of(&self.tools, &id));
            // The rig's next ordinal, from its record (absent: 1); unreadable → `None`, and the
            // re-plan refuses as the record's read does.
            let next_ordinal = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .map(|found| found.map_or(1, |record| record.ordinal.saturating_add(1)));
            plans.insert(
                wire_word(ActKind::Mine, false).to_owned(),
                mine_fixed(
                    engine,
                    actor,
                    rig,
                    drill,
                    has_battery,
                    rates.as_ref(),
                    &mut peeks,
                    next_ordinal,
                ),
            );
            plans.insert(
                "mine_stop".to_owned(),
                FixedPlan {
                    ok: true,
                    ..FixedPlan::default()
                },
            );
            // Doc 20 cut SM.8 (the brief's 3.7): the unload's two sources, fixed; the prepare
            // picks by the start's `how`.
            plans.insert(
                wire_word(ActKind::Unload, false).to_owned(),
                unload_fixed(engine, actor, rig),
            );
            // Doc 20 cut SM.11 (the brief's 3.6): the plan's variants, each under its own key so
            // it never meets a hand variant — the filing (the RAM order, re-compiled and required
            // equal), the plan's next move (the interpreter over the durable record, only for a
            // rig whose record holds a live plan: no plan, no cost), the clear and the strike.
            let record: Option<RigRecord> = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .flatten()
                .cloned();
            plans.insert(
                "plan_file".to_owned(),
                self.fill_fixed(engine, actor, rig, record.as_ref(), &mut peeks),
            );
            plans.extend(self.plan_variants(
                engine,
                actor,
                rig,
                record.as_ref(),
                &mut peeks,
                has_battery,
                true,
            ));
            fixed_rigs.insert(
                rig.tv.0.to_string(),
                FixedRig {
                    crew: rig.crew.clone(),
                    slot: rig.slot,
                    hull: rig.hull,
                    battery_id,
                    plans,
                },
            );
        }
        // The lead's pre-commit round, item 3 (the judge's correctness/F5; D22 "stays
        // clearable"): the actor's own leased rigs off the helm whose record holds the actor's own
        // plan — the plan's variants only: its next move (off the helm the interpreter answers a
        // wait, or past the bound the `Bound` end, which the tick submits as the author wherever
        // they stand) and the clear and the strike, while the author is aboard the rig's hull.
        for rig in &off_helm {
            let record: Option<RigRecord> = engine
                .action_state()
                .get(&self.excursions, &RigKey(rig.tv))
                .ok()
                .flatten()
                .cloned();
            if !record
                .as_ref()
                .and_then(|record| record.plan.as_ref())
                .is_some_and(|plan| plan.author == actor)
            {
                continue;
            }
            let battery_id = rig_battery_id(w, rig.tv);
            let has_battery = battery_id.is_some();
            let aboard = crate::port::voyage::aboard(w, rig.hull, actor);
            let plans: BTreeMap<String, FixedPlan> = self
                .plan_variants(
                    engine,
                    actor,
                    rig,
                    record.as_ref(),
                    &mut peeks,
                    has_battery,
                    aboard,
                )
                .into_iter()
                .collect();
            fixed_rigs.insert(
                rig.tv.0.to_string(),
                FixedRig {
                    crew: rig.crew.clone(),
                    slot: rig.slot,
                    hull: rig.hull,
                    battery_id,
                    plans,
                },
            );
        }
        if fixed_rigs.is_empty() {
            return Ok(None);
        }
        let world: Vec<u32> = world_rigs(w).iter().map(|tv| tv.0).collect();
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = json!({
            "now": w.world_ic_now().at,
            "rigs": serde_json::to_value(&fixed_rigs)
                .map_err(|_| integrity("the rigs' plans do not encode"))?,
            "world": world,
        });
        let parameters = vec![
            parameter(
                "tv",
                true,
                ProjectedParameterKind::Choice {
                    // Every rig the offer fixes: the helm's and, the lead's pre-commit round
                    // item 3, the actor's own off it.
                    choices: rigs
                        .iter()
                        .chain(off_helm.iter())
                        .map(|rig| rig.tv.0.to_string())
                        .filter(|tv| fixed_rigs.contains_key(tv))
                        .collect(),
                },
            )?,
            parameter(
                "act",
                true,
                ProjectedParameterKind::Choice {
                    choices: ACT_WIRE_WORDS.iter().map(|w| (*w).to_owned()).collect(),
                },
            )?,
            parameter(
                "field_order",
                false,
                ProjectedParameterKind::Text { maximum_bytes: 20 },
            )?,
            parameter(
                "ordinal",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            // Doc 20 cut SM.4b (the review's correctness/F5): a drive's planned end cell, which
            // the prepare holds to the order the offer fixed.
            parameter(
                "to",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: 65_535,
                },
            )?,
            // Doc 20 cut SM.8 (the brief's 3.7, decision R15; the drive's `to` precedent): how an
            // unload was planned, and a part unload's tonnes — the prepare holds the start to the
            // variant they name.
            parameter(
                "how",
                false,
                ProjectedParameterKind::Choice {
                    choices: ["part", "queued", "whole"]
                        .iter()
                        .map(|w| (*w).to_owned())
                        .collect(),
                },
            )?,
            parameter(
                "qty",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::try_from(tv::MAX_TYPED_TONNES).unwrap_or(99),
                },
            )?,
            // Doc 20 cut SM.11 (the brief's 3.6): who set the act going, and a plan step's plan
            // and step — server-authored by the surface tick; the prepare holds them to the
            // offer's plan variant and refuses any other combination.
            parameter(
                "by",
                false,
                ProjectedParameterKind::Choice {
                    choices: ["idle", "plan"].iter().map(|w| (*w).to_owned()).collect(),
                },
            )?,
            parameter(
                "plan",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            parameter(
                "step",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::try_from(MAX_PLAN_STEPS).unwrap_or(64),
                },
            )?,
            // Doc 17h cut M8e E4 (doc 23 S.4a): a launch's mind sealed aboard a cored rig — the
            // operator's `mind=` word (`filed:<n>` or a draft) and, for an organisation's
            // licence, the organisation the operator acts for (M8c B3).
            parameter(
                "mind",
                false,
                ProjectedParameterKind::Text {
                    maximum_bytes: crate::surface::dispatch::MAX_MIND_WORD_BYTES,
                },
            )?,
            parameter(
                "org",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 0,
                    maximum: i64::MAX,
                },
            )?,
        ];
        offer_draft(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-surface-act",
                "Work a rig",
                "Send a rig down the ramp, scan, sound or mine the ground, bring it in, unload its \
                 hopper into the hold, or empty it onto the spoil.",
            ),
        )
        .map(Some)
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare_start(invocation, txn, None)
    }

    /// **Doc 17h cut M8e E4**: the start takes its preparation context, through which a
    /// launch's `mind=` reads the Roll's holdings and seals over the rig's unit
    /// (`crate::surface::dispatch`); every other start ignores it.
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare_start(invocation, txn, Some(context))
    }
}

impl SurfaceActProvider {
    fn task_work(
        &self,
        context: Option<&mush_core::PrepareCtx<'_>>,
        txn: &mut ActionTxn<'_>,
        invocation: &ActionInvocation,
        rig: &FixedRig,
        step: crate::detachment::sortie::FieldStep,
        plan: Option<&FixedPlan>,
    ) -> Result<Option<Box<crate::detachment::sortie::FieldWork>>, ActionError> {
        use crate::detachment::sortie::{FieldStep, work};
        let parameters = parameters_of(invocation)?;
        let sequence = work::sequence(parameters)?;
        let tv = tv_of(parameters)?;
        let ordinal = ordinal_of(parameters)?;
        if sequence.is_none() {
            if step == FieldStep::Act(ActKind::Unload)
                && how_of(parameters) == Some(UnloadHow::Queued)
                && let Some(crew) = rig.crew.as_deref()
                && let Some(record) = read_record(txn, &self.excursions, tv)?
                && let (Some(previous), Some(queue)) =
                    (record.field_work.as_deref(), record.queued.as_ref())
                && previous.step == FieldStep::Act(ActKind::Recover)
                && previous.ordinal == queue.after
                && previous.flight == crew.flight
                && previous.operation == crew.operation
                && queue.operator == invocation.actor()
            {
                let mut inherited = previous.clone();
                inherited.ordinal = ordinal;
                inherited.step = step;
                return Ok(Some(Box::new(inherited)));
            }
            return Ok(None);
        }
        let crew = rig
            .crew
            .as_deref()
            .ok_or_else(|| invalid("A field task needs its actual physical operator."))?;
        let record = read_record(txn, &self.excursions, tv)?;
        if step != FieldStep::Stop
            && record
                .as_ref()
                .is_some_and(|r| r.act.is_some() || r.run.is_some())
        {
            return Err(invalid("Earlier field work is still settling."));
        }
        if step == FieldStep::Stop
            && record
                .as_ref()
                .and_then(|r| r.run.as_ref())
                .is_none_or(|r| r.stop)
        {
            return Err(invalid("No new mining stop is needed."));
        }
        let actual = work::admit(
            context.ok_or_else(|| invalid("Field work needs composed preparation."))?,
            txn,
            work::WorkAdmission {
                actor: invocation.actor(),
                crew,
                sequence: sequence.ok_or_else(|| invalid("Field sequence unavailable."))?,
                ordinal,
                step,
                target: plan
                    .and_then(|p| p.params.as_ref())
                    .and_then(ActParams::leg)
                    .map(|(_, to)| to),
                family: plan.and_then(|p| p.mine.as_ref()).map(|m| m.family),
                previous: record.as_ref().and_then(|r| r.field_work.as_deref()),
            },
        )?;
        Ok(Some(Box::new(actual)))
    }
    fn adopt_field(
        &self,
        rig: &FixedRig,
        flight: Option<&crate::detachment::sortie::Flight>,
        record: &mut RigRecord,
    ) -> Result<(), ActionError> {
        if let (Some(crew), Some(flight)) = (&rig.crew, flight) {
            let battery = rig
                .battery_id
                .as_ref()
                .and_then(|id| battery_of(&self.tools, id));
            crate::detachment::sortie::surface::adopt_record(
                crew, flight, record, rig.slot, battery,
            )?;
        } else if rig.crew.is_none()
            && record.excursion.is_none()
            && record.act.is_none()
            && record.run.is_none()
        {
            record.field_crew = None;
            record.field_work = None;
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one bundle
    fn prepare_start(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: Option<&mut mush_core::PrepareCtx<'_>>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = parameters_of(invocation)?;
        let actor = invocation.actor();
        let tv = tv_of(parameters)?;
        // The composed production path supplies World. The older resolved
        // owner-only preparation deliberately does not; its engine still
        // validates the detachment availability revision above. Preserve that
        // existing ingress rather than adding a new World requirement to it.
        if selected_crew(parameters, tv)?.is_none()
            && let Some(context) = context.as_ref()
            && let Ok(world) = context.world()
        {
            crate::detachment::sortie::require_available(world, txn, tv)?;
        }
        let ordinal = ordinal_of(parameters)?;
        let now = now_of(parameters)?;
        let word = act_param(parameters)?;
        if crate::detachment::sortie::work::sequence(parameters)?.is_some()
            && !matches!(word, StartWord::Act { .. } | StartWord::MineStop)
        {
            return Err(invalid(
                "Field tasks bind individual work, not plan controls.",
            ));
        }
        if read_record(txn, &self.excursions, tv)?.is_some_and(|record| {
            record
                .recovery
                .as_ref()
                .is_some_and(|receipt| receipt.pending)
        }) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "staff recovery is pending; wait for the rig to reach its bay",
            ));
        }
        // Doc 17h cut M8e E4: `mind` and `org` belong to a launch's start alone, and `org` to a
        // `mind`.
        let mind = mind_param(parameters)?;
        let launching = matches!(
            word,
            StartWord::Act {
                kind: ActKind::Launch,
                ..
            }
        );
        if mind.is_some() && (!launching || by_word_of(parameters).is_some()) {
            return Err(invalid("a mind is named by a launch's own start"));
        }
        // Doc 20 cut SM.8 (decision R15): `how` and `qty` belong to an unload's start alone.
        let unloading = matches!(
            word,
            StartWord::Act {
                kind: ActKind::Unload,
                ..
            }
        );
        if !unloading && (parameters.contains_key("how") || parameters.contains_key("qty")) {
            return Err(invalid("an unload names how it was planned"));
        }
        // 1. The rig is the actor's and the act's fixed plan is ok.
        let rigs: BTreeMap<String, FixedRig> = parameters
            .get("rigs")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a surface act's offer fixes the rigs"))?;
        let Some(rig) = rigs.get(&tv.0.to_string()) else {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that rig is not yours to work",
            ));
        };
        let flight = if let Some(crew) = &rig.crew {
            if matches!(word, StartWord::PlanFile) {
                return Err(invalid(
                    "Field crews use individual drive, scan, sound, mine and recover orders.",
                ));
            }
            let context = context
                .as_ref()
                .ok_or_else(|| invalid("physical crew requires composed preparation"))?;
            Some(crate::detachment::sortie::surface::admit(
                context
                    .world()
                    .map_err(|_| invalid("physical crew world unavailable"))?,
                txn,
                actor,
                tv,
                crew,
                matches!(
                    word,
                    StartWord::Act {
                        kind: ActKind::Drive | ActKind::Recover,
                        ..
                    }
                ),
            )?)
        } else {
            None
        };
        if flight
            .as_ref()
            .is_some_and(|f| f.phase == crate::detachment::sortie::Phase::Returned)
            && !matches!(
                word,
                StartWord::Act {
                    kind: ActKind::Unload,
                    ..
                } | StartWord::PlanClear
                    | StartWord::PlanStrike
                    | StartWord::PlanEnd
            )
        {
            return Err(invalid(
                "Launch the next DT flight before starting more field work.",
            ));
        }
        let world: Vec<u32> = parameters
            .get("world")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a surface act's offer fixes the world's rigs"))?;
        // Doc 20 cut SM.11 (the brief's 3.6): who set it going, and a plan step's binding — the
        // parameter rule first.
        let by = by_params(word, parameters)?;
        // Doc 20 cut SM.11 (D8): a live plan holds its rig from every hand start — a hand unload
        // too, never the recover's queued one — and its stop.
        let held = read_record(txn, &self.excursions, tv)?
            .and_then(|record| record.live_plan_for(actor).map(|plan| plan.no));
        let hand = by.by.is_none();
        let queued_unload = matches!(
            word,
            StartWord::Act {
                kind: ActKind::Unload,
                ..
            }
        ) && how_of(parameters) == Some(UnloadHow::Queued);
        let control = matches!(
            word,
            StartWord::PlanFile | StartWord::PlanClear | StartWord::PlanStrike | StartWord::PlanEnd
        );
        if hand && !control && !queued_unload && held.is_some() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("plan_live"),
            ));
        }
        // Doc 20 cut SM.7: `mine_stop` is a control on a committed run, no act; doc 20 cut SM.11:
        // so are the plan's four.
        let (kind, quiet) = match word {
            StartWord::Act { kind, quiet } => (kind, quiet),
            StartWord::MineStop => {
                let work = self.task_work(
                    context.as_deref(),
                    txn,
                    invocation,
                    rig,
                    crate::detachment::sortie::FieldStep::Stop,
                    None,
                )?;
                return self.stop_run(invocation, txn, (actor, tv, ordinal), work);
            }
            StartWord::PlanFile => {
                return self.file_plan(
                    invocation,
                    txn,
                    (actor, tv, ordinal, now),
                    rig,
                    flight.as_ref(),
                );
            }
            StartWord::PlanClear | StartWord::PlanStrike => {
                return self.clear_plan(
                    invocation,
                    txn,
                    (actor, tv, now),
                    rig,
                    word == StartWord::PlanStrike,
                    by.plan,
                );
            }
            StartWord::PlanEnd => {
                return self.end_plan(invocation, txn, (actor, tv, now), rig, &by);
            }
        };
        let wire = wire_word(kind, quiet);
        let (plan, stamp) = match by.by {
            None => (
                rig.plans
                    .get(wire)
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?,
                Stamp::default(),
            ),
            // A plan's step: the offer's plan variant, held to the start's plan, step and word.
            Some(ByWord::Plan) => {
                let fixed = rig
                    .plans
                    .get("plan")
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?;
                if !fixed.ok {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        code_of_word(fixed.code.as_deref().unwrap_or("plan_moved")),
                    ));
                }
                let binding = fixed.plan.clone().ok_or_else(moved_on)?;
                let live = read_record(txn, &self.excursions, tv)?
                    .and_then(|record| record.live_plan_for(actor).map(|plan| (plan.no, plan.seq)));
                if Some(binding.no) != by.plan
                    || Some(binding.step) != by.step
                    || binding.word != wire
                    || live != Some((binding.no, binding.seq))
                {
                    return Err(moved_on());
                }
                let step = binding.step;
                (
                    fixed,
                    Stamp {
                        by: Some(ActBy::Plan {
                            no: binding.no,
                            step,
                        }),
                        plan: Some(binding),
                    },
                )
            }
            // The idle return: the recover's own plan, the idle condition checked on the record.
            Some(ByWord::Idle) => {
                let fixed = rig
                    .plans
                    .get(wire)
                    .cloned()
                    .ok_or_else(|| invalid("that act is not planned"))?;
                let record = read_record(txn, &self.excursions, tv)?;
                let idle = record.as_ref().is_some_and(|record| {
                    record.live_plan_for(actor).is_none()
                        && record.act.is_none()
                        && record.run.is_none()
                        && record.queued.is_none()
                        && record.excursion.is_some()
                        && record.idle_since().is_some_and(|since| {
                            since
                                .checked_add(crate::surface::plan::IDLE_PARK_IC_SECS)
                                .is_some_and(|due| due <= now)
                        })
                });
                if !idle {
                    return Err(port_error(
                        ActionErrorCode::Conflict,
                        "that rig is not idle",
                    ));
                }
                (
                    fixed,
                    Stamp {
                        by: Some(ActBy::Idle),
                        plan: None,
                    },
                )
            }
        };
        if !plan.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(plan.code.as_deref().unwrap_or("tv_dark")),
            ));
        }
        let work = self.task_work(
            context.as_deref(),
            txn,
            invocation,
            rig,
            crate::detachment::sortie::FieldStep::Act(kind),
            Some(&plan),
        )?;
        // Doc 20 cut SM.11: nothing in flight for a plan's step (SM.7/SM.8's idleness rule).
        if stamp.plan.is_some()
            && read_record(txn, &self.excursions, tv)?.is_some_and(|record| {
                record.act.is_some() || record.run.is_some() || record.queued.is_some()
            })
        {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        // Doc 20 cut SM.7: a run starts on its own path (one start action, SM.3b's decision 25).
        if kind == ActKind::Mine {
            return self.start_run(
                invocation,
                txn,
                (actor, tv, ordinal, now),
                rig,
                &plan,
                &world,
                &stamp,
                flight.as_ref(),
                work,
            );
        }
        // Doc 20 cut SM.8: an unload starts on its own path too.
        if kind == ActKind::Unload {
            return self.start_unload(
                invocation,
                txn,
                (actor, tv, ordinal, now),
                rig,
                &plan,
                flight.as_ref(),
                work,
            );
        }
        self.start_act(
            invocation,
            txn,
            (actor, tv, ordinal, now),
            (kind, quiet),
            rig,
            &plan,
            &world,
            &stamp,
            flight.as_ref(),
            work,
            (mind.as_ref().map(|(word, org)| (word, *org)), context),
        )
    }
}

/// **A launch's `mind` and `org` parameters** (doc 17h cut M8e E4): the word parsed
/// ([`crate::surface::dispatch::MindWord::parse`]) and the organisation; `org` without
/// `mind`, or a word that does not parse, is invalid.
fn mind_param(
    parameters: &Map<String, Value>,
) -> Result<Option<(crate::surface::dispatch::MindWord, Option<mush_org::OrgId>)>, ActionError> {
    let org = match parameters.get("org") {
        None => None,
        Some(value) => Some(mush_org::OrgId(
            value
                .as_u64()
                .ok_or_else(|| invalid("an organisation is its number"))?,
        )),
    };
    match parameters.get("mind") {
        None if org.is_some() => Err(invalid("an organisation names a mind's licence")),
        None => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(crate::surface::dispatch::MindWord::parse)
            .map(|word| Some((word, org)))
            .ok_or_else(|| invalid("a mind is filed:<n> or a draft's name")),
    }
}

/// Whether the start names a `by` (a plan's step or the idle return): never a hand's `mind`.
fn by_word_of(parameters: &Map<String, Value>) -> Option<&Value> {
    parameters.get("by")
}

impl SurfaceActProvider {
    /// Stage a non-mining, non-unloading act after its fixed plan and control binding
    /// have been checked. Keep the transactional record work separate from dispatch
    /// so the two paths do not share one large stack frame.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // one ordered list, one bundle
    pub(super) fn start_act(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        (kind, quiet): (ActKind, bool),
        rig: &FixedRig,
        plan: &FixedPlan,
        world: &[u32],
        stamp: &Stamp,
        flight: Option<&crate::detachment::sortie::Flight>,
        work: Option<Box<crate::detachment::sortie::FieldWork>>,
        (mind, context): (
            Option<(&crate::surface::dispatch::MindWord, Option<mush_org::OrgId>)>,
            Option<&mut mush_core::PrepareCtx<'_>>,
        ),
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = parameters_of(invocation)?;
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        // The battery, only where the act needs it (`needs_battery`; §4.1, the fixer round's
        // item 3): a recover or a dump starts with the row unresolvable.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        if battery.is_none() && needs_battery(kind, plan.cost_cu, plan.home_cu) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that rig cannot be read",
            ));
        }
        // 2. The record and the ordinal.
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| {
            battery.map_or_else(
                || RigRecord::fresh_unread(tv),
                |battery| RigRecord::fresh(tv, battery),
            )
        });
        self.adopt_field(rig, flight, &mut record)?;
        if record.ordinal.checked_add(1) != Some(ordinal) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        // Doc 20 cut SM.4b (the brief's decision P2): a drive's fixed plan is its order, bound
        // to the ordinal it was planned for — and (the review's correctness/F5) to the end cell
        // the start names, so a start whose order a later `go` replaced never runs the later
        // order's leg.
        // Doc 20 cut SM.11: a plan's drive is bound by its plan and step instead of `to`.
        if kind == ActKind::Drive
            && (plan.ordinal != Some(ordinal)
                || (stamp.plan.is_none()
                    && plan.params.as_ref().and_then(order_to_word) != to_word_of(parameters)))
        {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        // 3. One act at a time — and (doc 20 cut SM.7, decision R1) only a scan or a sounding
        // beside a run.
        if record.act.is_some() {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        if record.run.is_some() && !matches!(kind, ActKind::Scan | ActKind::Sound) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("mine_running"),
            ));
        }
        // 4. The launch's open excursion and the caps — a launch's, and a scan's, a sounding's
        // or (doc 20 cut SM.4b) a drive's that implies one (doc 20 cut SM.4a, lead ruling 2); a
        // recover from the bay closes one.
        let launches =
            kind == ActKind::Launch || plan.params.as_ref().is_some_and(ActParams::launches);
        if launches {
            if record.excursion.is_some() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "that rig has an excursion open",
                ));
            }
            // Doc 23 cut S.1 (the lead's L5): a rig whose last epoch is still open cannot
            // leave again — it cannot happen through the verbs (a rig launches from its bay
            // only, and the bay is reached only by a close), so it is a conflict, paged.
            if let Some(open) = &record.sortie {
                tracing::error!(
                    tv = tv.0,
                    sortie = %open.id,
                    abandoned = open.abandoned,
                    "tacsim: rig {} (tacsim.excursion {}) is asked to leave with its last epoch \
                     still open; the start is refused; staff `home` the rig to close it",
                    rig.slot,
                    tv.0
                );
                return Err(port_error(
                    ActionErrorCode::Conflict,
                    "that rig's last trip is still open; staff must bring it home first",
                ));
            }
            let mut counts = ExcursionCounts::default();
            for &other in world {
                if let Some(found) = read_record(txn, &self.excursions, ObjId(other))?
                    && let Some(excursion) = &found.excursion
                {
                    counts.world = counts.world.saturating_add(1);
                    if excursion.site == site {
                        counts.site = counts.site.saturating_add(1);
                    }
                    if excursion.operator == actor {
                        counts.operator = counts.operator.saturating_add(1);
                    }
                }
            }
            if let Some(cap) = cap_refusal(&counts) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    match cap {
                        CapRefused::Operator => "you have as many rigs out as one pilot may run",
                        CapRefused::Site => "that site has as many rigs out as it holds",
                        CapRefused::World => "the Registry has as many rigs out as it allows",
                    },
                ));
            }
        }
        if kind == ActKind::Recover && plan.from_bay && record.excursion.is_none() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "that rig is in the bay",
            ));
        }
        let mut params = plan
            .params
            .clone()
            .ok_or_else(|| invalid("an act's plan names its parameters"))?;
        // 5. The chart (doc 20 cut SM.4a; lead ruling 3, D4, D3): the same-cell rule from the
        // fixed plan's frozen stand, then the body caps over the actor's rows on the body. The
        // chart is not written by the start.
        if let Some(reach) = Reach::of(&params) {
            if plan.body_sites.len() > chart::MAX_CHART_SITES_PER_BODY {
                return Err(invalid(
                    "a reading act's offer names a bounded set of sites",
                ));
            }
            let row = read_chart(txn, &self.charts, actor, &site)?;
            if reach.refused(row.as_ref()) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(Refusal::NothingNew.word()),
                ));
            }
            let mut rows = Vec::new();
            for other in &plan.body_sites {
                if let Some(found) = read_chart(txn, &self.charts, actor, other)? {
                    rows.push(found);
                }
            }
            if let Some(row) = row
                && !plan.body_sites.contains(&site)
            {
                rows.push(row);
            }
            let (fields, blocks) = reach.units();
            if !body_admits(&rows, &site, &fields, &blocks) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(
                        Refusal::ChartFull {
                            body: String::new(),
                        }
                        .word(),
                    ),
                ));
            }
        }
        // 6. The reserve rule (§4.1; lead ruling 3): here and nowhere else, from the charge the
        // act will spend from (an implied launch: the bay credited no further than the launch
        // half, D22). With no battery the act spends nothing (`needs_battery` refused every
        // spend above), which the rule always passes. **Doc 20 cut SM.4b** (lead ruling 5,
        // decision P6): a recover is never held to the floor — it IS the way home — and a
        // recover with a leg must only afford its own leg, on the record's own value (no
        // battery read). SM.11's idle return-and-park must treat that refusal as "parked where
        // it stands".
        if kind == ActKind::Recover {
            if !leg_affordable(&record.charge, reserved_total(&record), plan.cost_cu) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("leg_short"),
                ));
            }
        } else if let Some(battery) = battery
            && reserve_check(
                charge_for_act(
                    &record.charge,
                    now,
                    plan.params.as_ref().is_some_and(ActParams::launches),
                    battery,
                ),
                reserved_total(&record),
                plan.cost_cu,
                plan.home_cu,
                battery,
            )
            .is_err()
        {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the rig's charge is under the reserve",
            ));
        }
        // 7. A dump freezes what the hopper holds now: each pool's **available** quantity
        // (its balance less every live hold), the one figure the finish compares and the
        // inventory's `transfer` can move — freezing the balance would refuse every dump of
        // a pool with a hold on it as `HopperChanged` (review finding tests/T4).
        if let ActParams::Dump { hopper, goods, .. } = &mut params {
            for good in &plan.goods {
                let held = self
                    .inventory
                    .pool(txn, EndpointRef(VaultId(*hopper)), good)
                    .map_err(|_| {
                        port_error(ActionErrorCode::Integrity, "the hopper cannot be read")
                    })?
                    .map_or(0, |pool| pool.available);
                if held > 0 {
                    goods.insert(good.clone(), held);
                }
            }
            if goods.is_empty() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "the hopper is empty",
                ));
            }
            if goods.len() > MAX_DUMP_GOODS {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "the hopper holds more goods than a dump freezes",
                ));
            }
        }
        // 8. The write: the act, its due instant and the charge it reserves; a launch (or the
        // launch a reading act implies) opens the excursion. The charge itself is not written
        // (lazy).
        let due_ic = now.checked_add(i64::from(plan.secs)).ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the act's due instant overflows",
            )
        })?;
        // 9. Doc 17h cut M8e E4 (doc 23 S.4a; D-UM-45): the mind sealed aboard at the dispatch,
        // on the composed path alone (a typed-only start has no holding reads, and refuses a
        // named mind rather than sealing the ROM in its place).
        let seal = match (mind, context) {
            (None, _) => None,
            (Some(_), None) => {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    crate::surface::dispatch::MIND_UNAVAILABLE,
                ));
            }
            (Some(word), Some(context)) => {
                crate::surface::dispatch::dispatch(context, txn, (actor, tv, now), Some(word))?
                    .map(Box::new)
            }
        };
        record.ordinal = ordinal;
        record.act = Some(Act {
            ordinal,
            kind,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site: site.clone(),
            started_ic: now,
            due_ic,
            reserved_cu: plan.cost_cu,
            params,
            by: stamp.by,
            seal,
        });
        // Doc 20 cut SM.8 (lead ruling 1): any start that lands supersedes a queued unload.
        record.queued = None;
        if launches {
            record.excursion = Some(Excursion {
                id: ExcursionId { tv, launch_ic: now },
                operator: actor,
                hull: rig.hull,
                slot: rig.slot,
                site,
            });
        }
        // Doc 20 cut SM.11: a plan step's update rides the same bundle.
        if let Some(binding) = &stamp.plan {
            apply_update(&mut record, binding, now)?;
        }
        if let Some(work) = work {
            record.field_work = Some(work);
        }
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": wire_word(kind, quiet),
                "actor": actor.0,
                "due_ic": due_ic,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// A pocket cursor read in the transaction, its key re-checked (the families rule).
pub(crate) fn read_cursor(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    deposit: mush_industry::OreDepositId,
) -> Result<Option<PocketCursor>, ActionError> {
    let found = txn
        .get(family, &deposit)
        .map_err(|_| {
            port_error(
                ActionErrorCode::Integrity,
                "the pocket cursors are unavailable",
            )
        })?
        .cloned();
    if let Some(cursor) = &found
        && cursor.deposit != deposit
    {
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a pocket cursor disagrees with its own key",
        ));
    }
    Ok(found)
}

/// **Stage a cursor's new state** (insert, replace or remove; `before` is whether one stood):
/// `None` removes it (an emptied cursor), and nothing is staged when none stood and none is due.
pub(crate) fn write_cursor(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    deposit: mush_industry::OreDepositId,
    before: bool,
    after: Option<PocketCursor>,
) -> Result<(), ActionError> {
    let integrity = || {
        port_error(
            ActionErrorCode::Integrity,
            "the pocket cursor did not stage",
        )
    };
    match (before, after) {
        (_, Some(cursor)) => {
            cursor.check().map_err(|_| integrity())?;
            if cursor.deposit != deposit {
                return Err(integrity());
            }
            let staged = if before {
                txn.replace(family, &deposit, cursor)
            } else {
                txn.insert(family, &deposit, cursor)
            };
            staged.map(|_| ()).map_err(|_| integrity())
        }
        (true, None) => txn
            .remove(family, &deposit)
            .map(|_| ())
            .map_err(|_| integrity()),
        (false, None) => Ok(()),
    }
}

/// **The pocket's occupancy, in the transaction** (doc 20 cut SM.7; lead ruling 3, decision R4):
/// another rig's live lock on any of `lock_rows` (`lock_live` over the holder's record read here,
/// matched by the row itself: the SM.7 lead's pre-commit round, item 1),
/// or another of the world's rigs whose run stands on `(site, pocket)` — the record check makes an
/// all-barren pocket, which has no row, refuse alike.
pub(super) fn pocket_taken(
    txn: &mut ActionTxn<'_>,
    (excursions, pockets): (
        &StateFamilyHandle<RigKey, RigRecord>,
        &StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    ),
    tv: ObjId,
    (site, pocket): (&CanonicalId, [i64; 2]),
    lock_rows: &[LockRow],
    world: &[u32],
) -> Result<bool, ActionError> {
    for row in lock_rows {
        let lock = read_cursor(txn, pockets, row.deposit)?
            .and_then(|cursor| cursor.row(row.pocket).and_then(|r| r.lock.clone()));
        if let Some(lock) = lock
            && lock.tv != tv
        {
            let holder = read_record(txn, excursions, lock.tv)?;
            if lock_live(&lock, holder.as_ref(), site, row) {
                return Ok(true);
            }
        }
    }
    for other in world
        .iter()
        .copied()
        .map(ObjId)
        .filter(|other| *other != tv)
    {
        if read_record(txn, excursions, other)?
            .and_then(|record| record.run)
            .is_some_and(|run| &run.site == site && run.pocket == pocket)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

impl SurfaceActProvider {
    /// **A run's start** (doc 20 cut SM.7; the brief's 3.4; lead rulings 1–6): the order's
    /// ordinal (`Conflict` "that rig's orders moved on"), the rig idle — no act, no run
    /// (`Conflict` "that rig is busy") — the licence's lapse after `now` (the offer re-read the
    /// standing at landing; `unlicensed`), the pocket free in the transaction (`locked`), the
    /// family's cursor row drawn exactly as the preview saw it (`Conflict` "that pocket changed;
    /// preview it again"), the hull's runs exactly as the hold's room counted them (`Conflict` "the
    /// hold's room changed; preview it again"), a blind core's body caps (`chart_full`), and the
    /// reserve over the run's whole cost and the trip home; then **one bundle**: the record's
    /// ordinal and its run (cycle 1, due `now + secs`, the whole reservation), each lock row
    /// written on its cursor. The lease, the tenure and the plan itself are the offer's re-plan at
    /// landing (`mine_fixed`: the prepare holds no world). The outcome is the start's own shape,
    /// identical for a barren and an ore blind core.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // one ordered list, one bundle
    pub(super) fn start_run(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
        plan: &FixedPlan,
        world: &[u32],
        stamp: &Stamp,
        flight: Option<&crate::detachment::sortie::Flight>,
        work: Option<Box<crate::detachment::sortie::FieldWork>>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let conflict = |message: &'static str| port_error(ActionErrorCode::Conflict, message);
        let rejected =
            |word: &str| port_error(ActionErrorCode::ProviderRejected, code_of_word(word));
        let fixed = plan
            .mine
            .clone()
            .ok_or_else(|| invalid("a run's plan names its pocket"))?;
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id))
            .ok_or_else(|| rejected("tv_dark"))?;
        // 2. The record and the ordinal.
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| RigRecord::fresh(tv, battery));
        self.adopt_field(rig, flight, &mut record)?;
        if record.ordinal.checked_add(1) != Some(ordinal) || plan.ordinal != Some(ordinal) {
            return Err(conflict("that rig's orders moved on"));
        }
        // 3. Idle: no act and no run.
        if record.act.is_some() || record.run.is_some() {
            return Err(conflict("that rig is busy"));
        }
        // 4. The licence: in force at landing.
        if !fixed.lapses_ic.is_some_and(|lapses| now < lapses) {
            return Err(rejected("unlicensed"));
        }
        // 5. The lock.
        if pocket_taken(
            txn,
            (&self.excursions, &self.pockets),
            tv,
            (&site, fixed.pocket),
            &fixed.lock_rows,
            world,
        )? {
            return Err(rejected("locked"));
        }
        // 6. The cursor's drawn, as previewed.
        let drawn = read_cursor(txn, &self.pockets, fixed.deposit)?
            .and_then(|cursor| cursor.row(fixed.number).map(|row| row.drawn_t))
            .unwrap_or(0);
        if drawn != fixed.drawn {
            return Err(conflict("that pocket changed; preview it again"));
        }
        // 7. The hold's room, as counted: every hull rig's run as the plan saw it.
        if fixed.outstanding.len()
            > usize::try_from(crate::definitions::logistics::MAX_BAY_ITEMS).unwrap_or(usize::MAX)
        {
            return Err(invalid("a run's plan names a bounded set of hull rigs"));
        }
        for (other, seen) in &fixed.outstanding {
            let now_seen = read_record(txn, &self.excursions, *other)?
                .and_then(|record| record.run)
                .map(|run| (run.ordinal, run.outstanding_t()));
            if now_seen != *seen {
                return Err(conflict("the hold's room changed; preview it again"));
            }
        }
        // 8. A blind core's body caps (the core adds the block to the row).
        if fixed.blind {
            if plan.body_sites.len() > chart::MAX_CHART_SITES_PER_BODY {
                return Err(invalid(
                    "a reading act's offer names a bounded set of sites",
                ));
            }
            let mut rows = Vec::new();
            for other in &plan.body_sites {
                if let Some(found) = read_chart(txn, &self.charts, actor, other)? {
                    rows.push(found);
                }
            }
            if !plan.body_sites.contains(&site)
                && let Some(found) = read_chart(txn, &self.charts, actor, &site)?
            {
                rows.push(found);
            }
            if !body_admits(&rows, &site, &[], &[fixed.letter.to_string()]) {
                return Err(rejected("chart_full"));
            }
        }
        // 9. The reserve: the run's whole cost and the trip home (§4.1; lead ruling 1).
        if reserve_check(
            charge_at(&record.charge, now, battery),
            reserved_total(&record),
            plan.cost_cu,
            plan.home_cu,
            battery,
        )
        .is_err()
        {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the rig's charge is under the reserve",
            ));
        }
        // 10. The write. A rig on the ground whose charge still reads the bay's (a staff
        // placement) leaves the umbilical now, so each cycle settles out of the bay without the
        // battery (the brief's 3.7 step 5).
        if record.charge.in_bay {
            record.charge = Charge {
                value_cu: charge_at(&record.charge, now, battery),
                at_ic: now,
                in_bay: false,
            };
        }
        let due_ic = now
            .checked_add(i64::from(fixed.secs))
            .ok_or_else(|| integrity("the cycle's due instant overflows"))?;
        record.ordinal = ordinal;
        record.run = Some(Run {
            ordinal,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site: site.clone(),
            body: fixed.body.clone(),
            stand: fixed.stand,
            pocket: fixed.pocket,
            number: fixed.number,
            letter: fixed.letter,
            family: fixed.family,
            deposit: fixed.deposit,
            good: fixed.good.clone(),
            source: fixed.source,
            hopper: fixed.hopper,
            cycles: fixed.cycles,
            k: 1,
            secs: fixed.secs,
            cost_cu: fixed.cost_cu,
            reserve_t: fixed.reserve_t.clone(),
            class_t: fixed.class_t,
            pocket_t: fixed.pocket_t,
            blind: fixed.blind,
            stop: false,
            lapses_ic: fixed.lapses_ic,
            lock_rows: fixed.lock_rows.clone(),
            due_ic,
            reserved_cu: plan.cost_cu,
            by: stamp.by,
        });
        // Doc 20 cut SM.8 (lead ruling 1): a run's start supersedes a queued unload too (a run is
        // never started from the bay, so none is expected).
        record.queued = None;
        // Doc 20 cut SM.11: a plan's mine step's update rides the same bundle.
        if let Some(binding) = &stamp.plan {
            apply_update(&mut record, binding, now)?;
        }
        let lock = PocketLock { tv, ordinal };
        for row in &fixed.lock_rows {
            let before = read_cursor(txn, &self.pockets, row.deposit)?;
            let after = cursor_locked(before.as_ref(), row.deposit, row.pocket, lock.clone())
                .ok_or_else(|| integrity("the pocket's cursor cannot take the lock"))?;
            write_cursor(
                txn,
                &self.pockets,
                row.deposit,
                before.is_some(),
                Some(after),
            )?;
        }
        if let Some(work) = work {
            record.field_work = Some(work);
        }
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": wire_word(ActKind::Mine, false),
                "actor": actor.0,
                "due_ic": due_ic,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }

    /// **An unload's start** (doc 20 cut SM.8; §7.1 "re-checked at prepare"; the brief's 3.7,
    /// decisions R4, R7, R15 and R16), one bundle:
    ///
    /// 1. **the parameters** — `how` names the variant and `qty` comes exactly with `part` (else
    ///    `InvalidRequest` "an unload names how it was planned");
    /// 2. **the ordinal** — the record's next (`Conflict` "that rig's orders moved on");
    /// 3. **the variant, held to the parameter** — `queued`: the offer's queued variant for this
    ///    ordinal and the record still holding its queue; `whole`/`part`: the offer's hand variant
    ///    for this ordinal with the same `how` and `qty`; anything else `Conflict` "that rig's
    ///    orders moved on" (a queued start never runs a hand order's goods, a hand `go` never a
    ///    queued plan or another `go`'s; a race of the two is settled here, whichever prepares
    ///    first);
    /// 4. **idle** — no act and no run (`Conflict` "that rig is busy");
    /// 5. **the re-check** — this hopper's pools, read here, hold each frozen good (`available`),
    ///    and the hold's room recomputed here with the one rule (`capacity::room_less` over the
    ///    hold's ore compartment of the fixed plan — the plan's own goods table — its contents,
    ///    and every sibling rig's hopper and outstanding run) admits the frozen goods in id order;
    ///    either failing → `ProviderRejected` `unload_changed`;
    /// 6. **the write** — the ordinal, the act (`UNLOAD_S`, or 0 s when a queued unload moves
    ///    nothing: told once, R7), the queue cleared.
    ///
    /// No licence, tenure, lease or battery is read (the battery's row names only a fresh record's
    /// charge). The outcome is the start's own shape.
    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one bundle
    #[allow(clippy::too_many_arguments)] // source plan, physical flight and optional causal task
    pub(super) fn start_unload(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
        plan: &FixedPlan,
        flight: Option<&crate::detachment::sortie::Flight>,
        work: Option<Box<crate::detachment::sortie::FieldWork>>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let conflict = |message: &'static str| port_error(ActionErrorCode::Conflict, message);
        let changed = || {
            port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("unload_changed"),
            )
        };
        let parameters = parameters_of(invocation)?;
        // 1. The parameters.
        let how =
            how_of(parameters).ok_or_else(|| invalid("an unload names how it was planned"))?;
        let qty = qty_of(parameters);
        let named = parameters.contains_key("qty");
        if (how == UnloadHow::Part) != named || (named && qty.is_none()) {
            return Err(invalid("an unload names how it was planned"));
        }
        let fixed = plan
            .unload
            .clone()
            .ok_or_else(|| invalid("an unload's plan names its vaults"))?;
        let site = plan
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        if fixed.siblings.len()
            > usize::try_from(crate::definitions::logistics::MAX_BAY_ITEMS).unwrap_or(usize::MAX)
            || fixed.goods_of_hold.len() > mush_inventory::MAX_POOLS_PER_ENDPOINT
        {
            return Err(invalid(
                "an unload's plan names a bounded set of rigs and goods",
            ));
        }
        // 2. The record and the ordinal.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| {
            battery.map_or_else(
                || RigRecord::fresh_unread(tv),
                |battery| RigRecord::fresh(tv, battery),
            )
        });
        self.adopt_field(rig, flight, &mut record)?;
        if record.ordinal.checked_add(1) != Some(ordinal) {
            return Err(conflict("that rig's orders moved on"));
        }
        // 3. The variant, held to the parameter.
        let (goods, excursion) = match how {
            UnloadHow::Queued => match (&fixed.queued, &record.queued) {
                (Some(queued), Some(_)) if queued.ordinal == ordinal => {
                    (queued.goods.clone(), queued.excursion)
                }
                _ => return Err(conflict("that rig's orders moved on")),
            },
            UnloadHow::Whole | UnloadHow::Part => match &fixed.hand {
                Some(hand) if hand.ordinal == ordinal && hand.how == how && hand.qty == qty => {
                    (hand.goods.clone(), None)
                }
                _ => return Err(conflict("that rig's orders moved on")),
            },
        };
        if goods.len() > MAX_DUMP_GOODS || goods.values().any(|qty| *qty == 0) {
            return Err(invalid(
                "an unload moves a bounded set of positive quantities",
            ));
        }
        // 4. Idle: no act and no run.
        if record.act.is_some() || record.run.is_some() {
            return Err(conflict("that rig is busy"));
        }
        // 5. The re-check: the hopper holds the frozen goods; the hold's room admits them.
        let hopper = EndpointRef(VaultId(fixed.hopper));
        for (good, qty) in &goods {
            let available = self
                .inventory
                .pool(txn, hopper, good)
                .map_err(|_| integrity("the hopper cannot be read"))?
                .map_or(0, |pool| pool.available);
            if available < *qty {
                return Err(changed());
            }
        }
        if !goods.is_empty() {
            let table = crate::port::market::plan_goods(&fixed.plan).ok_or_else(|| {
                tracing::error!(
                    tv = tv.0,
                    "tacsim: an unload's hold plan has no goods table"
                );
                integrity("the hold's plan cannot be read")
            })?;
            let ore = CanonicalId::new("metal_ore").map_err(|_| integrity("the ore id"))?;
            let compartment = crate::port::capacity::compartment_for(
                table,
                &crate::port::market::plan_compartments(&fixed.plan),
                &ore,
            )
            .ok_or_else(|| {
                tracing::error!(
                    tv = tv.0,
                    "tacsim: an unload's hold has no one ore compartment"
                );
                integrity("the hold's plan cannot be read")
            })?;
            let hold = EndpointRef(VaultId(fixed.hold));
            let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
            for good in &fixed.goods_of_hold {
                let held = self
                    .inventory
                    .pool(txn, hold, good)
                    .map_err(|_| integrity("the hold cannot be read"))?
                    .map_or(0, |pool| pool.quantity);
                if held > 0 {
                    contents.insert(good.clone(), held);
                }
            }
            let mut reserved: Vec<BTreeMap<CanonicalId, u64>> = Vec::new();
            for sibling in &fixed.siblings {
                let mut theirs: BTreeMap<CanonicalId, u64> = BTreeMap::new();
                for good in &sibling.goods {
                    let held = self
                        .inventory
                        .pool(txn, EndpointRef(VaultId(sibling.hopper)), good)
                        .map_err(|_| integrity("a rig's hopper cannot be read"))?
                        .map_or(0, |pool| pool.quantity);
                    if held > 0 {
                        theirs.insert(good.clone(), held);
                    }
                }
                if let Some(run) =
                    read_record(txn, &self.excursions, sibling.tv)?.and_then(|record| record.run)
                {
                    let slot = theirs.entry(run.good.clone()).or_insert(0);
                    *slot = slot.saturating_add(run.outstanding_t());
                }
                reserved.push(theirs);
            }
            let mut room =
                crate::port::capacity::room_less(table, &compartment, &contents, &reserved)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
            for (good, qty) in &goods {
                let fits = crate::port::capacity::units_in(table, &compartment, &room, good)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
                if fits < *qty {
                    return Err(changed());
                }
                crate::port::capacity::take_room(table, &compartment, &mut room, good, *qty)
                    .map_err(|_| integrity("the hold's room cannot be weighed"))?;
            }
        }
        // 6. The write: the act (a queued unload that moves nothing is a 0 s act, told once —
        // decision R7), the queue cleared.
        let secs = if goods.is_empty() { 0 } else { UNLOAD_S };
        let due_ic = now
            .checked_add(i64::from(secs))
            .ok_or_else(|| integrity("the act's due instant overflows"))?;
        record.ordinal = ordinal;
        record.act = Some(Act {
            ordinal,
            kind: ActKind::Unload,
            operator: actor,
            hull: rig.hull,
            slot: rig.slot,
            site,
            started_ic: now,
            due_ic,
            reserved_cu: 0,
            params: ActParams::Unload {
                hopper: fixed.hopper,
                hold: fixed.hold,
                goods,
                how,
                excursion,
            },
            by: None,
            seal: None,
        });
        record.queued = None;
        if let Some(work) = work {
            record.field_work = Some(work);
        }
        write_record(txn, &self.excursions, record, existed)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": wire_word(ActKind::Unload, false),
                "actor": actor.0,
                "due_ic": due_ic,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }

    /// **The plan's three variants a rig's offer fixes** (doc 20 cut SM.11; the brief's 3.6): the
    /// plan's next move — the interpreter over the durable record, only for a rig whose record
    /// holds a plan live for the actor (no plan, no cost) — and the clear and the strike, each
    /// only of the actor's own plan (a previous lessee's is inert: the lead's pre-commit round,
    /// item 3) and only when `controls` (the actor aboard the rig's hull, for a rig off the
    /// actor's helm; always at the helm, as before).
    #[allow(clippy::too_many_arguments)] // the offer's own reads, passed through
    pub(super) fn plan_variants(
        &self,
        engine: &mush_core::Engine,
        actor: ObjId,
        rig: &RigAt,
        record: Option<&RigRecord>,
        peeks: &mut Peeks,
        has_battery: bool,
        controls: bool,
    ) -> [(String, FixedPlan); 3] {
        let w: &World = &engine.world;
        let next = record.map_or(Some(1), |record| record.ordinal.checked_add(1));
        let stepped = match (record, next) {
            (Some(record), Some(next)) => match record.live_plan_for(actor) {
                Some(plan) => {
                    let reads = crate::port::site_map::EngineReads(engine);
                    let counts = excursion_counts(w, &|tv| reads.rig(tv), plan.author, &plan.site);
                    match crate::surface::plan::decide(
                        w,
                        &reads,
                        rig,
                        record,
                        &counts,
                        &self.tools,
                        peeks,
                        w.world_ic_now().at,
                    ) {
                        Some(moved) => plan_fixed(w, plan, next, moved, has_battery),
                        None => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
                    }
                }
                None => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
            },
            _ => FixedPlan::refused(Refusal::PlanNone { n: rig.slot }.word()),
        };
        let (live, ended) = record
            .and_then(|record| record.plan.as_ref())
            .filter(|plan| plan.author == actor)
            .map_or((false, false), |plan| (plan.live(), !plan.live()));
        // The reviewer's R2: a control the record would allow but the actor may not work from
        // where they stand (off the helm and off the rig's hull) is refused `not_aboard`.
        let control = |ok: bool, word: &str| match (ok, controls) {
            (true, true) => FixedPlan {
                ok: true,
                ..FixedPlan::default()
            },
            (true, false) => FixedPlan::refused("not_aboard"),
            (false, _) => FixedPlan::refused(word),
        };
        [
            ("plan".to_owned(), stepped),
            (
                "plan_clear".to_owned(),
                control(live, if ended { "plan_ended" } else { "plan_none" }),
            ),
            (
                "plan_strike".to_owned(),
                control(ended, if live { "plan_live" } else { "plan_none" }),
            ),
        ]
    }

    /// **The offer's filing variant** (doc 20 cut SM.11; the brief's 3.6, the reviewer's F11): the
    /// rig's RAM [`PlanOrder`] for this actor **planned at the record's ordinal and for its next
    /// plan number**, both checked before any compile — a stale order (its filing landed, or the
    /// rig's orders moved on) is `unplanned` at once, with no compile — then [`plan_ask`] re-run
    /// over the engine with the order's list and family and **required equal** to the order's
    /// compile, else `plan_changed`: `go` files exactly the preview or nothing.
    pub(super) fn fill_fixed(
        &self,
        engine: &mush_core::Engine,
        actor: ObjId,
        rig: &RigAt,
        record: Option<&RigRecord>,
        peeks: &mut Peeks,
    ) -> FixedPlan {
        let w: &World = &engine.world;
        let (ordinal, plans) = record.map_or((0, 0), |record| (record.ordinal, record.plans));
        let Some(order) = plan_order_of(w, rig.tv).filter(|order| {
            order.actor == actor
                && order.ordinal == ordinal
                && plans.checked_add(1) == Some(order.no)
        }) else {
            return FixedPlan::refused(Refusal::Unplanned.word());
        };
        crate::surface::plan::count(w, rig.tv, |c| &c.compiles);
        let reads = crate::port::site_map::EngineReads(engine);
        match plan_ask(
            w,
            &reads,
            actor,
            rig,
            &order.list,
            order.family,
            &self.tools,
            peeks,
            w.world_ic_now().at,
        ) {
            Err(refusal) => FixedPlan::refused(refusal.word()),
            Ok((Ok(compiled), facts)) if compiled == order.compiled => FixedPlan {
                ok: true,
                site: Some(facts.site),
                ordinal: Some(ordinal),
                fill: Some(FillFixed {
                    no: order.no,
                    ordinal,
                    list: compiled.list,
                    family: compiled.family,
                    steps: compiled.steps,
                }),
                ..FixedPlan::default()
            },
            Ok(_) => FixedPlan::refused(Refusal::PlanChanged { n: rig.slot }.word()),
        }
    }

    /// **A filing's prepare** (doc 20 cut SM.11; the brief's 3.6): the offer's filing variant ok
    /// (its word refused otherwise); the record's ordinal the order's (`Conflict` "that rig's orders
    /// moved on"); no plan live for the actor (`plan_filed`); the number the record's next — then
    /// ONE write of the record alone: `plans = no` and the plan, live, its deadline 288 IC hours on,
    /// its cursor 0 (an older ended plan replaced). `now` is the offer's instant.
    pub(super) fn file_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, now): (ObjId, ObjId, u32, i64),
        rig: &FixedRig,
        flight: Option<&crate::detachment::sortie::Flight>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let fixed = rig
            .plans
            .get("plan_file")
            .cloned()
            .ok_or_else(|| invalid("that act is not planned"))?;
        if !fixed.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(fixed.code.as_deref().unwrap_or("unplanned")),
            ));
        }
        let fill = fixed
            .fill
            .clone()
            .ok_or_else(|| invalid("a filing's plan names its steps"))?;
        let site = fixed
            .site
            .clone()
            .ok_or_else(|| invalid("an act's plan names its site"))?;
        let existing = read_record(txn, &self.excursions, tv)?;
        let existed = existing.is_some();
        let mut record = existing.unwrap_or_else(|| RigRecord::fresh_unread(tv));
        self.adopt_field(rig, flight, &mut record)?;
        // A filing writes no act: the invocation's ordinal is the record's (at least 1, the
        // parameter's floor) and the order's.
        if record.ordinal != fill.ordinal || record.ordinal.max(1) != ordinal {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        if record.live_plan_for(actor).is_some() {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word("plan_filed"),
            ));
        }
        if record.plans.checked_add(1) != Some(fill.no) {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's orders moved on",
            ));
        }
        let deadline_ic = now
            .checked_add(PLAN_BOUND_IC_SECS)
            .ok_or_else(|| integrity("the plan's deadline overflows"))?;
        record.plans = fill.no;
        record.plan = Some(FillPlan {
            no: fill.no,
            author: actor,
            hull: rig.hull,
            slot: rig.slot,
            site,
            family: fill.family,
            filed_ic: now,
            deadline_ic,
            list: fill.list,
            steps: fill.steps,
            cursor: 0,
            passed: Vec::new(),
            state: FillState::Live,
            seq: 1,
            tally: PlanTally::default(),
            notes: Vec::new(),
        });
        write_record(txn, &self.excursions, record, existed)?;
        self.control_outcome(invocation, "plan_file", actor, tv, ordinal, now)
    }

    /// **A clear's or a strike's prepare** (doc 20 cut SM.11; the brief's 3.6, D17): the record's
    /// plan of the invocation's number (`Conflict` "the plan moved on" otherwise). A clear: the plan
    /// live (`plan_ended` when it ended by itself between the command and the landing) → ended
    /// `Cleared` now, one `Ended` note, `seq` on — the step in flight untouched. A strike: the plan
    /// ended (`plan_live` otherwise) → struck from the record. One write each.
    pub(super) fn clear_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, now): (ObjId, ObjId, i64),
        rig: &FixedRig,
        strike: bool,
        no: Option<u32>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let word = if strike { "plan_strike" } else { "plan_clear" };
        // The offer names the rig the actor's, and its variant must be offered ok: the reviewer's
        // R2 found the variant's `controls` (the author aboard the rig's hull, for a rig off their
        // helm) fixed by the offer and never read here, so an author who had left the hull could
        // still land a clear typed aboard. The variant's other answer (live or ended) is the
        // record's, re-read below: the record is the authority.
        match rig.plans.get(word) {
            None => return Err(invalid("that act is not planned")),
            Some(fixed) if !fixed.ok => {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word(fixed.code.as_deref().unwrap_or("plan_none")),
                ));
            }
            Some(_) => {}
        }
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(moved_on());
        };
        // Only the actor's own plan (the lead's pre-commit round, item 3: a rig off the helm is
        // offered to its plan's author; a previous lessee's plan is never cleared by another).
        let Some(plan) = record
            .plan
            .as_mut()
            .filter(|plan| Some(plan.no) == no && plan.author == actor)
        else {
            return Err(moved_on());
        };
        if strike {
            if plan.live() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("plan_live"),
                ));
            }
            record.plan = None;
        } else {
            if !plan.live() {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    code_of_word("plan_ended"),
                ));
            }
            plan.passed.clear();
            plan.state = FillState::Ended {
                why: PlanEnd::Cleared,
                at_ic: now,
            };
            push_note(
                plan,
                PlanNoteKind::Ended {
                    why: PlanEnd::Cleared,
                },
                now,
            )?;
            plan.seq = plan
                .seq
                .checked_add(1)
                .ok_or_else(|| port_error(ActionErrorCode::Integrity, "a plan's seq overflows"))?;
        }
        let ordinal = record.ordinal.max(1);
        write_record(txn, &self.excursions, record, true)?;
        self.control_outcome(invocation, word, actor, tv, ordinal, now)
    }

    /// **A plan's end's prepare** (doc 20 cut SM.11; the brief's 3.6): the offer's plan variant an
    /// end (`word == plan_end`) bound to the start's plan and step and the record's live plan's
    /// `seq` (`Conflict` "the plan moved on" otherwise), nothing in flight — then the plan's update
    /// and its end, one `Ended` note, `seq` on. No act; a `Stranded` end writes nothing else.
    pub(super) fn end_plan(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, now): (ObjId, ObjId, i64),
        rig: &FixedRig,
        by: &ByParams,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fixed = rig
            .plans
            .get("plan")
            .cloned()
            .ok_or_else(|| invalid("that act is not planned"))?;
        if !fixed.ok {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                code_of_word(fixed.code.as_deref().unwrap_or("plan_moved")),
            ));
        }
        let binding = fixed.plan.ok_or_else(moved_on)?;
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(moved_on());
        };
        let live = record.live_plan_for(actor).map(|plan| (plan.no, plan.seq));
        if binding.word != "plan_end"
            || binding.end.is_none()
            || Some(binding.no) != by.plan
            || Some(binding.step) != by.step
            || live != Some((binding.no, binding.seq))
        {
            return Err(moved_on());
        }
        // An act or a run in flight holds the end (the step finishes by its own rules); a queued
        // unload does not — the interpreter answers an end with one standing only when the plan's
        // bound has passed or the queue is orphaned (the lead's pre-commit round, item 3), and the
        // queue is SM.8's own, which lands by itself whether the plan lives or not.
        if record.act.is_some() || record.run.is_some() {
            return Err(port_error(ActionErrorCode::Conflict, "that rig is busy"));
        }
        apply_update(&mut record, &binding, now)?;
        let ordinal = record.ordinal.max(1);
        write_record(txn, &self.excursions, record, true)?;
        self.control_outcome(invocation, "plan_end", actor, tv, ordinal, now)
    }

    /// The start's outcome for a plan's control (no act: `due_ic` is `now`).
    pub(super) fn control_outcome(
        &self,
        invocation: &ActionInvocation,
        word: &str,
        actor: ObjId,
        tv: ObjId,
        ordinal: u32,
        now: i64,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": word,
                "actor": actor.0,
                "due_ic": now,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }

    /// **`+tv/mine stop`'s prepare** (doc 20 cut SM.7; the brief's 3.4, decision R7): the record's
    /// run of the invocation's ordinal, the actor's (`Conflict` "that rig is not drilling"
    /// otherwise), marked to close after the cycle in flight — one write; already marked, nothing
    /// changes. No ordinal advance; never a refund.
    pub(super) fn stop_run(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal): (ObjId, ObjId, u32),
        work: Option<Box<crate::detachment::sortie::FieldWork>>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let not_drilling = || port_error(ActionErrorCode::Conflict, "that rig is not drilling");
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(not_drilling());
        };
        let Some((due_ic, stopping)) = record
            .run
            .as_ref()
            .filter(|run| run.ordinal == ordinal && run.operator == actor)
            .map(|run| (run.due_ic, run.stop))
        else {
            return Err(not_drilling());
        };
        if !stopping {
            if let Some(run) = record.run.as_mut() {
                run.stop = true;
            }
            if let Some(work) = work {
                record.field_work = Some(work);
            }
            write_record(txn, &self.excursions, record, true)?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "act": "mine_stop",
                "actor": actor.0,
                "due_ic": due_ic,
                "ordinal": ordinal,
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the surface act's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// A principal's chart row for a site, cloned; `None` when absent. A row under another key
/// fails closed (the families rule: readers re-check the key).
pub(super) fn read_chart(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<ChartKey, ChartRecord>,
    principal: ObjId,
    site: &CanonicalId,
) -> Result<Option<ChartRecord>, ActionError> {
    let key = ChartKey {
        principal,
        site: site.clone(),
    };
    let found = txn
        .get(family, &key)
        .map_err(|_| port_error(ActionErrorCode::Integrity, "the charts are unavailable"))?
        .cloned();
    if let Some(row) = &found
        && row.key() != key
    {
        return Err(port_error(
            ActionErrorCode::Integrity,
            "a chart row disagrees with its own key",
        ));
    }
    Ok(found)
}

// ---- tacsim_surface_done: the finish ------------------------------------------------------------

/// **`tacsim_surface_done`** — every act's finish (the brief's 3.4).
pub(crate) struct SurfaceDoneProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub excursions: StateFamilyHandle<RigKey, RigRecord>,
    pub charts: StateFamilyHandle<ChartKey, ChartRecord>,
    pub surveys:
        StateFamilyHandle<crate::surface::survey::SurveyKey, crate::surface::survey::SurveyRecord>,
    pub quest_survey_evidence: StateFamilyHandle<ObjId, crate::quest::evidence::SurveyEvidenceBook>,
    /// Doc 20 cut SM.7: the pocket cursors (`tacsim.pockets`) a run locks and draws.
    pub pockets: StateFamilyHandle<mush_industry::OreDepositId, PocketCursor>,
    pub inventory: InventoryFamilies,
    pub tools: Arc<BTreeMap<CanonicalId, ToolDef>>,
    /// **Doc 23 cut S.1** (L2, L4): the black box participant's handle for this action — the
    /// open line at a launch's `Done`, the close line at a recover's.
    pub black_box: Option<
        mush_core::participant::ParticipantHandle<crate::surface::black_box::BlackBoxParticipant>,
    >,
}

/// **An implied launch's settlement** (doc 20 cut SM.4a; the brief's reviewer finding F1,
/// D22; §4.1): the bay credits charge up to the end of the launch half (`started_ic +
/// LAUNCH_S`) and no further; the act's cost then comes off out of the bay, stamped at the
/// act's `due_ic`. SM.3b's [`spent`] — which would credit the bay for the whole act — is never
/// called on a scan or a sounding from the bay.
pub(super) fn spent_after_launch(
    charge: &Charge,
    act: &Act,
    battery: &BatteryDef,
) -> Result<Charge, ActionError> {
    let launched_at = act.started_ic.saturating_add(i64::from(LAUNCH_S));
    let value = charge_at(charge, launched_at, battery)
        .checked_sub(act.reserved_cu)
        .ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the act spent more than the charge",
            )
        })?;
    Ok(Charge {
        value_cu: value,
        at_ic: act.due_ic,
        in_bay: false,
    })
}

/// **An implied launch refused `Moved`** (the SM.4a fixer round's finding F1; §4.1, D22): the
/// rig left the bay (a staff move) and its excursion stays open, so its charge leaves the bay
/// as [`spent_after_launch`]'s would — the bay's credit up to `started_ic + LAUNCH_S` and no
/// further — with nothing spent (the act did not run), stamped at the act's `due_ic`, out of
/// the bay. Without the battery's row no credit is read at all (the stored value stands): a
/// rig never gains charge it cannot be shown to have. A charge already out of the bay is
/// unchanged.
pub(super) fn left_after_launch(
    charge: &Charge,
    act: &Act,
    battery: Option<&BatteryDef>,
) -> Charge {
    if !charge.in_bay {
        return *charge;
    }
    let launched_at = act.started_ic.saturating_add(i64::from(LAUNCH_S));
    Charge {
        value_cu: battery.map_or(charge.value_cu, |battery| {
            charge_at(charge, launched_at, battery)
        }),
        at_ic: act.due_ic,
        in_bay: false,
    }
}

/// **How a reading was made, from who set its act going** (doc 20 cut SM.11; lead ruling 4): a
/// plan's step writes `via: Plan`; the hand (and the idle return, which reads nothing) `Hand`.
pub(super) fn via_of(by: Option<ActBy>) -> chart::Via {
    match by {
        Some(ActBy::Plan { .. }) => chart::Via::Plan,
        Some(ActBy::Idle) | None => chart::Via::Hand,
    }
}

/// **The plan's tally, counted at a finish** (doc 20 cut SM.11; the brief's 3.7, the reviewer's
/// F9): only on the record's plan whose number `by` names — riding the record write the finish
/// already makes (no extra write, no `seq` bump).
pub(super) fn tally(record: &mut RigRecord, by: Option<ActBy>, count: impl FnOnce(&mut PlanTally)) {
    let Some(ActBy::Plan { no, .. }) = by else {
        return;
    };
    if let Some(plan) = record.plan.as_mut().filter(|plan| plan.no == no) {
        count(&mut plan.tally);
    }
}

/// **What a reading act's finish wrote to the chart**: its span on the outcome.
pub(super) fn write_reading(
    txn: &mut ActionTxn<'_>,
    charts: &StateFamilyHandle<ChartKey, ChartRecord>,
    act: &Act,
) -> Result<ReadSpan, ActionError> {
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let body = match &act.params {
        ActParams::Scan { body, .. } | ActParams::Sound { body, .. } => body.clone(),
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => {
            return Err(integrity("a reading act carries its reach"));
        }
    };
    let existing = read_chart(txn, charts, act.operator, &act.site)?;
    let existed = existing.is_some();
    let mut row =
        existing.unwrap_or_else(|| ChartRecord::new(act.operator, act.site.clone(), body));
    let at = act.due_ic;
    let span = match &act.params {
        ActParams::Scan {
            cell,
            footing,
            fields,
            hidden,
            ..
        } => {
            write_scan(&mut row, *cell, fields, *footing, at, via_of(act.by));
            ReadSpan::Scan {
                cell: *cell,
                footing: *footing,
                fields: fields.clone(),
                hidden: hidden.clone(),
            }
        }
        ActParams::Sound {
            cell,
            radius,
            quiet,
            ..
        } => {
            let pockets = pockets_in_reach((cell[0], cell[1]), i64::from(*radius));
            write_sound(&mut row, *cell, &pockets, *quiet, at, via_of(act.by));
            // The heard budget (D7): one heard line a sounder an IC hour on this site; a quiet
            // sounding is never heard and never touches it.
            let heard = !*quiet && heard_line_due(row.heard_ic, at);
            if heard {
                row.heard_ic = Some(at);
            }
            ReadSpan::Sound {
                cell: *cell,
                radius: *radius,
                quiet: *quiet,
                heard,
            }
        }
        ActParams::None {}
        | ActParams::Dump { .. }
        | ActParams::Drive { .. }
        | ActParams::Unload { .. } => {
            return Err(integrity("a reading act carries its reach"));
        }
    };
    row.check()
        .map_err(|_| integrity("the chart row is out of bounds"))?;
    let key = row.key();
    let staged = if existed {
        txn.replace(charts, &key, row)
    } else {
        txn.insert(charts, &key, row)
    };
    staged.map_err(|_| integrity("the chart row did not stage"))?;
    Ok(span)
}

/// The charge at the act's own stamp, less its cost; an underflow is `Integrity` (the
/// reserve already guaranteed it).
pub(super) fn spent(
    charge: &Charge,
    act: &Act,
    battery: &BatteryDef,
    in_bay: bool,
) -> Result<Charge, ActionError> {
    let value = charge_at(charge, act.due_ic, battery)
        .checked_sub(act.reserved_cu)
        .ok_or_else(|| {
            port_error(
                ActionErrorCode::Integrity,
                "the act spent more than the charge",
            )
        })?;
    Ok(Charge {
        value_cu: value,
        at_ic: act.due_ic,
        in_bay,
    })
}

/// **Append a launch's seal mark** (doc 17h cut M8e E4; M8c B5) to `licence`'s ring through
/// the Roll's writer in the finish's own transaction: no writer composed (no ring, no consumer)
/// writes nothing; a ring that refuses is paged and the finish still completes — the mark alone
/// is lost, as a black box line is.
fn seal_mark(
    context: &mut mush_core::PrepareCtx<'_>,
    txn: &mut ActionTxn<'_>,
    (tv, licence): (ObjId, u64),
    open: &crate::surface::excursion::OpenSortie,
    governed: bool,
) {
    let Ok(world) = context.world() else {
        return;
    };
    let Some(writer) = world
        .module_state::<TacsimStore>()
        .and_then(|store| store.seal_marks.clone())
        .filter(mush_roll::commerce::seal_marks::SealMarkWriter::installed)
    else {
        return;
    };
    let at_utc = world.now();
    // The flight's IC second on the world's own IC clock (the epoch opened at the launch's due).
    let stamp = mush_core::ClockStamp {
        at: open.opened_ic,
        ..world.world_ic_now()
    };
    let (Ok(licence), Ok(at_ic)) = (
        mush_roll::commerce::LicenceId::new(licence),
        mush_roll::commerce::WorldIcSeconds::from_stamp(&stamp),
    ) else {
        return;
    };
    if let Err(error) = writer.push(
        txn,
        licence,
        (tv, at_ic, at_utc),
        governed,
        open.id.to_string(),
    ) {
        tracing::error!(
            tv = tv.0,
            ?error,
            "tacsim: rig #{}'s seal mark was refused; the launch completes without it",
            tv.0
        );
    }
}

/// **The seal epoch's move at an act's finish** (doc 23 cut S.1; §6 "What the host does at each
/// end", D-23-1, D-23-2, D-23-5, D-23-8; the lead's L3–L6), on the record the finish is about to
/// write, and the black box line it owes:
///
/// - **an act that leaves the bay** (a launch, or an implied one: a scan, a sounding or a drive
///   from the bay) whose excursion survives the finish with the rig out of every bay **opens**
///   the epoch — `SortieId::ic(tv, launch_ic)`, the hangar it left, the seal fixed at the offer
///   (or absent, paged), the launch's operator as the warrant principal (D-23-5: a plan's step or
///   the idle return does not move it) and the site's admission count (D-23-8) — and owes
///   `mind_sealed`. A rig whose last epoch is still open is a conflict the finish refuses, paged
///   (the start refuses it first; this is the backstop). `NeverLeft` closed the excursion, so
///   it opens nothing;
/// - **a recover's `Done`** closes the open epoch `ReturnedToBase` into the hull it climbed into
///   and owes `sortie_closed`, `abandoned` as the epoch carried it.
pub(super) fn epoch_move(
    record: &mut RigRecord,
    act: &Act,
    (tv, result): (ObjId, ActResult),
    rig: &DoneRig,
) -> Result<Option<crate::surface::black_box::BlackBoxLine>, ActionError> {
    use crate::surface::black_box::BlackBoxLine;
    if act.kind == ActKind::Launch || act.params.launches() {
        let Some(excursion) = record.excursion.clone() else {
            return Ok(None);
        };
        if matches!(rig.facts, Facts::Bay { .. }) {
            return Ok(None);
        }
        if let Some(open) = &record.sortie {
            tracing::error!(
                tv = tv.0,
                sortie = %open.id,
                "tacsim: rig #{} left its bay with its last epoch still open; the finish is \
                 refused; staff `home` the rig to close it",
                tv.0
            );
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that rig's last trip is still open; staff must bring it home first",
            ));
        }
        let (opened, line) =
            crate::surface::black_box::open_epoch(tv, &excursion, act.due_ic, rig.epoch.as_ref());
        record.sortie = Some(opened);
        return Ok(Some(line));
    }
    if act.kind == ActKind::Recover
        && result == ActResult::Done
        && let Some(open) = record.sortie.take()
    {
        return Ok(Some(BlackBoxLine::sortie_closed(
            tv,
            &open,
            act.due_ic,
            mush_roll::commerce::ic_claims::IcCloseKind::ReturnedToBase { base: act.hull },
        )));
    }
    Ok(None)
}

impl ActionProvider for SurfaceDoneProvider {
    fn resolved_revision_keys(
        &self,
        r: &mush_core::ResolvedActionInvocation,
    ) -> Result<Vec<mush_core::RevisionKey>, ActionError> {
        let p = parameters_of(r.invocation())?;
        let tv = tv_of(p)?;
        if selected_crew(p, tv)?.is_some() {
            Ok(vec![crate::detachment::sortie::surface::work_key(
                r.invocation().actor(),
                tv,
            )?])
        } else {
            Ok(Vec::new())
        }
    }
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        let parameters = parameters_of(invocation)?;
        tv_of(parameters)?;
        ordinal_of(parameters)?;
        cycle_param(parameters)?;
        Ok(())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if !self_query(action, &self.action, query) {
            return Ok(None);
        }
        let w: &World = &engine.world;
        let actor = query.actor();
        // Finding F6: only to the pilot of a live lease, over that pilot's own rigs —
        // whatever the helm, so a pilot whose charter is in trouble still finishes an act.
        let Some(store) = w.module_state::<TacsimStore>() else {
            return Ok(None);
        };
        // Doc 20 cut SM.12b: a yard's dispatcher finishes its house rigs' acts (house views).
        let mut records: Vec<TvLease> = store
            .tv_leases_of(actor)
            .filter(|record| record.live())
            .cloned()
            .chain(crate::port::yard::house_rigs(w, actor))
            .collect();
        // Physical NPC operators finish their own admitted work, with the real
        // lease retained as its financial contract. Completion needs no proxy
        // account and cannot become another operator's work after succession.
        let units = world_rigs(w);
        if units.len() > crate::surface::excursion::MAX_EXCURSIONS_WORLD as usize {
            return Err(invalid("surface work exceeds the record visit budget"));
        }
        for unit in units {
            let Some(found) = engine
                .action_state()
                .get(&self.excursions, &RigKey(unit))
                .ok()
                .flatten()
            else {
                continue;
            };
            let Some(crew) = found.field_crew.as_deref().filter(|c| c.operator == actor) else {
                continue;
            };
            if let Some(lease) = store
                .tv_lease_on(unit)
                .filter(|lease| lease.pilot == crew.principal)
            {
                records.push(lease.clone());
            }
        }
        records.sort_by_key(|record| record.tv);
        records.dedup_by_key(|record| record.tv);
        if records.is_empty() {
            return Ok(None);
        }
        let mut rigs: BTreeMap<String, DoneRig> = BTreeMap::new();
        let now = w.world_ic_now();
        let reads = crate::port::site_map::EngineReads(engine);
        for record in &records {
            let current = engine
                .action_state()
                .get(&self.excursions, &RigKey(record.tv))
                .ok()
                .flatten();
            let principal = current
                .and_then(|r| r.field_crew.as_deref())
                .map_or(actor, |c| c.principal);
            // Doc 20 cut SM.7 (§8 "rights are per act"; the brief's 3.7): a rig drilling has the
            // rights of its run's next cycle read here, at landing — the licence's standing, the
            // block's tenure, the lease.
            let rights = engine
                .action_state()
                .get(&self.excursions, &RigKey(record.tv))
                .ok()
                .flatten()
                .and_then(|found| found.run.clone())
                .map(|run| RightsVerdict {
                    licence: licence_verdict(w, principal, now.at),
                    // The SM.7 lead's pre-commit round, item 3: a read that fails is `Dark`, never
                    // `Claimed` — the run ends with the dark line, not a false claim.
                    tenure: match site_def_of(w, &run.site) {
                        None => TenureRead::Dark {
                            why: DarkCause::Site,
                        },
                        Some(site) => {
                            match mine_tenure(
                                w,
                                &reads,
                                principal,
                                (&run.site, &site),
                                run.letter,
                                &now,
                            ) {
                                Ok(()) => TenureRead::Allowed,
                                Err(TenureBar::Claimed) => TenureRead::Claimed,
                                Err(TenureBar::Dark(why)) => TenureRead::Dark { why },
                            }
                        }
                    },
                    lease: lease_gate(w, record).map_err(|bar| bar == LeaseBar::Forfeit),
                });
            // Doc 20 cut SM.8: the hopper (a recover queues its unload from it) and the hull's
            // hold (an unload's finish re-checks against its plan).
            let hopper = crate::port::vehicle::hopper_of(w, record.tv).and_then(|vault| {
                tv::hopper_goods_of(w, record.tv).map(|(goods, _)| HopperFacts {
                    vault: vault.0,
                    goods,
                })
            });
            let hold = hull_view(w, record.hull).and_then(|view| {
                let vault = crate::port::ship::hold_vault(w, &view)?;
                let plan = crate::port::market::hold_plan(&view)?;
                Some(HoldFacts {
                    vault: vault.0,
                    plan,
                })
            });
            // Doc 23 cut S.1 (L3, L8, D-23-8): a launching act's epoch facts — the ROM seal, the
            // class, the site's open epochs — fixed here, where the World is in hand.
            let read_rig = |tv: ObjId| {
                engine
                    .action_state()
                    .get(&self.excursions, &RigKey(tv))
                    .ok()
                    .flatten()
                    .cloned()
            };
            // Doc 23 cut S.2 (V7): the carrier's open epoch, its typed record.
            let read_carrier = |hull: ObjId| crate::port::epoch::open_id_in(engine, hull);
            let epoch = read_rig(record.tv).and_then(|found| {
                crate::surface::black_box::epoch_facts(w, (&read_rig, &read_carrier), &found)
            });
            rigs.insert(
                record.tv.0.to_string(),
                DoneRig {
                    crew: current.and_then(|r| r.field_crew.clone()),
                    battery_id: rig_battery_id(w, record.tv),
                    facts: facts_of(w, record.tv, record.hull),
                    spoil: spoil_of(w, record.hull),
                    failed: crate::surface::tick::finish_retried(w, record.tv),
                    rights,
                    hopper,
                    hold,
                    epoch,
                },
            );
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        // Doc 20 cut SM.8: the IC clock stamp and the real second beside `now` (the market lot
        // offer's precedent) — an unload's marked transfer stamps its markers with them.
        let (survey_offer, survey_revisions) =
            crate::surface::survey::reading_offer(engine, &self.surveys, actor)?;
        let fixed = json!({
            "now": w.world_ic_now().at,
            "clock": w.world_ic_now(),
            "wall": w.now(),
            "surveys": survey_offer,
            "rigs": serde_json::to_value(&rigs)
                .map_err(|_| integrity("the rigs' facts do not encode"))?,
        });
        let parameters = vec![
            parameter(
                "tv",
                true,
                ProjectedParameterKind::Choice {
                    choices: records
                        .iter()
                        .map(|record| record.tv.0.to_string())
                        .collect(),
                },
            )?,
            parameter(
                "ordinal",
                true,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(u32::MAX),
                },
            )?,
            // Doc 20 cut SM.7 (the brief's 3.7): a run's cycle; absent for an act's finish.
            parameter(
                "cycle",
                false,
                ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: i64::from(crate::surface::excursion::MAX_RUN_CYCLES),
                },
            )?,
        ];
        offer_draft_with_revisions(
            action,
            engine,
            fixed,
            parameters,
            (
                "tacsim-surface-done",
                "Finish a rig's act",
                "Record how a rig's act came out, at its own due instant.",
            ),
            survey_revisions,
        )
        .map(Some)
    }

    /// **Doc 23 cut S.1** (L2, L4): the finish takes its preparation context, through which the
    /// launch's `Done` and the recover's append the black box line in this act's own bundle
    /// (`crate::surface::black_box::append`).
    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one decision per kind
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters = parameters_of(invocation)?;
        let actor = invocation.actor();
        let tv = tv_of(parameters)?;
        let ordinal = ordinal_of(parameters)?;
        let now = now_of(parameters)?;
        let rigs: BTreeMap<String, DoneRig> = parameters
            .get("rigs")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| invalid("a finish's offer fixes the rigs"))?;
        let Some(rig) = rigs.get(&tv.0.to_string()) else {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that rig is not yours to work",
            ));
        };
        if rig.failed.len() > crate::surface::tick::MAX_RETRIED_ORDINALS {
            return Err(invalid(
                "a finish's offer fixes at most a bounded set of retried acts",
            ));
        }
        // Doc 20 cut SM.7 (the brief's 3.7): a run's cycle finishes on its own path.
        if let Some(k) = cycle_param(parameters)? {
            return self.finish_cycle(invocation, txn, (actor, tv, ordinal, k, now), rig, context);
        }
        // 1. The act in flight, this one, this operator's, and due.
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that act is already finished",
            ));
        };
        let Some(act) = record.act.clone().filter(|act| act.ordinal == ordinal) else {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that act is already finished",
            ));
        };
        if act.operator != actor {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that act is another pilot's",
            ));
        }
        if act.due_ic > now {
            return Err(port_error(ActionErrorCode::Conflict, "not due"));
        }
        // The charge at the act's stamp: through the battery when its row resolves; without
        // it a zero spend that does not leave the bay settles (`settle_free`), and — since cut
        // SM.4a's pre-commit round (item 2) — so does a spend that stays out of the bay (a scan
        // or a sounding from the ground: `settle_spent_out_of_bay`, which is what `spent` gives
        // out of the bay), so a recover and a ground reading land with the row unresolvable and
        // a launch (or an implied launch) fails closed (§4.1 "never stranded by its battery";
        // the fixer round's item 3). A dump never touches it.
        let battery = rig
            .battery_id
            .as_ref()
            .and_then(|id| battery_of(&self.tools, id));
        let settle = |charge: &Charge, in_bay: bool| -> Result<Charge, ActionError> {
            match battery {
                Some(battery) => spent(charge, &act, battery, in_bay),
                None if act.reserved_cu == 0 => {
                    settle_free(charge, act.due_ic, in_bay).ok_or_else(|| {
                        port_error(ActionErrorCode::Integrity, "that rig cannot be read")
                    })
                }
                None if !charge.in_bay && !in_bay => {
                    settle_spent_out_of_bay(charge, act.reserved_cu, act.due_ic).ok_or_else(|| {
                        port_error(
                            ActionErrorCode::Integrity,
                            "the act spent more than the charge",
                        )
                    })
                }
                None => Err(port_error(
                    ActionErrorCode::Integrity,
                    "that rig cannot be read",
                )),
            }
        };
        // 2. Decide from the facts at landing and the kind (decision 5).
        let mut moved: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        let mut span: Option<ReadSpan> = None;
        let mut arrived: Option<[i64; 2]> = None;
        let mut stays: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        let working = crate::detachment::sortie::surface::working_preimage(
            context,
            txn,
            &record,
            matches!(
                act.kind,
                ActKind::Drive | ActKind::Recover | ActKind::Launch
            ),
        )?;
        let result = match act.kind {
            ActKind::Scan | ActKind::Sound | ActKind::Dump | ActKind::Unload if !working => {
                ActResult::Refused {
                    code: RefusedCode::Unworked,
                }
            }
            ActKind::Launch => {
                let beside = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cells: Some(cells), .. }
                        if *site == act.site && *cells <= 1
                );
                if beside {
                    record.charge = settle(&record.charge, false)?;
                    ActResult::Done
                } else {
                    // The excursion closes; the charge is untouched (the reservation
                    // released).
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                }
            }
            ActKind::Recover => {
                if matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull) {
                    // Doc 20 cut SM.8 (lead ruling 1, decision R1): the recover's `Done` files the
                    // unload when the hopper holds ore — a durable queue on the record the surface
                    // tick starts as the operator (a provider never submits). A hopper that will
                    // not read queues nothing (the departure's `HOPPER_LOADED` then names the hand
                    // unload).
                    let loaded = match &rig.hopper {
                        Some(hopper) => {
                            let mut any = false;
                            for good in &hopper.goods {
                                match self.inventory.pool(
                                    txn,
                                    EndpointRef(VaultId(hopper.vault)),
                                    good,
                                ) {
                                    Ok(found) => {
                                        any |= found.is_some_and(|pool| pool.available > 0);
                                    }
                                    Err(error) => {
                                        tracing::warn!(tv = tv.0, %error, "tacsim: a recover's finish could not read the hopper; no unload is queued");
                                        any = false;
                                        break;
                                    }
                                }
                            }
                            any
                        }
                        None => false,
                    };
                    if loaded {
                        record.queued = Some(Queued {
                            kind: ActKind::Unload,
                            operator: act.operator,
                            hull: act.hull,
                            slot: act.slot,
                            site: act.site.clone(),
                            after: ordinal,
                            at_ic: act.due_ic,
                            excursion: record.excursion.as_ref().map(|open| open.id),
                        });
                        // Doc 20 cut SM.11 (the reviewer's F9): a plan's recover that brought ore
                        // home is a trip; an empty one is not.
                        tally(&mut record, act.by, |tally| {
                            tally.trips = tally.trips.saturating_add(1);
                        });
                    }
                    record.excursion = None;
                    // Doc 20 cut SM.4b (§3.6 of the brief): a recover that drove home spends its
                    // leg's charge off the record's own value — no battery read (§4.1) — and the
                    // bay's recharge starts at its due instant.
                    record.charge = if act.params.leg().is_some() {
                        Charge {
                            value_cu: record.charge.value_cu.saturating_sub(act.reserved_cu),
                            at_ic: act.due_ic,
                            in_bay: true,
                        }
                    } else {
                        settle(&record.charge, true)?
                    };
                    ActResult::Done
                } else {
                    ActResult::Refused {
                        code: RefusedCode::NotBeside,
                    }
                }
            }
            // Doc 20 cut SM.4b (§3.7 of the brief): a drive is `Done` when the rig stands, at
            // landing, on the act's site at the leg's end cell — the tick placed it there at the
            // due instant — and then the charge settles: from the ground the reservation off the
            // value (`settle_spent_out_of_bay`, no battery read: SM.4a's ground rule), from the bay
            // the bay's credit to the end of the launch half and then the leg's cost
            // (`spent_after_launch`). An implied launch whose rig never left the bay is
            // `NeverLeft` (the excursion closes); a from-bay drive moved after leaving keeps its
            // excursion and leaves the bay at the launch half (`left_after_launch`), `Moved`; a
            // ground drive found anywhere but its end is `Moved`, nothing spent.
            ActKind::Drive => {
                let from_bay = act.params.launches();
                let to = act.params.leg().map(|(_, to)| to);
                let arrived_there = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cell: Some(cell), .. }
                        if *site == act.site && Some(*cell) == to
                );
                if arrived_there {
                    record.charge = if from_bay {
                        match battery {
                            Some(battery) => spent_after_launch(&record.charge, &act, battery)?,
                            None => {
                                return Err(port_error(
                                    ActionErrorCode::Integrity,
                                    "that rig cannot be read",
                                ));
                            }
                        }
                    } else {
                        settle_spent_out_of_bay(&record.charge, act.reserved_cu, act.due_ic)
                            .ok_or_else(|| {
                                port_error(
                                    ActionErrorCode::Integrity,
                                    "the act spent more than the charge",
                                )
                            })?
                    };
                    arrived = to;
                    ActResult::Done
                } else if from_bay && matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull)
                {
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                } else {
                    if from_bay && !matches!(&rig.facts, Facts::Bay { .. }) {
                        record.charge = left_after_launch(&record.charge, &act, battery);
                    }
                    ActResult::Refused {
                        code: RefusedCode::Moved,
                    }
                }
            }
            ActKind::Dump => {
                let ActParams::Dump {
                    hopper,
                    spoil,
                    goods,
                } = &act.params
                else {
                    return Err(port_error(
                        ActionErrorCode::Integrity,
                        "a dump carries its frozen goods",
                    ));
                };
                let spoil_vault = *spoil;
                let (hopper, spoil) = (EndpointRef(VaultId(*hopper)), EndpointRef(VaultId(*spoil)));
                let mut holds = true;
                for (good, qty) in goods {
                    let available = self
                        .inventory
                        .pool(txn, hopper, good)
                        .map_err(|_| {
                            port_error(ActionErrorCode::Integrity, "the hopper cannot be read")
                        })?
                        .map_or(0, |pool| pool.available);
                    if available < *qty {
                        holds = false;
                    }
                }
                // The spoil, before anything is staged (the fixer round's item 4): still the
                // site's live spoil (the offer's fact), every pool on it readable (a read stages
                // nothing), and no earlier finish of this dump (its own ordinal, `failed`) refused
                // mid-transfer. Any of them records `SpoilGone` with nothing moved: the bundle
                // then holds only the record, so the hopper is exactly as it was.
                let why = if !holds {
                    None
                } else if rig.spoil != Some(spoil_vault) {
                    Some("the site's spoil is gone or no longer the site's")
                } else if rig.failed.contains(&(act.ordinal, 0)) {
                    Some("an earlier finish of this dump refused mid-transfer")
                } else if goods
                    .keys()
                    .any(|good| self.inventory.pool(txn, spoil, good).is_err())
                {
                    Some("a spoil pool will not read")
                } else {
                    None
                };
                if !holds {
                    ActResult::Refused {
                        code: RefusedCode::HopperChanged,
                    }
                } else if let Some(why) = why {
                    tracing::error!(
                        tv = tv.0,
                        act = ordinal,
                        spoil = spoil_vault,
                        site = %act.site,
                        why,
                        "tacsim: a dump's spoil would not take the load; the finish records it \
                         refused (SpoilGone) and nothing moved; staff check the site's spoil \
                         vault and its pools"
                    );
                    ActResult::Refused {
                        code: RefusedCode::SpoilGone,
                    }
                } else {
                    // Doc 20 cut SM.6 (lead ruling 5; SM.3b's round F3): ONE
                    // `transfer_many`, which stages the spoil's pool census once for every
                    // new pool, so a two-ore hopper dumps onto a fresh spoil in one bundle.
                    // A refusal here cannot be recorded in this bundle (a debit may be
                    // staged): the whole bundle is refused, nothing is written, and the
                    // tick's next finish records `SpoilGone` (`failed`).
                    let moves: Vec<(CanonicalId, u64)> = goods
                        .iter()
                        .filter(|(_, qty)| **qty > 0)
                        .map(|(good, qty)| (good.clone(), *qty))
                        .collect();
                    if !moves.is_empty() {
                        self.inventory
                            .transfer_many(txn, hopper, spoil, &moves)
                            .map_err(|error| {
                                tracing::warn!(
                                    tv = tv.0,
                                    act = ordinal,
                                    spoil = spoil_vault,
                                    goods = moves.len(),
                                    %error,
                                    "tacsim: a dump's transfer onto the spoil refused; the next \
                                     finish records it refused"
                                );
                                port_error(
                                    ActionErrorCode::ProviderRejected,
                                    "the hopper could not be emptied onto the spoil",
                                )
                            })?;
                    }
                    moved.clone_from(goods);
                    ActResult::Done
                }
            }
            // Doc 20 cut SM.4a (D14, D22): a scan or a sounding is `Done` when the rig stands,
            // at landing, on the act's site at the frozen stand cell — the tick placed an implied
            // launch's rig at the ramp — and then the charge settles out of the bay and the
            // operator's chart is written in this bundle, stamped at the act's own `due_ic`. An
            // implied launch whose rig never left the bay is `NeverLeft` (the excursion closes,
            // the launch's rule); anything else is `Moved`. Neither refusal spends charge or
            // writes the chart.
            ActKind::Scan | ActKind::Sound => {
                let from_bay = act.params.launches();
                let stand = act.params.stand();
                let at_stand = matches!(
                    &rig.facts,
                    Facts::Ground { site: Some(site), cell: Some(cell), .. }
                        if *site == act.site && Some(*cell) == stand
                );
                if at_stand {
                    record.charge = if from_bay {
                        match battery {
                            Some(battery) => spent_after_launch(&record.charge, &act, battery)?,
                            None => {
                                return Err(port_error(
                                    ActionErrorCode::Integrity,
                                    "that rig cannot be read",
                                ));
                            }
                        }
                    } else {
                        settle(&record.charge, false)?
                    };
                    span = Some(write_reading(txn, &self.charts, &act)?);
                    let survey_offer: crate::surface::survey::ReadingOffer = parameters
                        .get("surveys")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .ok_or_else(|| invalid("a reading's offer fixes its survey commissions"))?;
                    let wall = parameters
                        .get("wall")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| invalid("a reading's offer fixes its wall instant"))?;
                    crate::surface::survey::log_reading(
                        txn,
                        &self.surveys,
                        &survey_offer,
                        &act,
                        tv,
                        wall,
                    )?;
                    if let Err(error) = crate::quest::evidence::log_survey(
                        txn,
                        &self.quest_survey_evidence,
                        &act,
                        tv,
                        wall,
                    ) {
                        // Optional lesson evidence must not strand an otherwise
                        // successful scan or sounding. Unreadable proof grants no
                        // lesson progress; ordinary chart and charge effects stand.
                        tracing::warn!(
                            operator = act.operator.0,
                            ?error,
                            "quest survey evidence unavailable"
                        );
                    }
                    ActResult::Done
                } else if from_bay && matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull)
                {
                    record.excursion = None;
                    ActResult::Refused {
                        code: RefusedCode::NeverLeft,
                    }
                } else {
                    // An implied launch whose rig left the bay but not for its stand (a staff
                    // move before the due instant): the excursion the start opened stays open
                    // (the rig is out), so the charge leaves the bay too — credited to the end
                    // of the launch half and no further, nothing spent (the SM.4a fixer round's
                    // finding F1; §4.1: out of the bay charge only falls by acts). A rig found
                    // in some bay keeps its charge as it is.
                    if from_bay && !matches!(&rig.facts, Facts::Bay { .. }) {
                        record.charge = left_after_launch(&record.charge, &act, battery);
                    }
                    ActResult::Refused {
                        code: RefusedCode::Moved,
                    }
                }
            }
            // Doc 20 cut SM.7: a run never sits in the act slot (`RigRecord::check`); its cycles
            // finish by their own path.
            ActKind::Mine => {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "a run finishes by its cycle",
                ));
            }
            // Doc 20 cut SM.8 (§7.1; the brief's 3.8).
            ActKind::Unload => {
                let (result, done_moved, done_stays) =
                    self.finish_unload(txn, (tv, ordinal), &act, rig, parameters)?;
                // The unload spends nothing; in the bay the charge recharges through it (SM.3b's
                // lazy charge: with the battery `spent` of a zero reservation, so the bay credits
                // the whole act; without it `settle_free`, unchanged in the bay — §4.1 "never
                // stranded by its battery"). A rig a staff move took out of the bay settles as it
                // stands; leaving the bay with no battery row reads no credit (the stored value
                // stands, `left_after_launch`'s rule).
                let in_bay = matches!(&rig.facts, Facts::Bay { .. });
                record.charge = settle(&record.charge, in_bay).unwrap_or(record.charge);
                // Doc 20 cut SM.11 (the lead's pre-commit round, item 1): a QUEUED unload the
                // hopper's full marker ring refused keeps its queue — re-filed after this unload's
                // ordinal, so the surface tick starts it again under its attempt rule once the
                // ring frees (the start's offer refuses `unload_markers` until then, so nothing
                // lands and nothing more is told). A wait with the record's back-off, never a
                // park: a plan waiting on it resumes by itself. A hand unload's refusal stays
                // "try again later".
                if result
                    == (ActResult::Refused {
                        code: RefusedCode::MarkersFull,
                    })
                    && let ActParams::Unload {
                        how: UnloadHow::Queued,
                        excursion,
                        ..
                    } = &act.params
                {
                    record.queued = Some(Queued {
                        kind: ActKind::Unload,
                        operator: act.operator,
                        hull: act.hull,
                        slot: act.slot,
                        site: act.site.clone(),
                        after: ordinal,
                        at_ic: act.due_ic,
                        excursion: *excursion,
                    });
                }
                moved = done_moved;
                stays = done_stays;
                result
            }
        };
        // 2b. Doc 23 cut S.1 (§6, D-23-1; L3–L6): the seal epoch's moves ride this finish. An
        // act that leaves the bay — a launch, or (an implied launch) a scan, a sounding or a
        // drive from the bay, by the hand, a plan's step or the idle return alike — opens the
        // rig's epoch when its excursion survives the finish with the rig out of every bay; a
        // `NeverLeft` closed the excursion above and opens nothing. A recover's `Done` closes
        // the epoch `ReturnedToBase` into the hull it climbed into. The record moves once per
        // act ordinal, so a re-run finish finds the act gone and writes no second line.
        let line = epoch_move(&mut record, &act, (tv, result), rig)?;
        // The line rides this bundle; an open whose line was not written is recorded on the
        // epoch (the S.1 fix round's F2), in the same record write.
        if let Some(line) = line {
            let opens = matches!(
                line,
                crate::surface::black_box::BlackBoxLine::MindSealed { .. }
            );
            let written =
                crate::surface::black_box::append(context, self.black_box.as_ref(), tv, line)?;
            if opens
                && !written
                && let Some(open) = record.sortie.as_mut()
            {
                open.unrecorded = true;
            }
            // Doc 17h cut M8e E4 (M8c B5): a launch whose licence's mind flies marks the
            // licence's ring in this same bundle — the flight, its hull and whether its core was
            // governed. A seal that fell to the ROM (a lapsed or revoked licence) flew no mind
            // under it and marks nothing (fix round 1, J3).
            if opens
                && let (Some(sealed), Some(open)) = (act.seal.as_ref(), record.sortie.as_ref())
                && let Some(licence) = sealed.mark_licence()
            {
                seal_mark(context, txn, (tv, licence), open, sealed.facts.governed);
            }
        }
        // 3. The act clears; the outcome is stamped at the act's own due instant.
        record.act = None;
        record.last = Some(Outcome {
            ordinal,
            kind: act.kind,
            operator: act.operator,
            slot: act.slot,
            site: act.site.clone(),
            at_ic: act.due_ic,
            result,
            moved,
            span,
            arrived,
            mined: None,
            stays,
            by: act.by,
        });
        crate::detachment::sortie::work::finish(context, txn, &mut record)?;
        crate::detachment::sortie::surface::reconcile_return(context, txn, &mut record)
            .map_err(|_| invalid("field return unavailable"))?;
        write_record(txn, &self.excursions, record, true)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": actor.0,
                "ordinal": ordinal,
                "result": match result {
                    ActResult::Done => "done",
                    ActResult::Refused { .. } => "refused",
                },
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the finish's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl SurfaceDoneProvider {
    /// **An unload's finish** (doc 20 §7.1; cut SM.8, the brief's 3.8; lead rulings 2 and 3,
    /// decisions R3 and R8), decided before anything is staged, then at most ONE inventory call:
    ///
    /// 1. **in the bay** — the rig in its act's hull's bay at landing, else `Refused { LeftBay }`;
    /// 2. **the hopper** — each frozen good's `available`, read here, at least its quantity, else
    ///    `Refused { HopperChanged }`;
    /// 3. **the hold** — the hull's hold facts present with the frozen vault, the plan's goods
    ///    table and its one ore compartment, the compartment admitting every frozen good
    ///    cumulatively on top of the hold's pools read here (`capacity::admits`), and no earlier
    ///    finish of this unload refused mid-bundle (`DoneRig::failed` holding `(ordinal, 0)`: the
    ///    dump's `SpoilGone` precedent) — any failing → `Refused { HoldRefused }`, nothing staged,
    ///    paged (staff state only: §6's gate keeps an honest fill inside the hold's room);
    /// 4. **the move** — ONE `InventoryFamilies::transfer_many_marked_keeping` hopper → hold (its
    ///    ring entries kept [`UNLOAD_MARKER_KEEP_S`], doc 20 cut SM.12c decision (c)), one
    ///    `inventory.transferred` marker a good under `crate::surface::unload::marker`'s id, stamped
    ///    at the act's due on the offer's IC clock; the hopper's marker ring full of young markers
    ///    (`MarkerRingFull`, decided there before anything is staged) is `Refused { MarkersFull }`
    ///    (the fix round's correctness/F2: never told as a hold that would not take the load);
    ///    any other refusal there refuses the whole bundle (nothing is written) and the next
    ///    finish records `HoldRefused` through `failed`.
    ///
    /// Returns the result, what moved and — for a `Done` queued or whole unload — what stayed:
    /// each hopper good's availability before the move less what moved, when positive. A queued
    /// unload with nothing to move (the hold had no room) is `Done` with every good staying.
    #[allow(clippy::type_complexity)]
    pub(super) fn finish_unload(
        &self,
        txn: &mut ActionTxn<'_>,
        (tv, ordinal): (ObjId, u32),
        act: &Act,
        rig: &DoneRig,
        parameters: &Map<String, Value>,
    ) -> Result<
        (
            ActResult,
            BTreeMap<CanonicalId, u64>,
            BTreeMap<CanonicalId, u64>,
        ),
        ActionError,
    > {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let refused = |code: RefusedCode| {
            Ok((
                ActResult::Refused { code },
                BTreeMap::new(),
                BTreeMap::new(),
            ))
        };
        let ActParams::Unload {
            hopper,
            hold,
            goods,
            how,
            excursion,
        } = &act.params
        else {
            return Err(integrity("an unload carries its frozen goods"));
        };
        // 1. In the bay.
        if !matches!(&rig.facts, Facts::Bay { hull } if *hull == act.hull) {
            return refused(RefusedCode::LeftBay);
        }
        // 2. The hopper: every good's availability now (the frozen ones must hold).
        let source = EndpointRef(VaultId(*hopper));
        let mut accepted: Vec<CanonicalId> = rig
            .hopper
            .as_ref()
            .filter(|facts| facts.vault == *hopper)
            .map(|facts| facts.goods.clone())
            .unwrap_or_default();
        for good in goods.keys() {
            if !accepted.contains(good) {
                accepted.push(good.clone());
            }
        }
        if accepted.len() > mush_inventory::MAX_POOLS_PER_ENDPOINT {
            return Err(invalid(
                "a finish's offer fixes a bounded set of hopper goods",
            ));
        }
        let mut before: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        for good in &accepted {
            let available = self
                .inventory
                .pool(txn, source, good)
                .map_err(|_| integrity("the hopper cannot be read"))?
                .map_or(0, |pool| pool.available);
            before.insert(good.clone(), available);
        }
        if goods
            .iter()
            .any(|(good, qty)| before.get(good).copied().unwrap_or(0) < *qty)
        {
            return refused(RefusedCode::HopperChanged);
        }
        if !goods.is_empty() {
            // 3. The hold, before anything is staged.
            let destination = EndpointRef(VaultId(*hold));
            let why = match rig.hold.as_ref() {
                None => Some("the hull's hold cannot be read"),
                Some(facts) if facts.vault != *hold => {
                    Some("the hull's hold is no longer that vault")
                }
                Some(_) if rig.failed.contains(&(act.ordinal, 0)) => {
                    Some("an earlier finish of this unload refused mid-transfer")
                }
                Some(facts) => {
                    match crate::port::market::plan_goods(&facts.plan).and_then(|table| {
                        let ore = CanonicalId::new("metal_ore").ok()?;
                        crate::port::capacity::compartment_for(
                            table,
                            &crate::port::market::plan_compartments(&facts.plan),
                            &ore,
                        )
                        .map(|compartment| (table, compartment))
                    }) {
                        None => Some("the hold's plan cannot be read"),
                        Some((table, compartment)) => {
                            let mut contents: BTreeMap<CanonicalId, u64> = BTreeMap::new();
                            let mut unread = false;
                            for good in table
                                .goods
                                .iter()
                                .filter(|good| compartment.accepts.contains(&good.stowage))
                            {
                                match self.inventory.pool(txn, destination, &good.id) {
                                    Ok(found) => {
                                        let held = found.map_or(0, |pool| pool.quantity);
                                        if held > 0 {
                                            contents.insert(good.id.clone(), held);
                                        }
                                    }
                                    Err(_) => unread = true,
                                }
                            }
                            let mut admitted = !unread;
                            for (good, qty) in goods {
                                if !admitted {
                                    break;
                                }
                                match crate::port::capacity::admits(
                                    table,
                                    &compartment,
                                    &contents,
                                    (good, *qty),
                                ) {
                                    crate::port::capacity::Admission::Admitted(_) => {
                                        let slot = contents.entry(good.clone()).or_insert(0);
                                        *slot = slot.saturating_add(*qty);
                                    }
                                    crate::port::capacity::Admission::Refused(_)
                                    | crate::port::capacity::Admission::Error(_) => {
                                        admitted = false
                                    }
                                }
                            }
                            (!admitted).then_some(if unread {
                                "a hold pool will not read"
                            } else {
                                "the hold's ore compartment would not admit the load"
                            })
                        }
                    }
                }
            };
            if let Some(why) = why {
                tracing::error!(
                    tv = tv.0,
                    act = ordinal,
                    hold = *hold,
                    why,
                    "tacsim: an unload's hold would not take the load; the finish records it \
                     refused (HoldRefused) and nothing moved; staff check the hull's hold and the \
                     rig's hopper"
                );
                return refused(RefusedCode::HoldRefused);
            }
            // 4. The move: ONE marked transfer, one marker a good.
            let clock = parameters
                .get("clock")
                .and_then(|value| {
                    serde_json::from_value::<mush_core::ClockStamp>(value.clone()).ok()
                })
                .ok_or_else(|| invalid("a finish's offer fixes the clock"))?;
            let wall = parameters
                .get("wall")
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid("a finish's offer fixes the real second"))?;
            let mut moves: Vec<(CanonicalId, u64, smol_str::SmolStr)> = Vec::new();
            for (good, qty) in goods {
                let id = crate::surface::unload::marker(tv, *excursion, ordinal, good, act.due_ic)
                    .map_err(|_| integrity("an unload's marker did not form"))?;
                moves.push((good.clone(), *qty, id));
            }
            let stamp = mush_core::ClockStamp {
                clock: clock.clock,
                at: act.due_ic,
            };
            let moved = self.inventory.transfer_many_marked_keeping(
                txn,
                source,
                destination,
                &moves,
                act.operator,
                &stamp,
                wall,
                UNLOAD_MARKER_KEEP_S,
            );
            if matches!(moved, Err(mush_inventory::InventoryError::MarkerRingFull)) {
                // Decided before anything was staged (`transfer_many_marked`'s contract), so the
                // refusal is this finish's outcome. The ring is the hopper vault's and outlives a
                // lease (the fix round's correctness/F2; the brief's ring-bound residual).
                tracing::error!(
                    tv = tv.0,
                    act = ordinal,
                    hopper = *hopper,
                    "tacsim: an unload's hopper marker ring is full of young markers; the finish \
                     records it refused (MarkersFull) and nothing moved; the ring frees as its \
                     markers pass the retention horizon"
                );
                return refused(RefusedCode::MarkersFull);
            }
            moved.map_err(|error| {
                tracing::warn!(
                    tv = tv.0,
                    act = ordinal,
                    hold = *hold,
                    goods = moves.len(),
                    %error,
                    "tacsim: an unload's transfer into the hold refused; the next finish \
                     records it refused"
                );
                port_error(
                    ActionErrorCode::ProviderRejected,
                    "the hopper could not be unloaded into the hold",
                )
            })?;
        }
        // The outcome: what moved, and for a queued or whole unload what stays.
        let mut stays: BTreeMap<CanonicalId, u64> = BTreeMap::new();
        if matches!(how, UnloadHow::Queued | UnloadHow::Whole) {
            for (good, available) in &before {
                let left = available.saturating_sub(goods.get(good).copied().unwrap_or(0));
                if left > 0 {
                    stays.insert(good.clone(), left);
                }
            }
        }
        Ok((ActResult::Done, goods.clone(), stays))
    }

    /// **One mining cycle's finish** (doc 20 §6 "the completion"; cut SM.7, the brief's 3.7; lead
    /// rulings 1, 5, 6 and 7), **one bundle** (≤ 8 writes, §11's Tick row):
    ///
    /// 1. **this cycle**: the run of the invocation's ordinal at cycle `k`, the actor's, due — else
    ///    `Conflict` "that cycle is already finished" (the replay guard behind the key's HIT);
    /// 2. **at the stand**: the rig on the run's site at its frozen stand cell, else `Moved` —
    ///    nothing moved, nothing spent, the chart untouched, the run closed;
    /// 3. **the yield**: `mine::cycle_yield` over the family's cursor row read here; tonnes to move
    ///    need the block vault to hold them (`available`), else `Unworked` with nothing staged (and
    ///    logged); then ONE `InventoryFamilies::transfer` of the family's ore good from the block
    ///    vault to the hopper — a refusal there refuses the whole bundle, and the tick's next finish
    ///    of the same `(ordinal, k)`, marked `failed`, records `Unworked` with nothing moved (the
    ///    dump's `SpoilGone` precedent);
    /// 4. **the end**: `mine::run_end` — refused, worked out, stopped, the next cycle's rights
    ///    (`DoneRig::rights`, read at landing: the licence lapsing at or before this cycle's due
    ///    instant, the block's tenure, the lease), the run's last cycle — or re-arm;
    /// 5. **the cursor**: the family's row drawn by the yield, its lock kept while the run re-arms
    ///    (`mine::cursor_after`); at a close every other lock row released (`cursor_released`); a
    ///    barren core writes no cursor of its family (lead ruling 2);
    /// 6. **the charge**: `settle_spent_out_of_bay` of one cycle's cost at its due instant — no
    ///    battery read (SM.4b's ground-drive precedent); a refused cycle spends nothing;
    /// 7. **the chart** (a `Done` cycle): the operator's row — created with the run's body when they
    ///    have none — gains the pocket's R2 with `PIT_SEEN` (`chart::write_core`);
    /// 8. **re-arm or close**, and the outcome (`Outcome::mined`). The action's outcome keeps the
    ///    finish's shape: nothing of the class, the tonnes or the deposit.
    ///
    /// The running cycle always yields: rights never undo it (§8).
    #[allow(clippy::too_many_lines)] // one ordered list, then one bundle
    pub(super) fn finish_cycle(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        (actor, tv, ordinal, k, now): (ObjId, ObjId, u32, u8, i64),
        rig: &DoneRig,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let finished = || port_error(ActionErrorCode::Conflict, "that cycle is already finished");
        // 1. This cycle.
        let Some(mut record) = read_record(txn, &self.excursions, tv)? else {
            return Err(finished());
        };
        let Some(mut run) = record
            .run
            .clone()
            .filter(|run| run.ordinal == ordinal && run.k == k)
        else {
            return Err(finished());
        };
        if run.operator != actor {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that run is another pilot's",
            ));
        }
        if run.due_ic > now {
            return Err(port_error(ActionErrorCode::Conflict, "not due"));
        }
        let rights = rig
            .rights
            .clone()
            .ok_or_else(|| integrity("a run's next rights are read at landing"))?;
        // 2. At the stand.
        let working =
            crate::detachment::sortie::surface::working_preimage(context, txn, &record, false)?;
        let at_stand = working
            && matches!(
                &rig.facts,
                Facts::Ground { site: Some(site), cell: Some(cell), .. }
                    if *site == run.site && *cell == run.stand
            );
        // 3. The yield.
        let own = read_cursor(txn, &self.pockets, run.deposit)?;
        let drawn = own
            .as_ref()
            .and_then(|cursor| cursor.row(run.number))
            .map_or(0, |row| row.drawn_t);
        let mut result = ActResult::Done;
        let mut tonnes = 0_u32;
        if at_stand {
            let y = cycle_yield(run.class_t, run.pocket_t, drawn);
            if y > 0 {
                let source =
                    EndpointRef(VaultId(run.source.ok_or_else(|| {
                        integrity("an ore-bearing run names its block vault")
                    })?));
                let available = self
                    .inventory
                    .pool(txn, source, &run.good)
                    .map_err(|_| integrity("the block's ground cannot be read"))?
                    .map_or(0, |pool| pool.available);
                let retried = rig.failed.contains(&(ordinal, k));
                if available < u64::from(y) || retried {
                    tracing::error!(
                        tv = tv.0,
                        run = ordinal,
                        cycle = k,
                        deposit = %run.deposit,
                        available,
                        wanted = y,
                        retried,
                        runbook = "doc 20 cut SM.7: the cycle is recorded Unworked and nothing \
                                   moved; industry's sweep quarantines the block if its books \
                                   disagree (initial − pool == Σ drawn); staff read the block \
                                   vault's pool and the pocket cursor",
                        "tacsim: a mining cycle's block would not give up its ore"
                    );
                    result = ActResult::Refused {
                        code: RefusedCode::Unworked,
                    };
                } else {
                    self.inventory
                        .transfer(
                            txn,
                            source,
                            EndpointRef(VaultId(run.hopper)),
                            &run.good,
                            u64::from(y),
                        )
                        .map_err(|error| {
                            tracing::warn!(
                                tv = tv.0,
                                run = ordinal,
                                cycle = k,
                                %error,
                                "tacsim: a mining cycle's transfer refused; the next finish \
                                 records it Unworked"
                            );
                            port_error(
                                ActionErrorCode::ProviderRejected,
                                "the block's ore could not be moved to the hopper",
                            )
                        })?;
                    tonnes = y;
                }
            }
        } else {
            result = ActResult::Refused {
                code: if working {
                    RefusedCode::Moved
                } else {
                    RefusedCode::Unworked
                },
            };
        }
        // 4. The end.
        let refused = result != ActResult::Done;
        let drawn_after = drawn.saturating_add(tonnes);
        let worked_out = !refused && u32::from(run.pocket_t).saturating_sub(drawn_after) == 0;
        let end = run_end(
            refused,
            worked_out,
            run.stop,
            run.k == run.cycles,
            &rights,
            run.due_ic,
        );
        let ours = PocketLock { tv, ordinal };
        // 5. The cursor: the family's row drawn and its lock kept or released; at a close every
        // other row this run locked released.
        if !refused {
            let after = cursor_after(
                own.as_ref(),
                run.deposit,
                run.number,
                tonnes,
                &ours,
                end.is_none(),
            )
            .ok_or_else(|| integrity("the pocket's cursor cannot take the draw"))?;
            // A barren core of a family with no row writes nothing (lead ruling 2); nor does any
            // cycle that leaves the cursor as it stood.
            if after != own {
                write_cursor(txn, &self.pockets, run.deposit, own.is_some(), after)?;
            }
        }
        if end.is_some() {
            for row in &run.lock_rows {
                if !refused && row.deposit == run.deposit {
                    continue;
                }
                let before = read_cursor(txn, &self.pockets, row.deposit)?;
                if let Some(before) = before {
                    let after = cursor_released(&before, row.pocket, &ours);
                    if after.as_ref() != Some(&before) {
                        write_cursor(txn, &self.pockets, row.deposit, true, after)?;
                    }
                }
            }
        }
        // 6. The charge, and 7. the chart (a `Done` cycle).
        if !refused {
            record.charge = settle_spent_out_of_bay(&record.charge, run.cost_cu, run.due_ic)
                .ok_or_else(|| integrity("the cycle spent more than the charge"))?;
            run.reserved_cu = run
                .reserved_cu
                .checked_sub(run.cost_cu)
                .ok_or_else(|| integrity("the run's reservation is short of its cycle"))?;
            let existing = read_chart(txn, &self.charts, run.operator, &run.site)?;
            let existed = existing.is_some();
            let mut row = existing.unwrap_or_else(|| {
                ChartRecord::new(run.operator, run.site.clone(), run.body.clone())
            });
            chart::write_core(
                &mut row,
                (run.pocket[0], run.pocket[1]),
                run.due_ic,
                via_of(run.by),
            );
            row.check()
                .map_err(|_| integrity("the chart row is out of bounds"))?;
            let key = row.key();
            let staged = if existed {
                txn.replace(&self.charts, &key, row)
            } else {
                txn.insert(&self.charts, &key, row)
            };
            staged.map_err(|_| integrity("the chart row did not stage"))?;
        }
        // 8. Re-arm or close, and the outcome.
        let mined = Mined {
            k: run.k,
            cycles: run.cycles,
            pocket: run.pocket,
            good: run.good.clone(),
            tonnes,
            blind: run.blind,
            end,
        };
        let at_ic = run.due_ic;
        let (slot, site) = (run.slot, run.site.clone());
        // Doc 20 cut SM.11 (the brief's 3.7): a plan's cycle counts its tonnes and itself.
        let run_by = run.by;
        if !refused {
            tally(&mut record, run_by, |tally| {
                tally.tonnes = tally.tonnes.saturating_add(tonnes);
                tally.cycles = tally.cycles.saturating_add(1);
            });
        }
        record.run = match end {
            Some(_) => None,
            None => {
                run.k = run
                    .k
                    .checked_add(1)
                    .ok_or_else(|| integrity("a run's cycle overflows"))?;
                run.due_ic = run
                    .due_ic
                    .checked_add(i64::from(run.secs))
                    .ok_or_else(|| integrity("a cycle's due instant overflows"))?;
                Some(run)
            }
        };
        record.last = Some(Outcome {
            ordinal,
            kind: ActKind::Mine,
            operator: actor,
            slot,
            site,
            at_ic,
            result,
            moved: BTreeMap::new(),
            span: None,
            arrived: None,
            mined: Some(mined),
            stays: BTreeMap::new(),
            by: run_by,
        });
        crate::detachment::sortie::work::finish(context, txn, &mut record)?;
        crate::detachment::sortie::surface::reconcile_return(context, txn, &mut record)
            .map_err(|_| invalid("field return unavailable"))?;
        write_record(txn, &self.excursions, record, true)?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": actor.0,
                "ordinal": ordinal,
                "result": match result {
                    ActResult::Done => "done",
                    ActResult::Refused { .. } => "refused",
                },
                "tv": tv.0,
            }),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the finish's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}
