//! Continues the genuine scoped house filing and its actual measured certificate.
//! Signed Market offers, actual E4 funds and complete owner/core installation;
//! no synthetic grant, certificate, source privilege or server ACK claim.
use super::*;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::{OfferExecutionContext, OfferSubmission};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_market::mind_house::{
    HouseConfig, HousePolicy, HousePurchaseV1, HouseQuoteV1, HouseReceiptV1,
};
use mush_org::{OrgId, OrgScopeId, OrgStore};
use mush_roll::commerce::house_purchase::{
    HouseAllocationV1, HouseProtocol, writer::HousePurchaseWriter,
};
use mush_roll::commerce::{LicenceId, purchase::PurchasePhase};
use std::collections::BTreeSet;

#[path = "house_renewal_integration_tests.rs"]
mod renewal;
type RenewalShared = Arc<Mutex<Option<mush_market::mind_house_renewal::RenewalConfig>>>;

const FIRST: u128 = 19_000_000;
type HouseShared = Arc<Mutex<Option<HouseConfig>>>;
struct Roll {
    renewals: bool,
    shared: Shared,
    house: HouseShared,
    renewal: RenewalShared,
    inner: Option<RollModule>,
}
impl GameModule for Roll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let shared = self.shared.lock().map_err(module_error)?;
        let manifest = shared
            .manifest
            .clone()
            .ok_or_else(|| ModError::msg("manifest"))?;
        let compatibility = shared
            .compatibility
            .clone()
            .ok_or_else(|| ModError::msg("compatibility"))?;
        drop(shared);
        let participants = configuration(manifest.clone(), false)
            .map_err(module_error)?
            .participants;
        let config = HouseConfig::new(
            HousePolicy {
                price: Amount::from_minor(400),
                instrument: InstrumentId::new("cr").ok_or_else(|| ModError::msg("currency"))?,
                registry: GrossRecipientCut {
                    permille: 100,
                    recipient: Some(Recipient {
                        account: WalletId(1),
                        owner: EconomicOwner::System(issuer().map_err(module_error)?),
                    }),
                },
                venue_name: MarketId::new("fixture").ok_or_else(|| ModError::msg("venue"))?,
                venue: GrossRecipientCut {
                    permille: 0,
                    recipient: None,
                },
                transferable: false,
                term_seconds: TERM_SECONDS,
                grace_seconds: GRACE_SECONDS,
                quote_seconds: QUOTE_SECONDS,
                renewal_price: Amount::from_minor(300),
                renewal_term_seconds: TERM_SECONDS - 60,
                renewal_grace_seconds: GRACE_SECONDS - 60,
            },
            manifest.clone(),
            participants.clone(),
            HousePurchaseWriter::new(),
        )?;
        let commerce = CommerceConfig::new(
            CommerceMode::Active,
            InitialFilingTerms::new(TERM_SECONDS, GRACE_SECONDS).map_err(module_error)?,
            manifest,
            compatibility,
        )?
        .with_house_filings()?
        .with_house_mind_purchases(
            config.writer(),
            participants.clone(),
            config.owner_actions(HouseProtocol::HouseV1)?,
        )?;
        let commerce = if self.renewals {
            let r = mush_market::mind_house_renewal::RenewalConfig::new(
                &config,
                mush_roll::commerce::house_renewal::writer::RenewalWriter::new(),
                mush_market::mind_house_renewal::RenewalPolicy {
                    registry: GrossRecipientCut {
                        permille: 100,
                        recipient: Some(Recipient {
                            account: WalletId(1),
                            owner: EconomicOwner::System(issuer().map_err(module_error)?),
                        }),
                    },
                    venue: None,
                    offer_ic_seconds: 600,
                },
            )?
            .with_service()?;
            let next = commerce
                .with_house_mind_renewals(r.writer(), participants, r.actions()?)?
                .with_house_mind_renewal_service(
                    r.service_action()?
                        .ok_or_else(|| ModError::msg("service"))?,
                )?;
            *self.renewal.lock().map_err(module_error)? = Some(r);
            next
        } else {
            commerce
        };
        self.shared.lock().map_err(module_error)?.filing = Some(commerce.initial_filing_action()?);
        *self.house.lock().map_err(module_error)? = Some(config);
        let mut inner =
            Resources::roll_owners(RollModule::with_writers(["tacsim"]).with_commerce(commerce));
        inner.register(reg)?;
        self.inner = Some(inner);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Roll"))?
            .on_world_loaded(ctx, data)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Roll"))?
            .finalize_owner_composition(ctx)
    }
}
struct Economy {
    shared: Shared,
    house: HouseShared,
    renewal: RenewalShared,
    inner: Option<EconomyModule>,
}
impl GameModule for Economy {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let config = self
            .house
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or_else(|| ModError::msg("House"))?;
        let shared = self.shared.lock().map_err(module_error)?;
        let commerce = CommerceConfig::new(
            CommerceMode::Active,
            InitialFilingTerms::new(TERM_SECONDS, GRACE_SECONDS).map_err(module_error)?,
            shared
                .manifest
                .clone()
                .ok_or_else(|| ModError::msg("manifest"))?,
            shared
                .compatibility
                .clone()
                .ok_or_else(|| ModError::msg("compatibility"))?,
        )?
        .with_house_filings()?;
        let mut inner = EconomyModule::new()
            .with_title_issuers(vec![issuer().map_err(module_error)?])
            .with_initial_filing(
                filing_policy().map_err(module_error)?,
                commerce.initial_filing_action()?,
            )
            .with_house_initial_filing(commerce.house_filing_action()?)
            .with_house_mind_purchases(config.purchase_actions(HouseProtocol::HouseV1)?);
        if let Some(r) = self.renewal.lock().map_err(module_error)?.clone() {
            inner = inner
                .with_house_mind_renewals(
                    r.manifest().clone(),
                    r.participants().ordered(),
                    r.actions()?,
                )
                .with_house_mind_renewal_service(
                    r.service_action()?
                        .ok_or_else(|| ModError::msg("service"))?,
                )?;
        }
        inner.register(reg)?;
        self.inner = Some(inner);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Economy"))?
            .on_world_loaded(ctx, data)
    }
}
struct Market {
    house: HouseShared,
    renewal: RenewalShared,
    inner: Option<MarketModule>,
}
impl GameModule for Market {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let config = self
            .house
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or_else(|| ModError::msg("House"))?;
        let mut inner = MarketModule::new()
            .with_venue_gate(Arc::new(Venue))
            .with_house_minds(config);
        if let Some(r) = self.renewal.lock().map_err(module_error)?.clone() {
            inner = inner.with_house_mind_renewals(r);
        }
        inner.register(reg)?;
        reg.command(
            mush_core::CommandSpec {
                name: "@house-purchase-curator-test".into(),
                shape: mush_core::ArgShape::Raw,
                ..mush_core::CommandSpec::default()
            },
            Arc::new(revoke_curator),
        )?;
        self.inner = Some(inner);
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Market"))?
            .on_world_loaded(ctx, data)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner
            .as_mut()
            .ok_or_else(|| ModError::msg("Market"))?
            .finalize_owner_composition(ctx)
    }
}
// The test admin submits a real Org service mutation and persists its audit.
// No purchase provider calls this adapter or receives an authority substitute.
fn revoke_curator(ctx: &mut mush_core::Ctx<'_>) -> Result<(), mush_core::CmdError> {
    use mush_org::{CapabilityId, OrgAction, OrgMutationOp, OrgMutationPlan, ScopeSelector};
    if ctx.actor != GOD {
        return Err(mush_core::CmdError::msg("fixture admin"));
    }
    let scope = OrgScopeId(
        ctx.args
            .raw()
            .parse()
            .map_err(|_| mush_core::CmdError::msg("scope"))?,
    );
    let store = ctx
        .w
        .module_state_mut::<OrgStore>()
        .ok_or_else(|| mush_core::CmdError::msg("Org"))?;
    let org = store
        .state()
        .scope(scope)
        .ok_or_else(|| mush_core::CmdError::msg("scope"))?
        .org;
    let capability = CapabilityId::new("roll.mind.publish")
        .ok_or_else(|| mush_core::CmdError::msg("capability"))?;
    let ops: Vec<_> = store
        .state()
        .grants_in(org)
        .filter(|grant| {
            grant.grantee == GOD
                && grant.capability == capability
                && grant.scope == ScopeSelector::Exact(scope)
        })
        .map(|grant| OrgMutationOp::RevokeGrant { grant: grant.id })
        .collect();
    if ops.is_empty() {
        return Err(mush_core::CmdError::msg("actual curator grant"));
    }
    mush_org::submit(
        ctx,
        &OrgMutationPlan::of(ops),
        OrgAction {
            action: "house-purchase.test".into(),
            org: Some(org),
            detail: json!({"scope":scope,"mode":"revoke"}),
        },
    )
    .map_err(|e| mush_core::CmdError::msg(e.to_string()))
}
fn boot(registry: &Fixture, saved: &Durable) -> TestResult<PlayerWorld> {
    boot_options(registry, saved, false)
}
fn boot_options(registry: &Fixture, saved: &Durable, renewals: bool) -> TestResult<PlayerWorld> {
    let shared: Shared = Arc::new(Mutex::new(Registered::default()));
    let house: HouseShared = Arc::new(Mutex::new(None));
    let renewal: RenewalShared = Arc::new(Mutex::new(None));
    let manifest = registry.definitions.manifest().clone();
    let mut cfg = registry.resources.config();
    cfg.enabled = true;
    cfg.match_.ai_fill = true;
    let real = crate::TacsimRuleset::with_config(cfg).with_house_mind_holdings(manifest.clone());
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_comsys::ComsysModule::new()),
        Box::new(mush_sim::SimulationModule::new().with_accounts(registry.accounts.clone())),
        Box::new(mush_atlas::AtlasModule::new()),
        Box::new(mush_tactics::TacticsModule::with_catalog(
            crate::builtin_definitions()?.units,
        )),
        Box::new(mush_npc::NpcModule::new()),
        Box::new(PlayerTacsim {
            authored_forks: false,
            marked_registry: false,
            real,
            resources: registry.resources.clone(),
            shared: shared.clone(),
        }),
        Box::new(Roll {
            renewals,
            renewal: renewal.clone(),
            shared: shared.clone(),
            house: house.clone(),
            inner: None,
        }),
        Box::new(Economy {
            renewal: renewal.clone(),
            shared: shared.clone(),
            house: house.clone(),
            inner: None,
        }),
        Box::new(Market {
            house,
            renewal,
            inner: None,
        }),
        content()?,
    ];
    let mut engine = Engine::with_modules(saved.world()?, Registry::new(), modules)?;
    engine.set_account_identity(registry.accounts.clone());
    load(&mut engine, &saved.disk)?;
    assert_eq!(
        shared.lock().map_err(module_error)?.manifest,
        Some(manifest)
    );
    let tasks = engine
        .world
        .module_state::<ArenaRegistry>()
        .ok_or("ArenaRegistry")?
        .tasks()
        .clone();
    let egress = tasks.take_egress().ok_or("unique egress")?;
    let mut image: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    mush_testkit::fold_module_ops_into(
        &mut image,
        saved
            .disk
            .iter()
            .map(|((ns, key), value)| ModuleOp {
                ns: ns.as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: key.as_str().into(),
                    value: value.clone(),
                },
            })
            .collect(),
    );
    let factory = shared
        .lock()
        .map_err(module_error)?
        .factory
        .clone()
        .ok_or("Registry factory")?;
    let mut w = PlayerWorld {
        minds_facts: None,
        engine,
        tasks,
        egress,
        image,
        serial: 0,
        factory,
        snapshots: BTreeMap::new(),
    };
    w.commit()?;
    for (actor, conn) in [(BUYER, BUYER_CONN), (GOD, OBSERVER_CONN)] {
        w.engine.world.move_to(actor, ROOM)?;
        w.step(Inbound::ConnUp {
            conn,
            actor,
            caps_structured: false,
            from_linkdead: false,
        })?;
    }
    let _ = w.engine.world.take_dirty();
    Ok(w)
}
fn disk(w: &PlayerWorld) -> Disk {
    w.image
        .iter()
        .flat_map(|(owner, data)| {
            data.kv
                .iter()
                .map(move |(key, value)| ((owner.to_string(), key.to_string()), value.clone()))
        })
        .collect()
}
fn saved(w: &PlayerWorld) -> Durable {
    Durable::capture(&w.engine, &disk(w))
}
fn economy(w: &PlayerWorld) -> TestResult<&EconomyStore> {
    w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or_else(|| "Economy".into())
}
fn wallet(
    w: &PlayerWorld,
    owner: EconomicOwner,
    scope: Option<OrgScopeId>,
) -> TestResult<WalletId> {
    let mut found = economy(w)?.snapshot().accounts().filter(|a| {
        a.owner == owner
            && a.scope == scope
            && a.instrument.as_str() == "cr"
            && a.status == AccountStatus::Active
    });
    let id = found.next().ok_or("actual wallet")?.id;
    if found.next().is_some() {
        return Err("ambiguous wallet".into());
    }
    Ok(id)
}
fn balance(w: &PlayerWorld, id: WalletId) -> TestResult<(Amount, Amount)> {
    let a = economy(w)?.snapshot().account(id).ok_or("account")?;
    Ok((a.posted_balance, a.held_amount))
}
fn command(w: &mut PlayerWorld, line: &str) -> TestResult {
    let e = w.say(GOD, line)?;
    assert!(!e.iter().any(|(_,event)|matches!(event,OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))),"house setup refused");
    Ok(())
}
fn action(w: &PlayerWorld, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "market"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| "house action".into())
}
fn signed(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    parameters: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x65; 32])?,
        [],
    )?);
    let exact = action(w, id)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("signed house offer")?;
    let req = request(n, actor, "market", parameters.clone())?;
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: req.request(),
            principal,
            actor,
            idempotency: req.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
        },
    )?;
    let (actual, invocation, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, exact);
    Ok(w.engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &invocation, token(n))?)
}
fn persist(
    w: &mut PlayerWorld,
    writes: &[ReplayWrite],
    installation: mush_core::composed::ComposedInstallation,
) -> TestResult {
    let operations = writes
        .iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value: value.clone(),
                },
            })),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            })),
            ReplayWrite::LedgerChunkSet { write } => Ok(Some(ModuleOp {
                ns: write.namespace().to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().to_string().into(),
                    value: serde_json::to_value(write.chunk())?,
                },
            })),
            ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => Ok(None),
            ReplayWrite::ObjectDelete { .. } => Err("house purchase deletes no objects".into()),
        })
        .collect::<TestResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    mush_testkit::fold_module_ops_into(&mut w.image, operations);
    w.engine.install_composed_action(installation)?;
    w.commit()
}
fn apply(w: &mut PlayerWorld, prepared: PreparedComposedTxn) -> TestResult<Vec<ReplayWrite>> {
    let (writes, installation) = prepared.into_parts();
    persist(w, &writes, installation)?;
    Ok(writes)
}
fn perform(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    parameters: Value,
) -> TestResult<Vec<ReplayWrite>> {
    let (p, _) = signed(w, id, actor, n, parameters)?;
    apply(w, p)
}
fn allocation(w: &PlayerWorld, n: u128) -> TestResult<HouseAllocationV1> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_house_purchase")
            .into_iter()
            .find(|v| v.get("operation") == Some(&json!(token(n))))
            .ok_or("actual house allocation")?,
    )?)
}
fn term(w: &PlayerWorld, id: LicenceId) -> TestResult<PaidTermRecord> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(id)))
            .ok_or("actual paid term")?,
    )?)
}
fn set_ic(w: &mut PlayerWorld, at: i64) -> TestResult {
    let old = w.engine.world.world_ic_now();
    let utc = w
        .engine
        .world
        .now()
        .checked_add(at.checked_sub(old.at).ok_or("IC delta")?)
        .ok_or("UTC")?;
    w.engine.world.set_now(utc);
    assert_eq!(w.engine.world.world_ic_now().at, at);
    assert_eq!(w.engine.world.world_ic_now().clock, old.clock);
    Ok(())
}

fn direct(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let a = action(w, id)?;
    Ok(w.engine.prepare_registered_composed_authoritative_action(
        &a,
        &request(n, actor, "market", params)?,
        token(n),
    )?)
}
fn stage(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    params: Value,
    p: PreparedComposedTxn,
    o: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(w, id)?;
    let inv = request(n, actor, "market", params)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        o.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(n),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([91; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([92; 32]),
        action,
        provider: OwnerId::new("market")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(n))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = w
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = w.engine.world.take_dirty();
    w.engine.stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}
fn typed_families(writes: &[ReplayWrite]) -> BTreeSet<String> {
    writes
        .iter()
        .filter_map(|w| match w {
            ReplayWrite::ModuleJsonSet { value, .. } => value
                .get("family")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}
fn selection(w: &mut PlayerWorld, licence: LicenceId, allowed: bool) -> TestResult {
    let lobby = match_::centre_object(&w.engine.world, match_::LOBBY_ENTRY).ok_or("lobby")?;
    w.engine.world.move_to(BUYER, lobby)?;
    w.say(BUYER, "SIM TEAM red")?;
    w.say(BUYER, "SIM AI 1")?;
    w.say(BUYER, "SIM UNIT heavy_mk1")?;
    let events = w.say(
        BUYER,
        &format!("SIM MIND filed:{} PROFILE Default", licence.get()),
    )?;
    let text = PlayerWorld::private_text(&events, BUYER);
    assert_eq!(
        text.contains("Your slot will carry"),
        allowed,
        "house selection disposition"
    );
    if allowed {
        let c = match_::centre_of(&w.engine.world).ok_or("centre")?;
        assert_eq!(
            c.slots
                .iter()
                .find(|s| s.pilot == BUYER)
                .and_then(|s| s.mind.as_ref())
                .and_then(|m| m.filed),
            Some(licence)
        );
    }
    Ok(())
}
fn fresh_fences(registry: &Fixture, quoted: &Durable, funded: &Durable, n: u128) -> TestResult {
    for (base, id, params) in [
        (
            quoted,
            "purchase_house_mind",
            json!({"quote":token(FIRST+1)}),
        ),
        (
            funded,
            "settle_house_mind",
            json!({"operation":token(FIRST+2)}),
        ),
    ] {
        for boundary in 0..3 {
            let mut w = boot(registry, base)?;
            let old = balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?;
            if boundary == 0 {
                command(&mut w, "@org/status House=suspended")?;
                assert!(direct(&mut w, id, BUYER, n, params.clone()).is_err());
            } else {
                let (p, o) = direct(&mut w, id, BUYER, n, params.clone())?;
                if boundary == 1 {
                    command(&mut w, "@org/status House=suspended")?;
                    assert!(stage(&mut w, id, BUYER, n, params.clone(), p, o).is_err());
                } else {
                    stage(&mut w, id, BUYER, n, params.clone(), p, o)?;
                    command(&mut w, "@org/status House=suspended")?;
                    let _ = w.engine.world.take_dirty();
                    assert!(matches!(
                        w.engine.take_staged_action(),
                        Err(mush_core::engine::StagedActionAdmissionError::Participant(
                            _
                        ))
                    ));
                }
            }
            assert_eq!(
                balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?,
                old
            );
            assert_eq!(
                rows(&disk(&w), "roll", "roll_mind_house_purchase"),
                rows(&base.disk, "roll", "roll_mind_house_purchase")
            );
            assert!(rows(&disk(&w), "roll", "roll_mind_personal_paid_licence").is_empty());
        }
    }
    Ok(())
}
fn account_fences(registry: &Fixture, quoted: &Durable, funded: &Durable) -> TestResult {
    for (base, id, params) in [
        (
            quoted,
            "purchase_house_mind",
            json!({"quote":token(FIRST+1)}),
        ),
        (
            funded,
            "settle_house_mind",
            json!({"operation":token(FIRST+2)}),
        ),
    ] {
        for after_stage in [false, true] {
            let mut w = boot(registry, base)?;
            let old = balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?;
            let original_disk = disk(&w);
            let (p, o) = direct(&mut w, id, BUYER, FIRST + 25, params.clone())?;
            if after_stage {
                stage(&mut w, id, BUYER, FIRST + 25, params.clone(), p, o)?;
                registry.accounts.epoch.invalidate();
                assert!(w.engine.take_staged_action().is_err());
            } else {
                registry.accounts.epoch.invalidate();
                assert!(stage(&mut w, id, BUYER, FIRST + 25, params.clone(), p, o).is_err());
            }
            assert_eq!(
                balance(&w, wallet(&w, EconomicOwner::Object(BUYER), None)?)?,
                old
            );
            assert!(
                disk(&w) == original_disk,
                "stale account published no private rows"
            );
        }
    }
    Ok(())
}
async fn checkout(registry: &Fixture, snapshot: &Durable, licence: LicenceId) -> TestResult {
    let mut w = boot(registry, snapshot)?;
    let tasks = w.tasks.clone();
    let result: TestResult = async {
        let term = term(&w, licence)?;
        selection(&mut w, licence, true)?;
        for line in [
            format!("@mind/source filed:{}", licence.get()),
            "@mind/source Candidate@1".into(),
        ] {
            let output = w.say(BUYER, &line)?;
            assert!(
                PlayerWorld::private_text(&output, BUYER)
                    .contains("No such owned mind or revision.")
            );
        }
        w.start()?;
        w.board()?;
        assert_eq!(w.state()?, MatchState::Running);
        let id = match_::centre_of(&w.engine.world)
            .and_then(|c| c.match_id)
            .ok_or("running match")?;
        let record = w.record(id)?;
        let unit = record
            .teams
            .iter()
            .flat_map(|t| t.pilots.iter())
            .find_map(|(pilot, unit)| (*pilot == BUYER).then_some(*unit))
            .ok_or("customer hull")?;
        let seal = record
            .minds
            .get(&unit)
            .ok_or("actual house seal")?
            .seal
            .clone();
        assert_eq!(seal.origin.licence_label(), licence.get().to_string());
        assert_eq!(seal.engine, crate::arena::SealEngine::Scripted);
        assert!(seal.requested.origin.certificate().is_some());
        w.read_sealed_mind(&seal.mind).await?;
        let deadline = Instant::now() + WAIT;
        while !w.snapshots.contains_key(&id) {
            w.pump()?;
            if Instant::now() >= deadline {
                return Err("house opening snapshot".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let opening = w.snapshots.get(&id).ok_or("opening")?;
        let ground = crate::arena_bridge::ground_of(&w.engine.world, id).ok_or("ground")?;
        let arena = Snapshot::decode(&opening.document, (*ground).clone())?;
        assert_eq!(
            arena
                .units
                .iter()
                .find(|u| u.id == unit)
                .ok_or("opening unit")?
                .controller,
            Controller::Pilot
        );
        set_ic(&mut w, term.window().grace_end().get())?;
        let before = w.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        w.say(BUYER, "EXIT")?;
        let first = w.progressed_snapshot(id, unit, before, 0, -1).await?;
        let (thinks, landed) = assert_captured(&w, id, unit, &seal, &first)?;
        let later = w
            .progressed_snapshot(id, unit, first.at_ms, thinks, landed)
            .await?;
        let (next_thinks, next_landed) = assert_captured(&w, id, unit, &seal, &later)?;
        assert!(
            next_thinks > thinks && next_landed > landed,
            "captured house mind runs after own expiry"
        );
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
fn flow(
    registry: &Fixture,
    snapshot: &Durable,
    organization: OrgId,
    scope: OrgScopeId,
) -> TestResult<(Durable, LicenceId, Durable)> {
    let birth = one(rows(&snapshot.disk, "roll", "roll_mind_initial_filing"))?;
    let owner: LicenceId =
        serde_json::from_value(birth.get("licence").cloned().ok_or("owner licence")?)?;
    let owner_term: mush_roll::commerce::records::OwnerTerm =
        serde_json::from_value(one(rows(&snapshot.disk, "roll", "roll_mind_owner_term"))?)?;
    let mut w = boot(registry, snapshot)?;
    for actor in [GOD, BUYER] {
        command(&mut w, &format!("@account/open #{}=cr", actor.0))?;
    }
    let buyer = wallet(&w, EconomicOwner::Object(BUYER), None)?;
    let curator = wallet(&w, EconomicOwner::Object(GOD), None)?;
    let treasury = wallet(&w, EconomicOwner::Organization(organization), Some(scope))?;
    let faucet = wallet(
        &w,
        EconomicOwner::System(SystemPrincipalId::new("faucet").ok_or("faucet")?),
        None,
    )?;
    for account in [buyer, curator] {
        command(&mut w, &format!("@econ/mint {faucet}={account} 2000"))?;
    }
    // The nonmember cannot publish through staff or purchaser rights.
    assert!(
        signed(
            &mut w,
            "list_house_mind",
            BUYER,
            FIRST,
            json!({"owner_licence":owner.get().to_string()})
        )
        .is_err()
    );
    perform(
        &mut w,
        "list_house_mind",
        GOD,
        FIRST,
        json!({"owner_licence":owner.get().to_string()}),
    )?;
    perform(
        &mut w,
        "quote_house_mind",
        BUYER,
        FIRST + 1,
        json!({"listing":token(FIRST)}),
    )?;
    let quote: HouseQuoteV1 =
        serde_json::from_value(one(rows(&disk(&w), "market", "market_mind_house_quote"))?)?;
    command(&mut w, &format!("@house-purchase-curator-test {}", scope.0))?;
    assert!(
        signed(
            &mut w,
            "list_house_mind",
            GOD,
            FIRST + 30,
            json!({"owner_licence":owner.get().to_string()})
        )
        .is_err(),
        "a departed curator cannot publish another listing"
    );
    // The acknowledged quote still belongs to the actual house, independently
    // of the now-departed publisher. Both Fund and Grant below use that quote.
    let quoted = saved(&w);
    let q = serde_json::to_value(&quote)?;
    assert_eq!(
        q.pointer("/subject/issuer"),
        Some(&json!({"format":1,"organization":organization,"scope":scope}))
    );
    assert!(q.get("seller").is_none());
    assert_eq!(q.get("price"), Some(&json!(400)));
    assert_eq!(q.pointer("/product/grant/renewal/price"), Some(&json!(300)));
    let money = balance(&w, buyer)?;
    let house_money = balance(&w, treasury)?;
    let registry_money = balance(&w, WalletId(1))?;
    let funded_writes = perform(
        &mut w,
        "purchase_house_mind",
        BUYER,
        FIRST + 2,
        json!({"quote":token(FIRST+1)}),
    )?;
    assert!(typed_families(&funded_writes).contains("roll_mind_house_purchase"));
    assert_eq!(balance(&w, buyer)?, (money.0, Amount::from_minor(400)));
    assert!(rows(&disk(&w), "roll", "roll_mind_personal_paid_licence").is_empty());
    let funded = saved(&w);
    fresh_fences(registry, &quoted, &funded, FIRST + 20)?;
    account_fences(registry, &quoted, &funded)?;
    // Real issuer loss blocks Grant, but the funded buyer can still drain.
    let mut refund = boot(registry, &funded)?;
    command(&mut refund, "@org/status House=suspended")?;
    perform(
        &mut refund,
        "refund_house_mind",
        BUYER,
        FIRST + 3,
        json!({"operation":token(FIRST+2)}),
    )?;
    assert_eq!(balance(&refund, buyer)?, money);
    assert_eq!(balance(&refund, treasury)?, house_money);
    assert!(matches!(
        allocation(&refund, FIRST + 2)?.phase(),
        PurchasePhase::Refunded { .. }
    ));
    assert!(rows(&disk(&refund), "roll", "roll_mind_house_term_economics").is_empty());
    let _cold_refund = boot(registry, &saved(&refund))?;
    let next = w
        .engine
        .world
        .world_ic_now()
        .at
        .checked_add(17)
        .ok_or("time")?;
    set_ic(&mut w, next)?;
    let writes = perform(
        &mut w,
        "settle_house_mind",
        BUYER,
        FIRST + 4,
        json!({"operation":token(FIRST+2)}),
    )?;
    let required = BTreeSet::from(
        [
            "market_mind_house_purchase",
            "market_mind_house_receipt",
            "roll_mind_house_purchase",
            "roll_mind_personal_paid_licence",
            "roll_mind_paid_term",
            "roll_mind_house_term_economics",
        ]
        .map(str::to_owned),
    );
    assert!(
        required.is_subset(&typed_families(&writes)),
        "actual six terminal rows"
    );
    let allocation = allocation(&w, FIRST + 2)?;
    assert_eq!(allocation.holder(), BUYER);
    assert_eq!(allocation.issuer().organization(), organization);
    assert_eq!(allocation.issuer().scope(), scope);
    assert!(matches!(allocation.phase(), PurchasePhase::Granted { .. }));
    let paid = term(&w, allocation.licence())?;
    assert_eq!(
        paid.terms().form(),
        mush_roll::commerce::paid::PaidForm::Sealed {}
    );
    assert_eq!(paid.terms().seats().get(), 1);
    assert_eq!(
        balance(&w, buyer)?,
        (Amount::from_minor(money.0.minor() - 400), Amount::ZERO)
    );
    assert_eq!(
        balance(&w, treasury)?,
        (
            Amount::from_minor(house_money.0.minor() + 360),
            house_money.1
        )
    );
    assert_eq!(
        balance(&w, WalletId(1))?,
        (
            Amount::from_minor(registry_money.0.minor() + 40),
            registry_money.1
        )
    );
    let _: HousePurchaseV1 = serde_json::from_value(one(rows(
        &disk(&w),
        "market",
        "market_mind_house_purchase",
    ))?)?;
    let _: HouseReceiptV1 =
        serde_json::from_value(one(rows(&disk(&w), "market", "market_mind_house_receipt"))?)?;
    let granted = saved(&w);
    let renewed = renewal::exercise(
        registry,
        &granted,
        allocation.licence(),
        owner_term.window().grace_end().get(),
    )?;
    let _cold = boot(registry, &granted)?;
    for (owner, family) in [
        ("market", "market_mind_house_purchase"),
        ("market", "market_mind_house_receipt"),
        ("roll", "roll_mind_house_purchase"),
        ("roll", "roll_mind_house_term_economics"),
        ("roll", "roll_mind_paid_term"),
    ] {
        let mut broken = granted.clone();
        assert!(remove_family(&mut broken, owner, family) > 0);
        assert!(
            boot(registry, &broken).is_err(),
            "missing paired history accepted"
        );
    }
    let mut all_missing = granted.clone();
    for (owner, family) in [
        ("market", "market_mind_house_purchase"),
        ("market", "market_mind_house_receipt"),
        ("roll", "roll_mind_house_purchase"),
        ("roll", "roll_mind_house_term_economics"),
        ("roll", "roll_mind_paid_term"),
        ("roll", "roll_mind_personal_paid_licence"),
    ] {
        remove_family(&mut all_missing, owner, family);
    }
    assert!(
        boot(registry, &all_missing).is_err(),
        "E4 inverse cannot lose all six rows"
    );
    let mut crossed = granted.clone();
    change_one_family(&mut crossed, "roll", "roll_mind_house_purchase", |row| {
        *row.get_mut("issuer")
            .and_then(|v| v.get_mut("scope"))
            .ok_or("scope")? = json!(scope.0.checked_add(1).ok_or("scope overflow")?);
        Ok(())
    })?;
    assert!(boot(registry, &crossed).is_err());
    // Members and even the publisher pay the identical gross public price.
    perform(
        &mut w,
        "quote_house_mind",
        GOD,
        FIRST + 5,
        json!({"listing":token(FIRST)}),
    )?;
    let curator_before = balance(&w, curator)?;
    perform(
        &mut w,
        "purchase_house_mind",
        GOD,
        FIRST + 6,
        json!({"quote":token(FIRST+5)}),
    )?;
    assert_eq!(
        balance(&w, curator)?,
        (curator_before.0, Amount::from_minor(400))
    );
    perform(
        &mut w,
        "refund_house_mind",
        GOD,
        FIRST + 7,
        json!({"operation":token(FIRST+6)}),
    )?;
    assert_eq!(balance(&w, curator)?, curator_before);
    // Issuer term lapse is not inherited by the already-granted customer's Pin.
    set_ic(&mut w, owner_term.window().grace_end().get())?;
    assert!(w.engine.world.world_ic_now().at < paid.window().grace_end().get());
    assert!(
        direct(
            &mut w,
            "quote_house_mind",
            BUYER,
            FIRST + 8,
            json!({"listing":token(FIRST)})
        )
        .is_err()
    );
    command(&mut w, "@org/status House=suspended")?;
    let independent = saved(&w);
    let mut cold = boot(registry, &independent)?;
    selection(&mut cold, allocation.licence(), true)?;
    set_ic(&mut cold, paid.window().grace_end().get())?;
    selection(&mut cold, allocation.licence(), false)?;
    Ok((independent, allocation.licence(), renewed))
}
pub(super) fn exercise(
    registry: &Fixture,
    saved: &Durable,
    organization: OrgId,
    scope: OrgScopeId,
) -> TestResult {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (granted, licence, renewed) = flow(registry, saved, organization, scope)?;
        renewal::checkout(registry, &renewed, licence).await?;
        checkout(registry, &granted, licence).await
    })
}
