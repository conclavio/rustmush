//! The ordinary coordinator owns every durable edit and consent declaration.
use std::collections::{BTreeMap, BTreeSet};

use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, AuthorityWitnessRequest, CanonicalHash, CanonicalId, ChannelId, Ctx,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, Engine, Flags,
    IdempotencyKeyText, LegalActionQuery, ModError, ObjId, OwnerId, PerceivedRef, PrepareCtx,
    PrincipalId, ProjectedParameter, ProjectedParameterKind, ProviderOfferDraft, Registrar,
    ResolvedActionInvocation, RevisionKey, RevisionKind, RevisionSet, RevisionStamp,
    RevisionValidator, RevisionValidatorContext, RulesValue, SubjectKey, World,
};
use mush_org::unit_authority::reader::UnitAuthorityReader;
use mush_tactics::detachment::planning::{
    ConsentState, Draft, PlanId, PlannedController, PlanningBook, PlanningChange, PlanningOperation,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::families::{self, DraftRef, Families, InvitationIndex};
use super::projection::{self, ReadyStationProof};

pub const ACTION: &str = "tacsim_detachment_plan";
pub const OUTCOME: &str = "tacsim_detachment_plan_result";
const REVISION: &str = "detachment_plan_input";
const MAX_PENDING: usize = 256;
const MAX_INPUT_BYTES: usize = 16 * 1024;
const INPUT_SECONDS: i64 = 300;

/// The organizer selects the private book, never the authenticated actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub organizer: ObjId,
    pub change: PlanningOperation,
}

#[derive(Clone, Debug, Serialize)]
pub enum ReadRequest {
    Own,
    Invitations,
    Draft { organizer: ObjId, id: PlanId },
}
#[derive(Clone, Debug, Serialize)]
pub enum ReadResponse {
    Own(PlanningBook),
    Invitations(Vec<Draft>),
    Draft(Draft),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingIntent {
    at: i64,
    before_revision: u64,
    operation: Operation,
}
impl PendingIntent {
    fn current(&self, now: i64) -> bool {
        now.checked_sub(self.at)
            .is_some_and(|age| (0..=INPUT_SECONDS).contains(&age))
    }
}
#[derive(Clone, Default)]
struct PlanningPending(BTreeMap<ObjId, PendingIntent>);
fn pending(world: &World, actor: ObjId) -> Option<&PendingIntent> {
    world.module_state::<PlanningPending>()?.0.get(&actor)
}

#[derive(Clone, Debug)]
pub struct PlanningHandles {
    pub families: Families,
    pub action: DefinitionRef,
    pub reader: OwnerReadHandle<PlanningReader>,
    /// This handle is granted to Tacsim at composition, retaining the viewer's
    /// authenticated identity for advisory preparation permission reads.
    pub unit_authority_reader: Option<OwnerReadHandle<UnitAuthorityReader>>,
}
pub struct PlanningReader {
    families: Families,
}
#[derive(Clone, Debug, Serialize)]
pub enum ReaderRequest {
    Read(ReadRequest),
    Prepare(PendingIntent),
}
#[derive(Clone, Debug, Serialize)]
pub struct IndexChange {
    actor: ObjId,
    before: Option<InvitationIndex>,
    after: InvitationIndex,
}
#[derive(Clone, Debug, Serialize)]
pub enum ReaderResponse {
    Read(ReadResponse),
    Prepared {
        before_revision: u64,
        next: PlanningBook,
        affected: PlanId,
        indices: Vec<IndexChange>,
    },
}

fn error(message: &'static str) -> ActionError {
    crate::port_tick::port_error(ActionErrorCode::ProviderRejected, message)
}
fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(mod_error)?,
        DefinitionId::new(id).map_err(mod_error)?,
        1,
        payload,
    )
    .map_err(mod_error)
}
pub fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["actor","organizer","id","book_revision"]}),
    )
}
pub fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("detachment plan outcome missing"))?;
    descriptor(
        "action",
        ACTION,
        json!({"format":1,"subject":"self_actor","targets":0,"family":families::BOOK_FAMILY}),
    )
    .map(|d| d.as_current().with_references(vec![outcome]))
}
pub fn reader_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "owner_read",
        "detachment_plans",
        json!({"format":1,"visibility":"owner_or_current_invitee","actor":"authenticated"}),
    )
}

pub fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    unit_authority: bool,
) -> Result<PlanningHandles, ModError> {
    let unit_authority_reader = if unit_authority {
        let definition = mush_core::DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("org").map_err(mod_error)?,
                mush_org::unit_authority::reader::definition_descriptor()?,
            )
            .map_err(mod_error)?;
        Some(reg.request_owner_read::<UnitAuthorityReader>(definition)?)
    } else {
        None
    };
    let families = families::register(reg)?;
    let definition = reg.definition(reader_descriptor()?)?;
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([OwnerId::new(crate::NS).map_err(mod_error)?]),
        BTreeSet::new(),
        PlanningReader {
            families: families.clone(),
        },
    )?;
    let reader = reg.request_owner_read::<PlanningReader>(definition)?;
    let action = actions
        .get(ACTION)
        .cloned()
        .ok_or_else(|| ModError::msg("detachment plan action missing"))?;
    let outcome = outcomes
        .get(OUTCOME)
        .cloned()
        .ok_or_else(|| ModError::msg("detachment plan outcome missing"))?;
    reg.action_provider(
        action.clone(),
        Provider {
            action: action.clone(),
            outcome,
            families: families.clone(),
            reader: reader.clone(),
        },
    )?;
    reg.revision_validator(
        RevisionKind::new(CanonicalId::new(REVISION).map_err(mod_error)?),
        Validator {
            families: families.clone(),
        },
    )?;
    reg.state(PlanningPending::default());
    let handles = PlanningHandles {
        families,
        action,
        reader,
        unit_authority_reader,
    };
    reg.state(handles.clone());
    Ok(handles)
}

fn book_or_new(book: Option<PlanningBook>, owner: ObjId) -> Result<PlanningBook, String> {
    let book = match book {
        Some(book) => book,
        None => PlanningBook::new(owner).map_err(|e| e.to_string())?,
    };
    book.validate().map_err(|e| e.to_string())?;
    if book.owner != owner {
        return Err("detachment plan owner mismatch".into());
    }
    Ok(book)
}
fn live_actor(world: &World, actor: ObjId) -> bool {
    world
        .get(actor)
        .is_some_and(|o| o.kind.is_actor() && !o.flags.contains(Flags::GOING))
}
fn ready_draft<'a>(book: &'a PlanningBook, operation: &PlanningOperation) -> Option<&'a Draft> {
    match operation {
        PlanningOperation::Declare {
            id,
            state: ConsentState::Ready,
            ..
        } => book.drafts.get(id),
        _ => None,
    }
}
fn evaluate(
    world: &World,
    book: &PlanningBook,
    actor: ObjId,
    input: &PendingIntent,
) -> Result<(PlanningChange, Option<ReadyStationProof>), String> {
    if book.owner != input.operation.organizer
        || book.revision != input.before_revision
        || !input.current(world.now())
        || !live_actor(world, actor)
        || !live_actor(world, book.owner)
        || world
            .module_state::<crate::TacsimStore>()
            .is_none_or(|store| !store.enabled())
    {
        return Err("detachment plan input changed".into());
    }
    if matches!(&input.operation.change, PlanningOperation::Declare { .. })
        && world
            .get(actor)
            .is_none_or(|actor| actor.kind != mush_core::ObjKind::Player)
    {
        return Err("detachment consent requires an actual player character".into());
    }
    let ready = if let Some(draft) = ready_draft(book, &input.operation.change) {
        let proof = projection::ready_station(world, actor, draft)
            .ok_or("ready requires the current physical helm station")?;
        if !projection::physically_aboard(world, actor, draft.plan.carrier)
            || !projection::physically_aboard(world, proof.unit(), draft.plan.carrier)
        {
            return Err("ready assignment must be aboard its carrier".into());
        }
        Some(proof)
    } else {
        None
    };
    let change = book
        .apply(actor, &input.operation.change)
        .map_err(|e| e.to_string())?;
    Ok((change, ready))
}
fn players(draft: Option<&Draft>) -> BTreeSet<ObjId> {
    draft
        .into_iter()
        .flat_map(|d| {
            d.plan.slots.iter().filter_map(move |slot| {
                let PlannedController::Player { character } = slot.controller else {
                    return None;
                };
                // A decline frees this player's bounded inbox. The assignment
                // remains readable by its exact reference; accepting restores it.
                // A material edit clears declarations and may invite them again.
                let declined = d.consents.get(&character).is_some_and(|consent| {
                    consent.revision == d.revision
                        && consent.plan_hash == d.plan_hash
                        && consent.state == ConsentState::Declined
                });
                (!declined).then_some(character)
            })
        })
        .collect()
}
fn invitation_changes(
    before: &PlanningBook,
    next: &PlanningBook,
    affected: PlanId,
    mut load: impl FnMut(ObjId) -> Result<Option<InvitationIndex>, OwnerReadError>,
) -> Result<Vec<IndexChange>, OwnerReadError> {
    let old = players(before.drafts.get(&affected));
    let new = players(next.drafts.get(&affected));
    let reference = DraftRef {
        organizer: before.owner,
        id: affected,
    };
    let mut changes = Vec::new();
    for actor in old.union(&new).copied() {
        if actor == before.owner || old.contains(&actor) == new.contains(&actor) {
            continue;
        }
        let prior = load(actor)?;
        let mut after = prior.clone().unwrap_or(InvitationIndex {
            revision: 1,
            drafts: BTreeSet::new(),
        });
        after.validate().map_err(|_| OwnerReadError::Integrity)?;
        if old.contains(&actor) {
            if !after.drafts.remove(&reference) {
                return Err(OwnerReadError::Integrity);
            }
        } else if !after.drafts.insert(reference) {
            return Err(OwnerReadError::Integrity);
        }
        if prior.is_some() {
            after.revision = after.revision.checked_add(1).ok_or(OwnerReadError::Limit)?;
        }
        after.validate().map_err(|_| OwnerReadError::Limit)?;
        changes.push(IndexChange {
            actor,
            before: prior,
            after,
        });
    }
    Ok(changes)
}

impl OwnerReadService for PlanningReader {
    type State = crate::TacsimStore;
    type Request = ReaderRequest;
    type Output = ReaderResponse;
    fn read(
        &self,
        store: &Self::State,
        ctx: &OwnerReadContext<'_>,
        request: Self::Request,
    ) -> Result<Self::Output, OwnerReadError> {
        if !store.enabled() {
            return Err(OwnerReadError::Unavailable);
        }
        let load = |owner| {
            book_or_new(
                ctx.action_state(&self.families.books, &owner)?.cloned(),
                owner,
            )
            .map_err(|_| OwnerReadError::Integrity)
        };
        let answer = match request {
            ReaderRequest::Read(ReadRequest::Own) => ReadResponse::Own(load(ctx.actor())?),
            ReaderRequest::Read(ReadRequest::Draft { organizer, id }) => {
                let book = load(organizer)?;
                let draft = book
                    .drafts
                    .get(&id)
                    .filter(|d| d.visible_to(ctx.actor()))
                    .cloned()
                    .ok_or(OwnerReadError::Refused)?;
                ReadResponse::Draft(draft)
            }
            ReaderRequest::Read(ReadRequest::Invitations) => {
                let Some(index) = ctx.action_state(&self.families.invitations, &ctx.actor())?
                else {
                    return Ok(ReaderResponse::Read(ReadResponse::Invitations(Vec::new())));
                };
                index.validate().map_err(|_| OwnerReadError::Integrity)?;
                let owners: Vec<_> = index
                    .drafts
                    .iter()
                    .map(|r| r.organizer)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if owners.is_empty() {
                    return Ok(ReaderResponse::Read(ReadResponse::Invitations(Vec::new())));
                }
                let books = ctx.action_states(&self.families.books, &owners)?;
                let mut drafts = Vec::new();
                for reference in &index.drafts {
                    let position = owners
                        .binary_search(&reference.organizer)
                        .map_err(|_| OwnerReadError::Integrity)?;
                    let book = books
                        .get(position)
                        .copied()
                        .flatten()
                        .ok_or(OwnerReadError::Integrity)?;
                    book.validate().map_err(|_| OwnerReadError::Integrity)?;
                    if book.owner != reference.organizer {
                        return Err(OwnerReadError::Integrity);
                    }
                    let draft = book
                        .drafts
                        .get(&reference.id)
                        .filter(|d| players(Some(d)).contains(&ctx.actor()))
                        .ok_or(OwnerReadError::Integrity)?;
                    drafts.push(draft.clone());
                }
                ReadResponse::Invitations(drafts)
            }
            ReaderRequest::Prepare(input) => {
                if pending(ctx.world(), ctx.actor()) != Some(&input) {
                    return Err(OwnerReadError::Authority);
                }
                let before = load(input.operation.organizer)?;
                let (change, _) = evaluate(ctx.world(), &before, ctx.actor(), &input)
                    .map_err(|_| OwnerReadError::Refused)?;
                let indices =
                    invitation_changes(&before, &change.book, change.affected, |actor| {
                        ctx.action_state(&self.families.invitations, &actor)
                            .map(|i| i.cloned())
                    })?;
                return Ok(ReaderResponse::Prepared {
                    before_revision: before.revision,
                    next: change.book,
                    affected: change.affected,
                    indices,
                });
            }
        };
        Ok(ReaderResponse::Read(answer))
    }
}

pub fn read(ctx: &Ctx<'_>, request: ReadRequest) -> Result<ReadResponse, String> {
    let handles = ctx
        .w
        .module_state::<PlanningHandles>()
        .ok_or("Detachment planning unavailable.")?;
    match ctx
        .read_owner(&handles.reader, ReaderRequest::Read(request))
        .map_err(|_| "No visible detachment plan.")?
    {
        ReaderResponse::Read(result) => Ok(result),
        _ => Err("Detachment planning unavailable.".into()),
    }
}
fn fingerprint(actor: ObjId, input: &PendingIntent) -> Result<CanonicalHash, ActionError> {
    CanonicalHash::for_canonical_value(
        "tacsim.detachment_input.v1",
        1,
        &json!({"actor":actor,"input":input}),
        MAX_INPUT_BYTES * 2,
    )
    .map_err(|_| error("detachment input invalid"))
}
fn key(actor: ObjId) -> Result<RevisionKey, ActionError> {
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(|_| error("detachment owner invalid"))?,
        kind: RevisionKind::new(
            CanonicalId::new(REVISION).map_err(|_| error("detachment revision invalid"))?,
        ),
        subject: SubjectKey::Object { id: actor },
        facet: None,
    })
}
fn generation(world: &World, book: &PlanningBook, actor: ObjId) -> Result<u64, ActionError> {
    let input = pending(world, actor);
    let evaluated = input.and_then(|i| evaluate(world, book, actor, i).ok());
    let hash = CanonicalHash::for_canonical_value(
        "tacsim.detachment_revision.v1",
        1,
        &json!({"input":input,"book_revision":book.revision,"valid":evaluated.is_some(),
            "ready_station":evaluated.as_ref().and_then(|(_,proof)| proof.as_ref())}),
        MAX_INPUT_BYTES * 2,
    )
    .map_err(|_| error("detachment revision invalid"))?;
    let bytes: [u8; 8] = hash
        .as_bytes()
        .get(..8)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| error("detachment revision invalid"))?;
    Ok(u64::from_be_bytes(bytes))
}
struct Validator {
    families: Families,
}
impl RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &RevisionValidatorContext<'_>,
        coordinate: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        let SubjectKey::Object { id: actor } = coordinate.subject else {
            return Ok(None);
        };
        if coordinate != &key(actor)? {
            return Ok(None);
        }
        let Some(input) = pending(ctx.world(), actor) else {
            return Ok(None);
        };
        let owner = input.operation.organizer;
        let book = book_or_new(
            ctx.action_state(&self.families.books, &owner)
                .map_err(|_| error("detachment book unavailable"))?
                .cloned(),
            owner,
        )
        .map_err(|_| error("detachment book invalid"))?;
        generation(ctx.world(), &book, actor).map(Some)
    }
}

struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    families: Families,
    reader: OwnerReadHandle<PlanningReader>,
}
impl Provider {
    fn input(&self, i: &ActionInvocation) -> Result<PendingIntent, ActionError> {
        if i.subject() != &PerceivedRef::SelfActor
            || !i.targets().is_empty()
            || i.principal() != &(PrincipalId::Object { id: i.actor() })
            || i.parameters().as_object().is_none_or(|p| p.len() != 2)
        {
            return Err(error("detachment input refused"));
        }
        let value = i
            .parameters()
            .get("input")
            .ok_or_else(|| error("detachment input missing"))?;
        if serde_json::to_vec(value)
            .map_err(|_| error("detachment input invalid"))?
            .len()
            > MAX_INPUT_BYTES
        {
            return Err(error("detachment input too large"));
        }
        let input: PendingIntent =
            serde_json::from_value(value.clone()).map_err(|_| error("detachment input invalid"))?;
        if i.parameters().get("request").and_then(Value::as_str)
            != Some(fingerprint(i.actor(), &input)?.to_hex().as_str())
        {
            return Err(error("detachment input changed"));
        }
        Ok(input)
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, i: &ActionInvocation) -> Result<(), ActionError> {
        self.input(i).map(|_| ())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.projected_subject() != &PerceivedRef::SelfActor
            || query.truth_subject() != &(SubjectKey::Object { id: query.actor() })
        {
            return Ok(None);
        }
        let Some(input) = pending(&engine.world, query.actor()) else {
            return Ok(None);
        };
        let owner = input.operation.organizer;
        let book = book_or_new(
            engine
                .action_state()
                .get(&self.families.books, &owner)
                .map_err(|_| error("detachment book unavailable"))?
                .cloned(),
            owner,
        )
        .map_err(|_| error("detachment book invalid"))?;
        if evaluate(&engine.world, &book, query.actor(), input).is_err() {
            return Ok(None);
        }
        let revisions = RevisionSet::from_stamps([RevisionStamp {
            key: key(query.actor())?,
            generation: generation(&engine.world, &book, query.actor())?,
        }])
        .map_err(|_| error("detachment revision invalid"))?;
        crate::surface::act::offer_draft_with_revisions(
            action,
            engine,
            json!({"input":input}),
            vec![ProjectedParameter {
                id: CanonicalId::new("request")
                    .map_err(|_| error("detachment parameter invalid"))?,
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 64 },
            }],
            (
                "detachment-plan",
                "Record detachment plan",
                "Commit a private plan edit or player declaration.",
            ),
            revisions,
        )
        .map(Some)
    }
    fn resolved_authority_witnesses(
        &self,
        resolved: &ResolvedActionInvocation,
        engine: &Engine,
    ) -> Result<Vec<AuthorityWitnessRequest>, ActionError> {
        let i = resolved.invocation();
        let input = self.input(i)?;
        if !matches!(
            &input.operation.change,
            PlanningOperation::Declare {
                state: ConsentState::Ready,
                ..
            }
        ) {
            return Ok(Vec::new());
        }
        let owner = input.operation.organizer;
        let book = book_or_new(
            engine
                .action_state()
                .get(&self.families.books, &owner)
                .map_err(|_| error("detachment book unavailable"))?
                .cloned(),
            owner,
        )
        .map_err(|_| error("detachment book invalid"))?;
        let (_, proof) = evaluate(&engine.world, &book, i.actor(), &input)
            .map_err(|_| error("ready station changed"))?;
        let proof = proof.ok_or_else(|| error("ready station missing"))?;
        Ok(vec![AuthorityWitnessRequest::ControlLease {
            provider: OwnerId::new("sim").map_err(|_| error("control owner invalid"))?,
            subject: proof.unit(),
            channel: ChannelId::new("helm").ok_or_else(|| error("control channel invalid"))?,
            holder: PrincipalId::Object { id: i.actor() },
        }])
    }
    fn prepare_authoritative_with_context(
        &self,
        i: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let input = self.input(i)?;
        let reply = context
            .read_owner_preimage(txn, &self.reader, ReaderRequest::Prepare(input))
            .map_err(|_| error("detachment edit refused"))?;
        let ReaderResponse::Prepared {
            before_revision,
            next,
            affected,
            indices,
        } = reply
        else {
            return Err(error("detachment proposal missing"));
        };
        let owner = next.owner;
        let prior = txn
            .get(&self.families.books, &owner)
            .map_err(|_| error("detachment book unavailable"))?
            .cloned();
        let before =
            book_or_new(prior.clone(), owner).map_err(|_| error("detachment book invalid"))?;
        if before.revision != before_revision {
            return Err(error("detachment book changed"));
        }
        for index in &indices {
            if txn
                .get(&self.families.invitations, &index.actor)
                .map_err(|_| error("invitation index unavailable"))?
                != index.before.as_ref()
            {
                return Err(error("invitations changed"));
            }
        }
        let revision = next.revision;
        if prior.is_some() {
            txn.replace(&self.families.books, &owner, next)
        } else {
            txn.insert(&self.families.books, &owner, next)
        }
        .map_err(|_| error("detachment book write refused"))?;
        for index in indices {
            if index.before.is_some() {
                txn.replace(&self.families.invitations, &index.actor, index.after)
            } else {
                txn.insert(&self.families.invitations, &index.actor, index.after)
            }
            .map_err(|_| error("invitation write refused"))?;
        }
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor":i.actor(),"organizer":owner,"id":affected,"book_revision":revision}),
        )
        .map_err(|_| error("detachment outcome invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![i.principal().clone()]))
    }
}

pub fn submit(ctx: &mut Ctx<'_>, operation: Operation) -> Result<(), String> {
    if ctx.authoritative_pending(ctx.actor)
        || ctx.authoritative_commit_pending()
        || ctx.authoritative_queue_full()
    {
        return Err("An authoritative request is already pending.".into());
    }
    let handles = ctx
        .w
        .module_state::<PlanningHandles>()
        .cloned()
        .ok_or("Detachment planning unavailable.")?;
    let owner = operation.organizer;
    let book = book_or_new(
        ctx.action_state(&handles.families.books, &owner)
            .map_err(|_| "Detachment planning unavailable.")?
            .cloned(),
        owner,
    )?;
    let input = PendingIntent {
        at: ctx.w.now(),
        before_revision: book.revision,
        operation,
    };
    if serde_json::to_vec(&input)
        .map_err(|_| "Invalid detachment input.")?
        .len()
        > MAX_INPUT_BYTES
    {
        return Err("Detachment input too large.".into());
    }
    evaluate(ctx.w, &book, ctx.actor, &input)?;
    let now = ctx.w.now();
    let queue = ctx
        .w
        .module_state_mut::<PlanningPending>()
        .ok_or("Detachment planning unavailable.")?;
    queue.0.retain(|_, input| input.current(now));
    if queue.0.len() >= MAX_PENDING && !queue.0.contains_key(&ctx.actor) {
        return Err("Detachment requests busy.".into());
    }
    queue.0.insert(ctx.actor, input.clone());
    if ctx
        .read_owner(&handles.reader, ReaderRequest::Prepare(input.clone()))
        .is_err()
    {
        if let Some(queue) = ctx.w.module_state_mut::<PlanningPending>()
            && queue.0.get(&ctx.actor) == Some(&input)
        {
            queue.0.remove(&ctx.actor);
        }
        return Err("Detachment edit refused.".into());
    }
    let digest = fingerprint(ctx.actor, &input)
        .map_err(|_| "Invalid detachment input.")?
        .to_hex();
    let key = IdempotencyKeyText::parse_canonical(format!("detachment/{digest}"))
        .map_err(|_| "Invalid detachment request.")?;
    ctx.submit_authoritative(
        handles.action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"request":digest}),
        key,
    );
    Ok(())
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
