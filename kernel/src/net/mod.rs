//! Theory OS network stack.

pub mod addr;
pub mod arp;
pub mod buffer;
pub mod device;
pub mod dhcp;
pub mod dns;
pub mod drivers;
pub mod eth;
pub mod http;
pub mod icmp;
pub mod ipv4;
pub mod ipv6;
pub mod pci;
pub mod socket;
pub mod tcp;
pub mod udp;
pub mod wifi;

use crate::console::Console;

pub fn init() {
    arp::init();
    udp::init();
    socket::init();
    Console::println("[net] protocol stack initialized");
}

pub fn late_init() {
    if drivers::probe() {
        dhcp::start();
    }
}

pub fn ensure_online() -> bool {
    if device::has_device() && dhcp::leased_ip().is_some() {
        return true;
    }
    if !device::has_device() {
        drivers::probe();
    }
    if !device::has_device() {
        return false;
    }
    dhcp::wait_for_lease(30_000)
}

pub fn rx_poll() {
    while let Some(pkt) = device::poll_rx() {
        eth::dispatch(&pkt);
    }
    tcp::tick();
}
