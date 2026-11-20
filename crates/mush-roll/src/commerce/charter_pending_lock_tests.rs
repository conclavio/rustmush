#![cfg(test)]

//! Tier 2A regression tests proving that an active funded `charter_pending`
//! escrow lock blocks all four Roll commerce mutation/sale paths:
//! - `OriginalSaleReader` (`sale_reader.rs`)
//! - `EntitlementSaleReader` (`entitlement/reader.rs`)
//! - Author withdrawal (`withdrawal/provider.rs`)
//! - Compatible line update (`line_update/provider.rs`)
//!
//! While allowing the exact same valid requests when no funded escrow is active.

use std::sync::{Arc, Mutex};

use mush_core::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey,
    IdempotencyScope, PerceivedRef, PrincipalId as Actor, RequestUid, ScopedIdempotencyKey,
};
use mush_core::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{
    Engine, GOD, GameModule, ModError, ModuleBootData, ModuleCtx, ObjId, ObjKind, Outbox,
    Registrar, Registry, World,
};
use mush_economy::{
    Account, AccountId, AccountStatus, Amount, CatalogRef, CreditPolicy, EconomyModule,
    InstrumentDef, InstrumentId, InstrumentKind, InstrumentStatus, NegativePolicy,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use serde_json::{Value, json};

use super::*;
use crate::commerce::charter_pending::{PendingCharterTransferConfig, protocol as pending_proto};
use crate::commerce::charter_transfer::CharterTransferConfig;
use crate::commerce::entitlement::reader::{
    EntitlementSaleFacts, EntitlementSaleReader, EntitlementSaleRequest,
};
use crate::commerce::purchase::PurchaseParticipants;
use crate::commerce::sale_reader::{OriginalSaleFacts, OriginalSaleReader, OriginalSaleRequest};
use crate::commerce::{LicenceId, withdrawal};

type OrigAnswers = Arc<Mutex<Option<Result<OriginalSaleFacts, OwnerReadError>>>>;
type EntAnswers = Arc<Mutex<Option<Result<EntitlementSaleFacts, OwnerReadError>>>>;

struct OrigMarket;
struct OrigQuery {
    orig: OwnerReadHandle<OriginalSaleReader>,
    ans: OrigAnswers,
}
impl mush_core::module::QueryHandler for OrigQuery {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("live context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::module::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let licence = LicenceId::new(1).unwrap();
        let req = match args.as_str().unwrap_or("") {
            "orig_own" => OriginalSaleRequest::own(licence),
            "orig_listed" => OriginalSaleRequest::listed(GOD, licence),
            other => {
                return Err(mush_proto::RichText::plain(format!(
                    "unexpected query mode: {other}"
                )));
            }
        };
        *self.ans.lock().unwrap() = Some(ctx.read_owner(&self.orig, req));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
impl GameModule for OrigMarket {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let manifest = reference("tacsim", manifest_descriptor());
        let orig = OriginalSaleReader::request(reg, &manifest)?;
        let ans: OrigAnswers = Arc::new(Mutex::new(None));
        reg.state(ans.clone());
        reg.query_handler("pending.orig.test", Arc::new(OrigQuery { orig, ans }))
    }
}

struct EntMarket;
struct EntQuery {
    ent: OwnerReadHandle<EntitlementSaleReader>,
    ans: EntAnswers,
}
impl mush_core::module::QueryHandler for EntQuery {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("live context required"))
    }
    fn query_with_context(
        &self,
        ctx: &mush_core::module::HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let licence = LicenceId::new(1).unwrap();
        let req = match args.as_str().unwrap_or("") {
            "ent_own" => EntitlementSaleRequest::own(licence, token(90)),
            "ent_listed" => EntitlementSaleRequest::listed(GOD, licence),
            other => {
                return Err(mush_proto::RichText::plain(format!(
                    "unexpected query mode: {other}"
                )));
            }
        };
        *self.ans.lock().unwrap() = Some(ctx.read_owner(&self.ent, req));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
impl GameModule for EntMarket {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let manifest = reference("tacsim", manifest_descriptor());
        let ent = EntitlementSaleReader::request(reg, &manifest)?;
        let ans: EntAnswers = Arc::new(Mutex::new(None));
        reg.state(ans.clone());
        reg.query_handler("pending.ent.test", Arc::new(EntQuery { ent, ans }))
    }
}

struct TestIdentity(mush_core::account::AccountIdentityEpoch);
impl mush_core::AccountIdentity for TestIdentity {
    fn account_of(&self, character: ObjId) -> Option<mush_core::account::AccountId> {
        mush_core::account::AccountId::mint(&format!("acct-{}", character.0), 1).ok()
    }
    fn capture_personal_account(
        &self,
        character: ObjId,
    ) -> Option<mush_core::account::PersonalAccountSnapshot> {
        self.0
            .personal_account(character, self.account_of(character)?)
    }
    fn observe_distinct_accounts(
        &self,
        requester: ObjId,
        other: ObjId,
    ) -> Option<mush_core::account::AccountPairSnapshot> {
        self.0.distinct_pair(
            requester,
            self.account_of(requester)?,
            other,
            self.account_of(other)?,
        )
    }
}

struct NoopProvider;
impl mush_core::action::ActionProvider for NoopProvider {
    fn validate_invocation(
        &self,
        _invocation: &ActionInvocation,
    ) -> Result<(), mush_core::action::ActionError> {
        Ok(())
    }
}

/// Minimal Roll module that registers `EntitlementSaleReader` directly over the
/// real `CommerceFamilies` + `RatingFamilies` so we can exercise
/// `EntitlementSaleReader::read` against the exact same hydrated disk snapshots
/// before and after `charter_pending` funding.
struct EntitlementCheckRoll {
    module: RollModule,
    sources: Sources,
    cfg: CommerceConfig,
    pending_cfg: PendingCharterTransferConfig,
    participants: PurchaseParticipants,
    handles: Option<(CommerceFamilies, RatingFamilies)>,
}
impl GameModule for EntitlementCheckRoll {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, r: &mut Registrar<'_>) -> Result<(), ModError> {
        self.module.register(r)?;
        for kind in ["action", "owner_read", "owner_participant", "outcome"] {
            r.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                |_: &mush_core::mechanics::DefinitionValidationContext<'_>| Ok(()),
            )?;
        }
        let mut root_def = None;
        for desc in self.cfg.definitions()? {
            let is_action = desc.kind().as_str() == "action";
            let is_root = desc.kind().as_str() == "owner_participant"
                && desc.id().as_str() == "commercial_roots";
            let def_ref = r.definition(desc)?;
            if is_action {
                r.action_provider(def_ref.clone(), NoopProvider)?;
            }
            if is_root {
                root_def = Some(def_ref);
            }
        }
        let pending_actions = self.pending_cfg.actions()?;
        let mut root_actions =
            std::collections::BTreeSet::from([self.cfg.initial_filing_action()?]);
        root_actions.extend(pending_actions.iter().cloned());
        r.owner_participant(
            root_def.expect("commercial_roots participant"),
            root_actions,
            crate::commerce::participant::RootParticipant,
        )?;
        for (i, action) in pending_actions.iter().enumerate() {
            r.request_owner_participant::<crate::commerce::participant::RootParticipant>(
                action.clone(),
            )?;
            if i > 0 {
                r.request_owner_participant::<mush_economy::registry_filing::InitialFilingParticipant>(
                    action.clone(),
                )?;
            }
        }
        r.charter_pending_capacity(self.participants.ordered(), pending_actions, None)?;
        let families = CommerceFamilies::register(r)?;
        let ratings = RatingFamilies::register(r)?;
        self.handles = Some((families.clone(), ratings.clone()));
        let manifest = reference("tacsim", manifest_descriptor());
        crate::commerce::entitlement::reader::register(
            r,
            families,
            ratings,
            &manifest,
            RatingCompatibility::combat_v1(manifest.digest()),
        )
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let loaded = RollStore::load_commerce(&data).map_err(|e| ModError::msg(e.to_string()))?;
        *ctx.state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("missing Roll"))? = loaded;
        Ok(())
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let roll = ctx
            .state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("missing Roll"))?;
        roll.audit_sources(&self.sources, &self.sources)
            .map_err(|e| ModError::msg(e.to_string()))
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        let (families, ratings) = self.handles.as_ref().unwrap();
        let follow = crate::commerce::follow::rebuild(ctx, families, ratings)
            .map_err(|e| ModError::msg(e.to_string()))?;
        let roll = ctx
            .state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("missing Roll"))?;
        let (generation, epoch) = roll
            .commerce_audit_base()
            .map_err(|e| ModError::msg(e.to_string()))?;
        roll.install_follow_audit((generation, epoch), follow)
            .map_err(|e| ModError::msg(e.to_string()))?;
        let (generation, epoch) = roll
            .commerce_audit_base()
            .map_err(|e| ModError::msg(e.to_string()))?;
        roll.finish_commerce_audit(generation, epoch)
            .map_err(|e| ModError::msg(e.to_string()))
    }
}

fn actor_invocation(actor: ObjId, n: u8, parameters: Value) -> ActionInvocation {
    let request = RequestUid::from_bytes([n; 16]);
    let principal = Actor::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([n; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll").unwrap(),
                namespace: CanonicalId::new("pending_lock_fixture").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}

struct LockRig {
    engine: Box<Engine>,
    cfg: CommerceConfig,
    pending_cfg: PendingCharterTransferConfig,
    participants: PurchaseParticipants,
    issuer: SystemPrincipalId,
    catalog: CatalogRef,
    sources: Sources,
    disk: Disk,
    buyer: ObjId,
    service_actor: ObjId,
    shell: ObjId,
    rev2: Revision,
    rating2: crate::commerce::rating::RatingRef,
}

fn build_rig() -> LockRig {
    let mut disk = Disk::new();
    let mut initialize = setup::engine(CommerceMode::Initialize);
    setup::boot(&mut initialize).unwrap();
    let (prepared, _) = initialize
        .prepare_registered_composed_authoritative_action(
            &setup::setup_action(),
            &setup::invocation(),
            token(1),
        )
        .unwrap();
    let (writes, install) = prepared.into_parts();
    retain(&mut disk, &writes);
    initialize.install_composed_action(install).unwrap();

    // Save revision 1 and compatible revision 2 of GOD's mind.
    let mut sources = Sources::default();
    let mut roll = RollStore::default();
    let draft1 = WorkingCopy {
        source: "fn think(view, card, memory) { memory }".into(),
        envelope: Envelope {
            schema: json!({"sections":[]}),
            presets: json!({}),
            profiles: json!({"Default":{}}),
            rules: json!({}),
            declared_class: "I".into(),
            mind_abi: "structural=1".into(),
        },
    };
    let create = roll.prepare_create(GOD, "Original", draft1).unwrap();
    roll.publish_with(create, |ops| retain_ops(&mut disk, ops))
        .unwrap();
    let base1 = roll.mind(GOD, "Original").unwrap().base();
    let p1 = roll
        .prepare_save(GOD, "Original", base1, "", None, &sources, &sources)
        .unwrap();
    let (c1, s1, rev1_ref) = p1.into_parts();
    sources.0.insert(s1.token.0, s1.token.1);
    roll.publish_with(c1, |ops| retain_ops(&mut disk, ops))
        .unwrap();
    let rev1 = roll.revision(GOD, rev1_ref.key).unwrap().clone();

    let draft2 = WorkingCopy {
        source: "fn think(view, card, memory) { let _x = 1; memory }".into(),
        envelope: Envelope {
            schema: json!({"sections":[]}),
            presets: json!({}),
            profiles: json!({"Default":{}}),
            rules: json!({}),
            declared_class: "I".into(),
            mind_abi: "structural=1".into(),
        },
    };
    let edit_base = roll.mind(GOD, "Original").unwrap().base();
    let edit = roll
        .prepare_edit(GOD, "Original", edit_base, draft2)
        .unwrap();
    roll.publish_with(edit, |ops| retain_ops(&mut disk, ops))
        .unwrap();
    let base2 = roll.mind(GOD, "Original").unwrap().base();
    let p2 = roll
        .prepare_save(GOD, "Original", base2, "v2", None, &sources, &sources)
        .unwrap();
    let (c2, s2, rev2_ref) = p2.into_parts();
    sources.0.insert(s2.token.0, s2.token.1);
    roll.publish_with(c2, |ops| retain_ops(&mut disk, ops))
        .unwrap();
    let rev2 = roll.revision(GOD, rev2_ref.key).unwrap().clone();

    let mut world = World::new_minimal();
    let buyer = world
        .create(ObjKind::Player, "Buyer", GOD, Some(mush_core::LIMBO))
        .unwrap();
    let service_actor = world
        .create(ObjKind::Player, "Service", GOD, Some(mush_core::LIMBO))
        .unwrap();

    // Seed accounts: 0 = GOD (credit -10000), 1 = roll.registry sink, 2 = Buyer (credit -10000).
    let instrument = InstrumentDef {
        id: InstrumentId::new("cr").unwrap(),
        ruleset: "fixture".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::Allow,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    };
    disk.insert(
        ("economy".into(), "instrument:cr".into()),
        json!(instrument),
    );
    for (id, owner, credit) in [
        (
            0,
            PrincipalId::Object(GOD),
            CreditPolicy {
                allow_negative: true,
                floor: Some(Amount::from_minor(-10_000)),
            },
        ),
        (
            1,
            PrincipalId::System(SystemPrincipalId::new("roll.registry").unwrap()),
            CreditPolicy::forbid(),
        ),
        (2, PrincipalId::Object(buyer), CreditPolicy::forbid()),
        (
            3,
            PrincipalId::System(SystemPrincipalId::new("test.mint").unwrap()),
            CreditPolicy::unbounded(),
        ),
    ] {
        let account = Account {
            id: AccountId(id),
            owner,
            instrument: instrument.id.clone(),
            scope: None,
            name: format!("fixture {id}").into(),
            status: AccountStatus::Active,
            posted_balance: Amount::ZERO,
            held_amount: Amount::ZERO,
            credit,
            last_journal_seq: 0,
            revision: 1,
            approval: None,
            last_hold_seq: 0,
        };
        disk.insert(("economy".into(), format!("account:{id}")), json!(account));
    }

    let issuer = SystemPrincipalId::new("roll.registry").unwrap();
    let catalog = CatalogRef {
        ruleset: "roll".into(),
        code: "mind_charter".into(),
    };
    let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(1),
        Amount::from_minor(100),
        catalog.clone(),
    )
    .unwrap();
    let economy_part = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("economy").unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").unwrap(),
                DefinitionId::new("initial_filing").unwrap(),
                1,
                policy.definition_payload(),
            )
            .unwrap(),
        )
        .unwrap();
    let retention_part = crate::commerce::authored_retirement::retention_reference().unwrap();
    let participants = PurchaseParticipants::new(economy_part, retention_part).unwrap();
    let pending_cfg = PendingCharterTransferConfig::new(service_actor, 3600).unwrap();

    let make_cfg = || {
        config()
            .with_original_market_quotes()
            .with_author_withdrawal()
            .unwrap()
            .with_compatible_updates()
            .unwrap()
            .with_charter_transfers(CharterTransferConfig::new())
            .unwrap()
            .with_pending_charter_transfers(pending_cfg.clone(), participants.clone())
            .unwrap()
    };
    let make_engine = |w: World, srcs: Sources| -> (Box<Engine>, Handles, CommerceConfig) {
        let cfg = make_cfg();
        let econ = EconomyModule::new()
            .with_title_issuers(vec![issuer.clone()])
            .with_initial_filing(policy.clone(), cfg.initial_filing_action().unwrap())
            .with_compatible_mind_updates(cfg.compatible_update_action().unwrap())
            .with_mind_charter_transfers(CharterTransferConfig::new().actions().unwrap()[1].clone())
            .with_mind_charter_pending(
                pending_cfg
                    .economy_config(issuer.clone(), catalog.clone(), None)
                    .unwrap(),
            );
        let handles = Arc::new(Mutex::new(None));
        let engine = boxed_test_engine(
            w,
            Registry::new(),
            vec![
                Box::new(ReadManifest(None)),
                Box::new(HydratedRoll {
                    module: RollModule::with_writers(["tacsim"]).with_commerce(cfg.clone()),
                    sources: srcs,
                    handles: handles.clone(),
                }),
                Box::new(econ),
                Box::new(OrigMarket),
            ],
        )
        .unwrap();
        let h = handles.lock().unwrap().clone().unwrap();
        (engine, h, cfg)
    };

    // Stage ratings for rev1 and rev2.
    let (mut staging, (_, ratings), _) = make_engine(World::new_minimal(), sources.clone());
    let manifest = reference("tacsim", manifest_descriptor());
    let mut seed_rating = |rev: &Revision, tok: u8| -> crate::commerce::rating::RatingRef {
        let identity = RevisionIdentity::capture(rev).unwrap();
        let def = DefinitionDigest::for_canonical_value(
            "roll.filing.fixture.definition.v1",
            &rev.envelope().profiles,
        )
        .unwrap();
        let subject = RatingSubject::new(
            identity,
            DefinitionDigest::for_canonical_value(
                "roll.filing.fixture.envelope.v1",
                &json!(rev.envelope()),
            )
            .unwrap(),
            schema_line_digest(rev.envelope()).unwrap(),
            def,
            def,
            RatingClass::I,
        );
        let plan = DefinitionDigest::for_canonical_value(
            "roll.filing.fixture.plan.v1",
            &json!({"case": tok}),
        )
        .unwrap();
        let complete = crate::commerce::rating::test_support::complete_rating(
            subject,
            RatingCompatibility::combat_v1(manifest.digest()),
            plan,
            token(tok),
            staging.world.world_ic_now(),
        )
        .unwrap();
        let rref = complete.receipt.reference().unwrap();
        let mut txn = staging.action_transaction(&ratings.receipts).unwrap();
        txn.insert(
            &ratings.receipts,
            &complete.receipt.publication().job(),
            complete.receipt.clone(),
        )
        .unwrap();
        for page in &complete.pages {
            txn.insert(&ratings.pages, &page.key(), page.clone())
                .unwrap();
        }
        txn.insert(&ratings.jobs, &complete.job.job(), complete.job.clone())
            .unwrap();
        txn.insert(
            &ratings.executions,
            &complete.job.job(),
            complete.execution.clone(),
        )
        .unwrap();
        txn.insert(
            &ratings.completions,
            &complete.completion.job(),
            complete.completion.clone(),
        )
        .unwrap();
        txn.insert(
            &ratings.status,
            &complete.receipt.publication().job(),
            complete.status.clone(),
        )
        .unwrap();
        let (ops, install) = txn.prepare().unwrap().into_parts();
        retain_ops(&mut disk, &ops);
        staging.install_prepared_action(install).unwrap();
        for anchor in [
            RootAnchor::Rating {
                job: complete.receipt.publication().job(),
                id: complete.receipt.id(),
            },
            RootAnchor::RatingJob(complete.job.job()),
        ] {
            let row = crate::model::ReferenceRecord {
                format: crate::FORMAT,
                key: anchor.key(),
                revisions: vec![rev.reference()],
            };
            row.validate().unwrap();
            disk.insert((crate::NS.into(), row.key.key()), json!(row));
        }
        rref
    };
    let rating1 = seed_rating(&rev1, 2);
    let rating2 = seed_rating(&rev2, 22);

    let (mut engine, _, cfg) = make_engine(world, sources.clone());
    engine.set_account_identity(Arc::new(TestIdentity(
        mush_core::account::AccountIdentityEpoch::default(),
    )));
    load(&mut engine, &disk).unwrap();

    // Fund Buyer's account (2) with positive posted balance so the pending
    // charter escrow hold satisfies Invariant 26 (holds require non-negative
    // available balance, never credit headroom).
    {
        let mut out = Outbox::new();
        let mut fx = mush_core::engine::Effects::new();
        engine.step(
            mush_core::Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@econ/mint 3=2 5000".into(),
            },
            &mut out,
            &mut fx,
        );
        retain_ops(&mut disk, &engine.take_module_ops());
    }

    // Reserve shell & complete initial filing.
    let reserve = reference(
        "roll",
        descriptor(
            "action",
            mush_core::reserved_shell::RESERVE_ACTION,
            json!({"format":1,"purpose":"roll.initial_filing","object":"inert"}),
        ),
    );
    let shell = ObjId(engine.world.next_id());
    let (prepared, _) = engine
        .prepare_registered_composed_authoritative_action(
            &reserve,
            &invocation(3, json!({})),
            token(3),
        )
        .unwrap();
    let (writes, install) = prepared.into_parts();
    retain(&mut disk, &writes);
    engine.install_composed_action(install).unwrap();

    let filing_inv = invocation(
        4,
        json!({
            "shell_request": RequestUid::from_bytes([3; 16]),
            "mind": rev1.reference().key.mind,
            "revision": rev1.reference().key.number,
            "revision_digest": rev1.reference().digest,
            "rating_job": rating1.job(),
            "rating_id": rating1.id(),
            "rating_digest": rating1.receipt_digest(),
            "payer": 0,
            "charter_name": "Original",
        }),
    );
    let (prepared, _) = engine
        .prepare_registered_composed_authoritative_action(
            &cfg.initial_filing_action().unwrap(),
            &filing_inv,
            token(4),
        )
        .unwrap();
    let (writes, install) = prepared.into_parts();
    retain(&mut disk, &writes);
    engine.install_composed_action(install).unwrap();

    LockRig {
        engine,
        cfg,
        pending_cfg,
        participants,
        issuer,
        catalog,
        sources,
        disk,
        buyer,
        service_actor,
        shell,
        rev2,
        rating2,
    }
}

fn read_orig(
    engine: &mut Engine,
    actor: ObjId,
    mode: &str,
) -> Result<OriginalSaleFacts, OwnerReadError> {
    engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 1,
            name: "pending.orig.test".into(),
            args: json!(mode),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    engine
        .world
        .module_state::<OrigAnswers>()
        .unwrap()
        .lock()
        .unwrap()
        .take()
        .unwrap()
}

fn build_entitlement_engine(rig: &LockRig) -> Box<Engine> {
    let mut world = World::new_minimal();
    let b = world
        .create(ObjKind::Player, "Buyer", GOD, Some(mush_core::LIMBO))
        .unwrap();
    let s = world
        .create(ObjKind::Player, "Service", GOD, Some(mush_core::LIMBO))
        .unwrap();
    let sh = world
        .create(ObjKind::Thing, "Original", GOD, Some(mush_core::LIMBO))
        .unwrap();
    assert_eq!(b, rig.buyer);
    assert_eq!(s, rig.service_actor);
    assert_eq!(sh, rig.shell);
    let econ = EconomyModule::new()
        .with_title_issuers(vec![rig.issuer.clone()])
        .with_initial_filing(
            mush_economy::registry_filing::InitialFilingPolicy::new(
                rig.issuer.clone(),
                AccountId(1),
                Amount::from_minor(100),
                rig.catalog.clone(),
            )
            .unwrap(),
            rig.cfg.initial_filing_action().unwrap(),
        )
        .with_mind_charter_transfers(CharterTransferConfig::new().actions().unwrap()[1].clone())
        .with_mind_charter_pending(
            rig.pending_cfg
                .economy_config(rig.issuer.clone(), rig.catalog.clone(), None)
                .unwrap(),
        );
    let mut engine = boxed_test_engine(
        world,
        Registry::new(),
        vec![
            Box::new(Manifest),
            Box::new(EntitlementCheckRoll {
                module: RollModule::with_writers(["tacsim"]),
                sources: rig.sources.clone(),
                cfg: rig.cfg.clone(),
                pending_cfg: rig.pending_cfg.clone(),
                participants: rig.participants.clone(),
                handles: None,
            }),
            Box::new(econ),
            Box::new(EntMarket),
        ],
    )
    .unwrap();
    let filtered: Disk = rig
        .disk
        .iter()
        .filter(|((_, k), _)| !k.contains("initial_filing_shell"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    load(&mut engine, &filtered).unwrap();
    let econ_frozen = engine
        .world
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .is_frozen();
    let roll_ready = engine
        .world
        .module_state::<RollStore>()
        .unwrap()
        .commerce_ready();
    assert!(!econ_frozen, "economy froze in EntitlementCheckRoll");
    assert!(roll_ready.is_ok(), "roll not ready: {roll_ready:?}");
    engine
}

fn read_entitlement_from_disk(
    rig: &LockRig,
    actor: ObjId,
    mode: &str,
) -> Result<EntitlementSaleFacts, OwnerReadError> {
    let mut engine = build_entitlement_engine(rig);
    engine.step(
        mush_core::Inbound::Query {
            conn: 1,
            actor,
            id: 2,
            name: "pending.ent.test".into(),
            args: json!(mode),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    engine
        .world
        .module_state::<EntAnswers>()
        .unwrap()
        .lock()
        .unwrap()
        .take()
        .unwrap()
}

fn update_invocation(rig: &LockRig, n: u8) -> ActionInvocation {
    invocation(
        n,
        json!({
            "line": 1,
            "mind": rig.rev2.reference().key.mind,
            "revision": rig.rev2.reference().key.number,
            "revision_digest": rig.rev2.reference().digest,
            "rating_job": rig.rating2.job(),
            "rating_id": rig.rating2.id(),
            "rating_digest": rig.rating2.receipt_digest(),
            "payer": 0,
        }),
    )
}

fn withdraw_invocation(n: u8) -> ActionInvocation {
    invocation(n, json!({"filed": "1"}))
}

#[test]
fn funded_charter_pending_escrow_locks_sale_reader_entitlement_withdrawal_and_line_update() {
    // 1. Baseline control (no funded pending escrow): all four paths succeed.
    {
        let mut clear = build_rig();
        assert!(read_orig(&mut clear.engine, GOD, "orig_own").is_ok());
        assert!(read_orig(&mut clear.engine, clear.buyer, "orig_listed").is_ok());
        read_entitlement_from_disk(&clear, GOD, "ent_own")
            .expect("EntitlementSaleReader own must succeed when charter_pending is clear");

        let upd = update_invocation(&clear, 30);
        let (prepared, _) = clear
            .engine
            .prepare_registered_composed_authoritative_action(
                &clear.cfg.compatible_update_action().unwrap(),
                &upd,
                token(30),
            )
            .expect("line update must succeed when charter_pending is clear");
        let (_, install) = prepared.into_parts();
        clear.engine.install_composed_action(install).unwrap();

        let wd = withdraw_invocation(31);
        let (prepared, _) = clear
            .engine
            .prepare_registered_composed_authoritative_action(
                &withdrawal::action_reference().unwrap(),
                &wd,
                token(31),
            )
            .expect("withdrawal must succeed when charter_pending is clear");
        let (_, install) = prepared.into_parts();
        clear.engine.install_composed_action(install).unwrap();
    }

    // 2. Active funded pending charter escrow: Offer -> Fund, then verify ALL FOUR
    //    paths fail closed.
    let mut locked = build_rig();
    let offer_inv = actor_invocation(
        GOD,
        40,
        json!({
            "charter": locked.shell.0.to_string(),
            "buyer": locked.buyer.0.to_string(),
            "seller_account": "0",
            "price_minor": "500",
            "valid_for_seconds": "3600",
        }),
    );
    let (prepared, _) = locked
        .engine
        .prepare_registered_composed_authoritative_action(
            &pending_proto::offer_action().unwrap(),
            &offer_inv,
            token(40),
        )
        .expect("pending charter offer must succeed");
    let (writes, install) = prepared.into_parts();
    retain(&mut locked.disk, &writes);
    locked.engine.install_composed_action(install).unwrap();

    // Unfunded offer alone does NOT lock yet.
    assert!(read_orig(&mut locked.engine, GOD, "orig_own").is_ok());
    assert!(read_entitlement_from_disk(&locked, GOD, "ent_own").is_ok());

    let fund_inv = actor_invocation(
        locked.buyer,
        41,
        json!({
            "offer": token(40),
            "payer": "2",
        }),
    );
    let (prepared, _) = locked
        .engine
        .prepare_registered_composed_authoritative_action(
            &pending_proto::fund_action().unwrap(),
            &fund_inv,
            token(41),
        )
        .expect("pending charter funding must succeed");
    let (writes, install) = prepared.into_parts();
    retain(&mut locked.disk, &writes);
    locked.engine.install_composed_action(install).unwrap();

    // Now PendingState::Funded {} is active: all four paths MUST refuse!
    assert_eq!(
        read_orig(&mut locked.engine, GOD, "orig_own"),
        Err(OwnerReadError::Refused),
        "OriginalSaleReader own must refuse under funded charter_pending escrow"
    );
    assert_eq!(
        read_orig(&mut locked.engine, locked.buyer, "orig_listed"),
        Err(OwnerReadError::Refused),
        "OriginalSaleReader listed must refuse under funded charter_pending escrow"
    );
    assert_eq!(
        read_entitlement_from_disk(&locked, GOD, "ent_own"),
        Err(OwnerReadError::Refused),
        "EntitlementSaleReader own must refuse under funded charter_pending escrow"
    );
    assert_eq!(
        read_entitlement_from_disk(&locked, locked.buyer, "ent_listed"),
        Err(OwnerReadError::Refused),
        "EntitlementSaleReader listed must refuse under funded charter_pending escrow"
    );

    let upd = update_invocation(&locked, 42);
    assert!(
        locked
            .engine
            .prepare_registered_composed_authoritative_action(
                &locked.cfg.compatible_update_action().unwrap(),
                &upd,
                token(42),
            )
            .is_err(),
        "compatible line_update must refuse under funded charter_pending escrow"
    );

    let wd = withdraw_invocation(43);
    assert!(
        locked
            .engine
            .prepare_registered_composed_authoritative_action(
                &withdrawal::action_reference().unwrap(),
                &wd,
                token(43),
            )
            .is_err(),
        "author withdrawal must refuse under funded charter_pending escrow"
    );
}
