//! DNS resolver — UDP queries to port 53.

use alloc::vec::Vec;

use super::addr::{IpAddr, Ipv4Addr};
use super::udp::{self, UdpDatagram};

const DNS_PORT: u16 = 53;
static DNS_SERVER: spin::Mutex<Ipv4Addr> = spin::Mutex::new(Ipv4Addr([8, 8, 8, 8]));

pub fn set_server(ip: Ipv4Addr) {
    *DNS_SERVER.lock() = ip;
}

static DNS_QUERY: spin::Mutex<Option<(u16, alloc::vec::Vec<u8>)>> = spin::Mutex::new(None);

pub fn reset_query() {
    *DNS_QUERY.lock() = None;
}

fn start_query(name: &str) -> u16 {
    let mut query = Vec::new();
    query.extend_from_slice(&[0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&[0, 1, 0, 1]);

    let src_port = 53000u16;
    udp::bind_port(src_port);
    let server = *DNS_SERVER.lock();
    let _ = udp::sendto(IpAddr::V4(server), DNS_PORT, src_port, &query);
    *DNS_QUERY.lock() = Some((src_port, query));
    src_port
}

fn parse_reply(reply: &UdpDatagram) -> Option<Ipv4Addr> {
    if reply.data.len() <= 12 {
        return None;
    }
    let mut i = 12;
    while i < reply.data.len() && reply.data[i] != 0 {
        i += 1 + reply.data[i] as usize;
    }
    i += 5;
    if i + 10 < reply.data.len() && reply.data[i + 1] == 1 {
        return Some(Ipv4Addr([
            reply.data[i + 8],
            reply.data[i + 9],
            reply.data[i + 10],
            reply.data[i + 11],
        ]));
    }
    None
}

/// One DNS poll step — call repeatedly from the HTTP state machine.
pub fn resolve_step(name: &str) -> Option<Ipv4Addr> {
    let port = if DNS_QUERY.lock().is_none() {
        start_query(name)
    } else {
        DNS_QUERY.lock().as_ref().map(|(p, _)| *p).unwrap_or(53000)
    };

    super::rx_poll();
    super::dhcp::tick();

    if let Some(reply) = udp::recv(port) {
        *DNS_QUERY.lock() = None;
        return parse_reply(&reply);
    }
    None
}

pub fn resolve(name: &str) -> Option<Ipv4Addr> {
    *DNS_QUERY.lock() = None;
    start_query(name);
    for _ in 0..15_000 {
        if let Some(ip) = resolve_step(name) {
            return Some(ip);
        }
    }
    *DNS_QUERY.lock() = None;
    None
}
