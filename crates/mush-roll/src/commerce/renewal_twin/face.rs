//! The writer's public face (doc 46 CT.6).
//!
//! The writer's methods sit in impl blocks bound on the issuer's rules (`Rules`), which is private to the crate,
//! so code in another crate that is generic over an [`Issuer`] could not call them. [`Writes`]
//! is the same methods over the same public types, implemented once for every issuer that has
//! the rules, which is exactly Economy's `Personal` and `House`: the rules are sealed by
//! [`Issuer`]'s seal. Each method is the inherent method of the same name and does nothing
//! else.
use super::super::{LicenceId, purchase::PurchaseParticipants, records::PublicationError};
use super::records::RenewalAttemptV1;
use super::writer::{PreparedRenewal, RegisteredRenewalWriter, RenewalWriter};
use super::{Issuer, Rules};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionTxn, CommitToken, DefinitionRef, ModError, ModuleCtx, PrepareCtx, Registrar,
};
use mush_economy::renewal_twin::{RenewalFundingOutput, RenewalOfferOutput, RenewalTerminalOutput};

/// Economy's offer of an issuer.
type EconomyOffer<I> = <I as mush_economy::renewal_twin::Issuer>::Offer;

/// The writer's methods, at an issuer named only by its public trait.
pub trait Writes: Issuer {
    /// [`RenewalWriter::operations_for_boot`].
    fn operations_for_boot(
        writer: &RenewalWriter<Self>,
        ctx: &ModuleCtx<'_>,
    ) -> Result<Vec<Self::Operation>, PublicationError>;
    /// [`RenewalWriter::operation_for_boot`].
    fn operation_for_boot(
        writer: &RenewalWriter<Self>,
        ctx: &ModuleCtx<'_>,
        operation: CommitToken,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RenewalWriter::validate_registration`].
    fn validate_registration(
        writer: &RenewalWriter<Self>,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 5],
    ) -> Result<(), ModError>;
    /// [`RenewalWriter::validate_service_registration`].
    fn validate_service_registration(
        writer: &RenewalWriter<Self>,
        service: &DefinitionRef,
    ) -> Result<(), ModError>;
    /// [`RenewalWriter::for_action`].
    fn for_action(
        writer: &RenewalWriter<Self>,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        reader: OwnerReadHandle<Self::Reader>,
    ) -> Result<RegisteredRenewalWriter<Self>, ModError>;
    /// [`RegisteredRenewalWriter::observe`].
    fn observe(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<PreparedRenewal<Self>, PublicationError>;
    /// [`RegisteredRenewalWriter::operation`].
    fn operation(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RegisteredRenewalWriter::preflight`].
    fn preflight(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedRenewal<Self>,
    ) -> Result<(), PublicationError>;
    /// [`RegisteredRenewalWriter::draft`].
    fn draft(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        offer: &EconomyOffer<Self>,
        attempt: RenewalAttemptV1,
        observation: &PreparedRenewal<Self>,
        output: &RenewalOfferOutput<Self>,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RegisteredRenewalWriter::fund`].
    fn fund(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &Self::Operation,
        offer: &EconomyOffer<Self>,
        observation: &PreparedRenewal<Self>,
        output: &RenewalFundingOutput<Self>,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RegisteredRenewalWriter::terminal`].
    fn terminal(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &Self::Operation,
        offer: &EconomyOffer<Self>,
        observation: Option<&PreparedRenewal<Self>>,
        output: &RenewalTerminalOutput<Self>,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RegisteredRenewalWriter::close`].
    fn close(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &Self::Operation,
        output: &RenewalOfferOutput<Self>,
    ) -> Result<Self::Operation, PublicationError>;
    /// [`RegisteredRenewalWriter::service_close`].
    fn service_close(
        writer: &RegisteredRenewalWriter<Self>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &Self::Operation,
        output: &RenewalOfferOutput<Self>,
    ) -> Result<Self::Operation, PublicationError>;
}

impl<I: Rules> Writes for I {
    fn operations_for_boot(
        writer: &RenewalWriter<I>,
        ctx: &ModuleCtx<'_>,
    ) -> Result<Vec<I::Operation>, PublicationError> {
        writer.operations_for_boot(ctx)
    }
    fn operation_for_boot(
        writer: &RenewalWriter<I>,
        ctx: &ModuleCtx<'_>,
        operation: CommitToken,
    ) -> Result<I::Operation, PublicationError> {
        writer.operation_for_boot(ctx, operation)
    }
    fn validate_registration(
        writer: &RenewalWriter<I>,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 5],
    ) -> Result<(), ModError> {
        writer.validate_registration(manifest, participants, actions)
    }
    fn validate_service_registration(
        writer: &RenewalWriter<I>,
        service: &DefinitionRef,
    ) -> Result<(), ModError> {
        writer.validate_service_registration(service)
    }
    fn for_action(
        writer: &RenewalWriter<I>,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        reader: OwnerReadHandle<I::Reader>,
    ) -> Result<RegisteredRenewalWriter<I>, ModError> {
        writer.for_action(reg, action, manifest, participants, reader)
    }
    fn observe(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<PreparedRenewal<I>, PublicationError> {
        writer.observe(ctx, txn, licence)
    }
    fn operation(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<I::Operation, PublicationError> {
        writer.operation(ctx, txn, operation)
    }
    fn preflight(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedRenewal<I>,
    ) -> Result<(), PublicationError> {
        writer.preflight(ctx, txn, observation)
    }
    fn draft(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        offer: &EconomyOffer<I>,
        attempt: RenewalAttemptV1,
        observation: &PreparedRenewal<I>,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        writer.draft(ctx, txn, offer, attempt, observation, output)
    }
    fn fund(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        offer: &EconomyOffer<I>,
        observation: &PreparedRenewal<I>,
        output: &RenewalFundingOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        writer.fund(ctx, txn, before, offer, observation, output)
    }
    fn terminal(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        offer: &EconomyOffer<I>,
        observation: Option<&PreparedRenewal<I>>,
        output: &RenewalTerminalOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        writer.terminal(ctx, txn, before, offer, observation, output)
    }
    fn close(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        writer.close(ctx, txn, before, output)
    }
    fn service_close(
        writer: &RegisteredRenewalWriter<I>,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &I::Operation,
        output: &RenewalOfferOutput<I>,
    ) -> Result<I::Operation, PublicationError> {
        writer.service_close(ctx, txn, before, output)
    }
}
