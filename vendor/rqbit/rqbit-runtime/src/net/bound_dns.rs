use super::{BoundTcpStream, InterfaceBinding};
use crate::{
    io::{AsyncReadExt, AsyncWriteExt},
    Cx,
};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

pub async fn resolve_host(
    cx: &Cx,
    host: &str,
    port: u16,
    binding: Option<&InterfaceBinding>,
) -> io::Result<Vec<SocketAddr>> {
    if let Some(binding) = binding {
        return resolve_bound(cx, host, port, binding).await;
    }
    super::lookup_host(cx, (host.to_owned(), port))
        .await
        .map(Iterator::collect)
}

/// Resolve using DNS packets sent only over sockets bound to the selected
/// interface. Never calls libc's resolver, NSS, or a local DNS proxy.
pub async fn resolve_bound(
    cx: &Cx,
    host: &str,
    port: u16,
    binding: &InterfaceBinding,
) -> io::Result<Vec<SocketAddr>> {
    cx.checkpoint()
        .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
    binding.check_ready()?;
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let servers = nameservers();
    let mut addresses = Vec::new();
    let mut last_error = io::Error::new(
        io::ErrorKind::NotFound,
        format!("no DNS addresses for {host}"),
    );
    for qtype in [1, 28] {
        for &server in &servers {
            cx.checkpoint()
                .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
            binding.check_ready()?;
            match crate::time::timeout(
                cx,
                Duration::from_secs(3),
                query(cx, host, qtype, server, binding),
            )
            .await
            {
                Ok(Ok(found)) => {
                    addresses.extend(found.into_iter().map(|ip| SocketAddr::new(ip, port)));
                    break;
                }
                Ok(Err(error)) => last_error = error,
                Err(_) => {
                    last_error =
                        io::Error::new(io::ErrorKind::TimedOut, "bound DNS query timed out")
                }
            }
        }
    }
    binding.check_ready()?;
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        Err(last_error)
    } else {
        Ok(addresses)
    }
}

fn nameservers() -> Vec<SocketAddr> {
    let mut servers: Vec<_> = std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            (words.next()? == "nameserver")
                .then(|| words.next()?.parse::<IpAddr>().ok())
                .flatten()
        })
        // Sending to a local stub would let that separate process resolve on
        // the ordinary network. Use a remote DNS service through the bound
        // VPN socket when the host only supplies a local stub.
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map(|ip| SocketAddr::new(ip, 53))
        .take(2)
        .collect();
    // LAN DNS supplied by the host may be unreachable through the tunnel.
    // These fallback requests also use bound sockets, never the system resolver.
    for address in ["1.1.1.1", "2606:4700:4700::1111"] {
        let server = SocketAddr::new(address.parse().unwrap(), 53);
        if !servers.contains(&server) {
            servers.push(server);
        }
    }
    servers
}

fn packet(host: &str, qtype: u16) -> io::Result<(u16, Vec<u8>)> {
    let mut entropy = [0; 2];
    getrandom::fill(&mut entropy).map_err(|error| io::Error::other(error.to_string()))?;
    let id = u16::from_be_bytes(entropy);
    let mut packet = vec![entropy[0], entropy[1], 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid DNS hostname",
        ));
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid DNS label",
            ));
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    Ok((id, packet))
}

async fn query(
    cx: &Cx,
    host: &str,
    qtype: u16,
    server: SocketAddr,
    binding: &InterfaceBinding,
) -> io::Result<Vec<IpAddr>> {
    let (id, request) = packet(host, qtype)?;
    let socket = Socket::new(
        if server.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        },
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    binding.bind_socket(&socket)?;
    let local = if server.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0u16; 8], 0))
    };
    socket.bind(&local.into())?;
    let raw: std::net::UdpSocket = socket.into();
    raw.connect(server)?;
    let mut socket = asupersync::net::UdpSocket::from_std(raw)?;
    binding.check_ready()?;
    socket.send_to(&request, server).await?;
    let mut response = vec![0; 65535];
    let size = loop {
        cx.checkpoint()
            .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
        binding.check_ready()?;
        let (size, source) = socket.recv_from(&mut response).await?;
        if source == server && response.get(..2) == Some(&id.to_be_bytes()) {
            break size;
        }
    };
    response.truncate(size);
    if response.get(2).is_some_and(|flags| flags & 2 != 0) {
        // Truncated UDP answers retry over an equally bound TCP socket.
        let mut stream = BoundTcpStream::connect(cx, server, Some(binding), None).await?;
        stream
            .write_all(&(request.len() as u16).to_be_bytes())
            .await?;
        stream.write_all(&request).await?;
        let mut length = [0; 2];
        stream.read_exact(&mut length).await?;
        response.resize(u16::from_be_bytes(length) as usize, 0);
        stream.read_exact(&mut response).await?;
    }
    binding.check_ready()?;
    parse_response(&response, id, host, qtype)
}

fn malformed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid DNS response")
}
fn read_u16(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(malformed)?
            .try_into()
            .unwrap(),
    ))
}

fn name(bytes: &[u8], offset: &mut usize) -> io::Result<String> {
    let mut pos = *offset;
    let mut jumped = false;
    let mut labels = Vec::new();
    let mut visited = HashSet::new();
    for _ in 0..128 {
        if !visited.insert(pos) {
            return Err(malformed());
        }
        let len = *bytes.get(pos).ok_or_else(malformed)?;
        if len & 0xc0 == 0xc0 {
            let pointer = (read_u16(bytes, pos)? & 0x3fff) as usize;
            if !jumped {
                *offset = pos + 2;
            }
            jumped = true;
            pos = pointer;
            continue;
        }
        if len & 0xc0 != 0 {
            return Err(malformed());
        }
        pos += 1;
        if len == 0 {
            if !jumped {
                *offset = pos;
            }
            let name = labels.join(".");
            if name.len() > 253 {
                return Err(malformed());
            }
            return Ok(name);
        }
        let label = bytes.get(pos..pos + len as usize).ok_or_else(malformed)?;
        let label = std::str::from_utf8(label).map_err(|_| malformed())?;
        labels.push(label.to_ascii_lowercase());
        pos += len as usize;
    }
    Err(malformed())
}

fn parse_response(bytes: &[u8], id: u16, host: &str, qtype: u16) -> io::Result<Vec<IpAddr>> {
    if bytes.len() < 12
        || read_u16(bytes, 0)? != id
        || read_u16(bytes, 2)? & 0xfa0f != 0x8000
        || read_u16(bytes, 4)? != 1
    {
        return Err(malformed());
    }
    let mut offset = 12;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if name(bytes, &mut offset)? != host
        || read_u16(bytes, offset)? != qtype
        || read_u16(bytes, offset + 2)? != 1
    {
        return Err(malformed());
    }
    offset += 4;
    let mut aliases = HashMap::new();
    let mut addresses = Vec::new();
    for _ in 0..read_u16(bytes, 6)? {
        let owner = name(bytes, &mut offset)?;
        let kind = read_u16(bytes, offset)?;
        let class = read_u16(bytes, offset + 2)?;
        let len = read_u16(bytes, offset + 8)? as usize;
        offset += 10;
        let data = bytes.get(offset..offset + len).ok_or_else(malformed)?;
        if class == 1 {
            match (kind, data.len()) {
                (1, 4) => addresses.push((
                    owner,
                    IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3])),
                )),
                (28, 16) => addresses.push((
                    owner,
                    IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(data).unwrap())),
                )),
                (5, _) => {
                    let mut cname_offset = offset;
                    let target = name(bytes, &mut cname_offset)?;
                    if cname_offset > offset + len {
                        return Err(malformed());
                    }
                    aliases.insert(owner, target);
                }
                _ => {}
            }
        }
        offset += len;
    }
    let mut accepted = HashSet::new();
    let mut next = host;
    for _ in 0..32 {
        if !accepted.insert(next.clone()) {
            break;
        }
        match aliases.get(&next) {
            Some(target) => next = target.clone(),
            None => break,
        }
    }
    Ok(addresses
        .into_iter()
        .filter_map(|(owner, address)| accepted.contains(&owner).then_some(address))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dns_parser_rejects_mismatch_and_compression_cycles() {
        let (id, mut response) = packet("tracker.invalid", 1).unwrap();
        response[2] = 0x81;
        response[3] = 0x80;
        response[6..8].copy_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1]);
        assert_eq!(
            parse_response(&response, id, "tracker.invalid", 1).unwrap(),
            vec!["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
        assert!(parse_response(&response, id.wrapping_add(1), "tracker.invalid", 1).is_err());
        assert!(parse_response(&response, id, "other.invalid", 1).is_err());
        let mut offset = 0;
        assert!(name(&[0xc0, 0], &mut offset).is_err());
    }

    #[test]
    fn isolated_dns_socket_and_truncated_tcp_answer() {
        if !super::super::interface::isolated_netns() {
            return;
        }
        super::super::interface::ip(&["link", "set", "lo", "up"]);
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let addr = udp.local_addr().unwrap();
        let tcp = std::net::TcpListener::bind(addr).unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut packet = [0; 512];
            let (len, source) = udp.recv_from(&mut packet).unwrap();
            let mut truncated = packet[..len].to_vec();
            truncated[2] = 0x83;
            truncated[3] = 0x80;
            udp.send_to(&truncated, source).unwrap();
            let (mut socket, _) = tcp.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut len = [0; 2];
            socket.read_exact(&mut len).unwrap();
            let mut answer = vec![0; u16::from_be_bytes(len) as usize];
            socket.read_exact(&mut answer).unwrap();
            answer[2] = 0x81;
            answer[3] = 0x80;
            answer[6..8].copy_from_slice(&1u16.to_be_bytes());
            answer.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 4]);
            socket
                .write_all(&(answer.len() as u16).to_be_bytes())
                .unwrap();
            socket.write_all(&answer).unwrap();
        });
        let runtime = crate::RuntimeBuilder::current_thread().build().unwrap();
        let owner = runtime.request_cx_with_budget(crate::Budget::INFINITE);
        let task = crate::task::spawn_with(&owner, move |cx| async move {
            let binding = InterfaceBinding::new("lo").unwrap();
            let answer = crate::time::timeout(
                &cx,
                Duration::from_secs(3),
                query(&cx, "tracker.invalid", 1, addr, &binding),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(answer, vec!["192.0.2.4".parse::<IpAddr>().unwrap()]);
            super::super::interface::ip(&["link", "set", "lo", "down"]);
            assert!(resolve_bound(&cx, "tracker.invalid", 6881, &binding)
                .await
                .is_err());
            assert!(resolve_bound(&cx, "192.0.2.1", 6881, &binding)
                .await
                .is_err());
            super::super::interface::ip(&["link", "set", "lo", "up"]);
        });
        runtime.block_on(task).unwrap();
        server.join().unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
    }
}
