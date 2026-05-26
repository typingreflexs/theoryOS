use spin::Mutex;

use crate::sched::timer;

const MAX_LINES: usize = 48;
const LINE_CAP: usize = 96;
const INPUT_CAP: usize = 120;

pub(crate) struct Terminal {
    lines: [[u8; LINE_CAP]; MAX_LINES],
    line_len: [u8; MAX_LINES],
    line_count: usize,
    input: [u8; INPUT_CAP],
    input_len: usize,
    dirty: bool,
}

impl Terminal {
    const fn new() -> Self {
        Self {
            lines: [[0; LINE_CAP]; MAX_LINES],
            line_len: [0; MAX_LINES],
            line_count: 0,
            input: [0; INPUT_CAP],
            input_len: 0,
            dirty: true,
        }
    }

    fn push_line(&mut self, text: &[u8]) {
        if self.line_count >= MAX_LINES {
            for i in 1..MAX_LINES {
                self.lines[i - 1] = self.lines[i];
                self.line_len[i - 1] = self.line_len[i];
            }
            self.line_count = MAX_LINES - 1;
        }
        let idx = self.line_count;
        let len = text.len().min(LINE_CAP);
        self.lines[idx][..len].copy_from_slice(&text[..len]);
        self.line_len[idx] = len as u8;
        self.line_count += 1;
        self.dirty = true;
    }

    fn push_str(&mut self, s: &str) {
        self.push_line(s.as_bytes());
    }

    fn clear_screen(&mut self) {
        self.line_count = 0;
        self.dirty = true;
    }

    pub(crate) fn line(&self, idx: usize) -> Option<&str> {
        if idx >= self.line_count {
            return None;
        }
        let len = self.line_len[idx] as usize;
        core::str::from_utf8(&self.lines[idx][..len]).ok()
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_count
    }

    pub(crate) fn input_text(&self) -> &str {
        core::str::from_utf8(&self.input[..self.input_len]).unwrap_or("")
    }
}

static TERM: Mutex<Terminal> = Mutex::new(Terminal::new());

pub fn init() {
    let mut term = TERM.lock();
    term.push_str("Theory OS shell — type 'help' for commands.");
    term.dirty = true;
}

pub fn take_dirty() -> bool {
    let mut term = TERM.lock();
    let dirty = term.dirty;
    term.dirty = false;
    dirty
}

pub fn force_redraw() {
    TERM.lock().dirty = true;
}

pub fn handle_key(key: u8) {
    let mut term = TERM.lock();
    match key {
        b'\n' => {
            let mut cmd_buf = [0u8; INPUT_CAP + 8];
            let prefix = b"$ ";
            cmd_buf[..2].copy_from_slice(prefix);
            cmd_buf[2..2 + term.input_len].copy_from_slice(&term.input[..term.input_len]);
            let echo_len = 2 + term.input_len;
            term.push_line(&cmd_buf[..echo_len]);

            let cmd_len = term.input_len;
            let mut cmd_storage = [0u8; INPUT_CAP];
            cmd_storage[..cmd_len].copy_from_slice(&term.input[..cmd_len]);
            let cmd = core::str::from_utf8(&cmd_storage[..cmd_len])
                .unwrap_or("")
                .trim();
            run_command(cmd, &mut term);

            term.input_len = 0;
            term.dirty = true;
        }
        0x08 => {
            if term.input_len > 0 {
                term.input_len -= 1;
                term.dirty = true;
            }
        }
        c if c >= 0x20 && c <= 0x7E => {
            if term.input_len + 1 < INPUT_CAP {
                let i = term.input_len;
                term.input[i] = c;
                term.input_len += 1;
                term.dirty = true;
            }
        }
        _ => {}
    }
}

fn run_command(cmd: &str, term: &mut Terminal) {
    if cmd.is_empty() {
        return;
    }
    let (name, args) = split_cmd(cmd);
    match name {
        "help" => {
            term.push_str("Commands:");
            term.push_str("  help     — this list");
            term.push_str("  whoami   — current user");
            term.push_str("  clear    — clear console");
            term.push_str("  echo     — print text");
            term.push_str("  pwd      — working directory");
            term.push_str("  ls       — list root entries");
            term.push_str("  uname    — system name");
            term.push_str("  date     — date and time");
            term.push_str("  version  — OS version");
            term.push_str("  open     — open app (settings/browser/files/wifi)");
            term.push_str("  apps     — open Start menu");
            term.push_str("  fetch    — download http:// page");
            term.push_str("  ping     — ping host");
            term.push_str("  wifi     — network status");
        }
        "whoami" => term.push_str("root"),
        "clear" => term.clear_screen(),
        "echo" => {
            if args.is_empty() {
                term.push_str("");
            } else {
                term.push_str(args);
            }
        }
        "pwd" => term.push_str("/"),
        "ls" => {
            term.push_str("bin  dev  etc  lib  proc  sys  tmp");
        }
        "uname" => term.push_str("Theory OS x86_64"),
        "date" | "time" => {
            let mut buf = [0u8; 32];
            let text = format_datetime(&mut buf);
            term.push_line(text.as_bytes());
        }
        "version" => term.push_str("Theory OS 0.1.0 (Limine + Rust kernel)"),
        "apps" | "start" | "menu" => {
            crate::video::apps::toggle_launcher();
            crate::video::apps::mark_dirty();
        }
        "wifi" | "net" => {
            crate::video::apps::init_network_once();
            let msg = crate::net::wifi::status_line();
            term.push_str(&msg);
        }
        "fetch" => {
            if args.is_empty() {
                term.push_str("Usage: fetch http://example.com");
            } else {
                crate::video::apps::init_network_once();
                for line in crate::net::http::fetch_lines(args) {
                    term.push_str(&line);
                }
            }
        }
        "ping" => {
            if args.is_empty() {
                term.push_str("Usage: ping 10.0.2.2");
            } else if crate::net::ensure_online() {
                if let Ok(octets) = parse_ping_ip(args) {
                    crate::net::icmp::ping(crate::net::addr::Ipv4Addr::new(
                        octets[0], octets[1], octets[2], octets[3],
                    ));
                    for _ in 0..5000 {
                        crate::net::rx_poll();
                    }
                    term.push_str("Ping sent.");
                } else {
                    term.push_str("Invalid IP.");
                }
            } else {
                term.push_str("Network offline.");
            }
        }
        "open" => {
            if args.is_empty() {
                term.push_str("Usage: open settings|browser|files|wifi|console");
            } else if crate::video::apps::shell_open(args) {
                term.push_str("Opened app.");
            } else {
                term.push_str("Unknown app. Try: settings browser files wifi");
            }
        }
        _ => {
            term.push_str("Unknown command. Type 'help'.");
        }
    }
}

fn split_cmd(cmd: &str) -> (&str, &str) {
    let cmd = cmd.trim();
    match cmd.find(char::is_whitespace) {
        Some(i) => (&cmd[..i], cmd[i..].trim()),
        None => (cmd, ""),
    }
}

fn parse_ping_ip(s: &str) -> Result<[u8; 4], ()> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return Err(());
        }
        let mut n = 0u16;
        if part.is_empty() {
            return Err(());
        }
        for b in part.bytes() {
            if !b.is_ascii_digit() {
                return Err(());
            }
            n = n * 10 + (b - b'0') as u16;
            if n > 255 {
                return Err(());
            }
        }
        parts[idx] = n as u8;
        idx += 1;
    }
    if idx != 4 {
        return Err(());
    }
    Ok(parts)
}

fn format_datetime(buf: &mut [u8; 32]) -> &str {
    let ns = timer::monotonic_ns();
    let total_sec = ns / 1_000_000_000;
    let days = total_sec / 86400;
    let h = (total_sec / 3600) % 24;
    let m = (total_sec / 60) % 60;
    let s = total_sec % 60;
    write_uptime(buf, days, h, m, s)
}

fn write_uptime(buf: &mut [u8; 32], days: u64, h: u64, m: u64, s: u64) -> &str {
    let mut i = 0usize;
    i += write_str(&mut buf[i..], b"Uptime ");
    i += write_u64(&mut buf[i..], days);
    i += write_str(&mut buf[i..], b"d ");
    i += write_two(&mut buf[i..], h as u8);
    buf[i] = b':';
    i += 1;
    i += write_two(&mut buf[i..], m as u8);
    buf[i] = b':';
    i += 1;
    i += write_two(&mut buf[i..], s as u8);
    unsafe { core::str::from_utf8_unchecked(&buf[..i]) }
}

fn write_str(dst: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
}

fn write_u64(dst: &mut [u8], mut n: u64) -> usize {
    if n == 0 {
        if !dst.is_empty() {
            dst[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut tmp = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for j in 0..len {
        dst[j] = tmp[len - 1 - j];
    }
    len
}

fn write_two(dst: &mut [u8], v: u8) -> usize {
    if dst.len() < 2 {
        return 0;
    }
    dst[0] = b'0' + (v / 10);
    dst[1] = b'0' + (v % 10);
    2
}

pub fn with_terminal<F, R>(f: F) -> R
where
    F: FnOnce(&Terminal) -> R,
{
    f(&TERM.lock())
}
