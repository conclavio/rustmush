//! Private commission evidence, read from its committed survey receipt.
//!
//! An exact contract may ask about its own attempt. Neither a guessed reference,
//! another contract nor an unscoped reader can discover coverage or completion.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use mush_core::action_state::StateFamilyHandle;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    DefinitionValidationContext, OwnerId,
};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{ModError, ObjId, Registrar, World};
use mush_economy::{
    ContractFactScope, ContractId, ContractState, EconomyStore, ExternalRef, FactProvider,
    FactPurpose, FactState, ScopedFactContext,
};
use mush_org::PrincipalId;
use serde::Serialize;

use super::survey::{SurveyKey, SurveyRecord};

/// The namespace composed only with the TacSim owner.
pub const PROVIDER: &str = "tacsim.survey";
/// The commissioned survey's successful recorded outcome (doc 20 phase 2).
pub const FACT_LOGGED: &str = "tacsim.survey.logged";
/// The contractor's committed cancellation, which returns the reserved reward.
pub const FACT_CANCELLED: &str = "tacsim.survey.cancelled";
const READER_ID: &str = "survey_contract_facts";

/// Economy's opaque, composition-bound handle to the survey owner.
#[derive(Clone, Default)]
pub struct SurveyFactProvider(Arc<OnceLock<OwnerReadHandle<Reader>>>);

/// TacSim's producer capability is separate from Economy's reader capability.
#[derive(Clone)]
struct AttachmentReader(OwnerReadHandle<Reader>);

impl std::fmt::Debug for SurveyFactProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurveyFactProvider").finish_non_exhaustive()
    }
}

impl SurveyFactProvider {
    fn scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<Reply> {
        let query = || Query {
            scope: ctx.scope().clone(),
            fact: fact.clone(),
        };
        if let Ok(reply) = ctx.read_owner(self.0.get()?, query()) {
            return Some(reply);
        }
        // A public Economy API preserves the actual command's owner. The
        // producer therefore needs its own handle; neither handle can be used
        // by a dispatch attributed to the other owner (or any third owner).
        if ctx.scope().purpose() != FactPurpose::Attach {
            return None;
        }
        let local = ctx.world().module_state::<AttachmentReader>()?;
        ctx.read_owner(&local.0, query()).ok()
    }
}

impl FactProvider for SurveyFactProvider {
    fn provider(&self) -> &str {
        PROVIDER
    }

    fn requires_scope(&self, _fact: &ExternalRef) -> bool {
        // Malformed and future spellings stay private too.
        true
    }

    fn resolve(
        &self,
        _w: &World,
        _kind: &str,
        _id: &str,
        _revision: Option<u64>,
    ) -> Option<FactState> {
        None
    }

    fn authorize_scope(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> bool {
        self.scoped(ctx, fact).is_some()
    }

    fn resolve_scoped(&self, ctx: &ScopedFactContext<'_>, fact: &ExternalRef) -> Option<FactState> {
        let Reply { truth } = self.scoped(ctx, fact)?;
        truth.map(|truth| FactState {
            satisfied: true,
            actor: Some(truth.actor),
            at: truth.at_utc,
        })
    }

    fn register_scoped(&self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        if reg.module() != "economy" {
            return Err(error("survey facts require actual Economy registration"));
        }
        let definition = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS).map_err(error)?, descriptor()?)
            .map_err(error)?;
        let handle = reg.request_owner_read::<Reader>(definition)?;
        self.0
            .set(handle)
            .map_err(|_| error("survey facts already registered"))
    }
}

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

fn descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(error)?,
        DefinitionId::new(READER_ID).map_err(error)?,
        1,
        serde_json::json!({
            "format": 1,
            "purpose": "survey_commission_fact",
            "scope": "exact_contract_and_current_attempt",
            "truth": "committed_completion_or_contractor_cancellation",
            "chart_data": false
        }),
    )
    .map_err(error)
}

/// TacSim has one validator for its owner-read kind. The optional authored-fork
/// reader keeps its exact descriptor, including its sole current combat manifest.
fn validate_definition(ctx: &DefinitionValidationContext<'_>) -> Result<(), String> {
    let actual = ctx.definition();
    let reference = actual.reference();
    let expected = match reference.key().id.as_str() {
        READER_ID => descriptor().map_err(|e| e.to_string())?,
        crate::port::passenger::facts::READER_ID => {
            crate::port::passenger::facts::descriptor().map_err(|e| e.to_string())?
        }
        "authored_fork_material" => {
            let [manifest] = actual.references() else {
                return Err("fork source material needs its exact Registry manifest".into());
            };
            if !ctx
                .get_exact(manifest)
                .map_err(|e| e.to_string())?
                .is_current()
            {
                return Err("fork source material needs the current Registry manifest".into());
            }
            mush_roll::commerce::fork_protocol::material_descriptor(manifest)
                .map_err(|e| e.to_string())?
        }
        _ => return Err("unknown TacSim owner-read definition".into()),
    };
    let mut references = expected.references().to_vec();
    references.sort();
    if reference.key().owner.as_str() != crate::NS
        || reference.key().kind != *expected.kind()
        || reference.key().id != *expected.id()
        || reference.version() != expected.version()
        || actual.payload() != expected.payload()
        || actual.references() != references
        || actual.is_current() != expected.is_current()
    {
        return Err("TacSim owner-read definition differs from its closed configuration".into());
    }
    Ok(())
}

/// Register under TacSim after the survey family, regardless of layer readiness.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    surveys: StateFamilyHandle<SurveyKey, SurveyRecord>,
    economy_wired: bool,
) -> Result<(), ModError> {
    if reg.module() != crate::NS {
        return Err(error("survey receipts are TacSim-owned"));
    }
    let definition = reg.definition(descriptor()?)?;
    reg.definition_validator(
        DefinitionKind::new("owner_read").map_err(error)?,
        validate_definition,
    )?;
    // Standalone TacSim rosters keep their definitions and the shared fork
    // validator without declaring a dependency on an absent Economy owner.
    // The full port roster supplies both Economy and the inventory capability.
    if !economy_wired {
        return Ok(());
    }
    reg.owner_read_service(
        definition.clone(),
        BTreeSet::from([
            OwnerId::new("economy").map_err(error)?,
            OwnerId::new(crate::NS).map_err(error)?,
        ]),
        BTreeSet::from([OwnerId::new("economy").map_err(error)?]),
        Reader { surveys },
    )?;
    let handle = reg.request_owner_read::<Reader>(definition)?;
    reg.state(AttachmentReader(handle));
    Ok(())
}

#[derive(Serialize)]
struct Query {
    scope: ContractFactScope,
    fact: ExternalRef,
}

#[derive(Serialize)]
struct Reply {
    truth: Option<Truth>,
}

#[derive(Serialize)]
struct Truth {
    actor: ObjId,
    at_utc: i64,
}

struct Reader {
    surveys: StateFamilyHandle<SurveyKey, SurveyRecord>,
}

fn reference_key(fact: &ExternalRef) -> Option<(SurveyKey, ContractId)> {
    if fact.provider != PROVIDER || !matches!(fact.kind.as_str(), FACT_LOGGED | FACT_CANCELLED) {
        return None;
    }
    let mut parts = fact.id.split('/');
    let offer = parts.next()?;
    let contract = parts.next()?;
    if let Some(digest) = parts.next()
        && (digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    let key = SurveyKey::parse(offer)?;
    let id = contract.parse::<u64>().ok()?;
    if id == u64::MAX || key.key_text() != offer || id.to_string() != contract {
        return None;
    }
    Some((key, ContractId(id)))
}

impl OwnerReadService for Reader {
    type State = crate::TacsimStore;
    type Request = Query;
    type Output = Reply;

    fn read(
        &self,
        _state: &Self::State,
        ctx: &OwnerReadContext<'_>,
        Query { scope, fact }: Query,
    ) -> Result<Reply, OwnerReadError> {
        let refuse = OwnerReadError::Unavailable;
        let authorized_requester = ctx.requester().as_str() == "economy"
            || (ctx.requester().as_str() == crate::NS && scope.purpose() == FactPurpose::Attach);
        if !authorized_requester || !scope.matches_read(ctx, &fact) || scope.generation() != 0 {
            return Err(refuse);
        }
        let (key, contract) = reference_key(&fact).ok_or(refuse)?;
        if scope.contract() != contract {
            return Err(refuse);
        }
        let candidate;
        let row = if scope.purpose() == FactPurpose::Attach {
            // This RAM candidate authorizes attachment only. It never answers
            // true; truth requires the later authoritative family receipt.
            candidate =
                crate::port::survey::attachment_candidate(ctx.world(), scope.actor(), contract)
                    .ok_or(refuse)?;
            &candidate
        } else {
            ctx.action_state(&self.surveys, &key)?.ok_or(refuse)?
        };
        row.check().map_err(|_| OwnerReadError::Integrity)?;
        let expected_fact = match fact.kind.as_str() {
            FACT_LOGGED => crate::port::survey::fact_ref(row),
            FACT_CANCELLED => crate::port::survey::cancel_fact_ref(row),
            _ => return Err(refuse),
        };
        if row.offer != key
            || row.contract != contract.0
            || expected_fact != fact
            || scope.issuer() != &PrincipalId::Object(row.pilot)
        {
            return Err(refuse);
        }
        if scope.purpose() == FactPurpose::Attach {
            let expected = crate::port::survey::expected_consent(row);
            if scope.actor() != row.pilot
                || scope.consent_hash()
                    != mush_economy::consent_hash(&PrincipalId::Object(row.pilot), &expected)
            {
                return Err(refuse);
            }
            return Ok(Reply { truth: None });
        }
        if !crate::port::survey::contract_valid(ctx.world(), row)
            || scope.attachment_actor(&fact) != Some(row.pilot)
            || (scope.purpose() == FactPurpose::Read
                && !mush_economy::acts_for(
                    ctx.world(),
                    scope.actor(),
                    &PrincipalId::Object(row.pilot),
                )
                && !mush_economy::acts_for(ctx.world(), scope.actor(), &row.sponsor))
        {
            return Err(refuse);
        }
        let economy = ctx.world().module_state::<EconomyStore>().ok_or(refuse)?;
        let current = economy.snapshot().contract(contract).ok_or(refuse)?;
        if scope.consent_hash() != current.consent_hash {
            return Err(refuse);
        }
        let funded = crate::port::survey::contract_active(ctx.world(), row)
            || matches!(current.state, ContractState::Settled { .. });
        let at_utc = if !funded {
            None
        } else {
            match fact.kind.as_str() {
                FACT_LOGGED => row
                    .completed
                    .as_ref()
                    .filter(|done| {
                        row.cancelled.is_none()
                            && row.covered() == 256
                            && done.at_ic >= row.accepted_ic
                            && done.at_ic <= row.deadline_ic
                    })
                    .map(|done| done.at_utc),
                FACT_CANCELLED => row
                    .cancelled
                    .as_ref()
                    .filter(|cancelled| {
                        row.completed.is_none() && cancelled.at_ic >= row.accepted_ic
                    })
                    .map(|cancelled| cancelled.at_utc),
                _ => None,
            }
        };
        Ok(Reply {
            truth: at_utc.map(|at_utc| Truth {
                actor: row.pilot,
                at_utc,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{Engine, GameModule, Registry};
    use serde_json::json;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn standalone_tacsim_composes_without_an_economy_reader_dependency() -> TestResult {
        Engine::compose_modules(
            Registry::new(),
            vec![Box::new(crate::TacsimRuleset::default())],
        )?;
        Ok(())
    }

    // Only the owner-read definitions are under test. This dependency is a
    // registry locator fixture, not a certified combat manifest or a grant.
    struct Definitions {
        manifest: Option<DefinitionDescriptor>,
        readers: Vec<DefinitionDescriptor>,
    }

    impl GameModule for Definitions {
        fn name(&self) -> &'static str {
            crate::NS
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.definition_validator(
                DefinitionKind::new("owner_read").map_err(error)?,
                validate_definition,
            )?;
            if let Some(manifest) = &self.manifest {
                let expected = manifest.clone();
                reg.definition_validator(
                    DefinitionKind::new("registry_manifest").map_err(error)?,
                    move |ctx: &DefinitionValidationContext<'_>| {
                        let actual = ctx.definition();
                        if actual.payload() == expected.payload()
                            && actual.references() == expected.references()
                            && actual.is_current() == expected.is_current()
                        {
                            Ok(())
                        } else {
                            Err("manifest locator fixture changed".into())
                        }
                    },
                )?;
                reg.definition(manifest.clone())?;
            }
            for reader in &self.readers {
                reg.definition(reader.clone())?;
            }
            Ok(())
        }
    }

    fn compose(
        manifest: Option<DefinitionDescriptor>,
        readers: Vec<DefinitionDescriptor>,
    ) -> Result<(), ModError> {
        Engine::compose_modules(
            Registry::new(),
            vec![Box::new(Definitions { manifest, readers })],
        )
        .map(|_| ())
    }

    fn changed_payload(
        original: &DefinitionDescriptor,
        payload: serde_json::Value,
    ) -> Result<DefinitionDescriptor, ModError> {
        DefinitionDescriptor::new(
            original.kind().clone(),
            original.id().clone(),
            original.version(),
            payload,
        )
        .map(|d| d.with_references(original.references().to_vec()))
        .map_err(error)
    }

    #[test]
    fn shared_validator_accepts_survey_and_exact_fork_but_rejects_changed_payloads() -> TestResult {
        let survey = descriptor()?;
        compose(None, vec![survey.clone()])?;
        let manifest = DefinitionDescriptor::new(
            DefinitionKind::new("registry_manifest")?,
            DefinitionId::new("combat")?,
            1,
            json!({"fixture": "owner_read_dependency"}),
        )?
        .as_current();
        let reference = DefinitionRegistryBuilder::new()
            .register(OwnerId::new(crate::NS)?, manifest.clone())?;
        let fork = mush_roll::commerce::fork_protocol::material_descriptor(&reference)?;
        compose(Some(manifest.clone()), vec![survey.clone(), fork.clone()])?;

        let mut leaked_chart = survey.payload().clone();
        leaked_chart["chart_data"] = json!(true);
        check!(compose(None, vec![changed_payload(&survey, leaked_chart)?]).is_err());

        let mut broadened_fork = fork.payload().clone();
        broadened_fork["source_authority"] = json!(true);
        check!(
            compose(
                Some(manifest.clone()),
                vec![survey.clone(), changed_payload(&fork, broadened_fork)?]
            )
            .is_err()
        );

        let mut repointed_fork = fork.payload().clone();
        repointed_fork["manifest"] =
            serde_json::to_value(DefinitionRegistryBuilder::new().register(
                OwnerId::new(crate::NS)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("registry_manifest")?,
                    DefinitionId::new("combat")?,
                    1,
                    json!({"fixture": "different_manifest"}),
                )?,
            )?)?;
        check!(
            compose(
                Some(manifest.clone()),
                vec![survey.clone(), changed_payload(&fork, repointed_fork)?]
            )
            .is_err()
        );
        check!(
            compose(
                Some(manifest),
                vec![survey, fork.with_references(Vec::new())]
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn survey_references_allow_the_first_contract_but_never_unscoped_disclosure() -> TestResult {
        let provider = SurveyFactProvider::default();
        let world = World::new_minimal();
        let mut fact = ExternalRef {
            provider: PROVIDER.into(),
            kind: FACT_LOGGED.into(),
            id: "ahuna-a/0".into(),
            revision: Some(1),
        };
        check_eq!(reference_key(&fact).map(|(_, id)| id), Some(ContractId(0)));
        fact.id = format!("ahuna-a/0/{}", "a".repeat(64)).into();
        check_eq!(reference_key(&fact).map(|(_, id)| id), Some(ContractId(0)));
        for suffix in [
            "A".repeat(64),
            "a".repeat(63),
            format!("{}/extra", "a".repeat(64)),
        ] {
            fact.id = format!("ahuna-a/0/{suffix}").into();
            check!(reference_key(&fact).is_none());
        }
        fact.id = "ahuna-a/0".into();
        fact.kind = FACT_CANCELLED.into();
        check_eq!(reference_key(&fact).map(|(_, id)| id), Some(ContractId(0)));
        check!(
            provider
                .resolve(&world, FACT_CANCELLED, &fact.id, Some(1))
                .is_none()
        );
        fact.kind = "tacsim.survey.unknown".into();
        check!(reference_key(&fact).is_none());
        check!(provider.requires_scope(&fact));
        fact.kind = FACT_LOGGED.into();
        for id in [
            "ahuna-a/0",
            "ahuna-a/9",
            "ahuna-a/09",
            "ahuna-a/+9",
            "ahuna-a/9/1",
            "missing/9",
            "ahuna-a/18446744073709551615",
            "",
        ] {
            fact.id = id.into();
            check!(provider.requires_scope(&fact));
            check!(provider.resolve(&world, FACT_LOGGED, id, Some(1)).is_none());
        }
        for id in [
            "ahuna-a/09",
            "ahuna-a/+9",
            "ahuna-a/9/1",
            "missing/9",
            "ahuna-a/18446744073709551615",
            "",
        ] {
            fact.id = id.into();
            check!(reference_key(&fact).is_none());
        }
        Ok(())
    }
}
