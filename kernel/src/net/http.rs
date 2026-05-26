//! Minimal HTTP/1.0 client (plain HTTP only).
//!
//! Resolution → connect → request → receive is driven incrementally by
//! `fetch_step`, which is called from the browser's idle tick. Real
//! wall-clock timeouts ensure the kernel never spins indefinitely if a
//! remote server is slow or unreachable.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::addr::Ipv4Addr;
use super::device;
use super::dhcp;
use super::dns;
use super::tcp::{self, TcpState};

const DEFAULT_HTTP_PORT: u16 = 80;
const HTTP_STEP_BUDGET: u32 = 200;
const CONNECT_TIMEOUT_NS: u64 = 12_000_000_000;
const RECEIVE_IDLE_TIMEOUT_NS: u64 = 8_000_000_000;
const TOTAL_TIMEOUT_NS: u64 = 30_000_000_000;
const MAX_BODY_BYTES: usize = 64 * 1024;
const INITIAL_BUF: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchPhase {
    ResolvingDns,
    Connecting,
    Sending,
    Receiving,
}

impl FetchPhase {
    pub fn label(self) -> &'static str {
        match self {
            FetchPhase::ResolvingDns => "Resolving",
            FetchPhase::Connecting => "Connecting",
            FetchPhase::Sending => "Sending request",
            FetchPhase::Receiving => "Receiving",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchProgress {
    pub phase: FetchPhase,
    pub host: String,
    pub remote_ip: Option<Ipv4Addr>,
    pub bytes_received: usize,
    pub status_code: Option<u16>,
}

pub enum FetchStep {
    Pending,
    Done(FetchResult),
    Error(&'static str),
}

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub status_code: u16,
    pub host: String,
    pub remote_ip: Option<Ipv4Addr>,
    pub bytes_received: usize,
    pub lines: Vec<RenderedLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Body,
    Heading,
    Link,
    Code,
}

#[derive(Clone, Debug)]
pub struct RenderedLine {
    pub text: String,
    pub style: LineStyle,
}

struct FetchState {
    host: String,
    path: String,
    port: u16,
    remote_ip: Option<Ipv4Addr>,
    conn: Option<u32>,
    phase: FetchPhase,
    dns_iters: u32,
    raw: Vec<u8>,
    recv_len: usize,
    started_ns: u64,
    last_progress_ns: u64,
    sent: bool,
}

static FETCH: Mutex<Option<FetchState>> = Mutex::new(None);
static PROGRESS: Mutex<Option<FetchProgress>> = Mutex::new(None);

pub fn is_online() -> bool {
    device::has_device() && dhcp::leased_ip().is_some()
}

pub fn current_progress() -> Option<FetchProgress> {
    PROGRESS.lock().clone()
}

pub fn fetch_cancel() {
    *FETCH.lock() = None;
    *PROGRESS.lock() = None;
    dns::reset_query();
}

/// Drive one incremental step. Returns `Done` when the page is rendered,
/// `Pending` to continue next idle tick, or `Error` if the fetch fails.
pub fn fetch_step(url: &str) -> FetchStep {
    let mut slot = FETCH.lock();
    if slot.is_none() {
        match begin_fetch(url) {
            Ok(st) => *slot = Some(st),
            Err(e) => return FetchStep::Error(e),
        }
    }

    let now = crate::sched::timer::monotonic_ns();
    let st = slot.as_mut().unwrap();

    if now.saturating_sub(st.started_ns) > TOTAL_TIMEOUT_NS {
        let host = st.host.clone();
        drop(slot);
        finish_with_error(&host);
        return FetchStep::Error("Request timed out");
    }

    for _ in 0..HTTP_STEP_BUDGET {
        match st.phase {
            FetchPhase::ResolvingDns => {
                if let Ok(octets) = parse_ipv4(&st.host) {
                    st.remote_ip = Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
                    st.phase = FetchPhase::Connecting;
                    st.last_progress_ns = now;
                    continue;
                }
                st.dns_iters += 1;
                if let Some(ip) = dns::resolve_step(&st.host) {
                    st.remote_ip = Some(ip);
                    st.phase = FetchPhase::Connecting;
                    st.last_progress_ns = now;
                } else if st.dns_iters > 30_000
                    || now.saturating_sub(st.last_progress_ns) > CONNECT_TIMEOUT_NS
                {
                    *slot = None;
                    let mut p = PROGRESS.lock();
                    *p = None;
                    return FetchStep::Error("DNS lookup failed");
                }
            }
            FetchPhase::Connecting => {
                let ip = st.remote_ip.unwrap();
                if st.conn.is_none() {
                    st.conn = tcp::connect(super::addr::IpAddr::V4(ip), st.port, ephemeral_port());
                    if st.conn.is_none() {
                        *slot = None;
                        return FetchStep::Error("No free TCP slots");
                    }
                }
                super::rx_poll();
                if tcp::with_connection(st.conn.unwrap(), |c| c.state == TcpState::Established)
                    .unwrap_or(false)
                {
                    st.phase = FetchPhase::Sending;
                    st.last_progress_ns = now;
                } else if now.saturating_sub(st.last_progress_ns) > CONNECT_TIMEOUT_NS {
                    *slot = None;
                    return FetchStep::Error("TCP connect timeout");
                }
            }
            FetchPhase::Sending => {
                let request = format!(
                    "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: TheoryOS/0.1\r\nAccept: text/html, text/plain\r\nConnection: close\r\n\r\n",
                    st.path, st.host
                );
                if tcp::send(st.conn.unwrap(), request.as_bytes()).is_err() {
                    *slot = None;
                    return FetchStep::Error("TCP send failed");
                }
                st.sent = true;
                st.phase = FetchPhase::Receiving;
                st.last_progress_ns = now;
            }
            FetchPhase::Receiving => {
                super::rx_poll();
                tcp::tick();
                if st.recv_len == st.raw.len() {
                    if st.raw.len() < MAX_BODY_BYTES {
                        let new_len = (st.raw.len() * 2).min(MAX_BODY_BYTES);
                        st.raw.resize(new_len, 0);
                    } else {
                        break;
                    }
                }
                if let Some(n) = tcp::recv(st.conn.unwrap(), &mut st.raw[st.recv_len..]) {
                    st.recv_len += n;
                    st.last_progress_ns = now;
                } else if tcp::with_connection(st.conn.unwrap(), |c| c.state == TcpState::Closed)
                    .unwrap_or(false)
                {
                    break;
                } else if now.saturating_sub(st.last_progress_ns) > RECEIVE_IDLE_TIMEOUT_NS {
                    if st.recv_len > 0 {
                        break;
                    }
                    *slot = None;
                    return FetchStep::Error("No response from server");
                }
            }
        }
    }

    // Publish progress for the UI.
    {
        let mut p = PROGRESS.lock();
        *p = Some(FetchProgress {
            phase: st.phase,
            host: st.host.clone(),
            remote_ip: st.remote_ip,
            bytes_received: st.recv_len,
            status_code: None,
        });
    }

    let buffer_full = st.recv_len >= MAX_BODY_BYTES;
    let conn_closed = st
        .conn
        .and_then(|c| tcp::with_connection(c, |t| t.state == TcpState::Closed))
        .unwrap_or(false);
    let receive_idle =
        st.recv_len > 0 && now.saturating_sub(st.last_progress_ns) > RECEIVE_IDLE_TIMEOUT_NS;
    let receiving_done =
        st.phase == FetchPhase::Receiving && (buffer_full || conn_closed || receive_idle);

    if !receiving_done {
        return FetchStep::Pending;
    }

    let status = parse_status(&st.raw[..st.recv_len]);
    let body = extract_body(&st.raw[..st.recv_len]);
    let lines = render_html(body, 88);
    let result = FetchResult {
        status_code: status.unwrap_or(0),
        host: st.host.clone(),
        remote_ip: st.remote_ip,
        bytes_received: st.recv_len,
        lines,
    };
    *slot = None;
    *PROGRESS.lock() = None;
    FetchStep::Done(result)
}

fn finish_with_error(host: &str) {
    *FETCH.lock() = None;
    *PROGRESS.lock() = Some(FetchProgress {
        phase: FetchPhase::Receiving,
        host: String::from(host),
        remote_ip: None,
        bytes_received: 0,
        status_code: None,
    });
}

fn begin_fetch(url: &str) -> Result<FetchState, &'static str> {
    if !device::has_device() {
        return Err("No network adapter");
    }
    if dhcp::leased_ip().is_none() {
        return Err("No IP address — connect to Network first");
    }

    let url = url.trim();
    let rest = url
        .strip_prefix("http://")
        .ok_or("Only http:// URLs supported (no TLS)")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if hostport.is_empty() {
        return Err("Missing host");
    }
    let (host, port) = parse_host_port(hostport)?;

    let now = crate::sched::timer::monotonic_ns();
    Ok(FetchState {
        host: String::from(host),
        path: String::from(path),
        port,
        remote_ip: None,
        conn: None,
        phase: FetchPhase::ResolvingDns,
        dns_iters: 0,
        raw: alloc::vec![0u8; INITIAL_BUF],
        recv_len: 0,
        started_ns: now,
        last_progress_ns: now,
        sent: false,
    })
}

fn parse_host_port(s: &str) -> Result<(&str, u16), &'static str> {
    if let Some(idx) = s.find(':') {
        let host = &s[..idx];
        let port_str = &s[idx + 1..];
        let mut p: u32 = 0;
        if port_str.is_empty() {
            return Err("Invalid port");
        }
        for b in port_str.bytes() {
            if !b.is_ascii_digit() {
                return Err("Invalid port");
            }
            p = p * 10 + (b - b'0') as u32;
            if p > 65535 {
                return Err("Invalid port");
            }
        }
        Ok((host, p as u16))
    } else {
        Ok((s, DEFAULT_HTTP_PORT))
    }
}

fn ephemeral_port() -> u16 {
    let tsc = crate::arch::x86_64::cpu::rdtsc();
    49152 + ((tsc as u16) & 0x3FFF)
}

/// Blocking convenience wrapper used by the shell.
pub fn fetch_blocking(url: &str) -> Result<FetchResult, &'static str> {
    fetch_cancel();
    loop {
        match fetch_step(url) {
            FetchStep::Done(r) => return Ok(r),
            FetchStep::Error(e) => return Err(e),
            FetchStep::Pending => {
                super::rx_poll();
                for _ in 0..200 {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

pub fn fetch_lines(url: &str) -> Vec<String> {
    match fetch_blocking(url) {
        Ok(r) => r.lines.into_iter().map(|l| l.text).collect(),
        Err(e) => alloc::vec![String::from(e)],
    }
}

pub fn fetch(url: &str) -> Result<String, &'static str> {
    fetch_blocking(url).map(|r| {
        let mut out = String::new();
        for l in r.lines {
            out.push_str(&l.text);
            out.push('\n');
        }
        out
    })
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

fn parse_status(data: &[u8]) -> Option<u16> {
    let text = core::str::from_utf8(data).ok()?;
    let line = text.split('\n').next()?;
    let mut parts = line.split_whitespace();
    let _http = parts.next()?;
    let code = parts.next()?;
    code.parse::<u16>().ok()
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

/// Render HTML to a list of styled lines. Strips `<script>` and `<style>`
/// content, decodes common entities, preserves block breaks at headings/
/// paragraphs/list items, and keeps link text inline as `[text]`.
fn render_html(html: &str, wrap: usize) -> Vec<RenderedLine> {
    let stripped = strip_script_and_style(html);
    let mut tokens: Vec<HtmlToken> = Vec::new();
    let mut iter = stripped.char_indices().peekable();
    let bytes = stripped.as_bytes();
    let mut last_pos = 0usize;

    while let Some(&(i, ch)) = iter.peek() {
        if ch == '<' {
            if last_pos < i {
                push_text_token(&mut tokens, &stripped[last_pos..i]);
            }
            iter.next();
            let mut tag = String::new();
            while let Some(&(_, c)) = iter.peek() {
                iter.next();
                if c == '>' {
                    break;
                }
                tag.push(c);
            }
            if let Some(t) = classify_tag(&tag) {
                tokens.push(HtmlToken::Tag(t));
            }
            last_pos = iter.peek().map(|&(p, _)| p).unwrap_or(bytes.len());
        } else {
            iter.next();
        }
    }
    if last_pos < stripped.len() {
        push_text_token(&mut tokens, &stripped[last_pos..]);
    }

    let mut blocks: Vec<(LineStyle, String)> = Vec::new();
    let mut current_text = String::new();
    let mut current_style = LineStyle::Body;
    let mut last_space = true;

    for tok in tokens {
        match tok {
            HtmlToken::Text(s) => {
                let decoded = decode_entities(&s);
                for ch in decoded.chars() {
                    if ch.is_whitespace() {
                        if !last_space {
                            current_text.push(' ');
                            last_space = true;
                        }
                    } else {
                        current_text.push(ch);
                        last_space = false;
                    }
                }
            }
            HtmlToken::Tag(tag) => match tag {
                TagKind::ParagraphBreak | TagKind::Heading => {
                    if !current_text.trim().is_empty() {
                        blocks.push((current_style.clone(), current_text.trim().into()));
                    }
                    current_text = String::new();
                    last_space = true;
                    current_style = if matches!(tag, TagKind::Heading) {
                        LineStyle::Heading
                    } else {
                        LineStyle::Body
                    };
                }
                TagKind::EndHeading => {
                    if !current_text.trim().is_empty() {
                        blocks.push((LineStyle::Heading, current_text.trim().into()));
                    }
                    current_text = String::new();
                    last_space = true;
                    current_style = LineStyle::Body;
                }
                TagKind::LinkOpen => {
                    if !last_space {
                        current_text.push(' ');
                    }
                    current_text.push('[');
                    last_space = false;
                }
                TagKind::LinkClose => {
                    current_text.push(']');
                    last_space = false;
                }
                TagKind::ListItem => {
                    if !current_text.trim().is_empty() {
                        blocks.push((current_style.clone(), current_text.trim().into()));
                    }
                    current_text = String::from("• ");
                    last_space = false;
                }
                TagKind::CodeOpen => {
                    current_style = LineStyle::Code;
                }
                TagKind::CodeClose => {
                    if !current_text.trim().is_empty() {
                        blocks.push((LineStyle::Code, current_text.trim().into()));
                    }
                    current_text = String::new();
                    current_style = LineStyle::Body;
                    last_space = true;
                }
                TagKind::LineBreak => {
                    if !current_text.trim().is_empty() {
                        blocks.push((current_style.clone(), current_text.trim().into()));
                    }
                    current_text = String::new();
                    last_space = true;
                }
            },
        }
    }
    if !current_text.trim().is_empty() {
        blocks.push((current_style, current_text.trim().into()));
    }

    // Word-wrap each block to the target width.
    let mut out = Vec::new();
    for (style, text) in blocks {
        for wrapped in wrap_text(&text, wrap) {
            out.push(RenderedLine {
                text: wrapped,
                style: style.clone(),
            });
        }
        // Blank line between blocks for readability.
        if matches!(style, LineStyle::Heading) {
            out.push(RenderedLine {
                text: String::new(),
                style: LineStyle::Body,
            });
        }
    }
    if out.is_empty() {
        out.push(RenderedLine {
            text: String::from("(empty page)"),
            style: LineStyle::Body,
        });
    }
    out
}

fn push_text_token(out: &mut Vec<HtmlToken>, s: &str) {
    if !s.is_empty() {
        out.push(HtmlToken::Text(String::from(s)));
    }
}

enum HtmlToken {
    Text(String),
    Tag(TagKind),
}

enum TagKind {
    ParagraphBreak,
    Heading,
    EndHeading,
    LinkOpen,
    LinkClose,
    ListItem,
    CodeOpen,
    CodeClose,
    LineBreak,
}

fn classify_tag(tag: &str) -> Option<TagKind> {
    let trimmed = tag.trim();
    let name = trimmed
        .split(|c: char| c.is_whitespace() || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match name.as_str() {
        "p" | "div" | "br/" | "hr" | "tr" | "table" | "section" | "article" => {
            Some(TagKind::ParagraphBreak)
        }
        "br" => Some(TagKind::LineBreak),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(TagKind::Heading),
        "/h1" | "/h2" | "/h3" | "/h4" | "/h5" | "/h6" => Some(TagKind::EndHeading),
        "a" => Some(TagKind::LinkOpen),
        "/a" => Some(TagKind::LinkClose),
        "li" => Some(TagKind::ListItem),
        "code" | "pre" => Some(TagKind::CodeOpen),
        "/code" | "/pre" => Some(TagKind::CodeClose),
        "/p" | "/div" | "/tr" | "/section" | "/article" => Some(TagKind::ParagraphBreak),
        _ => None,
    }
}

fn strip_script_and_style(html: &str) -> String {
    let lower: String = html.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest_lower = &lower[i..];
        if rest_lower.starts_with("<script") {
            if let Some(end) = lower[i..].find("</script>") {
                i += end + "</script>".len();
                continue;
            } else {
                break;
            }
        }
        if rest_lower.starts_with("<style") {
            if let Some(end) = lower[i..].find("</style>") {
                i += end + "</style>".len();
                continue;
            } else {
                break;
            }
        }
        if rest_lower.starts_with("<!--") {
            if let Some(end) = lower[i..].find("-->") {
                i += end + "-->".len();
                continue;
            } else {
                break;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars().peekable();
    while let Some(c) = iter.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut entity = String::new();
        let mut closed = false;
        for _ in 0..8 {
            match iter.next() {
                Some(';') => {
                    closed = true;
                    break;
                }
                Some(ec) => entity.push(ec),
                None => break,
            }
        }
        if !closed {
            out.push('&');
            out.push_str(&entity);
            continue;
        }
        let decoded = match entity.as_str() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            "mdash" => "—",
            "ndash" => "–",
            "hellip" => "…",
            _ => "",
        };
        if !decoded.is_empty() {
            out.push_str(decoded);
        } else if let Some(rest) = entity.strip_prefix('#') {
            let n = if let Some(hex) = rest.strip_prefix('x').or_else(|| rest.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                rest.parse::<u32>().ok()
            };
            if let Some(code) = n.and_then(char::from_u32) {
                out.push(code);
            }
        }
    }
    out
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
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
        lines.push(String::new());
    }
    lines
}
