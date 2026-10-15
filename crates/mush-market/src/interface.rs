//! S6 (doc 14 §4.6): the economist's interface — `market.list` and
//! `market.quote` over `+market`'s public projection, never the book beneath
//! it (doc 10 §14): the listing walk is the asker's own visibility, and a
//! quote is the asker's own record, reserving nothing.
//!
//! **Both replies are the asker's alone** (`ReplyAudience::Asker`, doc 18 cut IC0).
//! The list is filtered by `may_view_listing` **as the asker** — a hidden listing
//! they may see is in it and one they may not is out — and the quote is a record
//! issued to them, so the NPC said aloud to the room exactly what the read had kept
//! private. The default audience is the one that matches the read.

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::api::{self, MarketFilter};
use crate::ids::{ListingId, QuoteId};
use crate::store::MarketStore;
use crate::trade::{QuoteRequest, QuoteWindow};

/// The provider id.
pub const MARKET_INTERFACE: &str = "market";

/// The most listings one answer carries.
pub const MAX_LIST_ROWS: usize = 10;

/// The economist.
#[derive(Clone, Copy, Debug, Default)]
pub struct MarketInterface;

impl InterfaceAbilityProvider for MarketInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("market.list"),
                description: "What is for sale that the asker can see, as +market/list prints it; an optional filter word (a good, legal=..., a vendor)."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "filter": { "type": "string" } }
                }),
                command: SmolStr::new_static("+market/list"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("market.quote"),
                description: "A quote on one listing for the asker, reserving nothing: the price and how long it holds."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "listing": { "type": "integer" },
                        "quantity": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["listing"]
                }),
                command: SmolStr::new_static("+market/quote"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        match name {
            "market.list" => {
                let filter = input
                    .get("filter")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .map(MarketFilter::parse);
                let page = api::visible(ctx.w, call.asker, filter.as_ref(), MAX_LIST_ROWS, None);
                let mut rows: Vec<String> = Vec::new();
                for id in &page.ids {
                    if let Some(l) = ctx
                        .w
                        .module_state::<MarketStore>()
                        .and_then(|s| s.listing(*id).cloned())
                    {
                        rows.push(crate::commands::listing_line(ctx, &l));
                    }
                }
                if rows.is_empty() {
                    rows.push("Nothing is for sale.".to_string());
                }
                if page.next.is_some() {
                    rows.push("(There is more: +market/list shows the rest.)".to_string());
                }
                Ok(InterfaceReply::lines(&rows))
            }
            "market.quote" => {
                let listing = input
                    .get("listing")
                    .and_then(Value::as_u64)
                    .map(ListingId)
                    .ok_or_else(|| InterfaceError::Invalid("a listing number".to_string()))?;
                let quantity = input
                    .get("quantity")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1);
                let req = QuoteRequest {
                    listing,
                    quantity,
                    window: QuoteWindow::Live,
                    ttl_live: crate::trade::QUOTE_TTL_LIVE,
                    ttl_remote: crate::trade::QUOTE_TTL_REMOTE,
                };
                // The command context's actor is the asker (the NPC runtime
                // swaps it for the call), so the quote is theirs.
                let id: QuoteId = crate::trade::quote(ctx, &req)
                    .map_err(|e| InterfaceError::Refused(format!("No quote: {e:?}.")))?;
                let q = ctx
                    .w
                    .module_state::<MarketStore>()
                    .and_then(|s| s.quote(id).cloned());
                Ok(InterfaceReply::text(q.map_or_else(
                    || format!("Quote #{id}."),
                    |q| {
                        let instrument = crate::commands::quote_instrument(ctx.w, &q);
                        format!(
                            "Quote #{id}: {} × {} = {} — good until {} (real time). It reserves nothing; +market/accept #{id} takes it up.",
                            q.quantity,
                            crate::commands::money(ctx.w, instrument.as_ref(), q.price),
                            crate::commands::money(ctx.w, instrument.as_ref(), q.total()),
                            q.expires.at
                        )
                    },
                )))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, ObjId, Outbox, ParsedArgs, SwitchSet};
    use mush_testkit::TestWorld;

    fn invoke(
        tw: &mut TestWorld,
        asker: ObjId,
        name: &str,
        input: Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        let engine = tw.engine_mut();
        let provider = engine
            .registry
            .interface_provider(MARKET_INTERFACE)
            .expect("the market interface");
        let mut out = Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: asker,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new("npc")),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        provider.invoke(&mut ctx, AbilityCall { npc: GOD, asker }, name, &input)
    }

    #[test]
    fn the_economist_reads_the_public_projection_and_refuses_a_bad_listing() {
        let mut tw = TestWorld::with_modules(vec![Box::new(crate::MarketModule::new())]);
        let rae = tw.spawn_player("Rae");
        let names: Vec<SmolStr> = MarketInterface
            .abilities()
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(
            names,
            vec![SmolStr::new("market.list"), SmolStr::new("market.quote")]
        );
        // An empty market reads as +market/list prints it; a filter word that
        // matches nothing is an empty list, never a refusal (08a decision 13).
        let reply = invoke(&mut tw, rae, "market.list", json!({})).expect("a list");
        assert_eq!(reply.text, "Nothing is for sale.");
        // Doc 18 cut IC0: the list was filtered as the asker, so it is theirs alone.
        assert_eq!(reply.audience, mush_core::ReplyAudience::Asker);
        let reply = invoke(
            &mut tw,
            rae,
            "market.list",
            json!({ "filter": "legal=restricted" }),
        )
        .expect("a list");
        assert_eq!(reply.text, "Nothing is for sale.");
        let ops = tw.engine_mut().take_module_ops();
        assert!(ops.is_empty(), "a list is a read: {ops:?}");
        // A quote on nothing is refused in the market's words; a missing
        // listing number is invalid; an unknown ability is unknown.
        assert!(matches!(
            invoke(&mut tw, rae, "market.quote", json!({ "listing": 99 })),
            Err(InterfaceError::Refused(_))
        ));
        assert!(matches!(
            invoke(&mut tw, rae, "market.quote", json!({})),
            Err(InterfaceError::Invalid(_))
        ));
        assert!(matches!(
            invoke(&mut tw, rae, "market.buy", json!({})),
            Err(InterfaceError::Unknown(_))
        ));
    }
}
