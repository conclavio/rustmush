//! **`@registry/fine <inspection> <amount>`: the Registry fines what an inspection found**
//! (doc 17h cut M8d D3; doc 17 §10, D-UM-42). A Registry operator holding
//! `roll.registry.fine` on the inspection's scope (its own capability and its own observation,
//! `RegisteredInspectionWriter::authorize_fine`) fines the hull's title holder, read live at
//! the fine (the lead's ruling), for an inspection that read an ungoverned core, up to
//! `[tacsim.minds].fine_cap_minor`.
//!
//! **Two steps, one writer each.** The command stages `registry_mind_fine`; its provider
//! re-observes the grant, re-reads the inspection and the holder, and writes one
//! [`FineRecord`] into this crate's `tacsim.registry_fines` family — the durable fact of the
//! fine, keyed by the inspection's ring slot. The economy's obligation is not an action-state
//! family, so the action cannot write it: the fine tick (`tick`, GOD, coarse 1 s, the economy
//! carve-out lane the relay capitalization tick posts on) turns each recorded fine into one
//! `mush_economy::create_sanction_obligation` under the recording operator's grant, re-checked
//! live, keyed by the fine so a second fire or a restart levies nothing twice. The
//! obligation's own grace, delinquency and notices tell the holder (D8); nothing names the
//! core to anyone else.
//!
//! **The holder** (fix round K1, the lead's ruling): the hull's title holder **recorded with the
//! inspection** (`InspectionRecord::holder`), so a title moved after it dodges nothing. A
//! character's fine debits their own credit account and an organisation's (M8c) its treasury on
//! its root scope; a holder with none gets one opened at the levy and carries the debt
//! (`levy_account`) — a fine is never refused for want of an account. An inspection that
//! recorded no holder, or a system principal, answers `NO_FINE_HOLDER`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, ModError, ObjId, PerceivedRef,
    PrepareCtx, Registrar, StateFamilyHandle, World, WorldEvent, WorldEventKind, WorldObserver,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{InspectionHost, InspectionRecord, unwarranted};
use crate::minds::market_commands::words::{self, fill};

/// The record family.
pub const FINE_FAMILY: &str = "tacsim.registry_fines";
/// The fine action's id (the Roll's policy names it).
pub const FINE_ACTION: &str = "registry_mind_fine";
/// The fine outcome's id.
pub const FINE_OUTCOME: &str = "registry_mind_fined";
const MAX_FINE_BYTES: usize = 2_048;
const TICK: &str = "registry.fines";
const TICK_SECS: u64 = 1;
const FINES_PER_TICK: usize = 32;

/// **One recorded fine**: what the provider proved at the fine, so the tick levies exactly it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FineRecord {
    pub format: u16,
    /// The fine's own commit.
    pub fine: mush_core::CommitToken,
    /// The fining operator, whose grant levies it.
    pub operator: ObjId,
    pub hull: ObjId,
    /// The inspection's sequence on the hull, and its commit.
    pub seq: u64,
    pub inspection: mush_core::CommitToken,
    /// The hull's title holder recorded with the inspection (K1): a character, or an
    /// organisation (M8c). Its account is resolved, or opened, at the levy.
    pub holder: mush_org::PrincipalId,
    /// The levy sink, paid.
    pub credit: mush_economy::AccountId,
    pub amount_minor: i64,
    /// The fine's IC second: the obligation's one due instant.
    pub at_ic: i64,
}

impl FineRecord {
    /// The census.
    ///
    /// # Errors
    /// The first field out of bounds.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.operator.0 == 0
            || self.hull.0 == 0
            || self.seq == 0
            || matches!(
                self.holder,
                mush_org::PrincipalId::System(_) | mush_org::PrincipalId::Object(ObjId(0))
            )
            || self.amount_minor < 1
        {
            return Err("Invalid Registry fine".into());
        }
        Ok(())
    }

    /// The key the obligation's cause carries: the hull and the inspection's sequence.
    #[must_use]
    pub fn sanction_key(&self) -> SmolStr {
        sanction_key(self.hull, self.seq)
    }
}

/// `hull/<8 hex>/inspection/<seq>`: a record key, never a player surface.
#[must_use]
pub fn sanction_key(hull: ObjId, seq: u64) -> SmolStr {
    SmolStr::new(format!("hull/{:08x}/inspection/{seq}", hull.0))
}

/// The family over the inspection's slot keys.
///
/// # Errors
/// A registrar that refuses it.
pub(super) fn family(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<String, FineRecord>, ModError> {
    let descriptor = crate::port::families::family_descriptor(
        FINE_FAMILY,
        "tacsim.registry_fines.key.v1",
        "tacsim.registry_fines",
    )?
    .with_limits(64, MAX_FINE_BYTES)
    .map_err(|e| ModError::msg(e.to_string()))?;
    reg.state_family(
        descriptor,
        |key: &String| Ok(key.clone()),
        |v: &serde_json::Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        FineRecord::validate,
        |record: &FineRecord| serde_json::to_value(record).map_err(|e| e.to_string()),
    )
}

/// What the slot holds when the fine composes.
#[derive(Clone)]
pub struct FineHost {
    pub(crate) action: DefinitionRef,
    pub(crate) family: StateFamilyHandle<String, FineRecord>,
    pub(crate) pending: Pending,
    /// The slots paged this boot (RAM): a fine that cannot be levied pages once.
    pub(crate) paged: Arc<Mutex<BTreeSet<String>>>,
}

impl FineHost {
    /// Whether `key` is paged for the first time this boot.
    fn paged(&self, key: &str) -> bool {
        self.paged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned())
    }
    /// The record family.
    #[must_use]
    pub fn family(&self) -> &StateFamilyHandle<String, FineRecord> {
        &self.family
    }
}

/// The fine slots announced and not yet levied (RAM).
#[derive(Clone, Default)]
pub(crate) struct Pending(Arc<Mutex<BTreeSet<String>>>);

impl Pending {
    fn note(&self, key: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
    }
    fn take(&self, n: usize) -> Vec<String> {
        let mut set = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let taken: Vec<String> = set.iter().take(n).cloned().collect();
        for key in &taken {
            set.remove(key);
        }
        taken
    }
}

/// The principal a fine on `record` debits: the holder the inspection recorded (K1), never a
/// system principal or object 0.
pub(crate) fn fined_principal(record: &InspectionRecord) -> Option<mush_org::PrincipalId> {
    match record.holder.clone()? {
        mush_org::PrincipalId::Object(c) if c.0 != 0 => Some(mush_org::PrincipalId::Object(c)),
        mush_org::PrincipalId::Organization(org) => Some(mush_org::PrincipalId::Organization(org)),
        mush_org::PrincipalId::Object(_) | mush_org::PrincipalId::System(_) => None,
    }
}

/// The account a fine on `principal` debits, when it has one: a character's own unscoped,
/// active credit account (the lowest id), or an organisation's one active credit treasury on its
/// root scope (M8c's treasury rule; two are ambiguous and read as none).
fn account_of(w: &World, principal: &mush_org::PrincipalId) -> Option<mush_economy::AccountId> {
    let economy = w.module_state::<mush_economy::EconomyStore>()?;
    let snapshot = economy.snapshot();
    let credit = |a: &&mush_economy::Account| {
        a.status == mush_economy::AccountStatus::Active
            && a.instrument.as_str() == crate::economy::CREDIT
            && a.owner == *principal
    };
    match principal {
        mush_org::PrincipalId::Object(_) => snapshot
            .accounts()
            .filter(credit)
            .filter(|a| a.scope.is_none())
            .map(|a| a.id)
            .min(),
        mush_org::PrincipalId::Organization(org) => {
            let scope = mush_roll::commerce::org_holder::root_scope(w, *org)?;
            let mut treasuries = snapshot
                .accounts()
                .filter(credit)
                .filter(|a| a.scope == Some(scope));
            let account = treasuries.next()?.id;
            treasuries.next().is_none().then_some(account)
        }
        mush_org::PrincipalId::System(_) => None,
    }
}

/// **The account the levy debits** (K1's carried-debt rule): the principal's own, or one opened
/// now — a character's personal credit account, an organisation's treasury on its root scope —
/// so the fine is recorded as an obligation on them however empty-handed they are.
fn levy_account(
    ctx: &mut Ctx<'_>,
    principal: &mush_org::PrincipalId,
) -> Option<mush_economy::AccountId> {
    if let Some(account) = account_of(ctx.w, principal) {
        return Some(account);
    }
    let (scope, name) = match principal {
        mush_org::PrincipalId::Object(c) => (
            None,
            ctx.w
                .get(*c)
                .map(|o| o.name.to_string())
                .unwrap_or_default(),
        ),
        mush_org::PrincipalId::Organization(org) => (
            Some(mush_roll::commerce::org_holder::root_scope(ctx.w, *org)?),
            "Registry fines".to_owned(),
        ),
        mush_org::PrincipalId::System(_) => return None,
    };
    let id = ctx
        .w
        .module_state_mut::<mush_economy::EconomyStore>()
        .and_then(mush_economy::EconomyStore::alloc_account)?;
    mush_economy::open_account(
        ctx,
        mush_economy::Account {
            id,
            owner: principal.clone(),
            instrument: mush_economy::InstrumentId::new(crate::economy::CREDIT)?,
            scope,
            name: smol_str::SmolStr::new(name),
            status: mush_economy::AccountStatus::Active,
            posted_balance: mush_economy::Amount::ZERO,
            held_amount: mush_economy::Amount::ZERO,
            last_hold_seq: 0,
            credit: mush_economy::CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    Some(id)
}

/// The configured cap.
fn cap(w: &World) -> i64 {
    w.module_state::<crate::TacsimStore>()
        .map_or(0, |store| store.config().minds.fine_cap_minor)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    hull: u32,
    seq: u64,
    amount: i64,
}

pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    writer: mush_roll::commerce::registry_inspection::RegisteredInspectionWriter,
    family: StateFamilyHandle<String, FineRecord>,
    credit: mush_economy::AccountId,
    refusal: ActionError,
}

impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        writer: mush_roll::commerce::registry_inspection::RegisteredInspectionWriter,
        family: StateFamilyHandle<String, FineRecord>,
        credit: mush_economy::AccountId,
    ) -> Result<Self, ModError> {
        Ok(Self {
            action,
            outcome,
            writer,
            family,
            credit,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry fine is unavailable or stale.",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }

    fn request(&self, inv: &ActionInvocation) -> Result<Parameters, ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        if p.hull == 0 || p.seq == 0 || p.amount < 1 {
            return Err(self.refusal.clone());
        }
        Ok(p)
    }

    fn definitions(&self) -> Result<Vec<DefinitionRef>, ActionError> {
        let authority = self.writer.authority().map_err(|_| self.refusal.clone())?;
        Ok(vec![self.action.clone(), self.outcome.clone(), authority])
    }

    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<FineRecord, ActionError> {
        let fail = || self.refusal.clone();
        let p = self.request(inv)?;
        let hull = ObjId(p.hull);
        let world = ctx.world().map_err(|_| fail())?;
        // The fine's own capability, observed live, before any hull is read.
        let (_, at_ic) = self
            .writer
            .authorize_fine(world, inv.actor())
            .map_err(|_| fail())?;
        let inspections = self.writer.family().map_err(|_| fail())?;
        let key = mush_roll::commerce::registry_inspection::inspection_key(hull, p.seq);
        let record: InspectionRecord = txn
            .get(&inspections, &key)
            .map_err(|_| fail())?
            .cloned()
            .ok_or_else(fail)?;
        if record.hull != hull
            || record.seq != p.seq
            || !unwarranted::unwarranted(&record)
            || p.amount > cap(world)
        {
            return Err(fail());
        }
        // A fine of this or a later inspection held, or (K10) an older one not yet levied, keeps
        // the slot.
        if txn
            .get(&self.family, &key)
            .map_err(|_| fail())?
            .is_some_and(|held| held.seq >= p.seq || !levied(world, held))
        {
            return Err(fail());
        }
        let holder = fined_principal(&record).ok_or_else(fail)?;
        let fine = FineRecord {
            format: 1,
            fine: ctx.commit_token(),
            operator: inv.actor(),
            hull,
            seq: p.seq,
            inspection: record.inspection,
            holder,
            credit: self.credit,
            amount_minor: p.amount,
            at_ic: at_ic.at,
        };
        fine.validate().map_err(|_| fail())?;
        if txn.get(&self.family, &key).map_err(|_| fail())?.is_some() {
            txn.replace(&self.family, &key, fine.clone())
                .map_err(|_| fail())?;
        } else {
            txn.insert(&self.family, &key, fine.clone())
                .map_err(|_| fail())?;
        }
        Ok(fine)
    }
}

impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv).map(|_| ())
    }

    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let fine = self.prepare(inv, txn, ctx)?;
        let summary = fill(
            words::FINED,
            &[
                ("hull", &fine.hull.0.to_string()),
                ("amount", &crate::launch::credits(fine.amount_minor)),
            ],
        );
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "hull": fine.hull.0,
                "seq": fine.seq,
                "amount": fine.amount_minor,
                "operator": fine.operator.0,
                "summary": summary,
            }),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    /// The receipt the prepare rendered, for the fining operator only.
    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(self.refusal.clone());
        }
        let data = outcome.data();
        let operator = data
            .get("operator")
            .and_then(serde_json::Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| self.refusal.clone())?;
        if viewer != &(PrincipalId::Object { id: operator }) {
            return Err(self.refusal.clone());
        }
        let summary = data
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| self.refusal.clone())?;
        mush_proto::ProjectedActionResult::new(FINE_ACTION, summary, data.clone())
            .map(Some)
            .map_err(|_| self.refusal.clone())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !self.writer.fine_offered(&engine.world, query.actor())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in self.definitions()? {
            guards.add_definition(r, definitions).map_err(|_| fail())?;
        }
        let integer = |id: &str, maximum: i64| -> Result<_, ActionError> {
            Ok(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new(id).map_err(|_| fail())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum,
                },
            })
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![
                integer("hull", i64::from(u32::MAX))?,
                integer("seq", i64::MAX)?,
                integer("amount", i64::MAX)?,
            ])
            .map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-registry-fine",
                "Fine a hull's holder",
                "Fine the holder of a hull an inspection found ungoverned: one obligation, due once.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}

/// The `ActionStateChanged` observer: a changed fine slot is noted (RAM only).
struct Announce {
    family: StateFamilyHandle<String, FineRecord>,
    pending: Pending,
}

impl WorldObserver for Announce {
    fn on_event(&self, _: &mut Ctx<'_>, ev: &WorldEvent) {
        let WorldEvent::ActionStateChanged {
            owner,
            family,
            keys,
        } = ev
        else {
            return;
        };
        if owner != self.family.owner() || family != self.family.descriptor().id() {
            return;
        }
        for key in keys {
            self.pending.note(key.to_string());
        }
    }
}

/// Register the observer and the tick.
pub(super) fn register_levy(reg: &mut Registrar<'_>, host: &FineHost) -> Result<(), ModError> {
    reg.subscribe(
        WorldEventKind::ActionStateChanged,
        Arc::new(Announce {
            family: host.family.clone(),
            pending: host.pending.clone(),
        }),
    );
    reg.tick(
        TICK,
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(TICK_SECS)),
        Arc::new(tick),
    )?;
    Ok(())
}

/// The boot pass: note every known hull's fine slots that hold a fine no sanction carries yet,
/// so a fine recorded just before a restart is levied after it (a slot the boot cannot read
/// is noted too, for the tick to read).
pub(super) fn boot(ctx: &Ctx<'_>) {
    let Some(fines) = InspectionHost::of(ctx.w).and_then(|h| h.fines.as_ref()) else {
        return;
    };
    for hull in super::facts::known_hulls(ctx.w) {
        for key in mush_roll::commerce::registry_inspection::inspection_slots(hull) {
            match ctx.action_state(&fines.family, &key) {
                Ok(Some(record)) if !levied(ctx.w, record) => fines.pending.note(key),
                Ok(_) => {}
                Err(_) => fines.pending.note(key),
            }
        }
    }
}

/// **The levy tick** (GOD): each noted slot's fine, if no sanction already carries its key,
/// becomes one obligation under the recording operator's live grant, on the recorded holder's
/// account (opened when they have none). A retryable refusal — the economy frozen or busy, the
/// grant briefly withdrawn — re-notes the slot for the next fire; any other pages staff once.
fn tick(ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>) {
    let Some(host) = InspectionHost::of(ctx.w) else {
        return;
    };
    let Some(fines) = host.fines.clone() else {
        return;
    };
    let Ok(policy) = host.writer.policy() else {
        return;
    };
    let Some(capability) = mush_org::CapabilityId::new(
        mush_roll::commerce::registry_inspection::REGISTRY_FINE_CAPABILITY,
    ) else {
        return;
    };
    for key in fines.pending.take(FINES_PER_TICK) {
        let record = match ctx.action_state(&fines.family, &key) {
            Ok(Some(record)) => record.clone(),
            Ok(None) => continue,
            Err(error) => {
                tracing::debug!(?error, %key, "Registry fine: slot unread");
                fines.pending.note(key);
                continue;
            }
        };
        if levied(ctx.w, &record) {
            continue;
        }
        let Some(debit) = levy_account(ctx, &record.holder) else {
            fines.pending.note(key);
            continue;
        };
        let spec = mush_economy::SanctionSpec {
            debit_account: debit,
            credit_account: record.credit,
            amount: mush_economy::Amount::from_minor(record.amount_minor),
            due_ic: record.at_ic,
            grace: mush_economy::GracePolicy::new(1),
            inspection: record.sanction_key(),
            capability: capability.clone(),
            scope: policy.scope(),
            levied_by: record.operator,
        };
        match mush_economy::create_sanction_obligation(ctx, spec) {
            Ok(_) => {}
            Err(error) if retryable(&error) => {
                tracing::debug!(%error, hull = record.hull.0, "Registry fine: levy retried");
                fines.pending.note(key);
            }
            Err(error) => {
                if fines.paged(&key) {
                    tracing::error!(
                        %error,
                        hull = record.hull.0,
                        "tacsim: a Registry fine cannot be levied (an account closed or frozen, \
                         an instrument retired); it stays recorded, and staff settle it"
                    );
                }
            }
        }
    }
}

/// Whether a levy refusal clears by itself (K8): the economy frozen, its journal unavailable,
/// a retry asked, or the levier's grant withdrawn for now.
fn retryable(error: &mush_economy::EconomyError) -> bool {
    matches!(
        error,
        mush_economy::EconomyError::EconomyFrozen
            | mush_economy::EconomyError::JournalUnavailable
            | mush_economy::EconomyError::RetryShortly
            | mush_economy::EconomyError::Unauthorized
    )
}

/// Whether a sanction already carries this fine's key.
fn levied(w: &World, record: &FineRecord) -> bool {
    let key = record.sanction_key();
    w.module_state::<mush_economy::EconomyStore>()
        .is_some_and(|economy| {
            economy.snapshot().obligations().any(|o| {
                matches!(&o.cause, mush_economy::ObligationCause::Sanction { inspection } if *inspection == key)
            })
        })
}

/// **The command body** (`@registry/fine #<hull>[/<n>] <amount>`): the hull's newest
/// inspection, or its `n`th; the amount in credits as players type them. Every refusal is one
/// sentence; a non-operator is told the driver's one sentence before any hull is read.
pub(crate) fn command(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let (target, amount) = raw.split_once(' ').ok_or_else(|| CmdError::msg(usage))?;
    let target = target.strip_prefix('#').unwrap_or(target);
    let (hull, seq) = match target.split_once('/') {
        Some((hull, seq)) => (hull, Some(seq)),
        None => (target, None),
    };
    let number = |text: &str| crate::minds::market_commands::author::number(text);
    let hull = number(hull)
        .and_then(|n| u32::try_from(n).ok())
        .map(ObjId)
        .ok_or_else(|| CmdError::msg(usage))?;
    let seq = match seq {
        Some(text) => Some(number(text).ok_or_else(|| CmdError::msg(usage))?),
        None => None,
    };
    let actor = ctx.actor;
    let (action, key, minor, seq) = {
        let w: &World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::FINE_OFF))?;
        let fines = host
            .fines
            .as_ref()
            .ok_or_else(|| CmdError::msg(words::FINE_OFF))?;
        if !host.writer.fine_offered(w, actor) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        let cap = cap(w);
        let minor = crate::minds::market_commands::market::credits_minor(amount.trim())
            .filter(|minor| *minor <= cap)
            .ok_or_else(|| {
                CmdError::msg(fill(
                    words::FINE_RANGE,
                    &[("amount", &crate::launch::credits(cap))],
                ))
            })?;
        let inspections = host
            .writer
            .family()
            .map_err(|_| CmdError::msg(words::FINE_OFF))?;
        let ring: Vec<InspectionRecord> =
            mush_roll::commerce::registry_inspection::inspection_slots(hull)
                .filter_map(|slot| {
                    ctx.action_state(&inspections, &slot)
                        .ok()
                        .flatten()
                        .cloned()
                })
                .filter(|record| record.hull == hull)
                .collect();
        let record = match seq {
            Some(seq) => ring.into_iter().find(|r| r.seq == seq),
            None => ring.into_iter().max_by_key(|r| r.seq),
        }
        .ok_or_else(|| CmdError::msg(words::NO_SUCH_INSPECTION))?;
        if !unwarranted::unwarranted(&record) {
            return Err(CmdError::msg(words::NOTHING_TO_FINE));
        }
        let slot = mush_roll::commerce::registry_inspection::inspection_key(hull, record.seq);
        match ctx.action_state(&fines.family, &slot).ok().flatten() {
            Some(held) if held.seq >= record.seq => {
                return Err(CmdError::msg(words::ALREADY_FINED));
            }
            // K10: an older fine in the slot not yet levied keeps it.
            Some(held) if !levied(w, held) => {
                return Err(CmdError::msg(words::INSPECT_REFUSED));
            }
            _ => {}
        }
        if fined_principal(&record).is_none() {
            return Err(CmdError::msg(words::NO_FINE_HOLDER));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "fine",
            &format!("{}/{}", hull.0, record.seq),
            w.world_ic_now().at,
        )?;
        (fines.action.clone(), key, minor, record.seq)
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({"hull": hull.0, "seq": seq, "amount": minor}),
        key,
    );
    ctx.out.info(
        actor,
        fill(
            words::FINING,
            &[
                ("hull", &hull.0.to_string()),
                ("amount", &crate::launch::credits(minor)),
            ],
        ),
    );
    Ok(())
}
