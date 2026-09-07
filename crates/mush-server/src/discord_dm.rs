//! The on-call service's Discord DM delivery adapter (interface programme D5,
//! 03 §7a.3): a [`DurableOutboxAdapter`] under `owner = "oncall"`, `adapter =
//! "discord.dm"` over a REST-only [`DiscordApi`] (`DiscordRest` in production,
//! the fake in tests).
//!
//! Discord message creation has **no idempotency key**, so the adapter keeps the
//! outbox's existing contract — the outbox persists the attempt marker, then the
//! adapter sends — and *labels* a possible duplicate (`incident #id · attempt n`
//! in the text) rather than promising exactly-once. Retryable versus permanent
//! is decided from the HTTP class and Discord's error code: a closed DM is
//! permanent for that endpoint, so on-call falls through to SMS or email fast.
//!
//! Nothing produces an `oncall` intent yet — that is the on-call programme. The
//! adapter is wired whenever `[discord].enabled` and the token resolve, so the
//! first intent finds it registered.

use std::sync::Arc;

use futures::future::BoxFuture;
use mush_config::config::Config;
use mush_core::{CanonicalId, OwnerId};
use mush_net_discord::{ApiError, DiscordApi, DiscordConfig, DiscordRest, Outgoing};
use mush_store::AuthoritativeOutboxRecord;
use serde::Deserialize;

use crate::durable_outbox::{DurableDeliveryError, DurableOutboxAdapter};

/// The outbox owner the adapter serves.
pub const ONCALL_OWNER: &str = "oncall";
/// The adapter id under that owner.
pub const DISCORD_DM_ADAPTER: &str = "discord.dm";
/// The longest page text the adapter sends, characters (the label is added
/// after; Discord's cap is 2 000).
pub const MAX_PAGE_CHARS: usize = 1_800;

/// The intent payload the on-call service writes (03 §7a.3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DmPayload {
    /// The Discord user to page (resolved by on-call from the account's link).
    user_id: u64,
    /// The page text.
    text: String,
    /// The incident reference, for the duplicate label and the audit.
    incident: String,
    /// The delivery attempt number, for the duplicate label.
    #[serde(default = "one")]
    attempt: u32,
}

const fn one() -> u32 {
    1
}

/// The Discord DM adapter (see the module docs).
pub struct DiscordDmAdapter {
    owner: OwnerId,
    adapter: CanonicalId,
    api: Arc<dyn DiscordApi>,
}

impl DiscordDmAdapter {
    /// An adapter over `api`.
    #[must_use]
    pub fn new(api: Arc<dyn DiscordApi>) -> DiscordDmAdapter {
        DiscordDmAdapter {
            owner: OwnerId::new(ONCALL_OWNER).expect("static owner id is canonical"),
            adapter: CanonicalId::new(DISCORD_DM_ADAPTER).expect("static adapter id is canonical"),
            api,
        }
    }

    /// The labelled text: the page, then `incident #… · attempt n` so a
    /// duplicate delivery reads as one.
    fn labelled(payload: &DmPayload) -> String {
        let mut text: String = payload.text.chars().take(MAX_PAGE_CHARS).collect();
        if payload.text.chars().count() > MAX_PAGE_CHARS {
            text.push('…');
        }
        format!(
            "{text}\n_incident #{} · attempt {}_",
            payload.incident, payload.attempt
        )
    }

    async fn send(&self, payload: &DmPayload) -> Result<u64, DurableDeliveryError> {
        let channel = self
            .api
            .create_dm(payload.user_id)
            .await
            .map_err(classify)?;
        self.api
            .send(channel, Outgoing::plain(Self::labelled(payload)))
            .await
            .map_err(classify)
    }
}

/// Retryable versus permanent, from the API's own classification: a closed
/// DM is permanent for this endpoint.
fn classify(error: ApiError) -> DurableDeliveryError {
    match error {
        ApiError::Retryable(why) => DurableDeliveryError::Retryable(why),
        ApiError::Permanent(why) => DurableDeliveryError::Permanent(why),
        ApiError::CannotDm => {
            DurableDeliveryError::Permanent("the user's Discord DMs are closed".to_string())
        }
    }
}

impl DurableOutboxAdapter for DiscordDmAdapter {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn adapter(&self) -> &CanonicalId {
        &self.adapter
    }

    fn deliver<'a>(
        &'a self,
        item: &'a AuthoritativeOutboxRecord,
    ) -> BoxFuture<'a, Result<(), DurableDeliveryError>> {
        Box::pin(async move {
            if item.intent().owner() != &self.owner || item.intent().adapter() != &self.adapter {
                return Err(DurableDeliveryError::Permanent(
                    "discord dm intent was routed to the wrong adapter".to_owned(),
                ));
            }
            let payload: DmPayload = serde_json::from_value(item.intent().payload().clone())
                .map_err(|error| {
                    DurableDeliveryError::Permanent(format!(
                        "discord dm payload is invalid: {error}"
                    ))
                })?;
            if payload.user_id == 0 || payload.text.trim().is_empty() {
                return Err(DurableDeliveryError::Permanent(
                    "discord dm payload has no recipient or no text".to_owned(),
                ));
            }
            let message_id = self.send(&payload).await?;
            // Delivery evidence for on-call's audit (03 §7a.5): the outbox
            // result has no field for it, so it is the log's.
            tracing::info!(
                incident = %payload.incident,
                attempt = payload.attempt,
                message_id,
                "on-call Discord DM delivered"
            );
            Ok(())
        })
    }
}

/// The deployment's Discord outbox adapters from `[discord]`: one
/// [`DiscordDmAdapter`] when the bot is enabled and its token resolves, else
/// none (loud at info, never fatal — on-call then has no Discord endpoint and
/// falls through to its other channels).
#[must_use]
pub fn adapters_from_config(config: &Config) -> Vec<Arc<dyn DurableOutboxAdapter>> {
    let mut dc = match DiscordConfig::deserialize(config.discord.clone()) {
        Ok(dc) => dc,
        Err(_) => return Vec::new(),
    };
    if !dc.enabled {
        return Vec::new();
    }
    dc.resolve_ids_from_env();
    let Some(token) = dc.resolve_token() else {
        tracing::info!("[discord] enabled without a token: no on-call Discord DM adapter");
        return Vec::new();
    };
    let api: Arc<dyn DiscordApi> = Arc::new(DiscordRest::new(token, dc.application_id));
    tracing::info!("on-call Discord DM adapter registered (oncall / discord.dm)");
    vec![Arc::new(DiscordDmAdapter::new(api))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{CanonicalHash, CommitToken, DurableOutboxIntent};
    use mush_net_discord::testing::{Call, FakeApi};

    fn record(owner: &str, adapter: &str, payload: serde_json::Value) -> AuthoritativeOutboxRecord {
        let intent = DurableOutboxIntent::new(
            0,
            OwnerId::new(owner).unwrap(),
            CanonicalId::new(adapter).unwrap(),
            payload,
        )
        .unwrap();
        AuthoritativeOutboxRecord::new(
            CommitToken::from_bytes([7; 16]),
            CanonicalHash::from_bytes([0; 32]),
            intent,
        )
        .unwrap()
    }

    fn page(attempt: u32) -> serde_json::Value {
        serde_json::json!({
            "user_id": 123_456_789_012_345_678_u64,
            "text": "Quarantine spool full on dev1 *now*",
            "incident": "17",
            "attempt": attempt,
        })
    }

    #[tokio::test]
    async fn a_page_opens_the_dm_and_carries_the_duplicate_label() {
        let api = FakeApi::new();
        let adapter = DiscordDmAdapter::new(api.clone());
        assert_eq!(adapter.owner().as_str(), ONCALL_OWNER);
        assert_eq!(adapter.adapter().as_str(), DISCORD_DM_ADAPTER);
        adapter
            .deliver(&record(ONCALL_OWNER, DISCORD_DM_ADAPTER, page(2)))
            .await
            .expect("delivered");
        let calls = api.calls();
        assert!(matches!(
            calls[0],
            Call::CreateDm {
                user_id: 123_456_789_012_345_678
            }
        ));
        let Call::Send { msg, .. } = &calls[1] else {
            panic!("the page is sent to the DM");
        };
        assert!(
            msg.content
                .starts_with("Quarantine spool full on dev1 *now*")
        );
        assert!(
            msg.content.ends_with("_incident #17 · attempt 2_"),
            "{}",
            msg.content
        );
    }

    #[tokio::test]
    async fn retryable_and_permanent_outcomes_follow_the_api_classes() {
        let api = FakeApi::new();
        let adapter = DiscordDmAdapter::new(api.clone());
        // A rate limit on the send: retryable.
        api.script([None, Some(ApiError::Retryable("429".into()))]);
        assert!(matches!(
            adapter
                .deliver(&record(ONCALL_OWNER, DISCORD_DM_ADAPTER, page(1)))
                .await,
            Err(DurableDeliveryError::Retryable(_))
        ));
        // Closed DMs: permanent for this endpoint.
        api.close_dms(123_456_789_012_345_678);
        assert!(matches!(
            adapter
                .deliver(&record(ONCALL_OWNER, DISCORD_DM_ADAPTER, page(1)))
                .await,
            Err(DurableDeliveryError::Permanent(why)) if why.contains("closed")
        ));
    }

    #[tokio::test]
    async fn malformed_payloads_and_misrouted_intents_are_permanent() {
        let api = FakeApi::new();
        let adapter = DiscordDmAdapter::new(api.clone());
        for bad in [
            serde_json::json!({ "text": "no recipient" }),
            serde_json::json!({ "user_id": 5, "text": "", "incident": "1" }),
            serde_json::json!({ "user_id": 5, "text": "x", "incident": "1", "extra": true }),
        ] {
            assert!(matches!(
                adapter
                    .deliver(&record(ONCALL_OWNER, DISCORD_DM_ADAPTER, bad))
                    .await,
                Err(DurableDeliveryError::Permanent(_))
            ));
        }
        assert!(matches!(
            adapter
                .deliver(&record("jobs", DISCORD_DM_ADAPTER, page(1)))
                .await,
            Err(DurableDeliveryError::Permanent(why)) if why.contains("wrong adapter")
        ));
        assert!(api.calls().is_empty(), "nothing reached Discord");
    }

    #[test]
    fn no_adapter_without_the_bot_or_its_token() {
        assert!(adapters_from_config(&Config::default()).is_empty());
        // Enabled, but no token in this test's environment (the variable name
        // is unique): no adapter, no panic.
        let config = Config {
            discord: toml::from_str(
                "enabled = true
application_id = 1
guild_id = 2
                 token_env = \"RUSTMUSH_TEST_NO_SUCH_TOKEN\"",
            )
            .unwrap(),
            ..Default::default()
        };
        assert!(adapters_from_config(&config).is_empty());
    }
}
