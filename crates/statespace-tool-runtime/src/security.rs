//! SSRF protection for the curl tool.
//!
//! Validates URLs and blocks requests to private/internal networks.

use crate::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const NETWORK_COMMANDS: &[&str] = &["curl", "wget"];

/// Flags that can redirect curl/wget to arbitrary destinations regardless
/// of the URL argument. Block these to prevent SSRF via flag injection.
const DANGEROUS_FLAGS: &[&str] = &[
    "--connect-to",
    "--resolve",
    "--unix-socket",
    "--abstract-unix-socket",
    "-K",
    "--config",
    "--proxy",
    "-x",
    "--socks4",
    "--socks4a",
    "--socks5",
    "--socks5-hostname",
    "--dns-servers",
    "--doh-url",
    "--interface",
];

/// Validate arguments for network-capable commands against SSRF rules.
///
/// Only checks `curl` and `wget`. Blocks dangerous flags that can redirect
/// requests, and validates URL arguments against private/internal networks.
/// Fails closed — rejects anything it cannot confidently determine is safe.
/// Not exhaustive — defense in depth, not a complete sandbox.
///
/// # Errors
///
/// Returns `Error::Security` when an argument targets a restricted address
/// or uses a dangerous flag.
pub fn validate_network_args(command: &str, args: &[String]) -> Result<(), Error> {
    if !NETWORK_COMMANDS.contains(&command) {
        return Ok(());
    }

    for arg in args {
        let lower = arg.to_lowercase();
        if DANGEROUS_FLAGS.iter().any(|f| lower == *f || lower.starts_with(&format!("{f}="))) {
            return Err(Error::Security(
                "Restricted flag not allowed".to_string(),
            ));
        }
    }

    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg.starts_with('-') {
            if matches!(
                arg.as_str(),
                "-H" | "--header"
                    | "-A" | "--user-agent"
                    | "-e" | "--referer"
                    | "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode"
                    | "-u" | "--user"
                    | "-b" | "--cookie"
                    | "-c" | "--cookie-jar"
                    | "-X" | "--request"
                    | "-m" | "--max-time"
                    | "--connect-timeout"
                    | "-w" | "--write-out"
                    | "-r" | "--range"
                    | "--retry"
                    | "--retry-delay"
                    | "--retry-max-time"
            ) && !arg.contains('=')
            {
                skip_next = true;
            }
            continue;
        }

        let candidate = if arg.contains("://") {
            arg.clone()
        } else {
            format!("http://{arg}")
        };

        let Ok(url) = reqwest::Url::parse(&candidate) else {
            return Err(Error::Security(
                "Unrecognized URL argument rejected".to_string(),
            ));
        };

        match url.scheme() {
            "http" | "https" => {}
            _ => {
                return Err(Error::Security(
                    "Only http/https schemes allowed".to_string(),
                ));
            }
        }

        let Some(host) = url.host_str() else {
            return Err(Error::Security(
                "URL must have a host".to_string(),
            ));
        };

        if is_localhost_name(host) || is_metadata_service(host) {
            return Err(Error::Security(
                "Access to restricted network resource blocked".to_string(),
            ));
        }

        // Strip brackets for IPv6 (host_str returns "[::1]" for IPv6)
        let bare_host = host.strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);

        if let Ok(ip) = bare_host.parse::<IpAddr>() {
            if is_private_or_restricted_ip(&ip) {
                return Err(Error::Security(
                    "Access to restricted network resource blocked".to_string(),
                ));
            }
        }
    }

    Ok(())
}

/// # Errors
///
/// Returns errors for invalid URLs or restricted destinations.
pub fn validate_url_initial(url: &str) -> Result<reqwest::Url, Error> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| Error::InvalidCommand(format!("Invalid URL: {e}")))?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(Error::Security(format!(
            "Only http/https schemes allowed, got: {}",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| Error::InvalidCommand("URL must have a host".into()))?;

    if is_localhost_name(host) {
        return Err(Error::Security(format!(
            "Access to localhost is not allowed: {host}"
        )));
    }

    if is_metadata_service(host) {
        return Err(Error::Security(format!(
            "Access to metadata service blocked: {host}"
        )));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_or_restricted_ip(&ip) {
            return Err(Error::Security(format!(
                "Access to private/restricted IP blocked: {ip}"
            )));
        }
    }

    Ok(parsed)
}

fn is_localhost_name(host: &str) -> bool {
    matches!(
        host.to_lowercase().as_str(),
        "localhost" | "localhost.localdomain"
    )
}

fn is_metadata_service(host: &str) -> bool {
    host == "169.254.169.254" || host == "metadata.google.internal"
}

#[must_use]
pub fn is_private_or_restricted_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_private_ipv4(*ipv4),
        IpAddr::V6(ipv6) => is_private_ipv6(ipv6),
    }
}

const fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
}

fn is_private_ipv6(ip: &Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        || is_ipv6_site_local(ip)
        || is_ipv4_mapped_private(ip)
}

fn is_ipv6_site_local(ip: &Ipv6Addr) -> bool {
    let s0 = ip.segments()[0];
    (0xfec0..=0xfeff).contains(&s0)
}

const fn is_ipv4_mapped_private(ip: &Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        is_private_ipv4(mapped)
    } else {
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // -- validate_network_args --

    #[test]
    fn test_network_args_ignores_non_network_commands() {
        assert!(validate_network_args("ls", &["-la".into()]).is_ok());
        assert!(validate_network_args("cat", &["http://169.254.169.254".into()]).is_ok());
    }

    #[test]
    fn test_network_args_blocks_metadata_service() {
        let result =
            validate_network_args("curl", &["http://169.254.169.254/latest/meta-data".into()]);
        assert!(matches!(result, Err(Error::Security(_))));

        let result = validate_network_args("curl", &["169.254.169.254".into()]);
        assert!(matches!(result, Err(Error::Security(_))));

        let result =
            validate_network_args("wget", &["http://169.254.169.254/latest/meta-data".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_localhost() {
        let result = validate_network_args("curl", &["http://localhost:8080".into()]);
        assert!(matches!(result, Err(Error::Security(_))));

        let result = validate_network_args("curl", &["http://127.0.0.1".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_private_ips() {
        let result = validate_network_args("curl", &["http://10.0.0.1/internal".into()]);
        assert!(matches!(result, Err(Error::Security(_))));

        let result = validate_network_args("curl", &["http://192.168.1.1".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_ipv6_loopback() {
        let result = validate_network_args("curl", &["http://[::1]:8080".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_ipv4_mapped_ipv6() {
        let result =
            validate_network_args("curl", &["http://[::ffff:169.254.169.254]".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_file_scheme() {
        let result = validate_network_args("curl", &["file:///etc/passwd".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_blocks_dangerous_flags() {
        let result = validate_network_args(
            "curl",
            &["--unix-socket".into(), "/var/run/docker.sock".into(), "http://example.com".into()],
        );
        assert!(matches!(result, Err(Error::Security(_))));

        let result = validate_network_args(
            "curl",
            &["--resolve".into(), "example.com:80:127.0.0.1".into(), "http://example.com".into()],
        );
        assert!(matches!(result, Err(Error::Security(_))));

        let result = validate_network_args(
            "curl",
            &["--connect-to=::localhost:".into(), "http://example.com".into()],
        );
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_rejects_unparseable_urls() {
        let result = validate_network_args("curl", &[":::not-a-url".into()]);
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_network_args_allows_public_urls() {
        assert!(
            validate_network_args("curl", &["https://api.github.com/repos".into()]).is_ok()
        );
        assert!(
            validate_network_args("curl", &["-s".into(), "https://example.com".into()]).is_ok()
        );
    }

    #[test]
    fn test_network_args_skips_safe_flags() {
        assert!(validate_network_args(
            "curl",
            &["-s".into(), "-H".into(), "Authorization: Bearer tok".into(), "https://example.com".into()]
        )
        .is_ok());
    }

    // -- validate_url_initial --

    #[test]
    fn test_validate_url_allows_https() {
        assert!(validate_url_initial("https://example.com").is_ok());
        assert!(validate_url_initial("https://api.github.com/repos").is_ok());
    }

    #[test]
    fn test_validate_url_allows_http() {
        assert!(validate_url_initial("http://example.com").is_ok());
    }

    #[test]
    fn test_validate_url_blocks_ftp() {
        let result = validate_url_initial("ftp://example.com");
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_validate_url_blocks_file() {
        let result = validate_url_initial("file:///etc/passwd");
        assert!(matches!(result, Err(Error::Security(_))));
    }

    #[test]
    fn test_validate_url_blocks_localhost() {
        assert!(matches!(
            validate_url_initial("http://localhost"),
            Err(Error::Security(_))
        ));
        assert!(matches!(
            validate_url_initial("https://localhost:8080"),
            Err(Error::Security(_))
        ));
    }

    #[test]
    fn test_validate_url_blocks_metadata_service() {
        assert!(matches!(
            validate_url_initial("http://169.254.169.254"),
            Err(Error::Security(_))
        ));
        assert!(matches!(
            validate_url_initial("http://metadata.google.internal"),
            Err(Error::Security(_))
        ));
    }

    #[test]
    fn test_ipv4_blocks_private() {
        assert!(is_private_ipv4("10.0.0.1".parse().unwrap()));
        assert!(is_private_ipv4("172.16.0.1".parse().unwrap()));
        assert!(is_private_ipv4("192.168.1.1".parse().unwrap()));
        assert!(is_private_ipv4("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_ipv4_allows_public() {
        assert!(!is_private_ipv4("1.1.1.1".parse().unwrap()));
        assert!(!is_private_ipv4("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_blocks_loopback() {
        assert!(is_private_ipv6(&"::1".parse().unwrap()));
    }

    #[test]
    fn test_ipv6_blocks_unique_local() {
        assert!(is_private_ipv6(&"fc00::1".parse().unwrap()));
        assert!(is_private_ipv6(&"fd00::1".parse().unwrap()));
    }
}
