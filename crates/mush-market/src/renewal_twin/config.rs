//! Explicit personal Pin renewal composition, of either issuer. Prices and licence terms
//! always come from the immutable acquired template, never this host fee policy.
//!
//! Each family keeps its own constructor (`RenewalConfig::new`), in its `mod.rs`: the
//! personal issuer's takes the manifest and participants, the House's the House purchase
//! configuration they come from.
use super::Issuer;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError};
use mush_economy::managed_entitlement::GrossRecipientCut;
use mush_roll::commerce::{
    purchase::PurchaseParticipants,
    renewal_twin::{protocol, protocol::Step, writer::RenewalWriter},
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RenewalPolicy {
    pub registry: GrossRecipientCut,
    pub venue: Option<GrossRecipientCut>,
    pub offer_ic_seconds: i64,
}
impl RenewalPolicy {
    pub fn validate(&self) -> Result<(), ModError> {
        let cut = |c: &GrossRecipientCut| {
            c.permille <= 1000 && (c.permille == 0) == c.recipient.is_none()
        };
        if !cut(&self.registry)
            || self.venue.as_ref().is_some_and(|c| !cut(c))
            || u32::from(self.registry.permille)
                + u32::from(self.venue.as_ref().map_or(0, |c| c.permille))
                > 1000
            || !(1..=mush_economy::renewal_twin::MAX_RENEWAL_OFFER_IC_SECONDS)
                .contains(&self.offer_ic_seconds)
        {
            return Err(error("invalid renewal policy"));
        }
        Ok(())
    }
}
#[expect(
    private_bounds,
    reason = "doc 46 CT.6: the market's issuer is private to the crate, as the families it names are"
)]
#[derive(Clone, Debug)]
pub struct RenewalConfig<I: Issuer> {
    pub(crate) manifest: DefinitionRef,
    pub(crate) participants: PurchaseParticipants,
    pub(crate) writer: RenewalWriter<I>,
    pub(crate) policy: RenewalPolicy,
    pub(crate) purchase: I::Purchase,
    pub(crate) service: Option<super::service::ServiceConfig<I>>,
}
pub(crate) fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn descriptor(
    kind: &str,
    id: &str,
    payload: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        payload,
    )
    .map_err(error)
}
#[expect(
    private_bounds,
    reason = "doc 46 CT.6: the market's issuer is private to the crate, as the families it names are"
)]
impl<I: Issuer> RenewalConfig<I> {
    /// The family constructors' one body, after each has validated the policy.
    pub(crate) fn of(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        writer: RenewalWriter<I>,
        policy: RenewalPolicy,
        purchase: I::Purchase,
    ) -> Result<Self, ModError> {
        let value = Self {
            manifest,
            participants,
            writer,
            policy,
            purchase,
            service: None,
        };
        value.actions()?;
        Ok(value)
    }
    /// Explicit service opt-in. This never authorizes automatic acceptance.
    pub fn with_service(mut self) -> Result<Self, ModError> {
        if self.service.is_some() {
            return Err(error("renewal service already configured"));
        }
        self.service = Some(super::service::ServiceConfig::default());
        self.service_action()?;
        Ok(self)
    }
    /// 17g C2h (the lead's ruling R3): name the module the service's player
    /// front (`front`) is delegated to at composition, the module whose
    /// `+minds/renew` hands the actor's own step to the service. Only with the
    /// service configured ([`Self::with_service`]); without it there is no
    /// front and this is a no-op. Set only by the composition that composes
    /// the player market; it enters no definition descriptor.
    #[must_use]
    pub fn with_player_front(mut self, grantee: OwnerId) -> Self {
        if let Some(service) = self.service.as_mut() {
            service.player_front = Some(grantee);
        }
        self
    }
    /// 17g C2h: the module the player front is delegated to, if any.
    #[must_use]
    pub fn player_front(&self) -> Option<&OwnerId> {
        self.service.as_ref().and_then(|s| s.player_front.as_ref())
    }
    pub fn service_action(&self) -> Result<Option<DefinitionRef>, ModError> {
        self.service
            .as_ref()
            .map(|_| protocol::service_reference::<I>(&self.manifest, &self.participants))
            .transpose()
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub fn writer(&self) -> RenewalWriter<I> {
        self.writer.clone()
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 5], ModError> {
        let action = |t| {
            DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new(crate::NS).map_err(error)?,
                    protocol::descriptor::<I>(t, &self.manifest, &self.participants)?,
                )
                .map_err(error)
        };
        let refs = [
            action(Step::Draft)?,
            action(Step::Fund)?,
            action(Step::Grant)?,
            action(Step::Refund)?,
            action(Step::Close)?,
        ];
        Ok(refs)
    }
    pub(crate) fn policy_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(descriptor(
            I::POLICY_KIND,
            "personal",
            serde_json::to_value(&self.policy).map_err(error)?,
        )?
        .with_references(
            std::iter::once(self.manifest.clone())
                .chain(I::purchase_references(&self.purchase))
                .collect(),
        ))
    }
    pub(crate) fn policy_ref(&self) -> Result<DefinitionRef, ModError> {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::NS).map_err(error)?,
                self.policy_descriptor()?,
            )
            .map_err(error)
    }
    pub(crate) fn outcome_descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        descriptor(
            "outcome",
            I::OUTCOME_ID,
            serde_json::json!({"format":1,"fields":["kind","operation","commit"]}),
        )
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut all = vec![self.outcome_descriptor()?];
        for t in [
            Step::Draft,
            Step::Fund,
            Step::Grant,
            Step::Refund,
            Step::Close,
        ] {
            all.push(protocol::descriptor::<I>(
                t,
                &self.manifest,
                &self.participants,
            )?);
        }
        if self.service.is_some() {
            all.push(protocol::service_descriptor::<I>(
                &self.manifest,
                &self.participants,
            )?);
        }
        Ok(all)
    }
}
