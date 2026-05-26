//! ICMP and ICMPv6 echo handling.

use super::addr::{Ipv4Addr, Ipv6Addr};
use super::ipv4::{self, Ipv4Header, IP_PROTO_ICMP};
use super::ipv6::{self, Ipv6Header};

pub const ICMP_ECHO: u8 = 8;
pub const ICMP_ECHOREPLY: u8 = 0;

pub fn handle_incoming(ip: &Ipv4Header, data: &[u8]) {
    if data.len() < 8 {
        return;
    }
    let typ = data[0];
    if typ == ICMP_ECHO {
        let mut reply = [0u8; 576];
        reply[..data.len()].copy_from_slice(data);
        reply[0] = ICMP_ECHOREPLY;
        reply[2] = 0;
        reply[3] = 0;
        let csum = ipv4::checksum(&reply[..data.len()]);
        reply[2..4].copy_from_slice(&csum.to_be_bytes());
        ipv4::transmit(ip.src, IP_PROTO_ICMP, &reply[..data.len()]);
    }
}

pub fn handle_icmpv6(_ip: &Ipv6Header, _data: &[u8]) {
    // ICMPv6 neighbor discovery / echo — stub
}

pub fn ping(ip: Ipv4Addr) {
    let mut payload = [0u8; 64];
    payload[0] = ICMP_ECHO;
    payload[1] = 0;
    payload[2] = 0;
    payload[3] = 0;
    payload[4] = 0;
    payload[5] = 1;
    payload[6..8].copy_from_slice(&1u16.to_be_bytes());
    for i in 8..payload.len() {
        payload[i] = i as u8;
    }
    let csum = ipv4::checksum(&payload);
    payload[2..4].copy_from_slice(&csum.to_be_bytes());
    ipv4::transmit(ip, IP_PROTO_ICMP, &payload);
}
