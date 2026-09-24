//! Offline character and cast drafts. Model-authored data never supplies local
//! configuration paths, logins, process arguments, or executable game commands.
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Result,
    character::Character,
    config::Config,
    experiment,
    organization::{self, Affiliations, Group},
};

const MAX_SESSION_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

fn version() -> u32 {
    1
}
fn count() -> usize {
    10
}
fn max_tokens() -> u32 {
    16_384
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Character,
    Group,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Concept {
    #[serde(default = "version")]
    pub version: u32,
    pub kind: Kind,
    pub concept: String,
    #[serde(default = "count")]
    pub count: usize,
    #[serde(default = "max_tokens")]
    pub max_tokens: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredCharacter {
    pub actor: String,
    pub character: Character,
    pub role: String,
    pub goal: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub title: String,
    pub synopsis: String,
    pub characters: Vec<AuthoredCharacter>,
    pub group: Option<Group>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub number: usize,
    pub instruction: String,
    pub based_on: Option<usize>,
    pub request_revision: String,
    pub draft: Draft,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub version: u32,
    pub concept: Concept,
    pub candidates: Vec<Candidate>,
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub actor: String,
    pub config: PathBuf,
    pub login: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    #[serde(default = "version")]
    pub version: u32,
    pub actors: Vec<Binding>,
}

fn prose(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= limit
        && value
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
}

fn actor(value: &str) -> bool {
    organization::identifier(value) && !value.eq_ignore_ascii_case("_harness")
}

impl Concept {
    pub fn load(path: &Path) -> Result<Self> {
        let concept: Self = experiment::load_json(path, 32_000)?;
        concept.validate()?;
        Ok(concept)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !prose(&self.concept, 16_000)
            || !(1..=10).contains(&self.count)
            || !(512..=65_536).contains(&self.max_tokens)
        {
            return Err("studio concept requires version 1, nonempty text <= 16000 bytes, count 1..10, and max_tokens 512..65536".into());
        }
        Ok(())
    }
}

impl Draft {
    pub fn validate(&self, kind: Kind) -> Result<()> {
        let words = self.synopsis.split_whitespace().count();
        if !prose(&self.title, 200)
            || self.title.chars().any(char::is_control)
            || !prose(&self.synopsis, 16_000)
            || !(100..=500).contains(&words)
        {
            return Err("studio draft requires a title <= 200 bytes and a synopsis of 100..500 words (16000 bytes maximum)".into());
        }
        match kind {
            Kind::Character if self.characters.len() != 1 || self.group.is_some() => {
                return Err(
                    "character draft must contain exactly one character and no group".into(),
                );
            }
            Kind::Group if !(2..=16).contains(&self.characters.len()) || self.group.is_none() => {
                return Err("group draft must contain 2..16 characters and a group".into());
            }
            _ => {}
        }
        let mut names = BTreeSet::new();
        for authored in &self.characters {
            if !actor(&authored.actor)
                || !names.insert(authored.actor.to_ascii_lowercase())
                || !prose(&authored.role, 4000)
                || !prose(&authored.goal, 4000)
            {
                return Err("draft actors need unique portable identifiers and nonempty roles/goals <= 4000 bytes".into());
            }
            // Reuse the character tool's authoritative field and size checks.
            Character::parse(&serde_json::to_string(&authored.character)?)?;
        }
        if let Some(group) = &self.group {
            group.validate()?;
            if group
                .members
                .iter()
                .map(|member| member.actor.as_str())
                .collect::<BTreeSet<_>>()
                != self
                    .characters
                    .iter()
                    .map(|character| character.actor.as_str())
                    .collect::<BTreeSet<_>>()
            {
                return Err("group roster must exactly match the draft's actors".into());
            }
            for authored in &self.characters {
                Affiliations::for_actor(&authored.actor, std::slice::from_ref(group))?;
            }
        }
        Ok(())
    }
}

impl Session {
    pub fn new(concept: Concept) -> Result<Self> {
        concept.validate()?;
        Ok(Self {
            version: 1,
            concept,
            candidates: Vec::new(),
            selected: None,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let session: Self = experiment::load_json(path, MAX_SESSION_BYTES)?;
        session.validate()?;
        Ok(session)
    }

    pub fn validate(&self) -> Result<()> {
        self.concept.validate()?;
        if self.version != 1
            || self.candidates.len() > 100
            || self
                .selected
                .is_some_and(|selected| selected == 0 || selected > self.candidates.len())
            || serde_json::to_vec(self)?.len() > MAX_SESSION_BYTES
        {
            return Err(
                "invalid studio session version, selection, or limits (100 candidates, 16 MiB)"
                    .into(),
            );
        }
        for (index, candidate) in self.candidates.iter().enumerate() {
            if candidate.number != index + 1
                || candidate
                    .based_on
                    .is_some_and(|number| number == 0 || number >= candidate.number)
                || !prose(&candidate.instruction, 16_000)
                || candidate.request_revision.len() != 64
                || !candidate
                    .request_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("studio candidates need contiguous numbers, an earlier basis, a bounded instruction, and a SHA-256 request revision".into());
            }
            candidate.draft.validate(self.concept.kind)?;
        }
        Ok(())
    }

    pub fn select(&mut self, number: usize) -> Result<()> {
        self.validate()?;
        if number == 0 || number > self.candidates.len() {
            return Err("studio selection is not an existing candidate number".into());
        }
        self.selected = Some(number);
        Ok(())
    }

    pub fn selected(&self) -> Result<&Candidate> {
        let number = self
            .selected
            .ok_or("select a studio candidate before exporting or refining")?;
        self.candidates
            .get(number.checked_sub(1).ok_or("invalid studio selection")?)
            .filter(|candidate| candidate.number == number)
            .ok_or_else(|| "studio selection is not an existing candidate number".into())
    }

    /// Validate the complete batch before assigning host-owned stable numbers.
    /// Selection is retained; callers may explicitly select a refined candidate.
    pub fn add_response(
        &mut self,
        text: &str,
        instruction: &str,
        request_revision: &str,
        expected_count: usize,
    ) -> Result<Vec<usize>> {
        self.validate()?;
        if text.len() > MAX_RESPONSE_BYTES || !(1..=10).contains(&expected_count) {
            return Err(
                "studio response exceeds 1 MiB or expected candidate count is outside 1..10".into(),
            );
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Response {
            candidates: Vec<Draft>,
        }
        let trimmed = text.trim();
        let source = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|source| source.strip_suffix("```"))
            .unwrap_or(trimmed)
            .trim();
        let response: Response = serde_json::from_str(source)?;
        if response.candidates.len() != expected_count
            || self.candidates.len() + expected_count > 100
        {
            return Err("studio response must have exactly the requested count and retain at most 100 candidates".into());
        }
        let mut updated = self.clone();
        let first = updated.candidates.len() + 1;
        for draft in response.candidates {
            updated.candidates.push(Candidate {
                number: updated.candidates.len() + 1,
                instruction: instruction.into(),
                based_on: self.selected,
                request_revision: request_revision.into(),
                draft,
            });
        }
        updated.validate()?;
        let numbers = (first..=updated.candidates.len()).collect();
        *self = updated;
        Ok(numbers)
    }

    /// Export only into a newly created directory. Bindings are runner-authored;
    /// without them a group remains an offline draft, never a runnable cast.
    pub fn export(&self, directory: &Path, bindings: Option<&Bindings>) -> Result<PathBuf> {
        self.validate()?;
        let selected = self.selected()?;
        let draft = &selected.draft;
        let mut artifacts: Vec<(PathBuf, Value)> = vec![(
            PathBuf::from("selected.json"),
            json!({
                "version":1,"concept":self.concept,"candidate":selected
            }),
        )];
        let groups = draft.group.iter().cloned().collect::<Vec<_>>();
        for authored in &draft.characters {
            artifacts.push((
                character_path(&authored.actor),
                serde_json::to_value(&authored.character)?,
            ));
            artifacts.push((
                affiliations_path(&authored.actor),
                serde_json::to_value(Affiliations::for_actor(&authored.actor, &groups)?)?,
            ));
        }
        let result = if let Some(bindings) = bindings {
            bindings.validate()?;
            if bindings
                .actors
                .iter()
                .map(|binding| binding.actor.as_str())
                .collect::<BTreeSet<_>>()
                != draft
                    .characters
                    .iter()
                    .map(|authored| authored.actor.as_str())
                    .collect::<BTreeSet<_>>()
            {
                return Err(
                    "studio bindings must exactly match the selected draft's actors".into(),
                );
            }
            let mut actors = Vec::new();
            for authored in &draft.characters {
                let binding = bindings
                    .actors
                    .iter()
                    .find(|binding| binding.actor == authored.actor)
                    .expect("exact actor set checked");
                let config = Config::load(&binding.config)?;
                if !groups.is_empty() && config.affiliations.is_some() {
                    return Err("group export bindings must not also define affiliations in their bot config".into());
                }
                actors.push(json!({"name":authored.actor,"config":fs::canonicalize(&binding.config)?,"login":binding.login,
                    "goal":authored.goal,"role":authored.role,"character":character_path(&authored.actor),
                    "start":"/resume","expected_status":"done"}));
            }
            let manifest: crate::harness::Manifest = serde_json::from_value(json!({
                "version":1,"name":draft.title,"actors":actors,"groups":groups,"timeout_ms":900_000
            }))?;
            artifacts.push((
                PathBuf::from("harness.json"),
                serde_json::to_value(manifest)?,
            ));
            PathBuf::from("harness.json")
        } else if self.concept.kind == Kind::Character {
            character_path(&draft.characters[0].actor)
        } else {
            PathBuf::from("selected.json")
        };
        // Complete schema/path/config checks and serialization before creating
        // anything. create_dir and create_new preserve all existing artifacts.
        let artifacts = artifacts
            .into_iter()
            .map(|(path, value)| Ok((path, serde_json::to_vec_pretty(&value)?)))
            .collect::<Result<Vec<_>>>()?;
        fs::create_dir(directory)?;
        fs::create_dir(directory.join("characters"))?;
        fs::create_dir(directory.join("affiliations"))?;
        for (relative, bytes) in artifacts {
            let mut file = fs::File::options()
                .write(true)
                .create_new(true)
                .open(directory.join(relative))?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
        }
        Ok(directory.join(result))
    }
}

fn character_path(actor: &str) -> PathBuf {
    PathBuf::from("characters").join(format!("character-{actor}.json"))
}

fn affiliations_path(actor: &str) -> PathBuf {
    PathBuf::from("affiliations").join(format!("affiliations-{actor}.json"))
}

impl Bindings {
    pub fn load(path: &Path) -> Result<Self> {
        let mut bindings: Self = experiment::load_json(path, 64_000)?;
        let path = fs::canonicalize(path)?;
        let base = path
            .parent()
            .ok_or("bindings file has no parent directory")?;
        for binding in &mut bindings.actors {
            if binding.config.is_relative() {
                binding.config = base.join(&binding.config);
            }
        }
        bindings.validate()?;
        Ok(bindings)
    }

    pub fn validate(&self) -> Result<()> {
        let mut actors = BTreeSet::new();
        if self.version != 1
            || self.actors.is_empty()
            || self.actors.len() > 16
            || serde_json::to_vec(self)?.len() > 64_000
        {
            return Err(
                "studio bindings require version 1 and 1..16 actors (64 KB maximum)".into(),
            );
        }
        for binding in &self.actors {
            if !actor(&binding.actor)
                || !actors.insert(binding.actor.to_ascii_lowercase())
                || binding.config.as_os_str().is_empty()
                || binding.login.as_ref().is_some_and(|login| {
                    login.is_empty() || login.len() > 255 || login.chars().any(char::is_control)
                })
            {
                return Err("invalid studio binding actor, config path, or login".into());
            }
            let mut config = Config::load(&binding.config)?;
            if let Some(login) = &binding.login {
                config.ssh.username = login.clone();
            }
            config.validate()?;
            // Preflight referenced runner settings without credentials, an SSH
            // connection, a provider request, or any model-selected paths.
            let _ = crate::models::Catalog::from_config(&config)?.initial()?;
            if let Some(path) = &config.affiliations {
                Affiliations::load(path)?;
            }
            if let Some(path) = &config.prompts {
                crate::experiment::Prompts::load(path)?;
            }
            if let Some(path) = &config.context {
                crate::experiment::Context::load(path)?;
            }
            if let Some(path) = &config.debug {
                crate::debugger::Settings::load(path)?;
            }
            if let Some(path) = &config.budget {
                crate::budget::Settings::load(path)?;
            }
            if let Some(path) = &config.model_tape {
                crate::tape::Source::load(path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept(kind: Kind) -> Concept {
        Concept {
            version: 1,
            kind,
            concept: "An uneasy alliance of travelers.".into(),
            count: 10,
            max_tokens: 16_384,
        }
    }
    fn authored(actor: &str) -> AuthoredCharacter {
        AuthoredCharacter {
            actor: actor.into(),
            character: Character::for_login(actor),
            role: "A cautious traveler.".into(),
            goal: "Meet the guide in the square.".into(),
        }
    }
    fn draft(kind: Kind) -> Draft {
        Draft {
            title: "The Lantern Road".into(), synopsis: "A traveler remembers the road and listens carefully before choosing a destination. ".repeat(12),
            characters: if kind == Kind::Character { vec![authored("mira")] } else { vec![authored("mira"), authored("marco")] },
            group: (kind == Kind::Group).then(|| serde_json::from_value(json!({
                "id":"lanterns","name":"Lanterns","kind":"party","mission":"Find the missing guide",
                "shared_knowledge":["We agreed to meet in the square."],"members":[
                    {"actor":"mira","role":"scout","briefing":["MIRA_PRIVATE"]},
                    {"actor":"marco","role":"medic","briefing":["MARCO_PRIVATE"]}
                ]
            })).unwrap()),
        }
    }
    fn add(session: &mut Session, drafts: Vec<Draft>) -> Result<Vec<usize>> {
        let count = drafts.len();
        session.add_response(
            &json!({"candidates":drafts}).to_string(),
            "Develop these candidates.",
            &"a".repeat(64),
            count,
        )
    }
    fn directory() -> PathBuf {
        // Tests in one binary run in parallel under one pid, and the Windows
        // clock ticks in 100 ns steps, so the time alone can repeat: the counter
        // keeps two fixtures from racing to create_dir the same path.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-studio-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn candidate_numbers_are_transactional_and_refinements_retain_their_basis() {
        let mut session = Session::new(concept(Kind::Character)).unwrap();
        assert_eq!(
            add(&mut session, vec![draft(Kind::Character); 10]).unwrap(),
            (1..=10).collect::<Vec<_>>()
        );
        assert!(session.selected().is_err());
        session.select(8).unwrap();
        let before = serde_json::to_value(&session).unwrap();
        let mut bad = draft(Kind::Character);
        bad.synopsis = "Too short".into();
        assert!(add(&mut session, vec![draft(Kind::Character), bad]).is_err());
        assert_eq!(serde_json::to_value(&session).unwrap(), before);
        let response = format!(
            "```json\n{}\n```",
            json!({"candidates":[draft(Kind::Character)]})
        );
        assert_eq!(
            session
                .add_response(
                    &response,
                    "Blend candidate 8 with candidate 3.",
                    &"b".repeat(64),
                    1
                )
                .unwrap(),
            vec![11]
        );
        assert_eq!(session.selected, Some(8));
        assert_eq!(session.candidates[10].based_on, Some(8));
        session.select(11).unwrap();
        assert_eq!(session.selected().unwrap().number, 11);
        assert!(session.select(12).is_err());
        assert_eq!(session.selected, Some(11));
        let mut invalid_history = session.clone();
        invalid_history.candidates[0].based_on = Some(11);
        assert!(invalid_history.validate().is_err());
    }

    #[test]
    fn group_rosters_and_model_response_shapes_are_strict() {
        let mut session = Session::new(concept(Kind::Group)).unwrap();
        let mut bad = draft(Kind::Group);
        bad.group.as_mut().unwrap().members[1].actor = "stranger".into();
        assert!(add(&mut session, vec![bad]).is_err());
        let mut bad = draft(Kind::Group);
        bad.characters[1].actor = "MIRA".into();
        assert!(add(&mut session, vec![bad]).is_err());
        let mut raw = serde_json::to_value(draft(Kind::Group)).unwrap();
        raw["characters"][0]["config"] = json!("model-selected.toml");
        assert!(
            session
                .add_response(
                    &json!({"candidates":[raw]}).to_string(),
                    "Generate",
                    &"a".repeat(64),
                    1
                )
                .is_err()
        );
        assert!(
            session
                .add_response(
                    &json!({"candidates":[draft(Kind::Group)],"command":"@dig test"}).to_string(),
                    "Generate",
                    &"a".repeat(64),
                    1
                )
                .is_err()
        );
        assert!(session.candidates.is_empty());
        add(&mut session, vec![draft(Kind::Group)]).unwrap();
        session.validate().unwrap();
    }

    #[test]
    fn export_preserves_private_affiliations_and_requires_exact_runner_bindings() {
        let directory = directory();
        fs::write(directory.join("bot.toml"), "[ssh]\nusername = \"runner\"\n").unwrap();
        let binding_file = directory.join("bindings.json");
        fs::write(
            &binding_file,
            json!({"version":1,"actors":[
                {"actor":"mira","config":"bot.toml","login":"play+Mira"},
                {"actor":"marco","config":"bot.toml","login":"play+Marco"}
            ]})
            .to_string(),
        )
        .unwrap();
        let bindings = Bindings::load(&binding_file).unwrap();
        assert!(
            bindings
                .actors
                .iter()
                .all(|binding| binding.config.is_absolute())
        );
        let mut session = Session::new(concept(Kind::Group)).unwrap();
        add(&mut session, vec![draft(Kind::Group)]).unwrap();
        session.select(1).unwrap();
        let mut wrong = bindings.clone();
        wrong.actors.pop();
        let bad_output = directory.join("bad");
        assert!(session.export(&bad_output, Some(&wrong)).is_err());
        assert!(!bad_output.exists());
        let output = directory.join("cast");
        assert_eq!(
            session.export(&output, Some(&bindings)).unwrap(),
            output.join("harness.json")
        );
        let mira = fs::read_to_string(output.join(affiliations_path("mira"))).unwrap();
        assert!(mira.contains("MIRA_PRIVATE"));
        assert!(!mira.contains("MARCO_PRIVATE"));
        let marco = fs::read_to_string(output.join(affiliations_path("marco"))).unwrap();
        assert!(marco.contains("MARCO_PRIVATE"));
        assert!(!marco.contains("MIRA_PRIVATE"));
        let manifest = crate::harness::Manifest::load(&output.join("harness.json")).unwrap();
        assert_eq!(manifest.actors[0].login.as_deref(), Some("play+Mira"));
        assert_eq!(manifest.actors[0].start.as_deref(), Some("/resume"));
        assert_eq!(manifest.groups[0].members.len(), 2);
        let selected = fs::read(output.join("selected.json")).unwrap();
        assert!(session.export(&output, Some(&bindings)).is_err());
        assert_eq!(fs::read(output.join("selected.json")).unwrap(), selected);
        let offline = directory.join("offline");
        assert_eq!(
            session.export(&offline, None).unwrap(),
            offline.join("selected.json")
        );
        assert!(!offline.join("harness.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn single_character_export_is_live_schema_json_and_bad_bindings_create_nothing() {
        let directory = directory();
        let mut session = Session::new(concept(Kind::Character)).unwrap();
        add(&mut session, vec![draft(Kind::Character)]).unwrap();
        session.select(1).unwrap();
        let output = directory.join("character");
        let path = session.export(&output, None).unwrap();
        assert_eq!(Character::load(&path).unwrap().name, "mira");
        assert!(output.join("selected.json").exists());
        let bad = Bindings {
            version: 1,
            actors: vec![Binding {
                actor: "mira".into(),
                config: directory.join("missing.toml"),
                login: None,
            }],
        };
        let missing = directory.join("missing-export");
        assert!(session.export(&missing, Some(&bad)).is_err());
        assert!(!missing.exists());
        let malformed: Concept =
            serde_json::from_value(json!({"kind":"character","concept":"A traveler","count":11}))
                .unwrap();
        assert!(malformed.validate().is_err());
        let defaulted: Concept =
            serde_json::from_value(json!({"kind":"character","concept":"A traveler"})).unwrap();
        assert_eq!(defaulted.count, 10);
        assert_eq!(defaulted.max_tokens, 16_384);
        fs::remove_dir_all(directory).unwrap();
    }
}
