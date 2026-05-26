//! IPv6 layer — basic parsing and dispatch stub.

use super::addr::Ipv6Addr;
use super::eth::EthHeader;
use super::icmp;

pub const IP6_PROTO_ICMPV6: u8 = 58;
pub const IP6_PROTO_UDP: u8 = 17;
pub const IP6_PROTO_TCP: u8 = 6;

#[derive(Clone, Copy, Debug)]
pub struct Ipv6Header {
    pub src: Ipv6Addr,
    pub dst: Ipv6Addr,
    pub next_header: u8,
    pub payload_len: u16,
}

pub fn parse(data: &[u8]) -> Option<(Ipv6Header, usize)> {
    if data.len() < 40 {
        return None;
    }
    let ver = data[0] >> 4;
    if ver != 6 {
        return None;
    }
    let payload_len = u16::from_be_bytes([data[4], data[5]]);
    let next_header = data[6];
    let mut src = [0u8; 16];
    let mut dst = [0u8; 16];
    src.copy_from_slice(&data[8..24]);
    dst.copy_from_slice(&data[24..40]);
    Some((
        Ipv6Header {
            src: Ipv6Addr(src),
            dst: Ipv6Addr(dst),
            next_header,
            payload_len,
        },
        40,
    ))
}

pub fn handle_incoming(_eth: &EthHeader, payload: &[u8]) {
    let Some((hdr, offset)) = parse(payload) else {
        return;
    };
    let data = &payload[offset..offset + hdr.payload_len as usize];
    match hdr.next_header {
        IP6_PROTO_ICMPV6 => icmp::handle_icmpv6(&hdr, data),
        IP6_PROTO_UDP => super::udp::handle_incoming_v6(&hdr, data),
        IP6_PROTO_TCP => super::tcp::handle_incoming_v6(&hdr, data),
        _ => {}
    }
}

pub fn transmit(_dst: Ipv6Addr, _protocol: u8, _payload: &[u8]) {
    // Full IPv6 TX requires neighbor discovery; stub for now.
}
