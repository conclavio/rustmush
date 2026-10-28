//! Seller offers and named-buyer acceptance. Service terminals have a separate
//! retained-intent entry point; no public parameter can select that authority.
use super::{
    PaidResaleConfig, accounts,
    families::{Families, envelope},
    records::{MindPaidResaleOperation, MindPaidResaleReceipt},
};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ObjId, PrepareCtx, RulesValue};
use mush_economy::managed_paid_resale::{
    PaidResaleAcceptanceRequest, PaidResaleClaimsV1, PaidResaleModeV1,
};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_economy::{AccountId, Amount};
use mush_roll::commerce::{
    LicenceId,
    paid_resale::{reader::PaidResaleRequest, writer::RegisteredPaidResaleWriter},
};
use serde::Deserialize;
fn invalid(_: impl std::fmt::Display) -> String {
    "paid resale is unavailable or has changed".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Offer,
    Accept,
    Fund,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferParameters {
    licence: u64,
    buyer: u64,
    price: i64,
    seller_account: AccountId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptParameters {
    offer: CommitToken,
    payer: AccountId,
}
pub(super) enum Request {
    Offer {
        licence: LicenceId,
        buyer: ObjId,
        price: Amount,
        seller_account: AccountId,
    },
    Accept {
        offer: CommitToken,
        payer: AccountId,
    },
}
pub(super) fn decode(kind: Kind, value: &serde_json::Value) -> Result<Request, String> {
    match kind {
        Kind::Offer => {
            let v: OfferParameters = serde_json::from_value(value.clone()).map_err(invalid)?;
            if v.licence > i64::MAX as u64
                || v.buyer == 0
                || v.buyer > u32::MAX as u64
                || v.price <= 0
                || v.seller_account.0 > i64::MAX as u64
            {
                return Err(invalid("selector"));
            }
            Ok(Request::Offer {
                licence: LicenceId::new(v.licence).map_err(invalid)?,
                buyer: ObjId(v.buyer as u32),
                price: Amount::from_minor(v.price),
                seller_account: v.seller_account,
            })
        }
        Kind::Accept | Kind::Fund => {
            let v: AcceptParameters = serde_json::from_value(value.clone()).map_err(invalid)?;
            if v.payer.0 > i64::MAX as u64 {
                return Err(invalid("payer"));
            }
            Ok(Request::Accept {
                offer: v.offer,
                payer: v.payer,
            })
        }
    }
}
pub(super) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub kind: Kind,
    pub config: PaidResaleConfig,
    pub families: Families,
    pub legacy: Option<crate::mind_trade::PurchaseFamilies>,
    pub economy: Option<ParticipantHandle<InitialFilingParticipant>>,
    pub writer: RegisteredPaidResaleWriter,
    pub refusal: ActionError,
}
impl Provider {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(invalid("invocation"));
        }
        decode(self.kind, inv.parameters())
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let (operation, label) = match self.request(inv)? {
            Request::Offer {
                licence,
                buyer,
                price,
                seller_account,
            } => {
                let observed = self
                    .writer
                    .observe(ctx, txn, PaidResaleRequest::Seller { licence, buyer })
                    .map_err(invalid)?;
                let now = ctx.world().map_err(invalid)?.now();
                self.writer
                    .preflight(ctx, txn, &observed)
                    .map_err(invalid)?;
                let row = super::offer::build(
                    ctx.world().map_err(invalid)?,
                    txn,
                    self.legacy.as_ref(),
                    observed.facts(),
                    commit,
                    price,
                    seller_account,
                    self.config.offer_seconds,
                )?;
                envelope(
                    &self.families.offers,
                    commit,
                    serde_json::to_value(&row).map_err(invalid)?,
                )?;
                txn.insert(&self.families.offers, &commit, row.clone())
                    .map_err(invalid)?;
                if txn.get(&self.families.offers, &commit).map_err(invalid)? != Some(&row) {
                    return Err(invalid("offer readback"));
                }
                // 17g C2f: the named buyer's offer index, in this transaction.
                super::index::offered(txn, &self.families, &row, now).map_err(invalid)?;
                (commit, "offered")
            }
            Request::Accept { offer, payer } => {
                let row = txn
                    .get(&self.families.offers, &offer)
                    .map_err(invalid)?
                    .cloned()
                    .ok_or_else(|| invalid("offer missing"))?;
                row.validate()?;
                if row.operation() != offer || row.terms().buyer != inv.actor() {
                    return Err(invalid("named buyer"));
                }
                let observed = self
                    .writer
                    .observe(
                        ctx,
                        txn,
                        PaidResaleRequest::Buyer {
                            licence: row.binding().licence(),
                            seller: row.terms().seller,
                        },
                    )
                    .map_err(invalid)?;
                if observed.facts().binding() != row.binding() {
                    return Err(invalid("captured property differs"));
                }
                let immediate = self.kind == Kind::Accept;
                if immediate != (observed.facts().claims().open_paid() == 0) {
                    return Err(invalid("claims require other acceptance path"));
                }
                self.writer
                    .preflight(ctx, txn, &observed)
                    .map_err(invalid)?;
                preflight_market(&self.families)?;
                let world = ctx.world().map_err(invalid)?;
                accounts::personal(
                    world,
                    accounts::live(world)?,
                    &row.terms().instrument,
                    inv.actor(),
                    payer,
                )?;
                let output = ctx
                    .prepare_owner(
                        self.economy
                            .as_ref()
                            .ok_or_else(|| invalid("economy grant absent"))?,
                        EconomyRequest::PaidResaleAcceptance(Box::new(
                            PaidResaleAcceptanceRequest {
                                terms: row.terms().clone(),
                                offer_digest: row.digest()?,
                                payer,
                                claims: PaidResaleClaimsV1 {
                                    next_transition: observed.facts().claims().next_transition(),
                                    open_paid: observed.facts().claims().open_paid(),
                                },
                                mode: if immediate {
                                    PaidResaleModeV1::Immediate
                                } else {
                                    PaidResaleModeV1::Pending
                                },
                            },
                        )),
                    )
                    .and_then(|o| o.paid_resale())
                    .map_err(invalid)?;
                let roll = self
                    .writer
                    .accept(ctx, txn, &observed, &output)
                    .map_err(invalid)?;
                row.matches_roll(&roll)?;
                let market = MindPaidResaleOperation::from_roll(&roll)?;
                envelope(
                    &self.families.operations,
                    commit,
                    serde_json::to_value(&market).map_err(invalid)?,
                )?;
                txn.insert(&self.families.operations, &commit, market.clone())
                    .map_err(invalid)?;
                if immediate {
                    let receipt = MindPaidResaleReceipt::from_roll(&roll)?;
                    envelope(
                        &self.families.receipts,
                        commit,
                        serde_json::to_value(&receipt).map_err(invalid)?,
                    )?;
                    txn.insert(&self.families.receipts, &commit, receipt.clone())
                        .map_err(invalid)?;
                    if txn.get(&self.families.receipts, &commit).map_err(invalid)? != Some(&receipt)
                    {
                        return Err(invalid("receipt readback"));
                    }
                } else {
                    if txn
                        .get(&self.families.receipts, &commit)
                        .map_err(invalid)?
                        .is_some()
                    {
                        return Err(invalid("premature receipt"));
                    }
                }
                if txn
                    .get(&self.families.operations, &commit)
                    .map_err(invalid)?
                    != Some(&market)
                {
                    return Err(invalid("operation readback"));
                }
                // 17g C2f: the accepted offer leaves its buyer's offer index.
                super::index::consumed(txn, &self.families, &row).map_err(invalid)?;
                if !immediate {
                    ctx.reserve_paid_resale_completion(txn).map_err(invalid)?;
                }
                (commit, if immediate { "settled" } else { "funded" })
            }
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"kind":label,"operation":operation,"commit":commit}),
        )
        .map_err(invalid)?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
fn preflight_market(f: &Families) -> Result<(), String> {
    let token = CommitToken::from_bytes([255; 16]);
    let digest = mush_core::DefinitionDigest::from_bytes([255; 32]);
    // The Market operation is a fixed scalar mirror, not a second native copy.
    let state = serde_json::json!({"phase":"refunded","terminal":token,"reason":{"kind":"registry_revoked","sanction":token,"licence":u64::MAX,"generation":u64::MAX,"term_digest":digest}});
    let row = serde_json::json!({"format":1,"operation":token,"action":mush_roll::commerce::paid_resale::protocol::actions().map_err(invalid)?[2],"licence":u64::MAX.to_string(),"sequence":65536,"offer":token,"offer_digest":digest,"roll_digest":digest,"state":state});
    envelope(&f.operations, token, row)?;
    envelope(
        &f.receipts,
        token,
        serde_json::json!({"format":1,"operation":token,"acceptance":token,"licence":u64::MAX.to_string(),"offer":token,"offer_digest":digest,"roll_digest":digest,"economic_digest":digest,"outcome":"refunded"}),
    )
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        super::actions::offer(self, action, q, engine)
    }
}
