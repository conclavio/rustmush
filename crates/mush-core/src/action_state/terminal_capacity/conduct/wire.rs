use super::*;

fn text<'a>(v: &'a Value, n: &str) -> Result<&'a str, ActionStateError> {
    field(v, n)?
        .as_str()
        .ok_or_else(|| refused("Conduct string"))
}
fn checked_digest(v: &Value) -> Result<(), ActionStateError> {
    let _: DefinitionDigest = parsed(v)?;
    Ok(())
}
fn artifact(v: &Value, max: u64) -> Result<(), ActionStateError> {
    shape(v, &["sha256", "bytes"])?;
    checked_digest(field(v, "sha256")?)?;
    if !(1..=max).contains(&number(v, "bytes")?) {
        return Err(refused("Conduct artifact bound"));
    }
    Ok(())
}
fn decimal(v: &Value, maximum: u64) -> Result<(), ActionStateError> {
    let s = v.as_str().ok_or_else(|| refused("Conduct identifier"))?;
    let n = s.parse::<u64>().map_err(bad)?;
    if n == 0 || n > maximum || n.to_string() != s {
        return Err(refused("Conduct canonical identifier"));
    }
    Ok(())
}
#[derive(Clone)]
pub(super) struct Job {
    pub raw: Value,
    pub id: CommitToken,
    pub action: DefinitionRef,
    pub manifest: DefinitionRef,
    pub maximum: u64,
    pub policy: DefinitionDigest,
    pub subject: DefinitionDigest,
}
impl Job {
    pub fn parse(raw: Value, id: CommitToken) -> Result<Self, ActionStateError> {
        shape(
            &raw,
            &[
                "format",
                "operation",
                "action",
                "manifest",
                "subject",
                "admission",
                "at_utc",
            ],
        )?;
        let action: DefinitionRef = parsed(field(&raw, "action")?)?;
        let manifest: DefinitionRef = parsed(field(&raw, "manifest")?)?;
        if number(&raw, "format")? != 1
            || parsed::<CommitToken>(field(&raw, "operation")?)? != id
            || action != protocol::action(&manifest, 0)?
            || field(&raw, "at_utc")?.as_i64().is_none()
        {
            return Err(refused("Conduct immutable admission identity"));
        }
        let s = field(&raw, "subject")?;
        shape(
            s,
            &[
                "charter", "line", "filed", "revision", "rating", "worker", "profile", "role",
            ],
        )?;
        for (n, maximum) in [
            ("charter", u64::from(u32::MAX)),
            ("line", u64::MAX),
            ("filed", u64::MAX),
        ] {
            decimal(field(s, n)?, maximum)?;
        }
        let profile = text(s, "profile")?;
        if profile.is_empty()
            || profile.len() > 64
            || profile.trim() != profile
            || profile.chars().any(char::is_control)
            || !["escort", "transit"].contains(&text(s, "role")?)
        {
            return Err(refused("Conduct finite role/profile"));
        }
        let revision = field(s, "revision")?;
        shape(revision, &["format", "revision", "source"])?;
        if number(revision, "format")? != 1 {
            return Err(refused("Conduct revision format"));
        }
        checked_digest(field(revision, "source")?)?;
        // The registered Roll/artifact owners validate exact source key and hash
        // domains; Core retains the complete immutable tuple without deriving ACL.
        let worker = field(s, "worker")?;
        shape(
            worker,
            &[
                "revision",
                "envelope",
                "schema",
                "definition",
                "expanded",
                "declared",
                "role",
            ],
        )?;
        if field(worker, "revision")? != revision
            || !["I", "II", "III"].contains(&text(worker, "declared")?)
            || text(worker, "role")? != "combat"
        {
            return Err(refused("Conduct exact worker subject"));
        }
        for n in ["envelope", "schema", "definition", "expanded"] {
            checked_digest(field(worker, n)?)?;
        }
        let rating = field(s, "rating")?;
        shape(rating, &["job", "id", "schema", "receipt_digest"])?;
        if parsed::<CommitToken>(field(rating, "job")?)? == id || number(rating, "schema")? != 1 {
            return Err(refused("Conduct distinct birth rating"));
        }
        for n in ["id", "receipt_digest"] {
            checked_digest(field(rating, n)?)?;
        }
        canonical_json_bytes_with_limit(s, 4096).map_err(bad)?;
        let a = field(&raw, "admission")?;
        shape(
            a,
            &[
                "account_binding",
                "character",
                "run",
                "nonce",
                "subject",
                "policy",
                "schedule",
                "budget",
                "start",
                "maximum_unit_ms",
            ],
        )?;
        for n in ["account_binding", "subject", "policy", "schedule", "budget"] {
            checked_digest(field(a, n)?)?;
        }
        let maximum = number(a, "maximum_unit_ms")?;
        let subject = parsed(field(a, "subject")?)?;
        let policy = parsed(field(a, "policy")?)?;
        if parsed::<crate::ObjId>(field(a, "character")?)? != crate::GOD
            || number(a, "run")? == 0
            || number(a, "nonce")? == 0
            || maximum == 0
            || maximum > MAX_CHARGE
            || subject
                != digest(
                    "tacsim.conduct.subject.v1",
                    &json!([worker, field(s, "profile")?, field(s, "role")?]),
                    4096,
                )?
        {
            return Err(refused("Conduct captured owner admission"));
        }
        artifact(field(a, "start")?, 16 * 1024 * 1024)?;
        canonical_json_bytes_with_limit(a, 2048).map_err(bad)?;
        canonical_json_bytes_with_limit(&raw, 8192 - 1024).map_err(bad)?;
        Ok(Self {
            raw,
            id,
            action,
            manifest,
            maximum,
            policy,
            subject,
        })
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ActionStateError> {
        digest("roll.conduct.job.v1", &self.raw, 8192)
    }
}
#[derive(Clone)]
pub(super) struct Execution {
    pub raw: Value,
    pub operation: CommitToken,
    pub action: DefinitionRef,
    pub phase: String,
    pub sequence: u64,
    pub index: u64,
    pub simulated: u64,
    pub charged: u64,
    pub flight: Option<Value>,
    pub artifact: Option<Value>,
}
impl Execution {
    pub fn terminal(&self) -> bool {
        ["complete", "failed", "cancelled", "interrupted"].contains(&self.phase.as_str())
    }
    pub fn parse(raw: Value, job: &Job) -> Result<Self, ActionStateError> {
        shape(
            &raw,
            &["format", "job", "updated_by", "action", "observation"],
        )?;
        let operation = parsed(field(&raw, "updated_by")?)?;
        let action = parsed(field(&raw, "action")?)?;
        let transition = protocol::action_index(&job.manifest, &action)?;
        if number(&raw, "format")? != 1 || parsed::<CommitToken>(field(&raw, "job")?)? != job.id {
            return Err(refused("Conduct execution identity"));
        }
        let o = field(&raw, "observation")?;
        shape(
            o,
            &[
                "phase",
                "next_sequence",
                "cursor",
                "charged_unit_ms",
                "flight",
                "result",
            ],
        )?;
        let phase = text(o, "phase")?.to_owned();
        if ![
            "queued",
            "running",
            "cancel_requested",
            "complete",
            "failed",
            "cancelled",
            "interrupted",
        ]
        .contains(&phase.as_str())
        {
            return Err(refused("Conduct phase"));
        }
        let sequence = number(o, "next_sequence")?;
        let cursor = field(o, "cursor")?;
        shape(cursor, &["index", "simulated_ms"])?;
        let index = number(cursor, "index")?;
        let simulated = number(cursor, "simulated_ms")?;
        let charged = number(o, "charged_unit_ms")?;
        let flight = (!field(o, "flight")?.is_null())
            .then(|| field(o, "flight").cloned())
            .transpose()?;
        let result = (!field(o, "result")?.is_null())
            .then(|| field(o, "result").cloned())
            .transpose()?;
        let v = Self {
            raw: raw.clone(),
            operation,
            action,
            phase,
            sequence,
            index,
            simulated,
            charged,
            flight,
            artifact: result,
        };
        if sequence == 0
            || index >= 60
            || simulated > 121_000
            || charged > job.maximum
            || (v.terminal() && v.flight.is_some())
            || (v.phase == "complete") != v.artifact.is_some()
            || (v.phase == "queued") != (job.id == operation)
            || (transition == 0) != (v.phase == "queued")
        {
            return Err(refused("Conduct execution phase affinity"));
        }
        if v.phase == "queued"
            && (sequence != 1 || index != 0 || simulated != 0 || charged != 0 || v.flight.is_some())
        {
            return Err(refused("Conduct exact queued state"));
        }
        if v.phase == "complete" && (index != 59 || simulated < 120_000) {
            return Err(refused("Conduct complete trial extent"));
        }
        if let Some(a) = &v.artifact {
            artifact(a, 262144)?;
        }
        if let Some(f) = &v.flight {
            shape(
                f,
                &[
                    "sequence",
                    "utc_day",
                    "allowance_unit_ms",
                    "units",
                    "native_ms",
                    "cursor",
                ],
            )?;
            let units = number(f, "units")?;
            let native = number(f, "native_ms")?;
            let allowance = number(f, "allowance_unit_ms")?;
            if !(3..=4).contains(&units)
                || !(50..=1000).contains(&native)
                || allowance == 0
                || allowance > units * native * 8
                || !allowance.is_multiple_of(units)
                || allowance > charged
                || number(f, "sequence")?.checked_add(1) != Some(sequence)
                || field(f, "cursor")? != cursor
                || field(f, "utc_day")?.as_i64().is_none()
                || !["running", "cancel_requested"].contains(&v.phase.as_str())
            {
                return Err(refused("Conduct real flight shape"));
            }
        }
        canonical_json_bytes_with_limit(&raw, 4096 - 1024).map_err(bad)?;
        Ok(v)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ActionStateError> {
        digest("roll.conduct.execution.v1", &self.raw, 4096)
    }
}
pub(super) fn result(raw: &Value, job: &Job, e: &Execution) -> Result<(), ActionStateError> {
    shape(
        raw,
        &[
            "format",
            "job",
            "operation",
            "action",
            "job_digest",
            "execution_digest",
            "phase",
            "artifact",
            "measurement",
        ],
    )?;
    if number(raw, "format")? != 1
        || !e.terminal()
        || e.operation == job.id
        || field(raw, "job")? != &json!(job.id)
        || field(raw, "operation")? != &json!(e.operation)
        || field(raw, "action")? != &json!(e.action)
        || field(raw, "job_digest")? != &json!(job.digest()?)
        || field(raw, "execution_digest")? != &json!(e.digest()?)
        || text(raw, "phase")? != e.phase
        || field(raw, "artifact")? != e.artifact.as_ref().unwrap_or(&Value::Null)
    {
        return Err(refused("Conduct complete result joins"));
    }
    let m = field(raw, "measurement")?;
    if e.phase == "complete" {
        let a = field(raw, "artifact")?;
        artifact(a, 262144)?;
        shape(
            m,
            &[
                "digest",
                "policy",
                "subject",
                "charged_unit_ms",
                "paired_cases",
                "passed",
            ],
        )?;
        checked_digest(field(m, "digest")?)?;
        if field(m, "policy")? != &json!(job.policy)
            || field(m, "subject")? != &json!(job.subject)
            || number(m, "charged_unit_ms")? != e.charged
            || e.charged == 0
            || number(m, "paired_cases")? != 30
            || field(m, "passed")?.as_bool().is_none()
        {
            return Err(refused("Conduct exact measurement"));
        }
    } else if !m.is_null() || !field(raw, "artifact")?.is_null() {
        return Err(refused("Conduct noncomplete cannot qualify"));
    }
    canonical_json_bytes_with_limit(raw, 4096 - 1024).map_err(bad)?;
    Ok(())
}
fn envelope(
    job: CommitToken,
    f: &str,
    bytes: usize,
    value: &Value,
) -> Result<(), ActionStateError> {
    let d = protocol::descriptor(f, bytes)?;
    let e = json!({"family":{"owner":"roll","id":f},"key":job.to_hex(),"schema":{"id":d.value_schema(),"version":1},"value":value});
    canonical_json_bytes_with_limit(&e, bytes).map_err(bad)?;
    Ok(())
}
pub(super) fn future_room(view: &View<'_>, job: &Job) -> Result<(), ActionStateError> {
    for (f, bytes) in FAMILIES {
        if let Some(v) = get(view, f, job.id)? {
            envelope(job.id, f, bytes, &v)?;
        }
    }
    let operation = CommitToken::from_hex(&"f".repeat(32)).map_err(bad)?;
    let d = DefinitionDigest::from_hex(&"f".repeat(64)).map_err(bad)?;
    for i in 2..5 {
        let action = protocol::action(&job.manifest, i)?;
        // Fixed complete schemas have no unbounded future field. Probe their
        // longest scalar representations including both optional result values.
        let artifact = json!({"sha256":d,"bytes":262144});
        let execution = json!({"format":1,"job":job.id,"updated_by":operation,"action":action,
            "observation":{"phase":"cancel_requested","next_sequence":u64::MAX,"cursor":{"index":59,"simulated_ms":121000},
                "charged_unit_ms":MAX_CHARGE,"flight":null,"result":artifact}});
        let result = json!({"format":1,"job":job.id,"operation":operation,"action":action,"job_digest":job.digest()?,
            "execution_digest":d,"phase":"interrupted","artifact":artifact,"measurement":{"digest":d,"policy":job.policy,
                "subject":job.subject,"charged_unit_ms":MAX_CHARGE,"paired_cases":30,"passed":false}});
        envelope(job.id, EXECUTION, 4096, &execution)?;
        envelope(job.id, RESULT, 4096, &result)?;
    }
    Ok(())
}
