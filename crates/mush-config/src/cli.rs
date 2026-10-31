//! The one command line the three binaries share (WS22-S1).
//!
//! `mushd`, `mush-server` and `mush-gateway` each carried a byte-identical
//! hand-rolled scanner:
//!
//! ```text
//! args.iter().position(|a| a == "--config").and_then(|i| args.get(i + 1))
//!     .map_or_else(|| PathBuf::from("mushd.toml"), PathBuf::from)
//! ```
//!
//! which reads every argument it recognises and silently ignores every one it
//! does not. `mushd --confg prod.toml` booted the defaults — an empty world on
//! a default data dir — and said nothing; `mushd --config` with the path left
//! off did the same; `--config a.toml --config b.toml` quietly took the first.
//! Worst of all, an operator who typed the path wrong got a server that came
//! up healthy on a config nobody wrote.
//!
//! [`parse`] replaces all three. It knows the flags its caller declares in a
//! [`CliSpec`] and refuses everything else: an unknown token, a flag with no
//! value, a flag given twice. [`Cli::load_config`] then stats the path
//! *before* handing it to [`Config::load`], so an explicit `--config` that
//! names a missing file is [`CliError::MissingConfig`] rather than a silent
//! fall back to defaults — while the implicit `mushd.toml` keeps
//! [`Config::load`]'s defaults-on-`NotFound` behaviour, which is what makes a
//! config-less dev run work. That asymmetry is the whole point of tracking
//! [`Cli::config_explicit`], and it has to cross the process boundary too: the
//! gateway passes `--config` to its `mush-server` child only when it was given
//! one itself, or a defaults-booted gateway would spawn a child that refuses
//! to start on a path that does not exist.
//!
//! Nothing here may panic: `mush-config` denies `clippy::panic`/`unwrap_used`
//! at its crate root and is a ratchet crate in the panic-surface budget. Every
//! failure is a [`CliError`].

use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError};
use crate::generation::Generation;

/// The config file read when no `--config` is given, relative to the working
/// directory.
pub const DEFAULT_CONFIG_FILE: &str = "mushd.toml";

/// Which flags a binary accepts, and what its `--help` prints.
///
/// `--config <path>`, `--help` and `-h` are universal; the rest are opt-in, so
/// `mush-gateway --repl` is an error rather than a silently ignored token.
#[derive(Debug, Clone, Copy)]
pub struct CliSpec {
    /// The program name that prefixes an error on stderr (`mushd:`, ...).
    pub program: &'static str,
    /// The one-line usage printed for `--help`.
    pub usage: &'static str,
    /// Accept `--ipc <addr>` (the server half; the supervisor picks the socket).
    pub ipc: bool,
    /// Accept `--repl` (the all-in-one's offline stdin REPL).
    pub repl: bool,
    /// Accept `--check-config` (WS04-S3: validate and exit without booting).
    pub check_config: bool,
    /// Accept the offline `--bootstrap-staff <account>` operation.
    pub bootstrap_staff: bool,
    /// Accept `--generation <a|b>` (the gateway half, doc 33 §4.3: which of
    /// the two gateway generations this process is).
    pub generation: bool,
    /// Accept `--ipc-version` (the two split halves, doc 33 §4.7): print the
    /// binary's `IPC_VERSION` and exit, so a deploy can tell a server upgrade
    /// that bumps the boundary's version from one that does not.
    pub ipc_version: bool,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    /// The config path to load — the `--config` value, or
    /// [`DEFAULT_CONFIG_FILE`].
    pub config: PathBuf,
    /// True when [`Cli::config`] came from an explicit `--config`. A missing
    /// file is an error only then.
    pub config_explicit: bool,
    /// The `--ipc <addr>` override for `[ipc].listen`, when given.
    pub ipc: Option<String>,
    /// `--repl` was given.
    pub repl: bool,
    /// `--check-config` was given: load the config, run every validation that
    /// needs neither a socket nor the store, and exit on the verdict
    /// ([`crate::preflight`]).
    pub check_config: bool,
    /// Create the first durable account, linked to God, then exit.
    pub bootstrap_staff: Option<String>,
    /// The `--generation <a|b>` value, when given (doc 33 §4.3). `None` is
    /// generation `a` for the connection-id bit and says nothing else.
    pub generation: Option<Generation>,
    /// `--ipc-version` was given: the caller prints its `IPC_VERSION` and
    /// exits successfully without touching the config (doc 33 §4.7).
    pub ipc_version: bool,
    /// `--help`/`-h` was given; the caller prints [`CliSpec::usage`] and exits
    /// successfully without touching the config.
    pub help: bool,
}

impl Cli {
    /// The all-defaults line: implicit config, no overrides.
    fn empty() -> Cli {
        Cli {
            config: PathBuf::from(DEFAULT_CONFIG_FILE),
            config_explicit: false,
            ipc: None,
            repl: false,
            check_config: false,
            bootstrap_staff: None,
            generation: None,
            ipc_version: false,
            help: false,
        }
    }

    /// Load [`Cli::config`] and apply the command line's overrides.
    ///
    /// The stat lives here, once, rather than in three binaries: an explicit
    /// `--config` naming a file that is not there is
    /// [`CliError::MissingConfig`], and the implicit `mushd.toml` keeps
    /// [`Config::load`]'s defaults-on-`NotFound`. Any *other* IO error (a
    /// directory, a permission denial) falls through to `Config::load`, which
    /// reports it with its own path-carrying error.
    ///
    /// # Errors
    ///
    /// [`CliError::MissingConfig`] when an explicit path does not exist, and
    /// [`CliError::Config`] for anything [`Config::load`] rejects.
    pub fn load_config(&self) -> Result<Config, CliError> {
        if self.config_explicit
            && let Err(e) = std::fs::metadata(&self.config)
            && e.kind() == std::io::ErrorKind::NotFound
        {
            return Err(CliError::MissingConfig {
                path: self.config.clone(),
            });
        }
        let mut config = Config::load(&self.config).map_err(|e| CliError::Config(Box::new(e)))?;
        if let Some(addr) = &self.ipc {
            config.ipc.listen = addr.clone();
        }
        Ok(config)
    }

    /// The `--config` argument to hand a child process, if any.
    ///
    /// `None` means "the child should resolve its config the same implicit way
    /// I did" — passing `mushd.toml` explicitly here would turn the parent's
    /// tolerated absence into the child's hard failure.
    #[must_use]
    pub fn child_config_path(&self) -> Option<&Path> {
        self.config_explicit.then_some(self.config.as_path())
    }
}

/// Everything the shared command line can refuse.
#[derive(Debug)]
pub enum CliError {
    /// A token the [`CliSpec`] does not declare.
    UnknownArg { arg: String },
    /// A value-taking flag with nothing after it.
    MissingValue { flag: &'static str },
    /// A flag given more than once, where the second would have silently won
    /// or silently lost.
    Repeated { flag: &'static str },
    /// A value-taking flag given a value it does not take.
    InvalidValue { flag: &'static str, value: String },
    /// Two operations that cannot safely run together.
    ConflictingOperations,
    /// An explicit `--config` naming a file that is not there.
    MissingConfig { path: PathBuf },
    /// The config itself would not load.
    Config(Box<ConfigError>),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownArg { arg } => write!(f, "unrecognised argument `{arg}`"),
            CliError::MissingValue { flag } => write!(f, "`{flag}` needs a value"),
            CliError::Repeated { flag } => write!(f, "`{flag}` given more than once"),
            CliError::InvalidValue { flag, value } => {
                write!(f, "`{flag}` does not take `{value}`")
            }
            CliError::ConflictingOperations => write!(
                f,
                "`--bootstrap-staff` cannot be combined with `--check-config`, `--repl` or `--ipc`"
            ),
            CliError::MissingConfig { path } => {
                write!(f, "config file not found: {}", path.display())
            }
            CliError::Config(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Config(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

/// Store `value` in `slot`, refusing a repeat and a missing value.
fn take_once(
    slot: &mut Option<String>,
    flag: &'static str,
    value: Option<String>,
) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError::Repeated { flag });
    }
    match value {
        Some(v) => {
            *slot = Some(v);
            Ok(())
        }
        None => Err(CliError::MissingValue { flag }),
    }
}

/// Parse `argv` (the arguments *after* the program name) against `spec`.
///
/// `--help`/`-h` short-circuits: it is honoured wherever it appears, so a
/// mistyped line can still ask what the right one is.
///
/// # Errors
///
/// [`CliError::UnknownArg`], [`CliError::MissingValue`], [`CliError::Repeated`]
/// or [`CliError::InvalidValue`] — never a silent acceptance.
pub fn parse<I, S>(argv: I, spec: &CliSpec) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = argv
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Cli {
            help: true,
            ..Cli::empty()
        });
    }

    let mut config: Option<String> = None;
    let mut ipc: Option<String> = None;
    let mut repl = false;
    let mut check_config = false;
    let mut bootstrap_staff = None;
    let mut generation: Option<String> = None;
    let mut ipc_version = false;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--config" => take_once(&mut config, "--config", rest.next())?,
            "--ipc" if spec.ipc => take_once(&mut ipc, "--ipc", rest.next())?,
            "--bootstrap-staff" if spec.bootstrap_staff => {
                take_once(&mut bootstrap_staff, "--bootstrap-staff", rest.next())?;
            }
            "--repl" if spec.repl => {
                if repl {
                    return Err(CliError::Repeated { flag: "--repl" });
                }
                repl = true;
            }
            "--check-config" if spec.check_config => {
                if check_config {
                    return Err(CliError::Repeated {
                        flag: "--check-config",
                    });
                }
                check_config = true;
            }
            "--generation" if spec.generation => {
                take_once(&mut generation, "--generation", rest.next())?;
            }
            "--ipc-version" if spec.ipc_version => {
                if ipc_version {
                    return Err(CliError::Repeated {
                        flag: "--ipc-version",
                    });
                }
                ipc_version = true;
            }
            _ => return Err(CliError::UnknownArg { arg }),
        }
    }
    let generation = match generation {
        None => None,
        Some(value) => Some(
            value
                .parse::<Generation>()
                .map_err(|_| CliError::InvalidValue {
                    flag: "--generation",
                    value,
                })?,
        ),
    };

    if bootstrap_staff.is_some() && (check_config || repl || ipc.is_some()) {
        return Err(CliError::ConflictingOperations);
    }
    if ipc_version && (bootstrap_staff.is_some() || check_config || repl) {
        return Err(CliError::ConflictingOperations);
    }

    Ok(Cli {
        config_explicit: config.is_some(),
        config: config.map_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE), PathBuf::from),
        ipc,
        repl,
        check_config,
        bootstrap_staff,
        generation,
        ipc_version,
        help: false,
    })
}

/// Parse the real process arguments (skipping `argv[0]`).
///
/// # Errors
///
/// As [`parse`].
pub fn parse_env(spec: &CliSpec) -> Result<Cli, CliError> {
    parse(std::env::args().skip(1), spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MUSHD: CliSpec = CliSpec {
        program: "mushd",
        usage: "Usage: mushd [--repl] [--check-config] [--config <path>]",
        ipc: false,
        repl: true,
        check_config: true,
        bootstrap_staff: true,
        generation: false,
        ipc_version: false,
    };
    const SERVER: CliSpec = CliSpec {
        program: "mush-server",
        usage: "Usage: mush-server [--check-config] [--config <path>] [--ipc <addr>]",
        ipc: true,
        repl: false,
        check_config: true,
        bootstrap_staff: true,
        generation: false,
        ipc_version: true,
    };
    const GATEWAY: CliSpec = CliSpec {
        program: "mush-gateway",
        usage: "Usage: mush-gateway [--check-config] [--generation a|b] [--config <path>]",
        ipc: false,
        repl: false,
        check_config: true,
        bootstrap_staff: false,
        generation: true,
        ipc_version: true,
    };
    /// A spec that declares NOTHING beyond the universal flags, so the
    /// opt-in-ness of `--check-config` is provable rather than assumed.
    const BARE: CliSpec = CliSpec {
        program: "bare",
        usage: "Usage: bare [--config <path>]",
        ipc: false,
        repl: false,
        check_config: false,
        bootstrap_staff: false,
        generation: false,
        ipc_version: false,
    };

    /// Doc 33 §4.7: `--ipc-version` is the two split halves' flag, once,
    /// alone (it prints and exits, so it conflicts with every other
    /// operation); `mushd` and a bare spec refuse it.
    #[test]
    fn ipc_version_belongs_to_the_split_halves_and_stands_alone() {
        for spec in [&SERVER, &GATEWAY] {
            let cli = parse(["--ipc-version"], spec).unwrap();
            assert!(cli.ipc_version);
            assert!(!cli.check_config);
            let err = parse(["--ipc-version", "--ipc-version"], spec).unwrap_err();
            assert!(
                matches!(
                    err,
                    CliError::Repeated {
                        flag: "--ipc-version"
                    }
                ),
                "{err:?}"
            );
            let err = parse(["--ipc-version", "--check-config"], spec).unwrap_err();
            assert!(matches!(err, CliError::ConflictingOperations), "{err:?}");
        }
        for spec in [&MUSHD, &BARE] {
            let err = parse(["--ipc-version"], spec).unwrap_err();
            assert!(
                matches!(&err, CliError::UnknownArg { arg } if arg == "--ipc-version"),
                "{err:?}"
            );
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mush-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The failure the old scanner could not express: a mistyped flag booted
    /// the defaults in silence.
    #[test]
    fn mistyped_flag_is_refused_not_ignored() {
        let err = parse(["--confg", "prod.toml"], &MUSHD).unwrap_err();
        assert!(
            matches!(&err, CliError::UnknownArg { arg } if arg == "--confg"),
            "{err:?}"
        );
        // The offending token has to reach the operator's terminal.
        assert!(err.to_string().contains("--confg"), "{err}");
    }

    #[test]
    fn positional_argument_is_refused() {
        let err = parse(["prod.toml"], &MUSHD).unwrap_err();
        assert!(
            matches!(&err, CliError::UnknownArg { arg } if arg == "prod.toml"),
            "{err:?}"
        );
    }

    #[test]
    fn bootstrap_is_explicit_single_and_exclusive() {
        for spec in [&MUSHD, &SERVER] {
            assert_eq!(
                parse(["--bootstrap-staff", "operator"], spec)
                    .unwrap()
                    .bootstrap_staff
                    .as_deref(),
                Some("operator")
            );
            assert!(parse(["--bootstrap-staff"], spec).is_err());
            assert!(
                parse(
                    ["--bootstrap-staff", "one", "--bootstrap-staff", "two"],
                    spec
                )
                .is_err()
            );
            assert!(matches!(
                parse(["--bootstrap-staff", "operator", "--check-config"], spec),
                Err(CliError::ConflictingOperations)
            ));
        }
        assert!(matches!(
            parse(["--bootstrap-staff", "operator", "--repl"], &MUSHD),
            Err(CliError::ConflictingOperations)
        ));
        assert!(matches!(
            parse(["--bootstrap-staff", "operator", "--ipc", "foo"], &SERVER),
            Err(CliError::ConflictingOperations)
        ));
        assert!(parse(["--bootstrap-staff", "operator"], &BARE).is_err());
    }

    #[test]
    fn config_without_a_value_is_refused() {
        let err = parse(["--config"], &MUSHD).unwrap_err();
        assert!(
            matches!(err, CliError::MissingValue { flag: "--config" }),
            "{err:?}"
        );
    }

    #[test]
    fn repeated_config_is_refused() {
        let err = parse(["--config", "a.toml", "--config", "b.toml"], &MUSHD).unwrap_err();
        assert!(
            matches!(err, CliError::Repeated { flag: "--config" }),
            "{err:?}"
        );
    }

    #[test]
    fn repeated_repl_is_refused() {
        let err = parse(["--repl", "--repl"], &MUSHD).unwrap_err();
        assert!(
            matches!(err, CliError::Repeated { flag: "--repl" }),
            "{err:?}"
        );
    }

    /// A flag one binary takes is not a flag every binary takes.
    #[test]
    fn undeclared_flags_are_not_silently_accepted() {
        let err = parse(["--repl"], &SERVER).unwrap_err();
        assert!(
            matches!(&err, CliError::UnknownArg { arg } if arg == "--repl"),
            "{err:?}"
        );
        let err = parse(["--ipc", "127.0.0.1:1"], &MUSHD).unwrap_err();
        assert!(
            matches!(&err, CliError::UnknownArg { arg } if arg == "--ipc"),
            "{err:?}"
        );
    }

    #[test]
    fn help_short_circuits_before_anything_else() {
        let cli = parse(["--help"], &MUSHD).unwrap();
        assert!(cli.help);
        assert!(parse(["-h"], &SERVER).unwrap().help);
        // Even beside a token that would otherwise be refused.
        assert!(parse(["--confg", "--help"], &MUSHD).unwrap().help);
    }

    #[test]
    fn check_config_is_opt_in_and_not_repeatable() {
        assert!(parse(["--check-config"], &MUSHD).unwrap().check_config);
        assert!(parse(["--check-config"], &SERVER).unwrap().check_config);
        assert!(
            !parse(std::iter::empty::<&str>(), &MUSHD)
                .unwrap()
                .check_config
        );
        let err = parse(["--check-config", "--check-config"], &MUSHD).unwrap_err();
        assert!(
            matches!(
                err,
                CliError::Repeated {
                    flag: "--check-config"
                }
            ),
            "{err:?}"
        );
        // A binary that does not declare it must refuse it rather than drop it:
        // an operator who thought they had preflighted a config and had not is
        // exactly the silence this parser exists to abolish.
        let err = parse(["--check-config"], &BARE).unwrap_err();
        assert!(
            matches!(&err, CliError::UnknownArg { arg } if arg == "--check-config"),
            "{err:?}"
        );
    }

    /// Doc 33 §4.3: `--generation` is the gateway's, takes exactly `a` or
    /// `b`, once; absent means `None` (generation `a` for the id bit).
    #[test]
    fn generation_is_the_gateways_and_takes_a_or_b_once() {
        assert_eq!(
            parse(["--generation", "a"], &GATEWAY).unwrap().generation,
            Some(Generation::A)
        );
        assert_eq!(
            parse(["--generation", "B"], &GATEWAY).unwrap().generation,
            Some(Generation::B)
        );
        assert_eq!(
            parse(std::iter::empty::<&str>(), &GATEWAY)
                .unwrap()
                .generation,
            None
        );
        let err = parse(["--generation"], &GATEWAY).unwrap_err();
        assert!(
            matches!(
                err,
                CliError::MissingValue {
                    flag: "--generation"
                }
            ),
            "{err:?}"
        );
        let err = parse(["--generation", "c"], &GATEWAY).unwrap_err();
        assert!(
            matches!(&err, CliError::InvalidValue { flag: "--generation", value } if value == "c"),
            "{err:?}"
        );
        assert!(err.to_string().contains("`c`"), "{err}");
        let err = parse(["--generation", "a", "--generation", "b"], &GATEWAY).unwrap_err();
        assert!(
            matches!(
                err,
                CliError::Repeated {
                    flag: "--generation"
                }
            ),
            "{err:?}"
        );
        // The other two binaries have no generations.
        for spec in [&MUSHD, &SERVER, &BARE] {
            let err = parse(["--generation", "a"], spec).unwrap_err();
            assert!(
                matches!(&err, CliError::UnknownArg { arg } if arg == "--generation"),
                "{err:?}"
            );
        }
    }

    #[test]
    fn no_arguments_is_the_implicit_config() {
        let cli = parse(std::iter::empty::<&str>(), &MUSHD).unwrap();
        assert_eq!(cli.config, PathBuf::from(DEFAULT_CONFIG_FILE));
        assert!(!cli.config_explicit);
        assert!(!cli.repl);
        assert_eq!(cli.child_config_path(), None);
    }

    #[test]
    fn explicit_config_is_marked_and_crosses_to_a_child() {
        let cli = parse(["--config", "prod.toml"], &MUSHD).unwrap();
        assert_eq!(cli.config, PathBuf::from("prod.toml"));
        assert!(cli.config_explicit);
        assert_eq!(cli.child_config_path(), Some(Path::new("prod.toml")));
    }

    /// The bound: an explicit path that is not there stops the boot.
    #[test]
    fn explicit_missing_config_is_an_error() {
        let dir = scratch("missing");
        let path = dir.join("nope.toml");
        let cli = parse(["--config", &path.display().to_string()], &MUSHD).unwrap();
        let err = cli.load_config().unwrap_err();
        assert!(
            matches!(&err, CliError::MissingConfig { path: p } if p == &path),
            "{err:?}"
        );
        // The path the operator mistyped has to reach the operator.
        assert!(err.to_string().contains("nope.toml"), "{err}");
    }

    /// ...and the implicit one that is not there still boots the defaults.
    #[test]
    fn implicit_missing_config_is_the_defaults() {
        let dir = scratch("implicit");
        let cli = Cli {
            config: dir.join(DEFAULT_CONFIG_FILE),
            ..Cli::empty()
        };
        let config = cli.load_config().unwrap();
        assert_eq!(config.ipc.listen, Config::default().ipc.listen);
    }

    #[test]
    fn ipc_flag_overrides_the_configured_listen() {
        let dir = scratch("ipc");
        let path = dir.join("mushd.toml");
        std::fs::write(&path, "[ipc]\nlisten = \"127.0.0.1:7001\"\n").unwrap();

        let plain = parse(["--config", &path.display().to_string()], &SERVER).unwrap();
        assert_eq!(plain.load_config().unwrap().ipc.listen, "127.0.0.1:7001");

        let overridden = parse(
            [
                "--config",
                &path.display().to_string(),
                "--ipc",
                "127.0.0.1:7099",
            ],
            &SERVER,
        )
        .unwrap();
        assert_eq!(
            overridden.load_config().unwrap().ipc.listen,
            "127.0.0.1:7099"
        );
    }

    #[test]
    fn a_malformed_config_still_reports_itself() {
        let dir = scratch("malformed");
        let path = dir.join("bad.toml");
        std::fs::write(&path, "this is not toml = = =\n").unwrap();
        let cli = parse(["--config", &path.display().to_string()], &MUSHD).unwrap();
        let err = cli.load_config().unwrap_err();
        assert!(matches!(err, CliError::Config(_)), "{err:?}");
    }
}
