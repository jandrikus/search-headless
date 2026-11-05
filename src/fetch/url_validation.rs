use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::types::ToolError;

#[derive(Debug, Default, Clone, Copy)]
pub struct UrlValidationOptions {
    pub allow_private_urls: bool,
}

pub fn validate_public_http_url(input: &str, options: UrlValidationOptions) -> Result<Url, ToolError> {
    let url = Url::parse(input).map_err(|_| private_fail("INVALID_URL", "URL is not valid."))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(private_fail("INVALID_URL", "Only http and https URLs are supported."));
    }
    let Some(host) = url.host() else {
        return Err(private_fail("INVALID_URL", "URL is not valid."));
    };
    if !options.allow_private_urls && host_is_private(&host) {
        return Err(private_fail("PRIVATE_URL_BLOCKED", "Local and private network URLs are blocked by default."));
    }
    Ok(url)
}

/// Resolve the host and reject it if any address is private.
///
/// A public name that resolves to 169.254.169.254 is the whole point of the
/// check: the name alone tells you nothing.
pub async fn assert_public_dns(url: &Url, options: UrlValidationOptions) -> Option<ToolError> {
    if options.allow_private_urls {
        return None;
    }
    let host = url.host_str()?.to_string();
    let port = url.port_or_known_default().unwrap_or(80);

    // DNS failures are left to the fetch itself, which reports them properly.
    let addresses = tokio::net::lookup_host((host, port)).await.ok()?;
    for address in addresses {
        if is_private_ip(address.ip()) {
            return Some(ToolError::new(
                "PRIVATE_URL_BLOCKED",
                "URL resolves to a private network address.",
                false,
            ));
        }
    }
    None
}

fn private_fail(code: &str, message: &str) -> ToolError {
    ToolError::new(code, message, false)
}

fn host_is_private(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = name.to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        Host::Ipv4(addr) => is_private_ipv4(*addr),
        Host::Ipv6(addr) => is_private_ipv6(*addr),
    }
}

/// Whether a bare host string names a private target. Accepts names and IPs,
/// with or without the IPv6 brackets.
pub fn is_private_host(host: &str) -> bool {
    let normalized = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return true;
    }
    match normalized.parse::<IpAddr>() {
        Ok(ip) => is_private_ip(ip),
        Err(_) => false,
    }
}

pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => is_private_ipv4(addr),
        IpAddr::V6(addr) => is_private_ipv6(addr),
    }
}

fn is_private_ipv4(addr: Ipv4Addr) -> bool {
    let [a, b, ..] = addr.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
}

fn is_private_ipv6(addr: Ipv6Addr) -> bool {
    // `::ffff:10.0.0.1` and `::ffff:a00:1` are the same address; both have to be
    // judged as the IPv4 address they carry.
    if let Some(mapped) = addr.to_ipv4_mapped() {
        return is_private_ipv4(mapped);
    }
    if addr.is_loopback() || addr.is_unspecified() {
        return true;
    }
    let first = addr.segments()[0];
    // fc00::/7 unique local, fe80::/10 link local.
    (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public() -> UrlValidationOptions {
        UrlValidationOptions::default()
    }

    #[test]
    fn accepts_public_http_and_https_urls() {
        assert!(validate_public_http_url("https://example.com/a", public()).is_ok());
        assert!(validate_public_http_url("http://example.com/a", public()).is_ok());
    }

    #[test]
    fn rejects_non_http_urls() {
        let error = validate_public_http_url("file:///etc/passwd", public()).unwrap_err();
        assert_eq!(error.code, "INVALID_URL");
    }

    #[test]
    fn rejects_local_private_metadata_and_ipv6_local_hosts() {
        for url in [
            "http://localhost",
            "http://127.0.0.1",
            "http://0.0.0.0",
            "http://10.0.0.1",
            "http://100.64.0.1",
            "http://100.127.255.255",
            "http://172.16.0.1",
            "http://192.168.1.1",
            "http://169.254.169.254",
            "http://[::1]",
            "http://[fc00::1]",
            "http://[fd00::1]",
            "http://[fe80::1]",
            "http://[::ffff:127.0.0.1]",
            "http://[::ffff:10.0.0.1]",
        ] {
            let error = validate_public_http_url(url, public())
                .err()
                .unwrap_or_else(|| panic!("{url} should have been blocked"));
            assert_eq!(error.code, "PRIVATE_URL_BLOCKED", "{url}");
        }
    }

    #[test]
    fn can_allow_private_urls_only_through_an_internal_test_option() {
        let options = UrlValidationOptions { allow_private_urls: true };
        assert!(validate_public_http_url("http://127.0.0.1", options).is_ok());
    }

    #[test]
    fn is_private_host_covers_bare_names_and_addresses() {
        assert!(is_private_host("localhost"));
        assert!(is_private_host("api.localhost"));
        assert!(is_private_host("[::ffff:192.168.0.1]"));
        assert!(is_private_host("172.31.255.255"));
        assert!(!is_private_host("172.32.0.1"));
        assert!(!is_private_host("example.com"));
        assert!(!is_private_host("8.8.8.8"));
    }
}
