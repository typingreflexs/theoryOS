//! Network connection manager.
//!
//! Real Wi-Fi 802.11 hardware is not present in QEMU and no in-tree driver
//! exists yet for physical adapters. This manager handles the connections we
//! *do* have — wired Ethernet via e1000/virtio — with a real state machine,
//! per-link statistics, and a connect/disconnect interface that the desktop
//! Network app drives. When an 802.11 driver is added, additional
//! `LinkKind::Wireless` entries will appear in the scan.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::addr::{Ipv4Addr, MacAddr};
use super::device::{self, NetStats};
use super::dhcp;
use super::dns;
use super::drivers;

const DHCP_TIMEOUT_NS: u64 = 15_000_000_000;
const SCAN_TTL_NS: u64 = 5_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Wired,
    Wireless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    LinkUp,
    Acquiring,
    Connected,
    Failed,
}

impl ConnState {
    pub fn label(self) -> &'static str {
        match self {
            ConnState::Disconnected => "disconnected",
            ConnState::LinkUp => "link up",
            ConnState::Acquiring => "requesting address",
            ConnState::Connected => "connected",
            ConnState::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LinkInfo {
    pub kind: LinkKind,
    pub label: String,
    pub driver: String,
    pub state: ConnState,
    pub mac: Option<MacAddr>,
    pub ip: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
    pub stats: NetStats,
    pub lease_age_ms: u64,
    pub signal_dbm: Option<i8>,
}

struct Manager {
    state: ConnState,
    last_scan_ns: u64,
    connect_started_ns: u64,
    error: Option<&'static str>,
}

static MANAGER: Mutex<Manager> = Mutex::new(Manager {
    state: ConnState::Disconnected,
    last_scan_ns: 0,
    connect_started_ns: 0,
    error: None,
});

pub fn init() {}

/// Recompute the manager state from underlying NIC + DHCP state.
fn refresh() {
    let mut mgr = MANAGER.lock();
    let now = crate::sched::timer::monotonic_ns();

    if !device::has_device() {
        mgr.state = ConnState::Disconnected;
        return;
    }

    if dhcp::leased_ip().is_some() {
        mgr.state = ConnState::Connected;
        mgr.error = None;
        return;
    }

    if dhcp::is_requesting() {
        if mgr.state != ConnState::Acquiring {
            mgr.state = ConnState::Acquiring;
            mgr.connect_started_ns = now;
        }
        if now.saturating_sub(mgr.connect_started_ns) > DHCP_TIMEOUT_NS {
            mgr.state = ConnState::Failed;
            mgr.error = Some("DHCP timed out — no response from server");
        }
        return;
    }

    if mgr.state == ConnState::Acquiring {
        mgr.state = ConnState::LinkUp;
    }
    if mgr.state == ConnState::Disconnected {
        mgr.state = ConnState::LinkUp;
    }
}

/// User action: probe NIC if needed and start a DHCP lease cycle.
pub fn connect() {
    if !device::has_device() {
        if !drivers::probe() {
            let mut mgr = MANAGER.lock();
            mgr.state = ConnState::Failed;
            mgr.error = Some("No network adapter detected");
            return;
        }
    }
    device::reset_stats();
    {
        let mut mgr = MANAGER.lock();
        mgr.state = ConnState::Acquiring;
        mgr.connect_started_ns = crate::sched::timer::monotonic_ns();
        mgr.error = None;
    }
    dhcp::release();
    dhcp::start();
}

/// User action: release the lease and mark the link disconnected.
pub fn disconnect() {
    dhcp::release();
    let mut mgr = MANAGER.lock();
    mgr.state = ConnState::Disconnected;
    mgr.error = None;
}

/// Force a re-scan of available links.
pub fn rescan() {
    MANAGER.lock().last_scan_ns = crate::sched::timer::monotonic_ns();
    if !device::has_device() {
        drivers::probe();
    }
}

pub fn state() -> ConnState {
    refresh();
    MANAGER.lock().state
}

pub fn last_error() -> Option<&'static str> {
    MANAGER.lock().error
}

pub fn is_online() -> bool {
    device::has_device() && dhcp::leased_ip().is_some()
}

/// All currently visible network links.
pub fn links() -> Vec<LinkInfo> {
    refresh();
    let state = MANAGER.lock().state;
    let mut out = Vec::new();
    if device::has_device() {
        let driver = device::name().unwrap_or("unknown");
        let label = match driver {
            "e1000" => String::from("Ethernet (e1000)"),
            "virtio-net" => String::from("Ethernet (virtio-net)"),
            other => format!("Ethernet ({})", other),
        };
        out.push(LinkInfo {
            kind: LinkKind::Wired,
            label,
            driver: String::from(driver),
            state,
            mac: device::mac(),
            ip: dhcp::leased_ip(),
            gateway: dhcp::gateway(),
            dns: dhcp::dns_server(),
            stats: device::stats(),
            lease_age_ms: dhcp::lease_age_ns().unwrap_or(0) / 1_000_000,
            signal_dbm: None,
        });
    }
    out
}

/// Compact one-line status used by the Settings app and the shell.
pub fn status_line() -> String {
    refresh();
    if let Some(ip) = dhcp::leased_ip() {
        return format!(
            "Online — Ethernet {}.{}.{}.{}",
            ip.0[0], ip.0[1], ip.0[2], ip.0[3]
        );
    }
    match state() {
        ConnState::Acquiring => String::from("Requesting IP address..."),
        ConnState::LinkUp => String::from("Link up — not configured"),
        ConnState::Failed => String::from("Connection failed"),
        ConnState::Disconnected | ConnState::Connected => {
            String::from("Offline — open Network to connect")
        }
    }
}

pub fn ethernet_line() -> String {
    if let Some(ip) = dhcp::leased_ip() {
        format!(
            "Ethernet: {}.{}.{}.{} (DHCP)",
            ip.0[0], ip.0[1], ip.0[2], ip.0[3]
        )
    } else if device::has_device() {
        String::from("Ethernet: link up, no address")
    } else {
        String::from("Ethernet: not connected")
    }
}

pub fn wifi_line() -> &'static str {
    "Wi-Fi: no 802.11 adapter detected"
}

pub fn sysfs_state() -> String {
    if is_online() {
        String::from("up\nethernet\n")
    } else {
        String::from("down\nno link\n")
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{}.{:02} GiB", bytes / GIB, (bytes % GIB) * 100 / GIB)
    } else if bytes >= MIB {
        format!("{}.{:02} MiB", bytes / MIB, (bytes % MIB) * 100 / MIB)
    } else if bytes >= KIB {
        format!("{}.{:02} KiB", bytes / KIB, (bytes % KIB) * 100 / KIB)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_mac(mac: MacAddr) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5]
    )
}

pub fn format_ip(ip: Ipv4Addr) -> String {
    format!("{}.{}.{}.{}", ip.0[0], ip.0[1], ip.0[2], ip.0[3])
}

/// Used by older Settings code paths; kept for compatibility.
pub fn scan_results() -> Vec<String> {
    let mut v = Vec::new();
    for link in links() {
        v.push(link.label);
    }
    if v.is_empty() {
        v.push(String::from("(no network adapters found)"));
    }
    v
}

pub fn dns_status() -> String {
    match dns::current_server() {
        Some(s) => format!("DNS: {}.{}.{}.{}", s.0[0], s.0[1], s.0[2], s.0[3]),
        None => String::from("DNS: not configured"),
    }
}

#[allow(dead_code)]
fn _scan_ttl_alive(now_ns: u64, last_ns: u64) -> bool {
    now_ns.saturating_sub(last_ns) < SCAN_TTL_NS
}
