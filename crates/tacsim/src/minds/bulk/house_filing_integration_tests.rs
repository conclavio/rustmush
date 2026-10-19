//! Child of the genuine full Registry gate. Nothing below constructs a rating
//! or grants a house title except the actual registered scoped filing action.
use super::*;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_org::{OrgModule, OrgStore};

type HouseAction = Arc<Mutex<Option<DefinitionRef>>>;
struct HouseRoll {
    shared: Shared,
    house: HouseAction,
    inner: Option<RollModule>,
}
impl GameModule for HouseRoll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let shared = self.shared.lock().map_err(module_error)?;
        let config = CommerceConfig::new(
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
        drop(shared);
        self.shared.lock().map_err(module_error)?.filing = Some(config.initial_filing_action()?);
        *self.house.lock().map_err(module_error)? = Some(config.house_filing_action()?);
        let mut inner =
            Resources::roll_owners(RollModule::with_writers(["tacsim"]).with_commerce(config));
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
struct HouseEconomy {
    shared: Shared,
    house: HouseAction,
    inner: Option<EconomyModule>,
}
impl GameModule for HouseEconomy {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let personal = self
            .shared
            .lock()
            .map_err(module_error)?
            .filing
            .clone()
            .ok_or_else(|| ModError::msg("personal action"))?;
        let house = self
            .house
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or_else(|| ModError::msg("house action"))?;
        let mut inner = EconomyModule::new()
            .with_title_issuers(vec![issuer().map_err(module_error)?])
            .with_initial_filing(filing_policy().map_err(module_error)?, personal)
            .with_house_initial_filing(house);
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
struct HouseFixture {
    engine: Engine,
    disk: Disk,
    action: DefinitionRef,
}
impl HouseFixture {
    fn boot(registry: &Fixture, saved: &Durable) -> TestResult<Self> {
        let shared: Shared = Arc::new(Mutex::new(Registered::default()));
        let house: HouseAction = Arc::new(Mutex::new(None));
        let mut engine = Engine::with_modules(
            saved.world()?,
            Registry::new(),
            vec![
                Box::new(OrgModule::new()),
                Box::new(Tacsim {
                    real: None,
                    resources: registry.resources.clone(),
                    active: true,
                    shared: shared.clone(),
                    accounts: registry.accounts.clone(),
                }),
                Box::new(HouseRoll {
                    shared: shared.clone(),
                    house: house.clone(),
                    inner: None,
                }),
                Box::new(HouseEconomy {
                    shared,
                    house: house.clone(),
                    inner: None,
                }),
            ],
        )?;
        engine.set_account_identity(registry.accounts.clone());
        let mut disk = saved.disk.clone();
        load(&mut engine, &disk)?;
        module_ops(&mut disk, &engine.take_module_ops())?;
        let action = house
            .lock()
            .map_err(module_error)?
            .clone()
            .ok_or("house registration")?;
        engine.step(
            mush_core::Inbound::ConnUp {
                conn: 91,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        Ok(Self {
            engine,
            disk,
            action,
        })
    }
    fn command(&mut self, line: &str) -> TestResult {
        let mut out = Outbox::new();
        self.engine.step(
            mush_core::Inbound::Cmd {
                conn: 91,
                actor: GOD,
                line: line.into(),
            },
            &mut out,
            &mut mush_core::Effects::new(),
        );
        if out.events().iter().any(|(_,event)|matches!(event,mush_proto::OutputEvent::Sys(text) if text.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) { return Err(format!("house setup refused: {line}").into()); }
        module_ops(&mut self.disk, &self.engine.take_module_ops())?;
        Ok(())
    }
    fn snapshot(&self) -> Durable {
        Durable::capture(&self.engine, &self.disk)
    }
    fn economy(&self) -> TestResult<&EconomyStore> {
        self.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or_else(|| "Economy".into())
    }
    fn prepare(
        &mut self,
        number: u128,
        parameters: Value,
    ) -> TestResult<(
        PreparedComposedTxn,
        mush_core::action::PreparedAuthoritativeAction,
    )> {
        Ok(self
            .engine
            .prepare_registered_composed_authoritative_action(
                &self.action,
                &request(number, GOD, "roll", parameters)?,
                token(number),
            )?)
    }
    fn stage(
        &mut self,
        number: u128,
        parameters: Value,
        prepared: PreparedComposedTxn,
        output: mush_core::action::PreparedAuthoritativeAction,
    ) -> TestResult {
        let inv = request(number, GOD, "roll", parameters)?;
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = output.into_parts();
        definitions.push(self.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(number),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([91; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([92; 32]),
            action: self.action.clone(),
            provider: OwnerId::new("roll")?,
            cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: prepared.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(number))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })?;
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())?
            .finish()?;
        let _ = self.engine.world.take_dirty();
        self.engine
            .stage_authoritative_action(prepared, guards, bundle)?;
        Ok(())
    }
    fn acknowledge(&mut self) -> TestResult<Vec<ReplayWrite>> {
        let (bundle, installation) = self
            .engine
            .take_staged_action()?
            .ok_or("staged house filing")?
            .into_parts();
        let writes = bundle.writes().to_vec();
        retain_writes(&mut self.disk, &writes)?;
        self.engine.install_composed_action(installation)?;
        Ok(writes)
    }
}

pub(super) fn exercise(registry: &Fixture, rating: &RatingRef) -> TestResult {
    let saved = Durable::capture(&registry.engine, &registry.disk);
    let mut f = HouseFixture::boot(registry, &saved)?;
    // Real operator setup: no hand-written organization, account balance or
    // grant rows. The real bounded faucet transaction funds this exact scope.
    for line in [
        "@org/create House",
        "@org/scope create House=publication",
        "@org/scope create House=other",
        "@instrument/register cr=0",
        "@account/system faucet=cr",
        "@account/system roll.registry=cr",
    ] {
        f.command(line)?;
    }
    let org = f
        .engine
        .world
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state();
    let organization = org.orgs().next().ok_or("house")?.id;
    let scope = org
        .scopes_in(organization)
        .find(|s| s.name.as_str() == "publication")
        .ok_or("publication scope")?
        .id;
    let other = org
        .scopes_in(organization)
        .find(|s| s.name.as_str() == "other")
        .ok_or("other scope")?
        .id;
    f.command(&format!("@account/open org:House/{}=cr", scope.0))?;
    let payer = f
        .economy()?
        .snapshot()
        .accounts()
        .find(|a| a.owner == EconomicOwner::Organization(organization) && a.scope == Some(scope))
        .ok_or("actual payer")?
        .id;
    let faucet_owner = EconomicOwner::System(SystemPrincipalId::new("faucet").ok_or("faucet")?);
    let faucet = f
        .economy()?
        .snapshot()
        .accounts()
        .find(|a| a.owner == faucet_owner)
        .ok_or("actual faucet")?
        .id;
    assert_eq!(
        f.economy()?
            .snapshot()
            .account(WalletId(1))
            .ok_or("configured sink")?
            .owner,
        EconomicOwner::System(issuer()?)
    );
    f.command(&format!("@econ/mint {faucet}={payer} 1000"))?;
    let reserve = reference(
        "roll",
        "action",
        mush_core::reserved_shell::RESERVE_ACTION,
        json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
    )?;
    let shell = ObjId(f.engine.world.next_id());
    let (reserved, _) = f.engine.prepare_registered_composed_authoritative_action(
        &reserve,
        &request(6_000_000, GOD, "roll", json!({}))?,
        token(6_000_000),
    )?;
    let (writes, installation) = reserved.into_parts();
    retain_writes(&mut f.disk, &writes)?;
    f.engine.install_composed_action(installation)?;
    let revision = registry.revision.reference();
    let parameters = json!({"shell_request":RequestUid::from_bytes(6_000_000_u128.to_be_bytes()),
        "mind":revision.key.mind,"revision":revision.key.number,"revision_digest":revision.digest,
        "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
        "payer":payer,"charter_name":"Measured house original","organization":organization,"scope":scope});
    let before = f.snapshot();
    let before_revision = f.engine.action_state().revision();
    assert!(
        f.prepare(6_000_001, parameters.clone()).is_err(),
        "staff alone supplies none of the scoped rights"
    );
    assert_eq!(f.disk, before.disk);
    assert_eq!(f.engine.action_state().revision(), before_revision);
    let caps = [
        mush_economy::registry_filing::HOUSE_PUBLICATION_CAPABILITY,
        "asset.title.transfer",
        "economy.account.post",
    ];
    for capability in caps {
        f.command(&format!(
            "@org/grant #{}=House {capability} scope=#{}",
            GOD.0, other.0
        ))?;
    }
    assert!(
        f.prepare(6_000_001, parameters.clone()).is_err(),
        "another scope cannot spend or publish here"
    );
    for capability in caps {
        f.command(&format!(
            "@org/grant #{}=House {capability} scope=#{}",
            GOD.0, scope.0
        ))?;
    }
    let eligible = f.snapshot();
    // The exact same registered provider succeeds before the authority changes.
    let (prepared, output) = f.prepare(6_000_001, parameters.clone())?;
    f.stage(6_000_001, parameters.clone(), prepared, output)?;
    let writes = f.acknowledge()?;
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    assert_eq!(
        f.economy()?
            .snapshot()
            .account(payer)
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(900)
    );
    let title = f.economy()?.snapshot().asset(shell).ok_or("title")?;
    assert_eq!(title.title_owner, EconomicOwner::Organization(organization));
    assert_eq!(title.scope, Some(scope));
    let operation = one(rows(&f.disk, "roll", "roll_mind_initial_filing"))?;
    assert_eq!(operation.get("format"), Some(&json!(2)));
    assert_eq!(
        operation.pointer("/economic/house"),
        Some(&json!({"format":1,"curator":GOD,"organization":organization,"scope":scope}))
    );
    assert_eq!(operation.pointer("/rating/job"), Some(&json!(rating.job())));
    let committed = f.snapshot();
    let restarted = HouseFixture::boot(registry, &committed)?;
    assert_eq!(restarted.economy()?.snapshot().asset(shell), Some(title));
    assert_eq!(
        one(rows(&restarted.disk, "roll", "roll_mind_initial_filing"))?,
        operation
    );
    let mut missing = committed.clone();
    remove_family(&mut missing, "roll", "roll_mind_initial_filing");
    assert!(
        HouseFixture::boot(registry, &missing).is_err(),
        "history reverse join cannot lose the house operation"
    );
    let mut no_house = committed.clone();
    change_one_family(&mut no_house, "roll", "roll_mind_initial_filing", |row| {
        row.get_mut("economic")
            .and_then(Value::as_object_mut)
            .ok_or("economic")?
            .remove("house");
        Ok(())
    })?;
    assert!(HouseFixture::boot(registry, &no_house).is_err());
    let mut stale = HouseFixture::boot(registry, &eligible)?;
    let (prepared, output) = stale.prepare(6_000_002, parameters.clone())?;
    stale.command("@org/status House=suspended")?;
    let error = stale
        .stage(6_000_002, parameters.clone(), prepared, output)
        .err()
        .ok_or("stale authority accepted")?;
    assert_eq!(
        error.downcast_ref::<mush_core::engine::StagePreparedActionError>(),
        Some(&mush_core::engine::StagePreparedActionError::ParticipantInputsChanged),
        "scoped right is re-resolved before custody"
    );
    assert!(stale.economy()?.snapshot().asset(shell).is_none());
    assert_eq!(
        stale
            .economy()?
            .snapshot()
            .account(payer)
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(1000)
    );
    assert!(rows(&stale.disk, "roll", "roll_mind_initial_filing").is_empty());
    // A successful stage still cannot cross the final custody boundary after
    // the genuine organization service has made the compartment inert.
    let mut stale_take = HouseFixture::boot(registry, &eligible)?;
    let (prepared, output) = stale_take.prepare(6_000_003, parameters.clone())?;
    stale_take.stage(6_000_003, parameters, prepared, output)?;
    stale_take.command("@org/status House=suspended")?;
    assert!(matches!(
        stale_take.engine.take_staged_action(),
        Err(mush_core::engine::StagedActionAdmissionError::Participant(
            _
        ))
    ));
    assert!(!stale_take.engine.has_staged_action());
    assert!(stale_take.economy()?.snapshot().asset(shell).is_none());
    assert_eq!(
        stale_take
            .economy()?
            .snapshot()
            .account(payer)
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(1000)
    );
    assert!(rows(&stale_take.disk, "roll", "roll_mind_initial_filing").is_empty());
    Ok(())
}
