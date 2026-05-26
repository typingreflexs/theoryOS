//! UDP — datagram sockets and protocol handler.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use spin::Mutex;

use super::addr::{IpAddr, Ipv4Addr};
use super::ipv4::{self, Ipv4Header, IP_PROTO_UDP};
use super::ipv6::{Ipv6Header, IP6_PROTO_UDP};

#[derive(Clone, Debug)]
pub struct UdpDatagram {
    pub src: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UdpEndpoint {
    pub addr: IpAddr,
    pub port: u16,
}

static RX_QUEUES: Mutex<[VecDeque<UdpDatagram>; 64]> = Mutex::new([const { VecDeque::new() }; 64]);

pub fn init() {}

fn port_index(port: u16) -> usize {
    (port as usize) % RX_QUEUES.lock().len()
}

pub fn bind_port(port: u16) {}

pub fn recv(port: u16) -> Option<UdpDatagram> {
    RX_QUEUES.lock()[port_index(port)].pop_front()
}

pub fn sendto(dst: IpAddr, dst_port: u16, src_port: u16, data: &[u8]) -> Result<usize, ()> {
    match dst {
        IpAddr::V4(ip) => {
            let mut payload = [0u8; 1500];
            if data.len() + 8 > payload.len() {
                return Err(());
            }
            payload[0..2].copy_from_slice(&src_port.to_be_bytes());
            payload[2..4].copy_from_slice(&dst_port.to_be_bytes());
            payload[4..6].copy_from_slice(&(8 + data.len() as u16).to_be_bytes());
            payload[6] = 0;
            payload[7] = 0;
            payload[8..8 + data.len()].copy_from_slice(data);
            let csum = ipv4::checksum(&payload[..8 + data.len()]);
            payload[6..8].copy_from_slice(&csum.to_be_bytes());
            ipv4::transmit(ip, IP_PROTO_UDP, &payload[..8 + data.len()]);
            Ok(data.len())
        }
        IpAddr::V6(_ip) => Err(()),
    }
}

pub fn handle_incoming(ip: &Ipv4Header, data: &[u8]) {
    if data.len() < 8 {
        return;
    }
    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let len = u16::from_be_bytes([data[4], data[5]]) as usize;
    if len < 8 || len > data.len() {
        return;
    }
    let dg = UdpDatagram {
        src: IpAddr::V4(ip.src),
        src_port,
        dst_port,
        data: data[8..len].to_vec(),
    };
    RX_QUEUES.lock()[port_index(dst_port)].push_back(dg);
}

pub fn handle_incoming_v6(_ip: &Ipv6Header, _data: &[u8]) {}
