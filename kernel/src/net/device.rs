//! Network device abstraction.

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

pub fn set_device(dev: &'static dyn NetDevice) {
    *DEVICE.lock() = Some(dev);
}

pub fn with_device<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&dyn NetDevice) -> R,
{
    DEVICE.lock().as_ref().map(|d| f(*d))
}

pub fn send(pkt: &PacketBuf) -> Result<(), NetError> {
    DEVICE
        .lock()
        .as_ref()
        .ok_or(NetError::NoDevice)?
        .send(pkt)
}

pub fn poll_rx() -> Option<PacketBuf> {
    DEVICE.lock().as_ref()?.poll_rx()
}

pub fn mac() -> Option<MacAddr> {
    DEVICE.lock().as_ref().map(|d| d.mac())
}

pub fn has_device() -> bool {
    DEVICE.lock().is_some()
}
