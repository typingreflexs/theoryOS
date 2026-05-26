//! DHCP client — non-blocking discover / request.

use core::sync::atomic::{AtomicU32, Ordering};

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

static LEASED_IP: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static GATEWAY: spin::Mutex<Option<Ipv4Addr>> = spin::Mutex::new(None);
static TICKS: AtomicU32 = AtomicU32::new(0);
static ACTIVE: spin::Mutex<bool> = spin::Mutex::new(false);

pub fn leased_ip() -> Option<Ipv4Addr> {
    *LEASED_IP.lock()
}

pub fn gateway() -> Option<Ipv4Addr> {
    *GATEWAY.lock()
}

pub fn start() {
    if leased_ip().is_some() {
        return;
    }
    if !device::has_device() {
        return;
    }
    send_discover();
    *ACTIVE.lock() = true;
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
    if n % 5000 == 0 && leased_ip().is_none() {
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

fn send_discover() {
    let mac = device::mac().unwrap_or(super::addr::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]));
    let mut payload = [0u8; 300];
    payload[0] = 1;
    payload[1] = 1;
    payload[2] = 6;
    payload[3] = 0x12;
    payload[28..34].copy_from_slice(&mac.0);
    let mut p = 236usize;
    payload[p..p + 4].copy_from_slice(&[99, 130, 83, 99]);
    p += 4;
    payload[p] = 53;
    payload[p + 1] = 1;
    payload[p + 2] = DHCP_DISCOVER;
    p += 3;
    payload[p] = 55;
    payload[p + 1] = 3;
    payload[p + 2] = 1;
    payload[p + 3] = 3;
    payload[p + 4] = 6;
    p += 5;
    payload[p] = 255;

    udp::bind_port(DHCP_CLIENT);
    let _ = udp::sendto(
        super::addr::IpAddr::V4(Ipv4Addr::BROADCAST),
        DHCP_SERVER,
        DHCP_CLIENT,
        &payload[..p + 1],
    );
}

pub fn poll() {
    while let Some(dg) = udp::recv(DHCP_CLIENT) {
        if dg.data.len() < 240 {
            continue;
        }
        let mut typ = 0u8;
        let mut i = 240;
        while i + 2 < dg.data.len() && dg.data[i] != 255 {
            let opt = dg.data[i];
            let len = dg.data[i + 1] as usize;
            if opt == 53 && len == 1 {
                typ = dg.data[i + 2];
            }
            if opt == 1 && len == 4 {
                let ip = Ipv4Addr([
                    dg.data[i + 2],
                    dg.data[i + 3],
                    dg.data[i + 4],
                    dg.data[i + 5],
                ]);
                if typ == DHCP_OFFER || typ == DHCP_ACK {
                    *LEASED_IP.lock() = Some(ip);
                    arp::set_ip(ip);
                    if typ == DHCP_OFFER {
                        send_request(ip);
                    }
                }
            }
            if opt == 3 && len == 4 {
                *GATEWAY.lock() = Some(Ipv4Addr([
                    dg.data[i + 2],
                    dg.data[i + 3],
                    dg.data[i + 4],
                    dg.data[i + 5],
                ]));
            }
            if opt == 6 && len >= 4 {
                dns::set_server(Ipv4Addr([
                    dg.data[i + 2],
                    dg.data[i + 3],
                    dg.data[i + 4],
                    dg.data[i + 5],
                ]));
            }
            i += 2 + len;
        }
    }
}

fn send_request(offered: Ipv4Addr) {
    let mac = device::mac().unwrap_or(super::addr::MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]));
    let mut payload = [0u8; 300];
    payload[0] = 1;
    payload[1] = 1;
    payload[2] = 6;
    payload[28..34].copy_from_slice(&mac.0);
    payload[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut p = 240;
    payload[p] = 53;
    payload[p + 1] = 1;
    payload[p + 2] = DHCP_REQUEST;
    p += 3;
    payload[p] = 50;
    payload[p + 1] = 4;
    payload[p + 2..p + 6].copy_from_slice(&offered.0);
    p += 6;
    payload[p] = 255;
    let _ = udp::sendto(
        super::addr::IpAddr::V4(Ipv4Addr::BROADCAST),
        DHCP_SERVER,
        DHCP_CLIENT,
        &payload[..p + 1],
    );
}
