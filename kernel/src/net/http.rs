//! Minimal HTTP/1.0 client (plain HTTP only).

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::addr::Ipv4Addr;
use super::device;
use super::dhcp;
use super::dns;
use super::tcp::{self, TcpState};

const HTTP_PORT: u16 = 80;

pub enum FetchStep {
    Pending,
    Done(Vec<String>),
    Error(&'static str),
}

struct FetchState {
    url: String,
    host: String,
    path: String,
    remote_ip: Option<Ipv4Addr>,
    conn: Option<u32>,
    phase: u8,
    dns_iters: u32,
    tcp_iters: u32,
    recv_iters: u32,
    raw: Vec<u8>,
    recv_len: usize,
}

static FETCH: Mutex<Option<FetchState>> = Mutex::new(None);

pub fn is_online() -> bool {
    device::has_device() && dhcp::leased_ip().is_some()
}

pub fn fetch_cancel() {
    *FETCH.lock() = None;
    dns::reset_query();
}

/// One incremental step per idle tick — avoids stack overflow and UI freezes.
pub fn fetch_step(url: &str) -> FetchStep {
    let mut slot = FETCH.lock();
    if slot.is_none() {
        match begin_fetch(url) {
            Ok(st) => *slot = Some(st),
            Err(e) => return FetchStep::Error(e),
        }
    }

    let st = slot.as_mut().unwrap();
    for _ in 0..HTTP_STEP_BUDGET {
        match st.phase {
            0 => {
                if let Ok(octets) = parse_ipv4(&st.host) {
                    st.remote_ip = Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
                    st.phase = 2;
                } else {
                    st.phase = 1;
                }
            }
            1 => {
                st.dns_iters += 1;
                if let Some(ip) = dns::resolve_step(&st.host) {
                    st.remote_ip = Some(ip);
                    st.phase = 2;
                } else if st.dns_iters > 15_000 {
                    *slot = None;
                    return FetchStep::Error("DNS lookup failed");
                }
            }
            2 => {
                let ip = st.remote_ip.unwrap();
                if st.conn.is_none() {
                    st.conn = tcp::connect(super::addr::IpAddr::V4(ip), HTTP_PORT, 49152);
                    if st.conn.is_none() {
                        *slot = None;
                        return FetchStep::Error("TCP slot full");
                    }
                }
                st.tcp_iters += 1;
                super::rx_poll();
                if tcp::with_connection(st.conn.unwrap(), |c| c.state == TcpState::Established)
                    .unwrap_or(false)
                {
                    let request = alloc::format!(
                        "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
                        st.path, st.host
                    );
                    if tcp::send(st.conn.unwrap(), request.as_bytes()).is_err() {
                        *slot = None;
                        return FetchStep::Error("TCP send failed");
                    }
                    st.phase = 3;
                } else if st.tcp_iters > 30_000 {
                    *slot = None;
                    return FetchStep::Error("TCP connect timeout");
                }
            }
            3 => {
                st.recv_iters += 1;
                super::rx_poll();
                tcp::tick();
                if let Some(n) = tcp::recv(st.conn.unwrap(), &mut st.raw[st.recv_len..]) {
                    st.recv_len += n;
                    if st.recv_len >= st.raw.len() {
                        break;
                    }
                } else if tcp::with_connection(st.conn.unwrap(), |c| c.state == TcpState::Closed)
                    .unwrap_or(false)
                    || (st.recv_len > 0 && st.recv_iters > 4_000)
                {
                    break;
                } else if st.recv_iters > 100_000 {
                    *slot = None;
                    return FetchStep::Error("empty response");
                }
            }
            _ => break,
        }
    }

    if st.phase < 3 || (st.phase == 3 && st.recv_len == 0 && st.recv_iters <= 100_000) {
        return FetchStep::Pending;
    }

    if st.phase == 3 && st.recv_len == 0 {
        *slot = None;
        return FetchStep::Error("empty response");
    }

    let body = extract_body(&st.raw[..st.recv_len]);
    let text = strip_tags(body);
    let lines = wrap_lines(&text, 72);
    *slot = None;
    FetchStep::Done(lines)
}

const HTTP_STEP_BUDGET: u32 = 200;

fn begin_fetch(url: &str) -> Result<FetchState, &'static str> {
    if !device::has_device() {
        return Err("no network adapter");
    }
    if dhcp::leased_ip().is_none() {
        return Err("waiting for DHCP");
    }

    let url = url.trim();
    let rest = url.strip_prefix("http://").ok_or("use http:// only (no TLS yet)")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if host.is_empty() {
        return Err("missing host");
    }

    Ok(FetchState {
        url: String::from(url),
        host: String::from(host),
        path: String::from(path),
        remote_ip: None,
        conn: None,
        phase: 0,
        dns_iters: 0,
        tcp_iters: 0,
        recv_iters: 0,
        raw: alloc::vec![0u8; 8192],
        recv_len: 0,
    })
}

pub fn fetch(url: &str) -> Result<String, &'static str> {
    fetch_cancel();
    loop {
        match fetch_step(url) {
            FetchStep::Done(lines) => {
                return Ok(lines.join("\n"));
            }
            FetchStep::Error(e) => return Err(e),
            FetchStep::Pending => {}
        }
    }
}

pub fn fetch_lines(url: &str) -> Vec<String> {
    fetch_cancel();
    loop {
        match fetch_step(url) {
            FetchStep::Done(lines) => return lines,
            FetchStep::Error(e) => return alloc::vec![String::from(e)],
            FetchStep::Pending => {}
        }
    }
}

fn parse_ipv4(s: &str) -> Result<[u8; 4], ()> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return Err(());
        }
        parts[idx] = parse_u8(part).ok_or(())?;
        idx += 1;
    }
    if idx != 4 {
        return Err(());
    }
    Ok(parts)
}

fn parse_u8(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut n = 0u16;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u16;
        if n > 255 {
            return None;
        }
    }
    Some(n as u8)
}

fn extract_body(data: &[u8]) -> &str {
    let text = core::str::from_utf8(data).unwrap_or("");
    if let Some(pos) = text.find("\r\n\r\n") {
        &text[pos + 4..]
    } else if let Some(pos) = text.find("\n\n") {
        &text[pos + 2..]
    } else {
        text
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_space = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_space && !out.is_empty() {
                        out.push(' ');
                        last_space = true;
                    }
                } else {
                    out.push(ch);
                    last_space = false;
                }
            }
            _ => {}
        }
    }
    if out.len() > 4000 {
        out.truncate(4000);
        out.push_str("...");
    }
    out
}

fn wrap_lines(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::from(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::from("(empty page)"));
    }
    lines
}
