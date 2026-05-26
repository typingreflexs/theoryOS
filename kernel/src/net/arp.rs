//! ARP cache and request/reply handling.

use alloc::collections::BTreeMap;
use spin::Mutex;

use super::addr::{Ipv4Addr, MacAddr};
use super::buffer::PacketBuf;
use super::device;
use super::eth::{EthHeader, ETHERTYPE_ARP, ETH_HDR_LEN};

const ARP_HW_ETH: u16 = 1;
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

#[repr(C, packed)]
struct ArpPacket {
    hw_type: u16,
    proto_type: u16,
    hw_len: u8,
    proto_len: u8,
    opcode: u16,
    sender_mac: [u8; 6],
    sender_ip: [u8; 4],
    target_mac: [u8; 6],
    target_ip: [u8; 4],
}

static CACHE: Mutex<BTreeMap<Ipv4Addr, MacAddr>> = Mutex::new(BTreeMap::new());
static OUR_IP: Mutex<Ipv4Addr> = Mutex::new(Ipv4Addr::ANY);

pub fn init() {}

pub fn set_ip(ip: Ipv4Addr) {
    *OUR_IP.lock() = ip;
}

pub fn our_ip() -> Ipv4Addr {
    *OUR_IP.lock()
}

pub fn lookup(ip: Ipv4Addr) -> Option<MacAddr> {
    CACHE.lock().get(&ip).copied()
}

pub fn resolve(ip: Ipv4Addr) -> Option<MacAddr> {
    if let Some(mac) = lookup(ip) {
        return Some(mac);
    }
    send_request(ip);
    None
}

pub fn handle_incoming(eth: &EthHeader, payload: &[u8]) {
    if payload.len() < core::mem::size_of::<ArpPacket>() {
        return;
    }
    let arp = unsafe { &*(payload.as_ptr() as *const ArpPacket) };
    if arp.hw_type != ARP_HW_ETH.to_be() || arp.proto_type != super::eth::ETHERTYPE_IPV4.to_be() {
        return;
    }
    let sender_ip = Ipv4Addr(arp.sender_ip);
    let sender_mac = MacAddr(arp.sender_mac);
    CACHE.lock().insert(sender_ip, sender_mac);

    let opcode = u16::from_be(arp.opcode);
    if opcode == ARP_OP_REQUEST {
        let target_ip = Ipv4Addr(arp.target_ip);
        if target_ip == our_ip() {
            send_reply(sender_ip, sender_mac);
        }
    }
}

fn send_request(target: Ipv4Addr) {
    let Some(our_mac) = device::mac() else { return };
    let mut payload = [0u8; 28];
    let arp = ArpPacket {
        hw_type: ARP_HW_ETH.to_be(),
        proto_type: ETHERTYPE_ARP.to_be(),
        hw_len: 6,
        proto_len: 4,
        opcode: ARP_OP_REQUEST.to_be(),
        sender_mac: our_mac.0,
        sender_ip: our_ip().0,
        target_mac: [0; 6],
        target_ip: target.0,
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            &arp as *const ArpPacket as *const u8,
            payload.as_mut_ptr(),
            core::mem::size_of::<ArpPacket>(),
        );
    }
    transmit(MacAddr::BROADCAST, payload);
}

fn send_reply(target_ip: Ipv4Addr, target_mac: MacAddr) {
    let Some(our_mac) = device::mac() else { return };
    let mut payload = [0u8; 28];
    let arp = ArpPacket {
        hw_type: ARP_HW_ETH.to_be(),
        proto_type: ETHERTYPE_ARP.to_be(),
        hw_len: 6,
        proto_len: 4,
        opcode: ARP_OP_REPLY.to_be(),
        sender_mac: our_mac.0,
        sender_ip: our_ip().0,
        target_mac: target_mac.0,
        target_ip: target_ip.0,
    };
    unsafe {
        core::ptr::copy_nonoverlapping(
            &arp as *const ArpPacket as *const u8,
            payload.as_mut_ptr(),
            core::mem::size_of::<ArpPacket>(),
        );
    }
    transmit(target_mac, payload);
}

fn transmit(dst: MacAddr, arp_payload: [u8; 28]) {
    let Some(src) = device::mac() else { return };
    let mut pkt = PacketBuf::new();
    EthHeader::build(dst, src, ETHERTYPE_ARP, &arp_payload, &mut pkt);
    let _ = device::send(&pkt);
}

pub fn eth_dst_for(ip: Ipv4Addr) -> MacAddr {
    if ip.to_u32().to_be() == 0xFFFFFFFF {
        return MacAddr::BROADCAST;
    }
    let target = route_target(ip);
    resolve_blocking(target).unwrap_or(MacAddr::BROADCAST)
}

pub fn route_target(ip: Ipv4Addr) -> Ipv4Addr {
    let our = our_ip();
    if our.is_unspecified() {
        return ip;
    }
    if same_subnet(our, ip) {
        return ip;
    }
    super::dhcp::gateway().unwrap_or(ip)
}

fn same_subnet(a: Ipv4Addr, b: Ipv4Addr) -> bool {
    a.0[0] == b.0[0] && a.0[1] == b.0[1] && a.0[2] == b.0[2]
}

pub fn resolve_blocking(ip: Ipv4Addr) -> Option<MacAddr> {
    if let Some(mac) = lookup(ip) {
        return Some(mac);
    }
    send_request(ip);
    for _ in 0..20_000 {
        super::rx_poll();
        if let Some(mac) = lookup(ip) {
            return Some(mac);
        }
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
    None
}
