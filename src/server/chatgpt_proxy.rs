use std::io;

use anyhow::{Result, anyhow, bail};
use axum::http::Uri;
use hyper_util::client::legacy::connect::{HttpConnector, proxy::SocksV5};
use percent_encoding::percent_decode_str;
use tokio::net::{TcpStream, lookup_host};
use tower::Service;
use url::{Host, Url};

use super::config::WebsocketIdleTimeout;

#[derive(Clone)]
pub(crate) struct ChatgptTransport {
    client: reqwest::Client,
    proxy: Option<ChatgptProxy>,
    websocket_idle_timeout: WebsocketIdleTimeout,
}

impl ChatgptTransport {
    pub const fn new(
        client: reqwest::Client,
        proxy: Option<ChatgptProxy>,
        websocket_idle_timeout: WebsocketIdleTimeout,
    ) -> Self {
        Self {
            client,
            proxy,
            websocket_idle_timeout,
        }
    }

    /// Live idle timeout for proxied WebSockets (follows the settings page).
    pub fn websocket_idle_timeout(&self) -> WebsocketIdleTimeout {
        self.websocket_idle_timeout.clone()
    }

    pub const fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub const fn proxy(&self) -> Option<&ChatgptProxy> {
        self.proxy.as_ref()
    }
}

#[derive(Clone)]
pub(crate) struct ChatgptProxy {
    http_proxy: reqwest::Proxy,
    tunnel: Tunnel,
    auth: Option<ProxyAuth>,
}

/// How a WebSocket reaches the upstream through the proxy.
#[derive(Clone)]
enum Tunnel {
    Socks {
        proxy_uri: Uri,
        remote_dns: bool,
    },
    /// HTTP `CONNECT` to `host:port` of the proxy.
    HttpConnect {
        address: String,
    },
}

#[derive(Clone)]
struct ProxyAuth {
    username: String,
    password: String,
}

impl ChatgptProxy {
    pub fn parse(value: &str) -> Result<Self> {
        if value.trim() != value || value.is_empty() {
            bail!("proxy URL must not be empty or contain surrounding whitespace");
        }
        let url = Url::parse(value).map_err(|_| anyhow!("proxy URL is invalid"))?;
        let (default_port, remote_dns) = match url.scheme() {
            "socks5" => (1080, false),
            "socks5h" => (1080, true),
            "http" => (8080, true),
            _ => bail!("proxy URL must use socks5, socks5h, or http"),
        };
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            bail!("proxy URL must not contain a path, query, or fragment");
        }
        let proxy_host = match url
            .host()
            .ok_or_else(|| anyhow!("proxy URL must include a host"))?
        {
            Host::Domain(value) => value.to_owned(),
            Host::Ipv4(value) => value.to_string(),
            Host::Ipv6(value) => format!("[{value}]"),
        };
        let port = url.port().unwrap_or(default_port);
        if port == 0 {
            bail!("proxy port must be greater than zero");
        }
        let auth = match (url.username(), url.password()) {
            ("", None) => None,
            (username, Some(password)) if !username.is_empty() && !password.is_empty() => {
                let username = decode_credential(username)?;
                let password = decode_credential(password)?;
                if username.is_empty()
                    || password.is_empty()
                    || username.len() > u8::MAX as usize
                    || password.len() > u8::MAX as usize
                {
                    bail!("proxy username and password must each contain 1-255 UTF-8 bytes");
                }
                Some(ProxyAuth { username, password })
            }
            _ => bail!("proxy username and password must be provided together"),
        };
        let tunnel = if url.scheme() == "http" {
            Tunnel::HttpConnect {
                address: format!("{proxy_host}:{port}"),
            }
        } else {
            Tunnel::Socks {
                proxy_uri: format!("socks5://{proxy_host}:{port}")
                    .parse()
                    .map_err(|_| anyhow!("proxy URL is invalid"))?,
                remote_dns,
            }
        };
        let http_proxy = reqwest::Proxy::all(url.as_str())
            .map_err(|_| anyhow!("failed to configure the proxy"))?;
        Ok(Self {
            http_proxy,
            tunnel,
            auth,
        })
    }

    pub fn http_proxy(&self) -> reqwest::Proxy {
        self.http_proxy.clone()
    }

    pub async fn connect(&self, target: &Url) -> io::Result<TcpStream> {
        let port = target
            .port_or_known_default()
            .ok_or_else(|| invalid_input("WebSocket upstream URL has no port"))?;
        let host = target
            .host()
            .ok_or_else(|| invalid_input("WebSocket upstream URL has no host"))?;
        let remote_dns = match &self.tunnel {
            Tunnel::Socks { remote_dns, .. } => *remote_dns,
            Tunnel::HttpConnect { .. } => true,
        };
        match host {
            Host::Domain(host) if !remote_dns => {
                let addresses = lookup_host((host, port)).await?;
                let mut last_error = None;
                for address in addresses {
                    let host = match address.ip() {
                        std::net::IpAddr::V4(value) => value.to_string(),
                        std::net::IpAddr::V6(value) => format!("[{value}]"),
                    };
                    match self.connect_target(&host, address.port()).await {
                        Ok(stream) => return Ok(stream),
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "upstream hostname did not resolve")
                }))
            }
            Host::Domain(host) => self.connect_target(host, port).await,
            Host::Ipv4(address) => self.connect_target(&address.to_string(), port).await,
            Host::Ipv6(address) => self.connect_target(&format!("[{address}]"), port).await,
        }
    }

    async fn connect_target(&self, host: &str, port: u16) -> io::Result<TcpStream> {
        match &self.tunnel {
            Tunnel::Socks { proxy_uri, .. } => {
                let target = format!("https://{host}:{port}")
                    .parse()
                    .map_err(|_| invalid_input("WebSocket upstream URL is invalid"))?;
                let mut connector = HttpConnector::new();
                connector.enforce_http(false);
                let mut socks = SocksV5::new(proxy_uri.clone(), connector);
                if let Some(auth) = &self.auth {
                    socks = socks.with_auth(auth.username.clone(), auth.password.clone());
                }
                Service::call(&mut socks, target)
                    .await
                    .map(|stream| stream.into_inner())
                    .map_err(|error| {
                        io::Error::new(io::ErrorKind::ConnectionRefused, error.to_string())
                    })
            }
            Tunnel::HttpConnect { address } => {
                http_connect(address, &format!("{host}:{port}"), self.auth.as_ref()).await
            }
        }
    }
}

/// Most bytes of a `CONNECT` response head read before giving up.
const MAX_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;

/// Opens a tunnel to `target` through an HTTP proxy with `CONNECT`.
async fn http_connect(
    proxy: &str,
    target: &str,
    auth: Option<&ProxyAuth>,
) -> io::Result<TcpStream> {
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = TcpStream::connect(proxy).await?;
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = auth {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", auth.username, auth.password));
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    // Read byte by byte so nothing past the response head is consumed.
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "proxy closed the connection during CONNECT",
            ));
        }
        head.push(byte[0]);
        if head.len() > MAX_CONNECT_RESPONSE_BYTES {
            return Err(invalid_input("proxy CONNECT response is too large"));
        }
    }
    let status_line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let status = std::str::from_utf8(status_line)
        .ok()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    if status != "200" {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("proxy refused CONNECT with status {status}"),
        ));
    }
    Ok(stream)
}

/// The proxy URL with any password replaced, safe to show in the console.
pub(crate) fn redact_proxy_url(value: &str) -> String {
    match Url::parse(value) {
        Ok(mut url) if url.password().is_some() => {
            let _ = url.set_password(Some(REDACTED_PASSWORD));
            url.to_string().trim_end_matches('/').to_owned()
        }
        _ => value.to_owned(),
    }
}

/// Placeholder shown instead of a stored proxy password.
pub(crate) const REDACTED_PASSWORD: &str = "********";

/// Upstream transports keyed by proxy URL: one HTTP client per distinct proxy,
/// built on first use. Accounts without a proxy share the default transport.
#[derive(Clone)]
pub(crate) struct ChatgptTransports {
    default: ChatgptTransport,
    by_proxy: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, ChatgptTransport>>>,
}

impl ChatgptTransports {
    pub fn new(default: ChatgptTransport) -> Self {
        Self {
            default,
            by_proxy: std::sync::Arc::default(),
        }
    }

    /// Transport for traffic that belongs to no pooled account.
    pub const fn default_transport(&self) -> &ChatgptTransport {
        &self.default
    }

    /// Transport through `proxy`, or the default one when `None`.
    pub fn for_proxy(&self, proxy: Option<&str>) -> Result<ChatgptTransport> {
        let Some(proxy) = proxy else {
            return Ok(self.default.clone());
        };
        let mut cache = self
            .by_proxy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(transport) = cache.get(proxy) {
            return Ok(transport.clone());
        }
        let parsed = ChatgptProxy::parse(proxy)?;
        let client = upstream_client_builder()
            .proxy(parsed.http_proxy())
            .build()
            .map_err(|_| anyhow!("failed to create the proxied HTTP client"))?;
        let transport =
            ChatgptTransport::new(client, Some(parsed), self.default.websocket_idle_timeout());
        cache.insert(proxy.to_owned(), transport.clone());
        Ok(transport)
    }
}

/// Settings shared by every upstream HTTP client.
pub(crate) fn upstream_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
}

fn decode_credential(value: &str) -> Result<String> {
    percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| anyhow!("proxy credentials must be valid UTF-8"))
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn validates_socks5_proxy_urls() {
        for value in [
            "socks5://127.0.0.1:1080",
            "socks5h://proxy.example",
            "socks5://user:password@proxy.example:1081",
            "socks5://user:p%40ss@proxy.example",
            "http://127.0.0.1:8080",
            "http://user:secret@proxy.example:3128",
        ] {
            ChatgptProxy::parse(value).unwrap_or_else(|error| panic!("{value}: {error}"));
        }
        for value in [
            "",
            " socks5://127.0.0.1:1080",
            "https://127.0.0.1:1080",
            "ftp://127.0.0.1:1080",
            "socks5://127.0.0.1:1080/path",
            "socks5://127.0.0.1:1080?option=true",
            "socks5://127.0.0.1:0",
            "socks5://user@127.0.0.1:1080",
            "socks5://:password@127.0.0.1:1080",
        ] {
            assert!(ChatgptProxy::parse(value).is_err(), "{value}");
        }
    }

    #[tokio::test]
    async fn opens_an_authenticated_socks5_tunnel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut greeting = [0_u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [0x05, 0x01, 0x02]);
            stream.write_all(&[0x05, 0x02]).await.unwrap();

            let mut auth = [0_u8; 11];
            stream.read_exact(&mut auth).await.unwrap();
            assert_eq!(auth, *b"\x01\x04user\x04p@ss");
            stream.write_all(&[0x01, 0x00]).await.unwrap();

            let mut request = [0_u8; 5];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [0x05, 0x01, 0x00, 0x03, 11]);
            let mut target = [0_u8; 13];
            stream.read_exact(&mut target).await.unwrap();
            assert_eq!(&target[..11], b"chatgpt.com");
            assert_eq!(&target[11..], &443_u16.to_be_bytes());
            stream
                .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
        });

        let proxy = ChatgptProxy::parse(&format!("socks5h://user:p%40ss@{address}")).unwrap();
        let target = Url::parse("wss://chatgpt.com/backend-api/codex/responses").unwrap();
        let stream = proxy.connect(&target).await.unwrap();
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn opens_an_authenticated_http_connect_tunnel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(
                head.starts_with("CONNECT chatgpt.com:443 HTTP/1.1\r\n"),
                "{head}"
            );
            // base64("user:p@ss")
            assert!(
                head.contains("Proxy-Authorization: Basic dXNlcjpwQHNz\r\n"),
                "{head}"
            );
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nhello")
                .await
                .unwrap();
        });
        let proxy = ChatgptProxy::parse(&format!("http://user:p%40ss@{address}")).unwrap();
        let target = Url::parse("wss://chatgpt.com/backend-api/codex/responses").unwrap();
        let mut stream = proxy.connect(&target).await.unwrap();
        // Bytes after the response head belong to the tunnel.
        let mut tunneled = [0_u8; 5];
        stream.read_exact(&mut tunneled).await.unwrap();
        assert_eq!(&tunneled, b"hello");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_http_connect_fails() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 256];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await
                .unwrap();
        });
        let proxy = ChatgptProxy::parse(&format!("http://{address}")).unwrap();
        let target = Url::parse("wss://chatgpt.com/backend-api/codex/responses").unwrap();
        let error = proxy.connect(&target).await.unwrap_err();
        assert!(error.to_string().contains("407"), "{error}");
        server.await.unwrap();
    }

    #[test]
    fn redacts_proxy_passwords() {
        assert_eq!(
            redact_proxy_url("socks5://user:secret@proxy.example:1080"),
            "socks5://user:********@proxy.example:1080"
        );
        assert_eq!(
            redact_proxy_url("http://proxy.example:3128"),
            "http://proxy.example:3128"
        );
    }

    #[test]
    fn transports_are_shared_per_proxy() {
        let transports = ChatgptTransports::new(ChatgptTransport::new(
            reqwest::Client::new(),
            None,
            WebsocketIdleTimeout::fixed(0),
        ));
        assert!(transports.for_proxy(None).unwrap().proxy().is_none());
        let first = transports
            .for_proxy(Some("socks5h://127.0.0.1:1080"))
            .unwrap();
        assert!(first.proxy().is_some());
        transports
            .for_proxy(Some("socks5h://127.0.0.1:1080"))
            .unwrap();
        transports.for_proxy(Some("http://127.0.0.1:8080")).unwrap();
        assert_eq!(transports.by_proxy.lock().unwrap().len(), 2);
        assert!(transports.for_proxy(Some("ftp://x")).is_err());
    }
}
