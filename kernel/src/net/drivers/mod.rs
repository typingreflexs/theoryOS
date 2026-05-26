//! Network interface drivers — PCI probe for virtio-net and Intel e1000.
//!
//! `pci::map_bar0` maps MMIO before touching device registers.

pub mod e1000;
pub mod virtio;

use crate::console::Console;

use super::device;
use super::pci;

pub fn probe() -> bool {
    if let Some(pci_dev) = pci::find_virtio_net() {
        pci::enable_device(&pci_dev);
        pci::map_bar0(&pci_dev);
        if let Some(dev) = virtio::init(pci_dev) {
            device::set_device(dev);
            Console::println("[net] virtio-net initialized");
            return true;
        }
    }
    if let Some(pci_dev) = pci::find_device(0x8086, 0x100E) {
        pci::enable_device(&pci_dev);
        pci::map_bar0(&pci_dev);
        if let Some(dev) = e1000::init(pci_dev) {
            device::set_device(dev);
            Console::println("[net] e1000 initialized");
            return true;
        }
    }
    Console::println("[net] no NIC found");
    false
}
