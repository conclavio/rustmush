//! `sshdoorway.toml` (doc 29 §10): parsing and validation.
//!
//! Every rule is checked at load. A `SIGHUP` reload that fails validation
//! keeps the old table; it never half-applies.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// The default drain warning, with the owner's wording (Q-29-2, 2026-10-30).
pub const DEFAULT_DRAIN_MESSAGE: &str = "ATTENTION: Game Doorway updated and requires a restart, please find a safe place and reconnect within the next {remaining}.";

/// The whole configuration file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// `[listen]`.
    pub listen: ListenCfg,
    /// `[limits]`.
    #[serde(default)]
    pub limits: LimitsCfg,
    /// `[log]`.
    #[serde(default)]
    pub log: LogCfg,
    /// `[menu]`.
    #[serde(default)]
    pub menu: MenuCfg,
    /// `[[service]]`, in menu order.
    #[serde(default, rename = "service")]
    pub services: Vec<ServiceCfg>,
}

/// `[listen]`. Neither key reloads on `SIGHUP`; changing them needs a swap.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListenCfg {
    /// Explicit addresses only — never `0.0.0.0` or `[::]` (D-29-3).
    pub addrs: Vec<SocketAddr>,
    /// The doorway's ed25519 host key, shared by every generation.
    pub host_key: PathBuf,
}

/// `[limits]` (doc 29 §8, §9.4).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsCfg {
    /// Concurrent connections per address (IPv6: per /64).
    pub per_ip_connections: u32,
    /// New connections per address per minute (IPv6: per /64).
    pub per_ip_new_per_minute: u32,
    /// Concurrent connections in total.
    pub global_connections: u32,
    /// Concurrent connections per IPv6 /48, across all of its /64s.
    pub per_v6_48_connections: u32,
    /// Seconds to authenticate (or reach the menu) before being dropped.
    pub auth_deadline_secs: u64,
    /// Seconds the menu waits for input.
    pub menu_idle_secs: u64,
    /// The drain deadline, in seconds after the drain begins.
    pub drain_deadline_secs: u64,
    /// Warnings, as time **remaining** before the deadline (`12h`, `15m`).
    pub drain_warnings: Vec<String>,
    /// The warning line; `{remaining}` is replaced by the warning's label.
    pub drain_message: String,
    /// Seconds a forced finish (`SIGUSR1`) waits after its one warning.
    pub force_grace_secs: u64,
}

impl Default for LimitsCfg {
    fn default() -> Self {
        LimitsCfg {
            per_ip_connections: 8,
            per_ip_new_per_minute: 20,
            global_connections: 2048,
            per_v6_48_connections: 64,
            auth_deadline_secs: 30,
            menu_idle_secs: 120,
            drain_deadline_secs: 86_400,
            drain_warnings: ["12h", "6h", "2h", "1h", "15m", "1m"]
                .map(str::to_owned)
                .to_vec(),
            drain_message: DEFAULT_DRAIN_MESSAGE.to_owned(),
            force_grace_secs: 30,
        }
    }
}

/// `[log]` (doc 29 §8.1).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogCfg {
    /// Directory of the file sink (`<generation>.log`). `None` = no file.
    pub file_dir: Option<PathBuf>,
    /// The file sink's level.
    pub file_level: String,
    /// The journald (stderr) sink's level.
    pub journal_level: String,
}

impl Default for LogCfg {
    fn default() -> Self {
        LogCfg {
            file_dir: None,
            file_level: "info".to_owned(),
            journal_level: "warn".to_owned(),
        }
    }
}

/// `[menu]` (doc 29 §7).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuCfg {
    /// The first line of the menu.
    pub title: String,
    /// The host players type (`conclav.io`), used in hints.
    pub host: String,
    /// Q-29-3: route non-service names to this service as account names
    /// instead of showing the menu. Unset by the owner's ruling.
    pub default_service: Option<String>,
}

impl Default for MenuCfg {
    fn default() -> Self {
        MenuCfg {
            title: "Choose a game server".to_owned(),
            host: "this server".to_owned(),
            default_service: None,
        }
    }
}

/// One `[[service]]`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceCfg {
    /// The routing name (`dev1`): lowercase `[a-z0-9-]`, 1–32 bytes.
    pub name: String,
    /// The menu's description.
    #[serde(default)]
    pub title: String,
    /// The instance's trusted-ingress socket.
    pub socket: PathBuf,
    /// Shown in the menu (Q-29-6).
    #[serde(default = "yes")]
    pub listed: bool,
}

fn yes() -> bool {
    true
}

/// A configuration error, with the rule that failed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("reading {path}: {source}")]
    Read {
        /// The path.
        path: String,
        /// The error.
        source: std::io::Error,
    },
    /// The TOML did not parse.
    #[error("parsing: {0}")]
    Parse(#[from] toml::de::Error),
    /// A rule failed.
    #[error("{0}")]
    Invalid(String),
}

/// A parsed drain warning: how long before the deadline, and its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// Time remaining when the warning goes out.
    pub remaining: Duration,
    /// The label substituted for `{remaining}` (exactly as configured).
    pub label: String,
}

impl Config {
    /// Read and validate a file.
    pub fn load(path: &std::path::Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Config::parse(&text)
    }

    /// Parse and validate text.
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let cfg: Config = toml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Check every rule of doc 29 §10.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Invalid(m));
        if self.listen.addrs.is_empty() {
            return bad("[listen].addrs is empty".into());
        }
        for a in &self.listen.addrs {
            if a.ip().is_unspecified() {
                return bad(format!(
                    "[listen].addrs: {a} is a wildcard; name explicit addresses (D-29-3)"
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.services {
            if !valid_service_name(&s.name) {
                return bad(format!(
                    "service name {:?} must be 1-32 bytes of lowercase [a-z0-9-]",
                    s.name
                ));
            }
            if matches!(s.name.as_str(), "play" | "guest" | "new") {
                return bad(format!(
                    "service name {:?} is an instance front-door name",
                    s.name
                ));
            }
            if !seen.insert(s.name.clone()) {
                return bad(format!("service name {:?} appears twice", s.name));
            }
            if s.socket.as_os_str().is_empty() {
                return bad(format!("service {:?} has no socket", s.name));
            }
        }
        if let Some(d) = &self.menu.default_service
            && !seen.contains(d)
        {
            return bad(format!("[menu].default_service {d:?} is not a service"));
        }
        let l = &self.limits;
        if l.per_ip_connections == 0
            || l.per_ip_new_per_minute == 0
            || l.global_connections == 0
            || l.per_v6_48_connections == 0
            || l.auth_deadline_secs == 0
            || l.menu_idle_secs == 0
            || l.drain_deadline_secs == 0
            || l.force_grace_secs == 0
        {
            return bad("[limits] values must all be greater than zero".into());
        }
        if !l.drain_message.contains("{remaining}") {
            return bad("[limits].drain_message must contain {remaining}".into());
        }
        self.warnings()?;
        if !matches!(
            self.log.file_level.as_str(),
            "error" | "warn" | "info" | "debug" | "trace"
        ) || !matches!(
            self.log.journal_level.as_str(),
            "error" | "warn" | "info" | "debug" | "trace"
        ) {
            return bad("[log] levels must be error, warn, info, debug or trace".into());
        }
        Ok(())
    }

    /// The drain warnings, parsed, longest remaining first.
    pub fn warnings(&self) -> Result<Vec<Warning>, ConfigError> {
        let deadline = Duration::from_secs(self.limits.drain_deadline_secs);
        let mut out = Vec::new();
        for label in &self.limits.drain_warnings {
            let Some(remaining) = parse_duration(label) else {
                return Err(ConfigError::Invalid(format!(
                    "[limits].drain_warnings: {label:?} is not a duration like 12h, 15m or 30s"
                )));
            };
            if remaining >= deadline || remaining.is_zero() {
                return Err(ConfigError::Invalid(format!(
                    "[limits].drain_warnings: {label:?} is not shorter than the deadline"
                )));
            }
            out.push(Warning {
                remaining,
                label: label.clone(),
            });
        }
        out.sort_by_key(|w| std::cmp::Reverse(w.remaining));
        Ok(out)
    }

    /// The service named `name`, ASCII case-insensitively.
    #[must_use]
    pub fn service(&self, name: &str) -> Option<&ServiceCfg> {
        self.services
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

fn valid_service_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `30s`, `15m`, `12h` (one number, one unit).
#[must_use]
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let digits = s.get(..s.len().checked_sub(unit.len_utf8())?)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let secs = match unit {
        's' => n,
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[listen]
addrs = ["127.0.0.1:2200", "[::1]:2200"]
host_key = "host_ed25519"

[menu]
title = "conclav.io — choose a game server"
host = "conclav.io"

[[service]]
name = "dev1"
title = "Development server 1"
socket = "/run/rustmush-ssh/dev1.sock"

[[service]]
name = "dev2"
title = "Development server 2"
socket = "/run/rustmush-ssh/dev2.sock"
listed = false
"#;

    #[test]
    fn the_example_parses_with_the_owners_defaults() {
        let cfg = Config::parse(GOOD).unwrap();
        assert_eq!(cfg.services.len(), 2);
        assert!(cfg.services[0].listed);
        assert!(!cfg.services[1].listed);
        assert_eq!(cfg.limits.drain_deadline_secs, 86_400);
        let w = cfg.warnings().unwrap();
        let labels: Vec<_> = w.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["12h", "6h", "2h", "1h", "15m", "1m"]);
        assert_eq!(w[0].remaining, Duration::from_secs(12 * 3600));
        assert!(
            cfg.limits
                .drain_message
                .starts_with("ATTENTION: Game Doorway updated")
        );
        assert_eq!(cfg.service("DEV1").unwrap().name, "dev1");
    }

    #[test]
    fn the_shipped_example_config_parses() {
        let text = include_str!("../../../deploy/sshdoorway/sshdoorway.toml");
        Config::parse(text).unwrap();
    }

    #[test]
    fn every_rule_is_enforced() {
        let cases = [
            (
                "addrs = [\"127.0.0.1:2200\", \"[::1]:2200\"]",
                "addrs = []",
                "empty",
            ),
            (
                "addrs = [\"127.0.0.1:2200\", \"[::1]:2200\"]",
                "addrs = [\"0.0.0.0:22\"]",
                "wildcard",
            ),
            (
                "addrs = [\"127.0.0.1:2200\", \"[::1]:2200\"]",
                "addrs = [\"[::]:22\"]",
                "wildcard",
            ),
            ("name = \"dev2\"", "name = \"dev1\"", "twice"),
            ("name = \"dev2\"", "name = \"Dev2\"", "lowercase"),
            ("name = \"dev2\"", "name = \"play\"", "front-door"),
            ("name = \"dev2\"", "name = \"a+b\"", "lowercase"),
            ("name = \"dev2\"", "name = \"\"", "lowercase"),
            (
                "host = \"conclav.io\"",
                "host = \"conclav.io\"\ndefault_service = \"dev9\"",
                "not a service",
            ),
            ("[menu]", "[limits]\nper_ip_connections = 0\n[menu]", "zero"),
            ("[menu]", "[limits]\nforce_grace_secs = 0\n[menu]", "zero"),
            (
                "[menu]",
                "[limits]\ndrain_warnings = [\"25h\"]\n[menu]",
                "shorter",
            ),
            (
                "[menu]",
                "[limits]\ndrain_warnings = [\"10x\"]\n[menu]",
                "duration",
            ),
            (
                "[menu]",
                "[limits]\ndrain_message = \"bye\"\n[menu]",
                "{remaining}",
            ),
            ("[menu]", "[log]\nfile_level = \"loud\"\n[menu]", "levels"),
            ("[menu]", "[mystery]\n[menu]", "unknown field"),
        ];
        for (from, to, want) in cases {
            let text = GOOD.replacen(from, to, 1);
            assert_ne!(text, GOOD, "{from}");
            let err = Config::parse(&text).unwrap_err().to_string();
            assert!(err.contains(want), "{to}: {err}");
        }
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("15m"), Some(Duration::from_secs(900)));
        assert_eq!(parse_duration("12h"), Some(Duration::from_secs(43_200)));
        for bad in [
            "",
            "h",
            "1",
            "1d",
            "-1h",
            "1.5h",
            "99999999999999999999h",
            "1hh",
        ] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }
}
