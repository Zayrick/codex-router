use std::{
    io,
    net::{IpAddr, SocketAddr},
};

use anyhow::{Result, anyhow, bail};
use percent_encoding::percent_decode_str;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};
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
    url: Url,
    host: ProxyHost,
    port: u16,
    remote_dns: bool,
    auth: Option<ProxyAuth>,
}

#[derive(Clone)]
enum ProxyHost {
    Domain(String),
    Ip(IpAddr),
}

#[derive(Clone)]
struct ProxyAuth {
    username: String,
    password: String,
}

enum SocksTarget {
    Domain(String, u16),
    Ip(SocketAddr),
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
        let host = match url
            .host()
            .ok_or_else(|| anyhow!("proxy URL must include a host"))?
        {
            Host::Domain(value) => ProxyHost::Domain(value.to_owned()),
            Host::Ipv4(value) => ProxyHost::Ip(value.into()),
            Host::Ipv6(value) => ProxyHost::Ip(value.into()),
        };
        let port = url.port().unwrap_or(1080);
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
        Ok(Self {
            url,
            host,
            port,
            remote_dns,
            auth,
        })
    }

    pub fn reqwest_proxy(&self) -> Result<reqwest::Proxy> {
        reqwest::Proxy::all(self.url.as_str())
            .map_err(|_| anyhow!("failed to configure the SOCKS5 proxy"))
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
                    match self.connect_target(&SocksTarget::Ip(address)).await {
                        Ok(stream) => return Ok(stream),
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(last_error.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "upstream hostname did not resolve")
                }))
            }
            Host::Domain(host) => {
                self.connect_target(&SocksTarget::Domain(host.to_owned(), port))
                    .await
            }
            Host::Ipv4(address) => {
                self.connect_target(&SocksTarget::Ip(SocketAddr::new(address.into(), port)))
                    .await
            }
            Host::Ipv6(address) => {
                self.connect_target(&SocksTarget::Ip(SocketAddr::new(address.into(), port)))
                    .await
            }
        }
    }

    async fn connect_target(&self, target: &SocksTarget) -> io::Result<TcpStream> {
        let mut stream = match &self.host {
            ProxyHost::Domain(host) => TcpStream::connect((host.as_str(), self.port)).await?,
            ProxyHost::Ip(host) => TcpStream::connect(SocketAddr::new(*host, self.port)).await?,
        };
        let method = if self.auth.is_some() { 0x02 } else { 0x00 };
        stream.write_all(&[0x05, 0x01, method]).await?;
        let mut greeting = [0_u8; 2];
        stream.read_exact(&mut greeting).await?;
        if greeting != [0x05, method] {
            return Err(proxy_protocol_error(
                "SOCKS5 proxy rejected authentication method",
            ));
        }
        if let Some(auth) = &self.auth {
            let mut request = Vec::with_capacity(3 + auth.username.len() + auth.password.len());
            request.extend_from_slice(&[0x01, auth.username.len() as u8]);
            request.extend_from_slice(auth.username.as_bytes());
            request.push(auth.password.len() as u8);
            request.extend_from_slice(auth.password.as_bytes());
            stream.write_all(&request).await?;
            let mut response = [0_u8; 2];
            stream.read_exact(&mut response).await?;
            if response != [0x01, 0x00] {
                return Err(proxy_protocol_error("SOCKS5 proxy authentication failed"));
            }
        }

        let mut request = Vec::with_capacity(32);
        request.extend_from_slice(&[0x05, 0x01, 0x00]);
        match target {
            SocksTarget::Domain(host, port) => {
                let length = u8::try_from(host.len())
                    .map_err(|_| invalid_input("WebSocket upstream hostname is too long"))?;
                request.extend_from_slice(&[0x03, length]);
                request.extend_from_slice(host.as_bytes());
                request.extend_from_slice(&port.to_be_bytes());
            }
            SocksTarget::Ip(SocketAddr::V4(address)) => {
                request.push(0x01);
                request.extend_from_slice(&address.ip().octets());
                request.extend_from_slice(&address.port().to_be_bytes());
            }
            SocksTarget::Ip(SocketAddr::V6(address)) => {
                request.push(0x04);
                request.extend_from_slice(&address.ip().octets());
                request.extend_from_slice(&address.port().to_be_bytes());
            }
        }
        stream.write_all(&request).await?;

        let mut response = [0_u8; 4];
        stream.read_exact(&mut response).await?;
        if response[0] != 0x05 || response[2] != 0x00 {
            return Err(proxy_protocol_error("invalid SOCKS5 connect response"));
        }
        if response[1] != 0x00 {
            return Err(proxy_protocol_error(socks5_status_message(response[1])));
        }
        let address_length = match response[3] {
            0x01 => 4,
            0x03 => {
                let mut length = [0_u8; 1];
                stream.read_exact(&mut length).await?;
                usize::from(length[0])
            }
            0x04 => 16,
            _ => return Err(proxy_protocol_error("invalid SOCKS5 address type")),
        };
        let mut ignored = vec![0_u8; address_length + 2];
        stream.read_exact(&mut ignored).await?;
        Ok(stream)
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

fn proxy_protocol_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionRefused, message)
}

const fn socks5_status_message(status: u8) -> &'static str {
    match status {
        0x01 => "SOCKS5 proxy reported a general failure",
        0x02 => "SOCKS5 proxy denied the connection",
        0x03 => "SOCKS5 proxy reported network unreachable",
        0x04 => "SOCKS5 proxy reported host unreachable",
        0x05 => "SOCKS5 proxy reported connection refused",
        0x06 => "SOCKS5 proxy reported TTL expired",
        0x07 => "SOCKS5 proxy does not support CONNECT",
        0x08 => "SOCKS5 proxy does not support the address type",
        _ => "SOCKS5 proxy returned an unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_socks5_proxy_urls() {
        for value in [
            "socks5://127.0.0.1:1080",
            "socks5h://proxy.example",
            "socks5://user:password@proxy.example:1081",
            "socks5://user:p%40ss@proxy.example",
        ] {
            ChatgptProxy::parse(value)
                .and_then(|proxy| proxy.reqwest_proxy().map(|_| ()))
                .unwrap_or_else(|error| panic!("{value}: {error}"));
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
    async fn performs_authenticated_socks5_websocket_connect() {
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
