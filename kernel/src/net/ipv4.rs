//! IPv4 layer — header parsing, checksum, dispatch.

use super::addr::Ipv4Addr;
use super::buffer::PacketBuf;
use super::eth::EthHeader;
use super::{arp, icmp, tcp, udp};

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_TCP: u8 = 6;
pub const IP_PROTO_UDP: u8 = 17;

#[derive(Clone, Copy, Debug)]
pub struct Ipv4Header {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    pub ttl: u8,
    pub total_len: usize,
    pub header_len: usize,
}

pub fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

pub fn parse(data: &[u8]) -> Option<(Ipv4Header, usize)> {
    if data.len() < 20 {
        return None;
    }
    let ver_ihl = data[0];
    let version = ver_ihl >> 4;
    if version != 4 {
        return None;
    }
    let ihl = (ver_ihl & 0x0F) as usize * 4;
    if data.len() < ihl {
        return None;
    }
    let total_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let mut hdr_copy = [0u8; 20];
    hdr_copy[..ihl.min(20)].copy_from_slice(&data[..ihl.min(20)]);
    hdr_copy[10] = 0;
    hdr_copy[11] = 0;
    let csum = u16::from_be_bytes([data[10], data[11]]);
    if csum != 0 && checksum(&data[..ihl]) != csum {
        return None;
    }
    Some((
        Ipv4Header {
            src: Ipv4Addr([data[12], data[13], data[14], data[15]]),
            dst: Ipv4Addr([data[16], data[17], data[18], data[19]]),
            protocol: data[9],
            ttl: data[8],
            total_len,
            header_len: ihl,
        },
        ihl,
    ))
}

pub fn build(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, payload: &[u8], out: &mut [u8]) -> usize {
    let total = 20 + payload.len();
    out[0] = 0x45;
    out[1] = 0;
    out[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    out[4..6].copy_from_slice(&[0, 0]);
    out[6] = 0x40;
    out[7] = 0;
    out[8] = 64;
    out[9] = protocol;
    out[10] = 0;
    out[11] = 0;
    out[12..16].copy_from_slice(&src.0);
    out[16..20].copy_from_slice(&dst.0);
    let csum = checksum(&out[..20]);
    out[10..12].copy_from_slice(&csum.to_be_bytes());
    out[20..total].copy_from_slice(payload);
    total
}

pub fn handle_incoming(eth: &EthHeader, payload: &[u8]) {
    let Some((hdr, offset)) = parse(payload) else {
        return;
    };
    if hdr.dst != arp::our_ip() && hdr.dst != Ipv4Addr::BROADCAST {
        return;
    }
    let data = &payload[offset..hdr.total_len.min(payload.len())];
    match hdr.protocol {
        IP_PROTO_ICMP => icmp::handle_incoming(&hdr, data),
        IP_PROTO_UDP => udp::handle_incoming(&hdr, data),
        IP_PROTO_TCP => tcp::handle_incoming(&hdr, data),
        _ => {}
    }
}

pub fn transmit(dst: Ipv4Addr, protocol: u8, payload: &[u8]) {
    let src = arp::our_ip();
    let mut ip_buf = [0u8; 1500];
    let ip_len = build(src, dst, protocol, payload, &mut ip_buf);
    let dst_mac = arp::eth_dst_for(dst);
    let Some(src_mac) = super::device::mac() else { return };
    let mut pkt = PacketBuf::new();
    super::eth::EthHeader::build(dst_mac, src_mac, super::eth::ETHERTYPE_IPV4, &ip_buf[..ip_len], &mut pkt);
    let _ = super::device::send(&pkt);
}
