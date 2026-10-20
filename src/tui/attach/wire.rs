//! Bounded API framing for the TUI's single multiplexed connection.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::{fd::AsRawFd, unix::net::UnixStream};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(super) struct RemoteError(pub crate::api::Error);
impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.message)?;
        if let Some(hint) = &self.0.hint {
            write!(f, "; {hint}")?;
        }
        Ok(())
    }
}
impl std::error::Error for RemoteError {}

pub(super) struct Wire {
    reader: BufReader<UnixStream>,
    partial: Vec<u8>,
    began: Option<Instant>,
    id: u64,
    instance: Option<String>,
    hello: Option<crate::api::HelloResult>,
}
impl Wire {
    pub(super) fn connect() -> Result<Self> {
        let stream = UnixStream::connect(crate::config::runtime_dir().join("api.sock"))
            .context("connecting to the fbittorrent API")?;
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut size,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if credentials.uid != unsafe { libc::geteuid() } {
            bail!("API peer belongs to another user");
        }
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        let mut wire = Self {
            reader: BufReader::new(stream),
            partial: Vec::new(),
            began: None,
            id: 0,
            instance: None,
            hello: None,
        };
        let result = wire.call(
            "hello",
            json!({"proto":1,"interactive":true,"client":{
                "kind":"tui","name":format!("tui-{}",std::process::id()),"version":env!("CARGO_PKG_VERSION")
            }}),
        )?;
        let hello: crate::api::HelloResult = serde_json::from_value(result)?;
        anyhow::ensure!(
            hello.proto == crate::api::PROTO,
            "incompatible API protocol {}",
            hello.proto
        );
        wire.instance = Some(hello.instance.as_str().to_owned());
        wire.hello = Some(hello);
        Ok(wire)
    }
    pub(super) fn hello(&self) -> &crate::api::HelloResult {
        self.hello
            .as_ref()
            .expect("Wire is exposed only after hello succeeds")
    }
    pub(super) fn interrupt(&self) -> Result<UnixStream> {
        Ok(self.reader.get_ref().try_clone()?)
    }
    pub(super) fn encode(value: &Value) -> Result<Vec<u8>> {
        struct Capped(Vec<u8>);
        impl Write for Capped {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.0.len().saturating_add(bytes.len()) >= crate::api::LINE_IN {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "API request exceeds line limit",
                    ));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut encoded = Capped(Vec::new());
        serde_json::to_writer(&mut encoded, value)?;
        encoded.0.push(b'\n');
        Ok(encoded.0)
    }
    pub(super) fn start(
        &mut self,
        op: &str,
        args: Value,
        reference: Option<&crate::api::MutationRef>,
    ) -> Result<u64> {
        self.id = self.id.checked_add(1).context("API request id exhausted")?;
        let mut request = json!({"id":self.id,"op":op,"args":args});
        if let Some(instance) = &self.instance {
            request["instance"] = json!(instance);
        }
        if let Some(reference) = reference {
            request["ref"] = json!(reference);
        }
        let bytes = Self::encode(&request)?;
        let typed: crate::api::Request = serde_json::from_value(request.clone())?;
        crate::api::validation::Validate::validate(&typed)?;
        self.reader.get_mut().write_all(&bytes)?;
        Ok(self.id)
    }
    pub(super) fn result(op: &str, frame: &Value) -> Result<Value> {
        use crate::api::validation::Validate;
        if frame["ok"] != true {
            let error: crate::api::Error = serde_json::from_value(frame["error"].clone())?;
            return Err(RemoteError(error).into());
        }
        if op == "hello" {
            if let Some(protocol) = frame["result"]["proto"].as_u64().filter(|protocol|*protocol!=crate::api::PROTO) {
                return Err(RemoteError(crate::api::Error {
                    code:crate::api::ErrorCode::ProtoUnsupported,
                    message:crate::user_messages::PROTOCOL.into(),retryable:false,hint:None,
                    details:Some(serde_json::Map::from_iter([("server_proto".into(),json!(protocol))])),
                }).into());
            }
        }
        let reply = crate::api::Reply::decode(op, frame["result"].clone())?;
        reply.validate()?;
        Ok(frame["result"].clone())
    }
    pub(super) fn call(&mut self, op: &str, args: Value) -> Result<Value> {
        let id = self.start(op, args, None)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let Some(frame) = self.receive()? else {
                continue;
            };
            if frame["ev"] == "bye" {
                bail!("fbittorrent stopped");
            }
            if frame.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return Self::result(op, &frame);
        }
        bail!("API {op} timed out; outcome is unconfirmed")
    }
    pub(super) fn receive(&mut self) -> Result<Option<Value>> {
        loop {
            if self
                .began
                .is_some_and(|began| began.elapsed() > Duration::from_secs(2))
            {
                bail!("API frame timed out");
            }
            let available = match self.reader.fill_buf() {
                Ok(available) => available,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(None)
                }
                Err(error) => return Err(error.into()),
            };
            if available.is_empty() {
                bail!("fbittorrent stopped (API closed)");
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            anyhow::ensure!(
                self.partial.len() + count <= crate::api::LINE_OUT,
                "API frame exceeds line limit"
            );
            self.began.get_or_insert_with(Instant::now);
            self.partial.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let result = serde_json::from_slice(&self.partial);
                self.partial.clear();
                self.began = None;
                return result.map(Some).context("invalid API frame");
            }
        }
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        let _ = self.reader.get_mut().shutdown(std::net::Shutdown::Both);
    }
}
