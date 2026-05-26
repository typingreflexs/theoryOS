//! Ethernet frame helpers.

use super::addr::MacAddr;
use super::buffer::PacketBuf;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

pub const ETH_HDR_LEN: usize = 14;

#[derive(Clone, Copy, Debug)]
pub struct EthHeader {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
}

impl EthHeader {
    pub fn parse(buf: &PacketBuf) -> Option<(Self, usize)> {
        if buf.len() < ETH_HDR_LEN {
            return None;
        }
        let data = buf.data();
        let mut dst = [0u8; 6];
        let mut src = [0u8; 6];
        dst.copy_from_slice(&data[0..6]);
        src.copy_from_slice(&data[6..12]);
        let ethertype = u16::from_be_bytes([data[12], data[13]]);
        Some((
            Self {
                dst: MacAddr(dst),
                src: MacAddr(src),
                ethertype,
            },
            ETH_HDR_LEN,
        ))
    }

    pub fn build(dst: MacAddr, src: MacAddr, ethertype: u16, payload: &[u8], out: &mut PacketBuf) {
        out.clear();
        out.extend_from_slice(&dst.0);
        out.extend_from_slice(&src.0);
        out.extend_from_slice(&ethertype.to_be_bytes());
        out.extend_from_slice(payload);
    }
}

pub fn dispatch(buf: &PacketBuf) {
    let Some((hdr, offset)) = EthHeader::parse(buf) else {
        return;
    };
    let payload = &buf.data()[offset..];
    match hdr.ethertype {
        ETHERTYPE_ARP => super::arp::handle_incoming(&hdr, payload),
        ETHERTYPE_IPV4 => super::ipv4::handle_incoming(&hdr, payload),
        ETHERTYPE_IPV6 => super::ipv6::handle_incoming(&hdr, payload),
        _ => {}
    }
}
