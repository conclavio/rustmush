#![cfg(test)]

//! Actual faction commands and registered Engine guards. The tiny typed action
//! records admission only; it is not a Coprates certificate or purchase.
use super::*;
use mush_core::action::{
    ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput, CanonicalHash,
    IdempotencyKey, IdempotencyScope, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::mechanics::RulesValue;
use mush_core::{
    ActionPresentation, ActionTxn, CommandCompatibility, CommandSpec, CommitToken, Engine, GOD,
    GameModule, Inbound, LIMBO, OfferExecutionContext, OfferKeyRing, OfferSecret, OfferSubmission,
    Outbox, PerceivedRef, PrepareCtx, PrincipalId, ProjectedParameterSchema, ProjectedTargetSchema,
    ProviderOfferDraft, Registry, RevisionSet, SubjectKey,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Family = StateFamilyHandle<u32, u32>;

fn catalog() -> CatalogRef {
    CatalogRef {
        ruleset: "tacsim".into(),
        code: "coprates_membership_fixture".into(),
    }
}
fn policy() -> TestResult<CopratesMembership> {
    // Fresh FactionModule's real allocator starts at zero. The fixture checks
    // the actual newly created id; production never chooses an id by this name.
    Ok(CopratesMembership::new(FactionId(0), catalog())?)
}
fn refused() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "Buyer admission unavailable",
    )
    .expect("fixed bounded error")
}
fn descriptor(kind: &str, id: &str, references: Vec<DefinitionRef>) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).expect("kind"),
        DefinitionId::new(id).expect("id"),
        1,
        json!({"fixture":"membership_admission_only"}),
    )
    .expect("descriptor")
    .with_references(references)
}
#[derive(Clone)]
struct Captured {
    action: DefinitionRef,
    family: Family,
    membership: CopratesMembershipHandle,
}
struct FixtureModule {
    capture: Arc<Mutex<Option<Captured>>>,
    previews: Arc<Mutex<Vec<AvailabilityOutcome>>>,
}
impl GameModule for FixtureModule {
    fn name(&self) -> &'static str {
        OWNER
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let policy = policy().map_err(error)?;
        let membership = policy.register(reg)?;
        let action_desc = descriptor(
            "action",
            "coprates_membership_probe",
            vec![membership.definition().clone()],
        );
        let outcome_desc = descriptor("outcome", "coprates_membership_checked", Vec::new());
        for expected in [action_desc.clone(), outcome_desc.clone()] {
            reg.definition_validator(
                expected.kind().clone(),
                move |ctx: &DefinitionValidationContext<'_>| {
                    let row = ctx.definition();
                    if row.reference().key().owner.as_str() == OWNER
                        && row.reference().key().id == *expected.id()
                        && row.reference().version() == 1
                        && row.payload() == expected.payload()
                        && row.references() == expected.references()
                    {
                        Ok(())
                    } else {
                        Err("membership probe definition differs".into())
                    }
                },
            )?;
        }
        let action = reg.definition(action_desc)?;
        let outcome = reg.definition(outcome_desc)?;
        let name = |s| CanonicalId::new(s).expect("fixed codec id");
        let family = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("coprates_admission_probe").map_err(error)?,
                name("coprates_probe.key"),
                name("coprates_probe.value"),
                1,
                name("coprates_probe.decode"),
                name("coprates_probe.check"),
                name("coprates_probe.encode"),
            )
            .map_err(error)?,
            |key: &u32| Ok(key.to_string()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |_: &u32| Ok(()),
            |value: &u32| Ok(json!(value)),
        )?;
        reg.action_provider(
            action.clone(),
            Provider {
                action: action.clone(),
                outcome,
                family: family.clone(),
                membership: membership.clone(),
            },
        )?;
        reg.classify_commands(CommandCompatibility::ReadOnly);
        let previews = self.previews.clone();
        reg.command(
            CommandSpec {
                name: "coprates_preview".into(),
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let actor = ctx.actor;
                let rule = if ctx.args.raw() == "wrong_rule" {
                    AvailabilityRule::None
                } else {
                    policy.availability_rule()
                };
                let mut selected = catalog();
                if ctx.args.raw() == "foreign_catalog" {
                    selected.code = "other".into();
                }
                let result = policy
                    .resolve(ctx, &rule, &selected, actor)
                    .ok_or_else(|| mush_core::CmdError::msg("preview unavailable"))?;
                previews
                    .lock()
                    .map_err(|_| mush_core::CmdError::msg("preview unavailable"))?
                    .push(result);
                Ok(())
            }),
        )?;
        *self.capture.lock().map_err(error)? = Some(Captured {
            action,
            family,
            membership,
        });
        Ok(())
    }
}
struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    family: Family,
    membership: CopratesMembershipHandle,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.parameters() != &json!({})
        {
            return Err(refused());
        }
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(inv)?;
        self.membership
            .guard(ctx.world().map_err(|_| refused())?, &catalog(), inv.actor())
            .ok_or_else(refused)?;
        txn.insert(&self.family, &inv.actor().0, 1)
            .map_err(|_| refused())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({"admitted":true}))
                .map_err(|_| refused())?,
        )
        .with_definitions(vec![
            self.action.clone(),
            self.outcome.clone(),
            self.membership.definition().clone(),
        ])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some(guard) = self
            .membership
            .guard(&engine.world, &catalog(), query.actor())
        else {
            return Ok(None);
        };
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| refused())?;
        let defs = engine.registry.definitions().ok_or_else(refused)?;
        for reference in [&self.action, self.membership.definition()] {
            guards
                .add_definition(reference.clone(), defs)
                .map_err(|_| refused())?;
        }
        guards.add(guard).map_err(|_| refused())?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::new(Vec::new()).map_err(|_| refused())?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "coprates-membership-probe",
                "Membership admission probe",
                "Fixture only; no commercial grant.",
            )
            .map_err(|_| refused())?,
            guards: guards.finish().map_err(|_| refused())?,
        }))
    }
}
struct Rig {
    engine: Box<Engine>,
    buyer: ObjId,
    hidden: ObjId,
    handles: Captured,
    previews: Arc<Mutex<Vec<AvailabilityOutcome>>>,
}
impl Rig {
    fn new() -> TestResult<Self> {
        let mut world = World::new_minimal();
        world.set_now(1000);
        let buyer = world.create(ObjKind::Player, "CopratesBuyer", GOD, Some(LIMBO))?;
        let hidden = world.create(ObjKind::Player, "CopratesHidden", GOD, Some(LIMBO))?;
        let capture = Arc::new(Mutex::new(None));
        let previews = Arc::new(Mutex::new(Vec::new()));
        let mut registry = Registry::new();
        mush_core::register_core_commands(&mut registry)?;
        mush_core::register_engine_commands(&mut registry)?;
        let mut engine = boxed_fixture_engine(
            world,
            registry,
            vec![
                Box::new(mush_faction::FactionModule::new()),
                Box::new(FixtureModule {
                    capture: capture.clone(),
                    previews: previews.clone(),
                }),
            ],
        )?;
        engine.modules_loaded(
            HashMap::new(),
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x43; 32])?, [])?);
        let handles = capture
            .lock()
            .map_err(|_| "capture lock")?
            .clone()
            .ok_or("handles")?;
        let mut r = Self {
            engine,
            buyer,
            hidden,
            handles,
            previews,
        };
        let out = r.command(GOD, "@faction/create MembershipFixture");
        check!(out.events().iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.to_plain_text().contains("Created faction MembershipFixture"))));
        let state = r
            .engine
            .world
            .module_state::<FactionState>()
            .ok_or("faction")?;
        check_eq!(
            state
                .faction_by_name("MembershipFixture")
                .ok_or("created")?
                .id,
            FactionId(0)
        );
        r.flush();
        Ok(r)
    }
    fn command(&mut self, actor: ObjId, line: &str) -> Outbox {
        let mut out = Outbox::new();
        self.engine.step(
            Inbound::Cmd {
                conn: 1,
                actor,
                line: line.into(),
            },
            &mut out,
            &mut mush_core::engine::Effects::new(),
        );
        out
    }
    fn flush(&mut self) {
        let _ = self.engine.world.take_dirty();
        let _ = self.engine.take_module_ops();
    }
    fn add(&mut self, hidden: bool) {
        self.command(
            GOD,
            if hidden {
                "@faction/add MembershipFixture=CopratesHidden/level:1/vis:covert"
            } else {
                "@faction/add MembershipFixture=CopratesBuyer/level:1"
            },
        );
        assert!(self.handles.membership.policy.current(
            &self.engine.world,
            if hidden { self.hidden } else { self.buyer }
        ));
        self.flush();
    }
    fn remove(&mut self) {
        self.command(GOD, "@faction/remove MembershipFixture=CopratesBuyer");
        assert!(
            !self
                .handles
                .membership
                .policy
                .current(&self.engine.world, self.buyer)
        );
        self.flush();
    }
    fn resolved(&self) -> TestResult<mush_core::ResolvedAuthoritativeOffer> {
        let principal = PrincipalId::Object { id: self.buyer };
        let query = self.engine.legal_action_query(
            principal.clone(),
            self.buyer,
            PerceivedRef::SelfActor,
        )?;
        let offers = self.engine.project_legal_actions(&query)?;
        let projected = offers
            .projected_for_action(&self.handles.action)
            .ok_or("eligible offer")?;
        Ok(self.engine.resolve_legal_action(
            &offers,
            OfferExecutionContext {
                request: RequestUid::from_bytes([7; 16]),
                principal: principal.clone(),
                actor: self.buyer,
                idempotency: ScopedIdempotencyKey::new(
                    IdempotencyScope {
                        principal,
                        owner: OwnerId::new(OWNER)?,
                        namespace: CanonicalId::new("coprates_membership_probe")?,
                    },
                    1,
                    IdempotencyKey::Caller(mush_core::IdempotencyKeyText::parse_canonical(
                        "membership.probe",
                    )?),
                ),
                viewer_scope: query.viewer_scope(),
                now: None,
            },
            OfferSubmission {
                offer: projected.offer(),
                subject: PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters: json!({}),
            },
        )?)
    }
    fn unchanged(&self) {
        assert_eq!(
            self.engine
                .action_state()
                .get(&self.handles.family, &self.buyer.0)
                .expect("typed read"),
            None
        );
    }
}
fn bundle(
    action: &DefinitionRef,
    resolved: &mush_core::ResolvedActionInvocation,
    prepared: &mush_core::composed::PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult<AuthoritativeBundle> {
    let inv = resolved.invocation();
    let token = CommitToken::from_bytes([9; 16]);
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    Ok(AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([1; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([2; 32]),
        action: action.clone(),
        provider: OwnerId::new(OWNER)?,
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token)),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?)
}

#[test]
fn actual_member_preview_accepts_non_officers_and_covert_members_without_disclosure() -> TestResult
{
    let mut r = Rig::new()?;
    r.add(false);
    r.add(true);
    for actor in [r.buyer, r.hidden, GOD] {
        r.command(actor, "coprates_preview");
    }
    let answers = r.previews.lock().map_err(|_| "previews")?;
    check_eq!(
        answers.iter().map(|a| a.passed).collect::<Vec<_>>(),
        vec![true, true, false]
    );
    check!(
        answers
            .iter()
            .all(|a| a.note == "Coprates buyer eligibility checked")
    );
    check_eq!(
        answers.first().ok_or("first")?,
        answers.get(1).ok_or("second")?
    );
    let st = r
        .engine
        .world
        .module_state::<FactionState>()
        .ok_or("state")?;
    check!(!st.is_officer(r.buyer, FactionId(0)));
    check!(!st.is_officer(r.hidden, FactionId(0)));
    r.unchanged();
    Ok(())
}

#[test]
fn exact_policy_coordinates_and_actual_owner_state_fail_closed() -> TestResult {
    let mut r = Rig::new()?;
    r.add(false);
    let other = CatalogRef {
        ruleset: "tacsim".into(),
        code: "other".into(),
    };
    check!(
        r.handles
            .membership
            .guard(&r.engine.world, &other, r.buyer)
            .is_none()
    );
    let unknown = CopratesMembership::new(FactionId(u64::MAX), catalog())?;
    check!(!unknown.current(&r.engine.world, r.buyer));
    check!(!policy()?.current(&World::new_minimal(), GOD));
    let object = r
        .engine
        .world
        .create(ObjKind::Thing, "MembershipNonPlayer", GOD, Some(LIMBO))?;
    check!(!policy()?.current(&r.engine.world, object));
    r.engine
        .world
        .get_mut(r.buyer)
        .ok_or("buyer")?
        .flags
        .insert(Flags::GOING);
    check!(
        r.handles
            .membership
            .guard(&r.engine.world, &catalog(), r.buyer)
            .is_none()
    );
    check_ne!(
        policy()?.descriptor()?.payload(),
        unknown.descriptor()?.payload()
    );
    check!(
        CopratesMembership::new(
            FactionId(0),
            CatalogRef {
                ruleset: "".into(),
                code: "x".into()
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn membership_loss_after_preview_refuses_actual_registered_preparation() -> TestResult {
    let mut r = Rig::new()?;
    r.add(false);
    let offer = r.resolved()?;
    r.remove();
    check!(!r.engine.revalidate_guards(offer.guards())?.is_current());
    check!(
        r.engine
            .prepare_registered_composed_resolved_authoritative_action(
                offer.action(),
                offer.resolved_invocation(),
                CommitToken::from_bytes([9; 16])
            )
            .is_err()
    );
    r.unchanged();
    check!(!r.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn membership_loss_after_prepare_or_stage_refuses_before_writer_custody() -> TestResult {
    for after_stage in [false, true] {
        let mut r = Rig::new()?;
        r.add(false);
        let (action, resolved, guards) = r.resolved()?.into_authoritative_parts();
        let (prepared, output) = r
            .engine
            .prepare_registered_composed_resolved_authoritative_action(
                &action,
                &resolved,
                CommitToken::from_bytes([9; 16]),
            )?;
        let bundle = bundle(&action, &resolved, &prepared, output)?;
        r.unchanged();
        if !after_stage {
            r.remove();
        }
        r.engine
            .stage_authoritative_action(prepared, guards, bundle)?;
        if after_stage {
            r.remove();
        }
        check!(r.engine.take_staged_action().is_err());
        r.unchanged();
        check!(!r.engine.has_staged_action());
        check!(!r.engine.action_state().has_prepared());
        check!(!r.engine.action_state().reservation_lost());
    }
    Ok(())
}

#[test]
fn unchanged_member_reaches_actual_custody_with_exact_guard_and_inert_images() -> TestResult {
    let mut r = Rig::new()?;
    r.add(false);
    let (action, resolved, guards) = r.resolved()?.into_authoritative_parts();
    let (prepared, output) = r
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([9; 16]),
        )?;
    let bundle = bundle(&action, &resolved, &prepared, output)?;
    r.engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    let staged = r.engine.take_staged_action()?.ok_or("staged")?;
    r.unchanged();
    let (_, installation) = staged.into_parts();
    r.engine.install_composed_action(installation)?;
    check_eq!(
        r.engine.action_state().get(&r.handles.family, &r.buyer.0)?,
        Some(&1)
    );
    Ok(())
}

#[test]
fn preview_refuses_changed_rule_and_does_not_claim_a_foreign_catalogue() -> TestResult {
    let mut r = Rig::new()?;
    r.add(false);
    r.command(r.buyer, "coprates_preview wrong_rule");
    r.command(r.buyer, "coprates_preview foreign_catalog");
    let answers = r.previews.lock().map_err(|_| "previews")?;
    check_eq!(answers.len(), 1);
    check!(!answers.first().ok_or("owned refusal")?.passed);
    r.unchanged();
    Ok(())
}

#[test]
fn actual_membership_copied_into_a_foreign_owner_slot_conveys_no_eligibility() -> TestResult {
    struct ForeignState(FactionState);
    impl GameModule for ForeignState {
        fn name(&self) -> &'static str {
            "membership_foreign"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.state(self.0.clone());
            Ok(())
        }
    }
    let mut r = Rig::new()?;
    r.add(false);
    let state = r
        .engine
        .world
        .module_state::<FactionState>()
        .ok_or("state")?
        .clone();
    let mut world = World::new_minimal();
    let buyer = world.create(ObjKind::Player, "CopratesBuyer", GOD, Some(LIMBO))?;
    check_eq!(buyer, r.buyer);
    let engine = boxed_fixture_engine(world, Registry::new(), vec![Box::new(ForeignState(state))])?;
    check_eq!(
        FactionStandingProvider.eval(&engine.world, "member", &json!({"faction":0}), buyer),
        Some(true)
    );
    check!(!policy()?.current(&engine.world, buyer));
    Ok(())
}

#[test]
fn actual_registered_validator_refuses_changed_guard_coordinates() -> TestResult {
    let mut r = Rig::new()?;
    r.add(false);
    for change in 0..4 {
        let mut guard = r
            .handles
            .membership
            .guard(&r.engine.world, &catalog(), r.buyer)
            .ok_or("guard")?;
        let AuthorityGuard::Capability {
            provider,
            actor,
            capability,
            generation,
        } = &mut guard
        else {
            return Err("capability shape".into());
        };
        match change {
            0 => *actor = GOD,
            1 => *generation = 2,
            2 => *capability = CanonicalId::new("foreign_membership")?,
            _ => *provider = OwnerId::new("faction")?,
        }
        let mut builder = r
            .engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        builder.add_definition(
            r.handles.membership.definition().clone(),
            r.engine.registry.definitions().ok_or("definitions")?,
        )?;
        builder.add(guard)?;
        let guards = builder.finish()?;
        check!(
            !r.engine
                .revalidate_guards(&guards)
                .is_ok_and(|result| result.is_current())
        );
    }
    r.unchanged();
    Ok(())
}

#[test]
fn registration_refuses_foreign_owner_and_duplicate_policy() -> TestResult {
    struct Register {
        foreign: bool,
        duplicate: bool,
    }
    impl GameModule for Register {
        fn name(&self) -> &'static str {
            if self.foreign {
                "membership_foreign"
            } else {
                OWNER
            }
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let configured = CopratesMembership::new(FactionId(0), catalog())?;
            configured.register(reg)?;
            if self.duplicate {
                configured.register(reg)?;
            }
            Ok(())
        }
    }
    for (foreign, duplicate) in [(true, false), (false, true)] {
        check!(
            boxed_fixture_engine(
                World::new_minimal(),
                Registry::new(),
                vec![Box::new(Register { foreign, duplicate })]
            )
            .is_err()
        );
    }
    Ok(())
}

#[inline(never)]
fn boxed_fixture_engine(
    world: mush_core::World,
    registry: mush_core::Registry,
    modules: Vec<Box<dyn mush_core::GameModule>>,
) -> Result<Box<mush_core::Engine>, mush_core::ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}
