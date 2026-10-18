use crate::{
    net::{BoundTcpStream, InterfaceBinding},
    Cx,
};
use anyhow::{bail, Context, Result};
use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    ordinary: asupersync::http::Client,
    binding: Option<InterfaceBinding>,
    user_agent: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_future_stays_small() {
        let runtime = crate::RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(crate::Budget::INFINITE);
        let client = Client::new("HTTP future size regression");
        let request = client.get(&cx, "http://127.0.0.1/");
        let bytes = std::mem::size_of_val(&request);
        eprintln!("HTTP facade request future: {bytes} bytes");
        assert!(bytes <= 4096, "HTTP request future carries {bytes} bytes");
    }

    #[test]
    fn isolated_http_ipv6_redirect_and_interface_loss() {
        if !crate::net::isolated_netns() {
            return;
        }
        crate::net::ip(&["link", "set", "lo", "up"]);
        let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for expected in ["GET /start ", "GET /finish "] {
                let (mut socket, peer) = listener.accept().unwrap();
                assert!(peer.is_ipv6());
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut input = Vec::new();
                let mut bytes = [0; 512];
                while !input.windows(4).any(|s| s == b"\r\n\r\n") {
                    let size = socket.read(&mut bytes).unwrap();
                    assert!(size > 0);
                    input.extend_from_slice(&bytes[..size]);
                }
                assert!(std::str::from_utf8(&input).unwrap().starts_with(expected));
                if expected.contains("start") {
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /finish\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata").unwrap();
                }
            }
        });
        let runtime = crate::RuntimeBuilder::multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        assert_eq!(runtime.config().thread_stack_size, 2 * 1024 * 1024);
        let owner = runtime.request_cx_with_budget(crate::Budget::INFINITE);
        let task = crate::task::spawn_with(&owner, move |cx| async move {
            assert!(std::thread::current()
                .name()
                .unwrap()
                .starts_with("asupersync-worker-"));
            let binding = InterfaceBinding::new("lo").unwrap();
            let client = Client::new_bound("binding test", Some(binding));
            assert_eq!(
                client
                    .get(&cx, format!("http://{addr}/start"))
                    .await
                    .unwrap(),
                b"data"
            );
            crate::net::ip(&["link", "set", "lo", "down"]);
            assert!(client
                .get(&cx, format!("http://{addr}/start"))
                .await
                .is_err());
            assert!(client
                .get(&cx, "https://tracker.invalid/announce")
                .await
                .is_err());
            crate::net::ip(&["link", "set", "lo", "up"]);
        });
        runtime.block_on(task).unwrap();
        server.join().unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
    }
}
impl Client {
    pub fn new(user_agent: &str) -> Self {
        Self::new_bound(user_agent, None)
    }
    pub fn new_bound(user_agent: &str, binding: Option<InterfaceBinding>) -> Self {
        Self {
            ordinary: asupersync::http::Client::builder()
                .user_agent(user_agent)
                .request_timeout(Duration::from_secs(30))
                .max_body_size(32 * 1024 * 1024)
                .max_total_connections(16)
                .max_connections_per_host(4)
                .max_redirects(5)
                .build(),
            binding,
            user_agent: user_agent.to_owned(),
        }
    }
    pub async fn resolve(
        &self,
        cx: &Cx,
        host: &str,
        port: u16,
    ) -> std::io::Result<Vec<std::net::SocketAddr>> {
        crate::net::resolve_host(cx, host, port, self.binding.as_ref()).await
    }
    pub async fn get(&self, cx: &Cx, url: impl AsRef<str>) -> Result<Vec<u8>> {
        cx.checkpoint()?;
        if let Some(binding) = &self.binding {
            binding.check_ready()?;
            return crate::time::timeout(
                cx,
                Duration::from_secs(30),
                // Bound HTTP carries TLS and protocol state. Keep that future
                // on the heap so its size is not copied into every caller of
                // get(), including requests using the ordinary client.
                Box::pin(self.get_bound(cx, url.as_ref(), binding)),
            )
            .await
            .context("bound HTTP request timed out")?;
        }
        let response = self.ordinary.send_get(cx, url.as_ref()).await?;
        if !(200..300).contains(&response.status) {
            bail!("HTTP status {} for {}", response.status, url.as_ref());
        }
        cx.checkpoint()?;
        Ok(response.body)
    }
    async fn get_bound(
        &self,
        cx: &Cx,
        original_url: &str,
        binding: &InterfaceBinding,
    ) -> Result<Vec<u8>> {
        let mut url = url::Url::parse(original_url)?;
        for redirects in 0..=5 {
            cx.checkpoint()?;
            binding.check_ready()?;
            anyhow::ensure!(
                matches!(url.scheme(), "http" | "https"),
                "unsupported bound HTTP scheme"
            );
            anyhow::ensure!(
                url.username().is_empty() && url.password().is_none(),
                "HTTP URL credentials are unsupported"
            );
            let host = url
                .host_str()
                .context("HTTP URL has no host")?
                .trim_matches(['[', ']']);
            let port = url
                .port_or_known_default()
                .context("HTTP URL has no port")?;
            let addresses = self.resolve(cx, host, port).await?;
            let mut stream = None;
            let mut last_error = None;
            for address in addresses {
                binding.check_ready()?;
                match crate::time::timeout(
                    cx,
                    Duration::from_secs(8),
                    BoundTcpStream::connect(cx, address, Some(binding), None),
                )
                .await
                {
                    Ok(Ok(connected)) => {
                        stream = Some(connected);
                        break;
                    }
                    Ok(Err(error)) => last_error = Some(error),
                    Err(_) => {
                        last_error = Some(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "bound HTTP connection timed out",
                        ))
                    }
                }
            }
            let stream = stream.ok_or_else(|| {
                last_error.unwrap_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "HTTP hostname has no addresses",
                    )
                })
            })?;
            let mut path = url.path().to_owned();
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
            let request = asupersync::http::h1::Request::get(path)
                .header("Host", authority)
                .header("User-Agent", &self.user_agent)
                .header("Connection", "close")
                .build();
            let response = if url.scheme() == "https" {
                let tls = asupersync::tls::TlsConnector::builder()
                    .with_webpki_roots()
                    .alpn_protocols(vec![b"http/1.1".to_vec()])
                    .build()?;
                let stream = tls.connect(host, stream).await?;
                asupersync::http::h1::Http1Client::request_with_io_and_max_body_size(
                    stream,
                    request,
                    32 * 1024 * 1024,
                )
                .await?
                .0
            } else {
                asupersync::http::h1::Http1Client::request_with_io_and_max_body_size(
                    stream,
                    request,
                    32 * 1024 * 1024,
                )
                .await?
                .0
            };
            binding.check_ready()?;
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                anyhow::ensure!(redirects < 5, "too many HTTP redirects");
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value.as_str())
                    .context("HTTP redirect has no location")?;
                url = url.join(location)?;
                continue;
            }
            anyhow::ensure!(
                (200..300).contains(&response.status),
                "HTTP status {} for {}",
                response.status,
                url
            );
            return Ok(response.body);
        }
        bail!("too many HTTP redirects")
    }
}
