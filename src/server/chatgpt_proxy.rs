use std::io;

use anyhow::{Result, anyhow, bail};
use axum::http::Uri;
use hyper_util::client::legacy::connect::{HttpConnector, proxy::SocksV5};
use percent_encoding::percent_decode_str;
use tokio::net::{TcpStream, lookup_host};
use tower::Service;
use url::{Host, Url};

#[derive(Clone)]
pub(crate) struct ChatgptTransport {
    client: reqwest::Client,
    proxy: Option<ChatgptProxy>,
}

impl ChatgptTransport {
    pub const fn new(client: reqwest::Client, proxy: Option<ChatgptProxy>) -> Self {
        Self { client, proxy }
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
    proxy_uri: Uri,
    remote_dns: bool,
    auth: Option<ProxyAuth>,
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
        let remote_dns = match url.scheme() {
            "socks5" => false,
            "socks5h" => true,
            _ => bail!("proxy URL must use socks5 or socks5h"),
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
        let port = url.port().unwrap_or(1080);
        if port == 0 {
            bail!("proxy port must be greater than zero");
        }
        let proxy_uri = format!("socks5://{proxy_host}:{port}")
            .parse()
            .map_err(|_| anyhow!("proxy URL is invalid"))?;
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
        let http_proxy = reqwest::Proxy::all(url.as_str())
            .map_err(|_| anyhow!("failed to configure the SOCKS5 proxy"))?;
        Ok(Self {
            http_proxy,
            proxy_uri,
            remote_dns,
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
        match target
            .host()
            .ok_or_else(|| invalid_input("WebSocket upstream URL has no host"))?
        {
            Host::Domain(host) if !self.remote_dns => {
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
        let target = format!("https://{host}:{port}")
            .parse()
            .map_err(|_| invalid_input("WebSocket upstream URL is invalid"))?;
        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        let mut socks = SocksV5::new(self.proxy_uri.clone(), connector);
        if let Some(auth) = &self.auth {
            socks = socks.with_auth(auth.username.clone(), auth.password.clone());
        }
        Service::call(&mut socks, target)
            .await
            .map(|stream| stream.into_inner())
            .map_err(|error| io::Error::new(io::ErrorKind::ConnectionRefused, error.to_string()))
    }
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
        ] {
            ChatgptProxy::parse(value).unwrap_or_else(|error| panic!("{value}: {error}"));
        }
        for value in [
            "",
            " socks5://127.0.0.1:1080",
            "http://127.0.0.1:1080",
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
}
