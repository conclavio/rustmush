//! Client-address resolution behind an optional reverse proxy.

use std::net::IpAddr;

use axum::http::HeaderMap;

/// Resolves the real client address for ban checks and auth-failure
/// accounting.
///
/// `X-Forwarded-For` is honored **only** when the direct peer is listed in
/// `trusted_proxies` (design 02 §6.5): scanning the header right-to-left
/// (most recently appended first), the first address not itself a trusted
/// proxy is the client. An unparseable entry stops the scan — everything
/// left of it is attacker-controllable text. With no trusted proxies (the
/// default), the peer address is always the answer.
///
/// Note the rate-limit layer keys differently (tower_governor's
/// `SmartIpKeyExtractor`, only installed when `trusted_proxies` is set);
/// this function is the authority for IpGuard purposes.
pub(crate) fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted_proxies: &[IpAddr]) -> IpAddr {
    if trusted_proxies.is_empty() || !trusted_proxies.contains(&peer) {
        return peer;
    }
    let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return peer;
    };
    for part in xff.split(',').rev() {
        match part.trim().parse::<IpAddr>() {
            Ok(ip) if !trusted_proxies.contains(&ip) => return ip,
            Ok(_) => continue, // a trusted hop; keep walking left
            Err(_) => break,   // garbage — do not trust anything further left
        }
    }
    peer
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
    fn garbage_stops_the_scan() {
        let h = xff("evil, 10.0.0.2");
        let trusted = [ip("127.0.0.1"), ip("10.0.0.2")];
        // everything left of the garbage is untrustworthy; fall back to peer
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("127.0.0.1"));
    }

    #[test]
    fn missing_header_falls_back_to_peer() {
        let h = HeaderMap::new();
        let trusted = [ip("127.0.0.1")];
        assert_eq!(client_ip(ip("127.0.0.1"), &h, &trusted), ip("127.0.0.1"));
    }
}
