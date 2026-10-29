//! **`@registry/revoke principal=<who> <inspection>`: the Registry revokes a principal for a
//! term** (doc 17h cut M8d D4; Q-M8-3's spelling). A Registry operator holding the existing
//! `roll.registry.revoke` (its own observation, `RegisteredInspectionWriter::authorize_revoke`)
//! bars a character, or an organisation (M8c) as `@org` names it, from every new licence for
//! `[tacsim.minds].revocation_ic_days` IC days, on an inspection that read an ungoverned core.
//!
//! **Whom** (fix round K4, the lead's ruling): only the hull's holder the inspection recorded
//! (`InspectionRecord::holder`) — a character, or the organisation that held it. Any other
//! principal is the uniform refusal.
//!
//! The action writes one `mush_roll::commerce::registry_inspection::RevokedPrincipal`; the Roll
//! then bars the principal from new licences, from using held ones and from moving any (the
//! lead's option (c) and its three conditions; `principal` in mush-roll says where). Held terms
//! are not revoked: a command holds one authoritative submission at a time and a tick submits
//! nothing, so the per-licence fan-out cannot be issued; they resume when the window ends.
//!
//! **The notice** (K2): the principal is told once, through this slot's notice tick, when the
//! record lands — "The Registry has barred your mind licences until `<IC date>`." — naming no hull,
//! mind or inspection. An organisation's notice goes to each of its granted members (those
//! `org_holder::acts_for` admits) as they are connected, once each, until the window ends. A
//! session notice, like the revocation notices: a restart before delivery loses it.

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, Flags, ModError, ObjId, ObjKind,
    PerceivedRef, PrepareCtx, Registrar, World, WorldEvent, WorldEventKind, WorldObserver,
};
use mush_roll::commerce::registry_inspection::RevokedPrincipal;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use super::{InspectionHost, InspectionRecord, unwarranted};
use crate::minds::market_commands::words::{self, fill};

/// The principal revocation action's id.
pub const PRINCIPAL_ACTION: &str = "registry_principal_revocation";
/// Its outcome's id.
pub const PRINCIPAL_OUTCOME: &str = "registry_principal_revoked";

/// What the slot holds when the principal revocation composes.
#[derive(Clone)]
pub struct PrincipalHost {
    pub(crate) action: DefinitionRef,
    /// `[tacsim.minds].revocation_ic_days`, captured at composition.
    pub(crate) days: u32,
    /// K2: the records announced and the recipients each has reached (RAM).
    pub(crate) notices: Notices,
}

/// The notice queue (K2): per record key, the recipients told so far.
#[derive(Clone, Default)]
pub(crate) struct Notices(Arc<Mutex<BTreeMap<String, BTreeSet<ObjId>>>>);

/// At most this many records are waiting to be told.
const MAX_NOTICES: usize = 4096;

impl Notices {
    fn note(&self, key: String) {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if map.len() < MAX_NOTICES || map.contains_key(&key) {
            map.insert(key, BTreeSet::new());
        }
    }
}

/// The `ActionStateChanged` observer: a landed revocation is queued for its notice (RAM).
struct Announce {
    family: mush_core::StateFamilyHandle<String, RevokedPrincipal>,
    notices: Notices,
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
            self.notices.note(key.to_string());
        }
    }
}

/// Register the notice observer and tick.
pub(super) fn register_notices(
    reg: &mut Registrar<'_>,
    writer: &mush_roll::commerce::registry_inspection::RegisteredInspectionWriter,
    notices: &Notices,
) -> Result<(), ModError> {
    reg.subscribe(
        WorldEventKind::ActionStateChanged,
        Arc::new(Announce {
            family: writer.revoked_principals()?,
            notices: notices.clone(),
        }),
    );
    reg.tick(
        "registry.principal_notices",
        mush_core::TickCadence::Coarse(std::time::Duration::from_secs(5)),
        Arc::new(notice_tick),
    )?;
    Ok(())
}

/// **The notice tick** (K2): each queued record tells its principal once — a connected
/// character, or each connected member an organisation's grant admits, once each — and is
/// forgotten when the window ends or a character has been told.
fn notice_tick(ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>) {
    let Some(host) = InspectionHost::of(ctx.w) else {
        return;
    };
    let Some(principals) = host.principals.clone() else {
        return;
    };
    let Ok(family) = host.writer.revoked_principals() else {
        return;
    };
    let now = ctx.w.world_ic_now().at;
    let queued: Vec<String> = principals
        .notices
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    let mut sends: Vec<(ObjId, String)> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut told: Vec<(String, ObjId)> = Vec::new();
    for key in queued {
        let record = match ctx.action_state(&family, &key) {
            Ok(Some(record)) => record.clone(),
            Ok(None) => {
                done.push(key);
                continue;
            }
            Err(_) => continue,
        };
        if !record.bars(now) {
            done.push(key);
            continue;
        }
        let text = fill(
            words::PRINCIPALS_BARRED,
            &[("until", &crate::port::licence::ic_stamp(record.until_ic))],
        );
        let already = principals
            .notices
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned()
            .unwrap_or_default();
        match record.principal {
            mush_org::PrincipalId::Object(c) => {
                if ctx.w.is_connected(c) {
                    sends.push((c, text));
                    done.push(key);
                }
            }
            mush_org::PrincipalId::Organization(org) => {
                for member in ctx.w.connected().collect::<Vec<_>>() {
                    if !already.contains(&member)
                        && mush_roll::commerce::org_holder::acts_for(ctx.w, org, member).is_ok()
                    {
                        sends.push((member, text.clone()));
                        told.push((key.clone(), member));
                    }
                }
            }
            mush_org::PrincipalId::System(_) => done.push(key),
        }
    }
    {
        let mut map = principals
            .notices
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (key, member) in told {
            if let Some(set) = map.get_mut(&key) {
                set.insert(member);
            }
        }
        for key in done {
            map.remove(&key);
        }
    }
    for (actor, text) in sends {
        ctx.out.sys_to_actor(actor, crate::NS, text);
    }
}

/// Who is revoked. On the action's wire it is the record's key text (`{character:020}` or
/// `org/{org:020}`), a text parameter the offer declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Principal {
    Character(u32),
    Org(u64),
}

impl Principal {
    /// The wire text: the holder indexes' key.
    fn wire(self) -> String {
        match self {
            Self::Character(c) => format!("{c:020}"),
            Self::Org(org) => format!("org/{org:020}"),
        }
    }
    /// The wire text read back; exactly the canonical spelling.
    fn parse(text: &str) -> Option<Self> {
        let parsed = match text.strip_prefix("org/") {
            Some(org) => Self::Org(org.parse().ok()?),
            None => Self::Character(text.parse().ok().filter(|c| *c != 0)?),
        };
        (parsed.wire() == text).then_some(parsed)
    }
    fn principal(self) -> mush_org::PrincipalId {
        match self {
            Self::Character(c) => mush_org::PrincipalId::Object(ObjId(c)),
            Self::Org(org) => mush_org::PrincipalId::Organization(mush_org::OrgId(org)),
        }
    }
    /// The principal's name: a live character's, or an active organisation's.
    fn name(self, w: &World) -> Option<String> {
        match self {
            Self::Character(c) => w
                .get(ObjId(c))
                .filter(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
                .map(|o| o.name.to_string()),
            Self::Org(org) => w
                .module_state::<mush_org::OrgStore>()?
                .state()
                .org(mush_org::OrgId(org))
                .filter(|o| o.status == mush_org::OrgStatus::Active)
                .map(|o| o.name.to_string()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    principal: String,
    hull: u32,
    seq: u64,
}

pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    writer: mush_roll::commerce::registry_inspection::RegisteredInspectionWriter,
    days: u32,
    refusal: ActionError,
}

impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        writer: mush_roll::commerce::registry_inspection::RegisteredInspectionWriter,
        days: u32,
    ) -> Result<Self, ModError> {
        Ok(Self {
            action,
            outcome,
            writer,
            days,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "The Registry revocation is unavailable or stale.",
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }

    fn request(&self, inv: &ActionInvocation) -> Result<(Principal, ObjId, u64), ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        let principal = Principal::parse(&p.principal).ok_or_else(|| self.refusal.clone())?;
        if p.hull == 0 || p.seq == 0 {
            return Err(self.refusal.clone());
        }
        Ok((principal, ObjId(p.hull), p.seq))
    }

    fn definitions(&self) -> Result<Vec<DefinitionRef>, ActionError> {
        let authority = self.writer.authority().map_err(|_| self.refusal.clone())?;
        Ok(vec![self.action.clone(), self.outcome.clone(), authority])
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
        let fail = || self.refusal.clone();
        let (principal, hull, seq) = self.request(inv)?;
        let (name, at) = {
            let world = ctx.world().map_err(|_| fail())?;
            let (_, at) = self
                .writer
                .authorize_revoke(world, inv.actor())
                .map_err(|_| fail())?;
            (principal.name(world).ok_or_else(fail)?, at)
        };
        let inspections = self.writer.family().map_err(|_| fail())?;
        let key = mush_roll::commerce::registry_inspection::inspection_key(hull, seq);
        let record: InspectionRecord = txn
            .get(&inspections, &key)
            .map_err(|_| fail())?
            .cloned()
            .ok_or_else(fail)?;
        // K4: only the holder this inspection recorded.
        if record.hull != hull
            || record.seq != seq
            || !unwarranted::unwarranted(&record)
            || super::fine::fined_principal(&record) != Some(principal.principal())
        {
            return Err(fail());
        }
        let until = at
            .at
            .checked_add(i64::from(self.days).saturating_mul(86_400))
            .ok_or_else(fail)?;
        let revoked = self
            .writer
            .revoke_principal(
                ctx,
                txn,
                RevokedPrincipal {
                    format: 1,
                    principal: principal.principal(),
                    from_ic: at.at,
                    until_ic: until,
                    inspection: record.inspection,
                    hull,
                    seq,
                    operation: ctx.commit_token(),
                    operator: inv.actor(),
                },
            )
            .map_err(|_| fail())?;
        let summary = fill(
            words::PRINCIPAL_REVOKED,
            &[
                ("who", &name),
                ("until", &crate::port::licence::ic_stamp(revoked.until_ic)),
            ],
        );
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({
                "principal": principal.wire(),
                "until_ic": revoked.until_ic,
                "operator": inv.actor().0,
                "summary": summary,
            }),
        )
        .map_err(|_| fail())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }

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
        mush_proto::ProjectedActionResult::new(PRINCIPAL_ACTION, summary, data.clone())
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
            || !self.writer.revoke_offered(&engine.world, query.actor())
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
                mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new("principal")
                        .map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes: 24 },
                },
                integer("hull", i64::from(u32::MAX))?,
                integer("seq", i64::MAX)?,
            ])
            .map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "roll-registry-principal-revocation",
                "Revoke a principal for a term",
                "Bar a character or organisation from new licences after an inspection found an ungoverned core.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}

/// `who` as a principal: a live character (`#<n>`, `*name` or a name, as `page` resolves one)
/// or an organisation as `@org` names it. A name that is both is refused, so the operator says
/// which.
fn resolve(ctx: &Ctx<'_>, who: &str) -> Result<Principal, CmdError> {
    let query = who.trim();
    let character = if let Some(number) = query.strip_prefix('#') {
        number.parse::<u32>().ok().map(ObjId)
    } else {
        ctx.w
            .resolve_player(query.strip_prefix('*').unwrap_or(query))
    }
    .filter(|id| {
        ctx.w
            .get(*id)
            .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
    });
    let org = if query.starts_with('#') {
        None
    } else {
        mush_economy::resolve_org_visible(ctx, query)
    };
    match (character, org) {
        (Some(_), Some(_)) => Err(CmdError::msg(words::AMBIGUOUS_PRINCIPAL)),
        (Some(c), None) => Ok(Principal::Character(c.0)),
        (None, Some(org)) => Ok(Principal::Org(org.0)),
        (None, None) => Err(CmdError::msg(words::NO_SUCH_PRINCIPAL)),
    }
}

/// **The command body** (`@registry/revoke principal=<who> #<hull>[/<n>]`), dispatched by
/// `@registry/revoke` when its argument starts `principal=`.
pub(crate) fn command(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let rest = raw
        .strip_prefix("principal=")
        .ok_or_else(|| CmdError::msg(usage))?;
    let (who, target) = rest.rsplit_once(' ').ok_or_else(|| CmdError::msg(usage))?;
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
    let (action, key, principal, seq, name, days) = {
        let w: &World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::REGISTRY_OFF))?;
        let principals = host
            .principals
            .as_ref()
            .ok_or_else(|| CmdError::msg(words::REGISTRY_OFF))?;
        if !host.writer.revoke_offered(w, actor) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        let principal = resolve(ctx, who)?;
        let name = principal
            .name(w)
            .ok_or_else(|| CmdError::msg(words::NO_SUCH_PRINCIPAL))?;
        let inspections = host
            .writer
            .family()
            .map_err(|_| CmdError::msg(words::REGISTRY_OFF))?;
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
            return Err(CmdError::msg(words::NOTHING_TO_REVOKE));
        }
        // K4: only the hull's holder the inspection recorded; anyone else is the uniform
        // refusal.
        if super::fine::fined_principal(&record) != Some(principal.principal()) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "revoke_principal",
            &format!("{}/{}", hull.0, record.seq),
            w.world_ic_now().at,
        )?;
        let days = principals.days;
        (
            principals.action.clone(),
            key,
            principal,
            record.seq,
            name,
            days,
        )
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({"principal": principal.wire(), "hull": hull.0, "seq": seq}),
        key,
    );
    ctx.out.info(
        actor,
        fill(
            words::REVOKING_PRINCIPAL,
            &[("who", &name), ("days", &days.to_string())],
        ),
    );
    Ok(())
}
