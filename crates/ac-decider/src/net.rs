//! Outbound HTTP to user-configured URLs (decider endpoints, webhooks) with SSRF protection:
//! https only, no redirects, and the host must not resolve to private / loopback / link-local
//! addresses unless the server configuration explicitly allows it. The resolved addresses are
//! pinned for the request, so a DNS answer cannot change between the check and the connect.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

pub use url::Url;

#[derive(Debug, Clone, Copy, Default)]
pub struct EgressPolicy {
    /// Allow private / loopback / link-local destinations (self-hosted models).
    pub allow_private: bool,
    /// Allow plain http (tests, local development).
    pub allow_http: bool,
}

impl EgressPolicy {
    pub fn from_env() -> Self {
        let flag = |k: &str| {
            std::env::var(k).map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes")).unwrap_or(false)
        };
        EgressPolicy { allow_private: flag("EGRESS_ALLOW_PRIVATE"), allow_http: flag("EGRESS_ALLOW_HTTP") }
    }
}

fn v4_blocked(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (18..=19).contains(&o[1])) // benchmarking
        || o[0] >= 240
}

fn v6_blocked(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_blocked(v4);
    }
    let s = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00 // unique local
        || (s[0] & 0xffc0) == 0xfe80 // link local
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || (s[0] == 0x0064 && s[1] == 0xff9b) // NAT64 → could reach v4 private
}

pub fn ip_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_blocked(v4),
        IpAddr::V6(v6) => v6_blocked(v6),
    }
}

/// Validates a URL and resolves its host. Returns the parsed URL and the addresses to pin.
pub async fn check_url(raw: &str, policy: EgressPolicy) -> Result<(Url, Vec<SocketAddr>), String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("網址格式錯誤：{e}"))?;
    match url.scheme() {
        "https" => {}
        "http" if policy.allow_http => {}
        s => return Err(format!("只允許 https（目前是 {s}）")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("網址不能包含帳號密碼".into());
    }
    let host = url.host_str().ok_or("網址缺少主機")?.to_string();
    let port = url.port_or_known_default().ok_or("網址缺少連接埠")?;
    let host_for_lookup = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host_for_lookup.as_str(), port))
        .await
        .map_err(|e| format!("無法解析主機 {host}：{e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("無法解析主機 {host}"));
    }
    if !policy.allow_private
        && let Some(bad) = addrs.iter().find(|a| ip_blocked(a.ip()))
    {
        return Err(format!(
            "{host} 解析到私有或本機位址 {}，已拒絕（可由伺服器設定 EGRESS_ALLOW_PRIVATE 開放）",
            bad.ip()
        ));
    }
    Ok((url, addrs))
}

/// A client pinned to the checked addresses, without redirects.
pub fn pinned_client(url: &Url, addrs: &[SocketAddr], timeout: Duration) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(10)));
    if let Some(host) = url.host_str()
        && host.parse::<IpAddr>().is_err()
    {
        b = b.resolve_to_addrs(host, addrs);
    }
    b.build().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(ip_blocked(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "142.250.1.1", "2607:f8b0::1"] {
            assert!(!ip_blocked(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn rejects_bad_urls() {
        let p = EgressPolicy::default();
        assert!(check_url("http://example.com", p).await.is_err());
        assert!(check_url("https://127.0.0.1/x", p).await.is_err());
        assert!(check_url("https://localhost/x", p).await.is_err());
        assert!(check_url("https://user:pw@example.com", p).await.is_err());
        let open = EgressPolicy { allow_private: true, allow_http: true };
        assert!(check_url("http://127.0.0.1:9/x", open).await.is_ok());
    }
}
