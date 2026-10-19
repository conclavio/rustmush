//! Read-only, connection-correlated offer queries. This is deliberately a
//! whitelist, not serialization of retained subject/account/certificate rows.
use super::records::{MindListing, MindQuote, Row};
use super::{Families, OriginalQuoteConfig, index};
use mush_core::module::{HookReadContext, QueryHandler};
use mush_core::{CommitToken, ObjId, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Listings { after: Option<CommitToken> },
    Own { after: Option<CommitToken> },
}
pub(super) struct Offers {
    config: OriginalQuoteConfig,
    families: Families,
    gate: Arc<dyn crate::VenueGate>,
}
fn refusal() -> RichText {
    RichText::plain("Original mind offers are unavailable.")
}
fn product(row: &super::records::OriginalProduct) -> Value {
    json!({"class":row.class,"seats":row.seats,"form":match row.form { mush_roll::commerce::paid::PaidForm::Sealed {} => "sealed", mush_roll::commerce::paid::PaidForm::Open {} => "open", mush_roll::commerce::paid::PaidForm::OpenResale {} => "open_resale" },"revision":"pin",
        "transferable":row.transferable,"term_seconds":row.term_seconds,"grace_seconds":row.grace_seconds})
}
fn public_listing(row: &MindListing) -> Value {
    json!({"publication":row.publication,"product":product(&row.product),"price":row.price,
        "instrument":row.instrument,"venue":row.venue.name,"observed_at_utc":row.observed_at_utc})
}
fn own_quote(row: &MindQuote) -> Value {
    json!({"publication":row.publication,"listing":row.listing,"product":product(&row.product),
        "price":row.price,"instrument":row.instrument,"venue":row.venue.name,
        "expires_at_utc":row.expires_at_utc})
}
impl Offers {
    pub(super) fn new(
        config: OriginalQuoteConfig,
        families: Families,
        gate: Arc<dyn crate::VenueGate>,
    ) -> Self {
        Self {
            config,
            families,
            gate,
        }
    }
    fn read(&self, ctx: &HookReadContext<'_>, actor: ObjId, args: &Value) -> Result<Value, String> {
        if args.to_string().len() > 256 {
            return Err("query bound".into());
        }
        let request: Request = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
        if ctx.world().get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        }) {
            return Err("query actor unavailable".into());
        }
        let owner = mush_core::mechanics::OwnerId::new(crate::NS).map_err(|e| e.to_string())?;
        ctx.registry()
            .ok_or("live registry unavailable")?
            .require_mechanics_owner_ready(&owner)
            .map_err(|e| e.to_string())?;
        match request {
            Request::Listings { after } => {
                let venue = crate::MarketId::new(&self.config.policy.venue.name)
                    .ok_or("venue unavailable")?;
                let room = ctx
                    .world()
                    .containing_room(actor)
                    .ok_or("venue unavailable")?;
                if self.gate.venue_of_room(ctx.world(), room).as_ref() != Some(&venue) {
                    return Err("venue unavailable".into());
                }
                self.gate
                    .admit(ctx.world(), actor, &venue, None, crate::Presence::Here)
                    .map_err(|_| "venue unavailable")?;
                let after = after.map(|id| id.to_hex());
                let page = ctx
                    .scan_action_state(&self.families.listings, after.as_deref(), index::QUERY_PAGE)
                    .map_err(|e| e.to_string())?;
                let policy = self.config.policy_digest()?;
                let mut rows = Vec::new();
                for entry in page.entries() {
                    let row = entry.value();
                    row.validate()?;
                    if row.publication.to_hex() != entry.encoded_key() {
                        return Err("listing identity differs".into());
                    }
                    // A previous composition's terms may still be retired by its
                    // owner, but cannot be advertised as a current offer.
                    if row.policy == policy {
                        rows.push(public_listing(row));
                    }
                }
                Ok(json!({"kind":"listings","rows":rows,"after":page.next_after(),"funded":false}))
            }
            Request::Own { after } => {
                let Some(index) = ctx
                    .action_state(&self.families.indices.actors, &actor)
                    .map_err(|e| e.to_string())?
                else {
                    return Ok(
                        json!({"kind":"own","listings":[],"quotes":[],"after":null,"funded":false}),
                    );
                };
                index.validate()?;
                if index.actor != actor {
                    return Err("actor index differs".into());
                }
                let mut ids: Vec<_> = index
                    .listings
                    .iter()
                    .chain(index.quotes.iter())
                    .copied()
                    .filter(|id| after.is_none_or(|after| *id > after))
                    .collect();
                ids.sort();
                let has_more = ids.len() > index::QUERY_PAGE;
                ids.truncate(index::QUERY_PAGE);
                let next = if has_more { ids.last().copied() } else { None };
                let mut listings = Vec::new();
                let mut quotes = Vec::new();
                for id in ids {
                    if index.listings.contains(&id) {
                        let row = ctx
                            .action_state(&self.families.listings, &id)
                            .map_err(|e| e.to_string())?
                            .ok_or("listing missing")?;
                        row.validate()?;
                        if row.publication != id || row.subject.seller != actor {
                            return Err("listing owner differs".into());
                        }
                        listings.push(public_listing(row));
                    } else {
                        let row = ctx
                            .action_state(&self.families.quotes, &id)
                            .map_err(|e| e.to_string())?
                            .ok_or("quote missing")?;
                        row.validate()?;
                        if row.publication != id || row.buyer != actor {
                            return Err("quote owner differs".into());
                        }
                        quotes.push(own_quote(row));
                    }
                }
                Ok(
                    json!({"kind":"own","listings":listings,"quotes":quotes,"after":next,"funded":false}),
                )
            }
        }
    }
}
impl QueryHandler for Offers {
    fn query(&self, _: &World, _: ObjId, _: &Value) -> Result<OutputEvent, RichText> {
        Err(refusal())
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        args: &Value,
    ) -> Result<OutputEvent, RichText> {
        let data = self.read(ctx, actor, args).map_err(|_| refusal())?;
        // At most 16 small, closed projections, with an explicit transport cap.
        if data.to_string().len() > 32 * 1024 {
            return Err(refusal());
        }
        Ok(OutputEvent::Panel {
            feed: super::QUERY.into(),
            kind: PanelFrameKind::Keyframe,
            data,
        })
    }
}

pub(super) fn expire_tick(
    ctx: &mut mush_core::Ctx<'_>,
    families: &Families,
    action: &mush_core::DefinitionRef,
) {
    if ctx.actor != mush_core::GOD || ctx.authoritative_pending(mush_core::GOD) {
        return;
    }
    let read = ctx.hook_read_context(crate::NS);
    let Ok(page) = read.scan_action_state(&families.indices.expiry, None, 1) else {
        return;
    };
    let Some(first) = page.entries().first() else {
        return;
    };
    if first.value().at > ctx.w.now() || first.value().key().encoded() != first.encoded_key() {
        return;
    }
    // Actual provider projection selects the due bounded batch again. No IDs or
    // success are manufactured here, and refusal is retried on a later tick.
    let raw = format!(
        "market.mind.expire.{}.{:x}",
        first.value().quote.to_hex(),
        ctx.w.now() as u64
    );
    let Ok(key) = mush_core::action::IdempotencyKeyText::parse_canonical(raw) else {
        return;
    };
    ctx.submit_authoritative(
        action.clone(),
        mush_core::action::PerceivedRef::SelfActor,
        vec![],
        json!({}),
        key,
    );
}
