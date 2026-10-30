//! The username grammar (doc 29 §3).
//!
//! The username splits at the **first** `+` only. A first segment naming a
//! service routes there with the remainder forwarded byte for byte; a bare
//! service name (or a remainder that is itself a front-door name) is the
//! instance's front door; anything else is the menu — or, when the owner sets
//! `[menu].default_service` (Q-29-3), that service with the whole username.

use mush_sshdoorway_proto::user_is_forwardable;

use crate::config::Config;

/// The instance front-door usernames (`mush-net-ssh`'s `is_front_door`).
pub const FRONT_DOOR: [&str; 3] = ["play", "guest", "new"];

/// Where a username goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// A configured service.
    Service {
        /// The service's configured name (canonical case).
        service: String,
        /// The username the doorway authenticates upstream as.
        upstream_user: String,
        /// The upstream name is a front door (`none` auth, welcome screen).
        front_door: bool,
    },
    /// The menu (doc 29 §7).
    Menu,
    /// A name the doorway will not forward (empty, control characters,
    /// over-long, an empty account part). Every method is refused.
    Invalid,
}

/// Whether `name` is an instance front-door name.
#[must_use]
pub fn is_front_door(name: &str) -> bool {
    FRONT_DOOR.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// Route one username against the configured services.
#[must_use]
pub fn route(user: &str, cfg: &Config) -> Route {
    if !user_is_forwardable(user) {
        return Route::Invalid;
    }
    let (first, rest) = match user.split_once('+') {
        Some((first, rest)) => (first, Some(rest)),
        None => (user, None),
    };
    let Some(service) = cfg.service(first) else {
        return match cfg
            .menu
            .default_service
            .as_deref()
            .and_then(|d| cfg.service(d))
        {
            Some(default) => forward(&default.name, user),
            None => Route::Menu,
        };
    };
    match rest {
        None | Some("") => Route::Service {
            service: service.name.clone(),
            upstream_user: "play".to_owned(),
            front_door: true,
        },
        Some(rest) => forward(&service.name, rest),
    }
}

fn forward(service: &str, upstream_user: &str) -> Route {
    let account = upstream_user
        .split_once('+')
        .map_or(upstream_user, |(a, _)| a);
    if account.is_empty() {
        return Route::Invalid;
    }
    let front_door = is_front_door(account);
    Route::Service {
        service: service.to_owned(),
        // A front door is `none` upstream as the bare front-door name: the
        // instance's `auth_none` matches the whole name, so `play+Kira` would
        // be refused (adversarial review F4).
        upstream_user: if front_door {
            account.to_owned()
        } else {
            upstream_user.to_owned()
        },
        front_door,
    }
}

/// The first `+` segment, escaped for a log line (doc 29 §8: the remainder is
/// never logged).
#[must_use]
pub fn log_segment(user: &str) -> String {
    let first = user.split_once('+').map_or(user, |(f, _)| f);
    let mut out: String = first
        .chars()
        .take(40)
        .flat_map(char::escape_debug)
        .collect();
    if first.chars().count() > 40 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(default: Option<&str>) -> Config {
        let mut text =
            String::from("[listen]\naddrs = [\"127.0.0.1:22\"]\nhost_key = \"k\"\n[menu]\n");
        if let Some(d) = default {
            text.push_str(&format!("default_service = \"{d}\"\n"));
        }
        text.push_str("[[service]]\nname = \"dev1\"\nsocket = \"s1\"\n[[service]]\nname = \"dev2\"\nsocket = \"s2\"\n");
        Config::parse(&text).unwrap()
    }

    fn svc(s: &str, up: &str, front: bool) -> Route {
        Route::Service {
            service: s.into(),
            upstream_user: up.into(),
            front_door: front,
        }
    }

    #[test]
    fn the_doc_29_table() {
        let c = cfg(None);
        assert_eq!(route("dev1", &c), svc("dev1", "play", true));
        assert_eq!(route("dev1+faenix", &c), svc("dev1", "faenix", false));
        assert_eq!(
            route("dev1+faenix+Kira", &c),
            svc("dev1", "faenix+Kira", false)
        );
        assert_eq!(route("dev1+play", &c), svc("dev1", "play", true));
        assert_eq!(route("dev1+Guest", &c), svc("dev1", "Guest", true));
        assert_eq!(route("dev1+play+Kira", &c), svc("dev1", "play", true));
        assert_eq!(route("DEV1+faenix", &c), svc("dev1", "faenix", false));
        assert_eq!(route("dev1+", &c), svc("dev1", "play", true));
        for menu in [
            "nick", "root", "ubuntu", "faenix", "nick+foo", "+dev1", "dev3+x",
        ] {
            assert_eq!(route(menu, &c), Route::Menu, "{menu}");
        }
    }

    #[test]
    fn hostile_names_are_invalid_not_forwarded() {
        let c = cfg(None);
        for bad in [
            "",
            "dev1++x",
            "dev1\n+x",
            "dev1+fa\u{1b}[2J",
            "dev1+a\0b",
            &"x".repeat(300),
            &format!("dev1+{}", "y".repeat(300)),
        ] {
            assert_eq!(route(bad, &c), Route::Invalid, "{bad:?}");
        }
    }

    #[test]
    fn a_default_service_takes_bare_names_as_accounts() {
        let c = cfg(Some("dev2"));
        assert_eq!(route("faenix", &c), svc("dev2", "faenix", false));
        assert_eq!(route("faenix+Kira", &c), svc("dev2", "faenix+Kira", false));
        assert_eq!(route("play", &c), svc("dev2", "play", true));
        assert_eq!(route("dev1+x", &c), svc("dev1", "x", false));
    }

    #[test]
    fn log_segments_never_carry_the_remainder_or_raw_controls() {
        assert_eq!(log_segment("dev1+faenix"), "dev1");
        assert_eq!(log_segment("a\nb"), "a\\nb");
        assert_eq!(log_segment(&"z".repeat(100)).chars().count(), 41);
    }
}
