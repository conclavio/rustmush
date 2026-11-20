//! Client-address resolution behind an optional reverse proxy.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use axum::http::HeaderMap;

/// Canonicalize IPv4-mapped IPv6 addresses to IPv4 and fold native IPv6
/// addresses to their `/64` network prefix, matching `IpGuard`'s default
/// `/32` IPv4 and `/64` IPv6 network accounting (HIGH-3).
pub(crate) fn fold_network_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & (u128::MAX << 64))),
    }
}

/// Resolves the real client address for rate-limiting, ban checks, and
/// auth-failure accounting (W-5).
///
/// `X-Forwarded-For` is honored **only** when the canonicalized direct peer is
/// listed in `trusted_proxies` (design 02 §6.5). Every `X-Forwarded-For` header
/// occurrence is scanned in reverse (most recently appended header first), and
/// each header's raw bytes are split on `,` right-to-left with ASCII whitespace
/// trimmed:
///
/// - The first canonicalized IP not itself in `trusted_proxies` is returned.
/// - Any non-ASCII, empty, or unparseable entry stops scanning immediately
///   (everything left of it is attacker-controllable).
/// - When `X-Forwarded-For` was present and either malformed/non-ASCII or
///   composed exclusively of trusted hops, we never fall back to a loopback
///   proxy address (which would land in `IpGuardConfig::exempt_nets`): we
///   return `0.0.0.0` (`Ipv4Addr::UNSPECIFIED`) instead.
/// - With no `trusted_proxies` (the default) or no `X-Forwarded-For` header,
///   the canonicalized direct peer address is returned.
pub(crate) fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted_proxies: &[IpAddr]) -> IpAddr {
    let peer = peer.to_canonical();
    let is_trusted = |candidate: IpAddr| {
        trusted_proxies
            .iter()
            .any(|p| p.to_canonical() == candidate)
    };
    if trusted_proxies.is_empty() || !is_trusted(peer) {
        return peer;
    }

    let values: Vec<&axum::http::HeaderValue> = headers.get_all("x-forwarded-for").iter().collect();
    if values.is_empty() {
        return peer;
    }

    let mut malformed = false;
    'scan: for value in values.into_iter().rev() {
        for part in value.as_bytes().rsplit(|&b| b == b',') {
            let trimmed = part.trim_ascii();
            let parsed = std::str::from_utf8(trimmed)
                .ok()
                .and_then(|s| s.parse::<IpAddr>().ok());
            match parsed {
                Some(ip) => {
                    let canonical = ip.to_canonical();
                    if !is_trusted(canonical) {
                        return canonical;
                    }
                }
                None => {
                    malformed = true;
                    break 'scan;
                }
            }
        }
    }

    if malformed || peer.is_loopback() {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        peer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn xff(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn no_trusted_proxies_ignores_header() {
        let h = xff("203.0.113.9");
        assert_eq!(client_ip(ip("198.51.100.1"), &h, &[]), ip("198.51.100.1"));
    }

    #[test]
    fn untrusted_peer_ignores_header() {
        let h = xff("203.0.113.9");
        let trusted = [ip("127.0.0.1")];
        assert_eq!(
            client_ip(ip("198.51.100.1"), &h, &trusted),
            ip("198.51.100.1")
        );
    }

    #[test]
    fn trusted_peer_takes_rightmost_untrusted() {
        // client spoofed "1.2.3.4"; proxy appended the real client.
        let h = xff("1.2.3.4, 203.0.113.9");
        let trusted = [ip("127.0.0.1")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("203.0.113.9"));
    }

    #[test]
    fn skips_trusted_hops() {
        let h = xff("203.0.113.9, 10.0.0.2");
        let trusted = [ip("127.0.0.1"), ip("10.0.0.2")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("203.0.113.9"));
    }

    #[test]
    fn garbage_stops_the_scan_and_never_falls_back_to_loopback() {
        let h = xff("evil, 10.0.0.2");
        let trusted = [ip("127.0.0.1"), ip("10.0.0.2")];
        // Everything left of the garbage is untrustworthy, and we must never
        // fall back to loopback (which sits in IpGuard's exempt_nets).
        assert_eq!(
            client_ip(ip("127.0.0.1"), &h, &trusted),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
    }

    #[test]
    fn multiple_xff_headers_scan_rightmost_header_first() {
        // Client injected an earlier X-Forwarded-For header; reverse proxy
        // appended a second header with the real client address.
        let mut h = HeaderMap::new();
        h.append("x-forwarded-for", HeaderValue::from_static("198.51.100.7"));
        h.append("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        let trusted = [ip("127.0.0.1")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("203.0.113.9"));

        // Proxy-appended header contains only another trusted hop, so scan
        // continues leftward into the prior header.
        let mut h2 = HeaderMap::new();
        h2.append("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        h2.append("x-forwarded-for", HeaderValue::from_static("10.0.0.2"));
        let trusted2 = [ip("127.0.0.1"), ip("10.0.0.2")];
        assert_eq!(
            client_ip(ip("127.0.0.1"), &h2, &trusted2),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn non_ascii_left_of_valid_client_ip_still_resolves_client_ip() {
        // Attacker sent non-ASCII bytes in X-Forwarded-For; proxy appended
        // the real client IP to the right.
        let mut h = HeaderMap::new();
        let val = HeaderValue::from_bytes(b"\x80evil, 203.0.113.9").unwrap();
        h.insert("x-forwarded-for", val);
        let trusted = [ip("127.0.0.1")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("203.0.113.9"));

        // Non-ASCII at the rightmost untrusted position never falls back to loopback.
        let mut h2 = HeaderMap::new();
        let val2 = HeaderValue::from_bytes(b"\x80evil, 10.0.0.2").unwrap();
        h2.insert("x-forwarded-for", val2);
        let trusted2 = [ip("127.0.0.1"), ip("10.0.0.2")];
        assert_eq!(
            client_ip(ip("127.0.0.1"), &h2, &trusted2),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
    }

    #[test]
    fn canonicalizes_ipv4_mapped_peer_proxies_and_xff_hops() {
        let h = xff("1.2.3.4, ::ffff:203.0.113.9, ::ffff:10.0.0.2");
        let trusted = [ip("127.0.0.1"), ip("10.0.0.2")];
        assert_eq!(
            client_ip(ip("::ffff:127.0.0.1"), &h, &trusted),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn fold_network_key_folds_ipv6_to_slash_64_and_canonicalizes_v4_mapped() {
        assert_eq!(
            fold_network_key(ip("2001:db8:abcd:0012::1")),
            ip("2001:db8:abcd:0012::")
        );
        assert_eq!(
            fold_network_key(ip("2001:db8:abcd:0012:ffff:ffff:ffff:ffff")),
            ip("2001:db8:abcd:0012::")
        );
        assert_eq!(
            fold_network_key(ip("::ffff:203.0.113.9")),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn missing_header_falls_back_to_peer() {
        let h = HeaderMap::new();
        let trusted = [ip("127.0.0.1")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("127.0.0.1"));
    }
}
