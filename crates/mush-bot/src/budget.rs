//! Session request and provider-charge gates owned by the external bot. Limits
//! are checked before each request against receipts already in the ledger;
//! they cannot bound the final charge of a request that is already in flight.
use std::{collections::BTreeMap, fmt, path::Path};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};

use crate::{Result, costs::Amount, experiment};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnknownCost {
    #[default]
    Block,
    Allow,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "version")]
    pub version: u32,
    pub max_requests: Option<usize>,
    #[serde(deserialize_with = "currency_limits")]
    pub max_cost_by_currency: BTreeMap<String, Amount>,
    pub unknown_cost: UnknownCost,
}

fn version() -> u32 {
    1
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            max_requests: None,
            max_cost_by_currency: BTreeMap::new(),
            unknown_cost: UnknownCost::Block,
        }
    }
}

// A repeated currency must not silently replace an earlier spending limit.
fn currency_limits<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, Amount>, D::Error> {
    struct LimitsVisitor;
    impl<'de> Visitor<'de> for LimitsVisitor {
        type Value = BTreeMap<String, Amount>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("unique currency limits")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((currency, amount)) = map.next_entry::<String, Amount>()? {
                if result.insert(currency, amount).is_some() {
                    return Err(serde::de::Error::custom("duplicate budget currency"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(LimitsVisitor)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Usage {
    pub requests: usize,
    pub unknown_cost_requests: usize,
    pub provider_cost_by_currency: BTreeMap<String, Amount>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    RequestLimitReached {
        limit: usize,
    },
    CostLimitReached {
        currency: String,
        limit: Amount,
        spent: Amount,
    },
    UpcomingCurrencyUnknown,
    UpcomingCurrencyUncovered {
        currency: String,
    },
    UnknownCostRequests {
        requests: usize,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub allowed: bool,
    pub reasons: Vec<Reason>,
    pub remaining_requests: Option<usize>,
    pub remaining_cost_by_currency: BTreeMap<String, Amount>,
    pub usage: Usage,
}

impl Status {
    /// Runner-facing explanation uses fixed text, never provider response text.
    pub fn message(&self) -> String {
        if self.allowed {
            return "Session budget permits a model request.".into();
        }
        let reasons = self
            .reasons
            .iter()
            .map(|reason| match reason {
                Reason::RequestLimitReached { .. } => "request limit reached",
                Reason::CostLimitReached { .. } => "configured cost limit reached",
                Reason::UpcomingCurrencyUnknown => "upcoming model currency is unknown",
                Reason::UpcomingCurrencyUncovered { .. } => {
                    "upcoming model currency has no configured limit"
                }
                Reason::UnknownCostRequests { .. } => {
                    "one or more session requests have unknown costs"
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("Session budget blocks another model request: {reasons}.")
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let settings: Self = experiment::load_json(path, 16 * 1024)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err("budget version must be 1".into());
        }
        if self.max_requests.is_some_and(|limit| limit > 1_000_000) {
            return Err("budget max_requests must be at most 1000000".into());
        }
        if self.max_cost_by_currency.len() > 16
            || self.max_cost_by_currency.keys().any(|currency| {
                currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase())
            })
        {
            return Err(
                "budget requires at most 16 distinct three-letter uppercase ASCII currencies"
                    .into(),
            );
        }
        Ok(())
    }

    /// Hash normalized settings so equivalent exact decimal spellings produce
    /// the same revision and editing limits never changes the usage ledger.
    pub fn revision(&self) -> String {
        experiment::revision(&serde_json::to_vec(self).expect("budget settings serialize to JSON"))
    }

    pub fn evaluate(&self, usage: &Usage, upcoming_currency: Option<&str>) -> Result<Status> {
        self.validate()?;
        if usage.unknown_cost_requests > usage.requests {
            return Err("budget usage has more unknown-cost requests than requests".into());
        }
        let mut reasons = Vec::new();
        if let Some(limit) = self.max_requests
            && usage.requests >= limit
        {
            reasons.push(Reason::RequestLimitReached { limit });
        }
        let mut remaining_cost_by_currency = BTreeMap::new();
        for (currency, limit) in &self.max_cost_by_currency {
            let spent = usage
                .provider_cost_by_currency
                .get(currency)
                .copied()
                .unwrap_or_default();
            remaining_cost_by_currency.insert(
                currency.clone(),
                limit.checked_sub(spent).unwrap_or_default(),
            );
            if spent >= *limit {
                reasons.push(Reason::CostLimitReached {
                    currency: currency.clone(),
                    limit: *limit,
                    spent,
                });
            }
        }
        if !self.max_cost_by_currency.is_empty() {
            match upcoming_currency {
                None => reasons.push(Reason::UpcomingCurrencyUnknown),
                Some(currency) if !self.max_cost_by_currency.contains_key(currency) => {
                    reasons.push(Reason::UpcomingCurrencyUncovered {
                        currency: currency.into(),
                    });
                }
                Some(_) => {}
            }
            if self.unknown_cost == UnknownCost::Block && usage.unknown_cost_requests > 0 {
                reasons.push(Reason::UnknownCostRequests {
                    requests: usage.unknown_cost_requests,
                });
            }
        }
        Ok(Status {
            allowed: reasons.is_empty(),
            reasons,
            remaining_requests: self
                .max_requests
                .map(|limit| limit.saturating_sub(usage.requests)),
            remaining_cost_by_currency,
            usage: usage.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(value: serde_json::Value) -> Settings {
        let settings: Settings = serde_json::from_value(value).unwrap();
        settings.validate().unwrap();
        settings
    }

    fn amount(value: &str) -> Amount {
        serde_json::from_value(json!(value)).unwrap()
    }

    fn usage(requests: usize, unknown: usize, costs: &[(&str, &str)]) -> Usage {
        Usage {
            requests,
            unknown_cost_requests: unknown,
            provider_cost_by_currency: costs
                .iter()
                .map(|(currency, cost)| ((*currency).into(), amount(cost)))
                .collect(),
        }
    }

    #[test]
    fn defaults_are_unrestricted_and_request_caps_include_zero() {
        let unlimited = settings(json!({}));
        assert_eq!(unlimited.version, 1);
        assert!(unlimited.evaluate(&usage(3, 3, &[]), None).unwrap().allowed);
        assert!(
            unlimited
                .evaluate(&usage(100, 0, &[("USD", "999")]), None)
                .unwrap()
                .remaining_requests
                .is_none()
        );
        assert!(
            Settings::default()
                .evaluate(&Usage::default(), None)
                .unwrap()
                .allowed
        );
        let zero = settings(json!({"max_requests":0}));
        let status = zero.evaluate(&Usage::default(), None).unwrap();
        assert!(!status.allowed);
        assert_eq!(
            status.reasons,
            vec![Reason::RequestLimitReached { limit: 0 }]
        );
        assert_eq!(status.remaining_requests, Some(0));
        let two = settings(json!({"max_requests":2}));
        assert!(two.evaluate(&usage(1, 1, &[]), None).unwrap().allowed);
        assert!(!two.evaluate(&usage(2, 2, &[]), None).unwrap().allowed);
        assert_eq!(
            two.evaluate(&usage(20, 0, &[]), None)
                .unwrap()
                .remaining_requests,
            Some(0)
        );
    }

    #[test]
    fn exact_decimal_boundary_and_remaining_cost_never_round() {
        let policy = settings(json!({"max_cost_by_currency":{"USD":"0.3"}}));
        let just_below = policy
            .evaluate(
                &usage(1, 0, &[("USD", "0.299999999999999999")]),
                Some("USD"),
            )
            .unwrap();
        assert!(just_below.allowed);
        assert_eq!(
            just_below.remaining_cost_by_currency["USD"].to_string(),
            "0.000000000000000001"
        );
        let equal = policy
            .evaluate(&usage(1, 0, &[("USD", "0.3")]), Some("USD"))
            .unwrap();
        assert!(!equal.allowed);
        assert_eq!(equal.remaining_cost_by_currency["USD"], Amount::default());
        let exceeded = policy
            .evaluate(
                &usage(2, 0, &[("USD", "0.300000000000000001")]),
                Some("USD"),
            )
            .unwrap();
        assert!(!exceeded.allowed);
        assert_eq!(
            exceeded.remaining_cost_by_currency["USD"],
            Amount::default()
        );
        assert!(
            !settings(json!({"max_cost_by_currency":{"USD":"0"}}))
                .evaluate(&Usage::default(), Some("USD"))
                .unwrap()
                .allowed
        );
        assert_eq!(
            policy.revision(),
            settings(json!({"max_cost_by_currency":{"USD":"0.300000000000000000"}})).revision()
        );
    }

    #[test]
    fn unknown_charges_and_currency_coverage_are_explicit_and_independent() {
        let mut policy = settings(json!({"max_cost_by_currency":{"USD":"1","EUR":"2"}}));
        let unknown = usage(1, 1, &[]);
        let blocked = policy.evaluate(&unknown, Some("USD")).unwrap();
        assert_eq!(
            blocked.reasons,
            vec![Reason::UnknownCostRequests { requests: 1 }]
        );
        policy.unknown_cost = UnknownCost::Allow;
        assert!(policy.evaluate(&unknown, Some("USD")).unwrap().allowed);
        assert_eq!(
            policy.evaluate(&unknown, None).unwrap().reasons,
            vec![Reason::UpcomingCurrencyUnknown]
        );
        assert_eq!(
            policy.evaluate(&unknown, Some("GBP")).unwrap().reasons,
            vec![Reason::UpcomingCurrencyUncovered {
                currency: "GBP".into()
            }]
        );
        assert!(
            !policy
                .evaluate(&usage(1, 0, &[("EUR", "2")]), Some("USD"))
                .unwrap()
                .allowed
        );
        let unconfigured = policy
            .evaluate(&usage(1, 0, &[("GBP", "999")]), Some("USD"))
            .unwrap();
        assert!(unconfigured.allowed);
        assert_eq!(
            unconfigured.usage.provider_cost_by_currency["GBP"],
            amount("999")
        );
        assert!(!unconfigured.remaining_cost_by_currency.contains_key("GBP"));
        assert!(
            Settings::default()
                .evaluate(&usage(0, 1, &[]), None)
                .is_err()
        );
    }

    #[test]
    fn policy_json_is_strict_and_keeps_decimal_strings() {
        for text in [
            r#"{"unexpected":true}"#,
            r#"{"max_requests":1.5}"#,
            r#"{"max_requests":-1}"#,
            r#"{"max_cost_by_currency":{"USD":0.1}}"#,
            r#"{"max_cost_by_currency":{"USD":"-1"}}"#,
            r#"{"max_cost_by_currency":{"USD":"1","USD":"2"}}"#,
            r#"{"max_cost_by_currency":{"USD":"1","U\u0053D":"2"}}"#,
            r#"{"unknown_cost":"estimate"}"#,
        ] {
            assert!(serde_json::from_str::<Settings>(text).is_err(), "{text}");
        }
        for value in [
            json!({"version":2}),
            json!({"max_requests":1_000_001}),
            json!({"max_cost_by_currency":{"usd":"1"}}),
            json!({"max_cost_by_currency":{"US":"1"}}),
            json!({"max_cost_by_currency":{"ÜSD":"1"}}),
        ] {
            assert!(
                serde_json::from_value::<Settings>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut too_many = settings(json!({}));
        for letter in b'A'..=b'Q' {
            too_many
                .max_cost_by_currency
                .insert(format!("AA{}", char::from(letter)), amount("1"));
        }
        assert!(too_many.validate().is_err());
        let accepted =
            settings(json!({"max_requests":1_000_000,"max_cost_by_currency":{"USD":"0"}}));
        assert_eq!(
            serde_json::to_value(accepted).unwrap()["max_cost_by_currency"]["USD"],
            "0"
        );
    }
}
