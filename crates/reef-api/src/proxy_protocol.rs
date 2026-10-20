//! The PROXY protocol (v1 text, v2 binary) on the HTTP(S) TCP listeners
//! (`[http] proxy_protocol`).
//!
//! When enabled, each new connection is looked at before anything else reads it:
//! - a trusted peer (`[http] trusted_proxies`) that opens with a PROXY header has it consumed,
//!   and the source address it carries becomes the connection's peer (a `LOCAL` command or an
//!   address family other than TCP/UDP over IPv4/IPv6 keeps the socket peer);
//! - a trusted peer without a header is served as it is (a load balancer's health probes connect
//!   from loopback with none);
//! - an untrusted peer that sends a header is dropped (it would be claiming someone else's
//!   address); an untrusted peer without one is served as usual.
//!
//! Detection peeks at the first bytes without consuming them. A peer that sends nothing for
//! `DETECT_WAIT` (a client waiting for the server to speak first, a browser's speculative
//! connection) is taken to have no header.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const V1: &[u8] = b"PROXY ";
const V2: &[u8] = b"\r\n\r\n\0\r\nQUIT\n";
/// The longest v1 header, CRLF included.
const V1_MAX: usize = 107;
/// How long a silent connection is given to start with a header.
pub const DETECT_WAIT: Duration = Duration::from_millis(1500);

/// Who may send a PROXY header: `Some` only when `[http] proxy_protocol = true`.
#[derive(Debug, Clone)]
pub struct ProxyPolicy {
    trusted: Vec<ipnet::IpNet>,
}

impl ProxyPolicy {
    pub fn from_config(cfg: &crate::Config) -> Option<ProxyPolicy> {
        cfg.http.proxy_protocol.then(|| ProxyPolicy { trusted: cfg.http.trusted_proxy_nets().unwrap_or_default() })
    }

    pub fn new(trusted: Vec<ipnet::IpNet>) -> ProxyPolicy {
        ProxyPolicy { trusted }
    }

    fn trusts(&self, ip: IpAddr) -> bool {
        let ip = crate::ratelimit::canonical_ip(ip);
        self.trusted.iter().any(|n| n.contains(&ip))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Opening {
    V1,
    V2,
    Other,
}

/// Look at the start of a new connection from `peer` under `policy`: `Ok(Some(addr))` to
/// serve it as coming from `addr` (with any header consumed), `Ok(None)` to drop it.
pub async fn accept(stream: &mut TcpStream, peer: SocketAddr, policy: &ProxyPolicy) -> io::Result<Option<SocketAddr>> {
    let opening = detect(stream).await?;
    if opening == Opening::Other {
        return Ok(Some(peer));
    }
    if !policy.trusts(peer.ip()) {
        tracing::info!(peer = %peer, "dropping a PROXY protocol header from an untrusted peer");
        return Ok(None);
    }
    let read = async {
        match opening {
            Opening::V1 => read_v1(stream).await,
            _ => read_v2(stream).await,
        }
    };
    match tokio::time::timeout(DETECT_WAIT * 4, read).await {
        Ok(Ok(Some(addr))) => Ok(Some(SocketAddr::new(crate::ratelimit::canonical_ip(addr.ip()), addr.port()))),
        Ok(Ok(None)) => Ok(Some(peer)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "PROXY header incomplete")),
    }
}

/// Peek until the opening bytes show a v1 or v2 signature, or rule both out.
async fn detect(stream: &TcpStream) -> io::Result<Opening> {
    let deadline = tokio::time::Instant::now() + DETECT_WAIT;
    let mut buf = [0u8; 12];
    loop {
        let n = match tokio::time::timeout_at(deadline, stream.peek(&mut buf)).await {
            Err(_) => return Ok(Opening::Other), // silent: no header
            Ok(Err(e)) => return Err(e),
            Ok(Ok(0)) => return Ok(Opening::Other),
            Ok(Ok(n)) => n,
        };
        let seen = &buf[..n];
        if seen.starts_with(V1) {
            return Ok(Opening::V1);
        }
        if seen.starts_with(V2) {
            return Ok(Opening::V2);
        }
        let may_be = |sig: &[u8]| sig.starts_with(&seen[..n.min(sig.len())]);
        if !may_be(V1) && !may_be(V2) {
            return Ok(Opening::Other);
        }
        // A prefix of a signature so far: wait for more bytes.
        if tokio::time::Instant::now() >= deadline {
            return Ok(Opening::Other);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("PROXY header: {msg}"))
}

/// `PROXY TCP4|TCP6 <src> <dst> <sport> <dport>\r\n` or `PROXY UNKNOWN ...\r\n`.
async fn read_v1(stream: &mut TcpStream) -> io::Result<Option<SocketAddr>> {
    let mut line = Vec::with_capacity(V1_MAX);
    loop {
        let b = stream.read_u8().await?;
        line.push(b);
        if line.ends_with(b"\r\n") {
            break;
        }
        if line.len() >= V1_MAX {
            return Err(bad("v1 line too long"));
        }
    }
    parse_v1(&line)
}

fn parse_v1(line: &[u8]) -> io::Result<Option<SocketAddr>> {
    let text = std::str::from_utf8(line).map_err(|_| bad("v1 is not text"))?.trim_end_matches("\r\n");
    let parts: Vec<&str> = text.split(' ').collect();
    match parts.as_slice() {
        ["PROXY", "UNKNOWN", ..] => Ok(None),
        ["PROXY", fam @ ("TCP4" | "TCP6"), src, _dst, sport, _dport] => {
            let ip: IpAddr = src.parse().map_err(|_| bad("v1 source address"))?;
            if (*fam == "TCP4") != ip.is_ipv4() {
                return Err(bad("v1 family does not match the address"));
            }
            let port: u16 = sport.parse().map_err(|_| bad("v1 source port"))?;
            Ok(Some(SocketAddr::new(ip, port)))
        }
        _ => Err(bad("v1 malformed")),
    }
}

/// 12-byte signature, version/command, family/protocol, length, addresses (and TLVs).
async fn read_v2(stream: &mut TcpStream) -> io::Result<Option<SocketAddr>> {
    let mut head = [0u8; 16];
    stream.read_exact(&mut head).await?;
    let len = u16::from_be_bytes([head[14], head[15]]) as usize;
    let mut rest = vec![0u8; len];
    stream.read_exact(&mut rest).await?;
    parse_v2(&head, &rest)
}

fn parse_v2(head: &[u8; 16], rest: &[u8]) -> io::Result<Option<SocketAddr>> {
    if head[12] >> 4 != 2 {
        return Err(bad("v2 version"));
    }
    match head[12] & 0x0f {
        0 => return Ok(None), // LOCAL: the proxy's own connection (a health check)
        1 => {}
        _ => return Err(bad("v2 command")),
    }
    match head[13] >> 4 {
        1 if rest.len() >= 12 => {
            let ip = Ipv4Addr::new(rest[0], rest[1], rest[2], rest[3]);
            Ok(Some(SocketAddr::new(IpAddr::V4(ip), u16::from_be_bytes([rest[8], rest[9]]))))
        }
        2 if rest.len() >= 36 => {
            let mut a = [0u8; 16];
            a.copy_from_slice(&rest[..16]);
            Ok(Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(a)), u16::from_be_bytes([rest[32], rest[33]]))))
        }
        1 | 2 => Err(bad("v2 address block too short")),
        _ => Ok(None), // UNSPEC or AF_UNIX: keep the socket peer
    }
}

/// A v2 `PROXY` header for `src` → `dst` over TCP (for tests and tools).
pub fn v2_header(src: SocketAddr, dst: SocketAddr) -> Vec<u8> {
    let mut h = V2.to_vec();
    h.push(0x21);
    match (src, dst) {
        (SocketAddr::V4(s), SocketAddr::V4(d)) => {
            h.push(0x11);
            h.extend_from_slice(&12u16.to_be_bytes());
            h.extend_from_slice(&s.ip().octets());
            h.extend_from_slice(&d.ip().octets());
            h.extend_from_slice(&s.port().to_be_bytes());
            h.extend_from_slice(&d.port().to_be_bytes());
        }
        (s, d) => {
            let v6 = |a: SocketAddr| match a.ip() {
                IpAddr::V4(v4) => v4.to_ipv6_mapped(),
                IpAddr::V6(v6) => v6,
            };
            h.push(0x21);
            h.extend_from_slice(&36u16.to_be_bytes());
            h.extend_from_slice(&v6(s).octets());
            h.extend_from_slice(&v6(d).octets());
            h.extend_from_slice(&s.port().to_be_bytes());
            h.extend_from_slice(&d.port().to_be_bytes());
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// Connect to a fresh loopback listener, send `first`, and run `accept` on the server side;
    /// returns its outcome and what the server reads next.
    async fn run(policy: ProxyPolicy, first: &[u8]) -> (Option<SocketAddr>, Vec<u8>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let first = first.to_vec();
        let client = tokio::spawn(async move {
            let mut c = TcpStream::connect(addr).await.unwrap();
            if !first.is_empty() {
                c.write_all(&first).await.unwrap();
            }
            c.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
            c
        });
        let (mut s, peer) = l.accept().await.unwrap();
        let out = accept(&mut s, peer, &policy).await.unwrap();
        let mut next = vec![0u8; 16];
        if out.is_some() {
            s.read_exact(&mut next).await.unwrap();
        }
        drop(client.await.unwrap());
        (out, next)
    }

    fn loopback_trusted() -> ProxyPolicy {
        ProxyPolicy::new(vec!["127.0.0.1/32".parse().unwrap()])
    }

    #[tokio::test]
    async fn trusted_peers_may_send_a_header_or_none() {
        let src: SocketAddr = "198.51.100.7:40123".parse().unwrap();
        let dst: SocketAddr = "192.0.2.1:443".parse().unwrap();
        let (out, next) = run(loopback_trusted(), &v2_header(src, dst)).await;
        assert_eq!(out, Some(src), "v2: the carried source becomes the peer");
        assert_eq!(&next, b"GET / HTTP/1.1\r\n", "the header was consumed, nothing more");
        let (out, next) = run(loopback_trusted(), b"PROXY TCP4 198.51.100.8 192.0.2.1 5555 443\r\n").await;
        assert_eq!(out, Some("198.51.100.8:5555".parse().unwrap()), "v1");
        assert_eq!(&next, b"GET / HTTP/1.1\r\n");
        let (out, _) = run(loopback_trusted(), &v2_header("[2001:db8::7]:1".parse().unwrap(), "[2001:db8::1]:443".parse().unwrap())).await;
        assert_eq!(out, Some("[2001:db8::7]:1".parse().unwrap()), "v2 over IPv6");
        // An IPv4 client carried as an IPv4-mapped IPv6 address is the IPv4 address.
        let (out, _) =
            run(loopback_trusted(), &v2_header("[::ffff:198.51.100.9]:7".parse().unwrap(), "[2001:db8::1]:443".parse().unwrap())).await;
        assert_eq!(out, Some("198.51.100.9:7".parse().unwrap()));
        // No header from a trusted peer (a health probe): served as it is.
        let (out, next) = run(loopback_trusted(), b"").await;
        assert_eq!(out.map(|a| a.ip()), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(&next, b"GET / HTTP/1.1\r\n", "nothing was consumed");
    }

    #[tokio::test]
    async fn untrusted_peers_are_served_without_a_header_and_dropped_with_one() {
        let untrusted = ProxyPolicy::new(vec!["10.0.0.0/8".parse().unwrap()]);
        let (out, next) = run(untrusted.clone(), b"").await;
        assert_eq!(out.map(|a| a.ip()), Some("127.0.0.1".parse().unwrap()), "no header: served as usual");
        assert_eq!(&next, b"GET / HTTP/1.1\r\n");
        let (out, _) = run(untrusted.clone(), &v2_header("198.51.100.7:1".parse().unwrap(), "192.0.2.1:443".parse().unwrap())).await;
        assert_eq!(out, None, "a header from an untrusted peer: dropped");
        let (out, _) = run(untrusted, b"PROXY TCP4 198.51.100.8 192.0.2.1 5555 443\r\n").await;
        assert_eq!(out, None);
    }

    #[test]
    fn parses_the_edge_cases() {
        assert_eq!(parse_v1(b"PROXY UNKNOWN\r\n").unwrap(), None);
        assert!(parse_v1(b"PROXY TCP4 2001:db8::1 192.0.2.1 1 2\r\n").is_err());
        assert!(parse_v1(b"PROXY TCP4 nonsense\r\n").is_err());
        let mut head = [0u8; 16];
        head[..12].copy_from_slice(V2);
        head[12] = 0x20; // LOCAL
        assert_eq!(parse_v2(&head, &[]).unwrap(), None);
        head[12] = 0x21;
        head[13] = 0x11;
        assert!(parse_v2(&head, &[0; 4]).is_err(), "short address block");
        head[13] = 0x31; // AF_UNIX
        assert_eq!(parse_v2(&head, &[0; 216]).unwrap(), None);
    }
}
