//! Owned, checkpointed campaigns. Privileged fixture artifacts stay operator-side.
use crate::{
    Result,
    checkpoint::Snapshot,
    experiment::{load_json, revision, save_json},
    organization::identifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
};

pub const ADAPTER: &str = "rustmush-rehearsal-v1";
pub const CAPABILITIES: &str = "serialized_ingress;all_actor_ssh_fence;fixed_clock;no_rng;no_jobs;no_npcs;no_modules;no_external_clients";

pub(crate) fn new_epoch() -> Result<String> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    Ok(format!(
        "e{:x}-{:x}-{:x}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub fixture_program: PathBuf,
    #[serde(default)]
    pub fixture_args: Vec<String>,
    /// Optional immutable checkpoint to continue. The same campaign ledger remains in force.
    pub from_checkpoint: Option<PathBuf>,
    pub max_requests: usize,
    pub actors: Vec<Actor>,
    #[serde(default)]
    pub groups: Vec<crate::organization::Group>,
    pub branches: Vec<Branch>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub name: String,
    pub config: PathBuf,
    /// Requests reserved for this actor in EACH attempted branch; unused quota is not refunded.
    pub request_allowance: usize,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub name: String,
    /// Earlier branch in this record, or absent for the initial/from_checkpoint world.
    pub parent: Option<String>,
    #[serde(default)]
    pub interventions: Vec<Step>,
    #[serde(default)]
    pub steps: Vec<Step>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub actor: String,
    pub control: String,
    /// Event AFTER this control's started watermark. Omit to wait only for application.
    pub wait: Option<Wait>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Wait {
    pub kind: String,
    pub contains: Option<String>,
}
impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let mut value: Self = load_json(path, 2_000_000)?;
        let absolute = std::fs::canonicalize(path)?;
        let base = absolute
            .parent()
            .ok_or("campaign needs a parent directory")?;
        let resolve = |p: &mut PathBuf| {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        };
        resolve(&mut value.fixture_program);
        if let Some(p) = &mut value.from_checkpoint {
            resolve(p);
        }
        for a in &mut value.actors {
            resolve(&mut a.config);
        }
        for branch in &mut value.branches {
            for step in branch.interventions.iter_mut().chain(&mut branch.steps) {
                for verb in ["/tape ", "/probe "] {
                    if let Some(arg) = step.control.strip_prefix(verb) {
                        let mut path = PathBuf::from(arg.trim());
                        resolve(&mut path);
                        step.control = format!("{verb}{}", path.display());
                    }
                }
            }
        }
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !identifier(&self.id)
            || self.actors.is_empty()
            || self.actors.len() > 16
            || self.branches.is_empty()
            || self.branches.len() > 32
            || self.max_requests > 1_000_000
            || self.groups.len() > 8
            || (self.from_checkpoint.is_some() && !self.groups.is_empty())
        {
            return Err(
                "invalid campaign version, identifier or cast/branch/request limits".into(),
            );
        }
        let mut actors = BTreeSet::new();
        let mut account_names = BTreeSet::new();
        for actor in &self.actors {
            if !identifier(&actor.name)
                || !actors.insert(actor.name.clone())
                || !account_names.insert(actor.name.to_ascii_lowercase())
                || actor.request_allowance > 10_000
            {
                return Err(
                    "campaign actors require unique identifiers and request allowance <= 10000"
                        .into(),
                );
            }
            let config = crate::config::Config::load(&actor.config)?;
            config.validate()?;
            if !self.groups.is_empty() && config.affiliations.is_some() {
                return Err(
                    "campaign groups conflict with actor affiliations; choose one authored source"
                        .into(),
                );
            }
            if self.from_checkpoint.is_none() {
                let tape = config.model_tape.as_deref().ok_or("restored fixture campaigns currently require local model tapes; paid-provider admission is unsupported")?;
                crate::tape::Source::load(tape)?;
            }
        }
        let mut group_ids = BTreeSet::new();
        for group in &self.groups {
            group.validate()?;
            if !group_ids.insert(&group.id)
                || group.members.iter().any(|m| !actors.contains(&m.actor))
            {
                return Err("campaign groups need unique IDs and known actors".into());
            }
        }
        let mut names = BTreeSet::new();
        let mut directory_names = BTreeSet::new();
        for branch in &self.branches {
            if !identifier(&branch.name)
                || names.contains(&branch.name)
                || !directory_names.insert(branch.name.to_ascii_lowercase())
                || branch.parent.as_ref().is_some_and(|p| !names.contains(p))
            {
                return Err("branches need unique identifiers and an earlier parent (cycles/forward references unsupported)".into());
            }
            names.insert(branch.name.clone());
            if branch.steps.len() + branch.interventions.len() > 2000 {
                return Err("too many campaign controls".into());
            }
            for step in branch.interventions.iter().chain(&branch.steps) {
                let verb = step.control.split_whitespace().next().unwrap_or_default();
                if !actors.contains(&step.actor)
                    || step.control.len() > 16_000
                    || step.control.chars().any(char::is_control)
                    || ![
                        "/send",
                        "/goal",
                        "/role",
                        "/note",
                        "/remember",
                        "/step",
                        "/resume",
                        "/continue",
                        "/pause",
                        "/listen",
                        "/ask",
                        "/probe",
                        "/tape",
                        "/turns",
                        "/character-state",
                        "/state",
                        "/cost",
                        "/recall",
                        "/social",
                        "/route",
                        "/capture",
                        "/recap",
                    ]
                    .contains(&verb)
                {
                    return Err("unsupported campaign control or actor; checkpoint/quit/floor are coordinator-owned".into());
                }
            }
        }
        if let Some(path) = &self.from_checkpoint {
            Checkpoint::load(path)?;
        }
        Ok(())
    }
}

/// Operator-only adapter launch record. Never included in character knowledge.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureStart {
    pub version: u32,
    pub epoch: String,
    pub actors: Vec<String>,
    pub restore: Option<WorldSave>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub version: u32,
    pub adapter: String,
    pub capabilities: String,
    pub port: u16,
    pub host_key_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSave {
    pub version: u32,
    pub adapter: String,
    pub capabilities: String,
    pub epoch: String,
    pub actors: Vec<String>,
    pub frozen: bool,
    pub command_sequence: u64,
    pub logical_clock: i64,
    pub data: Value,
}
impl WorldSave {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.adapter != ADAPTER
            || self.capabilities != CAPABILITIES
            || !self.frozen
            || !identifier(&self.epoch)
            || self.actors.is_empty()
            || self.actors.len() > 16
            || self.actors.iter().collect::<BTreeSet<_>>().len() != self.actors.len()
            || serde_json::to_vec(self)?.len() > 8_000_000
        {
            return Err("unsupported or unfrozen world checkpoint: adapter capabilities/epoch/membership do not certify complete state".into());
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub path: PathBuf,
    pub id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub version: u32,
    pub id: String,
    pub mode: String,
    pub campaign: String,
    pub branch: String,
    pub parent: Option<Reference>,
    pub ledger: PathBuf,
    pub world: WorldSave,
    pub bots: BTreeMap<String, Snapshot>,
}
impl Checkpoint {
    pub fn seal(mut self) -> Result<Self> {
        self.id = self.digest()?;
        self.validate()?;
        Ok(self)
    }
    fn digest(&self) -> Result<String> {
        let mut data = serde_json::to_value(self)?;
        data["id"] = json!("");
        // v1 actor saves predate these fields. Reconstruct their original wire
        // representation for the immutable parent identity; validation below
        // permits only the injected defaults, never unbound new state.
        if let Some(bots) = data["bots"].as_object_mut() {
            for bot in bots.values_mut().filter(|b| b["version"] == 1) {
                bot.as_object_mut().unwrap().remove("social");
                bot["context"]
                    .as_object_mut()
                    .unwrap()
                    .remove("social_bytes");
                if let Some(groups) = bot
                    .get_mut("affiliations")
                    .and_then(Value::as_object_mut)
                    .and_then(|a| a.get_mut("groups"))
                    .and_then(Value::as_array_mut)
                {
                    for group in groups {
                        group.as_object_mut().unwrap().remove("background");
                    }
                }
            }
        }
        Ok(revision(&serde_json::to_vec(&data)?))
    }
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = load_json(path, 128_000_000)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.world.validate()?;
        if self.version != 1
            || self.mode != "restored_world_branch"
            || !identifier(&self.campaign)
            || !identifier(&self.branch)
            || self.id != self.digest()?
            || serde_json::to_vec(self)?.len() > 128_000_000
            || self.bots.keys().cloned().collect::<BTreeSet<_>>()
                != self.world.actors.iter().cloned().collect()
        {
            return Err("checkpoint identity/mode/hash/actor coverage mismatch".into());
        }
        for (actor, bot) in &self.bots {
            bot.validate()?;
            if bot.epoch != self.world.epoch || bot.actor != *actor || bot.tape.is_none() {
                return Err("world/bot epoch, identity or tape capability mismatch".into());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    attempt: String,
    max_requests: usize,
    requests: usize,
    provider_spend_usd: String,
}
fn reserve(path: &Path, attempt: &str, cap: usize, requests: usize) -> Result<usize> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    file.try_lock()
        .map_err(|_| "campaign accounting ledger is busy")?;
    if file.metadata()?.len() > 8_000_000 {
        return Err("campaign ledger too large".into());
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(
            "campaign ledger has an incomplete record; preserve and repair before continuing"
                .into(),
        );
    }
    let mut reserved = 0usize;
    for line in text.lines() {
        let entry: Reservation = serde_json::from_str(line)?;
        if entry.version != 1
            || entry.max_requests != cap
            || entry.provider_spend_usd != "0"
            || entry.attempt == attempt
        {
            return Err("campaign cap/ledger mismatch or duplicate attempt; restoration never replenishes the allowance".into());
        }
        reserved = reserved
            .checked_add(entry.requests)
            .ok_or("campaign quota overflow")?;
    }
    reserved = reserved
        .checked_add(requests)
        .ok_or("campaign quota overflow")?;
    if reserved > cap {
        return Err(
            "campaign request cap would be exceeded (failed/unused reservations are retained)"
                .into(),
        );
    }
    file.seek(SeekFrom::End(0))?;
    serde_json::to_writer(
        &mut file,
        &Reservation {
            version: 1,
            attempt: attempt.into(),
            max_requests: cap,
            requests,
            provider_spend_usd: "0".into(),
        },
    )?;
    writeln!(file)?;
    file.sync_all()?;
    Ok(reserved)
}

struct Player {
    child: Child,
    input: ChildStdin,
    events: mpsc::Receiver<Value>,
}
impl Player {
    async fn spawn(
        program: &Path,
        config: &Path,
        dir: &Path,
        epoch: &str,
        quota: usize,
        restore: Option<&Path>,
    ) -> Result<Self> {
        let mut cmd = Command::new(program);
        cmd.arg("--config")
            .arg(config)
            .arg("--paused")
            .arg("--json-events")
            .arg("--campaign-epoch")
            .arg(epoch)
            .arg("--campaign-quota")
            .arg(quota.to_string())
            .arg("--transcript")
            .arg(dir.join("trace.jsonl"))
            .arg("--report")
            .arg(dir.join("report.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(File::create(dir.join("stderr.txt"))?))
            .current_dir(dir)
            .env("MUSH_CAMPAIGN_FIXTURE_PASSWORD", "local-fixture-only")
            .kill_on_drop(true);
        if let Some(path) = restore {
            cmd.arg("--restore-bot").arg(path);
        }
        let mut child = cmd.spawn()?;
        let input = child.stdin.take().ok_or("missing child stdin")?;
        let output = child.stdout.take().ok_or("missing child stdout")?;
        let (tx, events) = mpsc::channel(1024);
        tokio::spawn(async move {
            let mut lines = BufReader::new(output).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let event = serde_json::from_str(&line)
                    .unwrap_or_else(|_| json!({"kind":"protocol-error"}));
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
        })
    }
    async fn send(&mut self, text: &str) -> Result<()> {
        self.input.write_all(format!("{text}\n").as_bytes()).await?;
        self.input.flush().await?;
        Ok(())
    }
    async fn event(&mut self) -> Result<Value> {
        let event = tokio::time::timeout(Duration::from_secs(35), self.events.recv())
            .await?
            .ok_or("bot exited before campaign boundary; inspect its stderr/trace")?;
        if ["control-error", "error", "protocol-error"]
            .contains(&event["kind"].as_str().unwrap_or_default())
        {
            return Err(format!("campaign bot error: {}", event["data"]).into());
        }
        Ok(event)
    }
    async fn until(&mut self, kind: &str) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
        loop {
            let e = tokio::time::timeout_at(deadline, self.event()).await??;
            if e["kind"] == kind {
                return Ok(e);
            }
        }
    }
    async fn step(&mut self, step: &Step, id: usize) -> Result<()> {
        let id = format!("campaign-{id}");
        self.send(&format!("/harness {id} {}", step.control))
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
        let (mut started, mut applied, mut matched) = (false, false, step.wait.is_none());
        loop {
            let e = tokio::time::timeout_at(deadline, self.event()).await??;
            if e["kind"] == "control-started" && e["data"]["id"] == id {
                started = true;
            }
            if started {
                if e["kind"] == "control-applied" && e["data"]["id"] == id {
                    applied = true;
                }
                if let Some(wait) = &step.wait
                    && e["kind"] == wait.kind
                    && wait
                        .contains
                        .as_ref()
                        .is_none_or(|s| e["data"].to_string().contains(s))
                {
                    matched = true;
                }
            }
            if applied && matched {
                return Ok(());
            }
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchResult {
    pub name: String,
    pub parent: Option<String>,
    pub checkpoint: Reference,
    pub directory: PathBuf,
    pub reserved_requests: usize,
    pub reports: BTreeMap<String, Value>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub version: u32,
    pub campaign: String,
    pub mode: String,
    pub ledger: PathBuf,
    pub provider_spend_usd: String,
    pub branches: Vec<BranchResult>,
}

pub async fn run(manifest: &Manifest, output: &Path, bot_program: &Path) -> Result<Summary> {
    manifest.validate()?;
    std::fs::create_dir(output)?;
    let output = std::fs::canonicalize(output)?;
    save_json(&output.join("manifest.json"), manifest)?;
    let initial = manifest
        .from_checkpoint
        .as_deref()
        .map(Checkpoint::load)
        .transpose()?;
    let ledger = initial
        .as_ref()
        .map_or_else(|| output.join("accounting.jsonl"), |c| c.ledger.clone());
    if initial.is_none() {
        File::options()
            .write(true)
            .create_new(true)
            .open(&ledger)?
            .sync_all()?;
    } else if !ledger.is_file() {
        return Err("original campaign accounting ledger is missing; restoration cannot recreate or reset it".into());
    }
    if initial.as_ref().is_some_and(|c| c.campaign != manifest.id) {
        return Err("continuation must retain campaign identity".into());
    }
    let mut summary = Summary {
        version: 1,
        campaign: manifest.id.clone(),
        mode: "restored_world_branch".into(),
        ledger: ledger.clone(),
        provider_spend_usd: "0".into(),
        branches: vec![],
    };
    for branch in &manifest.branches {
        let parent_ref = if let Some(parent) = &branch.parent {
            Some(
                summary
                    .branches
                    .iter()
                    .find(|b| b.name == *parent)
                    .ok_or("missing branch parent")?
                    .checkpoint
                    .clone(),
            )
        } else {
            initial.as_ref().map(|c| Reference {
                path: manifest.from_checkpoint.clone().unwrap(),
                id: c.id.clone(),
            })
        };
        let parent = parent_ref
            .as_ref()
            .map(|r| Checkpoint::load(&r.path))
            .transpose()?;
        if let (Some(p), Some(r)) = (&parent, &parent_ref)
            && p.id != r.id
        {
            return Err("parent checkpoint hash changed".into());
        }
        let result = run_branch(
            manifest,
            branch,
            &output,
            bot_program,
            &ledger,
            parent,
            parent_ref,
        )
        .await;
        match result {
            Ok(result) => {
                summary.branches.push(result);
            }
            Err(error) => {
                save_json(
                    &output.join("failure.json"),
                    &json!({"branch":branch.name,"error":error.to_string(),"completed":summary,"accounting_retained":true}),
                )?;
                return Err(error);
            }
        }
    }
    save_json(&output.join("summary.json"), &summary)?;
    Ok(summary)
}

async fn run_branch(
    manifest: &Manifest,
    branch: &Branch,
    output: &Path,
    program: &Path,
    ledger: &Path,
    parent: Option<Checkpoint>,
    parent_ref: Option<Reference>,
) -> Result<BranchResult> {
    let dir = output.join(&branch.name);
    std::fs::create_dir(&dir)?;
    let mut branch = branch.clone();
    for (index, step) in branch
        .interventions
        .iter_mut()
        .chain(&mut branch.steps)
        .enumerate()
    {
        for verb in ["/tape ", "/probe "] {
            if let Some(arg) = step.control.strip_prefix(verb) {
                let source: Value = if verb == "/tape " {
                    serde_json::to_value(crate::tape::Source::load(Path::new(arg.trim()))?)?
                } else {
                    serde_json::to_value(crate::probe::Spec::load(Path::new(arg.trim()))?)?
                };
                let path = dir.join(format!("control-input-{index}.json"));
                save_json(&path, &source)?;
                step.control = format!("{verb}{}", path.display());
            }
        }
    }
    save_json(&dir.join("branch.json"), &branch)?;
    let epoch = new_epoch()?;
    let quota = manifest
        .actors
        .iter()
        .map(|a| a.request_allowance)
        .sum::<usize>();
    let reserved = reserve(ledger, &epoch, manifest.max_requests, quota)?;
    let mut configs = BTreeMap::new();
    for a in &manifest.actors {
        let mut config = crate::config::Config::load(&a.config)?;
        config.ssh.username = a.name.clone();
        configs.insert(a.name.clone(), config);
    }
    if let Some(p) = &parent
        && p.bots.keys().collect::<Vec<_>>() != configs.keys().collect::<Vec<_>>()
    {
        return Err("restored cast must exactly match checkpoint membership".into());
    }
    let start = FixtureStart {
        version: 1,
        epoch: epoch.clone(),
        actors: configs.keys().cloned().collect(),
        restore: parent.as_ref().map(|p| p.world.clone()),
    };
    save_json(&dir.join("fixture-start.json"), &start)?;
    save_json(&dir.join("interventions.json"), &branch.interventions)?;
    let mut fixture = Command::new(&manifest.fixture_program)
        .args(&manifest.fixture_args)
        .env("MUSH_CAMPAIGN_START", dir.join("fixture-start.json"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(File::create(dir.join("fixture-stdout.txt"))?))
        .stderr(Stdio::from(File::create(dir.join("fixture-stderr.txt"))?))
        .current_dir(&dir)
        .kill_on_drop(true)
        .spawn()?;
    let endpoint_path = dir.join("endpoint.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !endpoint_path.exists() {
        if fixture.try_wait()?.is_some() || tokio::time::Instant::now() > deadline {
            return Err("fixture failed readiness; inspect fixture-stderr.txt".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let endpoint: Endpoint = load_json(&endpoint_path, 16_000)?;
    if endpoint.version != 1
        || endpoint.adapter != ADAPTER
        || endpoint.capabilities != CAPABILITIES
        || endpoint.port == 0
    {
        return Err("fixture cannot certify required checkpoint capabilities".into());
    }
    let mut players = BTreeMap::new();
    for actor in &manifest.actors {
        let actor_dir = dir.join(&actor.name);
        std::fs::create_dir(&actor_dir)?;
        let mut config = configs.remove(&actor.name).unwrap();
        config.ssh = crate::config::Ssh {
            port: endpoint.port,
            username: actor.name.clone(),
            password_env: Some("MUSH_CAMPAIGN_FIXTURE_PASSWORD".into()),
            host_key_sha256: Some(endpoint.host_key_sha256.clone()),
            ..Default::default()
        };
        config.memory.directory = actor_dir.join("memory");
        config.memory.file = None;
        config.memory.persistent = true;
        config.memory.namespace = epoch.clone();
        config.costs.directory = actor_dir.join("costs");
        config.costs.persistent = true;
        if let Some(snapshot) = parent.as_ref().map(|p| &p.bots[&actor.name]) {
            // Restoration is self-contained. Changed or deleted source documents
            // cannot replace inherited character knowledge or runtime policies.
            config.character = None;
            config.affiliations = None;
            config.knowledge.clear();
            config.prompts = None;
            config.context = None;
            config.debug = None;
            config.budget = None;
            config.model_profiles = None;
            config.model_profile = None;
            config.model = snapshot.model.as_ref().map(|m| m.settings.clone());
            config.conversation = None;
            let bootstrap_tape = actor_dir.join("restore-bootstrap-tape.json");
            save_json(
                &bootstrap_tape,
                &json!({"version":1,"replies":[{"error":"restore bootstrap placeholder must never execute"}]}),
            )?;
            config.model_tape = Some(bootstrap_tape);
        } else {
            if !manifest.groups.is_empty() {
                let path = actor_dir.join("affiliations.json");
                save_json(
                    &path,
                    &crate::organization::Affiliations::for_actor(&actor.name, &manifest.groups)?,
                )?;
                config.affiliations = Some(path);
            }
            freeze_config(&mut config, &actor_dir)?;
        }
        let mut conversation = if let Some(p) = &parent {
            p.bots[&actor.name].conversation.settings.clone()
        } else {
            config
                .conversation
                .as_deref()
                .map(crate::conversation::Settings::load)
                .transpose()?
                .unwrap_or_default()
        };
        conversation.scope.campaign = manifest.id.clone();
        conversation.scope.branch = branch.name.clone();
        config.conversation = Some(actor_dir.join("conversation.json"));
        save_json(config.conversation.as_ref().unwrap(), &conversation)?;
        let restore = parent.as_ref().map(|p| &p.bots[&actor.name]);
        let restore_path = actor_dir.join("restore.json");
        if let Some(snapshot) = restore {
            save_json(&restore_path, snapshot)?;
        }
        let config_path = actor_dir.join("bot.toml");
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&config_path)?;
        file.write_all(toml::to_string(&config)?.as_bytes())?;
        let player = Player::spawn(
            program,
            &config_path,
            &actor_dir,
            &epoch,
            actor.request_allowance,
            restore.map(|_| restore_path.as_path()),
        )
        .await?;
        players.insert(actor.name.clone(), player);
    }
    for player in players.values_mut() {
        player.until("ready").await?;
    }
    for (index, step) in branch.interventions.iter().chain(&branch.steps).enumerate() {
        players
            .get_mut(&step.actor)
            .ok_or("unknown actor")?
            .step(step, index)
            .await?;
    }
    for (actor, player) in &mut players {
        player
            .send(&format!(
                "/checkpoint {}",
                dir.join(actor).join("checkpoint.json").display()
            ))
            .await?;
    }
    let mut bots = BTreeMap::new();
    for (actor, player) in &mut players {
        let event = player.until("checkpoint-ready").await?;
        let snapshot = Snapshot::load(&dir.join(actor).join("checkpoint.json"))?;
        if event["data"]["sha256"] != snapshot.digest()? {
            return Err("bot output acknowledgement hash mismatch".into());
        }
        bots.insert(actor.clone(), snapshot);
    }
    let world: WorldSave = load_json(&dir.join("world-checkpoint.json"), 8_000_000)?;
    if world.epoch != epoch {
        return Err("fixture returned a stale checkpoint epoch".into());
    }
    let checkpoint = Checkpoint {
        version: 1,
        id: String::new(),
        mode: "restored_world_branch".into(),
        campaign: manifest.id.clone(),
        branch: branch.name.clone(),
        parent: parent_ref,
        ledger: ledger.into(),
        world,
        bots,
    }
    .seal()?;
    // The combined artifact is published only after every output acknowledgement validates.
    let checkpoint_path = dir.join("checkpoint.json");
    save_json(&checkpoint_path, &checkpoint)?;
    let mut reports = BTreeMap::new();
    for player in players.values_mut() {
        player.send("/quit").await?;
    }
    for (actor, player) in &mut players {
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            while player.events.recv().await.is_some() {}
            player.child.wait().await
        })
        .await??;
        if !status.success() {
            return Err("bot failed checkpoint shutdown".into());
        }
        reports.insert(
            actor.clone(),
            load_json(&dir.join(actor).join("report.json"), 32_000_000)?,
        );
    }
    fixture.kill().await?;
    fixture.wait().await?;
    Ok(BranchResult {
        name: branch.name.clone(),
        parent: branch.parent.clone(),
        checkpoint: Reference {
            path: checkpoint_path,
            id: checkpoint.id,
        },
        directory: dir,
        reserved_requests: reserved,
        reports,
    })
}

fn freeze_config(config: &mut crate::config::Config, directory: &Path) -> Result<()> {
    let sources = directory.join("sources");
    std::fs::create_dir(&sources)?;
    for (i, path) in config
        .character
        .iter_mut()
        .chain(config.affiliations.iter_mut())
        .chain(config.prompts.iter_mut())
        .chain(config.context.iter_mut())
        .chain(config.debug.iter_mut())
        .chain(config.budget.iter_mut())
        .chain(config.model_profiles.iter_mut())
        .chain(config.model_tape.iter_mut())
        .chain(config.conversation.iter_mut())
        .chain(config.knowledge.iter_mut())
        .enumerate()
    {
        let mut bytes = Vec::new();
        File::open(&*path)?
            .take(8_000_001)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8_000_000 {
            return Err("campaign input document exceeds 8 MB".into());
        }
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or("campaign input needs an extension")?;
        let copy = sources.join(format!("{i}-{}.{}", revision(&bytes), extension));
        let mut file = File::options().write(true).create_new(true).open(&copy)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        *path = copy;
    }
    Ok(())
}

pub fn inspect(path: &Path) -> Result<Value> {
    let summary: Summary = load_json(path, 128_000_000)?;
    if summary.version != 1
        || summary.mode != "restored_world_branch"
        || summary.branches.len() > 32
    {
        return Err("unsupported campaign summary".into());
    }
    let mut branches = Vec::new();
    for branch in &summary.branches {
        let checkpoint = Checkpoint::load(&branch.checkpoint.path)?;
        if checkpoint.id != branch.checkpoint.id
            || checkpoint.branch != branch.name
            || checkpoint.campaign != summary.campaign
            || checkpoint.ledger != summary.ledger
            || checkpoint.bots.keys().collect::<Vec<_>>()
                != branch.reports.keys().collect::<Vec<_>>()
        {
            return Err("branch checkpoint or report membership changed".into());
        }
        let actors = branch.reports.iter().map(|(name,r)| json!({"actor":name,"goal":r["state"]["goal"],"outcome":r["state"]["outcome"],
            "delta_costs":r["costs"]["session"],"probes":r["probe_results"].as_array().map(|v|v.iter().skip(r["inherited_probe_results"].as_u64().unwrap_or(0) as usize).collect::<Vec<_>>()),
            "inherited_probe_results":r["inherited_probe_results"],"trace":branch.directory.join(name).join("trace.jsonl"),
            "inherited_history":checkpoint.parent,"observation_watermark":checkpoint.bots[name].watermark,
            "recent_scene":r["state"]["recent"],"social_perspective":checkpoint.bots[name].social})).collect::<Vec<_>>();
        let views = checkpoint
            .bots
            .iter()
            .filter_map(|(name, s)| s.social.as_ref().map(|v| (name.clone(), v.clone())))
            .collect();
        branches.push(json!({"name":branch.name,"parent":branch.parent,"checkpoint":branch.checkpoint,
            "world_command_sequence":checkpoint.world.command_sequence,"reserved_requests_cumulative":branch.reserved_requests,"actors":actors,
            "social_comparison":crate::social::compare(&views)}));
    }
    Ok(
        json!({"version":1,"campaign":summary.campaign,"mode":summary.mode,"provider_spend_usd":summary.provider_spend_usd,
        "ledger":summary.ledger,"shared_prefix":"referenced once by checkpoint ID; delta reports never include ancestor charges",
        "comparison":"operator-only perspectives; omissions mean unestablished, not false. Never inject this combined view into a character. No automatic ranking or causal claim.","branches":branches}),
    )
}

pub fn select(summary_path: &Path, branch: &str, output: &Path) -> Result<()> {
    let summary: Summary = load_json(summary_path, 128_000_000)?;
    inspect(summary_path)?;
    let chosen = summary
        .branches
        .iter()
        .find(|b| b.name == branch)
        .ok_or("unknown branch selection")?;
    save_json(
        output,
        &json!({"version":1,"campaign":summary.campaign,"selected":chosen.checkpoint,
        "action":"explicit_runner_selection","continuation":"set from_checkpoint to selected.path in a new campaign record; keep the same id and max_requests"}),
    )
}
