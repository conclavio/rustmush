//! Gateway generations (doc 33 §4.3 and Q-33-1, cut GG.2).
//!
//! A server serves several gateways at once (two during a gateway swap: the
//! draining one and its successor). Each gateway allocates its own connection
//! ids — SSH from 1, the trusted ingress from 2^31, web from 2^32, Discord from
//! 2^48 — so two gateways would hand out the SAME ids, and the server, whose
//! world keys its connection map by id alone, could not tell the two apart.
//! `mush-gateway --generation a|b` names the gateway's generation, and a
//! generation `b` gateway sets [`Generation::CONN_ID_BIT`] (bit 62) on every id
//! it allocates, across all four transports; generation `a` sets none. The
//! per-transport bases stay where they are, so an id still says which
//! transport allocated it.
//!
//! The flag is optional: a gateway started without it (every test, and `mushd`,
//! which has no second gateway) is generation `a` for this purpose. A server
//! that sees one id announced on two live links refuses the second (doc 33
//! §4.3) and its log line names the generation of the id, so a `--generation`
//! left off one of two gateways is a line in the server log, not a player
//! wired to another player's character.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Which of the two gateway generations a process is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Generation {
    /// The generation that sets no bit: the default, and every gateway before
    /// the flag existed.
    A,
    /// The generation that sets [`Generation::CONN_ID_BIT`] on every
    /// connection id it allocates.
    B,
}

impl Generation {
    /// The bit generation `b` sets on every connection id (Q-33-1): above
    /// every transport's base (Discord's is 2^48) and below the sign bit, so
    /// an id stays a positive number wherever it is widened.
    pub const CONN_ID_BIT: u64 = 1 << 62;

    /// The bit this generation sets on every connection id it allocates:
    /// none for `a`, [`Generation::CONN_ID_BIT`] for `b`.
    #[must_use]
    pub fn conn_id_bit(self) -> u64 {
        match self {
            Generation::A => 0,
            Generation::B => Generation::CONN_ID_BIT,
        }
    }

    /// Which generation allocated `conn`, read off the bit.
    #[must_use]
    pub fn of_conn_id(conn: u64) -> Generation {
        if conn & Generation::CONN_ID_BIT == 0 {
            Generation::A
        } else {
            Generation::B
        }
    }

    /// The letter the flag takes: `a` or `b`.
    #[must_use]
    pub fn letter(self) -> &'static str {
        match self {
            Generation::A => "a",
            Generation::B => "b",
        }
    }

    /// This generation's trusted-ingress socket, beside the configured one
    /// (doc 33 §4.4): `<stem>.<gen>.sock` for a `<stem>.sock`, so
    /// `/run/rustmush-ssh/dev1.sock` is bound as `dev1.a.sock` or `dev1.b.sock`
    /// and the configured name is the symlink the swap script re-points. A
    /// name without an extension takes the letter as one (`dev2` →
    /// `dev2.a`); only the last extension is moved past.
    #[must_use]
    pub fn socket_path(self, configured: &Path) -> PathBuf {
        let stem = configured
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut name = format!("{stem}.{}", self.letter());
        if let Some(ext) = configured.extension() {
            name.push('.');
            name.push_str(&ext.to_string_lossy());
        }
        configured.with_file_name(name)
    }
}

// Above the highest transport base and below the sign bit (the module docs).
const _: () = assert!(Generation::CONN_ID_BIT > 1 << 48 && Generation::CONN_ID_BIT < 1 << 63);

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.letter())
    }
}

/// A `--generation` value that is neither `a` nor `b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseGenerationError {
    /// What was given.
    pub value: String,
}

impl fmt::Display for ParseGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a generation; expected `a` or `b`",
            self.value
        )
    }
}

impl std::error::Error for ParseGenerationError {}

impl FromStr for Generation {
    type Err = ParseGenerationError;

    /// `a`/`A` and `b`/`B`, surrounding whitespace ignored.
    fn from_str(s: &str) -> Result<Generation, ParseGenerationError> {
        match s.trim() {
            "a" | "A" => Ok(Generation::A),
            "b" | "B" => Ok(Generation::B),
            other => Err(ParseGenerationError {
                value: other.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_takes_a_or_b_and_nothing_else() {
        assert_eq!("a".parse::<Generation>(), Ok(Generation::A));
        assert_eq!(" B ".parse::<Generation>(), Ok(Generation::B));
        let err = "c".parse::<Generation>().unwrap_err();
        assert_eq!(err.value, "c");
        assert!(err.to_string().contains("`c`"), "{err}");
        assert!("".parse::<Generation>().is_err());
        assert!("ab".parse::<Generation>().is_err());
        assert_eq!(Generation::A.to_string(), "a");
        assert_eq!(Generation::B.to_string(), "b");
    }

    /// Generation `b` sets bit 62 and `a` sets nothing; the bit is read back
    /// off an id from any of the four transports' ranges, bases intact.
    #[test]
    fn the_bit_is_set_by_b_only_and_reads_back_from_any_transport() {
        assert_eq!(Generation::A.conn_id_bit(), 0);
        assert_eq!(Generation::B.conn_id_bit(), 1 << 62);
        for base in [1u64, 1 << 31, 1 << 32, 1 << 48] {
            let id = base + 7;
            assert_eq!(Generation::of_conn_id(id), Generation::A);
            let b = id | Generation::B.conn_id_bit();
            assert_eq!(Generation::of_conn_id(b), Generation::B);
            // The transport's base survives the bit.
            assert_eq!(b & !Generation::CONN_ID_BIT, id);
            assert!(
                i64::try_from(b).is_ok(),
                "an id stays positive when widened"
            );
        }
    }

    /// `<stem>.<gen>.sock` beside the configured `<stem>.sock`; a bare name
    /// takes the letter as its extension; an inner dot stays where it is.
    #[test]
    fn the_trusted_socket_is_named_for_its_generation_beside_the_configured_one() {
        let configured = Path::new("/run/rustmush-ssh/dev1.sock");
        assert_eq!(
            Generation::A.socket_path(configured),
            PathBuf::from("/run/rustmush-ssh/dev1.a.sock")
        );
        assert_eq!(
            Generation::B.socket_path(configured),
            PathBuf::from("/run/rustmush-ssh/dev1.b.sock")
        );
        assert_eq!(
            Generation::A.socket_path(Path::new("dev2")),
            PathBuf::from("dev2.a")
        );
        assert_eq!(
            Generation::B.socket_path(Path::new("relative/x.y.sock")),
            PathBuf::from("relative/x.y.b.sock")
        );
    }
}
