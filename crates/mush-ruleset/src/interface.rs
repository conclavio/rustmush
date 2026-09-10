//! S6 (doc 14 §4.6): the off-duty gamemaster's interface — `chargen.explain`
//! over the loaded ruleset's chargen stages and `rules.lookup` over the help
//! topics, both verbatim reads. Composed by the server as a small module
//! beside the ruleset, carrying the stages it snapshotted at composition.

use std::sync::Arc;

use mush_core::{
    AbilityCall, Ctx, GameModule, InterfaceAbility, InterfaceAbilityProvider, InterfaceError,
    InterfaceReply, ModError, Registrar,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::chargen::ChargenStage;

/// The provider id.
pub const RULES_INTERFACE: &str = "rules";

/// The module name.
pub const RULES_INTERFACE_MODULE: &str = "rules_interface";

/// The gamemaster's desk: the chargen stages of the loaded ruleset.
#[derive(Clone, Debug, Default)]
pub struct RulesInterface {
    /// The ruleset's display name.
    pub ruleset: String,
    /// The chargen stages, in order.
    pub stages: Vec<ChargenStage>,
}

impl InterfaceAbilityProvider for RulesInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("chargen.explain"),
                description: "Explain character generation: every stage in order, or one stage's prompt and budget."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "stage": { "type": "string", "description": "a stage id or label; omit for the overview" } }
                }),
                command: SmolStr::new_static("help chargen"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("rules.lookup"),
                description: "Look a rules topic up in the help, verbatim.".to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "topic": { "type": "string" } },
                    "required": ["topic"]
                }),
                command: SmolStr::new_static("help"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        _call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        match name {
            "chargen.explain" => {
                if self.stages.is_empty() {
                    return Err(InterfaceError::Refused(
                        "No ruleset with guided character generation is loaded here.".to_string(),
                    ));
                }
                let wanted = input
                    .get("stage")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty());
                match wanted {
                    Some(w) => {
                        let stage = self
                            .stages
                            .iter()
                            .find(|s| {
                                s.id.to_ascii_lowercase() == w || s.label.to_ascii_lowercase() == w
                            })
                            .ok_or_else(|| {
                                InterfaceError::Refused(format!(
                                    "No chargen stage called {w:?}; the stages are {}.",
                                    self.stages
                                        .iter()
                                        .map(|s| s.label.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ))
                            })?;
                        let mut lines = vec![format!("{} ({})", stage.label, stage.id)];
                        lines.push(stage.prompt.clone());
                        if let Some(b) = &stage.budget {
                            lines.push(format!(
                                "Budget: {} pool{}, {} cost rule{}.",
                                b.pools.len(),
                                if b.pools.len() == 1 { "" } else { "s" },
                                b.rules.len(),
                                if b.rules.len() == 1 { "" } else { "s" }
                            ));
                        }
                        Ok(InterfaceReply::public_lines(&lines))
                    }
                    None => {
                        let mut lines = vec![format!(
                            "Character generation for {} runs in {} stages:",
                            self.ruleset,
                            self.stages.len()
                        )];
                        for (i, s) in self.stages.iter().enumerate() {
                            lines.push(format!(
                                "{}. {} — {}",
                                i + 1,
                                s.label,
                                first_sentence(&s.prompt)
                            ));
                        }
                        lines.push(
                            "`+chargen` starts it; `help chargen` has the long form.".to_string(),
                        );
                        Ok(InterfaceReply::public_lines(&lines))
                    }
                }
            }
            "rules.lookup" => {
                let topic = input
                    .get("topic")
                    .and_then(Value::as_str)
                    .map(|t| t.trim().to_ascii_lowercase())
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| InterfaceError::Invalid("a topic".to_string()))?;
                match ctx.reg.help_topic(&topic) {
                    Some(t) => Ok(InterfaceReply::public(format!("{}\n{}", t.title, t.body))),
                    None => Err(InterfaceError::Refused(format!(
                        "Nothing in the help under {topic:?}; `help` lists the topics."
                    ))),
                }
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}

fn first_sentence(text: &str) -> String {
    let t = text.trim();
    let end = t.find(['.', '!', '?']).map_or(t.len(), |i| i + 1);
    t[..end.min(160)].to_string()
}

/// The module the server composes beside a ruleset to front its rules.
#[derive(Clone, Debug)]
pub struct RulesInterfaceModule {
    provider: Arc<RulesInterface>,
}

impl RulesInterfaceModule {
    /// A module fronting `ruleset`'s chargen `stages`.
    #[must_use]
    pub fn new(ruleset: impl Into<String>, stages: Vec<ChargenStage>) -> RulesInterfaceModule {
        RulesInterfaceModule {
            provider: Arc::new(RulesInterface {
                ruleset: ruleset.into(),
                stages,
            }),
        }
    }
}

impl GameModule for RulesInterfaceModule {
    fn name(&self) -> &'static str {
        RULES_INTERFACE_MODULE
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.interface_provider(
            RULES_INTERFACE,
            Arc::clone(&self.provider) as Arc<dyn InterfaceAbilityProvider>,
        )
    }
}
