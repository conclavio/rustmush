//! Bounded internal and projected legal-action offers.
//!
//! An [`InternalOffer`] retains exact action identity, truth subject, guards,
//! and provider-only parameters. Its [`ProjectedLegalAction`] contains only
//! viewer-safe choices plus an opaque HMAC binding. Submission regenerates an
//! offer set, constant-time matches that binding, validates projected input,
//! and only then constructs the canonical [`ActionInvocation`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Sha256;
use thiserror::Error;

use crate::action::{
    ActionError, ActionInvocation, ActionInvocationDraft, ActionMetadataError, ActionOfferId,
    CanonicalHash, CanonicalHashError, GuardSet, PerceivedRef, PrincipalId, RequestUid,
    ScopedIdempotencyKey, SubjectKey,
};
use crate::behavior::ChannelId;
use crate::clock::ClockStamp;
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, DefinitionRef, OwnerId, canonical_json_bytes_with_limit,
};
use crate::obj::ObjId;

/// Maximum offers projected for one viewer/subject scope.
pub const MAX_PROJECTED_LEGAL_ACTIONS: usize = 128;
/// Maximum public parameters on one offer.
pub const MAX_PROJECTED_ACTION_PARAMETERS: usize = 32;
/// Maximum choices on one parameter or target selector.
pub const MAX_PROJECTED_ACTION_CHOICES: usize = 256;
/// Maximum required channels on one offer.
pub const MAX_PROJECTED_ACTION_CHANNELS: usize = 32;
/// Maximum canonical bytes in one projected offer.
pub const MAX_PROJECTED_LEGAL_ACTION_BYTES: usize = 64 * 1024;
/// Maximum canonical bytes across one projected offer set.
pub const MAX_PROJECTED_LEGAL_ACTION_SET_BYTES: usize = 1024 * 1024;
/// Maximum bytes in one public presentation string or text parameter.
pub const MAX_PROJECTED_ACTION_TEXT_BYTES: usize = 4 * 1024;
/// Maximum simultaneously retained HMAC secret versions.
pub const MAX_OFFER_SECRET_VERSIONS: usize = 4;

const OFFER_HMAC_DOMAIN: &[u8] = b"rustmush.offer.hmac.sha256.v1\0";

/// Internal identity of one viewer/sharing projection generation.
///
/// This hash is never sent to a client. It makes scope binding explicit while
/// keeping an [`InternalOffer`] independent of connection IDs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectionScopeId(CanonicalHash);

impl ProjectionScopeId {
    pub fn derive(
        principal: &PrincipalId,
        projection_generation: u64,
    ) -> Result<Self, CanonicalHashError> {
        CanonicalHash::for_canonical_value(
            "projection_scope.v1",
            1,
            &json!({
                "principal": principal,
                "projection_generation": projection_generation,
            }),
            16 * 1024,
        )
        .map(Self)
    }
}

/// Trusted server query presented to every registered action provider in
/// canonical registry order. It contains both the truth and projected subject
/// identities; only the latter can enter a client DTO.
#[derive(Clone, Debug)]
pub struct LegalActionQuery {
    principal: PrincipalId,
    actor: ObjId,
    truth_subject: SubjectKey,
    projected_subject: PerceivedRef,
    viewer_scope: ProjectionScopeId,
    now: Option<ClockStamp>,
}

impl LegalActionQuery {
    pub fn new(
        principal: PrincipalId,
        actor: ObjId,
        truth_subject: SubjectKey,
        projected_subject: PerceivedRef,
        projection_generation: u64,
        now: Option<ClockStamp>,
    ) -> Result<Self, CanonicalHashError> {
        let viewer_scope = ProjectionScopeId::derive(&principal, projection_generation)?;
        Ok(Self {
            principal,
            actor,
            truth_subject,
            projected_subject,
            viewer_scope,
            now,
        })
    }

    #[must_use]
    pub const fn principal(&self) -> &PrincipalId {
        &self.principal
    }

    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }

    #[must_use]
    pub const fn truth_subject(&self) -> &SubjectKey {
        &self.truth_subject
    }

    #[must_use]
    pub const fn projected_subject(&self) -> &PerceivedRef {
        &self.projected_subject
    }

    #[must_use]
    pub const fn viewer_scope(&self) -> ProjectionScopeId {
        self.viewer_scope
    }

    #[must_use]
    pub const fn now(&self) -> Option<&ClockStamp> {
        self.now.as_ref()
    }
}

/// Viewer-safe parameter kind. Rulesets own semantics; core validates only
/// the projected shape and bounds before provider execution.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectedParameterKind {
    Boolean,
    Integer { minimum: i64, maximum: i64 },
    Text { maximum_bytes: usize },
    Choice { choices: Vec<String> },
}

/// One named public input accepted by an offer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectedParameter {
    pub id: CanonicalId,
    pub required: bool,
    pub kind: ProjectedParameterKind,
}

/// Canonically ordered public parameter schema.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProjectedParameterSchema(Vec<ProjectedParameter>);

impl ProjectedParameterSchema {
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn new(mut parameters: Vec<ProjectedParameter>) -> Result<Self, OfferBuildError> {
        if parameters.len() > MAX_PROJECTED_ACTION_PARAMETERS {
            return Err(OfferBuildError::ParameterLimit {
                actual: parameters.len(),
                max: MAX_PROJECTED_ACTION_PARAMETERS,
            });
        }
        parameters.sort_by(|left, right| left.id.cmp(&right.id));
        if parameters.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(OfferBuildError::DuplicateParameter);
        }
        for parameter in &parameters {
            validate_parameter_kind(&parameter.kind)?;
        }
        Ok(Self(parameters))
    }

    #[must_use]
    pub fn parameters(&self) -> &[ProjectedParameter] {
        &self.0
    }

    fn validate(&self, supplied: &Value) -> Result<(), OfferResolveError> {
        let supplied = supplied
            .as_object()
            .ok_or(OfferResolveError::InvalidParameters)?;
        if supplied.len() > self.0.len() {
            return Err(OfferResolveError::InvalidParameters);
        }
        for key in supplied.keys() {
            if !self.0.iter().any(|parameter| parameter.id.as_str() == key) {
                return Err(OfferResolveError::InvalidParameters);
            }
        }
        for parameter in &self.0 {
            let value = supplied.get(parameter.id.as_str());
            if parameter.required && value.is_none() {
                return Err(OfferResolveError::InvalidParameters);
            }
            if let Some(value) = value {
                validate_parameter_value(&parameter.kind, value)?;
            }
        }
        Ok(())
    }
}

fn validate_parameter_kind(kind: &ProjectedParameterKind) -> Result<(), OfferBuildError> {
    match kind {
        ProjectedParameterKind::Boolean => Ok(()),
        ProjectedParameterKind::Integer { minimum, maximum } => {
            if minimum > maximum {
                Err(OfferBuildError::InvalidParameterBounds)
            } else {
                Ok(())
            }
        }
        ProjectedParameterKind::Text { maximum_bytes } => {
            if *maximum_bytes == 0 || *maximum_bytes > MAX_PROJECTED_ACTION_TEXT_BYTES {
                Err(OfferBuildError::InvalidTextLimit {
                    actual: *maximum_bytes,
                    max: MAX_PROJECTED_ACTION_TEXT_BYTES,
                })
            } else {
                Ok(())
            }
        }
        ProjectedParameterKind::Choice { choices } => {
            if choices.is_empty() || choices.len() > MAX_PROJECTED_ACTION_CHOICES {
                return Err(OfferBuildError::ChoiceLimit {
                    actual: choices.len(),
                    max: MAX_PROJECTED_ACTION_CHOICES,
                });
            }
            let mut unique = BTreeSet::new();
            for choice in choices {
                if choice.is_empty() || choice.len() > MAX_PROJECTED_ACTION_TEXT_BYTES {
                    return Err(OfferBuildError::InvalidChoice);
                }
                if !unique.insert(choice) {
                    return Err(OfferBuildError::DuplicateChoice);
                }
            }
            Ok(())
        }
    }
}

fn validate_parameter_value(
    kind: &ProjectedParameterKind,
    value: &Value,
) -> Result<(), OfferResolveError> {
    let valid = match kind {
        ProjectedParameterKind::Boolean => value.is_boolean(),
        ProjectedParameterKind::Integer { minimum, maximum } => value
            .as_i64()
            .is_some_and(|value| (*minimum..=*maximum).contains(&value)),
        ProjectedParameterKind::Text { maximum_bytes } => value
            .as_str()
            .is_some_and(|value| value.len() <= *maximum_bytes),
        ProjectedParameterKind::Choice { choices } => value
            .as_str()
            .is_some_and(|value| choices.iter().any(|choice| choice == value)),
    };
    if valid {
        Ok(())
    } else {
        Err(OfferResolveError::InvalidParameters)
    }
}

/// Viewer-safe target selection contract.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectedTargetSchema {
    None,
    Choices {
        minimum: usize,
        maximum: usize,
        choices: Vec<PerceivedRef>,
    },
}

impl ProjectedTargetSchema {
    fn validate_shape(&self) -> Result<(), OfferBuildError> {
        let Self::Choices {
            minimum,
            maximum,
            choices,
        } = self
        else {
            return Ok(());
        };
        if minimum > maximum
            || *maximum > choices.len()
            || choices.len() > MAX_PROJECTED_ACTION_CHOICES
        {
            return Err(OfferBuildError::InvalidTargetBounds);
        }
        let unique: BTreeSet<_> = choices.iter().collect();
        if unique.len() != choices.len() {
            return Err(OfferBuildError::DuplicateTargetChoice);
        }
        Ok(())
    }

    fn validate_submission(&self, targets: &[PerceivedRef]) -> Result<(), OfferResolveError> {
        match self {
            Self::None => {
                if targets.is_empty() {
                    Ok(())
                } else {
                    Err(OfferResolveError::InvalidTargets)
                }
            }
            Self::Choices {
                minimum,
                maximum,
                choices,
            } => {
                if targets.len() < *minimum || targets.len() > *maximum {
                    return Err(OfferResolveError::InvalidTargets);
                }
                let unique: BTreeSet<_> = targets.iter().collect();
                if unique.len() != targets.len()
                    || targets.iter().any(|target| !choices.contains(target))
                {
                    return Err(OfferResolveError::InvalidTargets);
                }
                Ok(())
            }
        }
    }
}

/// Presentation shared by line, panel, NPC, and LLM projections.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionPresentation {
    pub verb: String,
    pub label: String,
    pub help: String,
}

impl ActionPresentation {
    pub fn new(
        verb: impl Into<String>,
        label: impl Into<String>,
        help: impl Into<String>,
    ) -> Result<Self, OfferBuildError> {
        let value = Self {
            verb: verb.into(),
            label: label.into(),
            help: help.into(),
        };
        if value.verb.is_empty() || value.label.is_empty() {
            return Err(OfferBuildError::EmptyPresentation);
        }
        if [&value.verb, &value.label, &value.help]
            .into_iter()
            .any(|part| part.len() > MAX_PROJECTED_ACTION_TEXT_BYTES)
        {
            return Err(OfferBuildError::PresentationTooLarge {
                max: MAX_PROJECTED_ACTION_TEXT_BYTES,
            });
        }
        Ok(value)
    }
}

/// The only legal-action data exposed to a caller.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectedLegalAction {
    offer: ActionOfferId,
    subject: PerceivedRef,
    parameters: ProjectedParameterSchema,
    targets: ProjectedTargetSchema,
    required_channels: Vec<ChannelId>,
    expires: Option<ClockStamp>,
    presentation: ActionPresentation,
}

impl ProjectedLegalAction {
    #[must_use]
    pub const fn offer(&self) -> ActionOfferId {
        self.offer
    }

    #[must_use]
    pub const fn subject(&self) -> &PerceivedRef {
        &self.subject
    }

    #[must_use]
    pub const fn parameters(&self) -> &ProjectedParameterSchema {
        &self.parameters
    }

    #[must_use]
    pub const fn targets(&self) -> &ProjectedTargetSchema {
        &self.targets
    }

    #[must_use]
    pub fn required_channels(&self) -> &[ChannelId] {
        &self.required_channels
    }

    #[must_use]
    pub const fn expires(&self) -> Option<&ClockStamp> {
        self.expires.as_ref()
    }

    #[must_use]
    pub const fn presentation(&self) -> &ActionPresentation {
        &self.presentation
    }
}

/// One HMAC secret version. Debug output intentionally redacts key material.
#[derive(Clone)]
pub struct OfferSecret {
    version: u32,
    key: [u8; 32],
}

impl fmt::Debug for OfferSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OfferSecret")
            .field("version", &self.version)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl OfferSecret {
    pub fn new(version: u32, key: [u8; 32]) -> Result<Self, OfferSecretError> {
        if version == 0 {
            return Err(OfferSecretError::ZeroVersion);
        }
        if key == [0; 32] {
            return Err(OfferSecretError::ZeroKey);
        }
        Ok(Self { version, key })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

/// Current plus retained offer-verification keys. Deployment code is
/// responsible for loading these bytes from durable secret storage before
/// accepting ingress.
#[derive(Clone)]
pub struct OfferKeyRing {
    current: OfferSecret,
    retained: BTreeMap<u32, OfferSecret>,
}

impl fmt::Debug for OfferKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OfferKeyRing")
            .field("current_version", &self.current.version)
            .field("retained_versions", &self.retained.keys())
            .finish()
    }
}

impl OfferKeyRing {
    pub fn new(
        current: OfferSecret,
        retained: impl IntoIterator<Item = OfferSecret>,
    ) -> Result<Self, OfferSecretError> {
        let mut retained_by_version = BTreeMap::new();
        for secret in retained {
            if secret.version >= current.version {
                return Err(OfferSecretError::NonMonotonicVersion);
            }
            if retained_by_version.insert(secret.version, secret).is_some() {
                return Err(OfferSecretError::DuplicateVersion);
            }
        }
        if retained_by_version.len().saturating_add(1) > MAX_OFFER_SECRET_VERSIONS {
            return Err(OfferSecretError::VersionLimit {
                max: MAX_OFFER_SECRET_VERSIONS,
            });
        }
        Ok(Self {
            current,
            retained: retained_by_version,
        })
    }

    #[must_use]
    pub const fn current_version(&self) -> u32 {
        self.current.version
    }

    fn sign(&self, binding: CanonicalHash) -> ActionOfferId {
        ActionOfferId(offer_mac(&self.current, binding))
    }

    fn matches(&self, offer: ActionOfferId, binding: CanonicalHash) -> bool {
        let mut matched = false;
        for secret in std::iter::once(&self.current).chain(self.retained.values()) {
            let mac = offer_mac_state(secret, binding);
            matched |= mac.verify_slice(offer.0.as_bytes()).is_ok();
        }
        matched
    }
}

fn offer_mac(secret: &OfferSecret, binding: CanonicalHash) -> CanonicalHash {
    let bytes = offer_mac_state(secret, binding).finalize().into_bytes();
    let mut result = [0_u8; 32];
    result.copy_from_slice(&bytes);
    CanonicalHash::from_bytes(result)
}

fn offer_mac_state(secret: &OfferSecret, binding: CanonicalHash) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret.key)
        .expect("HMAC-SHA256 accepts a fixed 32-byte key");
    mac.update(OFFER_HMAC_DOMAIN);
    mac.update(&secret.version.to_be_bytes());
    mac.update(binding.as_bytes());
    mac
}

/// Invalid durable offer key ring.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum OfferSecretError {
    #[error("offer secret version zero is reserved")]
    ZeroVersion,
    #[error("an all-zero offer secret is not accepted")]
    ZeroKey,
    #[error("retained offer-secret versions must precede the current version")]
    NonMonotonicVersion,
    #[error("offer secret version appears more than once")]
    DuplicateVersion,
    #[error("offer key ring exceeds its hard limit of {max} versions")]
    VersionLimit { max: usize },
}

/// Trusted provider input for one internal offer.
pub struct InternalOfferDraft {
    pub action: DefinitionRef,
    pub viewer_scope: ProjectionScopeId,
    pub truth_subject: SubjectKey,
    pub projected_subject: PerceivedRef,
    pub fixed_parameters: Value,
    pub parameters: ProjectedParameterSchema,
    pub targets: ProjectedTargetSchema,
    pub required_channels: Vec<ChannelId>,
    pub expires: Option<ClockStamp>,
    pub presentation: ActionPresentation,
    pub guards: GuardSet,
}

/// Provider-owned portion of one offer. The frozen registry supplies the
/// exact action/provider identity and the server query supplies both subject
/// identities and projection scope, so a provider cannot redirect an offer to
/// a different registered route.
pub struct ProviderOfferDraft {
    pub fixed_parameters: Value,
    pub parameters: ProjectedParameterSchema,
    pub targets: ProjectedTargetSchema,
    pub required_channels: Vec<ChannelId>,
    pub expires: Option<ClockStamp>,
    pub presentation: ActionPresentation,
    pub guards: GuardSet,
}

/// Server-only legal action. It is deliberately not serializable.
#[derive(Clone, Debug)]
pub struct InternalOffer {
    offer: ActionOfferId,
    binding_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    viewer_scope: ProjectionScopeId,
    truth_subject: SubjectKey,
    fixed_parameters: Value,
    projected: ProjectedLegalAction,
    guards: GuardSet,
}

impl InternalOffer {
    pub fn new(draft: InternalOfferDraft, keys: &OfferKeyRing) -> Result<Self, OfferBuildError> {
        if draft.action.key().kind.as_str() != "action" {
            return Err(OfferBuildError::DefinitionKind);
        }
        if !draft
            .guards
            .authority()
            .iter()
            .any(|guard| matches!(guard, crate::action::AuthorityGuard::Definition(reference) if reference == &draft.action))
        {
            return Err(OfferBuildError::MissingDefinitionGuard);
        }
        let fixed = draft
            .fixed_parameters
            .as_object()
            .ok_or(OfferBuildError::FixedParametersNotObject)?;
        canonical_json_bytes_with_limit(
            &draft.fixed_parameters,
            crate::action::MAX_ACTION_PARAMETER_BYTES,
        )?;
        for parameter in draft.parameters.parameters() {
            if fixed.contains_key(parameter.id.as_str()) {
                return Err(OfferBuildError::ParameterCollision {
                    id: parameter.id.clone(),
                });
            }
        }
        draft.targets.validate_shape()?;
        if draft.required_channels.len() > MAX_PROJECTED_ACTION_CHANNELS {
            return Err(OfferBuildError::ChannelLimit {
                actual: draft.required_channels.len(),
                max: MAX_PROJECTED_ACTION_CHANNELS,
            });
        }
        let mut required_channels = draft.required_channels;
        required_channels.sort();
        if required_channels.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(OfferBuildError::DuplicateChannel);
        }
        let provider = draft.action.key().owner.clone();
        let guard_hash = draft.guards.canonical_hash()?;
        let binding_hash = CanonicalHash::for_canonical_value(
            "internal_offer_binding.v1",
            1,
            &json!({
                "action": draft.action,
                "provider": provider,
                "viewer_scope": draft.viewer_scope.0,
                "truth_subject": draft.truth_subject,
                "projected_subject": draft.projected_subject,
                "fixed_parameters": draft.fixed_parameters,
                "parameters": draft.parameters,
                "targets": draft.targets,
                "required_channels": required_channels,
                "expires": draft.expires,
                "presentation": draft.presentation,
                "guard_hash": guard_hash,
            }),
            MAX_PROJECTED_LEGAL_ACTION_BYTES + crate::action::MAX_GUARD_SET_BYTES,
        )?;
        let offer = keys.sign(binding_hash);
        let projected = ProjectedLegalAction {
            offer,
            subject: draft.projected_subject,
            parameters: draft.parameters,
            targets: draft.targets,
            required_channels,
            expires: draft.expires,
            presentation: draft.presentation,
        };
        let projected_value =
            serde_json::to_value(&projected).map_err(|error| OfferBuildError::Serialization {
                message: error.to_string(),
            })?;
        canonical_json_bytes_with_limit(&projected_value, MAX_PROJECTED_LEGAL_ACTION_BYTES)?;
        Ok(Self {
            offer,
            binding_hash,
            action: draft.action,
            provider,
            viewer_scope: draft.viewer_scope,
            truth_subject: draft.truth_subject,
            fixed_parameters: draft.fixed_parameters,
            projected,
            guards: draft.guards,
        })
    }

    #[must_use]
    pub const fn offer(&self) -> ActionOfferId {
        self.offer
    }

    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn provider(&self) -> &OwnerId {
        &self.provider
    }

    #[must_use]
    pub const fn truth_subject(&self) -> &SubjectKey {
        &self.truth_subject
    }

    #[must_use]
    pub const fn projected(&self) -> &ProjectedLegalAction {
        &self.projected
    }
}

/// Bounded collection built by one projection pass. Canonical action order is
/// independent of provider enumeration or hash-map iteration order.
#[derive(Clone, Debug)]
pub struct ProjectedLegalActionSink {
    viewer_scope: ProjectionScopeId,
    offers: BTreeMap<DefinitionRef, InternalOffer>,
    projected_bytes: usize,
}

impl ProjectedLegalActionSink {
    #[must_use]
    pub const fn new(viewer_scope: ProjectionScopeId) -> Self {
        Self {
            viewer_scope,
            offers: BTreeMap::new(),
            projected_bytes: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.offers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offers.is_empty()
    }

    pub fn push(&mut self, offer: InternalOffer) -> Result<(), OfferBuildError> {
        if offer.viewer_scope != self.viewer_scope {
            return Err(OfferBuildError::ProjectionScopeMismatch);
        }
        if self.offers.len() >= MAX_PROJECTED_LEGAL_ACTIONS {
            return Err(OfferBuildError::OfferLimit {
                max: MAX_PROJECTED_LEGAL_ACTIONS,
            });
        }
        if self.offers.contains_key(&offer.action) {
            return Err(OfferBuildError::DuplicateAction);
        }
        if self
            .offers
            .values()
            .any(|existing| existing.offer == offer.offer)
        {
            return Err(OfferBuildError::DuplicateBinding);
        }
        let projected = serde_json::to_value(offer.projected()).map_err(|error| {
            OfferBuildError::Serialization {
                message: error.to_string(),
            }
        })?;
        let bytes =
            canonical_json_bytes_with_limit(&projected, MAX_PROJECTED_LEGAL_ACTION_BYTES)?.len();
        let total = self
            .projected_bytes
            .checked_add(bytes)
            .ok_or(OfferBuildError::ArithmeticOverflow)?;
        if total > MAX_PROJECTED_LEGAL_ACTION_SET_BYTES {
            return Err(OfferBuildError::OfferSetBytes {
                attempted: total,
                max: MAX_PROJECTED_LEGAL_ACTION_SET_BYTES,
            });
        }
        self.projected_bytes = total;
        self.offers.insert(offer.action.clone(), offer);
        Ok(())
    }

    #[must_use]
    pub fn projected(&self) -> Vec<ProjectedLegalAction> {
        self.offers
            .values()
            .map(|offer| offer.projected.clone())
            .collect()
    }

    /// Resolve one opaque submission without distinguishing unknown, expired,
    /// changed, or wrong-viewer offers to the caller.
    pub fn resolve(
        &self,
        keys: &OfferKeyRing,
        execution: OfferExecutionContext,
        submission: OfferSubmission,
    ) -> Result<ResolvedOffer, OfferResolveError> {
        let mut matched = None;
        for offer in self.offers.values() {
            if keys.matches(submission.offer, offer.binding_hash) {
                if matched.is_some() {
                    return Err(OfferResolveError::AmbiguousBinding);
                }
                matched = Some(offer);
            }
        }
        let offer = matched.ok_or(OfferResolveError::Unavailable)?;
        if execution.viewer_scope != self.viewer_scope
            || execution.viewer_scope != offer.viewer_scope
            || execution.idempotency.scope().owner != offer.provider
        {
            return Err(OfferResolveError::Unavailable);
        }
        if submission.subject != offer.projected.subject {
            return Err(OfferResolveError::Unavailable);
        }
        if let Some(expires) = &offer.projected.expires {
            let now = execution
                .now
                .as_ref()
                .ok_or(OfferResolveError::Unavailable)?;
            if !matches!(
                now.checked_cmp(expires)
                    .map_err(|_| OfferResolveError::Unavailable)?,
                std::cmp::Ordering::Less
            ) {
                return Err(OfferResolveError::Unavailable);
            }
        }
        offer
            .projected
            .parameters
            .validate(&submission.parameters)?;
        offer
            .projected
            .targets
            .validate_submission(&submission.targets)?;
        let parameters = merge_parameters(&offer.fixed_parameters, submission.parameters)?;
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: execution.request,
            principal: execution.principal,
            actor: execution.actor,
            offered_action: submission.offer,
            subject: submission.subject,
            targets: submission.targets,
            parameters,
            idempotency: execution.idempotency,
        })?;
        Ok(ResolvedOffer {
            action: offer.action.clone(),
            invocation,
            guards: offer.guards.clone(),
        })
    }
}

fn merge_parameters(fixed: &Value, public: Value) -> Result<Value, OfferResolveError> {
    let mut merged = fixed
        .as_object()
        .expect("internal offer construction validates fixed parameters")
        .clone();
    let public = public
        .as_object()
        .ok_or(OfferResolveError::InvalidParameters)?;
    for (key, value) in public {
        if merged.insert(key.clone(), value.clone()).is_some() {
            return Err(OfferResolveError::InvalidParameters);
        }
    }
    Ok(Value::Object(merged))
}

/// Authenticated, server-created fields used to turn an offer into an action.
pub struct OfferExecutionContext {
    pub request: RequestUid,
    pub principal: PrincipalId,
    pub actor: ObjId,
    pub idempotency: ScopedIdempotencyKey,
    pub viewer_scope: ProjectionScopeId,
    pub now: Option<ClockStamp>,
}

/// Untrusted fields supplied by any human, UI, NPC, or LLM caller.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferSubmission {
    pub offer: ActionOfferId,
    pub subject: PerceivedRef,
    pub targets: Vec<PerceivedRef>,
    pub parameters: Value,
}

/// Exact provider input after successful opaque-offer resolution.
#[derive(Debug)]
pub struct ResolvedOffer {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
}

impl ResolvedOffer {
    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn invocation(&self) -> &ActionInvocation {
        &self.invocation
    }

    #[must_use]
    pub const fn guards(&self) -> &GuardSet {
        &self.guards
    }

    #[must_use]
    pub fn into_parts(self) -> (DefinitionRef, ActionInvocation, GuardSet) {
        (self.action, self.invocation, self.guards)
    }
}

/// Invalid trusted offer construction.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum OfferBuildError {
    #[error("legal offer definition is not an action")]
    DefinitionKind,
    #[error("legal offer does not carry its exact action-definition guard")]
    MissingDefinitionGuard,
    #[error("fixed offer parameters must be an object")]
    FixedParametersNotObject,
    #[error("projected parameter {id} collides with a server-fixed parameter")]
    ParameterCollision { id: CanonicalId },
    #[error("offer projects {actual} parameters; maximum is {max}")]
    ParameterLimit { actual: usize, max: usize },
    #[error("offer projects the same parameter more than once")]
    DuplicateParameter,
    #[error("integer parameter bounds are inverted")]
    InvalidParameterBounds,
    #[error("text parameter limit {actual} is outside 1..={max}")]
    InvalidTextLimit { actual: usize, max: usize },
    #[error("choice parameter has {actual} choices; allowed range is 1..={max}")]
    ChoiceLimit { actual: usize, max: usize },
    #[error("choice value is empty or exceeds its text bound")]
    InvalidChoice,
    #[error("choice parameter contains a duplicate")]
    DuplicateChoice,
    #[error("target selection bounds or choice count are invalid")]
    InvalidTargetBounds,
    #[error("target selector contains a duplicate perceived reference")]
    DuplicateTargetChoice,
    #[error("offer requires {actual} channels; maximum is {max}")]
    ChannelLimit { actual: usize, max: usize },
    #[error("offer requires the same channel more than once")]
    DuplicateChannel,
    #[error("action presentation verb and label must be non-empty")]
    EmptyPresentation,
    #[error("action presentation exceeds its {max}-byte field bound")]
    PresentationTooLarge { max: usize },
    #[error("offer belongs to another projection scope")]
    ProjectionScopeMismatch,
    #[error("an action was projected more than once")]
    DuplicateAction,
    #[error("two projected actions have the same opaque binding")]
    DuplicateBinding,
    #[error("projected action count exceeds its hard limit of {max}")]
    OfferLimit { max: usize },
    #[error("projected action set is {attempted} bytes; maximum is {max}")]
    OfferSetBytes { attempted: usize, max: usize },
    #[error("offer byte accounting overflow")]
    ArithmeticOverflow,
    #[error("canonical offer data is invalid: {0}")]
    Canonical(#[from] CanonicalEncodingError),
    #[error("offer binding hash is invalid: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("projected offer serialization failed: {message}")]
    Serialization { message: String },
}

/// Uniform projected refusal while resolving an opaque offer.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum OfferResolveError {
    #[error("offered action is stale or unavailable")]
    Unavailable,
    #[error("offered action parameters are invalid")]
    InvalidParameters,
    #[error("offered action targets are invalid")]
    InvalidTargets,
    #[error("offer binding matched more than one internal action")]
    AmbiguousBinding,
    #[error("canonical action invocation is invalid: {0}")]
    Invocation(#[from] ActionMetadataError),
}

/// Engine service failure while resolving an untrusted offer submission.
#[derive(Debug, Error)]
pub enum LegalActionResolutionError {
    #[error("durable legal-action offer keys are not installed")]
    KeyRingUnavailable,
    #[error(transparent)]
    Resolve(#[from] OfferResolveError),
}

/// Failure while enumerating the bounded canonical offer set.
#[derive(Debug, Error)]
pub enum LegalActionProjectionError {
    #[error("durable legal-action offer keys are not installed")]
    KeyRingUnavailable,
    #[error("action-provider registry is unavailable")]
    RegistryUnavailable,
    #[error("provider {action} refused legal-action projection: {source}")]
    Provider {
        action: Box<DefinitionRef>,
        source: Box<ActionError>,
    },
    #[error("provider owner became unavailable while projecting {action}")]
    ProviderUnavailable { action: Box<DefinitionRef> },
    #[error("provider {action} produced an invalid offer: {source}")]
    Offer {
        action: Box<DefinitionRef>,
        source: Box<OfferBuildError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{IdempotencyKey, IdempotencyScope, RevisionSet};
    use crate::clock::ClockRef;
    use crate::mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
        MechanicalOwnerReadiness,
    };

    fn definition() -> (DefinitionRef, crate::mechanics::DefinitionRegistry) {
        let owner = OwnerId::new("toy").unwrap();
        let mut builder = DefinitionRegistryBuilder::new();
        let action = builder
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("brace").unwrap(),
                    1,
                    json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let owners = BTreeSet::from([owner]);
        let readiness = MechanicalOwnerReadiness::for_owners(&owners);
        let mut registry = builder.freeze().unwrap();
        registry.bind_owner_readiness(readiness);
        (action, registry)
    }

    fn principal() -> PrincipalId {
        PrincipalId::Object {
            id: crate::obj::ObjId(1),
        }
    }

    fn scope() -> ProjectionScopeId {
        ProjectionScopeId::derive(&principal(), 7).unwrap()
    }

    fn keys() -> OfferKeyRing {
        OfferKeyRing::new(OfferSecret::new(2, [2; 32]).unwrap(), []).unwrap()
    }

    fn guards(action: &DefinitionRef, registry: &crate::mechanics::DefinitionRegistry) -> GuardSet {
        let readiness = registry.owner_readiness().unwrap().clone();
        let mut builder =
            crate::action::ServerGuardSetBuilder::new(RevisionSet::default(), readiness);
        builder.add_definition(action.clone(), registry).unwrap();
        builder.finish().unwrap()
    }

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::SceneTurn {
                scene: 1,
                generation: 3,
            },
            at,
        }
    }

    fn internal_offer(keys: &OfferKeyRing, viewer_scope: ProjectionScopeId) -> InternalOffer {
        let (action, registry) = definition();
        InternalOffer::new(
            InternalOfferDraft {
                action: action.clone(),
                viewer_scope,
                truth_subject: SubjectKey::Object {
                    id: crate::obj::ObjId(1),
                },
                projected_subject: PerceivedRef::SelfActor,
                fixed_parameters: json!({ "scene_turn": stamp(11) }),
                parameters: ProjectedParameterSchema::empty(),
                targets: ProjectedTargetSchema::None,
                required_channels: Vec::new(),
                expires: None,
                presentation: ActionPresentation::new(
                    "brace",
                    "Brace",
                    "Spend one focus to become guarded.",
                )
                .unwrap(),
                guards: guards(&action, &registry),
            },
            keys,
        )
        .unwrap()
    }

    fn execution(viewer_scope: ProjectionScopeId) -> OfferExecutionContext {
        let request = RequestUid::from_bytes([9; 16]);
        OfferExecutionContext {
            request,
            principal: principal(),
            actor: crate::obj::ObjId(1),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal(),
                    owner: OwnerId::new("toy").unwrap(),
                    namespace: CanonicalId::new("brace").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            viewer_scope,
            now: None,
        }
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = OfferSecret::new(1, [0xAB; 32]).unwrap();
        let debug = format!("{secret:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("171"));
    }

    #[test]
    fn projected_offer_hides_internal_binding_and_builds_server_parameters() {
        let keys = keys();
        let offer = internal_offer(&keys, scope());
        let projected = serde_json::to_value(offer.projected()).unwrap();
        let encoded = projected.to_string();
        assert!(!encoded.contains("scene_turn"));
        assert!(!encoded.contains("truth_subject"));
        assert!(!encoded.contains("guard_hash"));
        assert!(!encoded.contains("\"guards\""));
        assert!(!encoded.contains("provider"));
        assert!(!encoded.contains(offer.action().digest().to_string().as_str()));

        let token = offer.offer();
        let mut sink = ProjectedLegalActionSink::new(scope());
        sink.push(offer).unwrap();
        let resolved = sink
            .resolve(
                &keys,
                execution(scope()),
                OfferSubmission {
                    offer: token,
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: json!({}),
                },
            )
            .unwrap();
        assert_eq!(resolved.action().key().id.as_str(), "brace");
        assert_eq!(
            serde_json::from_value::<ClockStamp>(
                resolved.invocation().parameters()["scene_turn"].clone()
            )
            .unwrap(),
            stamp(11)
        );
    }

    #[test]
    fn retained_secret_accepts_an_unchanged_pre_rotation_offer() {
        let old_secret = OfferSecret::new(2, [2; 32]).unwrap();
        let old_keys = OfferKeyRing::new(old_secret.clone(), []).unwrap();
        let old_token = internal_offer(&old_keys, scope()).offer();

        let rotated =
            OfferKeyRing::new(OfferSecret::new(3, [3; 32]).unwrap(), [old_secret]).unwrap();
        let mut regenerated = ProjectedLegalActionSink::new(scope());
        regenerated.push(internal_offer(&rotated, scope())).unwrap();
        assert!(
            regenerated
                .resolve(
                    &rotated,
                    execution(scope()),
                    OfferSubmission {
                        offer: old_token,
                        subject: PerceivedRef::SelfActor,
                        targets: Vec::new(),
                        parameters: json!({}),
                    },
                )
                .is_ok()
        );

        let without_old = OfferKeyRing::new(OfferSecret::new(3, [3; 32]).unwrap(), []).unwrap();
        assert_eq!(
            regenerated
                .resolve(
                    &without_old,
                    execution(scope()),
                    OfferSubmission {
                        offer: old_token,
                        subject: PerceivedRef::SelfActor,
                        targets: Vec::new(),
                        parameters: json!({}),
                    },
                )
                .unwrap_err(),
            OfferResolveError::Unavailable
        );
    }

    #[test]
    fn changed_scope_or_public_input_cannot_reuse_an_offer() {
        let keys = keys();
        let offer = internal_offer(&keys, scope());
        let token = offer.offer();
        let mut sink = ProjectedLegalActionSink::new(scope());
        sink.push(offer).unwrap();
        let other_scope = ProjectionScopeId::derive(&principal(), 8).unwrap();
        assert_eq!(
            sink.resolve(
                &keys,
                execution(other_scope),
                OfferSubmission {
                    offer: token,
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: json!({}),
                },
            )
            .unwrap_err(),
            OfferResolveError::Unavailable
        );
        assert_eq!(
            sink.resolve(
                &keys,
                execution(scope()),
                OfferSubmission {
                    offer: token,
                    subject: PerceivedRef::SelfActor,
                    targets: Vec::new(),
                    parameters: json!({ "scene_turn": stamp(99) }),
                },
            )
            .unwrap_err(),
            OfferResolveError::InvalidParameters
        );
    }

    #[test]
    fn sink_rejects_duplicate_actions() {
        let keys = keys();
        let mut sink = ProjectedLegalActionSink::new(scope());
        sink.push(internal_offer(&keys, scope())).unwrap();
        assert_eq!(
            sink.push(internal_offer(&keys, scope())).unwrap_err(),
            OfferBuildError::DuplicateAction
        );
    }
}
