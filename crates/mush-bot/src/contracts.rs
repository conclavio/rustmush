//! Assertions evaluated by the runner, independently of a model's claimed outcome.
use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Matcher {
    pub actor: Option<String>,
    pub kind: String,
    pub contains: Option<String>,
    /// Restrict contains to this string leaf of event.data. Without it, contains
    /// retains the legacy behavior of searching all serialized event.data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_pointer: Option<String>,
    /// JSON pointer relative to event.data; pair with equals for typed comparisons.
    pub pointer: Option<String>,
    #[serde(
        default,
        deserialize_with = "expected_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub equals: Option<Value>,
}

fn expected_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

impl Matcher {
    pub fn validate(&self) -> Result<()> {
        if self.kind.is_empty()
            || self.kind.len() > 100
            || self.kind.chars().any(char::is_control)
            || self
                .contains
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 16_000)
            || self.pointer.is_some() != self.equals.is_some()
            || (self.text_pointer.is_some() && self.contains.is_none())
            || self
                .pointer
                .iter()
                .chain(&self.text_pointer)
                .any(|p| !valid_pointer(p))
        {
            return Err("invalid event matcher: kind, nonempty contains, a valid pointer paired with equals, and text_pointer only with contains; pointers use ~0/~1 escapes".into());
        }
        Ok(())
    }
}

fn valid_pointer(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut bytes = pointer.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Contract {
    pub eventually: Vec<Matcher>,
    pub ordered: Vec<Matcher>,
    pub forbidden: Vec<Matcher>,
}

impl Contract {
    pub fn validate(&self) -> Result<()> {
        if self.eventually.len() + self.ordered.len() + self.forbidden.len() > 256 {
            return Err("at most 256 assertions per contract".into());
        }
        for matcher in self
            .eventually
            .iter()
            .chain(&self.ordered)
            .chain(&self.forbidden)
        {
            matcher.validate()?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct MatchState {
    text: std::collections::BTreeMap<String, String>,
}
impl MatchState {
    fn feed(&mut self, matcher: &Matcher, actor: &str, event: &Value) -> bool {
        if matcher.actor.as_deref().is_some_and(|a| a != actor) || event["kind"] != matcher.kind {
            return false;
        }
        // A missing data field is not the JSON null value. Indexing event["data"]
        // would conflate the two and let an incomplete event satisfy equals:null.
        let data = event.get("data");
        if let Some(pointer) = &matcher.pointer
            && data.and_then(|data| data.pointer(pointer)) != matcher.equals.as_ref()
        {
            // Do not splice a suffix from before a rejected observation onto a
            // later accepted one: that substring never occurred in the stream.
            self.text.remove(actor);
            return false;
        }
        let Some(needle) = &matcher.contains else {
            return true;
        };
        let Some(data) = data else {
            self.text.remove(actor);
            return false;
        };
        let text = if let Some(pointer) = &matcher.text_pointer {
            let Some(text) = data.pointer(pointer).and_then(Value::as_str) else {
                self.text.remove(actor);
                return false;
            };
            text.to_owned()
        } else {
            data.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| data.to_string())
        };
        if matcher.kind == "observation" {
            let previous = self.text.entry(actor.into()).or_default();
            let combined = format!("{previous}{text}");
            let found = combined.contains(needle);
            *previous = crate::tail(&combined, needle.len().saturating_sub(1)).into();
            found
        } else {
            text.contains(needle)
        }
    }
}

pub struct Monitor {
    contract: Contract,
    eventual: Vec<Option<Value>>,
    eventual_state: Vec<MatchState>,
    ordered: Vec<Value>,
    ordered_state: MatchState,
    forbidden_state: Vec<MatchState>,
    failure: Option<Value>,
}

impl Monitor {
    pub fn new(contract: Contract) -> Result<Self> {
        contract.validate()?;
        Ok(Self {
            eventual: vec![None; contract.eventually.len()],
            eventual_state: (0..contract.eventually.len())
                .map(|_| MatchState::default())
                .collect(),
            ordered: Vec::new(),
            ordered_state: MatchState::default(),
            forbidden_state: (0..contract.forbidden.len())
                .map(|_| MatchState::default())
                .collect(),
            failure: None,
            contract,
        })
    }
    pub fn feed(&mut self, actor: &str, event: &Value) {
        let evidence =
            || serde_json::json!({"actor":actor,"seq":event["seq"],"kind":event["kind"]});
        for (i, matcher) in self.contract.forbidden.iter().enumerate() {
            if self.forbidden_state[i].feed(matcher, actor, event) && self.failure.is_none() {
                self.failure = Some(serde_json::json!({"matcher":matcher,"evidence":evidence()}));
            }
        }
        for (i, matcher) in self.contract.eventually.iter().enumerate() {
            if self.eventual[i].is_none() && self.eventual_state[i].feed(matcher, actor, event) {
                self.eventual[i] = Some(evidence());
            }
        }
        if let Some(matcher) = self.contract.ordered.get(self.ordered.len())
            && self.ordered_state.feed(matcher, actor, event)
        {
            self.ordered.push(evidence());
            self.ordered_state = MatchState::default();
        }
    }
    pub fn failed(&self) -> bool {
        self.failure.is_some()
    }
    pub fn satisfied(&self) -> bool {
        !self.failed()
            && self.eventual.iter().all(Option::is_some)
            && self.ordered.len() == self.contract.ordered.len()
    }
    /// Serial group dispatch evaluates already-scoped expectations per member.
    pub fn actor_satisfied(&self, actor: &str) -> bool {
        !self.failed()
            && self
                .contract
                .eventually
                .iter()
                .zip(&self.eventual)
                .all(|(m, evidence)| m.actor.as_deref() != Some(actor) || evidence.is_some())
    }
    pub fn report(&self) -> Value {
        serde_json::json!({"success":self.satisfied(),"eventual_evidence":self.eventual,"ordered_evidence":self.ordered,"forbidden_failure":self.failure,
            "missing_eventually":self.contract.eventually.iter().zip(&self.eventual).filter(|(_,e)| e.is_none()).map(|(m,_)| m).collect::<Vec<_>>(),
            "remaining_ordered":&self.contract.ordered[self.ordered.len()..]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(sequence: u64, kind: &str, data: Value) -> Value {
        serde_json::json!({"seq":sequence,"kind":kind,"data":data})
    }

    #[test]
    fn assertions_require_order_evidence_and_detect_fragmented_forbidden_output() {
        let contract: Contract = serde_json::from_value(serde_json::json!({
            "eventually":[{"actor":"rose","kind":"observation","contains":"Flower Shop"}],
            "ordered":[{"kind":"send","contains":"east"},{"kind":"observation","contains":"Flower Shop"}],
            "forbidden":[{"actor":"rose","kind":"observation","contains":"Permission denied"}]})).unwrap();
        let mut monitor = Monitor::new(contract).unwrap();
        let event = |kind, data| serde_json::json!({"seq":1,"kind":kind,"data":data});
        monitor.feed("rose", &event("observation", "Flower Shop"));
        monitor.feed("rose", &event("send", "east"));
        assert!(!monitor.satisfied());
        monitor.feed("rose", &event("observation", "Flower "));
        monitor.feed("rose", &event("observation", "Shop"));
        assert!(monitor.satisfied());
        monitor.feed("rose", &event("observation", "Permission "));
        monitor.feed("marco", &event("observation", "denied"));
        assert!(!monitor.failed());
        monitor.feed("rose", &event("observation", "denied"));
        assert!(monitor.failed());
    }

    #[test]
    fn missing_data_is_distinct_from_explicit_null_and_kind_only_events() {
        let contract: Contract = serde_json::from_value(serde_json::json!({"eventually":[
            {"kind":"answer","pointer":"","equals":null},
            {"kind":"answer","contains":"null"},
            {"kind":"answer"}
        ]}))
        .unwrap();
        let mut monitor = Monitor::new(contract).unwrap();
        monitor.feed("rose", &serde_json::json!({"seq":1,"kind":"answer"}));
        assert!(!monitor.satisfied());
        let report = monitor.report();
        assert!(report["eventual_evidence"][0].is_null());
        assert!(report["eventual_evidence"][1].is_null());
        assert_eq!(report["eventual_evidence"][2]["seq"], 1);
        monitor.feed("rose", &event(2, "answer", Value::Null));
        assert!(monitor.satisfied());
        assert_eq!(monitor.report()["eventual_evidence"][0]["seq"], 2);
    }

    #[test]
    fn rejected_or_missing_observation_fragments_cannot_be_spliced_into_a_match() {
        let contract: Contract = serde_json::from_value(serde_json::json!({"forbidden":[
            {"kind":"observation","contains":"aa","pointer":"","equals":"a"}
        ]}))
        .unwrap();
        let mut monitor = Monitor::new(contract).unwrap();
        monitor.feed("rose", &event(1, "observation", "a".into()));
        monitor.feed("rose", &event(2, "observation", "x".into()));
        monitor.feed("rose", &event(3, "observation", "a".into()));
        assert!(!monitor.failed(), "the real stream is axa, not aa");
        monitor.feed("rose", &serde_json::json!({"seq":4,"kind":"observation"}));
        monitor.feed("rose", &event(5, "observation", "a".into()));
        assert!(
            !monitor.failed(),
            "missing text cannot establish stream continuity"
        );
        monitor.feed("rose", &event(6, "observation", "a".into()));
        assert!(monitor.failed());
        assert_eq!(monitor.report()["forbidden_failure"]["evidence"]["seq"], 6);
    }

    #[test]
    fn repeated_ordered_matchers_require_distinct_later_events() {
        let contract: Contract = serde_json::from_value(serde_json::json!({"ordered":[
            {"actor":"rose","kind":"observation","contains":"rose"},
            {"actor":"rose","kind":"observation","contains":"rose"}
        ]}))
        .unwrap();
        let mut monitor = Monitor::new(contract).unwrap();
        monitor.feed("rose", &event(1, "observation", "rose rose".into()));
        assert!(!monitor.satisfied());
        assert_eq!(
            monitor.report()["ordered_evidence"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        monitor.feed("marco", &event(2, "observation", "rose".into()));
        monitor.feed("rose", &event(3, "observation", "ro".into()));
        assert!(!monitor.satisfied());
        monitor.feed("rose", &event(4, "observation", "se".into()));
        assert!(monitor.satisfied());
        assert_eq!(monitor.report()["ordered_evidence"][1]["seq"], 4);
    }

    #[test]
    fn observation_tails_are_utf8_bounded_and_survive_unrelated_local_events() {
        let matcher: Matcher = serde_json::from_value(serde_json::json!({
            "kind":"observation","contains":"café🌹"
        }))
        .unwrap();
        let mut state = MatchState::default();
        for sequence in 1..=20 {
            assert!(!state.feed(
                &matcher,
                "rose",
                &event(sequence, "observation", "🌼".repeat(1000).into())
            ));
        }
        assert!(state.text["rose"].len() < matcher.contains.as_ref().unwrap().len());
        assert!(!state.feed(&matcher, "rose", &event(21, "observation", "ca".into())));
        assert!(!state.feed(&matcher, "marco", &event(22, "observation", "fé🌹".into())));
        assert!(!state.feed(&matcher, "rose", &event(23, "send", "north".into())));
        assert!(!state.feed(&matcher, "rose", &event(24, "costs", serde_json::json!({}))));
        assert!(state.feed(&matcher, "rose", &event(25, "observation", "fé🌹".into())));
    }

    #[test]
    fn json_pointer_escapes_are_validated_before_matching() {
        let make = |pointer: &str| {
            serde_json::from_value::<Matcher>(serde_json::json!({
                "kind":"answer","pointer":pointer,"equals":true
            }))
            .unwrap()
        };
        for pointer in ["plain", "/trailing~", "/wrong~2", "/wrong~~0"] {
            assert!(
                make(pointer).validate().is_err(),
                "accepted invalid pointer {pointer}"
            );
        }
        for pointer in ["", "/", "/a~1b/~0key", "/a~01"] {
            make(pointer).validate().unwrap();
        }
        let mut state = MatchState::default();
        assert!(state.feed(
            &make("/a~1b/~0key"),
            "rose",
            &event(1, "answer", serde_json::json!({"a/b":{"~key":true}}))
        ));
    }

    #[test]
    fn text_pointer_distinguishes_world_output_from_the_sent_command() {
        let matcher: Matcher = serde_json::from_value(serde_json::json!({
            "kind":"command-result","contains":"Flower Shop","text_pointer":"/output"
        }))
        .unwrap();
        matcher.validate().unwrap();
        let mut state = MatchState::default();
        let denied = event(
            1,
            "command-result",
            serde_json::json!({
                "command":"say Flower Shop","output":"Permission denied."
            }),
        );
        assert!(!state.feed(&matcher, "rose", &denied));
        let mut legacy = matcher.clone();
        legacy.text_pointer = None;
        assert!(state.feed(&legacy, "rose", &denied));
        for output in [Value::Null, true.into(), serde_json::json!(["Flower Shop"])] {
            assert!(!state.feed(
                &matcher,
                "rose",
                &event(2, "command-result", serde_json::json!({"output":output}))
            ));
        }
        assert!(!state.feed(
            &matcher,
            "rose",
            &event(
                3,
                "command-result",
                serde_json::json!({"command":"Flower Shop"})
            )
        ));
        assert!(state.feed(
            &matcher,
            "rose",
            &event(
                4,
                "command-result",
                serde_json::json!({"command":"east","output":"Flower Shop(#4)"})
            )
        ));
        for value in [
            serde_json::json!({"kind":"command-result","text_pointer":"/output"}),
            serde_json::json!({"kind":"command-result","contains":"Flower Shop","text_pointer":"/wrong~2"}),
        ] {
            assert!(
                serde_json::from_value::<Matcher>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}
