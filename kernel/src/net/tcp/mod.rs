//! TCP — sliding window, retransmission, CUBIC congestion control.

pub mod cubic;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use spin::Mutex;

use crate::sched::timer;

use super::addr::{IpAddr, Ipv4Addr};
use super::ipv4::{self, Ipv4Header, IP_PROTO_TCP};
use super::ipv6::{Ipv6Header, IP6_PROTO_TCP};

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_PSH: u8 = 0x08;
pub const TCP_ACK: u8 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

#[derive(Clone, Debug)]
pub struct TcpControlBlock {
    pub local_port: u16,
    pub remote: IpAddr,
    pub remote_port: u16,
    pub state: TcpState,
    pub iss: u32,
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub rcv_nxt: u32,
    pub snd_wnd: u32,
    pub rcv_wnd: u32,
    pub mss: u32,
    pub cwnd: u32,
    pub ssthresh: u32,
    pub w_max: u32,
    pub rtt_ms: u32,
    pub rto_ms: u32,
    pub retransmit_at: u64,
    pub rx_queue: VecDeque<Vec<u8>>,
    pub tx_buffer: Vec<u8>,
    pub tx_sent: u32,
    pub listen_backlog: u32,
}

impl TcpControlBlock {
    pub fn new(local_port: u16) -> Self {
        let mut tcb = Self {
            local_port,
            remote: IpAddr::V4(Ipv4Addr::ANY),
            remote_port: 0,
            state: TcpState::Closed,
            iss: 0x1234_5678,
            snd_una: 0,
            snd_nxt: 0,
            rcv_nxt: 0,
            snd_wnd: 65535,
            rcv_wnd: 65535,
            mss: 1460,
            cwnd: 2920,
            ssthresh: 65535,
            w_max: 2920,
            rtt_ms: 100,
            rto_ms: 1000,
            retransmit_at: 0,
            rx_queue: VecDeque::new(),
            tx_buffer: Vec::new(),
            tx_sent: 0,
            listen_backlog: 8,
        };
        cubic::init_window(&mut tcb);
        tcb
    }
}

static CONNECTIONS: Mutex<[Option<TcpControlBlock>; 128]> = Mutex::new([const { None }; 128]);
static NEXT_ISN: Mutex<u32> = Mutex::new(0xA5A5_0000);

fn alloc_slot() -> Option<usize> {
    let mut conns = CONNECTIONS.lock();
    for (i, slot) in conns.iter_mut().enumerate() {
        if slot.is_none() {
            return Some(i);
        }
    }
    None
}

pub fn create_listener(port: u16) -> Option<u32> {
    let idx = alloc_slot()?;
    let mut tcb = TcpControlBlock::new(port);
    tcb.state = TcpState::Listen;
    CONNECTIONS.lock()[idx] = Some(tcb);
    Some(idx as u32)
}

pub fn connect(remote: IpAddr, remote_port: u16, local_port: u16) -> Option<u32> {
    let idx = alloc_slot()?;
    let mut isn = *NEXT_ISN.lock();
    *NEXT_ISN.lock() = isn.wrapping_add(64000);
    let mut tcb = TcpControlBlock::new(local_port);
    tcb.remote = remote;
    tcb.remote_port = remote_port;
    tcb.state = TcpState::SynSent;
    tcb.iss = isn;
    tcb.snd_nxt = isn;
    tcb.snd_una = isn;
    send_syn(&tcb);
    CONNECTIONS.lock()[idx] = Some(tcb);
    Some(idx as u32)
}

pub fn accept(listener_idx: u32) -> Option<u32> {
    let idx = listener_idx as usize;
    let listener_port = CONNECTIONS.lock().get(idx)?.as_ref()?.local_port;
    for (i, slot) in CONNECTIONS.lock().iter().enumerate() {
        if let Some(c) = slot {
            if c.state == TcpState::Established && c.local_port == listener_port {
                return Some(i as u32);
            }
        }
    }
    None
}

pub fn recv(conn_idx: u32, buf: &mut [u8]) -> Option<usize> {
    let idx = conn_idx as usize;
    let mut conns = CONNECTIONS.lock();
    let tcb = conns.get_mut(idx).and_then(|s| s.as_mut())?;
    if let Some(data) = tcb.rx_queue.pop_front() {
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        return Some(n);
    }
    None
}

pub fn send(conn_idx: u32, data: &[u8]) -> Result<usize, ()> {
    let idx = conn_idx as usize;
    let mut conns = CONNECTIONS.lock();
    let tcb = conns.get_mut(idx).and_then(|s| s.as_mut()).ok_or(())?;
    if tcb.state != TcpState::Established {
        return Err(());
    }
    let in_flight = tcb.snd_nxt.wrapping_sub(tcb.snd_una);
    if in_flight + data.len() as u32 > tcb.cwnd.min(tcb.snd_wnd) {
        return Err(());
    }
    tcb.tx_buffer.extend_from_slice(data);
    send_data(tcb);
    Ok(data.len())
}

fn send_syn(tcb: &TcpControlBlock) {
    let mut seg = build_segment(tcb, TCP_SYN, tcb.iss, 0, &[]);
    seg.options.extend_from_slice(&[2, 4, 5, 0xB4]); // MSS 1460
    transmit(tcb, &seg);
}

fn send_data(tcb: &mut TcpControlBlock) {
    while tcb.tx_sent < tcb.tx_buffer.len() as u32 {
        let avail = tcb.cwnd.min(tcb.snd_wnd).saturating_sub(tcb.snd_nxt.wrapping_sub(tcb.snd_una));
        if avail == 0 {
            break;
        }
        let start = tcb.tx_sent as usize;
        let end = (start + avail as usize).min(tcb.tx_buffer.len());
        let chunk = &tcb.tx_buffer[start..end];
        let seg = build_segment(tcb, TCP_ACK | TCP_PSH, tcb.snd_nxt, tcb.rcv_nxt, chunk);
        tcb.snd_nxt = tcb.snd_nxt.wrapping_add(chunk.len() as u32);
        tcb.tx_sent = end as u32;
        tcb.retransmit_at = timer::ticks() + (tcb.rto_ms as u64);
        transmit(tcb, &seg);
    }
}

struct TcpSegment {
    header: [u8; 20],
    options: Vec<u8>,
    payload: Vec<u8>,
}

fn build_segment(tcb: &TcpControlBlock, flags: u8, seq: u32, ack: u32, payload: &[u8]) -> TcpSegment {
    let mut header = [0u8; 20];
    header[0..2].copy_from_slice(&tcb.local_port.to_be_bytes());
    header[2..4].copy_from_slice(&tcb.remote_port.to_be_bytes());
    header[4..8].copy_from_slice(&seq.to_be_bytes());
    header[8..12].copy_from_slice(&ack.to_be_bytes());
    header[12] = 0x50;
    header[13] = flags;
    header[14..16].copy_from_slice(&tcb.rcv_wnd.to_be_bytes());
    TcpSegment {
        header,
        options: Vec::new(),
        payload: payload.to_vec(),
    }
}

fn transmit(tcb: &TcpControlBlock, seg: &TcpSegment) {
    let mut buf = Vec::new();
    buf.extend_from_slice(&seg.header);
    buf.extend_from_slice(&seg.options);
    buf.extend_from_slice(&seg.payload);
    let mut pseudo = [0u8; 12];
    if let IpAddr::V4(ip) = tcb.remote {
        // use our IP from arp
        let src = super::arp::our_ip();
        pseudo[0..4].copy_from_slice(&src.0);
        pseudo[4..8].copy_from_slice(&ip.0);
        pseudo[9] = IP_PROTO_TCP;
        pseudo[10..12].copy_from_slice(&(buf.len() as u16).to_be_bytes());
    }
    let mut sum_buf = Vec::with_capacity(pseudo.len() + buf.len());
    sum_buf.extend_from_slice(&pseudo);
    sum_buf.extend_from_slice(&buf);
    let csum = ipv4::checksum(&sum_buf);
    buf[16..18].copy_from_slice(&csum.to_be_bytes());

    if let IpAddr::V4(dst) = tcb.remote {
        ipv4::transmit(dst, IP_PROTO_TCP, &buf);
    }
}

pub fn handle_incoming(ip: &Ipv4Header, data: &[u8]) {
    if data.len() < 20 {
        return;
    }
    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ack = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let flags = data[13];
    let hdr_len = ((data[12] >> 4) as usize) * 4;
    if data.len() < hdr_len {
        return;
    }
    let payload = &data[hdr_len..];

    let mut conns = CONNECTIONS.lock();
    for slot in conns.iter_mut() {
        let Some(tcb) = slot else { continue };
        if tcb.local_port != dst_port {
            continue;
        }
        if tcb.state == TcpState::Listen && (flags & TCP_SYN) != 0 {
            let mut child = TcpControlBlock::new(dst_port);
            child.remote = IpAddr::V4(ip.src);
            child.remote_port = src_port;
            child.state = TcpState::SynReceived;
            child.iss = NEXT_ISN.lock().wrapping_add(1);
            child.rcv_nxt = seq.wrapping_add(1);
            child.snd_nxt = child.iss.wrapping_add(1);
            child.snd_una = child.iss;
            let mut reply = build_segment(&child, TCP_SYN | TCP_ACK, child.iss, child.rcv_nxt, &[]);
            reply.options.extend_from_slice(&[2, 4, 5, 0xB4]);
            transmit(&child, &reply);
            child.state = TcpState::Established;
            if let Some(free) = conns.iter().position(|s| s.is_none()) {
                conns[free] = Some(child);
            }
            return;
        }
        if tcb.remote_port != src_port {
            continue;
        }
        if let IpAddr::V4(addr) = tcb.remote {
            if addr != ip.src {
                continue;
            }
        }
        process_segment(tcb, flags, seq, ack, payload);
        return;
    }
}

fn process_segment(tcb: &mut TcpControlBlock, flags: u8, seq: u32, ack: u32, payload: &[u8]) {
    if (flags & TCP_RST) != 0 {
        tcb.state = TcpState::Closed;
        return;
    }
    match tcb.state {
        TcpState::SynSent if (flags & (TCP_SYN | TCP_ACK)) == (TCP_SYN | TCP_ACK) => {
            tcb.rcv_nxt = seq.wrapping_add(1);
            tcb.snd_una = ack;
            tcb.state = TcpState::Established;
            let seg = build_segment(tcb, TCP_ACK, tcb.snd_nxt, tcb.rcv_nxt, &[]);
            transmit(tcb, &seg);
        }
        TcpState::Established => {
            if !payload.is_empty() && seq == tcb.rcv_nxt {
                tcb.rx_queue.push_back(payload.to_vec());
                tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(payload.len() as u32);
            }
            if (flags & TCP_ACK) != 0 && ack > tcb.snd_una {
                let acked = ack.wrapping_sub(tcb.snd_una);
                tcb.snd_una = ack;
                cubic::on_ack(tcb, acked);
            }
            if (flags & TCP_FIN) != 0 {
                tcb.state = TcpState::CloseWait;
            }
            let seg = build_segment(tcb, TCP_ACK, tcb.snd_nxt, tcb.rcv_nxt, &[]);
            transmit(tcb, &seg);
        }
        _ => {}
    }
}

pub fn handle_incoming_v6(_ip: &Ipv6Header, _data: &[u8]) {}

pub fn tick() {
    let now = timer::ticks();
    let mut conns = CONNECTIONS.lock();
    for slot in conns.iter_mut() {
        let Some(tcb) = slot else { continue };
        if tcb.state != TcpState::Established {
            continue;
        }
        if tcb.retransmit_at != 0 && now >= tcb.retransmit_at {
            cubic::on_loss(tcb);
            tcb.rto_ms = (tcb.rto_ms * 2).min(60000);
            tcb.snd_nxt = tcb.snd_una;
            tcb.tx_sent = tcb.snd_una.wrapping_sub(tcb.iss);
            send_data(tcb);
        }
    }
}

pub fn with_connection<F, R>(idx: u32, f: F) -> Option<R>
where
    F: FnOnce(&TcpControlBlock) -> R,
{
    CONNECTIONS.lock().get(idx as usize)?.as_ref().map(f)
}

pub fn with_connection_mut<F, R>(idx: u32, f: F) -> Option<R>
where
    F: FnOnce(&mut TcpControlBlock) -> R,
{
    CONNECTIONS.lock().get_mut(idx as usize)?.as_mut().map(f)
}

pub fn wait_established(conn_idx: u32, max_iters: u32) -> bool {
    for _ in 0..max_iters {
        crate::net::rx_poll();
        if with_connection(conn_idx, |c| c.state == TcpState::Established).unwrap_or(false) {
            return true;
        }
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
    false
}

pub fn recv_blocking(conn_idx: u32, buf: &mut [u8], max_iters: u32) -> usize {
    let mut total = 0usize;
    let mut idle = 0u32;
    for _ in 0..max_iters {
        crate::net::rx_poll();
        tick();
        if let Some(n) = recv(conn_idx, &mut buf[total..]) {
            total += n;
            idle = 0;
            if total >= buf.len() {
                break;
            }
        } else if with_connection(conn_idx, |c| c.state == TcpState::Closed).unwrap_or(false) {
            break;
        } else {
            idle += 1;
            if idle > 2000 && total > 0 {
                break;
            }
        }
        for _ in 0..50 {
            core::hint::spin_loop();
        }
    }
    total
}
