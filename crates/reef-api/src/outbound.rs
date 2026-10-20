//! Guard for URLs the server connects to on a user's behalf (webhooks, mirrors, imports).
//! Rejects loopback, link-local (including cloud metadata), private, multicast and
//! unspecified addresses unless `[network] allow_private_targets` is on, resolving host
//! names so DNS names for internal addresses are caught too.

use crate::AppState;
use std::net::IpAddr;

/// Why a target was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Host part of an http(s)/ssh/git URL or an scp-style `user@host:path` remote.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some((_, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let hostport = authority.rsplit('@').next().unwrap_or(authority);
        let host = if let Some(h) = hostport.strip_prefix('[') {
            h.split(']').next().unwrap_or("")
        } else {
            hostport.split(':').next().unwrap_or("")
        };
        return Some(host.to_ascii_lowercase()).filter(|h| !h.is_empty());
    }
    // scp-style: git@host:owner/repo.git
    let (left, _) = url.split_once(':')?;
    let host = left.rsplit('@').next().unwrap_or(left);
    Some(host.to_ascii_lowercase()).filter(|h| !h.is_empty())
}

pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT 100.64/10
                || (o[0] == 0)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
                || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64 well-known prefix
        }
    }
}

fn host_is_local_name(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
        || !host.contains('.') && host.parse::<IpAddr>().is_err()
}

/// Resolve `host` and check every address. Errors name the first refused address.
pub async fn check_host(s: &AppState, host: &str) -> Result<(), Refused> {
    if s.config.network.allow_private_targets {
        return Ok(());
    }
    if host_is_local_name(host) {
        return Err(Refused(format!("{host} is not a public host name")));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_private(ip) { Err(Refused(format!("{ip} is a private or local address"))) } else { Ok(()) };
    }
    let addrs = tokio::net::lookup_host((host, 443)).await.map_err(|e| Refused(format!("{host} does not resolve: {e}")))?;
    let mut any = false;
    for a in addrs {
        any = true;
        if is_private(a.ip()) {
            return Err(Refused(format!("{host} resolves to {}, a private or local address", a.ip())));
        }
    }
    if any { Ok(()) } else { Err(Refused(format!("{host} does not resolve"))) }
}

/// Check the host of a URL or remote. Local paths and `file://` are the caller's business.
pub async fn check_url(s: &AppState, url: &str) -> Result<(), Refused> {
    match host_of(url) {
        Some(h) => check_host(s, &h).await,
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://user:pw@example.com:8443/x?y").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[::1]:80/hook").as_deref(), Some("::1"));
        assert_eq!(host_of("git@github.com:owner/repo.git").as_deref(), Some("github.com"));
        assert_eq!(host_of("ssh://git@host.example/owner/repo").as_deref(), Some("host.example"));
        assert_eq!(host_of("/srv/repo.git"), None);
    }

    #[test]
    fn ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:10.0.0.1",
            "224.0.0.1",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "140.82.112.3", "2606:4700::1111"] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }
}
