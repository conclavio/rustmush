//! Durable request accounting owned by the external bot. Provider charges are
//! retained even when a response cannot be used as a decision.
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    Result,
    config::{Model, Ssh},
    models::Selection,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub persistent: bool,
    pub directory: PathBuf,
    pub max_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            persistent: true,
            directory: ".mush-bot/costs".into(),
            max_bytes: 64_000_000,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(1_000_000..=1_000_000_000).contains(&self.max_bytes) {
            return Err("costs.max_bytes must be in 1000000..1000000000".into());
        }
        Ok(())
    }
}

/// Exact nonnegative decimal amounts with up to 18 fractional places. Serialize
/// as decimal strings so JSON consumers do not silently round currency totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(u128);

impl Amount {
    const SCALE: u128 = 1_000_000_000_000_000_000;

    fn parse(text: &str) -> Option<Self> {
        if text.len() > 100 || text.starts_with(['-', '+']) {
            return None;
        }
        let (mantissa, exponent) = text
            .split_once(['e', 'E'])
            .map_or(Some((text, 0)), |(m, e)| Some((m, e.parse::<i32>().ok()?)))?;
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if whole.is_empty()
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut digits = format!("{whole}{fraction}");
        let shift = 18_i32
            .checked_add(exponent)?
            .checked_sub(fraction.len() as i32)?;
        if shift < 0 {
            let remove = usize::try_from(shift.checked_neg()?).ok()?;
            if remove > digits.len() {
                return (digits.bytes().all(|b| b == b'0')).then_some(Self(0));
            }
            if !digits[digits.len() - remove..].bytes().all(|b| b == b'0') {
                return None;
            }
            digits.truncate(digits.len() - remove);
        }
        let value = if digits.is_empty() {
            0
        } else {
            digits.parse::<u128>().ok()?
        };
        if value == 0 {
            return Some(Self(0));
        }
        Some(Self(
            value.checked_mul(10_u128.checked_pow(shift.max(0) as u32)?)?,
        ))
    }

    fn value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => Self::parse(&n.to_string()),
            Value::String(s) => Self::parse(s),
            _ => None,
        }
    }

    pub(crate) fn add(&mut self, other: Self) -> Result<()> {
        self.0 = self
            .0
            .checked_add(other.0)
            .ok_or("cost total exceeds decimal accounting capacity")?;
        Ok(())
    }

    pub(crate) fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let whole = self.0 / Self::SCALE;
        let fraction = self.0 % Self::SCALE;
        if fraction == 0 {
            write!(f, "{whole}")
        } else {
            write!(
                f,
                "{whole}.{}",
                format!("{fraction:018}").trim_end_matches('0')
            )
        }
    }
}
impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid decimal cost"))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub generation_id: Option<String>,
    pub response_model: Option<String>,
    pub provider: Option<String>,
    pub currency: Option<String>,
    pub cost: Option<Amount>,
    pub reported_cost: Option<String>,
    pub upstream_inference_cost: Option<Amount>,
    pub is_byok: Option<bool>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
}

impl Receipt {
    pub fn from_body(body: &Value, model: &Model) -> Self {
        let usage = &body["usage"];
        let text = |key: &str| {
            body[key]
                .as_str()
                .filter(|s| s.len() <= 512 && !s.chars().any(char::is_control))
                .map(str::to_owned)
        };
        Self {
            generation_id: text("id"),
            response_model: text("model"),
            provider: text("provider"),
            currency: model.currency(),
            cost: Amount::value(&usage["cost"]),
            reported_cost: match &usage["cost"] {
                Value::Number(n) => Some(n.to_string()),
                Value::String(s) if s.len() <= 100 => Some(s.clone()),
                _ => None,
            },
            upstream_inference_cost: Amount::value(
                &usage["cost_details"]["upstream_inference_cost"],
            ),
            is_byok: usage["is_byok"].as_bool(),
            prompt_tokens: usage["prompt_tokens"].as_u64(),
            completion_tokens: usage["completion_tokens"].as_u64(),
            total_tokens: usage["total_tokens"].as_u64(),
            reasoning_tokens: usage["completion_tokens_details"]["reasoning_tokens"].as_u64(),
            cached_tokens: usage["prompt_tokens_details"]["cached_tokens"].as_u64(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptType {
    GoalDecision,
    Conversation,
    RunnerQuestion,
    StructuredProbe,
    DecisionReplay,
    CharacterAuthoring,
    GroupAuthoring,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub session: u64,
    pub model: Selection,
    pub prompt_type: PromptType,
    pub prompt_revision: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Pending,
    Response,
    Error,
}

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Event {
    Header {
        version: u32,
        owner: String,
    },
    Session {
        session: u64,
    },
    Started(Request),
    Finished {
        request: u64,
        outcome: Outcome,
        receipt: Option<Receipt>,
        elapsed_ms: u64,
    },
    Cancelled {
        request: u64,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    seq: u64,
    timestamp_ms: u128,
    event: Event,
}

struct Entry {
    request: Request,
    outcome: Outcome,
    receipt: Option<Receipt>,
    cancelled: bool,
    elapsed_ms: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Totals {
    requests: usize,
    responses: usize,
    errors: usize,
    cancelled: usize,
    pending: usize,
    interrupted: usize,
    known_cost_requests: usize,
    unknown_cost_requests: usize,
    /// Provider-reported account charges. Upstream cost is a separate metric.
    provider_cost_by_currency: BTreeMap<String, Amount>,
    upstream_inference_cost_by_currency: BTreeMap<String, Amount>,
    prompt_tokens: u128,
    completion_tokens: u128,
    total_tokens: u128,
    reasoning_tokens: u128,
    cached_tokens: u128,
    elapsed_ms: u128,
}

impl Totals {
    fn merge(&mut self, other: &Self) -> Result<()> {
        macro_rules! add { ($($field:ident),*) => { $(self.$field = self.$field.checked_add(other.$field).ok_or("cost aggregation overflow")?;)* }; }
        add!(
            requests,
            responses,
            errors,
            cancelled,
            pending,
            interrupted,
            known_cost_requests,
            unknown_cost_requests,
            prompt_tokens,
            completion_tokens,
            total_tokens,
            reasoning_tokens,
            cached_tokens,
            elapsed_ms
        );
        for (currency, value) in &other.provider_cost_by_currency {
            self.provider_cost_by_currency
                .entry(currency.clone())
                .or_default()
                .add(*value)?;
        }
        for (currency, value) in &other.upstream_inference_cost_by_currency {
            self.upstream_inference_cost_by_currency
                .entry(currency.clone())
                .or_default()
                .add(*value)?;
        }
        Ok(())
    }

    fn include(&mut self, entry: &Entry, session: u64) -> Result<()> {
        self.requests += 1;
        self.cancelled += usize::from(entry.cancelled);
        match entry.outcome {
            Outcome::Response => self.responses += 1,
            Outcome::Error => self.errors += 1,
            Outcome::Pending if entry.request.session == session && !entry.cancelled => {
                self.pending += 1
            }
            Outcome::Pending if !entry.cancelled => self.interrupted += 1,
            _ => {}
        }
        self.elapsed_ms += u128::from(entry.elapsed_ms);
        let known = entry
            .receipt
            .as_ref()
            .and_then(|r| Some((r.currency.as_ref()?, r.cost?)));
        if let Some((currency, cost)) = known {
            self.known_cost_requests += 1;
            self.provider_cost_by_currency
                .entry(currency.clone())
                .or_default()
                .add(cost)?;
        } else {
            self.unknown_cost_requests += 1;
        }
        if let Some(r) = &entry.receipt {
            if let (Some(currency), Some(cost)) = (&r.currency, r.upstream_inference_cost) {
                self.upstream_inference_cost_by_currency
                    .entry(currency.clone())
                    .or_default()
                    .add(cost)?;
            }
            self.prompt_tokens += u128::from(r.prompt_tokens.unwrap_or(0));
            self.completion_tokens += u128::from(r.completion_tokens.unwrap_or(0));
            self.total_tokens += u128::from(r.total_tokens.unwrap_or(0));
            self.reasoning_tokens += u128::from(r.reasoning_tokens.unwrap_or(0));
            self.cached_tokens += u128::from(r.cached_tokens.unwrap_or(0));
        }
        Ok(())
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Breakdown {
    totals: Totals,
    by_model: BTreeMap<String, Totals>,
    by_profile: BTreeMap<String, Totals>,
    by_prompt_type: BTreeMap<String, Totals>,
}

impl Breakdown {
    /// Complete, single-currency session charges for offline regression checks.
    /// A received cancellation receipt can be known; an unreceived charge cannot.
    pub(crate) fn complete_provider_cost(&self, currency: &str) -> Option<(usize, Amount)> {
        let totals = &self.totals;
        if totals.known_cost_requests != totals.requests
            || totals.unknown_cost_requests != 0
            || totals.pending != 0
            || totals.interrupted != 0
            || totals.responses.checked_add(totals.errors)? != totals.requests
            || totals.cancelled > totals.requests
            || totals
                .provider_cost_by_currency
                .keys()
                .any(|key| key != currency)
        {
            return None;
        }
        let amount = match totals.provider_cost_by_currency.get(currency) {
            Some(amount) => *amount,
            None if totals.requests == 0 => Amount::default(),
            None => return None,
        };
        if totals.requests == 0 && amount != Amount::default() {
            return None;
        }
        Some((totals.requests, amount))
    }

    pub fn merge(&mut self, other: &Self) -> Result<()> {
        self.totals.merge(&other.totals)?;
        for (target, source) in [
            (&mut self.by_model, &other.by_model),
            (&mut self.by_profile, &other.by_profile),
            (&mut self.by_prompt_type, &other.by_prompt_type),
        ] {
            for (key, value) in source {
                target.entry(key.clone()).or_default().merge(value)?;
            }
        }
        Ok(())
    }

    fn include(&mut self, entry: &Entry, session: u64) -> Result<()> {
        self.totals.include(entry, session)?;
        let model = entry
            .receipt
            .as_ref()
            .and_then(|r| r.response_model.as_ref())
            .unwrap_or(&entry.request.model.settings.name);
        self.by_model
            .entry(model.clone())
            .or_default()
            .include(entry, session)?;
        self.by_profile
            .entry(entry.request.model.profile.clone())
            .or_default()
            .include(entry, session)?;
        let kind = serde_json::to_value(entry.request.prompt_type)?
            .as_str()
            .unwrap()
            .to_owned();
        self.by_prompt_type
            .entry(kind)
            .or_default()
            .include(entry, session)?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Report {
    pub file: Option<PathBuf>,
    pub session_id: u64,
    pub sessions_recorded: usize,
    pub session: Breakdown,
    pub all_sessions: Breakdown,
}

pub struct Ledger {
    file: Option<File>,
    path: Option<PathBuf>,
    owner: String,
    session: u64,
    sessions: usize,
    seq: u64,
    bytes: u64,
    max_bytes: u64,
    entries: BTreeMap<u64, Entry>,
}

impl Ledger {
    pub fn open(settings: &Settings, ssh: &Ssh, namespace: &str) -> Result<Self> {
        settings.validate()?;
        let owner =
            serde_json::to_string(&(namespace, ssh.host.to_lowercase(), ssh.port, &ssh.username))?;
        let hash = owner.bytes().fold(0xcbf29ce484222325_u64, |n, b| {
            (n ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        let path = settings
            .persistent
            .then(|| settings.directory.join(format!("{hash:016x}.jsonl")));
        let mut ledger = Self {
            file: None,
            path: path.clone(),
            owner,
            session: 0,
            sessions: 0,
            seq: 0,
            bytes: 0,
            max_bytes: settings.max_bytes,
            entries: BTreeMap::new(),
        };
        if let Some(path) = path {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?;
            file.try_lock().map_err(|_| "cost ledger is already in use or cannot be locked; use a separate character/namespace")?;
            let mut text = String::new();
            (&mut file)
                .take(settings.max_bytes + 1)
                .read_to_string(&mut text)?;
            if text.len() as u64 > settings.max_bytes || (!text.is_empty() && !text.ends_with('\n'))
            {
                return Err("cost ledger is oversized or has an incomplete record; preserve it before repair".into());
            }
            for line in text.lines() {
                let record: Record = serde_json::from_str(line)
                    .map_err(|_| "invalid cost ledger record; preserve it before repair")?;
                ledger.apply(record)?;
            }
            ledger.bytes = text.len() as u64;
            file.seek(SeekFrom::End(0))?;
            ledger.file = Some(file);
        }
        if ledger.seq == 0 {
            ledger.append(Event::Header {
                version: 1,
                owner: ledger.owner.clone(),
            })?;
        }
        ledger.append(Event::Session {
            session: ledger.seq + 1,
        })?;
        Ok(ledger)
    }

    fn apply(&mut self, record: Record) -> Result<()> {
        if record.seq != self.seq + 1 {
            return Err("cost ledger sequence is invalid".into());
        }
        match record.event {
            Event::Header { version: 1, owner } if self.seq == 0 && owner == self.owner => {}
            Event::Session { session } if self.seq > 0 && session == record.seq => {
                self.session = session;
                self.sessions += 1;
            }
            Event::Started(request)
                if self.session > 0
                    && request.session == self.session
                    && request.id == record.seq =>
            {
                request.model.settings.validate()?;
                self.entries.insert(
                    request.id,
                    Entry {
                        request,
                        outcome: Outcome::Pending,
                        receipt: None,
                        cancelled: false,
                        elapsed_ms: 0,
                    },
                );
            }
            Event::Finished {
                request,
                outcome,
                receipt,
                elapsed_ms,
            } if outcome != Outcome::Pending => {
                let entry = self
                    .entries
                    .get_mut(&request)
                    .ok_or("cost receipt has no request")?;
                if entry.outcome != Outcome::Pending {
                    return Err("duplicate cost receipt".into());
                }
                entry.outcome = outcome;
                entry.receipt = receipt;
                entry.elapsed_ms = elapsed_ms;
            }
            Event::Cancelled { request } => {
                self.entries
                    .get_mut(&request)
                    .ok_or("cost cancellation has no request")?
                    .cancelled = true
            }
            _ => return Err("cost ledger has invalid ownership, version or event order".into()),
        }
        self.seq = record.seq;
        Ok(())
    }

    fn append(&mut self, event: Event) -> Result<()> {
        // Reject invalid updates before writing anything, especially duplicate
        // receipts that would otherwise make the next startup reject the file.
        match &event {
            Event::Header { version: 1, owner } if self.seq == 0 && owner == &self.owner => {}
            Event::Session { session } if self.seq > 0 && *session == self.seq + 1 => {}
            Event::Started(request)
                if self.session > 0
                    && request.session == self.session
                    && request.id == self.seq + 1 =>
            {
                request.model.settings.validate()?
            }
            Event::Finished {
                request, outcome, ..
            } if *outcome != Outcome::Pending
                && self
                    .entries
                    .get(request)
                    .is_some_and(|e| e.outcome == Outcome::Pending) => {}
            Event::Cancelled { request } if self.entries.contains_key(request) => {}
            _ => return Err("invalid cost ledger update or duplicate receipt".into()),
        }
        let reserve = if matches!(event, Event::Started(_)) {
            16_000
        } else {
            0
        };
        let record = Record {
            seq: self.seq + 1,
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
            event,
        };
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        if self.bytes + bytes.len() as u64 + reserve > self.max_bytes {
            return Err(
                "cost ledger reached costs.max_bytes; no further model requests can start".into(),
            );
        }
        if let Some(file) = &mut self.file {
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_data()?;
        }
        self.bytes += bytes.len() as u64;
        self.apply(record)
    }

    pub fn start(
        &mut self,
        model: Selection,
        prompt_type: PromptType,
        prompt_revision: String,
    ) -> Result<Request> {
        let request = Request {
            id: self.seq + 1,
            session: self.session,
            model,
            prompt_type,
            prompt_revision,
        };
        self.append(Event::Started(request.clone()))?;
        Ok(request)
    }

    pub fn finish(
        &mut self,
        request: u64,
        outcome: Outcome,
        receipt: Option<Receipt>,
        elapsed_ms: u64,
    ) -> Result<()> {
        self.append(Event::Finished {
            request,
            outcome,
            receipt,
            elapsed_ms,
        })
    }

    pub fn cancel(&mut self, request: u64) -> Result<()> {
        self.append(Event::Cancelled { request })
    }

    pub fn report(&self) -> Result<Report> {
        let mut report = Report {
            file: self.path.clone(),
            session_id: self.session,
            sessions_recorded: self.sessions,
            session: Breakdown::default(),
            all_sessions: Breakdown::default(),
        };
        for entry in self.entries.values() {
            report.all_sessions.include(entry, self.session)?;
            if entry.request.session == self.session {
                report.session.include(entry, self.session)?;
            }
        }
        Ok(report)
    }

    /// The budget gate uses the same accounting rules as the session report.
    /// Prior sessions never consume this session's request or spending limits.
    pub fn session_usage(&self) -> Result<crate::budget::Usage> {
        let mut totals = Totals::default();
        for entry in self.entries.values() {
            if entry.request.session == self.session {
                totals.include(entry, self.session)?;
            }
        }
        Ok(crate::budget::Usage {
            requests: totals.requests,
            unknown_cost_requests: totals.unknown_cost_requests,
            provider_cost_by_currency: totals.provider_cost_by_currency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model() -> Selection {
        Selection {
            profile: "careful".into(),
            settings: serde_json::from_value(json!({
                "endpoint":"https://openrouter.ai/api/v1/chat/completions", "name":"requested-model"
            }))
            .unwrap(),
        }
    }

    #[test]
    fn decimal_costs_are_exact_zero_is_known_and_missing_is_not_zero() {
        for (input, expected) in [
            ("0.1", "0.1"),
            ("2.4e-7", "0.00000024"),
            ("1e-18", "0.000000000000000001"),
            ("1.0000000000000000000", "1"),
        ] {
            assert_eq!(Amount::parse(input).unwrap().to_string(), expected);
        }
        let mut amount = Amount::parse("0.1").unwrap();
        amount.add(Amount::parse("0.2").unwrap()).unwrap();
        assert_eq!(amount.to_string(), "0.3");
        for invalid in ["-1", "NaN", "1e-19", "1e999", "1e-2147483648"] {
            assert!(Amount::parse(invalid).is_none());
        }
        let body: Value =
            serde_json::from_str(r#"{"usage":{"cost":0.100000000000000001}}"#).unwrap();
        assert_eq!(
            Receipt::from_body(&body, &model().settings)
                .cost
                .unwrap()
                .to_string(),
            "0.100000000000000001"
        );
        assert_eq!(
            Receipt::from_body(&json!({"usage":{"cost":0}}), &model().settings).cost,
            Some(Amount(0))
        );
        assert!(
            Receipt::from_body(&json!({"usage":{}}), &model().settings)
                .cost
                .is_none()
        );
    }

    #[test]
    fn regression_costs_require_complete_single_currency_receipts() {
        let amount = Amount::parse("0.000000000000000001").unwrap();
        let complete = Breakdown {
            totals: Totals {
                requests: 1,
                responses: 1,
                cancelled: 1,
                known_cost_requests: 1,
                provider_cost_by_currency: BTreeMap::from([("USD".into(), amount)]),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(complete.complete_provider_cost("USD"), Some((1, amount)));
        assert_eq!(complete.complete_provider_cost("EUR"), None);
        assert_eq!(
            Breakdown::default().complete_provider_cost("USD"),
            Some((0, Amount(0)))
        );
        let mut unknown = complete.clone();
        unknown.totals.known_cost_requests = 0;
        unknown.totals.unknown_cost_requests = 1;
        assert_eq!(unknown.complete_provider_cost("USD"), None);
        let mut absent_currency = complete.clone();
        absent_currency.totals.provider_cost_by_currency.clear();
        assert_eq!(absent_currency.complete_provider_cost("USD"), None);
        let mut mixed_currency = complete.clone();
        mixed_currency
            .totals
            .provider_cost_by_currency
            .insert("EUR".into(), Amount(0));
        assert_eq!(mixed_currency.complete_provider_cost("USD"), None);
        let mut pending = complete.clone();
        pending.totals.pending = 1;
        assert_eq!(pending.complete_provider_cost("USD"), None);
        let mut contradictory = complete;
        contradictory.totals.responses = 0;
        assert_eq!(contradictory.complete_provider_cost("USD"), None);
    }

    #[test]
    fn authoring_costs_remain_separate_and_include_unusable_drafts() {
        let mut ledger = Ledger::open(
            &Settings {
                persistent: false,
                ..Settings::default()
            },
            &Ssh::default(),
            "authoring-fixture",
        )
        .unwrap();
        for (prompt_type, expected_kind, amount, outcome) in [
            (
                PromptType::CharacterAuthoring,
                "character_authoring",
                "0.1",
                Outcome::Response,
            ),
            (
                PromptType::GroupAuthoring,
                "group_authoring",
                "0.2",
                Outcome::Error,
            ),
        ] {
            let request = ledger
                .start(model(), prompt_type, "draft-revision".into())
                .unwrap();
            let stored = serde_json::to_value(&request).unwrap();
            assert_eq!(stored["prompt_type"], expected_kind);
            let restored: Request = serde_json::from_value(stored).unwrap();
            assert_eq!(
                serde_json::to_value(restored.prompt_type).unwrap(),
                expected_kind
            );
            ledger
                .finish(
                    request.id,
                    outcome,
                    Some(Receipt::from_body(
                        &json!({"usage":{"cost":amount}}),
                        &model().settings,
                    )),
                    1,
                )
                .unwrap();
        }
        let report = serde_json::to_value(ledger.report().unwrap()).unwrap();
        let session = &report["session"];
        assert_eq!(session["totals"]["requests"], 2);
        assert_eq!(session["totals"]["provider_cost_by_currency"]["USD"], "0.3");
        assert_eq!(
            session["by_prompt_type"]["character_authoring"]["provider_cost_by_currency"]["USD"],
            "0.1"
        );
        assert_eq!(
            session["by_prompt_type"]["group_authoring"]["provider_cost_by_currency"]["USD"],
            "0.2"
        );
        assert_eq!(session["by_prompt_type"]["group_authoring"]["errors"], 1);
        assert!(session["by_prompt_type"]["goal_decision"].is_null());
    }

    #[test]
    fn session_budget_usage_counts_started_cancelled_and_late_receipts_exactly() {
        let mut ledger = Ledger::open(
            &Settings {
                persistent: false,
                ..Default::default()
            },
            &Ssh::default(),
            "budget-fixture",
        )
        .unwrap();
        assert_eq!(ledger.session_usage().unwrap().requests, 0);
        let request = ledger
            .start(model(), PromptType::GoalDecision, "revision".into())
            .unwrap();
        assert_eq!(ledger.session_usage().unwrap().requests, 1);
        assert_eq!(ledger.session_usage().unwrap().unknown_cost_requests, 1);
        ledger.cancel(request.id).unwrap();
        assert_eq!(ledger.session_usage().unwrap().unknown_cost_requests, 1);
        ledger.finish(
            request.id, Outcome::Error,
            Some(Receipt::from_body(&json!({"usage":{"cost":"0.1","cost_details":{"upstream_inference_cost":"99"}}}), &model().settings)),
            1,
        ).unwrap();
        let usage = ledger.session_usage().unwrap();
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.unknown_cost_requests, 0);
        assert_eq!(usage.provider_cost_by_currency["USD"].to_string(), "0.1");
        let second = ledger
            .start(model(), PromptType::RunnerQuestion, "revision".into())
            .unwrap();
        ledger
            .finish(
                second.id,
                Outcome::Response,
                Some(Receipt::from_body(
                    &json!({"usage":{"cost":"0.2"}}),
                    &model().settings,
                )),
                1,
            )
            .unwrap();
        let usage = ledger.session_usage().unwrap();
        assert_eq!(usage.requests, 2);
        assert_eq!(usage.provider_cost_by_currency["USD"].to_string(), "0.3");
        let limit: crate::budget::Settings =
            serde_json::from_value(json!({"max_cost_by_currency":{"USD":"0.3"}})).unwrap();
        assert!(!limit.evaluate(&usage, Some("USD")).unwrap().allowed);
    }

    #[test]
    fn costs_survive_restart_include_unusable_responses_and_preserve_cancellation_receipts() {
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-cost-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let settings = Settings {
            directory: directory.clone(),
            ..Settings::default()
        };
        let ssh = Ssh::default();
        let mut ledger = Ledger::open(&settings, &ssh, "fixture").unwrap();
        assert!(
            Ledger::open(&settings, &ssh, "fixture").is_err(),
            "one writer per bot ledger"
        );
        let first = ledger
            .start(model(), PromptType::GoalDecision, "revision-a".into())
            .unwrap();
        let receipt = Receipt::from_body(
            &json!({"id":"gen-first", "model":"routed-model", "usage":{"cost":0.1, "cost_details":{"upstream_inference_cost":0.08}, "total_tokens":7}}),
            &model().settings,
        );
        ledger
            .finish(first.id, Outcome::Error, Some(receipt), 4)
            .unwrap();
        ledger.cancel(first.id).unwrap(); // A pause arriving after receipt persistence.
        let before = std::fs::metadata(ledger.path.as_ref().unwrap())
            .unwrap()
            .len();
        assert!(ledger.finish(first.id, Outcome::Response, None, 0).is_err());
        assert_eq!(
            std::fs::metadata(ledger.path.as_ref().unwrap())
                .unwrap()
                .len(),
            before
        );
        let second = ledger
            .start(model(), PromptType::RunnerQuestion, "revision-b".into())
            .unwrap();
        ledger.cancel(second.id).unwrap();
        ledger
            .start(model(), PromptType::Conversation, "revision-a".into())
            .unwrap(); // Crash before receipt.
        let usage = ledger.session_usage().unwrap();
        assert_eq!(usage.requests, 3);
        assert_eq!(usage.unknown_cost_requests, 2);
        assert_eq!(usage.provider_cost_by_currency["USD"].to_string(), "0.1");
        drop(ledger);
        let mut ledger = Ledger::open(&settings, &ssh, "fixture").unwrap();
        let usage = ledger.session_usage().unwrap();
        assert_eq!(usage.requests, 0);
        assert_eq!(usage.unknown_cost_requests, 0);
        assert!(usage.provider_cost_by_currency.is_empty());
        let next = ledger
            .start(model(), PromptType::RunnerQuestion, "revision-b".into())
            .unwrap();
        ledger
            .finish(
                next.id,
                Outcome::Response,
                Some(Receipt::from_body(
                    &json!({"usage":{"cost":0}}),
                    &model().settings,
                )),
                2,
            )
            .unwrap();
        let usage = ledger.session_usage().unwrap();
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.unknown_cost_requests, 0);
        assert_eq!(usage.provider_cost_by_currency["USD"], Amount::default());
        let limit: crate::budget::Settings =
            serde_json::from_value(json!({"max_cost_by_currency":{"USD":"0.1"}})).unwrap();
        assert!(limit.evaluate(&usage, Some("USD")).unwrap().allowed);
        let report = serde_json::to_value(ledger.report().unwrap()).unwrap();
        assert_eq!(report["session"]["totals"]["known_cost_requests"], 1);
        assert_eq!(
            report["session"]["totals"]["provider_cost_by_currency"]["USD"],
            "0"
        );
        assert_eq!(
            report["all_sessions"]["totals"]["provider_cost_by_currency"]["USD"],
            "0.1"
        );
        assert_eq!(
            report["all_sessions"]["totals"]["upstream_inference_cost_by_currency"]["USD"],
            "0.08"
        );
        assert_eq!(report["all_sessions"]["totals"]["unknown_cost_requests"], 2);
        assert_eq!(report["all_sessions"]["totals"]["cancelled"], 2);
        assert_eq!(report["all_sessions"]["totals"]["interrupted"], 1);
        assert_eq!(
            report["all_sessions"]["by_model"]["routed-model"]["totals"],
            Value::Null
        );
        assert_eq!(
            report["all_sessions"]["by_model"]["routed-model"]["provider_cost_by_currency"]["USD"],
            "0.1"
        );
        assert_eq!(
            report["all_sessions"]["by_prompt_type"]["runner_question"]["requests"],
            2
        );
        drop(ledger);
        // Interrupted writes are rejected instead of silently shrinking totals.
        let path = std::fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{")
            .unwrap();
        assert!(Ledger::open(&settings, &ssh, "fixture").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
