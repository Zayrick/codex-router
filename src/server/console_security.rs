//! Request hardening for the web console: client address resolution behind
//! trusted reverse proxies, sign-in throttling, origin checks, and cookies.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::Mutex,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Extensions, HeaderMap, Request, StatusCode, uri::Authority},
};
use ipnet::IpNet;
use percent_encoding::percent_decode_str;
use url::Url;

use super::config::{AppConfig, ServerConfig, parse_trusted_proxy};

/// Cookie used when the console is published over HTTPS.
pub const SECURE_SESSION_COOKIE: &str = "__Host-codex-session";
/// Cookie used for plain HTTP deployments (development or private networks).
pub const PLAIN_SESSION_COOKIE: &str = "codex-session";
/// Cookie name of the stateless session issued before the unified console.
const LEGACY_ADMIN_COOKIE: &str = "__Host-codex-admin";

const LOGIN_FAILURE_WINDOW_MS: i64 = 10 * 60 * 1_000;
const MAX_LOGIN_FAILURES: u32 = 10;
const MAX_TRACKED_CLIENTS: usize = 10_000;
const MAX_FORWARDED_HOPS: usize = 16;

/// Resolves the client address. `X-Forwarded-For` is honored only when the TCP
/// peer is a configured trusted proxy; hops are read right to left, skipping
/// further trusted proxies, so a client cannot spoof its address by prepending.
pub fn client_ip(request: &Request<Body>, config: &AppConfig) -> Option<IpAddr> {
    client_address(request.extensions(), request.headers(), config)
}

/// [`client_ip`] for callers that already split the request into parts.
pub fn client_address(
    extensions: &Extensions,
    headers: &HeaderMap,
    config: &AppConfig,
) -> Option<IpAddr> {
    let peer = peer_ip(extensions)?;
    let trusted: Vec<_> = trusted_proxy_networks(&config.server).collect();
    Some(forwarded_client(peer, headers, &trusted))
}

fn peer_ip(extensions: &Extensions) -> Option<IpAddr> {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| canonical_ip(info.0.ip()))
}

fn trusted_proxy_networks(server: &ServerConfig) -> impl Iterator<Item = IpNet> + '_ {
    server
        .trusted_proxies
        .iter()
        .filter_map(|entry| parse_trusted_proxy(entry).ok())
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(first)
}

/// Apply the proxy's listener address to the configuration snapshot for this request.
pub fn apply_proxy_origin(
    request: &Request<Body>,
    server: &mut ServerConfig,
) -> Result<(), StatusCode> {
    if !server.public_origin_from_proxy {
        return Ok(());
    }
    let peer = peer_ip(request.extensions()).ok_or(StatusCode::FORBIDDEN)?;
    if !trusted_proxy_networks(server).any(|net| net.contains(&peer)) {
        return Err(StatusCode::FORBIDDEN);
    }
    let headers = request.headers();
    if !headers.contains_key("x-forwarded-host") && !headers.contains_key("x-forwarded-proto") {
        // Local callers on the trusted connection use the configured internal origin.
        return Ok(());
    }
    let scheme = single_header(headers, "x-forwarded-proto")
        .filter(|scheme| matches!(*scheme, "http" | "https"))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let authority = single_header(headers, "x-forwarded-host")
        .filter(|value| !value.contains(['@', ',', '%', '\\']))
        .and_then(|value| value.parse::<Authority>().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let url =
        Url::parse(&format!("{scheme}://{authority}")).map_err(|_| StatusCode::BAD_REQUEST)?;
    server.public_origin = url.origin().ascii_serialization();
    Ok(())
}

fn forwarded_client(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|network| network.contains(ip));
    if !is_trusted(&peer) {
        return peer;
    }
    let mut hops = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(value) = value.to_str() else {
            return peer;
        };
        hops.extend(value.split(',').map(str::trim));
    }
    if hops.len() > MAX_FORWARDED_HOPS {
        hops.drain(..hops.len() - MAX_FORWARDED_HOPS);
    }
    let mut client = peer;
    for hop in hops.iter().rev() {
        let Some(ip) = parse_forwarded_ip(hop) else {
            break;
        };
        client = ip;
        if !is_trusted(&ip) {
            break;
        }
    }
    client
}

fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(canonical_ip(ip));
    }
    value
        .parse::<SocketAddr>()
        .ok()
        .map(|address| canonical_ip(address.ip()))
}

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// Throttle bucket: IPv4 addresses individually, IPv6 by /64 (one end site).
fn throttle_key(ip: Option<IpAddr>) -> Option<IpAddr> {
    ip.map(|ip| match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            IpAddr::V6(Ipv6Addr::new(
                segments[0],
                segments[1],
                segments[2],
                segments[3],
                0,
                0,
                0,
                0,
            ))
        }
    })
}

#[derive(Debug, Clone, Copy)]
struct FailureWindow {
    started_at_ms: i64,
    failures: u32,
    /// Sign-ins that reserved budget and have not finished yet.
    pending: u32,
}

/// Counts failed sign-ins per client. Each attempt reserves budget before the
/// request body is read, so concurrent requests cannot all slip past the check.
/// A successful sign-in only releases its reservation: earlier failures from the
/// same client stay counted, so one valid low-privilege credential cannot reset
/// guessing of another. While a client is throttled every attempt is refused,
/// including correct credentials, so the endpoint cannot be used as an oracle.
#[derive(Default)]
pub struct LoginThrottle {
    clients: Mutex<HashMap<Option<IpAddr>, FailureWindow>>,
}

/// A reserved sign-in attempt. Dropping it without [`LoginAttempt::fail`]
/// releases the reservation without counting a failure.
pub struct LoginAttempt<'a> {
    throttle: &'a LoginThrottle,
    key: Option<IpAddr>,
}

impl LoginAttempt<'_> {
    pub fn fail(self, now_ms: i64) {
        {
            let mut clients = self.throttle.lock();
            if let Some(window) = clients.get_mut(&self.key) {
                if window_expired(window, now_ms) {
                    window.started_at_ms = now_ms;
                    window.failures = 0;
                }
                window.failures = window.failures.saturating_add(1);
            }
        }
        // Drop releases the reservation.
    }
}

impl Drop for LoginAttempt<'_> {
    fn drop(&mut self) {
        let mut clients = self.throttle.lock();
        if let Some(window) = clients.get_mut(&self.key) {
            window.pending = window.pending.saturating_sub(1);
        }
    }
}

impl LoginThrottle {
    /// Reserves one attempt, or returns the seconds to wait when throttled.
    pub fn begin(&self, ip: Option<IpAddr>, now_ms: i64) -> Result<LoginAttempt<'_>, u64> {
        let key = throttle_key(ip);
        let mut clients = self.lock();
        if clients.len() >= MAX_TRACKED_CLIENTS && !clients.contains_key(&key) {
            clients.retain(|_, window| window.pending > 0 || !window_expired(window, now_ms));
            if clients.len() >= MAX_TRACKED_CLIENTS {
                // Fail closed rather than silently stop tracking new clients.
                return Err(60);
            }
        }
        let window = clients.entry(key).or_insert(FailureWindow {
            started_at_ms: now_ms,
            failures: 0,
            pending: 0,
        });
        if window_expired(window, now_ms) {
            window.started_at_ms = now_ms;
            window.failures = 0;
        }
        if window.failures.saturating_add(window.pending) >= MAX_LOGIN_FAILURES {
            let remaining = window.started_at_ms + LOGIN_FAILURE_WINDOW_MS - now_ms;
            let seconds = if window.failures >= MAX_LOGIN_FAILURES {
                u64::try_from((remaining + 999) / 1_000).unwrap_or(1).max(1)
            } else {
                1
            };
            return Err(seconds);
        }
        window.pending += 1;
        Ok(LoginAttempt {
            throttle: self,
            key,
        })
    }

    pub fn purge(&self, now_ms: i64) {
        self.lock()
            .retain(|_, window| window.pending > 0 || !window_expired(window, now_ms));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Option<IpAddr>, FailureWindow>> {
        self.clients.lock().expect("login throttle lock poisoned")
    }
}

fn window_expired(window: &FailureWindow, now_ms: i64) -> bool {
    now_ms - window.started_at_ms >= LOGIN_FAILURE_WINDOW_MS
}

/// Requires a same-origin browser request for every console write. Cross-site
/// forms cannot set a JSON content type without a CORS preflight, which the
/// console never answers, and `Origin` must exactly match the public origin.
pub fn is_same_origin_write(headers: &HeaderMap, public_origin: &str) -> bool {
    single_header(headers, "origin") == Some(public_origin)
}

pub fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

pub fn uses_https(config: &AppConfig) -> bool {
    config.server.public_origin.starts_with("https://")
}

pub fn session_cookie_name(config: &AppConfig) -> &'static str {
    if uses_https(config) {
        SECURE_SESSION_COOKIE
    } else {
        PLAIN_SESSION_COOKIE
    }
}

/// Builds the session cookie. HTTPS deployments use the `__Host-` prefix
/// (Secure, Path=/, no Domain) so sibling subdomains cannot overwrite it.
pub fn session_cookie(config: &AppConfig, token: &str, max_age_seconds: i64) -> String {
    let max_age = max_age_seconds.max(0);
    if uses_https(config) {
        format!(
            "{SECURE_SESSION_COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}"
        )
    } else {
        format!(
            "{PLAIN_SESSION_COOKIE}={token}; Path=/admin; HttpOnly; SameSite=Lax; Max-Age={max_age}"
        )
    }
}

pub fn clear_session_cookies(config: &AppConfig) -> Vec<String> {
    let mut cookies = vec![session_cookie(config, "", 0)];
    if uses_https(config) {
        cookies.push(format!(
            "{LEGACY_ADMIN_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
        ));
    }
    cookies
}

/// Reads the console session token from all `Cookie` headers.
pub fn session_token(headers: &HeaderMap, config: &AppConfig) -> Option<String> {
    let name = session_cookie_name(config);
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .and_then(|(_, value)| percent_decode_str(value.trim()).decode_utf8().ok())
        .map(|value| value.into_owned())
        .filter(|value| !value.is_empty())
}

/// Removes this application's session cookies before a request is relayed
/// upstream, keeping every other cookie intact.
pub fn strip_console_cookies(headers: &mut HeaderMap) {
    let original: Vec<String> = headers
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    if original.is_empty() {
        return;
    }
    let kept: Vec<&str> = original
        .iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(name, _)| name).trim();
            !matches!(
                name,
                SECURE_SESSION_COOKIE | PLAIN_SESSION_COOKIE | LEGACY_ADMIN_COOKIE
            )
        })
        .collect();
    headers.remove("cookie");
    if !kept.is_empty()
        && let Ok(value) = kept.join("; ").parse()
    {
        headers.insert("cookie", value);
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn nets(values: &[&str]) -> Vec<IpNet> {
        values
            .iter()
            .map(|value| parse_trusted_proxy(value).unwrap())
            .collect()
    }

    fn forwarded(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn proxy_listener_sets_the_exact_browser_origin_on_each_port() {
        let server = ServerConfig {
            public_origin_from_proxy: true,
            trusted_proxies: vec!["127.0.0.1".into()],
            ..Default::default()
        };
        for (scheme, authority, expected) in [
            ("http", "192.168.161.10:8787", "http://192.168.161.10:8787"),
            ("http", "10.10.1.1:8318", "http://10.10.1.1:8318"),
            ("https", "[::1]:443", "https://[::1]"),
        ] {
            let mut request = Request::builder()
                .extension(ConnectInfo(
                    "127.0.0.1:54000".parse::<SocketAddr>().unwrap(),
                ))
                .header("x-forwarded-proto", scheme)
                .header("x-forwarded-host", authority)
                .header("origin", expected)
                .body(Body::empty())
                .unwrap();
            let mut snapshot = server.clone();
            apply_proxy_origin(&request, &mut snapshot).unwrap();
            assert_eq!(snapshot.public_origin, expected);
            assert!(is_same_origin_write(
                request.headers(),
                &snapshot.public_origin
            ));
            request
                .headers_mut()
                .insert("origin", HeaderValue::from_static("http://10.10.1.1:8317"));
            assert!(!is_same_origin_write(
                request.headers(),
                &snapshot.public_origin
            ));
        }
    }

    #[test]
    fn proxy_mode_requires_a_trusted_peer_and_complete_forwarding_headers() {
        let mut server = ServerConfig {
            trusted_proxies: vec!["127.0.0.1".into()],
            ..Default::default()
        };
        let original = server.public_origin.clone();
        let mut request = Request::builder()
            .extension(ConnectInfo(
                "192.0.2.1:49152".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        apply_proxy_origin(&request, &mut server).unwrap();
        server.public_origin_from_proxy = true;
        assert_eq!(
            apply_proxy_origin(&request, &mut server),
            Err(StatusCode::FORBIDDEN)
        );
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:49152".parse::<SocketAddr>().unwrap(),
        ));
        apply_proxy_origin(&request, &mut server).unwrap();
        assert_eq!(server.public_origin, original);
        request
            .headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("http"));
        assert_eq!(
            apply_proxy_origin(&request, &mut server),
            Err(StatusCode::BAD_REQUEST)
        );
        request.headers_mut().append(
            "x-forwarded-host",
            HeaderValue::from_static("router.example:8080"),
        );
        request.headers_mut().append(
            "x-forwarded-host",
            HeaderValue::from_static("router.example:8081"),
        );
        assert_eq!(
            apply_proxy_origin(&request, &mut server),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn honors_forwarded_for_only_from_trusted_proxies() {
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        let headers = forwarded("6.6.6.6, 203.0.113.9");
        assert_eq!(forwarded_client(peer, &headers, &[]), peer);
        assert_eq!(
            forwarded_client(peer, &headers, &nets(&["127.0.0.1"])),
            "203.0.113.9".parse::<IpAddr>().unwrap()
        );
        // Additional trusted hops are skipped from the right.
        let chained = forwarded("6.6.6.6, 203.0.113.9, 10.0.0.2");
        assert_eq!(
            forwarded_client(peer, &chained, &nets(&["127.0.0.1", "10.0.0.0/8"])),
            "203.0.113.9".parse::<IpAddr>().unwrap()
        );
        // A malformed hop stops the walk at the last valid address.
        let garbage = forwarded("203.0.113.9, nonsense");
        assert_eq!(
            forwarded_client(peer, &garbage, &nets(&["127.0.0.1"])),
            peer
        );
        assert_eq!(
            forwarded_client(peer, &HeaderMap::new(), &nets(&["127.0.0.1"])),
            peer
        );
    }

    #[test]
    fn throttles_failures_per_client_and_window() {
        let throttle = LoginThrottle::default();
        let attacker = Some("203.0.113.9".parse().unwrap());
        let other = Some("198.51.100.1".parse().unwrap());
        let now = 1_000_000;
        for _ in 0..MAX_LOGIN_FAILURES {
            throttle.begin(attacker, now).unwrap().fail(now);
        }
        assert_eq!(throttle.begin(attacker, now).err(), Some(600));
        assert!(throttle.begin(other, now).is_ok());
        assert!(
            throttle
                .begin(attacker, now + LOGIN_FAILURE_WINDOW_MS)
                .is_ok()
        );
    }

    #[test]
    fn concurrent_attempts_reserve_budget() {
        let throttle = LoginThrottle::default();
        let client = Some("203.0.113.9".parse().unwrap());
        let now = 1_000_000;
        let pending: Vec<_> = (0..MAX_LOGIN_FAILURES)
            .map(|_| throttle.begin(client, now).unwrap())
            .collect();
        assert!(throttle.begin(client, now).is_err());
        drop(pending);
        assert!(throttle.begin(client, now).is_ok());
    }

    #[test]
    fn success_does_not_clear_earlier_failures() {
        let throttle = LoginThrottle::default();
        let client = Some("203.0.113.9".parse().unwrap());
        let now = 1_000_000;
        for _ in 0..MAX_LOGIN_FAILURES - 1 {
            throttle.begin(client, now).unwrap().fail(now);
        }
        // A valid sign-in (attempt dropped without failing) keeps the count.
        drop(throttle.begin(client, now).unwrap());
        throttle.begin(client, now).unwrap().fail(now);
        assert!(throttle.begin(client, now).is_err());
    }

    #[test]
    fn groups_ipv6_clients_by_prefix() {
        let first = Some("2001:db8:1:2::1".parse().unwrap());
        let second = Some("2001:db8:1:2:ffff::9".parse().unwrap());
        assert_eq!(throttle_key(first), throttle_key(second));
    }

    #[test]
    fn strips_only_console_cookies() {
        let mut headers = HeaderMap::new();
        headers.append(
            "cookie",
            HeaderValue::from_static("a=1; __Host-codex-session=secret"),
        );
        headers.append("cookie", HeaderValue::from_static("codex-session=x; b=2"));
        strip_console_cookies(&mut headers);
        assert_eq!(headers.get_all("cookie").iter().count(), 1);
        assert_eq!(headers["cookie"], "a=1; b=2");

        let mut only_console = HeaderMap::new();
        only_console.insert("cookie", HeaderValue::from_static("__Host-codex-admin=v"));
        strip_console_cookies(&mut only_console);
        assert!(only_console.get("cookie").is_none());
    }
}
