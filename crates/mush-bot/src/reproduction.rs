//! Bounded reduction against fresh, certified, tape-only SSH fixture campaigns.
use crate::{
    Result,
    campaign::{self, Manifest},
    experiment::{load_json, revision, save_json},
    memory::{Event, Source},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub actor: String,
    /// Exact complete observed line. Sent commands and echo-only lines cannot match.
    pub line: String,
    pub after_command: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub manifest: PathBuf,
    pub protected_steps: Vec<usize>,
    pub protected_actors: Vec<String>,
    pub oracle: Vec<Observation>,
    pub repeats: usize,
    pub max_trials: usize,
    pub max_requests: usize,
    pub trial_timeout_ms: u64,
}
impl Spec {
    pub fn load(path: &Path) -> Result<(Self, Manifest)> {
        let mut spec: Self = load_json(path, 128_000)?;
        spec.manifest = std::fs::canonicalize(path)?
            .parent()
            .ok_or("recipe needs parent directory")?
            .join(&spec.manifest);
        let manifest = Manifest::load(&spec.manifest)?;
        spec.validate(&manifest)?;
        Ok((spec, manifest))
    }
    pub fn validate(&self, m: &Manifest) -> Result<()> {
        m.validate()?;
        if self.version != 1
            || !(1..=5).contains(&self.repeats)
            || self.max_trials < 2 * self.repeats
            || self.max_trials > 128
            || self.max_requests > 100_000
            || !(1000..=120_000).contains(&self.trial_timeout_ms)
            || self.oracle.is_empty()
            || self.oracle.len() > 32
            || m.from_checkpoint.is_some()
            || m.branches.len() != 1
            || m.branches[0].parent.is_some()
            || !m.branches[0].interventions.is_empty()
            || !m.groups.is_empty()
        {
            return Err("reproduction needs a fresh single-branch ungrouped tape campaign, 1..5 repeats, bounded trials/requests/time, and 1..32 observed oracle lines".into());
        }
        let steps: BTreeSet<_> = self.protected_steps.iter().copied().collect();
        let actors: BTreeSet<_> = self.protected_actors.iter().collect();
        if steps.len() != self.protected_steps.len()
            || actors.len() != self.protected_actors.len()
            || steps.iter().any(|i| *i >= m.branches[0].steps.len())
            || actors
                .iter()
                .any(|a| !m.actors.iter().any(|v| &v.name == *a))
        {
            return Err("invalid protected setup steps/actors".into());
        }
        // No uncontrolled autonomous loops or mutable input-file controls inside
        // the reducer. All model turns and tape contents are frozen and explicit.
        for s in &m.branches[0].steps {
            if ![
                "/send",
                "/goal",
                "/role",
                "/note",
                "/step",
                "/pause",
                "/state",
                "/feedback",
                "/mission",
            ]
            .contains(&s.control.split_whitespace().next().unwrap_or_default())
                || (s.control.starts_with("/mission") && s.control != "/mission")
            {
                return Err("reducer supports explicit turns and in-game commands only; no resume, file mutations, or mission activation".into());
            }
        }
        for o in &self.oracle {
            crate::evidence::bounded(&o.line, 4000)?;
            if !actors.contains(&o.actor) {
                return Err("every oracle observer must be a protected actor".into());
            }
            if let Some(c) = &o.after_command {
                crate::transport::validate_command(c)?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Reproduced,
    NotReproduced,
    Inconclusive,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub actor: String,
    pub episode: u64,
    pub line: usize,
    pub quote: String,
}
/// Order is evaluated independently per observer; different SSH streams have no
/// reliable total causal order. Each observer's required subsequence is exact.
pub fn observe(
    checkpoint: &campaign::Checkpoint,
    oracle: &[Observation],
) -> Result<Option<Vec<Receipt>>> {
    checkpoint.validate()?;
    let mut cursors = std::collections::BTreeMap::<String, (u64, usize)>::new();
    let mut receipts = vec![];
    for o in oracle {
        let bot = checkpoint
            .bots
            .get(&o.actor)
            .ok_or("oracle actor absent from checkpoint")?;
        let cursor = cursors.get(&o.actor).copied().unwrap_or((0, 0));
        let found = bot.memory.records.iter().find_map(|r| {
            if let Event::Episode(e) = &r.event && e.source == Source::Observed {
                if let Some(command) = &o.after_command && !bot.memory.records.iter().any(|sent| Some(sent.id) == e.after_command && matches!(&sent.event,Event::Episode(s) if s.source == Source::Sent && &s.text == command)) { return None; }
                for (index,line) in e.text.lines().enumerate() {
                    if (r.id,index+1) > cursor && line.trim_end() == o.line {
                        return Some(Receipt { actor:o.actor.clone(),episode:r.id,line:index+1,quote:line.trim_end().into() });
                    }
                }
            }
            None
        });
        let Some(found) = found else {
            return Ok(None);
        };
        cursors.insert(o.actor.clone(), (found.episode, found.line));
        receipts.push(found);
    }
    Ok(Some(receipts))
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Trial {
    pub index: usize,
    pub purpose: String,
    pub status: Status,
    pub reason: String,
    pub manifest_revision: String,
    pub oracle_revision: String,
    pub checkpoint: Option<campaign::Reference>,
    pub receipts: Vec<Receipt>,
    pub reserved_requests: usize,
}
impl Trial {
    pub fn verify(&self, oracle: &[Observation]) -> Result<()> {
        if self.status != Status::Reproduced
            || self.oracle_revision != revision(&serde_json::to_vec(oracle)?)
        {
            return Err("trial is not a reproduced observation under this oracle".into());
        }
        let reference = self
            .checkpoint
            .as_ref()
            .ok_or("reproduced trial lacks checkpoint")?;
        let checkpoint = campaign::Checkpoint::load(&reference.path)?;
        if checkpoint.id != reference.id
            || serde_json::to_value(observe(&checkpoint, oracle)?)?
                != serde_json::to_value(Some(&self.receipts))?
        {
            return Err("reproduction checkpoint or cited receipts changed".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize)]
pub struct Report {
    pub version: u32,
    pub mode: String,
    pub behavior_only: String,
    pub status: Status,
    pub trials: Vec<Trial>,
    pub reserved_requests: usize,
    pub provider_spend_usd: String,
    pub retained_original_steps: Vec<usize>,
    pub removed_actors: Vec<String>,
    pub minimal_under_supported_operations: bool,
    pub exported_recipe: Option<PathBuf>,
    pub limitations: Vec<String>,
}
struct Run<'a> {
    maximum_quota: usize,
    spec: &'a Spec,
    output: &'a Path,
    program: &'a Path,
    report: Report,
}
impl Run<'_> {
    fn room(&self, m: &Manifest, reserved_final: usize) -> bool {
        let quota = m.actors.iter().map(|a| a.request_allowance).sum::<usize>();
        self.report.trials.len() + self.spec.repeats + reserved_final <= self.spec.max_trials
            && self.report.reserved_requests
                + quota * self.spec.repeats
                + self.maximum_quota * reserved_final
                <= self.spec.max_requests
    }
    async fn trial(&mut self, m: &Manifest, purpose: &str) -> Result<Status> {
        let index = self.report.trials.len();
        let quota = m.actors.iter().map(|a| a.request_allowance).sum::<usize>();
        if index >= self.spec.max_trials
            || self.report.reserved_requests + quota > self.spec.max_requests
        {
            return Err("reproduction trial budget exhausted".into());
        }
        self.report.reserved_requests += quota;
        let path = self.output.join(format!("trial-{index:03}"));
        let result = tokio::time::timeout(
            Duration::from_millis(self.spec.trial_timeout_ms),
            campaign::run(m, &path, self.program),
        )
        .await;
        let mut trial = Trial {
            index,
            purpose: purpose.into(),
            status: Status::Inconclusive,
            reason: String::new(),
            manifest_revision: revision(&serde_json::to_vec(m)?),
            oracle_revision: revision(&serde_json::to_vec(&self.spec.oracle)?),
            checkpoint: None,
            receipts: vec![],
            reserved_requests: quota,
        };
        match result {
            Err(_) => trial.reason = "fresh fixture trial timed out; artifacts retained".into(),
            Ok(Err(error)) => {
                trial.reason = format!("campaign did not certify a complete run: {error}")
            }
            Ok(Ok(summary)) => {
                let branch = &summary.branches[0];
                let c = campaign::Checkpoint::load(&branch.checkpoint.path)?;
                trial.checkpoint = Some(branch.checkpoint.clone());
                if c.bots
                    .values()
                    .any(|b| matches!(b.resume_status.as_str(), "error" | "blocked"))
                    || branch
                        .reports
                        .values()
                        .any(|r| matches!(r["state"]["status"].as_str(), Some("error" | "blocked")))
                {
                    trial.reason = "actor execution blocked or failed".into();
                } else if let Some(receipts) = observe(&c, &self.spec.oracle)? {
                    trial.status = Status::Reproduced;
                    trial.receipts = receipts;
                    trial.reason =
                        "ordered observed subsequences reproduced in a certified fresh fixture"
                            .into();
                    trial.verify(&self.spec.oracle)?;
                } else {
                    trial.status = Status::NotReproduced;
                    trial.reason = "complete run did not match the observed oracle".into();
                }
            }
        }
        let status = trial.status.clone();
        save_json(&self.output.join(format!("trial-{index:03}.json")), &trial)?;
        self.report.trials.push(trial);
        Ok(status)
    }
    async fn repeated(&mut self, m: &Manifest, purpose: &str) -> Result<bool> {
        let mut reproduced = true;
        for _ in 0..self.spec.repeats {
            reproduced &= self.trial(m, purpose).await? == Status::Reproduced;
        }
        Ok(reproduced)
    }
}
fn freeze(m: &mut Manifest, dir: &Path, zero_delays: bool) -> Result<()> {
    std::fs::create_dir(dir)?;
    for a in &mut m.actors {
        let actor_dir = dir.join(&a.name);
        std::fs::create_dir(&actor_dir)?;
        let mut c = crate::config::Config::load(&a.config)?;
        campaign::freeze_config(&mut c, &actor_dir)?;
        if zero_delays {
            let path = c.model_tape.as_ref().ok_or("tape required")?;
            let mut tape = crate::tape::Source::load(path)?;
            for reply in &mut tape.replies {
                reply.delay_ms = 0;
            }
            let path = actor_dir.join("zero-delay.json");
            save_json(&path, &tape)?;
            c.model_tape = Some(path);
        }
        let config_path = actor_dir.join("bot.toml");
        let bytes = toml::to_string(&c)?;
        std::fs::write(&config_path, bytes)?;
        a.config = config_path;
    }
    Ok(())
}
pub async fn run(
    spec: &Spec,
    manifest: &Manifest,
    output: &Path,
    program: &Path,
    minimize: bool,
) -> Result<Report> {
    spec.validate(manifest)?;
    std::fs::create_dir(output)?;
    let output = std::fs::canonicalize(output)?;
    save_json(&output.join("source-recipe.json"), spec)?;
    let mut best = manifest.clone();
    freeze(&mut best, &output.join("frozen"), false)?;
    save_json(&output.join("frozen-manifest.json"), &best)?;
    let mut retained: Vec<_> = (0..best.branches[0].steps.len()).collect();
    let mut runner = Run { maximum_quota:manifest.actors.iter().map(|a|a.request_allowance).sum(), spec, output:&output, program, report:Report { version:1,mode:if minimize{"deterministic_reduction"}else{"reproduction"}.into(),behavior_only:"Local tapes replay behavior; this does not establish a model's causal susceptibility to an adversary.".into(),status:Status::Inconclusive,trials:vec![],reserved_requests:0,provider_spend_usd:"0".into(),retained_original_steps:retained.clone(),removed_actors:vec![],minimal_under_supported_operations:false,exported_recipe:None,limitations:vec!["Exact lines ordered per observer; different SSH streams have no total order.".into(),"Fresh owned adapter only, no groups or mutable controls; live models require the existing repeat harness.".into()] } };
    if !runner.room(&best, spec.repeats) {
        return Err("budget must reserve baseline and final repeats before starting".into());
    }
    if !runner.repeated(&best, "baseline").await? {
        runner.report.status = if runner
            .report
            .trials
            .iter()
            .any(|t| t.status == Status::Inconclusive)
        {
            Status::Inconclusive
        } else {
            Status::NotReproduced
        };
        save_json(&output.join("report.json"), &runner.report)?;
        return Ok(runner.report);
    }
    let mut complete = true;
    if minimize {
        // Restart after each accepted deletion to establish a fixed point under
        // single-step deletion. Setup indexes refer to the immutable input.
        loop {
            let mut changed = false;
            for index in 0..retained.len() {
                if spec.protected_steps.contains(&retained[index]) {
                    continue;
                }
                let mut candidate = best.clone();
                candidate.branches[0].steps.remove(index);
                if !runner.room(&candidate, spec.repeats) {
                    complete = false;
                    break;
                }
                if runner.repeated(&candidate, "remove_optional_step").await? {
                    best = candidate;
                    retained.remove(index);
                    changed = true;
                    break;
                }
            }
            if !changed || !complete {
                break;
            }
        }
        for actor in manifest
            .actors
            .iter()
            .map(|a| &a.name)
            .filter(|a| !spec.protected_actors.contains(a))
        {
            // Actor removal cannot remove a protected action belonging to it.
            if best.branches[0]
                .steps
                .iter()
                .zip(&retained)
                .any(|(s, i)| &s.actor == actor && spec.protected_steps.contains(i))
            {
                continue;
            }
            let mut candidate = best.clone();
            candidate.actors.retain(|a| &a.name != actor);
            candidate.branches[0].steps.retain(|s| &s.actor != actor);
            if !runner.room(&candidate, spec.repeats) {
                complete = false;
                break;
            }
            if runner.repeated(&candidate, "remove_optional_actor").await? {
                retained = best.branches[0]
                    .steps
                    .iter()
                    .zip(retained)
                    .filter_map(|(s, i)| (&s.actor != actor).then_some(i))
                    .collect();
                best = candidate;
                runner.report.removed_actors.push(actor.clone());
            }
        }
        let mut candidate = best.clone();
        freeze(&mut candidate, &output.join("zero-delay"), true)?;
        if runner.room(&candidate, spec.repeats) {
            if runner.repeated(&candidate, "zero_response_delays").await? {
                best = candidate;
            }
        } else {
            complete = false;
        }
        // Actor/delay removal can enable further reductions. Avoid claiming a
        // global minimum after only a greedy pass; retain the precise boundary.
        runner.report.limitations.push("Greedy single-step/actor deletion and all-response-delay removal; no global minimality claim.".into());
    }
    // Export self-contained bot inputs and relative paths. The fixture executable
    // remains explicit (operator-owned toolchain), as in ordinary campaign JSON.
    let export = output.join("regression");
    std::fs::create_dir(&export)?;
    freeze(&mut best, &export.join("inputs"), false)?;
    for a in &mut best.actors {
        let mut c = crate::config::Config::load(&a.config)?;
        let base = a.config.parent().ok_or("config directory absent")?;
        for p in c
            .character
            .iter_mut()
            .chain(c.affiliations.iter_mut())
            .chain(c.prompts.iter_mut())
            .chain(c.context.iter_mut())
            .chain(c.debug.iter_mut())
            .chain(c.budget.iter_mut())
            .chain(c.model_profiles.iter_mut())
            .chain(c.model_tape.iter_mut())
            .chain(c.conversation.iter_mut())
            .chain(c.activities.iter_mut())
            .chain(c.mission.iter_mut())
            .chain(c.knowledge.iter_mut())
        {
            *p = p.strip_prefix(base)?.to_owned();
        }
        std::fs::write(&a.config, toml::to_string(&c)?)?;
        a.config = a.config.strip_prefix(&export)?.to_owned();
    }
    save_json(&export.join("campaign.json"), &best)?;
    let mut recipe = spec.clone();
    recipe.manifest = "campaign.json".into();
    recipe.protected_steps = (0..best.branches[0].steps.len())
        .filter(|i| spec.protected_steps.contains(&retained[*i]))
        .collect();
    save_json(&export.join("reproduce.json"), &recipe)?;
    let (_, final_manifest) = Spec::load(&export.join("reproduce.json"))?;
    runner.report.exported_recipe = Some(export.join("reproduce.json"));
    runner.report.retained_original_steps = retained;
    runner.report.status = if runner
        .repeated(&final_manifest, "final_export_verification")
        .await?
    {
        Status::Reproduced
    } else {
        Status::Inconclusive
    };
    runner.report.minimal_under_supported_operations = false;
    if !complete {
        runner.report.limitations.push(
            "Reduction budget exhausted; best reproduced candidate exported and rerun.".into(),
        );
    }
    save_json(&output.join("report.json"), &runner.report)?;
    Ok(runner.report)
}
