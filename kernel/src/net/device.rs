//! Network device abstraction and global traffic accounting.

use core::sync::atomic::{AtomicU64, Ordering};

use super::addr::MacAddr;
use super::buffer::PacketBuf;

pub trait NetDevice: Send + Sync {
    fn name(&self) -> &'static str;
    fn mac(&self) -> MacAddr;
    fn send(&self, pkt: &PacketBuf) -> Result<(), NetError>;
    fn poll_rx(&self) -> Option<PacketBuf>;
    fn irq(&self) -> u8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    NoDevice,
    TxBusy,
    InvalidPacket,
    Hardware,
}

use spin::Mutex;

static DEVICE: Mutex<Option<&'static dyn NetDevice>> = Mutex::new(None);

static TX_BYTES: AtomicU64 = AtomicU64::new(0);
static RX_BYTES: AtomicU64 = AtomicU64::new(0);
static TX_PACKETS: AtomicU64 = AtomicU64::new(0);
static RX_PACKETS: AtomicU64 = AtomicU64::new(0);

pub fn set_device(dev: &'static dyn NetDevice) {
    *DEVICE.lock() = Some(dev);
}

pub fn clear_device() {
    *DEVICE.lock() = None;
}

pub fn with_device<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&dyn NetDevice) -> R,
{
    DEVICE.lock().as_ref().map(|d| f(*d))
}

pub fn send(pkt: &PacketBuf) -> Result<(), NetError> {
    let result = DEVICE
        .lock()
        .as_ref()
        .ok_or(NetError::NoDevice)?
        .send(pkt);
    if result.is_ok() {
        TX_BYTES.fetch_add(pkt.len() as u64, Ordering::Relaxed);
        TX_PACKETS.fetch_add(1, Ordering::Relaxed);
    }
    result
}

pub fn poll_rx() -> Option<PacketBuf> {
    let pkt = DEVICE.lock().as_ref()?.poll_rx()?;
    RX_BYTES.fetch_add(pkt.len() as u64, Ordering::Relaxed);
    RX_PACKETS.fetch_add(1, Ordering::Relaxed);
    Some(pkt)
}

pub fn mac() -> Option<MacAddr> {
    DEVICE.lock().as_ref().map(|d| d.mac())
}

pub fn name() -> Option<&'static str> {
    DEVICE.lock().as_ref().map(|d| d.name())
}

pub fn has_device() -> bool {
    DEVICE.lock().is_some()
}

/// Cumulative byte/packet counters since the link came up.
pub fn stats() -> NetStats {
    NetStats {
        tx_bytes: TX_BYTES.load(Ordering::Relaxed),
        rx_bytes: RX_BYTES.load(Ordering::Relaxed),
        tx_packets: TX_PACKETS.load(Ordering::Relaxed),
        rx_packets: RX_PACKETS.load(Ordering::Relaxed),
    }
}

pub fn reset_stats() {
    TX_BYTES.store(0, Ordering::Relaxed);
    RX_BYTES.store(0, Ordering::Relaxed);
    TX_PACKETS.store(0, Ordering::Relaxed);
    RX_PACKETS.store(0, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NetStats {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
}
