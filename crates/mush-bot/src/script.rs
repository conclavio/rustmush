//! Source-controlled JSON command scripts, validated before connecting to the game.
use std::{io::Read, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Result, transport::validate_command};

const MAX_SCRIPT_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default = "version")]
    pub version: u32,
    pub name: String,
    pub steps: Vec<ScenarioStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub command: Option<String>,
    pub expect: Vec<String>,
    #[serde(default)]
    pub reject: Vec<String>,
    #[serde(default = "step_timeout")]
    pub timeout_ms: u64,
}

fn version() -> u32 {
    1
}
fn step_timeout() -> u64 {
    10_000
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Self> {
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        {
            return Err(format!("{}: script extension must be .json", path.display()).into());
        }
        let mut source = String::new();
        std::fs::File::open(path)?
            .take((MAX_SCRIPT_BYTES + 1) as u64)
            .read_to_string(&mut source)?;
        Self::parse(&source).map_err(|e| format!("{}: {e}", path.display()).into())
    }

    pub fn parse(source: &str) -> Result<Self> {
        if source.len() > MAX_SCRIPT_BYTES {
            return Err("script exceeds 1 MB".into());
        }
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let script: Self = serde_json::from_str(source)?;
        script.validate()?;
        Ok(script)
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(format!("unsupported script version {} (expected 1)", self.version).into());
        }
        if self.name.trim().is_empty() {
            return Err("script must have a nonempty name".into());
        }
        if self.steps.is_empty() {
            return Err("script must have at least one step".into());
        }
        for (index, step) in self.steps.iter().enumerate() {
            let validate = || -> Result<()> {
                if let Some(command) = &step.command {
                    validate_command(command)?;
                }
                if step.expect.is_empty()
                    || step
                        .expect
                        .iter()
                        .chain(&step.reject)
                        .any(|s| s.trim().is_empty())
                    || !(1..=300_000).contains(&step.timeout_ms)
                {
                    return Err(
                        "steps need nonempty expect strings and timeout_ms in 1..300000".into(),
                    );
                }
                Ok(())
            };
            validate().map_err(|e| format!("step {}: {e}", index + 1))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_preserves_literal_commands_assertions_and_wait_defaults() {
        let json = r#"{"name":"Garden","steps":[
            {"command":"@gen here=Moonlit: roses, #4, \"brass\".","expect":["Worldgen draft","Desc:"],"reject":["Permission denied"],"timeout_ms":120000},
            {"expect":["A bell rings."]}
        ]}"#;
        let script = Scenario::parse(&format!("\u{feff}{json}")).unwrap();
        assert_eq!(script.version, 1);
        assert_eq!(
            script.steps[0].command.as_deref(),
            Some("@gen here=Moonlit: roses, #4, \"brass\".")
        );
        assert_eq!(script.steps[0].expect, ["Worldgen draft", "Desc:"]);
        assert_eq!(script.steps[0].reject, ["Permission denied"]);
        assert_eq!(script.steps[0].timeout_ms, 120000);
        assert_eq!(script.steps[1].command, None);
        assert_eq!(script.steps[1].timeout_ms, 10000);
        assert!(script.steps[1].reject.is_empty());
        assert_eq!(
            Scenario::parse(&serde_json::to_string(&script).unwrap()).unwrap(),
            script
        );
    }

    #[test]
    fn malformed_scripts_fail_instead_of_silently_skipping_steps() {
        for (source, message) in [
            (r#"{"name":"X","steps":[]}"#, "at least one step"),
            (
                r#"{"name":" ","steps":[{"expect":["Garden"]}]}"#,
                "nonempty name",
            ),
            (
                r#"{"name":"X","version":2,"steps":[{"expect":["Garden"]}]}"#,
                "unsupported script version",
            ),
            (
                r#"{"name":"X","steps":[{"commmand":"look","expect":["Garden"]}]}"#,
                "unknown field",
            ),
            (
                r#"{"name":"X","steps":[{"command":"look","command":"QUIT","expect":["Garden"]}]}"#,
                "duplicate field",
            ),
            (
                r#"{"name":"X","steps":[{"expect":["Garden"],"timeout_ms":"later"}]}"#,
                "invalid type",
            ),
            (
                r#"{"name":"X","steps":[{"command":"look"}]}"#,
                "missing field",
            ),
            (r#"{"name":"X","steps":[{"expect":[]}]}"#, "nonempty expect"),
            (
                r#"{"name":"X","steps":[{"expect":["  "]}]}"#,
                "nonempty expect",
            ),
            (
                r#"{"name":"X","steps":[{"expect":["Garden"],"timeout_ms":0}]}"#,
                "timeout_ms",
            ),
            (
                r#"{"name":"X","steps":[{"expect":["Garden"],"timeout_ms":300001}]}"#,
                "timeout_ms",
            ),
            (
                r#"{"name":"X","steps":[{"command":"look\nQUIT","expect":["Garden"]}]}"#,
                "control characters",
            ),
            (
                r#"{"name":"X","steps":[{"command":"look\u001b[31m","expect":["Garden"]}]}"#,
                "control characters",
            ),
        ] {
            let error = Scenario::parse(source).unwrap_err().to_string();
            assert!(error.contains(message), "wanted {message:?}, got {error:?}");
        }
        assert!(Scenario::parse("# Garden\ncommand: look\nexpect: Garden\n").is_err());
        assert!(Scenario::parse(&" ".repeat(MAX_SCRIPT_BYTES + 1)).is_err());
    }

    #[test]
    fn checked_in_json_examples_are_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bots/scripts");
        for (name, steps) in [("smoke", 2), ("rose-garden", 7)] {
            let script = Scenario::load(&root.join(format!("{name}.json"))).unwrap();
            assert_eq!(script.steps.len(), steps);
        }
    }
}
