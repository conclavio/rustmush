//! Real Registry preparation: exact source/envelope expansion, real fixture
//! schedule/costs, bounded immutable recipe and the actual metered Machine.
//! This runs on the shared off-world preparation lane, never a world callback.

use mush_core::{AccountId, ObjId};
use mush_roll::{
    Revision,
    commerce::rating::{MAX_RESULT_BYTES, RatingError},
};
use std::sync::Arc;

use super::{
    machine::RegistryMachine,
    manifest::{FrozenPolicy, RegisteredManifest},
    plan::Schedule,
    subject::PreparedSubject,
};
use crate::minds::bulk::{
    executor::Machine,
    model::{ArtifactBytes, Opponent, Plan, Scenario, WorkKind},
    registry_tickets::AdmissionData,
};

/// Owns the immutable actual registered policy. Its constructor remains private
/// to this module; trusted registration creates it from FrozenPolicy::register.
pub(crate) struct Factory {
    policy: Arc<FrozenPolicy>,
    registered: RegisteredManifest,
}
pub(in crate::minds) struct PreparedRegistry {
    pub(in crate::minds) data: AdmissionData,
    pub(in crate::minds) machine: Box<dyn Machine>,
}
impl Factory {
    pub(super) fn registered(
        policy: Arc<FrozenPolicy>,
        registered: RegisteredManifest,
    ) -> Result<Self, RatingError> {
        if !policy.matches_registration(&registered)? {
            return Err(RatingError::Identity);
        }
        Ok(Self { policy, registered })
    }
    pub(in crate::minds) fn prepare(
        &self,
        account: AccountId,
        character: ObjId,
        nonce: u64,
        revision: Revision,
        source: Arc<str>,
    ) -> Result<PreparedRegistry, RatingError> {
        if character.0 == 0 || nonce == 0 || revision.reference().key.character != character {
            return Err(RatingError::Identity);
        }
        let subject = Arc::new(PreparedSubject::prepare(
            &revision,
            &source,
            &self.policy.cfg.arena,
        )?);
        let schedule = Schedule::capture(&subject, &self.policy)?;
        let budget = schedule.budget(&subject, self.registered.compatibility().digest())?;
        // The recipe records all variants/programs and every admitted case.
        // Sources stay with their existing immutable owner under the actual job
        // root. Boot interrupts a private computation; it never replays a prefix
        // using current content or invents a completed result from this recipe.
        let ground = crate::arena::replay_ground::ReplayGround::capture(&self.policy.ground)
            .map_err(|_| RatingError::Invalid)?;
        #[derive(serde::Serialize)]
        struct Recipe<'a> {
            format: u16,
            manifest: &'a mush_core::DefinitionRef,
            compatibility: &'a mush_roll::commerce::rating::RatingCompatibility,
            subject: &'a mush_roll::commerce::rating::RatingSubject,
            variants: &'a [super::subject::Variant],
            pairs: &'a [super::subject::Pair],
            aliases: &'a [mush_roll::commerce::rating::VariantAlias],
            schedule: &'a Schedule,
            configuration: (
                &'a crate::config::ArenaConfig,
                &'a crate::config::MatchConfig,
                &'a crate::config::SensorsConfig,
                &'a crate::config::MindsConfig,
            ),
            ground: &'a crate::arena::replay_ground::ReplayGround,
        }
        let cfg = &self.policy.cfg;
        let start = ArtifactBytes::encode(&Recipe {
            format: 1,
            manifest: self.registered.definition(),
            compatibility: self.registered.compatibility(),
            subject: subject.subject(),
            variants: subject.variants(),
            pairs: subject.pairs(),
            aliases: subject.aliases(),
            schedule: &schedule,
            configuration: (&cfg.arena, &cfg.match_, &cfg.sensors, &cfg.minds),
            ground: &ground,
        })
        .map_err(|_| RatingError::Bound)?;
        let plan = Plan {
            kind: WorkKind::Registry,
            registry: Some(budget),
            scenario: Scenario::Duel,
            opponent: Opponent::Doctrine,
            per_side: 0,
            unit_count: 0,
            games: 0,
            seeds: Vec::new(),
            limit_ms: 0,
            native_substep_ms: 0,
            start: start.descriptor().clone(),
            result_reservation_bytes: MAX_RESULT_BYTES as u64,
        };
        plan.validate().map_err(|_| RatingError::Invalid)?;
        let data = AdmissionData {
            account,
            character,
            nonce,
            subject: subject.subject().clone(),
            compatibility: self.registered.compatibility().clone(),
            plan,
            start,
        };
        let machine = RegistryMachine::new(
            Arc::clone(&self.policy),
            subject,
            &self.registered,
            schedule,
        )?;
        Ok(PreparedRegistry {
            data,
            machine: Box::new(machine),
        })
    }
}
