//! TCP CUBIC congestion control (integer-friendly for no_std).

use super::TcpControlBlock;

const BETA_NUM: u32 = 7;
const BETA_DEN: u32 = 10;

pub fn on_ack(tcb: &mut TcpControlBlock, acked_bytes: u32) {
    if tcb.cwnd < tcb.ssthresh {
        tcb.cwnd = tcb.cwnd.saturating_add(acked_bytes.max(tcb.mss));
    } else {
        // Simplified CUBIC-inspired growth: W += (acked * C) / cwnd
        let cubic_inc = (acked_bytes * 4) / tcb.cwnd.max(1);
        let tcp_inc = acked_bytes / tcb.cwnd.max(1);
        tcb.cwnd = tcb.cwnd.saturating_add(cubic_inc.max(tcp_inc).max(1));
    }
    tcb.cwnd = tcb.cwnd.min(tcb.snd_wnd);
}

pub fn on_loss(tcb: &mut TcpControlBlock) {
    tcb.w_max = tcb.cwnd;
    tcb.ssthresh = (tcb.cwnd * BETA_NUM / BETA_DEN).max(2 * tcb.mss);
    tcb.cwnd = tcb.mss;
}

pub fn init_window(tcb: &mut TcpControlBlock) {
    tcb.mss = 1460;
    tcb.cwnd = 2 * tcb.mss;
    tcb.ssthresh = 65535;
    tcb.w_max = tcb.cwnd;
}
