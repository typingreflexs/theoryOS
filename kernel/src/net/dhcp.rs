//! DHCP client — non-blocking discover, offer, request, ack lifecycle.
//!
//! Driven by `tick()` from the idle loop; `start()` kicks off a new lease,
//! `release()` clears state when the user disconnects.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::addr::Ipv4Addr;
use super::arp;
use super::device;
use super::dns;
use super::udp;

const DHCP_SERVER: u16 = 67;
const DHCP_CLIENT: u16 = 68;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_RELEASE: u8 = 7;

const REDISCOVER_INTERVAL_TICKS: u32 = 5000;

static LEASED_IP: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static GATEWAY: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static DNS_SERVER: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static SERVER_ID: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static TICKS: AtomicU32 = AtomicU32::new(0);
static XID: AtomicU32 = AtomicU32::new(0);
static ACTIVE: spin::Mutex<bool> = spin::Mutex::new(false);
static LEASE_AT_NS: AtomicU64 = AtomicU64::new(0);

pub fn leased_ip() -> Option<Ipv4Addr> {
    *LEASED_IP.lock()
}

pub fn gateway() -> Option<Ipv4Addr> {
    *GATEWAY.lock()
}

pub fn dns_server() -> Option<Ipv4Addr> {
    *DNS_SERVER.lock()
}

pub fn server_id() -> Option<Ipv4Addr> {
    *SERVER_ID.lock()
}

pub fn lease_age_ns() -> Option<u64> {
    let at = LEASE_AT_NS.load(Ordering::Relaxed);
    if at == 0 {
        return None;
    }
    let now = crate::sched::timer::monotonic_ns();
    Some(now.saturating_sub(at))
}

pub fn is_requesting() -> bool {
    *ACTIVE.lock() && leased_ip().is_none()
}

/// Begin a new DHCP lease cycle. Idempotent — only sends DISCOVER if no lease.
pub fn start() {
    if leased_ip().is_some() {
        return;
    }
    if !device::has_device() {
        return;
    }
    new_xid();
    send_discover();
    *ACTIVE.lock() = true;
    TICKS.store(0, Ordering::Relaxed);
}

/// Release the current lease and clear all networking state.
pub fn release() {
    if let Some(ip) = leased_ip() {
        send_release(ip);
    }
    *LEASED_IP.lock() = None;
    *GATEWAY.lock() = None;
    *DNS_SERVER.lock() = None;
    *SERVER_ID.lock() = None;
    *ACTIVE.lock() = false;
    LEASE_AT_NS.store(0, Ordering::Relaxed);
    TICKS.store(0, Ordering::Relaxed);
}

pub fn tick() {
    if leased_ip().is_some() {
        *ACTIVE.lock() = false;
        return;
    }
    if !*ACTIVE.lock() {
        return;
    }
    poll();
    let n = TICKS.fetch_add(1, Ordering::Relaxed);
    if n > 0 && n % REDISCOVER_INTERVAL_TICKS == 0 && leased_ip().is_none() {
        new_xid();
        send_discover();
    }
}

pub fn wait_for_lease(max_iters: u32) -> bool {
    if leased_ip().is_some() {
        return true;
    }
    start();
    for _ in 0..max_iters {
        super::rx_poll();
        tick();
        if leased_ip().is_some() {
            crate::console::Console::println("[net] DHCP address configured");
            return true;
        }
        for _ in 0..200 {
            core::hint::spin_loop();
        }
    }
    false
}

fn new_xid() {
    let tsc = crate::arch::x86_64::cpu::rdtsc();
    XID.store(((tsc >> 16) as u32) ^ (tsc as u32), Ordering::Relaxed);
}

fn current_xid() -> u32 {
    XID.load(Ordering::Relaxed)
}

fn build_header(buf: &mut [u8; 300], op: u8) -> usize {
    let mac = device::mac().unwrap_or(super::addr::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]));
    buf[0] = op;
    buf[1] = 1; // htype = Ethernet
    buf[2] = 6; // hlen
    buf[3] = 0; // hops
    let xid = current_xid().to_be_bytes();
    buf[4..8].copy_from_slice(&xid);
    buf[8..10].copy_from_slice(&[0, 0]); // secs
    buf[10..12].copy_from_slice(&[0x80, 0x00]); // flags = broadcast
    // ciaddr/yiaddr/siaddr/giaddr left zero
    buf[28..34].copy_from_slice(&mac.0);
    // Magic cookie
    buf[236..240].copy_from_slice(&[99, 130, 83, 99]);
    240
}

fn send_discover() {
    let mut payload = [0u8; 300];
    let mut p = build_header(&mut payload, 1);
    payload[p] = 53; payload[p + 1] = 1; payload[p + 2] = DHCP_DISCOVER; p += 3;
    payload[p] = 55; payload[p + 1] = 4;
    payload[p + 2] = 1;  // subnet
    payload[p + 3] = 3;  // router
    payload[p + 4] = 6;  // dns
    payload[p + 5] = 51; // lease time
    p += 6;
    payload[p] = 57; payload[p + 1] = 2; payload[p + 2] = 0x02; payload[p + 3] = 0x40; p += 4;
    payload[p] = 255; p += 1;

    udp::bind_port(DHCP_CLIENT);
    let _ = udp::sendto(
        super::addr::IpAddr::V4(Ipv4Addr::BROADCAST),
        DHCP_SERVER,
        DHCP_CLIENT,
        &payload[..p],
    );
}

fn send_request(offered: Ipv4Addr) {
    let mut payload = [0u8; 300];
    let mut p = build_header(&mut payload, 1);
    payload[p] = 53; payload[p + 1] = 1; payload[p + 2] = DHCP_REQUEST; p += 3;
    payload[p] = 50; payload[p + 1] = 4;
    payload[p + 2..p + 6].copy_from_slice(&offered.0);
    p += 6;
    if let Some(sid) = server_id() {
        payload[p] = 54; payload[p + 1] = 4;
        payload[p + 2..p + 6].copy_from_slice(&sid.0);
        p += 6;
    }
    payload[p] = 255; p += 1;
    let _ = udp::sendto(
        super::addr::IpAddr::V4(Ipv4Addr::BROADCAST),
        DHCP_SERVER,
        DHCP_CLIENT,
        &payload[..p],
    );
}

fn send_release(ip: Ipv4Addr) {
    let mut payload = [0u8; 300];
    let mut p = build_header(&mut payload, 1);
    // ciaddr = our IP
    payload[12..16].copy_from_slice(&ip.0);
    payload[p] = 53; payload[p + 1] = 1; payload[p + 2] = DHCP_RELEASE; p += 3;
    if let Some(sid) = server_id() {
        payload[p] = 54; payload[p + 1] = 4;
        payload[p + 2..p + 6].copy_from_slice(&sid.0);
        p += 6;
    }
    payload[p] = 255; p += 1;
    let dst = server_id().unwrap_or(Ipv4Addr::BROADCAST);
    let _ = udp::sendto(
        super::addr::IpAddr::V4(dst),
        DHCP_SERVER,
        DHCP_CLIENT,
        &payload[..p],
    );
}

pub fn poll() {
    while let Some(dg) = udp::recv(DHCP_CLIENT) {
        if dg.data.len() < 240 {
            continue;
        }
        // Validate XID matches current transaction.
        if dg.data.len() >= 8 {
            let xid = u32::from_be_bytes([dg.data[4], dg.data[5], dg.data[6], dg.data[7]]);
            if xid != current_xid() {
                continue;
            }
        }
        let yiaddr = Ipv4Addr([dg.data[16], dg.data[17], dg.data[18], dg.data[19]]);
        let mut typ = 0u8;
        let mut i = 240;
        while i + 2 < dg.data.len() && dg.data[i] != 255 {
            let opt = dg.data[i];
            let len = dg.data[i + 1] as usize;
            if i + 2 + len > dg.data.len() {
                break;
            }
            match opt {
                53 if len == 1 => typ = dg.data[i + 2],
                3 if len >= 4 => {
                    *GATEWAY.lock() = Some(Ipv4Addr([
                        dg.data[i + 2], dg.data[i + 3], dg.data[i + 4], dg.data[i + 5],
                    ]));
                }
                6 if len >= 4 => {
                    let server = Ipv4Addr([
                        dg.data[i + 2], dg.data[i + 3], dg.data[i + 4], dg.data[i + 5],
                    ]);
                    *DNS_SERVER.lock() = Some(server);
                    dns::set_server(server);
                }
                54 if len == 4 => {
                    *SERVER_ID.lock() = Some(Ipv4Addr([
                        dg.data[i + 2], dg.data[i + 3], dg.data[i + 4], dg.data[i + 5],
                    ]));
                }
                _ => {}
            }
            i += 2 + len;
        }

        match typ {
            DHCP_OFFER => {
                send_request(yiaddr);
            }
            DHCP_ACK => {
                *LEASED_IP.lock() = Some(yiaddr);
                arp::set_ip(yiaddr);
                LEASE_AT_NS.store(crate::sched::timer::monotonic_ns(), Ordering::Relaxed);
                *ACTIVE.lock() = false;
            }
            _ => {}
        }
    }
}
