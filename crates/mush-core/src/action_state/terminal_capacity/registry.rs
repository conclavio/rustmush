//! Closed coordinator protocol for the registered Roll rating producer.
//! Business certification and actual paid execution remain the producer's job;
//! this module owns the exact future-key promise and its reverse completeness.

use super::*;
use crate::ObjId;
use crate::action::{ActionInvocation, ActionProviderEntry, ActionProviderRegistry, PrincipalId};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistry,
    DefinitionRegistryBuilder,
};

const ACTIONS: [&str; 5] = [
    "rating_admit",
    "rating_grant",
    "rating_settle",
    "rating_cancel",
    "rating_recover",
];
mod reconformance;
use reconformance::{Binding, Profile};
const TRANSITIONS: [&str; 5] = ["admit", "grant", "settle", "cancel", "recover"];
const FAMILIES: [(&str, usize); 7] = [
    ("rating_jobs", 4096),
    ("rating_executions", 4096),
    ("rating_receipts", 65536),
    ("rating_nonconformance", 65536),
    ("rating_completions", 4096),
    ("rating_status", 4096),
    ("rating_pages", 32768),
];

// Exact closed v1 reconstruction, shared by current declaration validation and
// historical cleanup. The saved compatibility is the actual combat manifest
// digest. Reconstructing data never grants execution or certification authority.
fn closed_participants() -> Result<Vec<(OwnerId, DefinitionDescriptor)>, ActionStateError> {
    [
        (
            "roll",
            "commercial_roots",
            json!({"format":1,"protocol":"commercial_retention_v1"}),
        ),
        (
            "tacsim",
            "registry_quota",
            json!({"format":1,"protocol":"registry_rating_v1","slot":"quota"}),
        ),
        (
            "tacsim",
            "registry_artifacts",
            json!({"format":1,"protocol":"registry_rating_v1","slot":"artifacts"}),
        ),
    ]
    .into_iter()
    .map(|(owner, id, payload)| {
        Ok((
            OwnerId::new(owner).map_err(|_| refused("Registry closed owner"))?,
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")
                    .map_err(|_| refused("Registry closed kind"))?,
                DefinitionId::new(id).map_err(|_| refused("Registry closed id"))?,
                1,
                payload,
            )
            .map_err(|_| refused("Registry closed participant"))?,
        ))
    })
    .collect()
}
#[cfg(test)]
fn closed_actions(manifest: &DefinitionRef) -> Result<[DefinitionDescriptor; 5], ActionStateError> {
    closed_profile_actions(manifest, Profile::Ordinary)
}
fn closed_profile_actions(
    manifest: &DefinitionRef,
    profile: Profile,
) -> Result<[DefinitionDescriptor; 5], ActionStateError> {
    let mut refs = vec![manifest.clone()];
    for (owner, descriptor) in closed_participants()? {
        refs.push(
            DefinitionRegistryBuilder::new()
                .register(owner, descriptor)
                .map_err(|_| refused("Registry closed participant reference"))?,
        );
    }
    profile
        .ids()
        .into_iter()
        .zip(TRANSITIONS)
        .map(|(id, transition)| {
            DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(|_| refused("Registry closed kind"))?,
                DefinitionId::new(id).map_err(|_| refused("Registry closed id"))?,
                1,
                json!({"format":1,"protocol":profile.word(),"transition":transition}),
            )
            .map(|row| row.with_references(refs.clone()))
            .map_err(|_| refused("Registry closed action"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| refused("Registry closed action count"))
}
fn closed_references(manifest: &DefinitionRef) -> Result<[DefinitionRef; 5], ActionStateError> {
    closed_profile_references(manifest, Profile::Ordinary)
}
fn closed_profile_references(
    manifest: &DefinitionRef,
    profile: Profile,
) -> Result<[DefinitionRef; 5], ActionStateError> {
    closed_profile_actions(manifest, profile)?
        .into_iter()
        .map(|descriptor| {
            DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new("roll").map_err(|_| refused("Registry closed owner"))?,
                    descriptor,
                )
                .map_err(|_| refused("Registry closed action reference"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| refused("Registry closed reference count"))
}

/// Declaration can enter the registry only through the owning Roll registrar.
/// No caller supplies target keys, sizes, callbacks or wire authority.
#[derive(Clone, Debug, Serialize)]
pub(in crate::action_state) struct Protocol {
    manifest: DefinitionRef,
    actions: [DefinitionRef; 5],
    #[serde(skip_serializing_if = "Option::is_none")]
    reconformance: Option<[DefinitionRef; 5]>,
}

impl Protocol {
    pub(in crate::action_state) fn new(
        owner: &OwnerId,
        manifest: DefinitionRef,
        actions: [DefinitionRef; 5],
    ) -> Result<Self, ActionStateError> {
        if owner.as_str() != "roll"
            || manifest.key().owner.as_str() != "tacsim"
            || manifest.key().kind.as_str() != "registry_manifest"
            || manifest.key().id.as_str() != "combat"
            || manifest.version() != 1
        {
            return Err(refused("Registry capacity declaration owner or manifest"));
        }
        for (action, id) in actions.iter().zip(ACTIONS) {
            if action.key().owner != *owner
                || action.key().kind.as_str() != "action"
                || action.key().id.as_str() != id
                || action.version() != 1
            {
                return Err(refused("Registry capacity action identity or order"));
            }
        }
        if actions != closed_references(&manifest)? {
            return Err(refused(
                "Registry current declaration is not the closed v1 recipe",
            ));
        }
        Ok(Self {
            manifest,
            actions,
            reconformance: None,
        })
    }

    pub(in crate::action_state) fn with_reconformance(
        &mut self,
        owner: &OwnerId,
        actions: [DefinitionRef; 5],
    ) -> Result<(), ActionStateError> {
        if owner.as_str() != "roll"
            || self.reconformance.is_some()
            || actions != closed_profile_references(&self.manifest, Profile::Reconformance)?
        {
            return Err(refused("Registry reconformance declaration differs"));
        }
        self.reconformance = Some(actions);
        Ok(())
    }
    fn actions_for(&self, profile: Profile) -> Result<&[DefinitionRef; 5], ActionStateError> {
        match profile {
            Profile::Ordinary => Ok(&self.actions),
            Profile::Reconformance => self
                .reconformance
                .as_ref()
                .ok_or_else(|| refused("Registry reconformance profile unavailable")),
        }
    }
    pub(in crate::action_state) fn admission_actions(&self) -> Vec<&DefinitionRef> {
        std::iter::once(&self.actions[0])
            .chain(self.reconformance.as_ref().map(|a| &a[0]))
            .collect()
    }
    pub(in crate::action_state) fn validate_families(
        &self,
        registry: &StateFamilyRegistry,
    ) -> Result<(), ActionStateError> {
        let owner = OwnerId::new("roll").map_err(|_| refused("Registry owner"))?;
        for (name, bytes) in FAMILIES {
            let family = StateFamilyId::new(name).map_err(|_| refused("Registry family"))?;
            let canonical = |suffix: &str| {
                CanonicalId::new(format!("roll.{name}.{suffix}.v1"))
                    .map_err(|_| refused("Registry family schema"))
            };
            let expected = StateFamilyDescriptor::new(
                family.clone(),
                canonical("key")?,
                canonical("value")?,
                1,
                canonical("decode")?,
                canonical("validate")?,
                canonical("serialize")?,
            )
            .and_then(|row| row.with_limits(80, bytes))
            .map_err(|_| refused("Registry family bounds"))?;
            let actual = registry
                .entries
                .get(&(owner.clone(), family))
                .ok_or_else(|| refused("Registry family unavailable"))?;
            if actual.descriptor().canonical_value() != expected.canonical_value() {
                return Err(refused("Registry family descriptor differs"));
            }
        }
        if self.reconformance.is_some() {
            reconformance::validate_family(registry)?;
        }
        Ok(())
    }

    pub(in crate::action_state) fn validate_composition(
        &self,
        definitions: &DefinitionRegistry,
        providers: &ActionProviderRegistry,
        participants: &crate::participant::ParticipantRegistry,
    ) -> Result<(), ActionStateError> {
        definitions
            .get_exact(&self.manifest)
            .map_err(|_| refused("Registry manifest unavailable"))?;
        let participant = DefinitionKind::new("owner_participant")
            .map_err(|_| refused("Registry participant kind"))?;
        for (profile, actions) in std::iter::once((Profile::Ordinary, &self.actions)).chain(
            self.reconformance
                .as_ref()
                .map(|a| (Profile::Reconformance, a)),
        ) {
            for (action, transition) in actions.iter().zip(TRANSITIONS) {
                let definition = definitions
                    .get_exact(action)
                    .map_err(|_| refused("Registry action definition unavailable"))?;
                let provider = providers
                    .get_exact(action)
                    .map_err(|_| refused("Registry action provider unavailable"))?;
                if provider.owner().as_str() != "roll"
                    || definition.payload()
                        != &json!({"format":1,"protocol":profile.word(),"transition":transition})
                {
                    return Err(refused("Registry action protocol differs"));
                }
                let references = definition.references();
                if references.len() != 4 || !references.contains(&self.manifest) {
                    return Err(refused("Registry action reference set differs"));
                }
                let mut roll = 0;
                let mut tacsim = 0;
                for reference in references
                    .iter()
                    .filter(|reference| *reference != &self.manifest)
                {
                    definitions
                        .get_exact(reference)
                        .map_err(|_| refused("Registry participant definition unavailable"))?;
                    if reference.key().kind != participant
                        || reference.version() != 1
                        || !participants.grants_exact(reference, provider.owner(), action)
                    {
                        return Err(refused("Registry participant reference kind"));
                    }
                    match (reference.key().owner.as_str(), reference.key().id.as_str()) {
                        ("roll", "commercial_roots") => roll += 1,
                        ("tacsim", "registry_quota" | "registry_artifacts") => tacsim += 1,
                        _ => return Err(refused("Registry participant reference identity")),
                    }
                }
                if roll != 1 || tacsim != 2 {
                    return Err(refused("Registry participant reference set differs"));
                }
            }
        }
        Ok(())
    }

    fn captured_admission(
        &self,
        compatibility: DefinitionDigest,
        profile: Profile,
    ) -> Result<DefinitionRef, ActionStateError> {
        self.actions_for(profile)?;
        let manifest = DefinitionRef::new(self.manifest.key().clone(), 1, compatibility)
            .map_err(|_| refused("Registry captured manifest reference"))?;
        let [admission, ..] = closed_profile_references(&manifest, profile)?;
        Ok(admission)
    }

    fn action_index(
        &self,
        identity: &RegisteredTxnIdentity,
        profile: Profile,
    ) -> Result<usize, ActionStateError> {
        self.actions_for(profile)?
            .iter()
            .position(|action| action == &identity.action)
            .ok_or_else(|| refused("Registry transaction action differs"))
    }
}

/// Only actual provider dispatch binds this to its transaction, before seed or
/// module callbacks. It is not deserialized, stored or constructed by modules.
pub(in crate::action_state) struct RegisteredTxnIdentity {
    pub(super) action: DefinitionRef,
    pub(super) ordinal: u32,
    pub(super) commit: CommitToken,
    pub(super) actor: ObjId,
    pub(super) principal: PrincipalId,
}

impl RegisteredTxnIdentity {
    pub(in crate::action_state) fn new(
        provider: &ActionProviderEntry,
        invocation: &ActionInvocation,
        commit: CommitToken,
    ) -> Self {
        Self {
            action: provider.action().clone(),
            ordinal: provider.registration_ordinal().get(),
            commit,
            actor: invocation.actor(),
            principal: invocation.principal().clone(),
        }
    }
}

fn key(family: &str, job: CommitToken, page: Option<usize>) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: OwnerId::new("roll").map_err(|_| refused("Registry owner"))?,
        family: StateFamilyId::new(family).map_err(|_| refused("Registry family"))?,
        encoded: SmolStr::new(match page {
            Some(page) => format!("{}/{page:03}", job.to_hex()),
            None => job.to_hex(),
        }),
    })
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ActionStateError> {
    value
        .get(name)
        .ok_or_else(|| refused("Registry field unavailable"))
}
fn number(value: &Value, name: &str) -> Result<u64, ActionStateError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| refused("Registry unsigned field"))
}
fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ActionStateError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| refused("Registry string field"))
}
fn token(value: &Value) -> Result<CommitToken, ActionStateError> {
    serde_json::from_value(value.clone()).map_err(|_| refused("Registry token"))
}
fn digest(domain: &str, value: &Value) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value(domain, value).map_err(|_| refused("Registry digest"))
}
fn value(
    view: &View<'_>,
    family: &str,
    job: CommitToken,
    page: Option<usize>,
) -> Result<Option<Value>, ActionStateError> {
    let key = key(family, job, page)?.resolve(view.registry)?;
    let document = match view.mutations.and_then(|mutations| mutations.get(&key)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&key)
            .map(|row| {
                serde_json::from_slice(row.canonical.as_slice())
                    .map_err(|_| refused("Registry retained encoding"))
            })
            .transpose()?,
    };
    document
        .map(|row| field(&row, "value").cloned())
        .transpose()
}
fn required(
    view: &View<'_>,
    family: &str,
    job: CommitToken,
    page: Option<usize>,
) -> Result<Value, ActionStateError> {
    value(view, family, job, page)?.ok_or_else(|| refused("Registry paired row unavailable"))
}
fn immutable_job(job: &Value) -> Result<Value, ActionStateError> {
    let mut value = job
        .as_object()
        .cloned()
        .ok_or_else(|| refused("Registry job object"))?;
    value
        .remove("state")
        .ok_or_else(|| refused("Registry job state"))?;
    Ok(Value::Object(value))
}
fn captured_compatibility(pair: &Pair) -> Result<DefinitionDigest, ActionStateError> {
    serde_json::from_value(field(field(&pair.job, "compatibility")?, "digest")?.clone())
        .map_err(|_| refused("Registry captured compatibility digest"))
}

fn fingerprint(job: &Value, execution: &Value) -> Result<DefinitionDigest, ActionStateError> {
    digest(
        "action_state.registry.admission.v1",
        &json!({
            "job": immutable_job(job)?, "admission": field(execution,"admission")?
        }),
    )
}
fn is_terminal(phase: &str) -> bool {
    matches!(phase, "complete" | "failed" | "cancelled" | "interrupted")
}

struct Pair {
    job: Value,
    execution: Value,
    phase: String,
    pages: usize,
    measured_rejection: bool,
    reconformance: Option<Binding>,
}
impl Pair {
    fn profile(&self) -> Profile {
        if self.reconformance.is_some() {
            Profile::Reconformance
        } else {
            Profile::Ordinary
        }
    }
}
fn pair(view: &View<'_>, id: CommitToken) -> Result<Pair, ActionStateError> {
    let job = required(view, "rating_jobs", id, None)?;
    let execution = required(view, "rating_executions", id, None)?;
    let admission = field(&execution, "admission")?;
    let observation = field(&execution, "observation")?;
    let phase = text(observation, "phase")?;
    let state = field(&job, "state")?;
    let state_kind = text(state, "kind")?;
    let measured_rejection = match admission.get("measured_rejection") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(refused("Registry measured rejection admission")),
    };
    let reconformance = reconformance::binding(admission, measured_rejection)?;
    let expected = match phase {
        "running" | "cancel_requested" => "running",
        "complete" if measured_rejection && state_kind == "nonconforming" => "nonconforming",
        "complete" => "completed",
        other => other,
    };
    let cases = number(admission, "cases")?;
    let pages = number(admission, "maximum_pages")?;
    if number(&job, "format")? != 1
        || number(&execution, "format")? != 1
        || token(field(&job, "job")?)? != id
        || token(field(&execution, "job")?)? != id
        || state_kind != expected
        || !(1..=1755).contains(&cases)
        || pages != cases.div_ceil(15)
        || !(1..=117).contains(&pages)
        || !matches!(
            phase,
            "queued"
                | "running"
                | "cancel_requested"
                | "complete"
                | "failed"
                | "cancelled"
                | "interrupted"
        )
        || field(admission, "plan")? != field(&job, "plan")?
        || field(admission, "compatibility")? != field(field(&job, "compatibility")?, "digest")?
        || *field(admission, "subject")?
            != serde_json::to_value(digest(
                "tacsim.registry.subject.v1",
                field(&job, "subject")?,
            )?)
            .map_err(|_| refused("Registry subject encoding"))?
    {
        return Err(refused("Registry paired identity or state differs"));
    }
    let next = number(observation, "next_sequence")?;
    let charge = number(observation, "charged_unit_ms")?;
    let maximum = number(admission, "maximum_unit_ms")?;
    let cursor = field(observation, "cursor")?;
    let flight = field(observation, "flight")?;
    let result = field(observation, "result")?;
    if next == 0
        || maximum == 0
        || charge > maximum
        || number(cursor, "index")? >= cases
        || (is_terminal(phase) && !flight.is_null())
        || (phase != "complete" && !result.is_null())
        || (phase == "complete" && result.is_null())
        || (phase == "queued") != (token(field(&execution, "updated_by")?)? == id)
    {
        return Err(refused("Registry execution observation differs"));
    }
    if !result.is_null() && !(1..=8 * 1024 * 1024).contains(&number(result, "bytes")?) {
        return Err(refused("Registry result bound"));
    }
    if phase == "queued"
        && (next != 1
            || charge != 0
            || !flight.is_null()
            || number(cursor, "index")? != 0
            || number(cursor, "simulated_ms")? != 0)
    {
        return Err(refused("Registry queued observation differs"));
    }
    if state_kind == "running" && number(state, "lease_sequence")?.checked_add(1) != Some(next) {
        return Err(refused("Registry job lease differs"));
    }
    if !flight.is_null() {
        let units = number(flight, "units")?;
        let native = number(flight, "native_ms")?;
        let allowance = number(flight, "allowance_unit_ms")?;
        if !(1..=33).contains(&units)
            || native < 50
            || native > u64::from(u32::MAX)
            || allowance == 0
            || allowance > units * native * 8
            || !allowance.is_multiple_of(units)
            || allowance > charge
            || number(flight, "sequence")?.checked_add(1) != Some(next)
            || field(flight, "cursor")? != cursor
            || !matches!(phase, "running" | "cancel_requested")
        {
            return Err(refused("Registry paid flight differs"));
        }
    }
    Ok(Pair {
        job,
        execution: execution.clone(),
        phase: phase.into(),
        pages: pages as usize,
        measured_rejection,
        reconformance,
    })
}

fn targets(
    job: CommitToken,
    pages: usize,
    measured_rejection: bool,
) -> Result<Vec<Target>, ActionStateError> {
    let mut targets = Vec::with_capacity(5 + pages);
    for (family, ceiling) in FAMILIES {
        if family == "rating_nonconformance" && !measured_rejection {
            continue;
        }
        let count = if family == "rating_pages" { pages } else { 1 };
        for page in 0..count {
            targets.push(Target {
                key: key(family, job, (family == "rating_pages").then_some(page))?,
                schema: CanonicalId::new(format!("roll.{family}.value.v1"))
                    .map_err(|_| refused("Registry schema"))?,
                schema_version: 1,
                ceiling,
                mode: if matches!(family, "rating_jobs" | "rating_executions") {
                    SlotMode::UpdateExisting
                } else {
                    SlotMode::Create
                },
            });
        }
    }
    targets.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(targets)
}

/// Forward validation uses only core-owned snapshots of already validated
/// canonical values. No owner callback is invoked after acknowledgement.
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    let protocol = view
        .registry
        .registry_rating
        .as_ref()
        .ok_or_else(|| refused("Registry protocol unavailable"))?;
    let pair = pair(view, reservation.funding)?;
    let captured = captured_compatibility(&pair)?;
    if reservation.effect != protocol.captured_admission(captured, pair.profile())?
        || (matches!(phase, Phase::Funding) && captured != protocol.manifest.digest())
        || (matches!(phase, Phase::Terminal)
            && captured != protocol.manifest.digest()
            && pair.phase != "interrupted")
        || reservation.sponsor != key("rating_executions", reservation.funding, None)?
        || reservation.targets != targets(reservation.funding, pair.pages, pair.measured_rejection)?
        || reservation.protocol_fingerprint != Some(fingerprint(&pair.job, &pair.execution)?)
        || matches!(phase, Phase::Funding) && pair.phase != "queued"
        || matches!(phase, Phase::Funding | Phase::Retained) && is_terminal(&pair.phase)
        || matches!(phase, Phase::Terminal) && !is_terminal(&pair.phase)
    {
        return Err(refused(
            "Registry reservation identity or lifecycle differs",
        ));
    }
    reconformance::require_head(view, reservation.funding, &pair)?;
    outputs(view, reservation.funding, &pair)?;
    Ok(())
}

fn verify_pages(
    view: &View<'_>,
    id: CommitToken,
    receipt: &Value,
    result: &Value,
    refs: &[Value],
) -> Result<(), ActionStateError> {
    let mut cases = 0usize;
    for (ordinal, reference) in refs.iter().enumerate() {
        let page = required(view, "rating_pages", id, Some(ordinal))?;
        let content = field(&page, "content")?;
        let rows = field(content, "rows")?
            .as_array()
            .ok_or_else(|| refused("Registry page rows"))?;
        if token(field(&page, "job")?)? != id
            || field(&page, "rating")? != field(receipt, "id")?
            || field(&page, "result")? != field(result, "id")?
            || number(content, "ordinal")? != ordinal as u64
            || number(reference, "ordinal")? != ordinal as u64
            || number(reference, "rows")? != rows.len() as u64
            || rows.is_empty()
            || rows.len() > 15
            || *field(reference, "digest")? != json!(digest("tacsim.registry.page.v1", content)?)
        {
            return Err(refused("Registry page identity differs"));
        }
        cases = add(cases, rows.len())?;
    }
    if number(result, "cases")? != cases as u64 {
        return Err(refused("Registry case count differs"));
    }
    Ok(())
}

fn outputs(view: &View<'_>, id: CommitToken, pair: &Pair) -> Result<(), ActionStateError> {
    let mut expected_pages = 0;
    if pair.phase == "complete" && text(field(&pair.job, "state")?, "kind")? == "completed" {
        let receipt = required(view, "rating_receipts", id, None)?;
        let completion = required(view, "rating_completions", id, None)?;
        let status = required(view, "rating_status", id, None)?;
        let reference = field(field(&pair.job, "state")?, "receipt")?;
        let content = field(&receipt, "content")?;
        let result = field(content, "result")?;
        let admission = field(&pair.execution, "admission")?;
        let refs = field(result, "pages")?
            .as_array()
            .ok_or_else(|| refused("Registry page refs"))?;
        expected_pages = refs.len();
        if token(field(reference, "job")?)? != id
            || token(field(field(&receipt, "publication")?, "job")?)? != id
            || field(reference, "id")? != field(&receipt, "id")?
            || number(reference, "schema")? != 1
            || *field(reference, "receipt_digest")?
                != json!(digest("tacsim.registry.receipt.v1", &receipt)?)
            || field(content, "subject")? != field(&pair.job, "subject")?
            || field(content, "compatibility")? != field(&pair.job, "compatibility")?
            || field(content, "plan")? != field(&pair.job, "plan")?
            || token(field(&completion, "job")?)? != id
            || field(&completion, "receipt")? != reference
            || field(&status, "rating")? != reference
            || field(&completion, "subject")? != field(admission, "subject")?
            || field(&completion, "compatibility")? != field(admission, "compatibility")?
            || field(&completion, "result")? != field(result, "id")?
            || *field(&completion, "execution")?
                != json!(digest("roll.registry.execution.v1", &pair.execution)?)
            || refs.is_empty()
            || refs.len() > pair.pages
            || number(result, "cases")? > number(admission, "cases")?
        {
            return Err(refused("Registry terminal completion differs"));
        }
        let standing = text(field(&status, "standing")?, "kind")?;
        if !matches!(standing, "active" | "recalled") {
            return Err(refused("Registry rating standing"));
        }
        verify_pages(view, id, &receipt, result, refs)?;
    } else if pair.phase == "complete" {
        if !pair.measured_rejection {
            return Err(refused("Registry rejection lacks reserved output"));
        }
        let receipt = required(view, "rating_nonconformance", id, None)?;
        let reference = field(field(&pair.job, "state")?, "receipt")?;
        let result = field(&receipt, "result")?;
        let admission = field(&pair.execution, "admission")?;
        let refs = field(result, "pages")?
            .as_array()
            .ok_or_else(|| refused("Registry rejection page refs"))?;
        expected_pages = refs.len();
        if token(field(reference, "job")?)? != id
            || token(field(field(&receipt, "publication")?, "job")?)? != id
            || number(&receipt, "format")? != 1
            || text(&receipt, "kind")? != "measured_nonconformance"
            || text(&receipt, "cause")? != "maximum_class_budget"
            || field(reference, "id")? != field(&receipt, "id")?
            || number(reference, "schema")? != 1
            || *field(reference, "receipt_digest")?
                != json!(digest(
                    "tacsim.registry.nonconformance.receipt.v1",
                    &receipt
                )?)
            || field(&receipt, "subject")? != field(&pair.job, "subject")?
            || field(&receipt, "compatibility")? != field(&pair.job, "compatibility")?
            || field(&receipt, "plan")? != field(&pair.job, "plan")?
            || refs.is_empty()
            || refs.len() > pair.pages
            || number(result, "cases")? > number(admission, "cases")?
        {
            return Err(refused("Registry terminal nonconformance differs"));
        }
        for family in ["rating_receipts", "rating_completions", "rating_status"] {
            if value(view, family, id, None)?.is_some() {
                return Err(refused("Registry rejected job has successful output"));
            }
        }
        verify_pages(view, id, &receipt, result, refs)?;
    } else {
        for family in ["rating_receipts", "rating_completions", "rating_status"] {
            if value(view, family, id, None)?.is_some() {
                return Err(refused("Registry premature successful output"));
            }
        }
    }
    if (pair.phase != "complete" || text(field(&pair.job, "state")?, "kind")? != "nonconforming")
        && value(view, "rating_nonconformance", id, None)?.is_some()
    {
        return Err(refused("Registry unexpected nonconformance output"));
    }
    for ordinal in expected_pages..117 {
        if value(view, "rating_pages", id, Some(ordinal))?.is_some() {
            return Err(refused("Registry extra result page"));
        }
    }
    Ok(())
}

pub(in crate::action_state) fn registry_change(
    txn: &mut ActionTxn<'_>,
    commit: CommitToken,
    terminal: Option<CommitToken>,
) -> Result<(), ActionStateError> {
    let protocol = txn
        .store
        .registry
        .registry_rating
        .as_ref()
        .ok_or_else(|| refused("Registry protocol unavailable"))?;
    let identity = txn
        .registered_identity
        .as_ref()
        .ok_or_else(|| refused("Registry invocation unavailable"))?;
    if identity.commit != commit
        || txn.provider_ordinal != Some(identity.ordinal)
        || txn.owner != identity.action.key().owner
        || txn.capacity_changes.funding.is_some()
        || txn.capacity_changes.terminal.is_some()
        || txn.capacity_changes.progress.is_some()
    {
        return Err(refused("Registry preparation identity differs"));
    }
    let view = View {
        registry: &txn.store.registry,
        records: &txn.store.records,
        mutations: Some(&txn.mutations),
    };
    let profile = pair(&view, terminal.unwrap_or(commit))?.profile();
    let action = protocol.action_index(identity, profile)?;
    if let Some(job) = terminal {
        if !matches!(action, 2..=4) {
            return Err(refused("Registry terminal action differs"));
        }
        txn.capacity_changes.terminal = Some(ReservationId(txn.owner.clone(), job.to_hex()));
    } else {
        if action != 0 {
            return Err(refused("Registry funding action differs"));
        }
        let view = View {
            registry: &txn.store.registry,
            records: &txn.store.records,
            mutations: Some(&txn.mutations),
        };
        let pair = pair(&view, commit)?;
        let record = Reservation {
            format: 1,
            funding: commit,
            effect: protocol.actions_for(profile)?[0].clone(),
            sponsor: key("rating_executions", commit, None)?,
            targets: targets(commit, pair.pages, pair.measured_rejection)?,
            protocol_fingerprint: Some(fingerprint(&pair.job, &pair.execution)?),
        };
        verify(Phase::Funding, &record, &view)?;
        txn.capacity_changes.funding = Some(record);
    }
    Ok(())
}

fn protected(key: &StateKey) -> bool {
    key.owner.as_str() == "roll"
        && FAMILIES
            .iter()
            .any(|(family, _)| key.family.as_str() == *family)
}
fn key_job(key: &StateKey) -> Result<CommitToken, ActionStateError> {
    let encoded = key.encoded.as_str();
    let job_text = if key.family.as_str() == "rating_pages" {
        let (job, page) = encoded
            .split_once('/')
            .ok_or_else(|| refused("Registry page key"))?;
        let ordinal: usize = page.parse().map_err(|_| refused("Registry page ordinal"))?;
        if ordinal >= 117 || page != format!("{ordinal:03}") {
            return Err(refused("Registry page ordinal"));
        }
        job
    } else {
        encoded
    };
    let job = token(&json!(job_text))?;
    if job.to_hex() != job_text {
        return Err(refused("Registry canonical job key"));
    }
    Ok(job)
}

/// Reverse audit runs even with zero private metadata. A partially missing pair
/// or an orphan success row never masquerades as an empty Registry deployment.
pub(super) fn audit_boot(
    view: &View<'_>,
    reservations: &BTreeMap<ReservationId, Arc<StoredReservation>>,
) -> Result<(), ActionStateError> {
    if view.registry.registry_rating.is_none() {
        return reconformance::audit(view, &BTreeMap::new());
    }
    let mut jobs = BTreeSet::new();
    for key in view.records.keys().filter(|key| protected(key)) {
        jobs.insert(key_job(key)?);
    }
    let mut checked = BTreeMap::new();
    for job in jobs {
        let pair = pair(view, job)?;
        let id = ReservationId(
            OwnerId::new("roll").map_err(|_| refused("Registry owner"))?,
            job.to_hex(),
        );
        match (is_terminal(&pair.phase), reservations.get(&id)) {
            (false, Some(reservation)) => verify(Phase::Retained, &reservation.record, view)?,
            (true, None) => outputs(view, job, &pair)?,
            _ => return Err(refused("Registry reverse reservation completeness")),
        }
        checked.insert(job, pair);
    }
    reconformance::audit(view, &checked)
}

pub(super) fn validate_changes(
    store: &ActionStateStore,
    mutations: &BTreeMap<StateKey, TxnMutation>,
    changes: &Changes,
    identity: Option<&RegisteredTxnIdentity>,
) -> Result<(), ActionStateError> {
    let Some(protocol) = &store.registry.registry_rating else {
        if mutations.keys().any(reconformance::protected) {
            return Err(refused("Registry head without declared protocol"));
        }
        return Ok(());
    };
    let mut jobs = BTreeSet::new();
    for key in mutations.keys().filter(|key| protected(key)) {
        jobs.insert(key_job(key)?);
    }
    if jobs.is_empty() {
        if mutations.keys().any(reconformance::protected) {
            return Err(refused("Registry head without admission"));
        }
        return Ok(());
    }
    let identity = identity.ok_or_else(|| refused("Registry write lacks actual invocation"))?;
    let before = View {
        registry: &store.registry,
        records: &store.records,
        mutations: None,
    };
    let after = View {
        registry: &store.registry,
        records: &store.records,
        mutations: Some(mutations),
    };
    reconformance::validate_changes(&before, &after, mutations, identity, &jobs)?;
    for id in jobs {
        let old_job = value(&before, "rating_jobs", id, None)?;
        let current = pair(&after, id)?;
        let pair_keys = [
            key("rating_jobs", id, None)?.resolve(&store.registry)?,
            key("rating_executions", id, None)?.resolve(&store.registry)?,
        ];
        let pair_changed = pair_keys.iter().any(|key| mutations.contains_key(key));
        if !pair_changed {
            // After completion only a one-way recall may change; all evidence
            // and the exact full RatingRef remain immutable.
            let old = pair(&before, id)?;
            if old.phase != "complete" {
                return Err(refused("Registry output without terminal transition"));
            }
            for (key, mutation) in mutations
                .iter()
                .filter(|(key, _)| protected(key) && key_job(key).ok() == Some(id))
            {
                if key.family.as_str() != "rating_status"
                    || !matches!(mutation, TxnMutation::Put { .. })
                {
                    return Err(refused("Registry completion evidence is immutable"));
                }
            }
            let prior = required(&before, "rating_status", id, None)?;
            let status = required(&after, "rating_status", id, None)?;
            let standing = field(&status, "standing")?;
            if field(&prior, "rating")? != field(&status, "rating")?
                || text(field(&prior, "standing")?, "kind")? != "active"
                || text(standing, "kind")? != "recalled"
                || token(field(standing, "operation")?)? != identity.commit
                || identity.action.key().owner.as_str() != "roll"
            {
                return Err(refused("Registry recall identity differs"));
            }
            outputs(&after, id, &current)?;
            continue;
        }
        if !pair_keys
            .iter()
            .all(|key| matches!(mutations.get(key), Some(TxnMutation::Put { .. })))
        {
            return Err(refused("Registry transition requires both post-images"));
        }
        let action = protocol.action_index(identity, current.profile())?;
        if current.profile() == Profile::Reconformance
            && (identity.actor != crate::GOD
                || identity.principal != (PrincipalId::Object { id: crate::GOD }))
        {
            return Err(refused("Registry service invocation differs"));
        }
        if token(field(&current.execution, "updated_by")?)? != identity.commit {
            return Err(refused("Registry transition commit differs"));
        }
        let reservation_id = ReservationId(
            OwnerId::new("roll").map_err(|_| refused("Registry owner"))?,
            id.to_hex(),
        );
        if old_job.is_none() {
            if value(&before, "rating_executions", id, None)?.is_some()
                || action != 0
                || id != identity.commit
                || identity.principal != (PrincipalId::Object { id: identity.actor })
                || number(field(&current.execution, "admission")?, "character")?
                    != u64::from(identity.actor.0)
                || changes.funding.as_ref().map(|row| row.funding) != Some(id)
            {
                return Err(refused("Registry funding invocation differs"));
            }
            verify(
                Phase::Funding,
                changes
                    .funding
                    .as_ref()
                    .ok_or_else(|| refused("Registry funding absent"))?,
                &after,
            )?;
        } else {
            let old = pair(&before, id)?;
            if is_terminal(&old.phase)
                || (captured_compatibility(&old)? != protocol.manifest.digest() && action != 4)
                || action == 0
                || identity.commit == id
                || token(field(&old.execution, "updated_by")?)? == identity.commit
                || fingerprint(&old.job, &old.execution)?
                    != fingerprint(&current.job, &current.execution)?
            {
                return Err(refused(
                    "Registry immutable admission or transition differs",
                ));
            }
            let old_observation = field(&old.execution, "observation")?;
            let observation = field(&current.execution, "observation")?;
            let old_next = number(old_observation, "next_sequence")?;
            let next = number(observation, "next_sequence")?;
            let old_flight = field(old_observation, "flight")?;
            let flight = field(observation, "flight")?;
            let allowed = match action {
                1 => {
                    matches!(old.phase.as_str(), "queued" | "running")
                        && old_flight.is_null()
                        && !flight.is_null()
                        && current.phase == "running"
                        && old_next.checked_add(1) == Some(next)
                }
                2 => {
                    !old_flight.is_null()
                        && flight.is_null()
                        && next == old_next
                        && (old.phase != "cancel_requested" || current.phase == "cancelled")
                        && matches!(
                            current.phase.as_str(),
                            "running" | "complete" | "failed" | "cancelled"
                        )
                }
                3 => {
                    next == old_next
                        && ((old_flight.is_null() && current.phase == "cancelled")
                            || (!old_flight.is_null()
                                && current.phase == "cancel_requested"
                                && old_flight == flight
                                && number(old_observation, "charged_unit_ms")?
                                    == number(observation, "charged_unit_ms")?))
                }
                4 => {
                    current.phase == "interrupted"
                        && next == old_next
                        && flight.is_null()
                        && number(old_observation, "charged_unit_ms")?
                            == number(observation, "charged_unit_ms")?
                }
                _ => false,
            };
            if !allowed {
                return Err(refused("Registry closed transition differs"));
            }
            let reservation = store
                .capacity
                .records
                .get(&reservation_id)
                .ok_or_else(|| refused("Registry retained reservation absent"))?;
            if is_terminal(&current.phase) {
                if changes.terminal.as_ref() != Some(&reservation_id) {
                    return Err(refused("Registry terminal metadata not retired"));
                }
                verify(Phase::Terminal, &reservation.record, &after)?;
                if current.phase == "complete"
                    && text(field(&current.job, "state")?, "kind")? == "completed"
                {
                    let status = required(&after, "rating_status", id, None)?;
                    if text(field(&status, "standing")?, "kind")? != "active" {
                        return Err(refused("Registry initial rating status"));
                    }
                }
            } else {
                if changes.terminal.as_ref() == Some(&reservation_id) {
                    return Err(refused("Registry pending metadata retired"));
                }
                verify(Phase::Retained, &reservation.record, &after)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
