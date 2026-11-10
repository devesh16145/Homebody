//! Destination names: what is acceptable, and how a name matches a rule.
//!
//! This module is the heart of the "bypass corpus". Every sandbox bypass published in 2025–2026
//! that we know of was a parsing or matching mistake in exactly this kind of code (NUL bytes in
//! hostnames, empty lists treated as allow-all, IP literals slipping past name rules). The rules
//! here are deliberately strict and every one has a test below.

use std::net::IpAddr;

/// Normalise a destination name, or reject it.
///
/// Accepts only ordinary DNS names: ASCII letters, digits, hyphen, underscore and dots; at least
/// two labels; no empty labels; no label starting or ending with a hyphen; total length ≤ 253.
/// The last label must not start with a digit, which rejects every spelling of an IPv4 address
/// (dotted, single integer, hex such as `0x7f000001`, octal). IPv6 literals fail the character
/// rule. A rule list is by name: raw addresses would bypass it, so they are never acceptable.
pub fn canonical(host: &str) -> Option<String> {
    let mut h = host.to_ascii_lowercase();
    if h.ends_with('.') {
        h.pop();
    }
    if h.is_empty() || h.len() > 253 {
        return None;
    }
    if !h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.') {
        return None;
    }
    let labels: Vec<&str> = h.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    for l in &labels {
        if l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-') {
            return None;
        }
    }
    if labels.last().map(|l| l.as_bytes()[0].is_ascii_digit()).unwrap_or(true) {
        return None;
    }
    Some(h)
}

/// Is this a rule pattern we accept: an exact name, or `*.` followed by a name.
pub fn valid_pattern(p: &str) -> bool {
    let p = p.trim().to_ascii_lowercase();
    match p.strip_prefix("*.") {
        Some(rest) => canonical(rest).is_some(),
        None => !p.contains('*') && canonical(&p).is_some(),
    }
}

/// Normalise a pattern for storage. Call only after [`valid_pattern`].
pub fn normalize_pattern(p: &str) -> String {
    let p = p.trim().to_ascii_lowercase();
    p.strip_suffix('.').map(str::to_string).unwrap_or(p)
}

/// Does the (already canonical) host match the pattern?
/// `example.com` matches only itself. `*.example.com` matches any name below `example.com`, at
/// any depth, but not `example.com` itself and not `evilexample.com`.
pub fn matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(suffix) && host.as_bytes()[host.len() - suffix.len() - 1] == b'.',
        None => pattern == host,
    }
}

/// Is this address on the public internet? A trusted *name* must never become a tunnel to this
/// computer or the local network, so names that resolve to anything else are refused.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64)   // 100.64.0.0/10 carrier-grade NAT
                || (o[0] == 198 && (o[1] & 0xfe) == 18)   // 198.18.0.0/15 benchmarking
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00   // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80   // link local fe80::/10
                || (s[0] == 0x2001 && s[1] == 0x0db8)   // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b && is_nat64_private(&s)))
        }
    }
}

fn is_nat64_private(s: &[u16; 8]) -> bool {
    // 64:ff9b::/96 embeds an IPv4 address in the last 32 bits.
    let v4 = std::net::Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
    !is_public(IpAddr::V4(v4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        assert_eq!(canonical("API.Anthropic.com").as_deref(), Some("api.anthropic.com"));
        assert_eq!(canonical("example.com.").as_deref(), Some("example.com"));
        assert_eq!(canonical("xn--bcher-kva.example").as_deref(), Some("xn--bcher-kva.example"));
        assert_eq!(canonical("a_b.example.org").as_deref(), Some("a_b.example.org"));
        assert_eq!(canonical("dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com").as_deref(), Some("dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com"));
    }

    #[test]
    fn rejects_every_spelling_of_an_ip_address() {
        for bad in ["127.0.0.1", "93.184.216.34", "2130706433", "0x7f000001", "0177.0.0.1", "0x7f.0.0.1", "1.1", "::1", "[::1]", "fe80::1%en0", "::ffff:127.0.0.1", "example.0", "foo.123"] {
            assert_eq!(canonical(bad), None, "{bad} must be rejected");
        }
    }

    #[test]
    fn rejects_control_characters_and_separators() {
        for bad in ["evil.com\0.trusted.com", "trusted.com\0", "a\r\nb.com", "a b.com", "user@host.com", "host.com/path", "host.com?x", "host.com#x", "host.com:443", "exa%6dple.com", "ex\\ample.com", "éxample.com", "ｅxample.com", "\u{202e}moc.elpmaxe"] {
            assert_eq!(canonical(bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn rejects_degenerate_names() {
        for bad in ["", ".", "..", "localhost", "com", "a..b.com", ".example.com", "-a.example.com", "a-.example.com", "*.example.com", "*"] {
            assert_eq!(canonical(bad), None, "{bad:?} must be rejected");
        }
        assert_eq!(canonical(&format!("{}.com", "a".repeat(64))), None);
        assert_eq!(canonical(&format!("{}.com", "a.".repeat(130))), None);
    }

    #[test]
    fn exact_patterns_match_only_themselves() {
        assert!(matches("example.com", "example.com"));
        assert!(!matches("example.com", "www.example.com"));
        assert!(!matches("example.com", "example.com.evil.net"));
        assert!(!matches("example.com", "notexample.com"));
    }

    #[test]
    fn wildcards_match_subdomains_only() {
        assert!(matches("*.example.com", "a.example.com"));
        assert!(matches("*.example.com", "a.b.example.com"));
        assert!(!matches("*.example.com", "example.com"));
        assert!(!matches("*.example.com", "evilexample.com"));
        assert!(!matches("*.example.com", "example.com.evil.net"));
        assert!(!matches("*.example.com", "a.example.com.evil.net"));
    }

    #[test]
    fn pattern_validation() {
        assert!(valid_pattern("example.com"));
        assert!(valid_pattern("*.example.com"));
        assert!(valid_pattern(" Example.COM "));
        for bad in ["*", "*.*", "*.com", "ex*mple.com", "*example.com", "a.*.com", "", "127.0.0.1", "*.0.0.1", "http://example.com", "example.com/path"] {
            assert!(!valid_pattern(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn public_address_check() {
        for private in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0", "100.64.0.1", "224.0.0.1", "255.255.255.255", "198.18.0.1", "240.0.0.1", "::1", "::", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "64:ff9b::7f00:1"] {
            assert!(!is_public(private.parse().unwrap()), "{private} must not be public");
        }
        for public in ["1.1.1.1", "93.184.216.34", "2606:4700::1111", "2a04:4e42::644", "::ffff:1.1.1.1", "64:ff9b::101:101"] {
            assert!(is_public(public.parse().unwrap()), "{public} must be public");
        }
    }
}
