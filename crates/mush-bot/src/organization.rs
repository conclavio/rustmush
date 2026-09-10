//! Authored group membership, never a shared view of other players' private state.
use crate::{Result, experiment::load_json};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const POLICY: &str = "group_affiliations supplies runner-authored membership, roles, mission and briefing.\n\
Use these to cooperate with your group while pursuing the runner's current goal. Membership gives no\n\
extra game powers, telepathy or access to another member's observations or private memories.\n\
Roster actor IDs are harness identifiers, not automatically in-character names. Shared knowledge is\n\
authored prior knowledge; a member's reported discovery remains reported until verified. Coordinate\n\
through ordinary game speech, pages or other observed game facilities. Do not invent progress by\n\
absent teammates or claim a team task succeeded merely because the mission describes its outcome.";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// Free-form setting vocabulary: pack, coterie, chantry, mercenary unit, party.
    pub kind: String,
    pub mission: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub shared_knowledge: Vec<String>,
    #[serde(default)]
    pub background: Vec<crate::social::Grant>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub actor: String,
    pub role: String,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    /// Only this member receives these runner-authored instructions.
    #[serde(default)]
    pub briefing: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Affiliations {
    #[serde(default = "version")]
    pub version: u32,
    pub actor: String,
    #[serde(default)]
    pub groups: Vec<Membership>,
}
fn version() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mission: String,
    pub values: Vec<String>,
    pub shared_knowledge: Vec<String>,
    /// Only facts granted to this actor; never the grant list or another view.
    #[serde(default)]
    pub background: Vec<crate::social::Fact>,
    pub role: String,
    pub responsibilities: Vec<String>,
    pub briefing: Vec<String>,
    /// Roster deliberately omits other members' briefings and responsibilities.
    pub roster: Vec<RosterMember>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RosterMember {
    pub actor: String,
    pub role: String,
}

pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

impl Group {
    pub fn validate(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        if !identifier(&self.id)
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.kind.trim().is_empty()
            || self.kind.len() > 100
            || self.mission.len() > 4000
            || self.members.is_empty()
            || self.members.len() > 16
            || self.values.len() > 32
            || self.shared_knowledge.len() > 64
            || self
                .values
                .iter()
                .chain(&self.shared_knowledge)
                .any(|s| s.trim().is_empty() || s.len() > 4000)
            || self.members.iter().any(|m| {
                !identifier(&m.actor)
                    || !names.insert(&m.actor)
                    || m.role.trim().is_empty()
                    || m.role.len() > 200
                    || m.responsibilities.len() > 32
                    || m.briefing.len() > 32
                    || m.responsibilities
                        .iter()
                        .chain(&m.briefing)
                        .any(|s| s.trim().is_empty() || s.len() > 4000)
            })
            || serde_json::to_vec(self)?.len() > 128_000
        {
            return Err("invalid group identity, roster or briefing limits".into());
        }
        if self.background.len() > 256 {
            return Err("group background limited to 256 grants".into());
        }
        let mut facts = BTreeSet::new();
        for grant in &self.background {
            grant.fact.validate()?;
            if !facts.insert((&grant.fact.id, grant.fact.revision))
                || grant.known_by.is_empty()
                || grant.known_by.len() > 16
                || grant.known_by.iter().collect::<BTreeSet<_>>().len() != grant.known_by.len()
                || grant.known_by.iter().any(|a| !names.contains(a))
            {
                return Err(
                    "background grants require unique revisions and explicit roster members".into(),
                );
            }
        }
        Ok(())
    }

    fn membership(&self, actor: &str) -> Option<Membership> {
        let member = self.members.iter().find(|m| m.actor == actor)?;
        Some(Membership {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind.clone(),
            mission: self.mission.clone(),
            values: self.values.clone(),
            shared_knowledge: self.shared_knowledge.clone(),
            background: self
                .background
                .iter()
                .filter(|g| g.known_by.iter().any(|a| a == actor))
                .map(|g| g.fact.clone())
                .collect(),
            role: member.role.clone(),
            responsibilities: member.responsibilities.clone(),
            briefing: member.briefing.clone(),
            roster: self
                .members
                .iter()
                .map(|m| RosterMember {
                    actor: m.actor.clone(),
                    role: m.role.clone(),
                })
                .collect(),
        })
    }
}

impl Affiliations {
    pub fn for_actor(actor: &str, groups: &[Group]) -> Result<Self> {
        for group in groups {
            group.validate()?;
        }
        let value = Self {
            version: 1,
            actor: actor.into(),
            groups: groups.iter().filter_map(|g| g.membership(actor)).collect(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let value: Self = load_json(path, 256_000)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !identifier(&self.actor)
            || self.groups.len() > 8
            || serde_json::to_vec(self)?.len() > 256_000
        {
            return Err("invalid affiliations version/actor/size".into());
        }
        let mut ids = BTreeSet::new();
        for g in &self.groups {
            if !ids.insert(&g.id)
                || g.roster
                    .iter()
                    .filter(|m| m.actor == self.actor && m.role == g.role)
                    .count()
                    != 1
            {
                return Err("affiliations need unique groups and matching self membership".into());
            }
            Group {
                id: g.id.clone(),
                name: g.name.clone(),
                kind: g.kind.clone(),
                mission: g.mission.clone(),
                values: g.values.clone(),
                shared_knowledge: g.shared_knowledge.clone(),
                background: g
                    .background
                    .iter()
                    .map(|fact| crate::social::Grant {
                        fact: fact.clone(),
                        known_by: vec![self.actor.clone()],
                    })
                    .collect(),
                members: g
                    .roster
                    .iter()
                    .map(|m| Member {
                        actor: m.actor.clone(),
                        role: m.role.clone(),
                        responsibilities: if m.actor == self.actor {
                            g.responsibilities.clone()
                        } else {
                            vec![]
                        },
                        briefing: if m.actor == self.actor {
                            g.briefing.clone()
                        } else {
                            vec![]
                        },
                    })
                    .collect(),
            }
            .validate()?;
        }
        Ok(())
    }

    pub fn prior_facts(&self) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|g| {
                g.shared_knowledge.iter().map(move |fact| {
                    format!("Authored prior knowledge for group {}: {fact}", g.name)
                })
            })
            .collect()
    }

    /// Large granted background is retrieved from the durable actor journal.
    pub fn prompt_view(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("serializable affiliations");
        for group in value["groups"].as_array_mut().unwrap() {
            group.as_object_mut().unwrap().remove("background");
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checked_in_group_and_prompt_examples_use_the_live_json_schemas() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        crate::experiment::Prompts::load(&root.join("bots/prompts/composed-observer.json"))
            .unwrap();
        crate::experiment::Context::load(&root.join("bots/context/selected-voice.json")).unwrap();
        let manifest: crate::harness::Manifest = serde_json::from_slice(
            &std::fs::read(root.join("bots/harness/organized-party.json")).unwrap(),
        )
        .unwrap();
        for group in &manifest.groups {
            group.validate().unwrap();
        }
        for actor in &manifest.actors {
            Affiliations::for_actor(&actor.name, &manifest.groups).unwrap();
        }
    }
    #[test]
    fn membership_never_includes_another_members_private_briefing() {
        let group: Group = serde_json::from_value(json!({"id":"party","name":"Lanterns","kind":"adventuring party","mission":"Find the missing guide",
            "shared_knowledge":["Our rendezvous is the square."],"members":[
                {"actor":"scout","role":"pathfinder","briefing":["Suspect the patron."]},
                {"actor":"healer","role":"medic","briefing":["Private debt to the patron."]}]})).unwrap();
        let scout = Affiliations::for_actor("scout", std::slice::from_ref(&group)).unwrap();
        let text = serde_json::to_string(&scout).unwrap();
        assert!(text.contains("Suspect the patron"));
        assert!(!text.contains("Private debt"));
        assert_eq!(scout.groups[0].roster.len(), 2);
        assert!(scout.prior_facts()[0].contains("rendezvous"));
        let mut bad = group;
        bad.members[1].actor = "scout".into();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn grants_are_projected_before_delivery_and_roster_membership_does_not_grant_a_fact() {
        let mut group:Group=serde_json::from_value(json!({"id":"band","name":"Band","kind":"band","mission":"rehearse","members":[{"actor":"Lead","role":"singer"},{"actor":"Bass","role":"bassist"}],
            "background":[{"fact":{"id":"audition","revision":1,"subject":"Bass","predicate":"agenda","value":"PRIVATE: evaluating whether to stay","source":"private briefing","confidence":"strong"},"known_by":["Bass"]}]})).unwrap();
        let lead = Affiliations::for_actor("Lead", std::slice::from_ref(&group)).unwrap();
        let bass = Affiliations::for_actor("Bass", std::slice::from_ref(&group)).unwrap();
        assert!(!serde_json::to_string(&lead).unwrap().contains("PRIVATE"));
        assert!(serde_json::to_string(&bass).unwrap().contains("PRIVATE"));
        assert!(!serde_json::to_string(&bass).unwrap().contains("known_by"));
        assert!(!bass.prompt_view().to_string().contains("PRIVATE"));
        group.background[0].known_by.push("stranger".into());
        assert!(group.validate().is_err());
    }
}
