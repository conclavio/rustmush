//! **`@registry/seize <hull>` and `@registry/forfeit <hull>[/<n>]`: the Registry impounds a
//! core in place** (doc 17h cut M8d2 S4/S5; doc 17 §10, D-UM-42; the lead's S3 ruling,
//! "impounded in place"). A Registry operator holding `roll.registry.seize` (its own
//! capability and observation, `RegisteredInspectionWriter::authorize_seize`) seizes the core
//! fitted on a hull whose **newest** inspection grounds it — an ungoverned core, or a `grey` or
//! `heirloom` one (M8b-2) — no older than `[tacsim.minds].seizure_inspection_max_age_ic_days`.
//!
//! **One act, one union.** The provider proves the inspection
//! ([`mush_roll::commerce::registry_inspection::seizure::prove`]) in its own transaction, binds
//! the proof into the economy's closed seizure plan through the economy participant
//! (`EconomyRequest::CoreSeizure`: the core title `Disputed { the Registry org, the inspection
//! key }`), and writes one [`SeizureRecord`] (`roll_registry_seizures`, delegated from the Roll)
//! — all in the same coordinator union, the charter transfer's shape. The core stays fitted;
//! the seal reads it as no core from its title (`match_::seal::core_impounded`). `minds.seized`
//! reads the economy's replayed seizure receipt (`Findings`).
//!
//! **The forfeit** (`roll.registry.forfeit`): after `[tacsim.minds].seizure_hold_ic_days` from
//! the seizure, the economy's closed forfeit plan releases the claim and moves the core's title
//! to the Registry org (Q-M8-4); the record takes its forfeit fields. Before the hold: "That
//! seizure is still within its hold."; twice: the uniform refusal.
//!
//! **What the holder hears**: nothing from here. The core's title carries the Registry's claim
//! (`@asset/title`), and later the Registry's name; the box shows the trip's normal ROM lines.

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, ModError, ObjId, PerceivedRef,
    PrepareCtx, Registrar, StateFamilyHandle, World,
};
use mush_economy::registry_filing::seizure::{
    RegistryCoreSeizure, SeizureStep, inspection_key as seizure_key,
};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_roll::commerce::registry_inspection::seizure::{self as proof, NotSeizable};
use mush_roll::commerce::registry_inspection::{RegisteredInspectionWriter, SeizureRecord};
use serde::Deserialize;

use super::InspectionHost;
use crate::minds::market_commands::words::{self, fill};

/// The seizure action's id (the economy's closed plan names it).
pub const SEIZE_ACTION: &str = mush_economy::registry_filing::seizure::SEIZE_ACTION_ID;
/// The seizure outcome's id.
pub const SEIZE_OUTCOME: &str = "registry_core_seized";
/// The forfeit action's id.
pub const FORFEIT_ACTION: &str = mush_economy::registry_filing::seizure::FORFEIT_ACTION_ID;
/// The forfeit outcome's id.
pub const FORFEIT_OUTCOME: &str = "registry_core_forfeited";
const IC_DAY: i64 = 86_400;

/// **The two configured numbers** (`[tacsim.minds]`, Q-M8-6, provisional): how old an
/// inspection may be to ground a seizure, and how long a seizure is held before a forfeit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeizureTerms {
    pub max_age_ic_days: u32,
    pub hold_ic_days: u32,
}

/// What the slot holds when the seizure composes.
#[derive(Clone)]
pub struct SeizureHost {
    pub(crate) seize: DefinitionRef,
    pub(crate) forfeit: DefinitionRef,
    pub(crate) terms: SeizureTerms,
}

/// **The core asset fitted on `hull` now**: its tactical unit's core component's titled asset.
#[must_use]
pub fn fitted_core(world: &World, hull: ObjId) -> Option<ObjId> {
    let unit = mush_tactics::module::unit_of(world, hull)?;
    let hw = crate::arena::core::read_core(unit)?;
    let component = mush_tactics::ComponentId::new(hw.component.as_str()).ok()?;
    unit.component(&component)?.asset
}

fn title_revision(world: &World, core: ObjId) -> Option<u64> {
    world
        .module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .asset(core)
        .map(|title| title.revision)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    hull: u32,
    #[serde(default)]
    seq: Option<u64>,
}

/// **The provider of both acts**: `forfeit` picks which.
pub(super) struct Provider {
    forfeit: bool,
    action: DefinitionRef,
    outcome: DefinitionRef,
    writer: RegisteredInspectionWriter,
    economy: ParticipantHandle<InitialFilingParticipant>,
    terms: SeizureTerms,
    refusal: ActionError,
}

impl Provider {
    pub(super) fn new(
        forfeit: bool,
        (action, outcome): (DefinitionRef, DefinitionRef),
        writer: RegisteredInspectionWriter,
        economy: ParticipantHandle<InitialFilingParticipant>,
        terms: SeizureTerms,
    ) -> Result<Self, ModError> {
        Ok(Self {
            forfeit,
            action,
            outcome,
            writer,
            economy,
            terms,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry seizure is unavailable or stale.",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }

    fn request(&self, inv: &ActionInvocation) -> Result<(ObjId, Option<u64>), ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        if p.hull == 0 || p.seq == Some(0) || self.forfeit != p.seq.is_some() {
            return Err(self.refusal.clone());
        }
        Ok((ObjId(p.hull), p.seq))
    }

    fn definitions(&self) -> Result<Vec<DefinitionRef>, ActionError> {
        let authority = self.writer.authority().map_err(|_| self.refusal.clone())?;
        Ok(vec![self.action.clone(), self.outcome.clone(), authority])
    }

    /// The seizure: prove, bind the economy's Seize, record.
    fn seize(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        hull: ObjId,
    ) -> Result<SeizureRecord, ActionError> {
        let fail = || self.refusal.clone();
        let (proof, core, revision, now) = {
            let world = ctx.world().map_err(|_| fail())?;
            // The seizure's own capability, observed live, before any hull is read.
            let (_, now) = self
                .writer
                .authorize_seize(world, inv.actor())
                .map_err(|_| fail())?;
            let inspections = self.writer.family().map_err(|_| fail())?;
            let record = proof::newest(txn, &inspections, hull)
                .map_err(|_| fail())?
                .ok_or_else(fail)?;
            let fitted = fitted_core(world, hull);
            let proven = proof::prove(&record, fitted, &now, self.terms.max_age_ic_days)
                .map_err(|_| fail())?;
            let core = proven.core;
            let revision = title_revision(world, core).ok_or_else(fail)?;
            ((proven, record.inspection), core, revision, now)
        };
        let (proven, inspection) = proof;
        let seq = proven.seq;
        let family = self.writer.seizures().map_err(|_| fail())?;
        if txn
            .get(&family, &seizure_key(hull, seq))
            .map_err(|_| fail())?
            .is_some()
        {
            return Err(fail());
        }
        let native = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::CoreSeizure(RegistryCoreSeizure {
                    step: SeizureStep::Seize(proven),
                    core,
                    title_revision: revision,
                }),
            )
            .map_err(|_| fail())?
            .core_seizure()
            .map_err(|_| fail())?;
        if native.action() != &self.action || native.commit_token() != ctx.commit_token() {
            return Err(fail());
        }
        let hold = i64::from(self.terms.hold_ic_days)
            .checked_mul(IC_DAY)
            .ok_or_else(fail)?;
        let record = SeizureRecord {
            format: 1,
            inspection,
            hull,
            seq,
            core,
            seizure: native.commit_token(),
            at_ic: now.at,
            forfeit_after_ic: now.at.checked_add(hold).ok_or_else(fail)?,
            seized_by: inv.actor(),
            forfeit: None,
            forfeited_at_ic: None,
            forfeited_by: None,
        };
        self.writer
            .write_seizure(ctx, txn, record)
            .map_err(|_| fail())
    }

    /// The forfeit: the held record, past its hold; bind the economy's Forfeit; record.
    fn forfeit(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        (hull, seq): (ObjId, u64),
    ) -> Result<SeizureRecord, ActionError> {
        let fail = || self.refusal.clone();
        let family = self.writer.seizures().map_err(|_| fail())?;
        let held = txn
            .get(&family, &seizure_key(hull, seq))
            .map_err(|_| fail())?
            .cloned()
            .ok_or_else(fail)?;
        let (revision, now) = {
            let world = ctx.world().map_err(|_| fail())?;
            let (_, now) = self
                .writer
                .authorize_forfeit(world, inv.actor())
                .map_err(|_| fail())?;
            (title_revision(world, held.core).ok_or_else(fail)?, now)
        };
        if held.forfeited() || now.at < held.forfeit_after_ic {
            return Err(fail());
        }
        let native = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::CoreSeizure(RegistryCoreSeizure {
                    step: SeizureStep::Forfeit {
                        seizure: held.seizure,
                    },
                    core: held.core,
                    title_revision: revision,
                }),
            )
            .map_err(|_| fail())?
            .core_seizure()
            .map_err(|_| fail())?;
        if native.action() != &self.action || native.commit_token() != ctx.commit_token() {
            return Err(fail());
        }
        let record = SeizureRecord {
            forfeit: Some(native.commit_token()),
            forfeited_at_ic: Some(now.at),
            forfeited_by: Some(inv.actor()),
            ..held
        };
        self.writer
            .write_seizure(ctx, txn, record)
            .map_err(|_| fail())
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
        let (hull, seq) = self.request(inv)?;
        let (record, summary) = match seq {
            None => {
                let record = self.seize(inv, txn, ctx, hull)?;
                let summary = fill(
                    words::SEIZED,
                    &[
                        ("hull", &hull.0.to_string()),
                        ("days", &self.terms.hold_ic_days.to_string()),
                    ],
                );
                (record, summary)
            }
            Some(seq) => {
                let record = self.forfeit(inv, txn, ctx, (hull, seq))?;
                (
                    record,
                    fill(words::FORFEITED, &[("hull", &hull.0.to_string())]),
                )
            }
        };
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "hull": record.hull.0,
                "seq": record.seq,
                "operator": inv.actor().0,
                "summary": summary,
            }),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

    /// The receipt the prepare rendered, for the acting operator only.
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
        let id = if self.forfeit {
            FORFEIT_ACTION
        } else {
            SEIZE_ACTION
        };
        mush_proto::ProjectedActionResult::new(id, summary, data.clone())
            .map(Some)
            .map_err(|_| self.refusal.clone())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let offered = if self.forfeit {
            self.writer.forfeit_offered(&engine.world, query.actor())
        } else {
            self.writer.seize_offered(&engine.world, query.actor())
        };
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !offered
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
        let mut parameters = vec![integer("hull", i64::from(u32::MAX))?];
        let (id, label, help) = if self.forfeit {
            parameters.push(integer("seq", i64::MAX)?);
            (
                "roll-registry-forfeit",
                "Forfeit a seized core",
                "Take title to a core the Registry seized, once its hold has run.",
            )
        } else {
            (
                "roll-registry-seize",
                "Seize a hull's core",
                "Seize the core a hull's latest inspection found ungoverned or uncertified; it stays fitted and flies nothing.",
            )
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(id, label, help)
                .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}

/// **Register both providers** over the registered `refs` (`[seize action, seize outcome,
/// forfeit action, forfeit outcome]`), each with the economy participant it binds.
///
/// # Errors
/// A registrar that refuses a provider or the participant request.
pub(super) fn register(
    reg: &mut Registrar<'_>,
    refs: [DefinitionRef; 4],
    writer: &RegisteredInspectionWriter,
    terms: SeizureTerms,
) -> Result<SeizureHost, ModError> {
    let [seize, seized, forfeit, forfeited] = refs;
    for (is_forfeit, action, outcome) in [
        (false, seize.clone(), seized),
        (true, forfeit.clone(), forfeited),
    ] {
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        reg.action_provider(
            action.clone(),
            Provider::new(
                is_forfeit,
                (action, outcome),
                writer.clone(),
                economy,
                terms,
            )?,
        )?;
    }
    Ok(SeizureHost {
        seize,
        forfeit,
        terms,
    })
}

/// The operator's sentence for why a hull's newest inspection grounds no seizure.
fn not_seizable(why: NotSeizable) -> &'static str {
    match why {
        NotSeizable::NoInspection => words::NO_SUCH_INSPECTION,
        NotSeizable::NothingToSeize | NotSeizable::Stale | NotSeizable::CoreMoved => {
            words::NOTHING_TO_SEIZE
        }
    }
}

fn hull_of(text: &str, usage: &str) -> Result<(ObjId, Option<u64>), CmdError> {
    let target = text.trim();
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
    Ok((hull, seq))
}

/// The hull's seizures, newest inspection first.
fn seizures(ctx: &Ctx<'_>, host: &InspectionHost, hull: ObjId) -> Vec<SeizureRecord> {
    let Ok(family) = host.writer.seizures() else {
        return Vec::new();
    };
    let mut rows: Vec<SeizureRecord> = held(ctx, &family, hull);
    rows.sort_by_key(|r| std::cmp::Reverse(r.seq));
    rows
}

fn held(
    ctx: &Ctx<'_>,
    family: &StateFamilyHandle<String, SeizureRecord>,
    hull: ObjId,
) -> Vec<SeizureRecord> {
    let Some(host) = InspectionHost::of(ctx.w) else {
        return Vec::new();
    };
    let Ok(inspections) = host.writer.family() else {
        return Vec::new();
    };
    mush_roll::commerce::registry_inspection::inspection_slots(hull)
        .filter_map(|slot| ctx.action_state(&inspections, &slot).ok().flatten())
        .filter(|r| r.hull == hull)
        .filter_map(|r| {
            ctx.action_state(family, &seizure_key(hull, r.seq))
                .ok()
                .flatten()
                .cloned()
        })
        .collect()
}

/// **`@registry/seize #<hull>`**: the hull's newest inspection must ground a seizure. Every
/// refusal is one sentence; a non-operator is told the driver's one sentence before any hull
/// is read.
pub(crate) fn seize_command(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let (hull, seq) = hull_of(ctx.args.raw(), usage)?;
    if seq.is_some() {
        return Err(CmdError::msg(usage));
    }
    let actor = ctx.actor;
    let (action, key) = {
        let w: &World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::SEIZE_OFF))?;
        let seizure = host
            .seizures
            .as_ref()
            .ok_or_else(|| CmdError::msg(words::SEIZE_OFF))?;
        let (_, now) = host
            .writer
            .authorize_seize(w, actor)
            .map_err(|_| CmdError::msg(words::INSPECT_REFUSED))?;
        let inspections = host
            .writer
            .family()
            .map_err(|_| CmdError::msg(words::SEIZE_OFF))?;
        let newest = mush_roll::commerce::registry_inspection::inspection_slots(hull)
            .filter_map(|slot| {
                ctx.action_state(&inspections, &slot)
                    .ok()
                    .flatten()
                    .cloned()
            })
            .filter(|record| record.hull == hull)
            .max_by_key(|record| record.seq)
            .ok_or_else(|| CmdError::msg(not_seizable(NotSeizable::NoInspection)))?;
        proof::prove(
            &newest,
            fitted_core(w, hull),
            &now,
            seizure.terms.max_age_ic_days,
        )
        .map_err(|why| CmdError::msg(not_seizable(why)))?;
        if seizures(ctx, host, hull)
            .iter()
            .any(|r| r.seq == newest.seq)
        {
            return Err(CmdError::msg(words::ALREADY_SEIZED));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "seize",
            &format!("{}/{}", hull.0, newest.seq),
            w.world_ic_now().at,
        )?;
        (seizure.seize.clone(), key)
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({"hull": hull.0}),
        key,
    );
    ctx.out.info(
        actor,
        fill(words::SEIZING, &[("hull", &hull.0.to_string())]),
    );
    Ok(())
}

/// **`@registry/forfeit #<hull>[/<n>]`**: the hull's live seizure (or the one on its `n`th
/// inspection), once its hold has run.
pub(crate) fn forfeit_command(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let (hull, seq) = hull_of(ctx.args.raw(), usage)?;
    let actor = ctx.actor;
    let (action, key, seq) = {
        let w: &World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::SEIZE_OFF))?;
        let seizure = host
            .seizures
            .as_ref()
            .ok_or_else(|| CmdError::msg(words::SEIZE_OFF))?;
        let (_, now) = host
            .writer
            .authorize_forfeit(w, actor)
            .map_err(|_| CmdError::msg(words::INSPECT_REFUSED))?;
        // A named inspection's seizure by its own key; the bare hull, its live seizure among
        // the inspections its ring still holds.
        let record = match seq {
            Some(seq) => host.writer.seizures().ok().and_then(|family| {
                ctx.action_state(&family, &seizure_key(hull, seq))
                    .ok()
                    .flatten()
                    .cloned()
            }),
            None => seizures(ctx, host, hull)
                .into_iter()
                .find(|r| !r.forfeited()),
        }
        .ok_or_else(|| CmdError::msg(words::NO_SUCH_SEIZURE))?;
        if record.forfeited() {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        if now.at < record.forfeit_after_ic {
            return Err(CmdError::msg(words::IN_HOLD));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "forfeit",
            &format!("{}/{}", hull.0, record.seq),
            w.world_ic_now().at,
        )?;
        (seizure.forfeit.clone(), key, record.seq)
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({"hull": hull.0, "seq": seq}),
        key,
    );
    ctx.out.info(
        actor,
        fill(words::FORFEITING, &[("hull", &hull.0.to_string())]),
    );
    Ok(())
}

/// **The public findings' source** (doc 17h cut M8d D6, M8d2's producer): `minds.seized` on
/// `hull`'s inspection `seq` resolves to the seizure's IC second, read from the economy's
/// replayed seizure receipt (a world read; the action-state record is not reachable from a
/// fact). The unwarranted finding is scoped (K6) and read by no condition.
pub(crate) struct Findings;

impl mush_roll::commerce::facts::RegistryFindings for Findings {
    fn seized(&self, world: &World, hull: ObjId, seq: u64) -> Option<i64> {
        world
            .module_state::<mush_economy::EconomyStore>()?
            .core_seizure_on(hull, seq)
            .ok()
            .flatten()
            .map(|receipt| receipt.at_ic)
    }
}
